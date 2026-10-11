//! NetworkOpponent — implements Opponent trait over TCP.
//!
//! Two-plane architecture:
//! - Control plane: immediate send/recv of ControlMessages
//! - Data plane: batched gameplay garbage at CommCycle boundaries
//!
//! The network thread is autonomous: it reads TCP continuously
//! and sends control messages immediately. It does NOT block
//! waiting for the game thread.

use std::sync::mpsc;
use std::thread;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crackattack_core::vs::Opponent;
use crate::connection::NetConnection;
use crate::protocol::*;
use crate::communicator;

/// Game thread → Network thread.
enum ToNet {
    /// Garbage to accumulate for the next data plane flush.
    SendGarbage { height: i32, width: i32, flavor: i32, timestamp: i32 },
    /// Send a control message immediately.
    SendControl(ControlMessage),
    /// Flush accumulated garbage now (data plane, every COMMUNICATION_PERIOD).
    FlushGarbage,
    /// Shutdown the network thread.
    Shutdown,
}

/// Network thread → Game thread.
enum FromNet {
    /// Garbage from opponent (data plane).
    Garbage { height: i32, width: i32, flavor: i32 },
    /// Control message from opponent (control plane).
    Control(ControlMessage),
    /// Network lag detected (no data for 2+ cycles).
    LagPause,
    /// Network lag resolved.
    LagResume,
    /// Opponent disconnected.
    Disconnected(String),
}

/// Network opponent implementing the Opponent trait.
/// All TCP I/O happens in a dedicated autonomous thread.
pub struct NetworkOpponent {
    to_net: mpsc::Sender<ToNet>,
    from_net: mpsc::Receiver<FromNet>,
    incoming_garbage: VecDeque<(i32, i32, i32)>,
    opponent_grid_level: u8,
    name: String,
    /// Persistent peer identity, received in the handshake. Used by the ladder.
    uuid: [u8; 16],
    connected: bool,
    lag_paused: bool,
    last_recv_sync: i32,
    local_round_ready: u32,
    opponent_round_ready_val: u32,
    current_tick: i32,
    local_grid_level: u8,
    evt_opponent_paused: bool,
    evt_opponent_unpaused: bool,
    evt_lag_pause: bool,
    evt_lag_resume: bool,
    remote_pause_target: i32,
    remote_pause_seq: u16,
    remote_round_target: i32,
    opponent_stats: Option<crackattack_core::stats::GameStats>,
    ping_seq: u32,
    ping_sent_at: Option<Instant>,
    /// Last 5 RTT samples, in ticks (sliding window).
    rtt_samples: Vec<i32>,
    /// Estimated RTT in ticks (median of recent samples). 0 = unknown.
    rtt_ticks: i32,
    received_life_report: Option<(RoundLifeStatus, u32)>,
    received_verdict: Option<(PeerRole, VerdictReason)>,
    _thread: Option<thread::JoinHandle<()>>,
}

impl NetworkOpponent {
    /// Create a NetworkOpponent from an established connection.
    /// Spawns the autonomous network thread immediately.
    pub fn new(
        mut conn: NetConnection,
        _role: crate::connection::Role,
        opponent_name: String,
        opponent_uuid: [u8; 16],
    ) -> Self {
        let (tx_to_net, rx_from_game) = mpsc::channel::<ToNet>();
        let (tx_to_game, rx_from_net) = mpsc::channel::<FromNet>();

        let handle = thread::Builder::new()
            .name("network-opponent".into())
            .spawn(move || {
                network_thread_main(&mut conn, rx_from_game, tx_to_game);
            })
            .expect("failed to spawn network thread");

        Self {
            to_net: tx_to_net,
            from_net: rx_from_net,
            incoming_garbage: VecDeque::new(),
            opponent_grid_level: 0,
            name: opponent_name,
            uuid: opponent_uuid,
            connected: true,
            lag_paused: false,
            last_recv_sync: 0,
            local_round_ready: 0,
            opponent_round_ready_val: 0,
            current_tick: 0,
            local_grid_level: 0,
            evt_opponent_paused: false,
            evt_opponent_unpaused: false,
            evt_lag_pause: false,
            evt_lag_resume: false,
            remote_pause_target: 0,
            remote_pause_seq: 0,
            remote_round_target: 0,
            opponent_stats: None,
            ping_seq: 0,
            ping_sent_at: None,
            rtt_samples: Vec::new(),
            rtt_ticks: COMMUNICATION_PERIOD, // Until measured, assume one communication cycle.
            received_life_report: None,
            received_verdict: None,
            _thread: Some(handle),
        }
    }

    pub(crate) fn is_connected(&self) -> bool { self.connected }
    /// Persistent peer identity (used by the ladder).
    pub(crate) fn opponent_uuid(&self) -> [u8; 16] { self.uuid }

    pub(crate) fn take_opponent_paused(&mut self) -> bool {
        let v = self.evt_opponent_paused; self.evt_opponent_paused = false; v
    }
    pub(crate) fn take_opponent_unpaused(&mut self) -> bool {
        let v = self.evt_opponent_unpaused; self.evt_opponent_unpaused = false; v
    }
    pub(crate) fn take_lag_pause(&mut self) -> bool {
        let v = self.evt_lag_pause; self.evt_lag_pause = false; v
    }
    pub(crate) fn take_lag_resume(&mut self) -> bool {
        let v = self.evt_lag_resume; self.evt_lag_resume = false; v
    }

    pub(crate) fn current_tick(&self) -> i32 { self.current_tick }
    pub(crate) fn last_recv_sync(&self) -> i32 { self.last_recv_sync }
    pub(crate) fn remote_pause_target(&self) -> i32 { self.remote_pause_target }
    pub(crate) fn remote_pause_seq(&self) -> u16 { self.remote_pause_seq }
    pub(crate) fn clear_remote_pause_sync(&mut self) {
        self.remote_pause_target = 0;
    }

    pub(crate) fn remote_round_target(&self) -> i32 { self.remote_round_target }
    pub(crate) fn clear_remote_round_sync(&mut self) {
        self.remote_round_target = 0;
    }

    pub(crate) fn set_local_grid_level(&mut self, level: i32) {
        self.local_grid_level = level.clamp(0, 12) as u8;
    }

    /// Estimated RTT in ticks (median of recent samples).
    /// Falls back to COMMUNICATION_PERIOD if not yet measured.
    pub(crate) fn rtt_ticks(&self) -> i32 {
        self.rtt_ticks
    }

    /// Opponent's post-game stats (received via ControlMessage::Stats).
    pub(crate) fn opponent_stats(&self) -> Option<&crackattack_core::stats::GameStats> {
        self.opponent_stats.as_ref()
    }

    /// Take the received life report (consumed once).
    pub(crate) fn take_life_report(&mut self) -> Option<(RoundLifeStatus, u32)> {
        self.received_life_report.take()
    }

    /// Take the received verdict (consumed once).
    pub(crate) fn take_verdict(&mut self) -> Option<(PeerRole, VerdictReason)> {
        self.received_verdict.take()
    }

    /// Send a RoundLifeReport to the opponent.
    pub(crate) fn send_life_report(&self, round_id: u16, status: RoundLifeStatus, swaps: u32) {
        self.send_control(ControlMessage::RoundLifeReport { round_id, status, swaps_performed: swaps });
    }

    /// Send a RoundVerdict to the opponent (server only).
    pub(crate) fn send_verdict(&self, round_id: u16, winner: PeerRole, reason: VerdictReason) {
        self.send_control(ControlMessage::RoundVerdict { round_id, winner, reason });
    }

    /// Send a control message immediately (not batched).
    pub(crate) fn send_control(&self, msg: ControlMessage) {
        let _ = self.to_net.send(ToNet::SendControl(msg));
    }

    /// Signal forfeit and shut down the network thread.
    /// The opponent will detect a disconnection.
    pub(crate) fn signal_forfeit(&self) {
        // Send EndPlay to inform the opponent, then shut down.
        let _ = self.to_net.send(ToNet::SendControl(ControlMessage::EndPlay));
        let _ = self.to_net.send(ToNet::Shutdown);
    }

    /// Poll the channel for incoming events (non-blocking).
    fn drain_incoming(&mut self) {
        while let Ok(msg) = self.from_net.try_recv() {
            match msg {
                FromNet::Garbage { height, width, flavor } => {
                    self.incoming_garbage.push_back((height, width, flavor));
                }
                FromNet::Control(ctrl) => {
                    self.process_control(ctrl);
                }
                FromNet::LagPause => {
                    self.lag_paused = true;
                    self.evt_lag_pause = true;
                    log::info!("Network lag pause activated");
                }
                FromNet::LagResume => {
                    self.lag_paused = false;
                    self.evt_lag_resume = true;
                    log::info!("Network lag pause resolved");
                }
                FromNet::Disconnected(reason) => {
                    log::warn!("Opponent disconnected: {}", reason);
                    self.connected = false;
                }
            }
        }
    }

    /// Process a received control message.
    fn process_control(&mut self, msg: ControlMessage) {
        match msg {
            ControlMessage::Pause => {
                self.evt_opponent_paused = true;
                log::info!("Opponent paused (control)");
            }
            ControlMessage::Unpause { epoch } => {
                self.evt_opponent_unpaused = true;
                self.remote_pause_seq = epoch;
                log::info!("Opponent unpaused (control, epoch={})", epoch);
            }
            ControlMessage::Ready { context, epoch } => {
                match context {
                    SyncContext::RoundStart => {
                        self.opponent_round_ready_val = epoch as u32;
                        log::info!("Opponent round ready: {} (control)", epoch);
                    }
                    SyncContext::PauseResume => {
                        // Pause resume is synchronized by `StartAt`, not `Ready`.
                    }
                }
            }
            ControlMessage::StartAt { context, epoch, tick } => {
                match context {
                    SyncContext::PauseResume => {
                        self.remote_pause_target = tick;
                        self.remote_pause_seq = epoch;
                        log::info!("Opponent StartAt(PauseResume): tick={} epoch={}", tick, epoch);
                    }
                    SyncContext::RoundStart => {
                        self.remote_round_target = tick;
                        log::info!("Opponent StartAt(RoundStart): tick={} epoch={}", tick, epoch);
                    }
                }
            }
            ControlMessage::GridLevel(level) => {
                self.opponent_grid_level = level;
            }
            ControlMessage::SyncCounter(counter) => {
                self.last_recv_sync = counter as i32;
            }
            ControlMessage::RoundLifeReport { round_id, status, swaps_performed } => {
                self.received_life_report = Some((status, swaps_performed));
                log::info!("Received RoundLifeReport: round={} status={:?} swaps={}", round_id, status, swaps_performed);
            }
            ControlMessage::RoundVerdict { round_id, winner, reason } => {
                self.received_verdict = Some((winner, reason));
                log::info!("Received RoundVerdict: round={} winner={:?} reason={:?}", round_id, winner, reason);
            }
            ControlMessage::EndPlay => {
                log::info!("Round ended (control)");
            }
            ControlMessage::Ping(seq) => {
                // Reply with a pong.
                let _ = self.to_net.send(ToNet::SendControl(ControlMessage::Pong(seq)));
            }
            ControlMessage::Stats {
                blocks_eliminated, chains_count, max_chain_level,
                garbage_sent, garbage_received, garbage_shattered,
                swaps_performed, duration_ticks,
            } => {
                self.opponent_stats = Some(crackattack_core::stats::GameStats {
                    blocks_eliminated,
                    chains_count,
                    max_chain_level,
                    garbage_sent,
                    garbage_received,
                    garbage_shattered,
                    swaps_performed,
                    duration_ticks,
                });
                log::info!("Received opponent stats: blocks={} chains={} duration={}",
                    blocks_eliminated, chains_count, duration_ticks);
            }
            ControlMessage::Pong(seq) => {
                // Compute the RTT if this answers our outstanding ping.
                if seq == self.ping_seq {
                    if let Some(sent_at) = self.ping_sent_at.take() {
                        let rtt_ms = sent_at.elapsed().as_millis() as i32;
                        // Convert ms to ticks (50 ticks/s = 20 ms/tick).
                        let rtt_in_ticks = (rtt_ms / 20).max(1);
                        self.rtt_samples.push(rtt_in_ticks);
                        if self.rtt_samples.len() > 5 {
                            self.rtt_samples.remove(0);
                        }
                        // Median of the samples.
                        let mut sorted = self.rtt_samples.clone();
                        sorted.sort();
                        self.rtt_ticks = sorted[sorted.len() / 2];
                        log::debug!("RTT: {}ms = {} ticks (median={})",
                            rtt_ms, rtt_in_ticks, self.rtt_ticks);
                    }
                }
            }
        }
    }
}

impl Opponent for NetworkOpponent {
    fn game_start(&mut self) {
        self.incoming_garbage.clear();
        self.opponent_grid_level = 0;
        self.opponent_stats = None;
        self.received_life_report = None;
        self.received_verdict = None;
        self.lag_paused = false;
        // `current_tick` and `last_recv_sync` run on for the whole connection. Both sides
        // prepare a round at slightly different moments, so a counter reset here would shift
        // the two clocks, and the round start ("start at tick N") would follow that shift.
        self.local_grid_level = 0;
        self.evt_opponent_paused = false;
        self.evt_opponent_unpaused = false;
        self.evt_lag_pause = false;
        self.evt_lag_resume = false;
        self.remote_pause_target = 0;
        self.remote_pause_seq = 0;
        self.remote_round_target = 0;
        // `local_round_ready` and `opponent_round_ready_val` are not reset: they carry across rounds.
    }

    fn time_step(&mut self) {
        self.current_tick += 1;
        self.drain_incoming();

        // Once per communication cycle: flush garbage, send grid level and sync counter.
        if self.current_tick % COMMUNICATION_PERIOD == 0 {
            // Data plane: flush gameplay garbage.
            let _ = self.to_net.send(ToNet::FlushGarbage);
            // Control plane: grid level and sync counter.
            let _ = self.to_net.send(ToNet::SendControl(
                ControlMessage::GridLevel(self.local_grid_level),
            ));
            let _ = self.to_net.send(ToNet::SendControl(
                ControlMessage::SyncCounter(self.current_tick as u32),
            ));
        }

        // RTT ping every 100 ticks (~2 s). The offset of 50 keeps it away from
        // the communication cycle boundary, so the sends do not burst.
        if self.current_tick % 100 == 50 {
            self.ping_seq = self.ping_seq.wrapping_add(1);
            self.ping_sent_at = Some(Instant::now());
            let _ = self.to_net.send(ToNet::SendControl(
                ControlMessage::Ping(self.ping_seq),
            ));
        }
    }

    fn send_garbage(&mut self, height: i32, width: i32, flavor: i32) {
        let _ = self.to_net.send(ToNet::SendGarbage {
            height, width, flavor, timestamp: self.current_tick,
        });
    }

    fn recv_garbage(&mut self) -> Vec<(i32, i32, i32)> {
        self.incoming_garbage.drain(..).collect()
    }

    fn grid_level(&mut self) -> i32 {
        self.opponent_grid_level as i32
    }

    fn notify_local_loss(&mut self, _time_step: i32) {
        // No-op: round arbitration uses `send_life_report()` instead.
    }

    fn has_opponent_lost(&self) -> bool { false }
    fn opponent_loss_timestamp(&self) -> Option<i32> { None }

    fn signal_paused(&mut self) {
        self.send_control(ControlMessage::Pause);
    }

    // There is no unpause hook here. The session sends the full Unpause
    // message with the right epoch (`request_resume`). A hook here would
    // send epoch 0 and overwrite `remote_pause_seq` on the peer.

    fn is_connected(&self) -> bool { self.connected }
    fn opponent_name(&self) -> &str { &self.name }

    fn set_local_round_ready(&mut self, round: u32) {
        self.local_round_ready = round;
        self.send_control(ControlMessage::Ready {
            context: SyncContext::RoundStart,
            epoch: round as u16,
        });
        log::info!("Local round ready set to {} (control)", round);
    }

    fn opponent_round_ready(&self) -> u32 {
        self.opponent_round_ready_val
    }
}

impl Drop for NetworkOpponent {
    fn drop(&mut self) {
        let _ = self.to_net.send(ToNet::Shutdown);
        if let Some(handle) = self._thread.take() {
            if let Err(e) = handle.join() {
                log::warn!("Network thread panicked: {:?}", e);
            }
        }
    }
}

/// Thread poll interval (between TCP reads when idle).
const THREAD_POLL_MS: u64 = 1;

fn network_thread_main(
    conn: &mut NetConnection,
    rx: mpsc::Receiver<ToNet>,
    tx: mpsc::Sender<FromNet>,
) {
    let mut garbage_buf = GameplayBuffer::new();
    let mut was_lag_paused = false;
    let mut last_recv = Instant::now();
    // Resumable reader: the bytes of a split frame survive from one poll to the next.
    let mut reader = communicator::MessageReader::new();

    log::info!("Network thread started (autonomous, two-plane)");

    loop {
        // 1. Handle all pending game thread messages (non-blocking).
        loop {
            match rx.try_recv() {
                Ok(ToNet::SendGarbage { height, width, flavor, timestamp }) => {
                    garbage_buf.garbage.push(GarbageEntry { height, width, flavor, timestamp });
                }
                Ok(ToNet::SendControl(ctrl)) => {
                    // Control plane: send immediately.
                    log::debug!("[NET CTRL SEND] {:?}", ctrl);
                    if let Err(e) = communicator::send_control(conn, &ctrl) {
                        log::error!("Control send failed: {}", e);
                        let _ = tx.send(FromNet::Disconnected(e));
                        return;
                    }
                }
                Ok(ToNet::FlushGarbage) => {
                    // Data plane: send the accumulated garbage.
                    if !garbage_buf.garbage.is_empty() {
                        for g in &garbage_buf.garbage {
                            log::info!("[NET DATA SEND] garbage h={} w={} f={} ts={}",
                                g.height, g.width, g.flavor, g.timestamp);
                        }
                    }
                    if let Err(e) = communicator::send_gameplay(conn, &garbage_buf) {
                        log::error!("Gameplay send failed: {}", e);
                        let _ = tx.send(FromNet::Disconnected(e));
                        return;
                    }
                    garbage_buf.garbage.clear();
                }
                Ok(ToNet::Shutdown) => {
                    log::info!("Network thread shutting down");
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    log::info!("Game thread disconnected, exiting");
                    return;
                }
            }
        }

        // 2. Poll TCP for incoming messages (non-blocking).
        match reader.poll(conn) {
            Ok(Some(communicator::ReceivedMessage::Data(buf))) => {
                if !buf.garbage.is_empty() {
                    for g in &buf.garbage {
                        log::info!("[NET DATA RECV] garbage h={} w={} f={} ts={}",
                            g.height, g.width, g.flavor, g.timestamp);
                    }
                }
                for entry in &buf.garbage {
                    let _ = tx.send(FromNet::Garbage {
                        height: entry.height,
                        width: entry.width,
                        flavor: entry.flavor,
                    });
                }
                // Any received data counts as activity.
                last_recv = Instant::now();
                if was_lag_paused {
                    let _ = tx.send(FromNet::LagResume);
                    was_lag_paused = false;
                }
            }
            Ok(Some(communicator::ReceivedMessage::Control(ctrl))) => {
                log::debug!("[NET CTRL RECV] {:?}", ctrl);
                let _ = tx.send(FromNet::Control(ctrl));
                last_recv = Instant::now();
                if was_lag_paused {
                    let _ = tx.send(FromNet::LagResume);
                    was_lag_paused = false;
                }
            }
            Ok(None) => {
                // No data: check lag from the real time since the last receive.
                let silence = last_recv.elapsed();
                let cycle_ms = COMMUNICATION_PERIOD as u64 * 20; // 32 ticks = ~640 ms per cycle
                if silence > Duration::from_millis(cycle_ms * communicator::DISCONNECT_CYCLES as u64) {
                    log::warn!("Opponent timed out ({:.1}s silence)", silence.as_secs_f32());
                    let _ = tx.send(FromNet::Disconnected("timeout".into()));
                    return;
                }
                if silence > Duration::from_millis(cycle_ms * communicator::LAG_PAUSE_CYCLES as u64)
                    && !was_lag_paused
                {
                    let _ = tx.send(FromNet::LagPause);
                    was_lag_paused = true;
                }
            }
            Err(e) => {
                log::error!("Network error: {}", e);
                let _ = tx.send(FromNet::Disconnected(e));
                return;
            }
        }

        // 3. Sleep briefly to avoid spinning.
        thread::sleep(Duration::from_millis(THREAD_POLL_MS));
    }
}
