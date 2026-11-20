// A duel against another player of the same server: looking for an opponent among the games
// that share the lobby (a server's, or the local network's), and joining or being joined.
//
// A player waits on the `Vs player` page (it listens on a port of its own and publishes it in
// the lobby, and sees the others who wait), and picks one of them: the picker connects to the
// one picked. The game itself is the normal network game of `crackattack-net`: this module
// only brings the two sides together.

use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crackattack_core::vs::{GameSession, VsSession};
use crackattack_net::communicator::{
    perform_handshake, recv_invite_name_timeout, send_invite_name, send_invite_response, try_recv_invite_response,
    HandshakeResult, InviteResponse,
};
use crackattack_net::connection::{NetConnection, Role};
use crackattack_net::opponent::NetworkOpponent;
use crackattack_net::session::VsNetworkGameSession;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal;

use crate::lobby::{Presence, State};
use crate::render::versus::clean_name;
use crate::{present, render, Ending, Limits};

/// A peer that could not be reached is left alone for this long before a new try.
const RETRY_AFTER: Duration = Duration::from_secs(2);
/// A player chosen from the list is tried for this long.
const CHOSEN_FOR: Duration = Duration::from_secs(8);
/// A person may have to accept an invitation (the graphical game asks them): waited for this long.
const INVITE_WAIT: Duration = Duration::from_secs(30);
/// Somebody who connects to a game that waits has this long to say who they are.
const NAME_WAIT: Duration = Duration::from_secs(10);
/// The loop wakes up this often: to draw, to read a key, to look for an opponent.
const TICK: Duration = Duration::from_millis(50);

/// An opponent found: the connection is made and the handshake is done.
pub(crate) struct Opponent {
    conn: NetConnection,
    role: Role,
    handshake: HandshakeResult,
}

/// How the search ended.
pub(crate) enum Found {
    Opponent(Opponent),
    /// The player went back to the menu.
    Back,
    /// A limit was reached, or the terminal is gone.
    Ending(Ending),
}

/// The handshake over a connection just made. `None` if it fails (the peer went away, or
/// does not answer within the delay of each step).
fn shake(mut conn: NetConnection, role: Role, name: &str, uuid: [u8; 16], seed: u32) -> Option<Opponent> {
    // The listening port is not announced: nobody reconnects to a game of this kind.
    let handshake = perform_handshake(&mut conn, role, name, uuid, 0, seed).ok()?;
    Some(Opponent { conn, role, handshake })
}

/// The network game against `opponent`, and the name to show for them.
pub(crate) fn session(opponent: Opponent) -> (Box<dyn GameSession>, String) {
    // The name comes from another process: cleaned before it is shown or kept.
    let name = clean_name(&opponent.handshake.opponent_name);
    let peer = NetworkOpponent::new(opponent.conn, opponent.role, name.clone(), opponent.handshake.opponent_uuid);
    let mut vs = VsSession::new(opponent.handshake.seed, peer);
    // The server wins the ties.
    vs.set_win_ties(opponent.role == Role::Server);
    (Box::new(VsNetworkGameSession::new(vs)), name)
}

/// An identity for the handshake, different for each game.
fn identity(seed: u32) -> [u8; 16] {
    let mut uuid = [0u8; 16];
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    uuid[..4].copy_from_slice(&seed.to_le_bytes());
    uuid[4..8].copy_from_slice(&std::process::id().to_le_bytes());
    uuid[8..].copy_from_slice(&(nanos as u64).to_le_bytes());
    uuid
}

/// How the other side answered an invitation, or why there was none.
enum Attempt {
    Opponent(Opponent),
    /// They said no (or are busy): not tried again.
    Declined,
    /// Nobody there, or the connection broke: may be tried again.
    Failed,
}

/// Invites the player at `address` and shakes hands, as the graphical game does: our name
/// first, then their answer (a person may have to say yes: it is waited for a good while), then
/// the handshake. Meant for a thread of its own: it blocks. `cancel` ends the wait.
fn invite_and_shake(address: SocketAddr, name: &str, uuid: [u8; 16], cancel: &AtomicBool) -> Attempt {
    let Ok(mut conn) = NetConnection::connect(&address.ip().to_string(), address.port()) else {
        return Attempt::Failed;
    };
    if send_invite_name(&mut conn, name, None).is_err() {
        return Attempt::Failed;
    }
    let deadline = Instant::now() + INVITE_WAIT;
    let response = loop {
        if cancel.load(Ordering::Relaxed) || Instant::now() > deadline {
            return Attempt::Failed;
        }
        match try_recv_invite_response(&mut conn) {
            Ok(Some(response)) => break response,
            Ok(None) => {}
            Err(_) => return Attempt::Failed,
        }
    };
    if response != InviteResponse::Accept {
        return Attempt::Declined;
    }
    match shake(conn, Role::Client, name, uuid, 0) {
        Some(opponent) => Attempt::Opponent(opponent),
        None => Attempt::Failed,
    }
}

/// The other side of `invite_and_shake`: somebody connected to us. Reads who they are, accepts
/// (a game that waits on the `Vs player` page has said yes to a duel already), and shakes
/// hands as the server, dealing the first board (`seed`). `None` if it fails or they leave.
fn accept_and_shake(stream: TcpStream, addr: SocketAddr, name: &str, uuid: [u8; 16], seed: u32) -> Option<Opponent> {
    // A socket accepted from a non-blocking listener may be non-blocking too.
    stream.set_nonblocking(false).ok()?;
    let mut conn = NetConnection::from_stream(stream, addr);
    recv_invite_name_timeout(&mut conn, NAME_WAIT).ok()?;
    send_invite_response(&mut conn, InviteResponse::Accept).ok()?;
    shake(conn, Role::Server, name, uuid, seed)
}

/// A game that waits for a duel: it listens on a port the system chooses (on 127.0.0.1 for the
/// games of a server, so nothing outside the machine can reach it; on every interface for
/// those of a network), and says so in the lobby. Whoever picks it in their list connects to
/// that port. Dropping it stops the listening; the caller says what the game does next.
pub(crate) struct Waiting {
    listener: TcpListener,
    /// A visitor being dealt with, in a thread of its own so that the screen goes on.
    visitor: Option<mpsc::Receiver<Option<Opponent>>>,
}

impl Waiting {
    /// Starts waiting and publishes the port, on the address the presence says. `None` if no
    /// port can be had.
    pub(crate) fn open(presence: &Presence) -> Option<Waiting> {
        let room = Waiting::bind(presence.listen_ip(), 0).ok()?;
        presence.set_state(&State::Waiting(room.listener.local_addr().ok()?.port()));
        Some(room)
    }

    /// Listens on `address` and `port` (0: one the system chooses).
    pub(crate) fn bind(address: IpAddr, port: u16) -> std::io::Result<Waiting> {
        let listener = TcpListener::bind((address, port))?;
        listener.set_nonblocking(true)?;
        Ok(Waiting { listener, visitor: None })
    }

    /// The port it listens on.
    pub(crate) fn port(&self) -> u16 {
        self.listener.local_addr().map_or(0, |a| a.port())
    }

    /// Somebody who picked us: the opponent, once the handshake is done. This game deals the
    /// first board (`seed`). `None` while nobody came, the handshake is under way, or it failed.
    /// Never waits.
    pub(crate) fn accept(&mut self, name: &str, seed: u32) -> Option<Opponent> {
        if let Some(visitor) = &self.visitor {
            return match visitor.try_recv() {
                Ok(found) => {
                    self.visitor = None;
                    found
                }
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.visitor = None;
                    None
                }
            };
        }
        let (stream, addr) = self.listener.accept().ok()?;
        let (name, uuid) = (name.to_string(), identity(seed));
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(accept_and_shake(stream, addr, &name, uuid, seed));
        });
        self.visitor = Some(receiver);
        None
    }
}

/// What a search for an opponent is after.
pub(crate) enum Goal<'a> {
    /// The player of that name, who waits (found in the lobby).
    Player(&'a str),
    /// The game at this address, which may be anywhere (`--vs network-join`).
    Address(SocketAddr),
    /// Nobody in particular: wait to be invited (`--vs network-create`, or a room kept open).
    Host,
}

/// Looks for an opponent for `goal` until one is found, the player goes back, or a limit is
/// reached. A refusal or a player who is gone ends it at once; a connection that fails is
/// tried again for a few seconds. `waiting`: this game's own waiting room, if open: it is
/// answered while the search goes on (two players who pick each other at once: the one
/// whose pseudo comes first keeps waiting, the other one connects). `presence` may be absent
/// (a game started from the command line, without a lobby). The terminal is already taken by
/// the caller.
pub(crate) fn search(
    mut presence: Option<&mut Presence>,
    name: &str,
    limits: &Limits,
    compact: bool,
    seed: u32,
    goal: Goal,
    mut waiting: Option<Waiting>,
) -> Found {
    let heading = match &goal {
        Goal::Player(target) => format!("Joining {}", target),
        Goal::Address(address) => format!("Inviting {}", address),
        Goal::Host => match waiting.as_ref() {
            Some(room) => format!("Waiting for an invitation on port {}", room.port()),
            None => "Waiting for an invitation".to_string(),
        },
    };
    let started = Instant::now();
    let mut renderer = render::Renderer::new();
    let uuid = identity(seed);
    let cancel = Arc::new(AtomicBool::new(false));
    // The attempt under way, in a thread of its own, and when the last one failed.
    let mut attempt: Option<mpsc::Receiver<Attempt>> = None;
    let mut failed_at: Option<Instant> = None;
    let mut last_activity = Instant::now();
    // Keys pressed before (the choice in the list) do not cancel anything.
    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    let found = loop {
        if let Some(presence) = presence.as_deref_mut() {
            presence.poll();
        }
        // A game that waits: somebody may pick it.
        if let Some(opponent) = waiting.as_mut().and_then(|room| room.accept(name, seed)) {
            break Found::Opponent(opponent);
        }
        if let Some(under_way) = &attempt {
            match under_way.try_recv() {
                Ok(Attempt::Opponent(opponent)) => break Found::Opponent(opponent),
                Ok(Attempt::Declined) => break Found::Back,
                Ok(Attempt::Failed) | Err(mpsc::TryRecvError::Disconnected) => {
                    attempt = None;
                    failed_at = Some(Instant::now());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        } else if !matches!(goal, Goal::Host) {
            let address = match &goal {
                Goal::Player(target) => presence.as_deref()
                    .and_then(|p| p.waiting().into_iter().find(|(other, _)| other == target))
                    .map(|(_, address)| address),
                Goal::Address(address) => Some(*address),
                Goal::Host => None,
            };
            match address {
                // Gone, or tried for too long.
                None => break Found::Back,
                Some(_) if started.elapsed() > CHOSEN_FOR => break Found::Back,
                Some(address) if failed_at.is_none_or(|at| at.elapsed() >= RETRY_AFTER) => {
                    let (sender, receiver) = mpsc::channel();
                    let (name, cancel) = (name.to_string(), cancel.clone());
                    std::thread::spawn(move || {
                        let _ = sender.send(invite_and_shake(address, &name, uuid, &cancel));
                    });
                    attempt = Some(receiver);
                }
                Some(_) => {}
            }
        }

        let size = terminal::size().ok().map(|(columns, rows)| (columns as usize, rows as usize));
        renderer.set_placement(render::place(size, compact));
        let view = presence.as_deref().map(|p| p.view());
        if !present(&renderer.menu_list(&heading, &[], 0, "Q: back to the menu", view.as_ref())) {
            break Found::Ending(Ending::Hangup);
        }
        match event::poll(TICK) {
            Err(_) => break Found::Ending(Ending::Hangup),
            Ok(false) => {}
            Ok(true) => match event::read() {
                Err(_) => break Found::Ending(Ending::Hangup),
                Ok(Event::Key(KeyEvent { code, kind: KeyEventKind::Press, .. })) => {
                    last_activity = Instant::now();
                    if matches!(code, KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc) {
                        break Found::Back;
                    }
                }
                Ok(_) => {}
            },
        }
        if let Some(ending) = limits.reached(last_activity, Instant::now()) {
            break Found::Ending(ending);
        }
    };
    // An attempt still going stops (its thread ends by itself), and we are not on the list of
    // those who wait any more, whatever came of it.
    cancel.store(true, Ordering::Relaxed);
    if let Some(presence) = presence.as_deref() {
        presence.set_state(&State::Menu);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The graphical game, as far as a duel goes: it invites (name, then waits for the answer,
    /// then the handshake as the client), or it is invited (reads the name, answers, then the
    /// handshake as the server).
    fn graphical_invites(address: SocketAddr) -> std::thread::JoinHandle<bool> {
        std::thread::spawn(move || {
            let mut conn = NetConnection::connect(&address.ip().to_string(), address.port()).unwrap();
            send_invite_name(&mut conn, "Zoé", Some(1500)).unwrap();
            let response = loop {
                if let Some(response) = try_recv_invite_response(&mut conn).unwrap() {
                    break response;
                }
            };
            response == InviteResponse::Accept && perform_handshake(&mut conn, Role::Client, "Zoé", [9; 16], 45678, 0).is_ok()
        })
    }

    fn graphical_is_invited(listener: TcpListener, answer: InviteResponse) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let (stream, addr) = listener.accept().unwrap();
            let mut conn = NetConnection::from_stream(stream, addr);
            let (name, _) = recv_invite_name_timeout(&mut conn, Duration::from_secs(5)).unwrap();
            assert_eq!(name, "Pixel");
            send_invite_response(&mut conn, answer).unwrap();
            if answer == InviteResponse::Accept {
                perform_handshake(&mut conn, Role::Server, "Zoé", [8; 16], 45678, 7).unwrap();
            }
        })
    }

    #[test]
    fn a_game_that_waits_answers_the_invitation_of_the_graphical_game() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let guest = graphical_invites(address);
        let (stream, peer) = listener.accept().unwrap();
        let opponent = accept_and_shake(stream, peer, "Pixel", [1; 16], 5).expect("no duel");
        assert_eq!(opponent.role, Role::Server);
        assert_eq!(opponent.handshake.opponent_name, "Zoé");
        assert!(guest.join().unwrap(), "the graphical side did not get its answer");
    }

    #[test]
    fn a_game_that_joins_invites_first_and_gives_up_on_a_refusal() {
        let cancel = AtomicBool::new(false);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host = graphical_is_invited(listener, InviteResponse::Accept);
        match invite_and_shake(address, "Pixel", [1; 16], &cancel) {
            Attempt::Opponent(opponent) => {
                assert_eq!(opponent.role, Role::Client);
                assert_eq!(opponent.handshake.seed, 7, "the seed is the host's");
            }
            _ => panic!("no duel"),
        }
        host.join().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host = graphical_is_invited(listener, InviteResponse::Decline);
        assert!(matches!(invite_and_shake(address, "Pixel", [1; 16], &cancel), Attempt::Declined));
        host.join().unwrap();
        // Nobody listens there: a failure that may be tried again, not a refusal.
        assert!(matches!(invite_and_shake(address, "Pixel", [1; 16], &cancel), Attempt::Failed));
    }

    #[test]
    fn an_identity_differs_from_one_game_to_the_next() {
        assert_ne!(identity(1), identity(2));
    }
}
