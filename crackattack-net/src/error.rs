//! Typed network error, so callers match on a variant instead of parsing message prefixes.
//!
//! It only covers the **handshake and the invite functions**. The gameplay plane
//! (`send_gameplay`, `MessageReader::poll`) still returns `Result<_, String>`.
//!
//! Invariant: `Display` produces exactly the same messages as the plain strings did.
//! They reach the logs and the UI unchanged.

/// Error from a network negotiation step (handshake, invite).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// Socket I/O error (message already carries its context, ready for log/UI).
    Io(String),
    /// Protocol violation (malformed packet, invalid length).
    Protocol(String),
    /// Timeout (silent peer: the socket is open but nothing arrives).
    Timeout(String),
    /// The peer speaks another protocol version (handshake). It has its own variant so
    /// the UI can tell the player to update instead of showing a technical message.
    VersionMismatch { local: u32, remote: u32 },
}

impl NetError {
    /// Classifies an I/O error: timeout (`TimedOut`/`WouldBlock`, raised by the bounded
    /// reads of `NetConnection`) or real socket error. The message has the form
    /// `"{context}: {error}"`.
    pub(crate) fn from_io(context: &str, e: &std::io::Error) -> Self {
        let msg = format!("{context}: {e}");
        match e.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Self::Timeout(msg),
            _ => Self::Io(msg),
        }
    }
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(m) | Self::Protocol(m) | Self::Timeout(m) => f.write_str(m),
            // Same text as a `Protocol("handshake error: ...")` message.
            Self::VersionMismatch { local, remote } => write!(f,
                "handshake error: protocol version mismatch: local={local}, remote={remote}"),
        }
    }
}

impl std::error::Error for NetError {}
