//! Network negotiation: an explicit phase, plus the zone services.
//!
//! The module keeps three invariants in one place:
//!
//! - Both active phases of [`NegotiationPhase`] carry a `cancel` flag, so an outgoing invite
//!   and an accepted incoming invite can both be cancelled.
//! - The mDNS status (Available/Busy/InGame) is derived by
//!   [`NegotiationState::announced_status`] from the phase and the game state. It is pushed
//!   from a single place, [`NegotiationState::sync_announced_status`].
//! - "A negotiation is in progress" has one predicate, [`NegotiationState::in_progress`].
//!
//! Scope: the module owns the zone services (TCP listener, mDNS discovery, seen peers), the
//! queue of incoming invites and the negotiation phase. It returns established connections
//! ([`EstablishedConnection`]). Creating the game (`VsNetworkGameSession`) and switching the
//! flow stay in `session_runtime`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crackattack_net::communicator::{HandshakeResult, InviteResponse};
use crackattack_net::connection::{NetConnection, Role};
use crackattack_net::discovery::{DiscoveryCommand, DiscoveryService, PeerStatus};
use crackattack_net::error::NetError;

use crate::app::meta_state::{IncomingInvite, LanPeer};

/// Network contact of the peer, captured at connection time (used to reconnect quickly).
#[derive(Clone, Debug)]
pub(crate) struct OpponentNetInfo {
    /// Peer IP address, as seen on the socket.
    pub addr: String,
    /// Listen port the peer announced in the handshake (0 = not listening).
    pub listen_port: u16,
}

/// Typed failure of an outgoing or incoming connection attempt.
///
/// `Display` gives the message shown in logs and UI.
#[derive(Debug)]
pub(crate) enum ConnectFailure {
    /// The invited player explicitly declined. We do not insist.
    Declined,
    /// The invited player is busy with another negotiation. This is the only variant shown on screen.
    Busy,
    /// The peer vanished or stayed silent while we waited: an implicit decline (silent).
    PeerGone,
    /// The invite was cancelled locally (the phase's `cancel` flag).
    Cancelled,
    /// The handshake failed. Carries the typed error from `crackattack-net`.
    HandshakeFailed(NetError),
    /// Connection or pre-handshake error (message already formatted for log/UI).
    Io(String),
}

impl ConnectFailure {
    /// Returns whether the failure is silent: back to the list with no message (explicit or
    /// implicit decline, cancellation). Other failures show an error (the `Error` phase of
    /// the LAN screen).
    pub(crate) fn is_silent(&self) -> bool {
        matches!(self, Self::Declined | Self::PeerGone | Self::Cancelled)
    }
}

impl ConnectFailure {
    /// Converts the failure to the error shown on the LAN screen.
    ///
    /// Only non-silent variants are expected here. The silent ones get a generic fallback.
    pub(crate) fn to_lan_error(&self) -> crate::app::meta_state::LanError {
        use crate::app::meta_state::LanError;
        match self {
            Self::Busy => LanError::PeerBusy,
            // Version mismatch: a dedicated player-facing message, without jargon.
            Self::HandshakeFailed(NetError::VersionMismatch { .. }) => LanError::VersionMismatch,
            Self::HandshakeFailed(e) => LanError::Handshake(e.to_string()),
            Self::Io(m) => LanError::Connection(m.clone()),
            // Silent variants are never shown. Generic fallback in case a future path shows them.
            Self::Declined | Self::PeerGone | Self::Cancelled => {
                LanError::Connection(self.to_string())
            }
        }
    }
}

impl std::fmt::Display for ConnectFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Log text only; the LAN screen maps failures through `to_lan_error()` and `locale.rs`.
            Self::Declined => f.write_str("Invite declined"),
            Self::Busy => f.write_str("Player is busy"),
            Self::PeerGone => f.write_str("peer left without answering (implicit decline)"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::HandshakeFailed(e) => write!(f, "Handshake failed: {}", e),
            Self::Io(m) => write!(f, "connection: {}", m),
        }
    }
}

/// An established network connection (invite accepted and handshake done).
///
/// The module returns the connection and the handshake result. The caller builds the game.
pub(crate) struct EstablishedConnection {
    pub(crate) conn: NetConnection,
    pub(crate) role: Role,
    pub(crate) handshake: HandshakeResult,
    /// Reconnection contact of the peer, derived from the socket and the handshake.
    pub(crate) net_info: OpponentNetInfo,
}

/// Result of an asynchronous connection attempt (run on a dedicated thread).
pub(crate) type ConnectResult = Result<EstablishedConnection, ConnectFailure>;

/// Result of reading an incoming invite on a thread: the connection (with the name already
/// read) and the announced rating, or an error (timeout, peer gone, invalid name).
pub(crate) type InviteReadResult = Result<(NetConnection, String, Option<i32>), NetError>;

/// An invite read in progress on a dedicated thread (see `pending_invite_reads`).
pub(crate) struct PendingInviteRead {
    rx: Receiver<InviteReadResult>,
    started: std::time::Instant,
}

/// Phase of the negotiation. Only one negotiation runs at a time, and the type enforces it.
pub(crate) enum NegotiationPhase {
    /// No negotiation in flight.
    Idle,
    /// An outgoing invite was sent (`ConnectToPeer`). Waiting for the reply and handshake.
    Inviting {
        rx: Receiver<ConnectResult>,
        cancel: Arc<AtomicBool>,
    },
    /// An incoming invite was accepted. Handshake in progress. Also carries `cancel`, so
    /// the accepting side can cancel too.
    Accepting {
        rx: Receiver<ConnectResult>,
        cancel: Arc<AtomicBool>,
    },
}

impl NegotiationPhase {
    pub(crate) fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// Cancel flag of the active phase (both active phases have one).
    fn cancel_flag(&self) -> Option<&Arc<AtomicBool>> {
        match self {
            Self::Idle => None,
            Self::Inviting { cancel, .. } | Self::Accepting { cancel, .. } => Some(cancel),
        }
    }

    /// Result channel of the active phase.
    fn result_rx(&self) -> Option<&Receiver<ConnectResult>> {
        match self {
            Self::Idle => None,
            Self::Inviting { rx, .. } | Self::Accepting { rx, .. } => Some(rx),
        }
    }
}

/// Full state of the network negotiation: zone services, incoming invites and the phase.
pub(crate) struct NegotiationState {
    /// Current negotiation phase (only one at a time).
    pub(crate) phase: NegotiationPhase,
    /// TCP listener for incoming LAN connections. Active in the whole VS Network zone
    /// (submenu, pages and LAN screens); see `sync_zone_services`.
    pub(crate) tcp_listener: Option<std::net::TcpListener>,
    /// LAN discovery service. Alive while we are in the network zone.
    pub(crate) discovery: Option<DiscoveryService>,
    /// mDNS announcements seen (available peers). Kept up to date while discovery runs,
    /// even without the LAN screen (used for the "online" indicator of opponents and for
    /// reconnection).
    pub(crate) net_peers: Vec<LanPeer>,
    /// Rating (`rt`) of the peer we are inviting. Moved to `opponent_announced_rt` in the
    /// session when the connection succeeds.
    pub(crate) pending_announced_rt: Option<i32>,
    /// Pending incoming connection waiting for user accept/decline.
    pub(crate) pending_invite: Option<NetConnection>,
    /// Received invite (session-level overlay, drawn above whatever screen is open).
    pub(crate) pending_invite_info: Option<IncomingInvite>,
    /// Incoming invite reads in progress. The name is read off the main thread, so a slow
    /// or silent peer never freezes the UI. Drained every tick by `poll_invite_reads`.
    pending_invite_reads: Vec<PendingInviteRead>,
    /// Last status sent to the mDNS service. Only `sync_announced_status` writes it.
    last_announced: Option<PeerStatus>,
}

impl NegotiationState {
    pub(crate) fn new() -> Self {
        Self {
            phase: NegotiationPhase::Idle,
            tcp_listener: None,
            discovery: None,
            net_peers: Vec::new(),
            pending_announced_rt: None,
            pending_invite: None,
            pending_invite_info: None,
            pending_invite_reads: Vec::new(),
            last_announced: None,
        }
    }

    /// Returns whether a negotiation is in progress: an active phase (invite sent or
    /// accepted) or an incoming invite waiting for a decision.
    pub(crate) fn in_progress(&self) -> bool {
        !self.phase.is_idle() || self.pending_invite.is_some()
    }

    /// Derives the mDNS status from the phase and the game state.
    ///
    /// `Accepting` announces `InGame` (the game is starting). `Inviting` and a pending
    /// incoming invite announce `Busy`. Otherwise the status is `Available`.
    pub(crate) fn announced_status(&self, in_game: bool) -> PeerStatus {
        if in_game {
            return PeerStatus::InGame;
        }
        match self.phase {
            NegotiationPhase::Accepting { .. } => PeerStatus::InGame,
            NegotiationPhase::Inviting { .. } => PeerStatus::Busy,
            NegotiationPhase::Idle if self.pending_invite.is_some() => PeerStatus::Busy,
            NegotiationPhase::Idle => PeerStatus::Available,
        }
    }

    /// Synchronizes the announced status. This is the only place that does it.
    ///
    /// Computes the derived status and sends an mDNS command only when it changes. Called
    /// once per tick.
    pub(crate) fn sync_announced_status(&mut self, in_game: bool) {
        let Some(ref discovery) = self.discovery else { return };
        let status = self.announced_status(in_game);
        if self.last_announced != Some(status) {
            discovery.send_command(DiscoveryCommand::UpdateStatus(status));
            self.last_announced = Some(status);
        }
    }

    /// Local TCP listen port (0 = no active listener). Announced in the handshake.
    pub(crate) fn local_listen_port(&self) -> u16 {
        self.tcp_listener.as_ref()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port())
            .unwrap_or(0)
    }

    /// Starts the zone services (TCP listener and mDNS discovery).
    ///
    /// Returns the bound port and the result of the mDNS start. Discovery can fail (network
    /// or OS) without making the screen unusable; the caller decides what to show.
    pub(crate) fn start_zone_services(
        &mut self,
        player_name: &str,
        announced_rating: Option<i32>,
        player_uuid_hex: &str,
        requested_port: Option<u16>,
    ) -> std::io::Result<(u16, Result<(), std::io::Error>)> {
        let (listener, port) = bind_zone_listener(requested_port)?;
        self.tcp_listener = Some(listener);
        let disco = match DiscoveryService::start(player_name, port, announced_rating, player_uuid_hex) {
            Ok(d) => {
                self.discovery = Some(d);
                // A freshly started service announces `Available`. Match that here to avoid
                // sending a redundant command.
                self.last_announced = Some(PeerStatus::Available);
                Ok(())
            }
            Err(e) => {
                self.discovery = None;
                Err(e)
            }
        };
        Ok((port, disco))
    }

    /// Stops the zone services (leaving the network zone).
    fn stop_zone_services(&mut self) {
        self.tcp_listener = None;
        self.discovery = None;
        self.net_peers.clear();
        self.last_announced = None;
    }

    /// Starts or stops the network services depending on whether we are in the VS Network
    /// zone.
    ///
    /// `in_zone` is computed by the caller (in a menu, not in a game). The LAN screen reuses
    /// these services instead of binding again. Nothing is stopped while a negotiation is in
    /// progress.
    pub(crate) fn sync_zone_services(
        &mut self,
        in_zone: bool,
        player_name: &str,
        announced_rating: Option<i32>,
        player_uuid_hex: &str,
        requested_port: Option<u16>,
    ) {
        if in_zone && self.tcp_listener.is_none() {
            match self.start_zone_services(player_name, announced_rating, player_uuid_hex, requested_port) {
                Ok((port, Ok(()))) => {
                    log::info!("Network zone: services started (TCP {port}, mDNS announced)");
                }
                Ok((_, Err(e))) => log::warn!("Network zone: discovery unavailable: {e}"),
                Err(e) => log::warn!("Network zone: cannot bind TCP: {e}"),
            }
        } else if !in_zone && self.tcp_listener.is_some() && !self.in_progress() {
            self.stop_zone_services();
            log::info!("Network zone: services stopped (left the zone)");
        }
    }

    /// Starts an outgoing invite: spawns the connection thread and enters `Inviting`.
    ///
    /// The thread checks the `cancel` flag between read timeouts. When it is set, the thread
    /// closes the socket, so the invited peer sees the close and leaves its modal.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_outgoing_invite(
        &mut self,
        host: String,
        port: u16,
        player_name: String,
        player_uuid: [u8; 16],
        our_rating: Option<i32>,
        seed: u32,
    ) {
        let our_listen_port = self.local_listen_port();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = cancel.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("network-connect".into())
            .spawn(move || {
                let result = connect_to_peer(
                    &host, port, &player_name, player_uuid, our_listen_port, our_rating,
                    seed, cancel_thread,
                );
                let _ = tx.send(result);
            })
            .expect("failed to spawn connection thread");
        self.phase = NegotiationPhase::Inviting { rx, cancel };
    }

    /// Accepts the pending incoming invite: sends `Accept`, spawns the handshake thread and
    /// enters `Accepting`.
    ///
    /// The overlay switches to its "connecting" state during the handshake (almost instant
    /// on a LAN).
    pub(crate) fn accept_pending_invite(
        &mut self,
        player_name: String,
        player_uuid: [u8; 16],
        seed: u32,
    ) {
        let Some(mut conn) = self.pending_invite.take() else { return };
        if let Err(e) = crackattack_net::communicator::send_invite_response(
            &mut conn, InviteResponse::Accept)
        {
            log::error!("Failed to send accept: {}", e);
            self.pending_invite_info = None;
            return;
        }
        let peer_name = self.pending_invite_info.as_ref()
            .map(|i| i.from_name.clone())
            .unwrap_or_else(|| "opponent".to_string());
        if let Some(ref info) = self.pending_invite_info {
            info.selection.set(0);
        }
        if let Some(info) = self.pending_invite_info.take() {
            self.pending_invite_info = Some(IncomingInvite { connecting: true, ..info });
        }
        // Rating of the inviter, if visible in discovery (matched by IP address).
        let inviter_ip = conn.remote_addr.ip();
        self.pending_announced_rt = self.net_peers.iter()
            .find(|p| p.addr == inviter_ip)
            .and_then(|p| p.announced_rating);
        let our_listen_port = self.local_listen_port();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = cancel.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("network-accept".into())
            .spawn(move || {
                let result = accept_peer_connection(
                    conn, &player_name, player_uuid, our_listen_port, seed, cancel_thread,
                );
                let _ = tx.send(result);
            })
            .expect("failed to spawn accept thread");
        self.phase = NegotiationPhase::Accepting { rx, cancel };
        log::info!("Invitation accepted from {}", peer_name);
    }

    /// Declines the pending incoming invite. The status goes back to `Available` through
    /// the derivation in `announced_status`.
    pub(crate) fn decline_pending_invite(&mut self) {
        self.pending_invite_info = None;
        if let Some(mut conn) = self.pending_invite.take() {
            let _ = crackattack_net::communicator::send_invite_response(
                &mut conn, InviteResponse::Decline);
            log::info!("Invitation declined");
        }
    }

    /// Cancels the active negotiation (invite sent, or acceptance in progress).
    ///
    /// Signals the thread through the phase's `cancel` flag, then goes back to `Idle`. The
    /// thread closes the socket, so the peer leaves too.
    pub(crate) fn cancel_active(&mut self) {
        if let Some(cancel) = self.phase.cancel_flag() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.phase = NegotiationPhase::Idle;
    }

    /// Accepts incoming TCP connections (non-blocking) and reads the invite name on a
    /// dedicated thread.
    ///
    /// The main thread never reads, so a slow or silent peer (or a port scan) cannot freeze
    /// the UI.
    pub(crate) fn poll_tcp_listener(&mut self) {
        if let Some(ref listener) = self.tcp_listener {
            match listener.accept() {
                Ok((stream, addr)) => {
                    log::info!("Incoming TCP connection from {}", addr);
                    stream.set_nodelay(true).ok();
                    // The accepted socket inherits the listener's non-blocking mode
                    // (macOS/BSD). If switching back to blocking fails, the invite read would
                    // fail in a confusing way, so at least log it.
                    if let Err(e) = stream.set_nonblocking(false) {
                        log::warn!("set_nonblocking(false) failed on accepted socket: {}", e);
                    }
                    let mut conn = NetConnection::from_stream(stream, addr);
                    let (tx, rx) = std::sync::mpsc::channel();
                    // If the spawn fails, drop the connection explicitly. Do not queue a
                    // `PendingInviteRead` with a dead sender: the `Disconnected` branch would
                    // drain it silently.
                    match std::thread::Builder::new()
                        .name("invite-read".into())
                        .spawn(move || {
                            let res = crackattack_net::communicator::recv_invite_name_timeout(
                                &mut conn,
                                std::time::Duration::from_secs(5),
                            )
                            .map(|(name, rating)| (conn, name, rating));
                            let _ = tx.send(res);
                        })
                    {
                        Ok(_) => {
                            self.pending_invite_reads.push(PendingInviteRead {
                                rx,
                                started: std::time::Instant::now(),
                            });
                        }
                        Err(e) => {
                            log::error!("failed to spawn invite-read thread — incoming connection from {} dropped: {}", addr, e);
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    log::debug!("TCP accept error: {}", e);
                }
            }
        }
    }

    /// Drains the threaded invite reads (see `poll_tcp_listener`) without ever blocking.
    ///
    /// If a negotiation is already in progress, the invite is auto-declined with `Busy`
    /// (checked at resolution time, so the state is fresh). Otherwise the invite overlay
    /// opens.
    ///
    /// `in_scope` tells whether we are still in the VS Network zone (computed by the caller).
    /// If not, the connection is closed without showing a modal out of context. `stored_rt`
    /// returns the last known rating for a uuid from the log (display fallback).
    pub(crate) fn poll_invite_reads(
        &mut self,
        in_scope: bool,
        stored_rt: &dyn Fn(&str) -> Option<i32>,
    ) {
        if self.pending_invite_reads.is_empty() {
            return;
        }
        let reads = std::mem::take(&mut self.pending_invite_reads);
        let mut still = Vec::new();
        for read in reads {
            match read.rx.try_recv() {
                Ok(Ok((mut conn, from_name, invite_rating))) => {
                    // Scope guard: the name read is asynchronous, so an invite can resolve
                    // just after we left the VS Network zone (or started a game). Close the
                    // connection and show no modal out of context.
                    if !in_scope {
                        log::info!("Invite from {} dropped: outside the VS Network zone", from_name);
                        continue; // `conn` is dropped, which closes the socket.
                    }
                    // Recompute `Busy` now, from fresh state rather than at accept time.
                    if self.in_progress() {
                        log::info!("Auto-refusing invite from {} (busy)", from_name);
                        let _ = crackattack_net::communicator::send_invite_response(
                            &mut conn, InviteResponse::Busy,
                        );
                        continue; // `conn` is dropped, which closes the socket.
                    }
                    log::info!("Invite received from {} (announced rating {:?})", from_name, invite_rating);
                    // Inviter's network rating: sent inside the invite (reliable, no mDNS
                    // needed). Fall back to fresh mDNS data (by IP), then to the last known
                    // rating in the log.
                    let inviter_peer = self.net_peers.iter()
                        .find(|p| p.addr == conn.remote_addr.ip());
                    let from_rating = invite_rating
                        .or_else(|| inviter_peer.and_then(|p| p.announced_rating))
                        .or_else(|| inviter_peer
                            .and_then(|p| p.uuid_hex.as_deref())
                            .and_then(|hex| stored_rt(hex)));
                    self.pending_invite_info = Some(IncomingInvite {
                        from_name,
                        from_rating,
                        connecting: false,
                        selection: std::cell::Cell::new(0),
                    });
                    // A pending invite makes us busy: the derived status becomes `Busy` at
                    // the next synchronization.
                    self.pending_invite = Some(conn);
                }
                Ok(Err(e)) => {
                    log::warn!("Failed to read invite name: {}", e);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Not ready yet. Keep it until it expires, in case the thread stalls.
                    if read.started.elapsed() < std::time::Duration::from_secs(8) {
                        still.push(read);
                    } else {
                        log::warn!("Incoming invite read timed out, dropping");
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The thread ended without a usable result (already logged): drop it.
                }
            }
        }
        self.pending_invite_reads = still;
    }

    /// Invited side: detects that the inviter closed the connection while we look at the
    /// invite modal.
    ///
    /// The check is a non-blocking peek. On detection we leave the wait and go back to the
    /// list, available again (derived status).
    pub(crate) fn poll_invite_gone(&mut self) {
        // Only while we are being asked (overlay not in the connecting state). The inviter
        // can vanish (cancel or crash); remove the overlay silently.
        let asking = self.pending_invite_info.as_ref().is_some_and(|i| !i.connecting);
        if !asking {
            return;
        }
        let gone = self.pending_invite.as_mut().map_or(false, |c| c.peer_closed());
        if gone {
            log::info!("Inviter disappeared (socket closed) — leaving invite");
            self.pending_invite = None;
            self.pending_invite_info = None;
        }
    }

    /// Drains mDNS events into `net_peers`. Returns `true` if the list changed (the caller
    /// mirrors it to the LAN screen if it is open).
    pub(crate) fn poll_discovery_events(&mut self) -> bool {
        let Some(ref discovery) = self.discovery else { return false };
        use crackattack_net::discovery::DiscoveryEvent;
        let mut changed = false;
        for event in discovery.poll_events() {
            changed = true;
            let peers = &mut self.net_peers;
            match event {
                DiscoveryEvent::PeerUpdated(peer) => {
                    // The list holds only available peers. A busy or in-game peer is removed,
                    // and added again when it becomes available.
                    if peer.status != PeerStatus::Available {
                        peers.retain(|p| p.session_id != peer.session_id);
                    } else if let Some(existing) = peers.iter_mut().find(|p| p.session_id == peer.session_id) {
                        existing.name = peer.name;
                        existing.addr = peer.addr;
                        existing.port = peer.port;
                        existing.status = peer.status;
                        existing.announced_rating = peer.announced_rating;
                        existing.uuid_hex = peer.uuid_hex;
                    } else {
                        peers.push(LanPeer {
                            name: peer.name,
                            addr: peer.addr,
                            port: peer.port,
                            session_id: peer.session_id,
                            status: peer.status,
                            announced_rating: peer.announced_rating,
                            uuid_hex: peer.uuid_hex,
                        });
                    }
                }
                DiscoveryEvent::PeerExpired(session_id) => {
                    peers.retain(|p| p.session_id != session_id);
                }
            }
        }
        changed
    }

    /// Polls the result of the negotiation in flight (non-blocking).
    ///
    /// `Some(..)` means the phase is resolved and back to `Idle`; the caller then builds the
    /// game or handles the failure.
    pub(crate) fn poll_connection(&mut self) -> Option<ConnectResult> {
        let result = match self.phase.result_rx()?.try_recv() {
            Ok(res) => Some(res),
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                log::error!("Connection thread crashed");
                None
            }
        };
        self.phase = NegotiationPhase::Idle;
        result
    }
}

/// Binds the zone listener.
///
/// Uses exactly the requested port (CLI `--vs network-create`) if given, otherwise the
/// dynamic range. Returns `(listener, port)`.
pub(crate) fn bind_zone_listener(
    requested_port: Option<u16>,
) -> std::io::Result<(std::net::TcpListener, u16)> {
    use crackattack_net::connection::{bind_tcp_range, TCP_PORT_RANGE, TCP_PORT_START};
    match requested_port {
        Some(p) => bind_tcp_range(p, 1),
        None => bind_tcp_range(TCP_PORT_START, TCP_PORT_RANGE),
    }
}

/// How long to wait for the reply to an invite before giving up on a frozen or silent peer.
/// The invited player is a human, so the delay is generous.
const INVITE_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// Connects, runs the pre-handshake and the handshake with a discovered peer (client path,
/// dedicated thread).
#[allow(clippy::too_many_arguments)]
fn connect_to_peer(
    host: &str,
    port: u16,
    player_name: &str,
    player_uuid: [u8; 16],
    our_listen_port: u16,
    our_rating: Option<i32>,
    seed: u32,
    cancel: Arc<AtomicBool>,
) -> ConnectResult {
    log::info!("Connecting to peer at {}:{}...", host, port);
    let mut conn = NetConnection::connect(host, port)
        .map_err(|e| ConnectFailure::Io(e.to_string()))?;

    // Pre-handshake: send our name, then wait for Accept/Decline/Busy in a loop with read
    // timeouts. This lets us give up if we cancel (`cancel`) or if the other side stops
    // answering: a close or crash gives an immediate socket error, a frozen or silent peer
    // hits the global deadline.
    crackattack_net::communicator::send_invite_name(&mut conn, player_name, our_rating)
        .map_err(|e| ConnectFailure::Io(e.to_string()))?;
    let deadline = std::time::Instant::now() + INVITE_RESPONSE_TIMEOUT;
    let response = loop {
        if cancel.load(Ordering::Relaxed) {
            // Cancelled: return, which closes the socket (`conn` is dropped). The invited
            // peer sees the close and leaves its modal.
            return Err(ConnectFailure::Cancelled);
        }
        // A vanished peer (close or crash) or no answer is an implicit decline: `PeerGone`,
        // handled silently (back to the list, no error shown).
        match crackattack_net::communicator::try_recv_invite_response(&mut conn)
            .map_err(|e| {
                log::debug!("Invite response read failed: {}", e);
                ConnectFailure::PeerGone
            })?
        {
            Some(r) => break r,
            None => {
                if std::time::Instant::now() > deadline {
                    return Err(ConnectFailure::PeerGone);
                }
            }
        }
    };
    match response {
        InviteResponse::Accept => {}
        // Decline and busy are distinct variants (the UI shows only `Busy`).
        InviteResponse::Decline => return Err(ConnectFailure::Declined),
        InviteResponse::Busy => return Err(ConnectFailure::Busy),
    }
    log::info!("Invitation accepted, proceeding with handshake");

    let hs = crackattack_net::communicator::perform_handshake(
        &mut conn, Role::Client, player_name, player_uuid, our_listen_port, seed,
    ).map_err(ConnectFailure::HandshakeFailed)?;

    log::info!("Connected to {} (seed: {})", hs.opponent_name, hs.seed);
    let net_info = OpponentNetInfo {
        // `to_canonical`: the dual-stack listener sees IPv4 peers as mapped IPv6
        // ("::ffff:127.0.0.1"). Store the canonical form, which is readable and reconnectable.
        addr: conn.remote_addr.ip().to_canonical().to_string(),
        listen_port: hs.opponent_listen_port,
    };
    Ok(EstablishedConnection { conn, role: Role::Client, handshake: hs, net_info })
}

/// Runs the handshake on an accepted incoming connection (server path, dedicated thread).
fn accept_peer_connection(
    mut conn: NetConnection,
    player_name: &str,
    player_uuid: [u8; 16],
    our_listen_port: u16,
    seed: u32,
    cancel: Arc<AtomicBool>,
) -> ConnectResult {
    // `Accepting` carries a cancel flag like `Inviting`. It is checked before the handshake
    // (each handshake step is bounded by a timeout in `crackattack-net` anyway).
    if cancel.load(Ordering::Relaxed) {
        return Err(ConnectFailure::Cancelled);
    }
    log::info!("Accepting peer connection from {}...", conn.remote_addr);
    let hs = crackattack_net::communicator::perform_handshake(
        &mut conn, Role::Server, player_name, player_uuid, our_listen_port, seed,
    ).map_err(ConnectFailure::HandshakeFailed)?;

    log::info!("Connected to {} (seed: {})", hs.opponent_name, hs.seed);
    let net_info = OpponentNetInfo {
        // `to_canonical`: the dual-stack listener sees IPv4 peers as mapped IPv6
        // ("::ffff:127.0.0.1"). Store the canonical form, which is readable and reconnectable.
        addr: conn.remote_addr.ip().to_canonical().to_string(),
        listen_port: hs.opponent_listen_port,
    };
    Ok(EstablishedConnection { conn, role: Role::Server, handshake: hs, net_info })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mDNS status is a pure function of the phase and the game state.
    #[test]
    fn announced_status_derives_from_phase() {
        let mut neg = NegotiationState::new();
        assert_eq!(neg.announced_status(false), PeerStatus::Available);
        assert_eq!(neg.announced_status(true), PeerStatus::InGame);

        // Outgoing invite in flight: busy.
        let (tx, rx) = std::sync::mpsc::channel::<ConnectResult>();
        let _ = tx; // no thread: only the derivation is tested
        neg.phase = NegotiationPhase::Inviting { rx, cancel: Arc::new(AtomicBool::new(false)) };
        assert_eq!(neg.announced_status(false), PeerStatus::Busy);

        let (tx2, rx2) = std::sync::mpsc::channel::<ConnectResult>();
        let _ = tx2;
        neg.phase = NegotiationPhase::Accepting { rx: rx2, cancel: Arc::new(AtomicBool::new(false)) };
        assert_eq!(neg.announced_status(false), PeerStatus::InGame);
    }

    /// Both active phases carry the cancel flag. `cancel_active` sets it and goes back to
    /// `Idle`, whichever side started the negotiation.
    #[test]
    fn cancel_works_in_both_active_phases() {
        for accepting in [false, true] {
            let mut neg = NegotiationState::new();
            let (_tx, rx) = std::sync::mpsc::channel::<ConnectResult>();
            let cancel = Arc::new(AtomicBool::new(false));
            neg.phase = if accepting {
                NegotiationPhase::Accepting { rx, cancel: cancel.clone() }
            } else {
                NegotiationPhase::Inviting { rx, cancel: cancel.clone() }
            };
            assert!(neg.in_progress());
            neg.cancel_active();
            assert!(cancel.load(Ordering::Relaxed), "the flag must be set (accepting={accepting})");
            assert!(neg.phase.is_idle());
        }
    }

    /// The predicate counts the active phase and the pending incoming connection, not the
    /// information overlay alone.
    #[test]
    fn in_progress_ignores_overlay_without_connection() {
        let mut neg = NegotiationState::new();
        assert!(!neg.in_progress());
        neg.pending_invite_info = Some(IncomingInvite {
            from_name: "X".into(),
            from_rating: None,
            connecting: false,
            selection: std::cell::Cell::new(0),
        });
        // The overlay info alone does not count. A `NetConnection` cannot be built without
        // a socket, so only the negative case is checked.
        assert!(!neg.in_progress());
    }

    /// Silent and shown failures are partitioned correctly, and `Display` messages are
    /// stable (only "busy" is shown on screen).
    #[test]
    fn connect_failure_display_and_silence() {
        assert!(ConnectFailure::Declined.is_silent());
        assert!(ConnectFailure::PeerGone.is_silent());
        assert!(ConnectFailure::Cancelled.is_silent());
        assert!(!ConnectFailure::Busy.is_silent());
        assert!(!ConnectFailure::Io("x".into()).is_silent());
        assert_eq!(ConnectFailure::Busy.to_string(), "Player is busy");
        assert_eq!(ConnectFailure::Declined.to_string(), "Invite declined");
        assert_eq!(
            ConnectFailure::HandshakeFailed(NetError::Io("boom".into())).to_string(),
            "Handshake failed: boom",
        );
        // `Io` carries the raw detail. `Display` (logs) adds a prefix, while the screen goes
        // through `to_lan_error()` and the locale.
        assert_eq!(ConnectFailure::Io("x".into()).to_string(), "connection: x");
        assert!(matches!(
            ConnectFailure::Io("x".into()).to_lan_error(),
            crate::app::meta_state::LanError::Connection(ref d) if d == "x"
        ));
        assert!(matches!(
            ConnectFailure::Busy.to_lan_error(),
            crate::app::meta_state::LanError::PeerBusy
        ));
        // A typed version mismatch gets a dedicated player-facing message. Other handshake
        // errors keep the technical detail.
        assert!(matches!(
            ConnectFailure::HandshakeFailed(NetError::VersionMismatch { local: 2, remote: 3 })
                .to_lan_error(),
            crate::app::meta_state::LanError::VersionMismatch
        ));
        assert!(matches!(
            ConnectFailure::HandshakeFailed(NetError::Protocol("bad".into())).to_lan_error(),
            crate::app::meta_state::LanError::Handshake(ref d) if d == "bad"
        ));
    }
}
