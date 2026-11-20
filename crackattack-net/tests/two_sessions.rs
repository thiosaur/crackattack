// Phase 0 of the ssh duel plan: two network sessions of the normal game, in two threads of one
// process, joined by 127.0.0.1 and each played by the grid AI, with no display. It checks what
// the terminal will rely on: the four-call construction, the opponent preview, the end of a
// round seen from both sides, and the forfeit when one side leaves.

use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crackattack_core::ai_difficulty::AIDifficulty;
use crackattack_core::frame_data::{SessionEvent, SessionInterruption, SessionLifecycle};
use crackattack_core::input::InputSnapshot;
use crackattack_core::vs::{GameSession, GridAutoplay, VsSession};
use crackattack_net::communicator::perform_handshake;
use crackattack_net::connection::{NetConnection, Role};
use crackattack_net::opponent::NetworkOpponent;
use crackattack_net::session::VsNetworkGameSession;

/// What one side saw.
#[derive(Debug, Default)]
struct Seen {
    opponent_name: String,
    /// The opponent's preview held at least one block at some point.
    preview_with_blocks: bool,
    /// Outcome of each round, in order (`true` = this side won).
    rounds: Vec<bool>,
    /// The session said the opponent left (forfeit or disconnect).
    opponent_left: bool,
    ticks: u32,
}

/// The four calls the terminal will make, from an established connection.
fn build(mut conn: NetConnection, role: Role, name: &str, uuid: u8, seed: u32) -> (VsNetworkGameSession, u32) {
    let handshake = perform_handshake(&mut conn, role, name, [uuid; 16], 0, seed).expect("handshake");
    let opponent = NetworkOpponent::new(conn, role, handshake.opponent_name.clone(), handshake.opponent_uuid);
    let mut vs = VsSession::new(handshake.seed, opponent);
    vs.set_win_ties(role == Role::Server);
    (VsNetworkGameSession::new(vs), handshake.seed)
}

/// Plays like the terminal's autoplay until `rounds` rounds ended, or the other side left, or
/// the time is up. `leave_after_round`: abandon as soon as the first round has ended.
/// `together`: the sides that must all have seen their rounds end before any of them stops
/// (a side that stops closes the connection, and the other would see a departure instead of
/// the end of its round: the round does not end on both sides at the same instant).
fn play(mut session: VsNetworkGameSession, rounds: usize, leave_after_round: bool, limit: Duration, together: Option<(Arc<AtomicUsize>, usize)>) -> Seen {
    let mut counted = false;
    let mut ai = GridAutoplay::new(AIDifficulty::Hard, 99);
    let mut seen = Seen::default();
    let mut prev_state = 0u8;
    let start = Instant::now();
    let others_pending = |counted: bool| match &together {
        Some((done, sides)) => done.load(Ordering::SeqCst) < *sides || !counted,
        None => false,
    };
    while start.elapsed() < limit && (seen.rounds.len() < rounds || others_pending(counted)) {
        let before = session.snapshot();
        let mut input = InputSnapshot::new();
        let ready = matches!(before.interruption, SessionInterruption::ReadyPrompt { opponent_present: true, .. });
        if ready {
            session.signal_local_ready();
        } else if before.lifecycle == SessionLifecycle::Playing && before.interruption == SessionInterruption::None {
            if let Some(game) = session.game_state_mut() {
                input.state = ai.update(game);
            }
        }
        input.pressed_this_frame = input.state & !prev_state;
        input.prev_state = prev_state;
        prev_state = input.state;
        session.tick(&input);
        for event in session.drain_events() {
            if let SessionEvent::RoundEnded { won } = event {
                if seen.rounds.len() < rounds {
                    seen.rounds.push(won);
                }
            }
        }
        if seen.rounds.len() >= rounds && !counted {
            counted = true;
            if let Some((done, _)) = &together {
                done.fetch_add(1, Ordering::SeqCst);
            }
        }
        let after = session.snapshot();
        if let Some(display) = &after.vs_display {
            seen.opponent_name = display.opponent_name.clone();
            if let Some(view) = &display.opponent_board {
                seen.preview_with_blocks |= (0..6).any(|x| (0..13).any(|y| !matches!(view.cell(x, y), crackattack_core::vs::BoardCell::Empty)));
            }
        }
        if matches!(after.interruption, SessionInterruption::Disconnected)
            || matches!(after.interruption, SessionInterruption::ReadyPrompt { opponent_present: false, .. })
        {
            seen.opponent_left = true;
            break;
        }
        if leave_after_round && !seen.rounds.is_empty() {
            session.abandon();
            // Give the network thread the time to send the forfeit before the sockets close.
            thread::sleep(Duration::from_millis(300));
            break;
        }
        seen.ticks += 1;
        thread::sleep(Duration::from_micros(200));
    }
    seen
}

/// A connected pair (server side first), or `None` where loopback is not allowed.
fn pair() -> Option<(NetConnection, NetConnection)> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    let client = thread::spawn(move || NetConnection::connect("127.0.0.1", port).unwrap());
    let (stream, addr) = listener.accept().ok()?;
    Some((NetConnection::from_stream(stream, addr), client.join().unwrap()))
}

#[test]
fn two_sessions_play_a_round_and_see_the_same_result() {
    let Some((server_conn, client_conn)) = pair() else { return };
    let done = Arc::new(AtomicUsize::new(0));
    let together = done.clone();
    let server = thread::spawn(move || {
        let (session, seed) = build(server_conn, Role::Server, "Alpha", 1, 4242);
        (play(session, 1, false, Duration::from_secs(90), Some((together, 2))), seed)
    });
    let client = thread::spawn(move || {
        let (session, seed) = build(client_conn, Role::Client, "Bravo", 2, 0);
        (play(session, 1, false, Duration::from_secs(90), Some((done, 2))), seed)
    });
    let ((a, seed_a), (b, seed_b)) = (server.join().unwrap(), client.join().unwrap());
    eprintln!("server saw {:?}\nclient saw {:?}", a, b);
    // The server dealt the seed, the client got it.
    assert_eq!((seed_a, seed_b), (4242, 4242));
    assert_eq!((a.opponent_name.as_str(), b.opponent_name.as_str()), ("Bravo", "Alpha"));
    assert!(a.preview_with_blocks && b.preview_with_blocks, "no opponent preview: {:?} {:?}", a, b);
    // One round, one winner: what is a win for one side is a loss for the other.
    assert_eq!((a.rounds.len(), b.rounds.len()), (1, 1), "round did not end on both sides: {:?} {:?}", a, b);
    assert_ne!(a.rounds[0], b.rounds[0], "both sides saw the same outcome");
}

#[test]
fn leaving_after_a_round_tells_the_other_side() {
    let Some((server_conn, client_conn)) = pair() else { return };
    let server = thread::spawn(move || {
        let (session, _) = build(server_conn, Role::Server, "Alpha", 1, 7);
        play(session, 5, false, Duration::from_secs(90), None)
    });
    let client = thread::spawn(move || {
        let (session, _) = build(client_conn, Role::Client, "Bravo", 2, 0);
        play(session, 5, true, Duration::from_secs(90), None)
    });
    let (a, b) = (server.join().unwrap(), client.join().unwrap());
    eprintln!("stayer saw {:?}\nleaver saw {:?}", a, b);
    assert!(a.opponent_left, "the side that stayed was not told: {:?}", a);
}

/// Ticks at the real rate (50 Hz), says nothing at the ready prompt for `wait`, then signals
/// ready, and reports whether the round started and whether the connection was ever lost.
fn wait_then_ready(mut session: VsNetworkGameSession, wait: Duration, watch: Duration) -> (bool, bool) {
    let start = Instant::now();
    let (mut playing, mut lost) = (false, false);
    while start.elapsed() < wait + watch {
        let snapshot = session.snapshot();
        let input = InputSnapshot::new();
        if start.elapsed() >= wait && matches!(snapshot.interruption, SessionInterruption::ReadyPrompt { opponent_present: true, .. }) {
            session.signal_local_ready();
        }
        playing |= snapshot.lifecycle == SessionLifecycle::Playing && snapshot.interruption == SessionInterruption::None;
        lost |= matches!(snapshot.interruption, SessionInterruption::ConnectionLost { .. } | SessionInterruption::Disconnected);
        session.tick(&input);
        session.drain_events();
        thread::sleep(Duration::from_millis(20));
    }
    (playing, lost)
}

#[test]
fn a_long_wait_at_the_ready_prompt_does_not_lose_the_connection() {
    let Some((server_conn, client_conn)) = pair() else { return };
    let wait = Duration::from_secs(12);
    let server = thread::spawn(move || {
        let (session, _) = build(server_conn, Role::Server, "Alpha", 1, 11);
        wait_then_ready(session, wait, Duration::from_secs(6))
    });
    let client = thread::spawn(move || {
        let (session, _) = build(client_conn, Role::Client, "Bravo", 2, 0);
        wait_then_ready(session, wait, Duration::from_secs(6))
    });
    let (a, b) = (server.join().unwrap(), client.join().unwrap());
    eprintln!("server (playing, lost) = {:?}, client = {:?}", a, b);
    assert_eq!((a.1, b.1), (false, false), "the connection was lost");
    assert_eq!((a.0, b.0), (true, true), "the round did not start on both sides");
}

#[test]
fn leaving_in_the_middle_of_a_round_tells_the_other_side_at_once() {
    let Some((server_conn, client_conn)) = pair() else { return };
    let leaver = thread::spawn(move || {
        let (mut session, _) = build(client_conn, Role::Client, "Bravo", 2, 0);
        // Ready, then play for two seconds of the round, then leave: the session is dropped.
        let start = Instant::now();
        let mut playing_since: Option<Instant> = None;
        while start.elapsed() < Duration::from_secs(30) {
            let snapshot = session.snapshot();
            if matches!(snapshot.interruption, SessionInterruption::ReadyPrompt { opponent_present: true, .. }) {
                session.signal_local_ready();
            }
            if snapshot.lifecycle == SessionLifecycle::Playing && snapshot.interruption == SessionInterruption::None {
                let since = *playing_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_secs(2) {
                    session.abandon();
                    return; // Dropped here, as the terminal does when it goes back to the menu.
                }
            }
            session.tick(&InputSnapshot::new());
            session.drain_events();
            thread::sleep(Duration::from_millis(20));
        }
    });
    let (mut session, _) = build(server_conn, Role::Server, "Alpha", 1, 5);
    let start = Instant::now();
    let mut told_after = None;
    let mut left_at = None;
    while start.elapsed() < Duration::from_secs(40) {
        let snapshot = session.snapshot();
        if matches!(snapshot.interruption, SessionInterruption::ReadyPrompt { opponent_present: true, .. }) {
            session.signal_local_ready();
        }
        if left_at.is_none() && leaver.is_finished() {
            left_at = Some(Instant::now());
        }
        if snapshot.interruption == SessionInterruption::Disconnected {
            told_after = left_at.map(|t| t.elapsed());
            break;
        }
        session.tick(&InputSnapshot::new());
        session.drain_events();
        thread::sleep(Duration::from_millis(20));
    }
    leaver.join().unwrap();
    eprintln!("told after {:?}", told_after);
    assert!(told_after.is_some_and(|d| d < Duration::from_secs(2)), "the side that stayed was not told: {:?}", told_after);
}
