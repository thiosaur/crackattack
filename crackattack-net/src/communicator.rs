//! Communicator — protocol logic over a NetConnection.
//!
//! Handles handshake, seed exchange, and framed message I/O.
//! Two planes: data (gameplay garbage, batched) and control (immediate).

use crate::connection::{NetConnection, Role, RecvResult, RecvSome};
use crate::error::NetError;
use crate::protocol::*;
use std::time::{Duration, Instant};

/// Lag detection thresholds (in communication cycles).
pub(crate) const LAG_PAUSE_CYCLES: i32 = 2;
pub(crate) const DISCONNECT_CYCLES: i32 = 10;

/// Limit for each handshake read. A peer that accepts the connection and then goes silent
/// makes the handshake fail instead of blocking the thread forever.
const HANDSHAKE_STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Result of the handshake phase.
pub struct HandshakeResult {
    pub opponent_name: String,
    /// Persistent identity of the peer (carried by the handshake), used for the network ranking.
    pub opponent_uuid: [u8; 16],
    /// Listen port announced by the peer, used for quick reconnection. 0 = not listening.
    pub opponent_listen_port: u16,
    pub seed: u32,
}

/// Perform the full handshake sequence.
/// Server sends seed, client receives it.
///
/// Each read is bounded by [`HANDSHAKE_STEP_TIMEOUT`]. On failure the calling thread (and
/// the session's negotiation phase) is released instead of hanging forever.
///
/// Errors are **typed** ([`NetError`]). Their `Display` text is the plain error message.
pub fn perform_handshake(
    conn: &mut NetConnection,
    role: Role,
    player_name: &str,
    player_uuid: [u8; 16],
    listen_port: u16,
    seed: u32,
) -> Result<HandshakeResult, NetError> {
    perform_handshake_with_timeout(conn, role, player_name, player_uuid, listen_port, seed,
        HANDSHAKE_STEP_TIMEOUT)
}

/// Same as [`perform_handshake`] with a configurable timeout (testable with a short delay).
pub(crate) fn perform_handshake_with_timeout(
    conn: &mut NetConnection,
    role: Role,
    player_name: &str,
    player_uuid: [u8; 16],
    listen_port: u16,
    seed: u32,
    step_timeout: Duration,
) -> Result<HandshakeResult, NetError> {
    let our_packet = HandshakePacket::new(player_name, player_uuid, listen_port);
    conn.send_all(&our_packet.to_bytes())
        .map_err(|e| NetError::Io(format!("failed to send handshake: {}", e)))?;

    let to_net_error = |e: ProtocolError| match e {
        // A version mismatch is typed (the UI tells the player to update). Other errors stay textual.
        ProtocolError::VersionMismatch { local, remote } => NetError::VersionMismatch { local, remote },
        e => NetError::Protocol(format!("handshake error: {}", e)),
    };
    let mut buf = vec![0u8; 256];
    // Head first (magic + version): another version may send a preamble of another length.
    match conn.recv_exact_blocking_timeout(&mut buf[..HANDSHAKE_HEAD_LEN], step_timeout) {
        Ok(()) => {}
        Err(e) => return Err(NetError::from_io("failed to receive handshake", &e)),
    }
    HandshakePacket::check_head(&buf[..HANDSHAKE_HEAD_LEN]).map_err(to_net_error)?;
    match conn.recv_exact_blocking_timeout(&mut buf[HANDSHAKE_HEAD_LEN..HANDSHAKE_PREAMBLE_LEN], step_timeout) {
        Ok(()) => {}
        Err(e) => return Err(NetError::from_io("failed to receive handshake", &e)),
    }
    let name_len = buf[HANDSHAKE_PREAMBLE_LEN - 1] as usize;
    // `name_len` is an untrusted network byte. Without this guard, the slice
    // `buf[..27 + name_len]` would overrun the 256-byte buffer (panic from name_len >= 230).
    // Reject it cleanly, as `recv_invite_name_timeout` does.
    if name_len > MAX_PLAYER_NAME_LEN {
        return Err(NetError::Protocol(format!(
            "handshake error: player name too long ({} > {})",
            name_len, MAX_PLAYER_NAME_LEN
        )));
    }
    if name_len > 0 {
        match conn.recv_exact_blocking_timeout(
            &mut buf[HANDSHAKE_PREAMBLE_LEN..HANDSHAKE_PREAMBLE_LEN + name_len], step_timeout)
        {
            Ok(()) => {}
            Err(e) => return Err(NetError::from_io("failed to receive player name", &e)),
        }
    }
    let (opponent_packet, _) = HandshakePacket::from_bytes(&buf[..HANDSHAKE_PREAMBLE_LEN + name_len])
        .map_err(to_net_error)?;

    log::info!(
        "Handshake OK — opponent: {} (uuid {})",
        opponent_packet.player_name,
        uuid_hex(&opponent_packet.player_uuid),
    );

    let game_seed = match role {
        Role::Server => {
            let pkt = SeedPacket { seed };
            conn.send_all(&pkt.to_bytes())
                .map_err(|e| NetError::Io(format!("failed to send seed: {}", e)))?;
            seed
        }
        Role::Client => {
            let mut seed_buf = [0u8; 4];
            conn.recv_exact_blocking_timeout(&mut seed_buf, step_timeout)
                .map_err(|e| NetError::from_io("failed to receive seed", &e))?;
            SeedPacket::from_bytes(&seed_buf).seed
        }
    };

    log::info!("Game seed: {}", game_seed);

    Ok(HandshakeResult {
        opponent_name: opponent_packet.player_name,
        opponent_uuid: opponent_packet.player_uuid,
        opponent_listen_port: opponent_packet.listen_port,
        seed: game_seed,
    })
}

/// Response byte values for the invitation pre-handshake.
pub(crate) const INVITE_ACCEPT: u8 = 0x01;
pub(crate) const INVITE_DECLINE: u8 = 0x00;
/// The invitee is already busy with another negotiation: automatic refusal, distinct from
/// a refusal chosen by the player.
pub(crate) const INVITE_BUSY: u8 = 0x02;

/// Response to an invite. `Busy` is an automatic refusal because the invitee is already
/// negotiating. It differs from `Decline`, the player's explicit refusal, so the sender can
/// show that the invitee is busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteResponse {
    Accept,
    Decline,
    Busy,
}

impl InviteResponse {
    fn as_byte(self) -> u8 {
        match self {
            Self::Accept => INVITE_ACCEPT,
            Self::Decline => INVITE_DECLINE,
            Self::Busy => INVITE_BUSY,
        }
    }
    fn from_byte(b: u8) -> Self {
        match b {
            INVITE_ACCEPT => Self::Accept,
            INVITE_BUSY => Self::Busy,
            _ => Self::Decline,
        }
    }
}

/// Sends our invite identity (initiator side), BEFORE the full handshake.
///
/// The identity is the player name plus the self-announced network rating (0 = unplaced or
/// unknown). The rating travels WITH the invite, so the invitee can show it without mDNS
/// (which makes it reliable for manual connections).
/// Wire format: `name_len (1) | name (<= 32) | rating (2 LE, u16; 0 = absent)`.
pub fn send_invite_name(conn: &mut NetConnection, name: &str, rating: Option<i32>) -> Result<(), NetError> {
    // Truncate on a character boundary so the peer never sees a `\u{FFFD}` replacement character.
    let name_bytes = crate::protocol::utf8_prefix(name, MAX_PLAYER_NAME_LEN);
    // Rating is clamped to u16 (the public scale, around 1500, fits easily). 0 means absent.
    let rt: u16 = rating.map(|r| r.clamp(1, u16::MAX as i32) as u16).unwrap_or(0);
    let mut buf = Vec::with_capacity(3 + name_bytes.len());
    buf.push(name_bytes.len() as u8);
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(&rt.to_le_bytes());
    conn.send_all(&buf)
        .map_err(|e| NetError::Io(format!("failed to send invite name: {}", e)))
}

/// Receives the inviter's identity (receiver side): `(name, rating)`, where rating `None`
/// means 0 on the wire.
///
/// Each read is bounded by `timeout` (via `recv_exact_blocking_timeout`). Call it on a
/// **dedicated thread**: a peer that connects and sends nothing makes the thread fail after
/// `timeout` instead of blocking it forever. The main thread never reads (see
/// `poll_tcp_listener`).
pub fn recv_invite_name_timeout(
    conn: &mut NetConnection,
    timeout: std::time::Duration,
) -> Result<(String, Option<i32>), NetError> {
    let mut len_buf = [0u8; 1];
    conn.recv_exact_blocking_timeout(&mut len_buf, timeout)
        .map_err(|e| NetError::from_io("failed to receive invite name length", &e))?;
    let len = len_buf[0] as usize;
    if len == 0 || len > MAX_PLAYER_NAME_LEN {
        return Err(NetError::Protocol("invalid invite name length".into()));
    }
    let mut name_buf = vec![0u8; len];
    conn.recv_exact_blocking_timeout(&mut name_buf, timeout)
        .map_err(|e| NetError::from_io("failed to receive invite name", &e))?;
    let mut rt_buf = [0u8; 2];
    conn.recv_exact_blocking_timeout(&mut rt_buf, timeout)
        .map_err(|e| NetError::from_io("failed to receive invite rating", &e))?;
    let rt = u16::from_le_bytes(rt_buf);
    let rating = if rt == 0 { None } else { Some(rt as i32) };
    Ok((String::from_utf8_lossy(&name_buf).to_string(), rating))
}

/// Sends the invite response (receiver side): Accept, Decline or Busy.
pub fn send_invite_response(conn: &mut NetConnection, response: InviteResponse) -> Result<(), NetError> {
    conn.send_all(&[response.as_byte()])
        .map_err(|e| NetError::Io(format!("failed to send invite response: {}", e)))
}

/// Tries to receive the invite response, waiting about `RECV_TIMEOUT` at most.
///
/// Returns `Some(response)` when received, `None` on timeout (nothing yet), and `Err` if the
/// connection is closed or fails. This lets the initiator wait for the answer while still
/// being able to cancel or notice that the other side is gone.
pub fn try_recv_invite_response(conn: &mut NetConnection) -> Result<Option<InviteResponse>, NetError> {
    let mut buf = [0u8; 1];
    match conn.recv_exact(&mut buf) {
        RecvResult::Ok => Ok(Some(InviteResponse::from_byte(buf[0]))),
        RecvResult::Timeout => Ok(None),
        RecvResult::Disconnected(e) => Err(NetError::Io(format!("invite response failed: {}", e))),
    }
}

/// Received message from the wire (after framing dispatch).
pub(crate) enum ReceivedMessage {
    Data(GameplayBuffer),
    Control(ControlMessage),
}

/// Send a gameplay buffer (data plane) with type prefix.
pub(crate) fn send_gameplay(conn: &mut NetConnection, buf: &GameplayBuffer) -> Result<(), String> {
    let payload = buf.to_bytes();
    let mut frame = Vec::with_capacity(1 + payload.len());
    frame.push(MSG_TYPE_DATA);
    frame.extend_from_slice(&payload);
    conn.send_all(&frame)
        .map_err(|e| format!("failed to send gameplay buffer: {}", e))
}

/// Send a control message (control plane) with type prefix. Immediate.
pub(crate) fn send_control(conn: &mut NetConnection, msg: &ControlMessage) -> Result<(), String> {
    let payload = msg.to_bytes();
    let mut frame = Vec::with_capacity(1 + payload.len());
    frame.push(MSG_TYPE_CONTROL);
    frame.extend_from_slice(&payload);
    conn.send_all(&frame)
        .map_err(|e| format!("failed to send control message: {}", e))
}

/// Time limit to complete a frame that has **started**. A legitimate frame is at most
/// ~1 KB (an extension at `MAX_EXTENSION_LEN`) and is sent in a single `send_all` with TCP_NODELAY. If it is not complete
/// 5 s after its first byte, the peer is dead or hostile.
const FRAME_STALL_TIMEOUT: Duration = Duration::from_secs(5);

/// **Resumable** frame reader.
///
/// It keeps the bytes already received for the current frame (including the type byte)
/// between two polls. This gives three guarantees:
/// - A frame split by the network (a TCP segment boundary anywhere) completes on later
///   polls, without losing a byte or shifting the stream.
/// - No read loops while waiting for the rest of a field. Each poll returns within about
///   50 ms, so the network thread keeps serving its channels.
/// - A frame that has started and is not complete within [`FRAME_STALL_TIMEOUT`] counts as
///   a disconnection.
pub(crate) struct MessageReader {
    /// Bytes received for the current frame (type byte included, `frame[0]`).
    frame: Vec<u8>,
    /// Bytes required before the next decision point. It grows as sizes are learned:
    /// type, then header, then payload.
    needed: usize,
    /// Time of the first byte of the current frame (for the stall limit).
    started_at: Option<Instant>,
    stall_timeout: Duration,
}

impl Default for MessageReader {
    fn default() -> Self { Self::new() }
}

impl MessageReader {
    pub fn new() -> Self {
        Self::with_stall_timeout(FRAME_STALL_TIMEOUT)
    }

    /// Creates a reader with a configurable stall limit (testable with a short delay).
    pub(crate) fn with_stall_timeout(stall_timeout: Duration) -> Self {
        Self { frame: Vec::with_capacity(64), needed: 1, started_at: None, stall_timeout }
    }

    fn reset(&mut self) {
        self.frame.clear();
        self.needed = 1;
        self.started_at = None;
    }

    /// Tries to complete the current frame. It returns at most **one** message per call.
    /// `Ok(None)` means nothing is complete yet (state is kept). `Err` means a disconnection,
    /// a corrupt stream or a stalled frame.
    pub fn poll(&mut self, conn: &mut NetConnection) -> Result<Option<ReceivedMessage>, String> {
        loop {
            // 1. Fill in the missing bytes (one read attempt per shortfall).
            if self.frame.len() < self.needed {
                let want = (self.needed - self.frame.len()).min(512);
                let mut chunk = [0u8; 512];
                match conn.recv_some(&mut chunk[..want]) {
                    RecvSome::Read(n) => {
                        self.frame.extend_from_slice(&chunk[..n]);
                        self.started_at.get_or_insert_with(Instant::now);
                        continue;
                    }
                    RecvSome::Timeout => {
                        if self.frame.is_empty() {
                            return Ok(None); // idle, nothing in progress
                        }
                        if self.started_at.map(|t| t.elapsed() > self.stall_timeout).unwrap_or(false) {
                            return Err(format!(
                                "frame stalled: {}/{} bytes after {:.1}s",
                                self.frame.len(), self.needed,
                                self.stall_timeout.as_secs_f32(),
                            ));
                        }
                        return Ok(None); // the started frame is kept for the next poll
                    }
                    RecvSome::Disconnected(e) => return Err(format!("disconnected: {}", e)),
                }
            }

            // 2. Decision point: the frame has exactly `needed` bytes.
            match self.frame[0] {
                MSG_TYPE_DATA => {
                    // [type][count u8][pad ×3][entries count×16]
                    if self.needed == 1 {
                        self.needed = 1 + 4;
                        continue;
                    }
                    if self.needed == 5 {
                        let count = self.frame[1] as usize;
                        if count > MAX_GARBAGE_PER_CYCLE {
                            return Err(format!("garbage count {} exceeds max", count));
                        }
                        if count > 0 {
                            self.needed = 5 + count * 16;
                            continue;
                        }
                    }
                    let buf = GameplayBuffer::from_bytes(&self.frame[1..])
                        .map_err(|e| format!("parse error: {}", e))?;
                    self.reset();
                    return Ok(Some(ReceivedMessage::Data(buf)));
                }
                MSG_TYPE_CONTROL => {
                    // [type][ctrl_type u8][payload]
                    if self.needed == 1 {
                        self.needed = 1 + 1;
                        continue;
                    }
                    if self.needed == 2 {
                        // The size comes from the protocol's layout table, the single
                        // source shared with the `from_bytes` guards.
                        let payload_size = match ctrl_payload_size(self.frame[1]) {
                            Some(n) => n,
                            None => return Err(format!("unknown control type: 0x{:02X}", self.frame[1])),
                        };
                        if payload_size > 0 {
                            self.needed = 2 + payload_size;
                            continue;
                        }
                    }
                    // Extension: its header is in, the announced payload follows.
                    if self.frame[1] == CTRL_EXTENSION && self.needed == 2 + EXTENSION_HEADER_LEN {
                        let len = extension_len(&self.frame[2..]);
                        if len > MAX_EXTENSION_LEN {
                            return Err(format!("extension 0x{:02X} too long: {} bytes", self.frame[2], len));
                        }
                        if len > 0 {
                            self.needed += len;
                            continue;
                        }
                    }
                    let msg = ControlMessage::from_bytes(&self.frame[1..])
                        .map_err(|e| format!("control parse error: {}", e))?;
                    self.reset();
                    return Ok(Some(ReceivedMessage::Control(msg)));
                }
                other => return Err(format!("unknown message type: 0x{:02X}", other)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Loopback connection pair (client, server), or `None` in a sandbox without network.
    fn loopback_pair() -> Option<(NetConnection, NetConnection)> {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => {
                // With CI_NET=1 the loopback is required.
                if std::env::var_os("CI_NET").is_some() {
                    panic!("CI_NET=1: loopback bind required, failed: {e}");
                }
                eprintln!("Skipping net test (bind failed: {})", e);
                return None;
            }
        };
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || NetConnection::connect("127.0.0.1", port).unwrap());
        let (stream, addr) = listener.accept().unwrap();
        let server = NetConnection::from_stream(stream, addr);
        Some((client.join().unwrap(), server))
    }

    /// A control frame sent **byte by byte** with pauses longer than `RECV_TIMEOUT` (50 ms)
    /// must arrive intact. No byte (the type byte included) may be lost between two polls.
    #[test]
    fn fragmented_control_frame_converges() {
        let Some((mut sender, mut receiver)) = loopback_pair() else { return };
        let frame = {
            let mut f = vec![MSG_TYPE_CONTROL];
            f.extend_from_slice(&ControlMessage::Unpause { epoch: 7 }.to_bytes());
            f
        };
        let writer = std::thread::spawn(move || {
            for b in frame {
                sender.send_all(&[b]).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
            sender // keep the connection open until the end
        });

        let mut reader = MessageReader::new();
        let mut polls = 0;
        let msg = loop {
            polls += 1;
            assert!(polls < 100, "message never completed");
            match reader.poll(&mut receiver).expect("no error expected") {
                Some(m) => break m,
                None => {}
            }
        };
        match msg {
            ReceivedMessage::Control(ControlMessage::Unpause { epoch }) => assert_eq!(epoch, 7),
            _ => panic!("wrong message decoded from fragmented frame"),
        }
        let _ = writer.join();
    }

    /// Polls until a message arrives (or panics after 100 polls).
    fn next_message(reader: &mut MessageReader, conn: &mut NetConnection) -> ReceivedMessage {
        for _ in 0..100 {
            if let Some(m) = reader.poll(conn).expect("no error expected") {
                return m;
            }
        }
        panic!("message never completed");
    }

    /// An extension this version does not know, between two known messages, is read whole and
    /// skipped: the messages around it arrive intact.
    #[test]
    fn unknown_extension_between_known_messages_is_skipped() {
        let Some((mut sender, mut receiver)) = loopback_pair() else { return };
        let mut stream = vec![MSG_TYPE_CONTROL];
        stream.extend_from_slice(&ControlMessage::Ping(3).to_bytes());
        stream.extend_from_slice(&[MSG_TYPE_CONTROL, CTRL_EXTENSION, 0xEE, 5, 0, 1, 2, 3, 4, 5]);
        stream.push(MSG_TYPE_CONTROL);
        stream.extend_from_slice(&ControlMessage::Pong(4).to_bytes());
        sender.send_all(&stream).unwrap();

        let mut reader = MessageReader::new();
        assert!(matches!(next_message(&mut reader, &mut receiver), ReceivedMessage::Control(ControlMessage::Ping(3))));
        assert!(matches!(next_message(&mut reader, &mut receiver), ReceivedMessage::Control(ControlMessage::UnknownExtension(0xEE))));
        assert!(matches!(next_message(&mut reader, &mut receiver), ReceivedMessage::Control(ControlMessage::Pong(4))));
    }

    /// An extension announcing more than `MAX_EXTENSION_LEN` is a stream error, not a wait.
    #[test]
    fn oversized_extension_is_an_error() {
        let Some((mut sender, mut receiver)) = loopback_pair() else { return };
        let len = (MAX_EXTENSION_LEN as u16 + 1).to_le_bytes();
        sender.send_all(&[MSG_TYPE_CONTROL, CTRL_EXTENSION, 0xEE, len[0], len[1]]).unwrap();
        let mut reader = MessageReader::new();
        for _ in 0..100 {
            match reader.poll(&mut receiver) {
                Ok(None) => {}
                Ok(Some(_)) => panic!("no complete message was sent"),
                Err(e) => {
                    assert!(e.contains("too long"), "unexpected error: {e}");
                    return;
                }
            }
        }
        panic!("oversized extension never rejected");
    }

    /// The type byte alone, a pause, then the rest of the frame: the type byte must NOT be
    /// dropped on the timeout.
    #[test]
    fn type_byte_then_pause_is_not_lost() {
        let Some((mut sender, mut receiver)) = loopback_pair() else { return };
        let mut reader = MessageReader::new();

        sender.send_all(&[MSG_TYPE_CONTROL]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(120));
        // Two empty polls during the pause: the state must be kept.
        assert!(reader.poll(&mut receiver).unwrap().is_none());
        assert!(reader.poll(&mut receiver).unwrap().is_none());

        sender.send_all(&ControlMessage::Pause.to_bytes()).unwrap();
        let mut polls = 0;
        loop {
            polls += 1;
            assert!(polls < 100, "message never completed");
            if let Some(m) = reader.poll(&mut receiver).unwrap() {
                assert!(matches!(m, ReceivedMessage::Control(ControlMessage::Pause)));
                break;
            }
        }
    }

    /// A frame that starts and is never completed (a peer that goes silent with the socket
    /// open) must end in an **error** within the stall limit: no blocking, and no endless
    /// `Ok(None)`.
    #[test]
    fn mid_frame_stall_disconnects() {
        let Some((mut sender, mut receiver)) = loopback_pair() else { return };
        let mut reader = MessageReader::with_stall_timeout(std::time::Duration::from_millis(200));

        // 2 bytes of a control frame that needs 4 (Unpause = type + ctrl + 2-byte epoch)...
        sender.send_all(&[MSG_TYPE_CONTROL, 0x02]).unwrap();
        // ... then nothing more, socket left open.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            assert!(std::time::Instant::now() < deadline, "stall never detected");
            match reader.poll(&mut receiver) {
                Ok(None) => {}
                Ok(Some(_)) => panic!("no complete message was sent"),
                Err(e) => {
                    assert!(e.contains("stalled"), "unexpected error: {e}");
                    break;
                }
            }
        }
    }

    /// A peer that connects and then goes silent makes the handshake fail within the
    /// timeout, so the calling thread is released.
    #[test]
    fn handshake_with_mute_peer_times_out() {
        let Some((mut client, _server)) = loopback_pair() else { return };
        let start = std::time::Instant::now();
        let res = perform_handshake_with_timeout(
            &mut client, Role::Client, "X", [0u8; 16], 0, 42,
            std::time::Duration::from_millis(200),
        );
        assert!(
            matches!(res, Err(NetError::Timeout(_))),
            "handshake against a mute peer must end in a Timeout"
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(3), "timeout not honored");
    }

    /// A peer on another protocol version gives a typed `NetError::VersionMismatch` (not a
    /// textual `Protocol` error) carrying both versions. `Display` gives the plain message.
    #[test]
    fn handshake_version_mismatch_is_typed() {
        let Some((mut client, mut server)) = loopback_pair() else { return };
        // The "server" poses as a peer with version PROTOCOL_VERSION + 1.
        let mut pkt = HandshakePacket::new("Futur", [7u8; 16], 0);
        pkt.protocol_version = PROTOCOL_VERSION + 1;
        server.send_all(&pkt.to_bytes()).unwrap();
        let res = perform_handshake_with_timeout(
            &mut client, Role::Client, "X", [0u8; 16], 0, 42,
            std::time::Duration::from_secs(2),
        );
        let err = res.err().expect("version mismatch must fail the handshake");
        assert_eq!(err, NetError::VersionMismatch {
            local: PROTOCOL_VERSION, remote: PROTOCOL_VERSION + 1,
        });
        assert_eq!(err.to_string(), format!(
            "handshake error: protocol version mismatch: local={}, remote={}",
            PROTOCOL_VERSION, PROTOCOL_VERSION + 1,
        ));
    }

    /// A peer whose preamble is shorter than ours (here: only the head) is still reported as a
    /// version mismatch at once, instead of a receive timeout.
    #[test]
    fn version_mismatch_detected_from_the_head_alone() {
        let Some((mut client, mut server)) = loopback_pair() else { return };
        let mut head = HANDSHAKE_MAGIC.to_le_bytes().to_vec();
        head.extend_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
        server.send_all(&head).unwrap();
        let started = std::time::Instant::now();
        let res = perform_handshake_with_timeout(
            &mut client, Role::Client, "X", [0u8; 16], 0, 42,
            std::time::Duration::from_secs(2),
        );
        assert_eq!(res.err(), Some(NetError::VersionMismatch {
            local: PROTOCOL_VERSION, remote: PROTOCOL_VERSION + 1,
        }));
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "no wait for the rest of the preamble");
    }

    #[test]
    fn invite_response_byte_round_trip() {
        // Accept, Decline and Busy are distinct and stable on the wire (1 byte).
        for r in [InviteResponse::Accept, InviteResponse::Decline, InviteResponse::Busy] {
            assert_eq!(InviteResponse::from_byte(r.as_byte()), r);
        }
        assert_eq!(InviteResponse::Accept.as_byte(), INVITE_ACCEPT);
        assert_eq!(InviteResponse::Decline.as_byte(), INVITE_DECLINE);
        assert_eq!(InviteResponse::Busy.as_byte(), INVITE_BUSY);
        // Unknown byte: safe fallback to Decline (never Accept by default).
        assert_eq!(InviteResponse::from_byte(0xFF), InviteResponse::Decline);
    }
}
