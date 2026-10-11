//! LAN discovery service over **mDNS / DNS-SD** (pure Rust, dual-stack IPv4/IPv6).
//!
//! It announces our presence and discovers peers through mDNS (`mdns-sd`). That crate
//! handles v4 multicast (224.0.0.251), v6 multicast (ff02::fb) and all interfaces by itself.
//! The `ServiceDaemon` runs in a dedicated thread. The app talks to it through mpsc
//! channels (commands in, events out).

use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::protocol;

/// mDNS (DNS-SD) service type announced and browsed on the LAN.
const SERVICE_TYPE: &str = "_crackattack._udp.local.";

/// Pause between two iterations of the discovery thread loop (drain commands and events).
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Status of a discovered peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerStatus {
    /// Available for an invite.
    Available,
    /// In a game (actually being played).
    InGame,
    /// Busy with an invite negotiation in progress (sending, receiving or connecting).
    /// Distinct from `InGame`: the game has not started (yet).
    Busy,
}

impl PeerStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::InGame => "in_game",
            Self::Busy => "busy",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "in_game" => Self::InGame,
            "busy" => Self::Busy,
            _ => Self::Available,
        }
    }
}

/// A discovered peer on the LAN.
#[derive(Debug, Clone)]
pub struct Peer {
    pub name: String,
    pub addr: IpAddr,
    pub port: u16,
    pub session_id: String,
    pub status: PeerStatus,
    /// Rating **self-announced** by the peer (TXT `rt`); absent if unplaced.
    /// It is self-declared and can be forged. The ranking is unofficial by design.
    pub announced_rating: Option<i32>,
    /// Persistent identity of the peer (TXT `uid`, 32 hex digits). It is the reliable
    /// target for quick reconnection, more so than matching on the name.
    pub uuid_hex: Option<String>,
}

/// Commands from the app to the discovery thread.
pub enum DiscoveryCommand {
    /// Update our announced status (re-registers the mDNS service).
    UpdateStatus(PeerStatus),
    /// Update our announced rating (re-registers). `None` means unplaced: no TXT `rt`.
    UpdateRating(Option<i32>),
    /// Shutdown the discovery service.
    Shutdown,
}

/// Events from the discovery thread to the app.
pub enum DiscoveryEvent {
    /// A peer appeared OR its info changed (name, status, port, addr).
    PeerUpdated(Peer),
    /// A peer expired / left.
    PeerExpired(String), // session_id
}

/// Handle to the LAN discovery service.
/// Dropping this sends Shutdown to the background thread.
pub struct DiscoveryService {
    cmd_tx: mpsc::Sender<DiscoveryCommand>,
    event_rx: mpsc::Receiver<DiscoveryEvent>,
    _thread: Option<thread::JoinHandle<()>>,
}

impl DiscoveryService {
    /// Start the discovery service.
    ///
    /// `tcp_port` must be the port of an already-bound TCP listener.
    /// The service announces this port in its mDNS records.
    /// `announced_rating` is our rating to announce in TXT `rt`; `None` means unplaced.
    /// `player_uuid_hex` is our persistent identity (TXT `uid`), the reconnection target.
    pub fn start(
        player_name: &str,
        tcp_port: u16,
        announced_rating: Option<i32>,
        player_uuid_hex: &str,
    ) -> io::Result<Self> {
        let session_id = generate_session_id();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        let name = player_name.to_string();
        let sid = session_id.clone();
        let uid = player_uuid_hex.to_string();

        let handle = thread::Builder::new()
            .name("lan-discovery".into())
            .spawn(move || {
                if let Err(e) = discovery_thread_main(&name, tcp_port, &sid, announced_rating, &uid, cmd_rx, event_tx) {
                    log::error!("Discovery thread failed: {}", e);
                }
            })
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        log::info!("LAN discovery started (session={}, tcp_port={})", session_id, tcp_port);

        Ok(Self {
            cmd_tx,
            event_rx,
            _thread: Some(handle),
        })
    }

    /// Poll for discovery events (non-blocking).
    pub fn poll_events(&self) -> Vec<DiscoveryEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Send a command to the discovery thread.
    pub fn send_command(&self, cmd: DiscoveryCommand) {
        let _ = self.cmd_tx.send(cmd);
    }


}

impl Drop for DiscoveryService {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(DiscoveryCommand::Shutdown);
        if let Some(handle) = self._thread.take() {
            if let Err(e) = handle.join() {
                log::warn!("Discovery thread panicked: {:?}", e);
            }
        }
    }
}

fn discovery_thread_main(
    initial_name: &str,
    tcp_port: u16,
    session_id: &str,
    initial_rating: Option<i32>,
    player_uuid_hex: &str,
    cmd_rx: mpsc::Receiver<DiscoveryCommand>,
    event_tx: mpsc::Sender<DiscoveryEvent>,
) -> io::Result<()> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    let to_io = |e: mdns_sd::Error| io::Error::new(io::ErrorKind::Other, e.to_string());

    let daemon = ServiceDaemon::new().map_err(to_io)?;
    let current_name = initial_name.to_string();
    let mut current_status = PeerStatus::Available;
    let mut current_rating = initial_rating;

    register_service(&daemon, session_id, tcp_port, &current_name, current_status, current_rating, player_uuid_hex);
    let browse_rx = daemon.browse(SERVICE_TYPE).map_err(to_io)?;
    log::info!("mDNS discovery running (service {})", SERVICE_TYPE);

    loop {
        // 1. Commands (non-blocking).
        loop {
            match cmd_rx.try_recv() {
                Ok(DiscoveryCommand::UpdateStatus(status)) => {
                    current_status = status;
                    register_service(&daemon, session_id, tcp_port, &current_name, current_status, current_rating, player_uuid_hex);
                }
                Ok(DiscoveryCommand::UpdateRating(rating)) => {
                    current_rating = rating;
                    register_service(&daemon, session_id, tcp_port, &current_name, current_status, current_rating, player_uuid_hex);
                }
                Ok(DiscoveryCommand::Shutdown) => {
                    log::info!("Discovery thread shutting down");
                    let _ = daemon.shutdown();
                    return Ok(());
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = daemon.shutdown();
                    return Ok(());
                }
            }
        }

        // 2. mDNS events (non-blocking). `ServiceResolved` becomes `PeerUpdated` (a new
        //    peer and an updated one are handled the same). `ServiceRemoved` becomes
        //    `PeerExpired`.
        while let Ok(event) = browse_rx.try_recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    if let Some(peer) = peer_from_resolved(&info, session_id) {
                        let _ = event_tx.send(DiscoveryEvent::PeerUpdated(peer));
                    }
                }
                ServiceEvent::ServiceRemoved(_ty, fullname) => {
                    if let Some(sid) = instance_of(&fullname) {
                        let _ = event_tx.send(DiscoveryEvent::PeerExpired(sid));
                    }
                }
                _ => {}
            }
        }

        thread::sleep(POLL_INTERVAL);
    }
}

/// (Re)publishes our mDNS service with up-to-date TXT records (name, status, session,
/// protocol version, announced rating). `enable_addr_auto` lets `mdns-sd` detect and
/// maintain our v4 and v6 addresses.
fn register_service(
    daemon: &mdns_sd::ServiceDaemon,
    session_id: &str,
    tcp_port: u16,
    name: &str,
    status: PeerStatus,
    rating: Option<i32>,
    player_uuid_hex: &str,
) {
    use mdns_sd::ServiceInfo;
    let host = format!("{}.local.", session_id);
    let nv = protocol::PROTOCOL_VERSION.to_string();
    let rt = rating.map(|r| r.to_string());
    let mut props: Vec<(&str, &str)> = vec![
        ("name", name),
        ("status", status.as_str()),
        ("sid", session_id),
        ("nv", nv.as_str()),
        // Persistent identity (reconnection target). LAN only.
        ("uid", player_uuid_hex),
    ];
    // `rt` only if placed: its absence IS the information "unplaced / unknown".
    if let Some(ref rt) = rt {
        props.push(("rt", rt.as_str()));
    }
    match ServiceInfo::new(SERVICE_TYPE, session_id, &host, "", tcp_port, &props[..]) {
        Ok(info) => {
            if let Err(e) = daemon.register(info.enable_addr_auto()) {
                log::warn!("mDNS register failed: {}", e);
            }
        }
        Err(e) => log::warn!("mDNS ServiceInfo invalid: {}", e),
    }
}

/// Extracts the instance name (the peer's `session_id`) from a fullname
/// `"<instance>._crackattack._udp.local."`.
fn instance_of(fullname: &str) -> Option<String> {
    fullname
        .strip_suffix(&format!(".{}", SERVICE_TYPE))
        .map(|s| s.to_string())
}

/// Builds a `Peer` from a resolved mDNS service. It drops our own service and incompatible
/// protocol versions. It picks a **connectable** address: IPv4 first (widest compatibility),
/// then non-link-local IPv6 (`fe80::` is not reachable without a scope id, which we do not
/// propagate), otherwise the first available one.
fn peer_from_resolved(info: &mdns_sd::ResolvedService, our_session_id: &str) -> Option<Peer> {
    let txt = &info.txt_properties;
    let sid = txt.get_property_val_str("sid").unwrap_or("");
    if sid.is_empty() || sid == our_session_id {
        return None;
    }
    let compatible = txt
        .get_property_val_str("nv")
        .and_then(|s| s.parse::<u32>().ok())
        == Some(protocol::PROTOCOL_VERSION);
    if !compatible {
        return None;
    }

    let ips: Vec<IpAddr> = info.addresses.iter().map(|s| s.to_ip_addr()).collect();
    let addr = ips
        .iter()
        .find(|ip| ip.is_ipv4())
        .copied()
        .or_else(|| {
            ips.iter()
                .find(|ip| match ip {
                    // Link-local v6 (fe80::/10) is not connectable without a scope id: avoid it.
                    IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
                    IpAddr::V4(_) => false,
                })
                .copied()
        })
        .or_else(|| ips.first().copied())?;

    let name = txt.get_property_val_str("name").unwrap_or("?").to_string();
    let status = PeerStatus::from_str(txt.get_property_val_str("status").unwrap_or("available"));
    let announced_rating = txt
        .get_property_val_str("rt")
        .and_then(|s| s.parse::<i32>().ok());
    let uuid_hex = txt
        .get_property_val_str("uid")
        .filter(|u| u.len() == 32)
        .map(str::to_string);
    Some(Peer {
        name,
        addr,
        port: info.port,
        session_id: sid.to_string(),
        status,
        announced_rating,
        uuid_hex,
    })
}

/// Generate a unique session ID from system time and process ID.
/// Not globally unique, which is enough for LAN disambiguation. Contains no `.` (valid DNS label).
fn generate_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let combined = (nanos as u64) ^ ((pid as u64) << 32);
    format!("{:016x}", combined)
}

/// Info about the local network interface (IPv4, used for the "Your IP" help).
#[derive(Debug, Clone)]
pub struct LocalNetInfo {
    /// Local IPv4 address.
    pub ip: Ipv4Addr,
}

/// Detect the local IPv4 address by binding a UDP socket.
/// Returns None if no suitable address is found.
pub fn detect_local_network() -> Option<LocalNetInfo> {
    // Connect to a non-routable address to find the default interface
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("10.254.254.254:1").ok()?;
    let local_addr = socket.local_addr().ok()?;

    match local_addr.ip() {
        IpAddr::V4(ip) => {
            if ip.is_loopback() || ip.is_unspecified() {
                return None;
            }
            Some(LocalNetInfo { ip })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End to end: two discovery services on the same machine see each other through
    /// **mDNS** (dual-stack v4/v6). Best effort: skipped if the environment (sandbox)
    /// blocks mDNS.
    #[test]
    fn two_services_discover_each_other() {
        let (a, b) = match (
            DiscoveryService::start("Alice", 50001, Some(1512), &"ab".repeat(16)),
            DiscoveryService::start("Bob", 50002, None, &"cd".repeat(16)),
        ) {
            (Ok(a), Ok(b)) => (a, b),
            _ => {
                // With CI_NET=1 mDNS is required.
                if std::env::var_os("CI_NET").is_some() {
                    panic!("CI_NET=1: mDNS discovery service required");
                }
                eprintln!("Skipping: discovery service unavailable");
                return;
            }
        };
        let (mut a_sees_b, mut b_sees_a) = (false, false);
        for _ in 0..60 {
            for ev in a.poll_events() {
                if let DiscoveryEvent::PeerUpdated(p) = ev {
                    if p.name == "Bob" {
                        a_sees_b = true;
                        // Bob announces no rating (unplaced), so TXT `rt` is absent.
                        assert_eq!(p.announced_rating, None, "Bob must not announce an rt");
                        assert_eq!(p.uuid_hex.as_deref(), Some("cd".repeat(16).as_str()));
                    }
                }
            }
            for ev in b.poll_events() {
                if let DiscoveryEvent::PeerUpdated(p) = ev {
                    if p.name == "Alice" {
                        b_sees_a = true;
                        // Alice announces 1512, received as is through TXT `rt`.
                        assert_eq!(p.announced_rating, Some(1512), "Alice's rt not received");
                    }
                }
            }
            if a_sees_b && b_sees_a {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            a_sees_b && b_sees_a,
            "mDNS discovery failed (a_sees_b={}, b_sees_a={})",
            a_sees_b,
            b_sees_a
        );
    }

    #[test]
    fn peer_status_round_trip() {
        // "available", "in_game" and "busy" are distinct and stable (mDNS TXT).
        assert_eq!(PeerStatus::Busy.as_str(), "busy");
        assert_eq!(PeerStatus::InGame.as_str(), "in_game");
        assert_eq!(PeerStatus::Available.as_str(), "available");
        assert_eq!(PeerStatus::from_str("busy"), PeerStatus::Busy);
        assert_eq!(PeerStatus::from_str("in_game"), PeerStatus::InGame);
        assert_eq!(PeerStatus::from_str("available"), PeerStatus::Available);
        assert_eq!(PeerStatus::from_str("???"), PeerStatus::Available); // safe fallback
    }

    #[test]
    fn instance_extracted_from_fullname() {
        let full = format!("deadbeef0badf00d.{}", SERVICE_TYPE);
        assert_eq!(instance_of(&full).as_deref(), Some("deadbeef0badf00d"));
        assert_eq!(instance_of("garbage"), None);
    }

    #[test]
    fn session_id_is_unique() {
        let id1 = generate_session_id();
        std::thread::sleep(Duration::from_millis(1));
        let id2 = generate_session_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 16);
        assert_eq!(id2.len(), 16);
    }

    #[test]
    fn detect_local_network_result_is_coherent() {
        // The result depends on the machine's network setup (None is fine when offline),
        // but if an interface is detected its structural invariants must hold.
        if let Some(info) = detect_local_network() {
            assert!(!info.ip.is_loopback(), "the detected IP must not be loopback");
            assert!(!info.ip.is_unspecified(), "the detected IP must not be 0.0.0.0");
        }
    }
}
