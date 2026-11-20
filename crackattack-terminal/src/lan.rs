// The other players of a local network, found over mDNS: the same questions as the lobby of
// a server (who is there, what are they doing, where do I reach the one who waits), answered
// by the announcements the graphical game also makes (`crackattack_net::discovery`).
//
// The announcement of a game says: its pseudo, and what it does. A game at the menu is
// "busy", one in a game "in game", one on the `Vs player` page "available", and then, and
// only then, it listens on a port of its own and announces it. Outside that page the
// announcement carries no port: nothing listens.

use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, Instant};

use crackattack_net::discovery::{DiscoveryCommand, DiscoveryEvent, DiscoveryService, Peer, PeerStatus};

use crate::lobby::{notices, pseudo, Notice, Player, State};
use crate::render::versus::clean_name;

/// Who arrived is not told during the first moments: the others who were already there are
/// found one by one, and they did not just arrive.
const SETTLE: Duration = Duration::from_secs(3);

pub(crate) struct Lan {
    name: String,
    service: DiscoveryService,
    /// The peers by session id, as announced.
    peers: BTreeMap<String, Peer>,
    others: Vec<Player>,
    started: Instant,
}

/// What an announcement says a player is doing.
fn state_of(peer: &Peer) -> State {
    match peer.status {
        PeerStatus::Available => State::Waiting(peer.port),
        PeerStatus::InGame => State::Playing,
        PeerStatus::Busy => State::Menu,
    }
}

/// The players to show: those on our protocol version, by name, never ourselves (the
/// service already leaves our own announcement out; this one is for a namesake).
fn players(peers: &BTreeMap<String, Peer>, me: &str) -> Vec<Player> {
    let mut all: Vec<Player> = peers.values()
        .filter(|p| p.compatible && (p.status != PeerStatus::Available || p.port != 0))
        .map(|p| Player { name: clean_name(&p.name), state: state_of(p), ip: p.addr })
        .filter(|p| p.name != me)
        .collect();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    // Two players with the same pseudo cannot be told apart in a list: the first stays.
    all.dedup_by(|b, a| a.name == b.name);
    all
}

impl Lan {
    /// Starts announcing, with the pseudo at `start` of the list.
    pub(crate) fn join(start: usize, name: Option<&str>) -> io::Result<Lan> {
        let name = name.map_or_else(|| pseudo(start).to_string(), str::to_string);
        let uuid = format!("{:016x}{:016x}", start as u64 ^ std::process::id() as u64, crate::random_seed() as u64 * 0x9e37_79b9);
        // No listener yet: the port is announced when the game waits.
        let service = DiscoveryService::start(&name, 0, None, &uuid)?;
        service.send_command(DiscoveryCommand::UpdateAnnounce(PeerStatus::Busy, 0));
        Ok(Lan { name, service, peers: BTreeMap::new(), others: Vec::new(), started: Instant::now() })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn others(&self) -> &[Player] {
        &self.others
    }

    /// Says what this player is doing.
    pub(crate) fn set_state(&self, state: &State) {
        let (status, port) = match state {
            State::Menu => (PeerStatus::Busy, 0),
            State::Playing | State::Duel(_) => (PeerStatus::InGame, 0),
            State::Waiting(port) => (PeerStatus::Available, *port),
        };
        self.service.send_command(DiscoveryCommand::UpdateAnnounce(status, port));
    }

    /// Takes in what the network announced since the last look.
    pub(crate) fn poll(&mut self) -> Vec<Notice> {
        let events = self.service.poll_events();
        if events.is_empty() {
            return Vec::new();
        }
        for event in events {
            match event {
                DiscoveryEvent::PeerUpdated(peer) => {
                    self.peers.insert(peer.session_id.clone(), peer);
                }
                DiscoveryEvent::PeerExpired(session_id) => {
                    self.peers.remove(&session_id);
                }
            }
        }
        let after = players(&self.peers, &self.name);
        let news = if self.started.elapsed() < SETTLE { Vec::new() } else { notices(&self.others, &after) };
        self.others = after;
        news
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn peer(sid: &str, name: &str, status: PeerStatus, port: u16, compatible: bool) -> Peer {
        Peer {
            name: name.to_string(), addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 7)), port, session_id: sid.to_string(),
            status, announced_rating: None, uuid_hex: None, compatible,
        }
    }

    #[test]
    fn an_announcement_is_a_state_and_only_a_player_who_waits_has_a_port() {
        let mut peers = BTreeMap::new();
        for p in [
            peer("a", "Ann", PeerStatus::Available, 4001, true),
            peer("b", "Bob", PeerStatus::Busy, 0, true),
            peer("c", "Cy", PeerStatus::InGame, 0, true),
            peer("d", "Old", PeerStatus::Available, 4002, false),
            peer("e", "Me", PeerStatus::Busy, 0, true),
            peer("f", "Ann", PeerStatus::Busy, 0, true),
        ] {
            peers.insert(p.session_id.clone(), p);
        }
        let shown = players(&peers, "Me");
        let names: Vec<&str> = shown.iter().map(|p| p.name.as_str()).collect();
        // Not the other protocol version, not a namesake of ours, one Ann (the first).
        assert_eq!(names, ["Ann", "Bob", "Cy"]);
        assert_eq!(shown[0].state, State::Waiting(4001));
        assert_eq!((&shown[1].state, &shown[2].state), (&State::Menu, &State::Playing));
        assert_eq!(shown[0].ip, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 7)));
    }

    #[test]
    fn a_name_from_the_network_cannot_carry_control_codes() {
        let mut peers = BTreeMap::new();
        let p = peer("a", "\x1b[2JEve\x07", PeerStatus::Busy, 0, true);
        peers.insert(p.session_id.clone(), p);
        let shown = players(&peers, "Me");
        assert!(shown[0].name.chars().all(|c| c.is_ascii_graphic() || c == ' '));
    }
}
