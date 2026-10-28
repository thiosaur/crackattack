//! Network protocol types and constants.
//!
//! Defines the wire format for Crack Attack! multiplayer.
//! Two planes: data (garbage batches) and control (immediate signals).

use bytemuck::{Pod, Zeroable};
use crackattack_core::vs::OpponentBoardView;

/// Communication period in game ticks.
/// Gameplay garbage is exchanged every this many frames.
pub(crate) const COMMUNICATION_PERIOD: i32 = 32;

/// Multiplayer compatibility number, compared in the handshake and announced over mDNS. Two
/// versions play together only if it is equal. Increment it in any release that changes the
/// wire format (handshake, invite, messages) OR anything that makes two versions play a different
/// game: rules, garbage, creep, communication period, round arbitration. The handshake carries
/// the player identity (a 16-byte uuid).
///
/// From version 4, a control message may be an EXTENSION (`CTRL_EXTENSION`): it carries its own
/// length, and a peer skips an extension type it does not know. An extension carries only what
/// can be lost without changing the game (display, comfort): the opponent board preview, whether
/// a player shares it. Anything about rules, garbage, synchronisation or arbitration is a
/// fixed-size message and needs a new version.
pub(crate) const PROTOCOL_VERSION: u32 = 4;

/// Length of the handshake head (magic + version). It is read and checked before the rest of
/// the preamble, so a version mismatch is always reported as such, whatever the length of the
/// other version's preamble.
pub(crate) const HANDSHAKE_HEAD_LEN: usize = 8;

/// Handshake magic number ("CRAK" in ASCII).
pub(crate) const HANDSHAKE_MAGIC: u32 = 0x4352414B;

/// Maximum player name length (bytes, UTF-8).
pub(crate) const MAX_PLAYER_NAME_LEN: usize = 32;

/// Size of the fixed handshake preamble:
/// magic (4) + version (4) + uuid (16) + listen_port (2) + name_len (1).
pub(crate) const HANDSHAKE_PREAMBLE_LEN: usize = 27;

/// Maximum garbage packets per communication cycle.
pub(crate) const MAX_GARBAGE_PER_CYCLE: usize = 32;

/// Type byte prefix for data plane messages (gameplay garbage).
pub(crate) const MSG_TYPE_DATA: u8 = 0x01;

/// Type byte prefix for control plane messages (immediate signals).
pub(crate) const MSG_TYPE_CONTROL: u8 = 0x02;

/// Sent at connection start by both peers.
/// Fields on the wire: magic (4 bytes, little-endian), protocol version (4, LE),
/// player uuid (16), listen port (2, LE), name length (1), then the name
/// (at most 32 bytes).
#[derive(Debug, Clone)]
pub(crate) struct HandshakePacket {
    pub magic: u32,
    pub(crate) protocol_version: u32,
    /// Persistent player identity (16 bytes, generated locally).
    pub player_uuid: [u8; 16],
    /// Local TCP LISTEN port, used for fast reconnection. The peer's source port
    /// is ephemeral, so this announces where we can be contacted. 0 = not listening.
    pub listen_port: u16,
    pub player_name: String,
}

impl HandshakePacket {
    pub(crate) fn new(player_name: &str, player_uuid: [u8; 16], listen_port: u16) -> Self {
        Self {
            magic: HANDSHAKE_MAGIC,
            protocol_version: PROTOCOL_VERSION,
            player_uuid,
            listen_port,
            player_name: player_name.to_string(),
        }
    }

    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let name_bytes = utf8_prefix(&self.player_name, MAX_PLAYER_NAME_LEN);
        let mut buf = Vec::with_capacity(HANDSHAKE_PREAMBLE_LEN + name_bytes.len());
        buf.extend_from_slice(&self.magic.to_le_bytes());
        buf.extend_from_slice(&self.protocol_version.to_le_bytes());
        buf.extend_from_slice(&self.player_uuid);
        buf.extend_from_slice(&self.listen_port.to_le_bytes());
        buf.push(name_bytes.len() as u8);
        buf.extend_from_slice(name_bytes);
        buf
    }

    /// Checks the head (magic + version), the first `HANDSHAKE_HEAD_LEN` bytes.
    pub(crate) fn check_head(data: &[u8]) -> Result<u32, ProtocolError> {
        if data.len() < HANDSHAKE_HEAD_LEN {
            return Err(ProtocolError::IncompletePreamble);
        }
        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if magic != HANDSHAKE_MAGIC {
            return Err(ProtocolError::BadMagic(magic));
        }
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::VersionMismatch { local: PROTOCOL_VERSION, remote: version });
        }
        Ok(version)
    }

    pub(crate) fn from_bytes(data: &[u8]) -> Result<(Self, usize), ProtocolError> {
        let version = Self::check_head(data)?;
        if data.len() < HANDSHAKE_PREAMBLE_LEN {
            return Err(ProtocolError::IncompletePreamble);
        }
        let magic = HANDSHAKE_MAGIC;

        let mut player_uuid = [0u8; 16];
        player_uuid.copy_from_slice(&data[8..24]);
        let listen_port = u16::from_le_bytes([data[24], data[25]]);
        let name_len = data[26] as usize;
        if data.len() < HANDSHAKE_PREAMBLE_LEN + name_len {
            return Err(ProtocolError::IncompletePreamble);
        }

        let name = String::from_utf8_lossy(&data[HANDSHAKE_PREAMBLE_LEN..HANDSHAKE_PREAMBLE_LEN + name_len]).to_string();
        Ok((Self { magic, protocol_version: version, player_uuid, listen_port, player_name: name }, HANDSHAKE_PREAMBLE_LEN + name_len))
    }
}

/// Formats a player uuid as 32 hex characters (for logs and the match journal).
pub(crate) fn uuid_hex(uuid: &[u8; 16]) -> String {
    uuid.iter().map(|b| format!("{b:02x}")).collect()
}

/// Returns the longest prefix of `s` that fits in `max` bytes **on a character boundary**.
/// Cutting inside a multi-byte character would show a `�` on the peer.
pub(crate) fn utf8_prefix(s: &str, max: usize) -> &[u8] {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s.as_bytes()[..end]
}

/// Single garbage packet.
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub(crate) struct GarbageEntry {
    pub height: i32,
    pub width: i32,
    pub flavor: i32,
    pub timestamp: i32,
}

/// Data plane buffer: garbage only.
///
/// Wire: [MSG_TYPE_DATA] [garbage_count: u8] [entries: count * 16 bytes]
#[derive(Debug, Clone)]
pub(crate) struct GameplayBuffer {
    pub garbage: Vec<GarbageEntry>,
}

impl GameplayBuffer {
    pub fn new() -> Self {
        Self { garbage: Vec::new() }
    }

    /// Serialize to bytes (WITHOUT the type prefix — caller adds it).
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let count = self.garbage.len().min(MAX_GARBAGE_PER_CYCLE) as u8;
        // 1 byte count + 3 bytes padding for alignment + count * 16 bytes of entries
        let mut buf = Vec::with_capacity(4 + count as usize * 16);
        buf.push(count);
        buf.extend_from_slice(&[0u8; 3]); // padding for 4-byte alignment
        for entry in self.garbage.iter().take(MAX_GARBAGE_PER_CYCLE) {
            buf.extend_from_slice(bytemuck::bytes_of(entry));
        }
        buf
    }

    /// Deserialize from bytes (WITHOUT the type prefix).
    ///
    /// The fields of each entry are **validated at the network boundary**. A hostile
    /// peer cannot inject out-of-range dimensions into the engine: an out-of-range
    /// `width` would call `rng.number(max <= 0)` and write outside the grid.
    /// The bounds are the core crate's constants (single source).
    pub(crate) fn from_bytes(data: &[u8]) -> Result<Self, ProtocolError> {
        use crackattack_core::constants::{MAX_GARBAGE_HEIGHT, PLAY_WIDTH};
        if data.is_empty() {
            return Err(ProtocolError::IncompleteBuffer);
        }
        let count = data[0] as usize;
        if count > MAX_GARBAGE_PER_CYCLE {
            return Err(ProtocolError::InvalidGarbageCount(count));
        }
        let expected = 4 + count * 16;
        if data.len() < expected {
            return Err(ProtocolError::IncompleteBuffer);
        }
        let mut garbage = Vec::with_capacity(count);
        for i in 0..count {
            let offset = 4 + i * 16;
            // `pod_read_unaligned`: `from_bytes` would need a 4-byte aligned slice,
            // which a `Vec<u8>` does not guarantee.
            let entry: GarbageEntry = bytemuck::pod_read_unaligned(&data[offset..offset + 16]);
            let valid = (1..=MAX_GARBAGE_HEIGHT).contains(&entry.height)
                && (1..=PLAY_WIDTH as i32).contains(&entry.width)
                && (crackattack_core::garbage::GARBAGE_FLAVOR_NORMAL
                    ..=crackattack_core::garbage::GARBAGE_FLAVOR_GRAY).contains(&entry.flavor)
                && entry.timestamp >= 0;
            if !valid {
                return Err(ProtocolError::InvalidGarbageEntry {
                    height: entry.height, width: entry.width,
                    flavor: entry.flavor, timestamp: entry.timestamp,
                });
            }
            garbage.push(entry);
        }
        Ok(Self { garbage })
    }
}

/// Context for synchronization messages (Ready, StartAt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum SyncContext {
    PauseResume = 1,
    RoundStart = 2,
}

/// Status of a player for the current round (alive or lost).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoundLifeStatus {
    Alive,
    Lost { loss_tick: i32 },
}

/// Which peer in the connection (server or client).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum PeerRole {
    Server = 1,
    Client = 2,
}

/// Reason for the server's round verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum VerdictReason {
    OnlyOneLost = 1,
    HigherLossTick = 2,
    FewerSwaps = 3,
    ServerTieBreak = 4,
}

/// Control plane message — sent immediately, not batched.
#[derive(Debug, Clone)]
pub(crate) enum ControlMessage {
    Pause,
    Unpause { epoch: u16 },
    Ready { context: SyncContext, epoch: u16 },
    StartAt { context: SyncContext, epoch: u16, tick: i32 },
    GridLevel(u8),
    SyncCounter(u32),
    RoundLifeReport { round_id: u16, status: RoundLifeStatus, swaps_performed: u32 },
    EndPlay,
    /// RTT measurement: sender includes a sequence number.
    Ping(u32),
    /// RTT measurement: echo back the sequence number.
    Pong(u32),
    /// End-of-game stats exchange.
    Stats {
        blocks_eliminated: u32,
        chains_count: u32,
        max_chain_level: u32,
        garbage_sent: u32,
        garbage_received: u32,
        garbage_shattered: u32,
        swaps_performed: u32,
        duration_ticks: u32,
    },
    /// Server-authoritative round verdict.
    RoundVerdict { round_id: u16, winner: PeerRole, reason: VerdictReason },
    /// Extension: the sender's board, for the opponent preview.
    BoardView(OpponentBoardView),
    /// Extension: whether the sender shares its board (sent at each round, before any view).
    BoardSharing(bool),
    /// Extension of a type this version does not know: received and skipped.
    UnknownExtension(u8),
}

// Wire type IDs for `ControlMessage` variants.
const CTRL_PAUSE: u8 = 0x01;
const CTRL_UNPAUSE: u8 = 0x02;
const CTRL_READY: u8 = 0x03;
const CTRL_START_AT: u8 = 0x04;
const CTRL_GRID_LEVEL: u8 = 0x05;
const CTRL_SYNC_COUNTER: u8 = 0x06;
const CTRL_ROUND_LIFE_REPORT: u8 = 0x07;
// 0x08 and 0x09 are RESERVED (removed control messages). Do not reuse these IDs
// without checking compatibility with older peers.
const CTRL_END_PLAY: u8 = 0x0A;
const CTRL_PING: u8 = 0x0B;
const CTRL_PONG: u8 = 0x0C;
const CTRL_STATS: u8 = 0x0D;
const CTRL_ROUND_VERDICT: u8 = 0x0E;
/// Extension: `[ext type u8][length u16 LE][payload]`, see `PROTOCOL_VERSION`.
pub(crate) const CTRL_EXTENSION: u8 = 0x0F;

/// Extension header after the id byte: type (1) + length (2).
pub(crate) const EXTENSION_HEADER_LEN: usize = 3;
/// Longest extension payload a peer may announce; beyond it the stream is treated as corrupt.
pub(crate) const MAX_EXTENSION_LEN: usize = 1024;

const EXT_BOARD_VIEW: u8 = 1;
const EXT_BOARD_SHARING: u8 = 2;

/// Payload length of an extension, read from its header (`header` starts at the extension type).
pub(crate) fn extension_len(header: &[u8]) -> usize {
    u16::from_le_bytes([header[1], header[2]]) as usize
}

/// Payload size (bytes AFTER the id byte) of each control message. This is the
/// **single source of the wire layout**: the framer (`communicator::MessageReader`)
/// and the guards in [`ControlMessage::from_bytes`] both use it. `None` = unknown id.
/// To add a variant, extend this table and `to_bytes`/`from_bytes`. The test
/// `wire_layout_single_source` checks that the table matches `to_bytes`.
pub(crate) fn ctrl_payload_size(ctrl_id: u8) -> Option<usize> {
    Some(match ctrl_id {
        CTRL_PAUSE | CTRL_END_PLAY => 0,
        CTRL_GRID_LEVEL => 1,
        CTRL_UNPAUSE => 2,
        CTRL_READY => 3,
        CTRL_SYNC_COUNTER | CTRL_PING | CTRL_PONG => 4,
        CTRL_ROUND_VERDICT => 4, // round_id (2) + winner + reason
        CTRL_START_AT => 7,      // ctx + epoch (2) + tick (4)
        CTRL_ROUND_LIFE_REPORT => 11, // round_id (2) + status + loss_tick (4) + swaps (4)
        CTRL_STATS => 32,        // 8 × u32
        // Header only: the length that follows is read from it (`extension_len`).
        CTRL_EXTENSION => EXTENSION_HEADER_LEN,
        _ => return None,
    })
}

impl ControlMessage {
    /// Serialize to bytes (WITHOUT the MSG_TYPE_CONTROL prefix).
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Pause => vec![CTRL_PAUSE],
            Self::Unpause { epoch } => {
                let mut buf = vec![CTRL_UNPAUSE];
                buf.extend_from_slice(&epoch.to_le_bytes());
                buf
            }
            Self::Ready { context, epoch } => {
                vec![CTRL_READY, *context as u8, epoch.to_le_bytes()[0], epoch.to_le_bytes()[1]]
            }
            Self::StartAt { context, epoch, tick } => {
                let mut buf = vec![CTRL_START_AT, *context as u8];
                buf.extend_from_slice(&epoch.to_le_bytes());
                buf.extend_from_slice(&tick.to_le_bytes());
                buf
            }
            Self::GridLevel(level) => vec![CTRL_GRID_LEVEL, *level],
            Self::SyncCounter(counter) => {
                let mut buf = vec![CTRL_SYNC_COUNTER];
                buf.extend_from_slice(&counter.to_le_bytes());
                buf
            }
            Self::RoundLifeReport { round_id, status, swaps_performed } => {
                let mut buf = Vec::with_capacity(12);
                buf.push(CTRL_ROUND_LIFE_REPORT);
                buf.extend_from_slice(&round_id.to_le_bytes());
                let (status_byte, loss_tick) = match status {
                    RoundLifeStatus::Alive => (0x00u8, 0i32),
                    RoundLifeStatus::Lost { loss_tick } => (0x01u8, *loss_tick),
                };
                buf.push(status_byte);
                buf.extend_from_slice(&loss_tick.to_le_bytes());
                buf.extend_from_slice(&swaps_performed.to_le_bytes());
                buf
            }
            Self::EndPlay => vec![CTRL_END_PLAY],
            Self::Ping(seq) => {
                let mut buf = vec![CTRL_PING];
                buf.extend_from_slice(&seq.to_le_bytes());
                buf
            }
            Self::Pong(seq) => {
                let mut buf = vec![CTRL_PONG];
                buf.extend_from_slice(&seq.to_le_bytes());
                buf
            }
            Self::Stats {
                blocks_eliminated, chains_count, max_chain_level,
                garbage_sent, garbage_received, garbage_shattered,
                swaps_performed, duration_ticks,
            } => {
                let mut buf = Vec::with_capacity(33);
                buf.push(CTRL_STATS);
                buf.extend_from_slice(&blocks_eliminated.to_le_bytes());
                buf.extend_from_slice(&chains_count.to_le_bytes());
                buf.extend_from_slice(&max_chain_level.to_le_bytes());
                buf.extend_from_slice(&garbage_sent.to_le_bytes());
                buf.extend_from_slice(&garbage_received.to_le_bytes());
                buf.extend_from_slice(&garbage_shattered.to_le_bytes());
                buf.extend_from_slice(&swaps_performed.to_le_bytes());
                buf.extend_from_slice(&duration_ticks.to_le_bytes());
                buf
            }
            Self::RoundVerdict { round_id, winner, reason } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(CTRL_ROUND_VERDICT);
                buf.extend_from_slice(&round_id.to_le_bytes());
                buf.push(*winner as u8);
                buf.push(*reason as u8);
                buf
            }
            Self::BoardView(view) => extension_bytes(EXT_BOARD_VIEW, &view.to_bytes()),
            Self::BoardSharing(shares) => extension_bytes(EXT_BOARD_SHARING, &[u8::from(*shares)]),
            Self::UnknownExtension(ext) => extension_bytes(*ext, &[]),
        }
    }

    /// Deserialize from bytes (WITHOUT the MSG_TYPE_CONTROL prefix).
    pub(crate) fn from_bytes(data: &[u8]) -> Result<Self, ProtocolError> {
        if data.is_empty() {
            return Err(ProtocolError::IncompleteControl);
        }
        // Single size guard, derived from the layout table. The arms below can
        // index their payload without checking the length again.
        let payload = ctrl_payload_size(data[0])
            .ok_or(ProtocolError::InvalidControlType(data[0]))?;
        if data.len() < 1 + payload {
            return Err(ProtocolError::IncompleteControl);
        }
        match data[0] {
            CTRL_PAUSE => Ok(Self::Pause),
            CTRL_UNPAUSE => {
                let epoch = u16::from_le_bytes([data[1], data[2]]);
                Ok(Self::Unpause { epoch })
            }
            CTRL_READY => {
                let context = parse_sync_context(data[1])?;
                let epoch = u16::from_le_bytes([data[2], data[3]]);
                Ok(Self::Ready { context, epoch })
            }
            CTRL_START_AT => {
                let context = parse_sync_context(data[1])?;
                let epoch = u16::from_le_bytes([data[2], data[3]]);
                let tick = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
                Ok(Self::StartAt { context, epoch, tick })
            }
            CTRL_GRID_LEVEL => Ok(Self::GridLevel(data[1])),
            CTRL_SYNC_COUNTER => {
                let counter = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                Ok(Self::SyncCounter(counter))
            }
            CTRL_ROUND_LIFE_REPORT => {
                let round_id = u16::from_le_bytes([data[1], data[2]]);
                let status_byte = data[3];
                let loss_tick = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
                let swaps_performed = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
                let status = match status_byte {
                    0x00 => RoundLifeStatus::Alive,
                    0x01 => RoundLifeStatus::Lost { loss_tick },
                    _ => return Err(ProtocolError::InvalidControlType(status_byte)),
                };
                Ok(Self::RoundLifeReport { round_id, status, swaps_performed })
            }
            CTRL_END_PLAY => Ok(Self::EndPlay),
            CTRL_PING => {
                let seq = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                Ok(Self::Ping(seq))
            }
            CTRL_PONG => {
                let seq = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                Ok(Self::Pong(seq))
            }
            CTRL_STATS => {
                let u = |off: usize| u32::from_le_bytes([data[off], data[off+1], data[off+2], data[off+3]]);
                Ok(Self::Stats {
                    blocks_eliminated: u(1),
                    chains_count: u(5),
                    max_chain_level: u(9),
                    garbage_sent: u(13),
                    garbage_received: u(17),
                    garbage_shattered: u(21),
                    swaps_performed: u(25),
                    duration_ticks: u(29),
                })
            }
            CTRL_ROUND_VERDICT => {
                let round_id = u16::from_le_bytes([data[1], data[2]]);
                let winner = match data[3] {
                    1 => PeerRole::Server,
                    2 => PeerRole::Client,
                    _ => return Err(ProtocolError::InvalidControlType(data[3])),
                };
                let reason = match data[4] {
                    1 => VerdictReason::OnlyOneLost,
                    2 => VerdictReason::HigherLossTick,
                    3 => VerdictReason::FewerSwaps,
                    4 => VerdictReason::ServerTieBreak,
                    _ => return Err(ProtocolError::InvalidControlType(data[4])),
                };
                Ok(Self::RoundVerdict { round_id, winner, reason })
            }
            CTRL_EXTENSION => {
                let len = extension_len(&data[1..]);
                let payload = data
                    .get(1 + EXTENSION_HEADER_LEN..1 + EXTENSION_HEADER_LEN + len)
                    .ok_or(ProtocolError::IncompleteControl)?;
                match data[1] {
                    EXT_BOARD_VIEW => OpponentBoardView::from_bytes(payload)
                        .map(Self::BoardView)
                        .ok_or(ProtocolError::InvalidBoardView),
                    EXT_BOARD_SHARING => match payload {
                        [0] => Ok(Self::BoardSharing(false)),
                        [1] => Ok(Self::BoardSharing(true)),
                        _ => Err(ProtocolError::InvalidControlType(CTRL_EXTENSION)),
                    },
                    other => Ok(Self::UnknownExtension(other)),
                }
            }
            // Unreachable while the table covers the same ids as this match (see the
            // guard above). Returns an error instead of panicking if they ever diverge.
            _ => Err(ProtocolError::InvalidControlType(data[0])),
        }
    }
}

/// `[CTRL_EXTENSION][ext type][length u16 LE][payload]`.
fn extension_bytes(ext: u8, payload: &[u8]) -> Vec<u8> {
    debug_assert!(payload.len() <= MAX_EXTENSION_LEN);
    let mut buf = Vec::with_capacity(1 + EXTENSION_HEADER_LEN + payload.len());
    buf.push(CTRL_EXTENSION);
    buf.push(ext);
    buf.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

fn parse_sync_context(b: u8) -> Result<SyncContext, ProtocolError> {
    match b {
        1 => Ok(SyncContext::PauseResume),
        2 => Ok(SyncContext::RoundStart),
        _ => Err(ProtocolError::InvalidControlType(b)),
    }
}

/// Seed packet: the server sends the game seed to the client.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SeedPacket {
    pub seed: u32,
}

impl SeedPacket {
    pub(crate) fn to_bytes(&self) -> [u8; 4] {
        self.seed.to_le_bytes()
    }

    pub(crate) fn from_bytes(data: &[u8; 4]) -> Self {
        Self { seed: u32::from_le_bytes(*data) }
    }
}

#[derive(Debug)]
pub(crate) enum ProtocolError {
    IncompletePreamble,
    IncompleteBuffer,
    IncompleteControl,
    BadMagic(u32),
    VersionMismatch { local: u32, remote: u32 },
    InvalidGarbageCount(usize),
    /// Garbage entry with dimensions outside the game's bounds (hostile peer or sender bug).
    InvalidGarbageEntry { height: i32, width: i32, flavor: i32, timestamp: i32 },
    InvalidControlType(u8),
    /// Opponent board view whose cells no board can produce.
    InvalidBoardView,
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncompletePreamble => write!(f, "incomplete handshake data"),
            Self::IncompleteBuffer => write!(f, "incomplete gameplay buffer"),
            Self::IncompleteControl => write!(f, "incomplete control message"),
            Self::BadMagic(m) => write!(f, "bad handshake magic: 0x{:08X}", m),
            Self::VersionMismatch { local, remote } =>
                write!(f, "protocol version mismatch: local={}, remote={}", local, remote),
            Self::InvalidGarbageCount(n) =>
                write!(f, "garbage count {} exceeds max {}", n, MAX_GARBAGE_PER_CYCLE),
            Self::InvalidGarbageEntry { height, width, flavor, timestamp } =>
                write!(f, "garbage entry out of bounds: h={} w={} f={} ts={}",
                    height, width, flavor, timestamp),
            Self::InvalidControlType(t) =>
                write!(f, "invalid control message type: 0x{:02X}", t),
            Self::InvalidBoardView => write!(f, "invalid opponent board view"),
        }
    }
}

impl std::error::Error for ProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout table `ctrl_payload_size` (used by the framer AND by the
    /// `from_bytes` guards) matches what `to_bytes` emits, for EVERY variant.
    /// Adding a variant without extending the table breaks this test.
    #[test]
    fn wire_layout_single_source() {
        let samples = [
            ControlMessage::Pause,
            ControlMessage::Unpause { epoch: 7 },
            ControlMessage::Ready { context: SyncContext::RoundStart, epoch: 3 },
            ControlMessage::StartAt { context: SyncContext::PauseResume, epoch: 1, tick: 99 },
            ControlMessage::GridLevel(4),
            ControlMessage::SyncCounter(123),
            ControlMessage::RoundLifeReport {
                round_id: 2, status: RoundLifeStatus::Lost { loss_tick: 500 }, swaps_performed: 42,
            },
            ControlMessage::EndPlay,
            ControlMessage::Ping(1),
            ControlMessage::Pong(1),
            ControlMessage::Stats {
                blocks_eliminated: 1, chains_count: 2, max_chain_level: 3, garbage_sent: 4,
                garbage_received: 5, garbage_shattered: 6, swaps_performed: 7, duration_ticks: 8,
            },
            ControlMessage::RoundVerdict {
                round_id: 1, winner: PeerRole::Server, reason: VerdictReason::OnlyOneLost,
            },
            ControlMessage::BoardView(sample_board_view()),
            ControlMessage::BoardSharing(true),
            ControlMessage::BoardSharing(false),
            ControlMessage::UnknownExtension(0xEE),
        ];
        for msg in samples {
            let bytes = msg.to_bytes();
            let mut expected = ctrl_payload_size(bytes[0])
                .unwrap_or_else(|| panic!("id 0x{:02X} missing from the table ({:?})", bytes[0], msg));
            if bytes[0] == CTRL_EXTENSION {
                expected += extension_len(&bytes[1..]);
            }
            assert_eq!(bytes.len(), 1 + expected,
                "layout mismatch for {:?}: to_bytes emits {} bytes, the table says 1+{}",
                msg, bytes.len(), expected);
            // Decoded value, not just `is_ok()`: a variant mapped to the wrong id (e.g.
            // Ping/Pong swapped) would otherwise pass.
            let decoded = ControlMessage::from_bytes(&bytes)
                .unwrap_or_else(|e| panic!("round-trip {msg:?}: {e:?}"));
            assert_eq!(format!("{decoded:?}"), format!("{msg:?}"), "round-trip {msg:?}");
        }
        assert!(ctrl_payload_size(0xFF).is_none());
    }

    fn sample_board_view() -> OpponentBoardView {
        let mut bytes = [0u8; crackattack_core::vs::board_view::BOARD_VIEW_LEN];
        bytes[0] = 1;
        bytes[1] = 9;
        bytes[6] = 9;
        OpponentBoardView::from_bytes(&bytes).unwrap()
    }

    /// An extension of a type this version does not know is skipped, whatever its payload.
    #[test]
    fn unknown_extension_is_skipped() {
        let bytes = extension_bytes(0xEE, &[1, 2, 3, 4, 5]);
        assert!(matches!(ControlMessage::from_bytes(&bytes), Ok(ControlMessage::UnknownExtension(0xEE))));
    }

    #[test]
    fn invalid_board_view_and_sharing_are_rejected() {
        let mut view = sample_board_view().to_bytes();
        view[3] = 0xFF;
        assert!(matches!(
            ControlMessage::from_bytes(&extension_bytes(EXT_BOARD_VIEW, &view)),
            Err(ProtocolError::InvalidBoardView)
        ));
        assert!(matches!(
            ControlMessage::from_bytes(&extension_bytes(EXT_BOARD_VIEW, &view[..10])),
            Err(ProtocolError::InvalidBoardView)
        ));
        assert!(ControlMessage::from_bytes(&extension_bytes(EXT_BOARD_SHARING, &[2])).is_err());
        // Truncated: the header announces more than what is there.
        let full = ControlMessage::BoardView(sample_board_view()).to_bytes();
        assert!(matches!(
            ControlMessage::from_bytes(&full[..full.len() - 1]),
            Err(ProtocolError::IncompleteControl)
        ));
    }

    /// Dimensions outside the game's bounds are rejected at the network boundary
    /// and never reach the engine.
    #[test]
    fn hostile_garbage_entry_rejected() {
        let make = |height: i32, width: i32, flavor: i32, timestamp: i32| {
            let mut buf = GameplayBuffer::new();
            buf.garbage.push(GarbageEntry { height, width, flavor, timestamp });
            GameplayBuffer::from_bytes(&buf.to_bytes())
        };
        // Legitimate entries pass.
        assert!(make(2, 6, 0, 100).is_ok());
        assert!(make(1, 3, 1, 0).is_ok());
        // Hostile entries are rejected (width=100 would call `rng.number(max <= 0)` in the core).
        assert!(matches!(make(2, 100, 0, 100), Err(ProtocolError::InvalidGarbageEntry { .. })));
        assert!(matches!(make(0, 6, 0, 100), Err(ProtocolError::InvalidGarbageEntry { .. })));
        assert!(matches!(make(999, 6, 0, 100), Err(ProtocolError::InvalidGarbageEntry { .. })));
        assert!(matches!(make(2, 6, 7, 100), Err(ProtocolError::InvalidGarbageEntry { .. })));
        assert!(matches!(make(2, 6, -1, 100), Err(ProtocolError::InvalidGarbageEntry { .. })));
        assert!(matches!(make(2, 6, 0, -5), Err(ProtocolError::InvalidGarbageEntry { .. })));
    }

    /// Name truncation respects character boundaries.
    #[test]
    fn name_truncation_on_char_boundary() {
        // 17 two-byte characters = 34 bytes > 32: a naive cut at 32 would split
        // the 17th character.
        let name = "é".repeat(17);
        let bytes = utf8_prefix(&name, MAX_PLAYER_NAME_LEN);
        assert!(bytes.len() <= MAX_PLAYER_NAME_LEN);
        assert!(std::str::from_utf8(bytes).is_ok(), "truncation split a character");
        assert_eq!(std::str::from_utf8(bytes).unwrap(), "é".repeat(16));
        // The full packet still parses and the name is clean.
        let pkt = HandshakePacket::new(&name, [1u8; 16], 0);
        let (parsed, _) = HandshakePacket::from_bytes(&pkt.to_bytes()).unwrap();
        assert_eq!(parsed.player_name, "é".repeat(16));
    }

    #[test]
    fn handshake_round_trip() {
        let uuid: [u8; 16] = [0xAB; 16];
        let pkt = HandshakePacket::new("PLAYER", uuid, 45678);
        let bytes = pkt.to_bytes();
        let (parsed, consumed) = HandshakePacket::from_bytes(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(parsed.magic, HANDSHAKE_MAGIC);
        assert_eq!(parsed.protocol_version, PROTOCOL_VERSION);
        assert_eq!(parsed.player_uuid, uuid);
        assert_eq!(parsed.listen_port, 45678);
        assert_eq!(parsed.player_name, "PLAYER");
    }

    #[test]
    fn handshake_bad_magic() {
        let mut bytes = HandshakePacket::new("X", [1; 16], 0).to_bytes();
        bytes[0] = 0xFF;
        assert!(matches!(
            HandshakePacket::from_bytes(&bytes),
            Err(ProtocolError::BadMagic(_))
        ));
    }

    #[test]
    fn handshake_version_mismatch_rejected() {
        let mut bytes = HandshakePacket::new("X", [1; 16], 0).to_bytes();
        bytes[4] = 0xFE; // corrupt the version's low byte
        assert!(matches!(
            HandshakePacket::from_bytes(&bytes),
            Err(ProtocolError::VersionMismatch { .. })
        ));
    }

    #[test]
    fn uuid_hex_format() {
        let mut uuid = [0u8; 16];
        uuid[0] = 0x0F;
        uuid[15] = 0xA0;
        let hex = uuid_hex(&uuid);
        assert_eq!(hex.len(), 32);
        assert!(hex.starts_with("0f"));
        assert!(hex.ends_with("a0"));
    }

    #[test]
    fn gameplay_buffer_round_trip() {
        let mut buf = GameplayBuffer::new();
        buf.garbage.push(GarbageEntry { height: 2, width: 6, flavor: 0, timestamp: 100 });
        buf.garbage.push(GarbageEntry { height: 1, width: 3, flavor: 1, timestamp: 132 });

        let bytes = buf.to_bytes();
        let parsed = GameplayBuffer::from_bytes(&bytes).unwrap();

        assert_eq!(parsed.garbage.len(), 2);
        assert_eq!(parsed.garbage[0].height, 2);
        assert_eq!(parsed.garbage[1].flavor, 1);
    }

    #[test]
    fn gameplay_buffer_empty() {
        let buf = GameplayBuffer::new();
        let bytes = buf.to_bytes();
        let parsed = GameplayBuffer::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.garbage.len(), 0);
    }

    #[test]
    fn control_message_pause_round_trip() {
        let msg = ControlMessage::Pause;
        let bytes = msg.to_bytes();
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        assert!(matches!(parsed, ControlMessage::Pause));
    }

    #[test]
    fn control_message_unpause_round_trip() {
        let msg = ControlMessage::Unpause { epoch: 42 };
        let bytes = msg.to_bytes();
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        match parsed {
            ControlMessage::Unpause { epoch } => assert_eq!(epoch, 42),
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn control_message_ready_round_trip() {
        let msg = ControlMessage::Ready { context: SyncContext::RoundStart, epoch: 3 };
        let bytes = msg.to_bytes();
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        match parsed {
            ControlMessage::Ready { context, epoch } => {
                assert_eq!(context, SyncContext::RoundStart);
                assert_eq!(epoch, 3);
            }
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn control_message_start_at_round_trip() {
        let msg = ControlMessage::StartAt {
            context: SyncContext::PauseResume, epoch: 7, tick: 12345,
        };
        let bytes = msg.to_bytes();
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        match parsed {
            ControlMessage::StartAt { context, epoch, tick } => {
                assert_eq!(context, SyncContext::PauseResume);
                assert_eq!(epoch, 7);
                assert_eq!(tick, 12345);
            }
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn control_message_grid_level_round_trip() {
        let msg = ControlMessage::GridLevel(9);
        let bytes = msg.to_bytes();
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        match parsed {
            ControlMessage::GridLevel(level) => assert_eq!(level, 9),
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn control_message_round_life_report_lost_round_trip() {
        let msg = ControlMessage::RoundLifeReport {
            round_id: 2,
            status: RoundLifeStatus::Lost { loss_tick: 5000 },
            swaps_performed: 42,
        };
        let bytes = msg.to_bytes();
        assert_eq!(bytes.len(), 12); // 1 type + 2 round_id + 1 status + 4 loss_tick + 4 swaps
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        match parsed {
            ControlMessage::RoundLifeReport { round_id, status, swaps_performed } => {
                assert_eq!(round_id, 2);
                assert_eq!(status, RoundLifeStatus::Lost { loss_tick: 5000 });
                assert_eq!(swaps_performed, 42);
            }
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn control_message_round_life_report_alive_round_trip() {
        let msg = ControlMessage::RoundLifeReport {
            round_id: 1,
            status: RoundLifeStatus::Alive,
            swaps_performed: 100,
        };
        let bytes = msg.to_bytes();
        assert_eq!(bytes.len(), 12);
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        match parsed {
            ControlMessage::RoundLifeReport { round_id, status, swaps_performed } => {
                assert_eq!(round_id, 1);
                assert_eq!(status, RoundLifeStatus::Alive);
                assert_eq!(swaps_performed, 100);
            }
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn control_message_round_verdict_round_trip() {
        let msg = ControlMessage::RoundVerdict {
            round_id: 3,
            winner: PeerRole::Client,
            reason: VerdictReason::HigherLossTick,
        };
        let bytes = msg.to_bytes();
        assert_eq!(bytes.len(), 5); // 1 type + 2 round_id + 1 winner + 1 reason
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        match parsed {
            ControlMessage::RoundVerdict { round_id, winner, reason } => {
                assert_eq!(round_id, 3);
                assert_eq!(winner, PeerRole::Client);
                assert_eq!(reason, VerdictReason::HigherLossTick);
            }
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn control_message_round_verdict_all_reasons() {
        for (winner, reason) in [
            (PeerRole::Server, VerdictReason::OnlyOneLost),
            (PeerRole::Client, VerdictReason::HigherLossTick),
            (PeerRole::Server, VerdictReason::FewerSwaps),
            (PeerRole::Server, VerdictReason::ServerTieBreak),
        ] {
            let msg = ControlMessage::RoundVerdict { round_id: 1, winner, reason };
            let bytes = msg.to_bytes();
            let parsed = ControlMessage::from_bytes(&bytes).unwrap();
            match parsed {
                ControlMessage::RoundVerdict { winner: w, reason: r, .. } => {
                    assert_eq!(w, winner);
                    assert_eq!(r, reason);
                }
                _ => panic!("wrong type"),
            }
        }
    }

    #[test]
    fn control_message_sync_counter_round_trip() {
        let msg = ControlMessage::SyncCounter(99999);
        let bytes = msg.to_bytes();
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        match parsed {
            ControlMessage::SyncCounter(c) => assert_eq!(c, 99999),
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn control_message_end_play_round_trip() {
        let msg = ControlMessage::EndPlay;
        let bytes = msg.to_bytes();
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        assert!(matches!(parsed, ControlMessage::EndPlay));
    }

    #[test]
    fn control_message_stats_round_trip() {
        let msg = ControlMessage::Stats {
            blocks_eliminated: 142,
            chains_count: 8,
            max_chain_level: 4,
            garbage_sent: 12,
            garbage_received: 6,
            garbage_shattered: 3,
            swaps_performed: 234,
            duration_ticks: 7500,
        };
        let bytes = msg.to_bytes();
        assert_eq!(bytes.len(), 33); // 1 type + 8 * 4 payload
        let parsed = ControlMessage::from_bytes(&bytes).unwrap();
        match parsed {
            ControlMessage::Stats {
                blocks_eliminated, chains_count, max_chain_level,
                garbage_sent, garbage_received, garbage_shattered,
                swaps_performed, duration_ticks,
            } => {
                assert_eq!(blocks_eliminated, 142);
                assert_eq!(chains_count, 8);
                assert_eq!(max_chain_level, 4);
                assert_eq!(garbage_sent, 12);
                assert_eq!(garbage_received, 6);
                assert_eq!(garbage_shattered, 3);
                assert_eq!(swaps_performed, 234);
                assert_eq!(duration_ticks, 7500);
            }
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn seed_round_trip() {
        let pkt = SeedPacket { seed: 12345 };
        let bytes = pkt.to_bytes();
        let parsed = SeedPacket::from_bytes(&bytes);
        assert_eq!(parsed.seed, 12345);
    }

    /// Reference bytes of the handshake for the current `PROTOCOL_VERSION`. If this test has to
    /// change, the wire format changed: increment `PROTOCOL_VERSION` and update both.
    #[test]
    fn handshake_reference_bytes() {
        let bytes = HandshakePacket::new("AB", [0x11; 16], 45678).to_bytes();
        let mut expected = vec![0x4B, 0x41, 0x52, 0x43, 4, 0, 0, 0];
        expected.extend_from_slice(&[0x11; 16]);
        expected.extend_from_slice(&[0x6E, 0xB2, 2, b'A', b'B']);
        assert_eq!(PROTOCOL_VERSION, 4, "update the reference bytes with the new version");
        assert_eq!(bytes, expected);
    }
}
