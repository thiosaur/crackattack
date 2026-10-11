//! TCP connection wrapper with timeouts.
//!
//! Provides connect/listen/send/recv with configurable timeouts.
//! No async — runs in a dedicated thread.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, SocketAddr, ToSocketAddrs};
use std::time::Duration;

/// Timeout for non-blocking recv attempts.
const RECV_TIMEOUT: Duration = Duration::from_millis(50);

/// Timeout for initial connection attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// TCP connection with timeout support.
pub struct NetConnection {
    stream: TcpStream,
    pub remote_addr: SocketAddr,
}

/// Role in the connection (affects tiebreaker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Server,
    Client,
}

/// Connection error.
#[derive(Debug)]
pub enum ConnectError {
    Io(io::Error),
    NoAddress,
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "connection error: {}", e),
            Self::NoAddress => write!(f, "could not resolve address"),
        }
    }
}

impl std::error::Error for ConnectError {}

impl From<io::Error> for ConnectError {
    fn from(e: io::Error) -> Self { Self::Io(e) }
}

/// Result of a recv_with_timeout call.
pub(crate) enum RecvResult {
    /// Data received successfully.
    Ok,
    /// Timeout — no data available.
    Timeout,
    /// Connection closed or error.
    Disconnected(io::Error),
}

/// Result of a **single** read attempt (see [`NetConnection::recv_some`]).
pub(crate) enum RecvSome {
    /// `n` octets lus (n ≥ 1).
    Read(usize),
    /// Nothing arrived within `RECV_TIMEOUT`.
    Timeout,
    /// Connection closed or error.
    Disconnected(io::Error),
}

/// Overall limit to finish a multi-byte read once it has started. Past it, the peer is
/// treated as dead or hostile. A legitimate frame is at most ~520 bytes, so a long stall
/// mid-field means a dropped link or a peer trickling bytes.
const STALL_TIMEOUT: Duration = Duration::from_secs(5);

impl NetConnection {
    /// Wrap an existing TCP stream (for tests or pre-established connections).
    pub fn from_stream(stream: TcpStream, remote_addr: SocketAddr) -> Self {
        stream.set_nodelay(true).ok();
        Self { stream, remote_addr }
    }

    /// Connect to a remote host (client mode). Accepts an IPv4 / IPv6 literal **or** a
    /// hostname, and tries **every** resolved address until one succeeds. The
    /// `(host, port)` tuple form handles IPv6 literals without brackets and hostnames,
    /// unlike `format!("{host}:{port}")`.
    pub fn connect(host: &str, port: u16) -> Result<Self, ConnectError> {
        let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs()?.collect();
        if addrs.is_empty() {
            return Err(ConnectError::NoAddress);
        }
        let mut last_err: Option<io::Error> = None;
        for addr in addrs {
            log::info!("Connecting to {}...", addr);
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(stream) => {
                    stream.set_nodelay(true).ok();
                    log::info!("Connected to {}", addr);
                    return Ok(Self { remote_addr: addr, stream });
                }
                Err(e) => {
                    log::debug!("Connect to {} failed: {} — trying next address", addr, e);
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.map(ConnectError::Io).unwrap_or(ConnectError::NoAddress))
    }

    /// Send all bytes (blocking).
    pub(crate) fn send_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.stream.write_all(data)
    }

    /// Receive exactly `size` bytes with timeout.
    ///
    /// A read that has **started** (at least 1 byte received) is bounded by `STALL_TIMEOUT`.
    /// A peer that sends part of the field and then goes silent gets `Disconnected` instead
    /// of blocking the thread forever. To receive whole frames, prefer
    /// `communicator::MessageReader`, which resumes without losing bytes between polls.
    pub(crate) fn recv_exact(&mut self, buf: &mut [u8]) -> RecvResult {
        self.stream.set_read_timeout(Some(RECV_TIMEOUT)).ok();
        let mut filled = 0;
        let mut started: Option<std::time::Instant> = None;
        while filled < buf.len() {
            match self.stream.read(&mut buf[filled..]) {
                Ok(0) => return RecvResult::Disconnected(
                    io::Error::new(io::ErrorKind::ConnectionReset, "connection closed")
                ),
                Ok(n) => {
                    filled += n;
                    started.get_or_insert_with(std::time::Instant::now);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut =>
                {
                    if filled == 0 {
                        return RecvResult::Timeout;
                    }
                    // Partial read: retry, but only until the overall stall limit.
                    if started.map(|t| t.elapsed() > STALL_TIMEOUT).unwrap_or(false) {
                        return RecvResult::Disconnected(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("peer stalled mid-read ({}/{} bytes)", filled, buf.len()),
                        ));
                    }
                }
                Err(e) => return RecvResult::Disconnected(e),
            }
        }
        RecvResult::Ok
    }

    /// Makes a **single** read attempt (timeout `RECV_TIMEOUT`) of at most `buf.len()` bytes.
    /// It never loops. The caller resumes after a partial read and keeps its own state
    /// between calls (see `communicator::MessageReader`).
    pub(crate) fn recv_some(&mut self, buf: &mut [u8]) -> RecvSome {
        self.stream.set_read_timeout(Some(RECV_TIMEOUT)).ok();
        match self.stream.read(buf) {
            Ok(0) => RecvSome::Disconnected(
                io::Error::new(io::ErrorKind::ConnectionReset, "connection closed")
            ),
            Ok(n) => RecvSome::Read(n),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock
                || e.kind() == io::ErrorKind::TimedOut => RecvSome::Timeout,
            Err(e) => RecvSome::Disconnected(e),
        }
    }

    /// Receive exactly `buf.len()` bytes, bounding the wait: if no data arrives within `timeout`,
    /// it fails with `WouldBlock`/`TimedOut` instead of blocking forever. Used to read an
    /// incoming invite on a dedicated thread without leaving a stuck thread behind.
    pub(crate) fn recv_exact_blocking_timeout(
        &mut self,
        buf: &mut [u8],
        timeout: std::time::Duration,
    ) -> io::Result<()> {
        self.stream.set_read_timeout(Some(timeout)).ok();
        let res = self.stream.read_exact(buf);
        self.stream.set_read_timeout(None).ok();
        res
    }

    /// Returns whether the peer has closed the connection. The check is **non-blocking**
    /// and uses `peek`, so it consumes nothing. `Ok(0)` is EOF (clean close) and an error
    /// means the peer is gone. `WouldBlock` (open, no data) or pending data means it is
    /// still there. The invitee uses it to notice that the inviter cancelled or left.
    pub fn peer_closed(&mut self) -> bool {
        self.stream.set_nonblocking(true).ok();
        let mut buf = [0u8; 1];
        let closed = match self.stream.peek(&mut buf) {
            Ok(0) => true,
            Ok(_) => false,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => false,
            Err(_) => true,
        };
        self.stream.set_nonblocking(false).ok();
        closed
    }
}

/// Default TCP game port (preferred).
pub const TCP_PORT_START: u16 = 45678;

/// Number of fallback ports to try.
pub const TCP_PORT_RANGE: u16 = 11;

/// Bind a TcpListener on the first available port in the range.
///
/// Returns the listener and the port it bound to.
/// The listener is set to non-blocking mode for polling with accept().
pub fn bind_tcp_range(start: u16, count: u16) -> io::Result<(TcpListener, u16)> {
    for port in start..start + count {
        match bind_dual_stack(port) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                let v6 = listener.local_addr().map(|a| a.is_ipv6()).unwrap_or(false);
                log::info!("TCP listener bound on port {} ({})", port,
                    if v6 { "dual-stack v6+v4" } else { "v4-only" });
                return Ok((listener, port));
            }
            Err(ref e) if e.kind() == io::ErrorKind::AddrInUse => {
                log::debug!("Port {} in use, trying next", port);
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        format!("all ports {}-{} in use", start, start + count - 1),
    ))
}

/// Binds a **dual-stack** listener on `[::]:port` (`IPV6_V6ONLY=false`, so it accepts IPv6
/// clients **and** IPv4 clients as v4-mapped). Falls back to IPv4 `0.0.0.0:port` if IPv6
/// is unavailable.
/// There is no `SO_REUSEADDR`: `AddrInUse` must be detected to move on to the next port,
/// so that each local instance gets its own port.
fn bind_dual_stack(port: u16) -> io::Result<TcpListener> {
    use socket2::{Domain, Socket, Type};
    use std::net::{IpAddr, Ipv6Addr};
    let try_v6 = || -> io::Result<TcpListener> {
        let sock = Socket::new(Domain::IPV6, Type::STREAM, None)?;
        sock.set_only_v6(false)?;
        let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);
        sock.bind(&addr.into())?;
        sock.listen(128)?;
        Ok(sock.into())
    };
    match try_v6() {
        Ok(listener) => Ok(listener),
        // Port in use: report `AddrInUse` so the caller tries the next port.
        Err(ref e) if e.kind() == io::ErrorKind::AddrInUse => Err(io::Error::new(
            io::ErrorKind::AddrInUse, "v6 port in use")),
        // IPv6 unavailable on this platform or interface: fall back to IPv4 on the same port.
        Err(e) => {
            log::warn!("Dual-stack v6 bind failed on port {} ({}), falling back to IPv4", port, e);
            TcpListener::bind(format!("0.0.0.0:{}", port))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_send_recv() {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => {
                // With CI_NET=1 the loopback is required: a silent skip would leave the
                // network code untested.
                if std::env::var_os("CI_NET").is_some() {
                    panic!("CI_NET=1: loopback bind required, failed: {e}");
                }
                eprintln!("Skipping loopback test (bind failed: {})", e);
                return; // Sandboxed environment: skip gracefully
            }
        };
        let port = listener.local_addr().unwrap().port();

        let client_thread = std::thread::spawn(move || {
            NetConnection::connect("127.0.0.1", port).unwrap()
        });

        let (stream, addr) = listener.accept().unwrap();
        let mut server = NetConnection::from_stream(stream, addr);

        let mut client = client_thread.join().unwrap();

        client.send_all(b"hello").unwrap();
        let mut buf = [0u8; 5];
        match server.recv_exact(&mut buf) {
            RecvResult::Ok => assert_eq!(&buf, b"hello"),
            _ => panic!("expected Ok"),
        }
    }

    /// The dual-stack listener accepts **IPv4 (v4-mapped)** and **IPv6** clients on the same
    /// port. `connect` reaches the v6 literal `::1` correctly.
    #[test]
    fn dual_stack_accepts_v4_and_v6_loopback() {
        let (listener, port) = match bind_tcp_range(54000, 20) {
            Ok(x) => x,
            Err(e) => { eprintln!("Skipping dual-stack test (bind failed: {})", e); return; }
        };
        // If the listener fell back to IPv4-only (no IPv6 on this platform), only test v4.
        let v6_ok = listener.local_addr().map(|a| a.is_ipv6()).unwrap_or(false);
        // Non-blocking accept with a deadline: if a `connect` fails, the test must fail
        // instead of hanging on `accept()` forever.
        listener.set_nonblocking(true).expect("non-blocking listener");

        for host in if v6_ok { vec!["127.0.0.1", "::1"] } else { vec!["127.0.0.1"] } {
            let h = host.to_string();
            let client = std::thread::spawn(move || NetConnection::connect(&h, port));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let (stream, addr) = loop {
                match listener.accept() {
                    Ok(x) => break x,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "no connection accepted via {host} within 5 s"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => panic!("accept via {host}: {e}"),
                }
            };
            stream.set_nonblocking(false).expect("blocking accepted stream");
            let _server = NetConnection::from_stream(stream, addr);
            assert!(client.join().unwrap().is_ok(), "connect via {host} should succeed");
        }
    }
}
