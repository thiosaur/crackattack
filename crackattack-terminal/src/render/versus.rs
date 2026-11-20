// Duel against the computer: what the screen shows of the opponent and of the round.
//
// Everything here comes from what the session publishes (`SessionSnapshot`): the opponent's
// board as the simplified picture of `VsDisplay::opponent_board`, the round and the tally,
// the countdown and the ready prompt. Nothing reaches into the opponent itself, so the same
// screen serves a network opponent.

use crackattack_core::constants::{PLAY_WIDTH, SAFE_HEIGHT};
use crackattack_core::frame_data::{
    SessionInterruption, SessionLifecycle, SessionResult, SessionSnapshot,
};
use crackattack_core::vs::{BoardCell, OpponentBoardView};

use super::font::Banner;
use super::palette::{DANGER_RED, COUNTDOWN_GO_COLOR, COUNTDOWN_ONE_COLOR, GARBAGE_COLOR, HINT_COLOR, OPPONENT_BLOCK_COLOR, OPPONENT_DANGER_COLOR};

/// Colors of the big words: the round number, a round won, a round lost.
const ROUND_COLOR: u8 = COUNTDOWN_ONE_COLOR;
const WIN_COLOR: u8 = COUNTDOWN_GO_COLOR;
const LOSE_COLOR: u8 = DANGER_RED[0];

/// Columns of the preview: two per cell.
#[cfg(test)]
pub(crate) const PREVIEW_COLUMNS: usize = 2 * PLAY_WIDTH;
/// Lines of the preview: one per row the cursor can reach. The view also holds the row above
/// (`SAFE_HEIGHT`), which nobody can play in: it is not drawn, but it counts for the danger.
pub(crate) const PREVIEW_LINES: usize = SAFE_HEIGHT - 1;
/// Rows from the top where anything means the opponent is close to losing.
const DANGER_ROWS: usize = 3;

/// What a duel adds to the screen.
#[derive(Clone, Debug, Default)]
pub(crate) struct Versus {
    pub name: String,
    pub wins: i32,
    pub losses: i32,
    /// The opponent's board, as last published.
    pub preview: Option<OpponentBoardView>,
}

impl Versus {
    /// The duel part of the screen for this snapshot; `None` outside a duel.
    pub(crate) fn from_snapshot(snapshot: &SessionSnapshot) -> Option<Self> {
        let display = snapshot.vs_display.as_ref()?;
        Some(Self {
            name: clean_name(&display.opponent_name),
            wins: display.session_wins,
            losses: display.session_losses,
            preview: display.opponent_board.clone(),
        })
    }

    /// The panel line above the preview.
    pub(crate) fn title(&self) -> String {
        format!("\x1b[1m{}\x1b[22m  {}-{}", self.name, self.wins, self.losses)
    }

    /// Line `row` (0 = top) of the preview: the opponent's stack, two columns per cell.
    pub(crate) fn preview_line(&self, row: usize) -> String {
        let view_row = PREVIEW_LINES - 1 - row;
        // The whole stack changes color when anything reaches the top rows.
        let near_top = self.preview.as_ref().is_some_and(|view| {
            (0..=DANGER_ROWS).any(|top| (0..PLAY_WIDTH).any(|x| view.cell(x, PREVIEW_LINES - top) != BoardCell::Empty))
        });
        let mut out = String::new();
        for x in 0..PLAY_WIDTH {
            let cell = self.preview.as_ref().map_or(BoardCell::Empty, |view| view.cell(x, view_row));
            let (color, glyphs) = match cell {
                BoardCell::Empty => (HINT_COLOR, "· "),
                BoardCell::Block if near_top => (OPPONENT_DANGER_COLOR, "██"),
                BoardCell::Block => (OPPONENT_BLOCK_COLOR, "██"),
                BoardCell::Garbage(_) => (GARBAGE_COLOR, "▒▒"),
            };
            out.push_str(&format!("\x1b[38;5;{}m{}\x1b[39m", color, glyphs));
        }
        out
    }
}

/// The most characters of an opponent's name that are shown.
pub(crate) const NAME_MAX: usize = 16;

/// Latin letters with accents, and the plain letter each one is shown as (the graphical game
/// has players named José or Zoë: "Jose" and "Zoe" read better than "Jos" and "Zo").
const ACCENTS: &[(&str, char)] = &[
    ("àáâãäåāăą", 'a'), ("ÀÁÂÃÄÅĀĂĄ", 'A'), ("çćĉċč", 'c'), ("ÇĆĈĊČ", 'C'), ("ďđ", 'd'), ("ĎĐ", 'D'),
    ("èéêëēĕėęě", 'e'), ("ÈÉÊËĒĔĖĘĚ", 'E'), ("ĝğġģ", 'g'), ("ĜĞĠĢ", 'G'), ("ìíîïĩīĭįı", 'i'),
    ("ÌÍÎÏĨĪĬĮİ", 'I'), ("ĵ", 'j'), ("ķ", 'k'), ("ĺļľŀł", 'l'), ("ĹĻĽĿŁ", 'L'), ("ñńņň", 'n'),
    ("ÑŃŅŇ", 'N'), ("òóôõöøōŏő", 'o'), ("ÒÓÔÕÖØŌŎŐ", 'O'), ("ŕŗř", 'r'), ("ŔŖŘ", 'R'),
    ("śŝşšß", 's'), ("ŚŜŞŠ", 'S'), ("ţťŧ", 't'), ("ŢŤŦ", 'T'), ("ùúûüũūŭůűų", 'u'),
    ("ÙÚÛÜŨŪŬŮŰŲ", 'U'), ("ýÿ", 'y'), ("ÝŸ", 'Y'), ("źżž", 'z'), ("ŹŻŽ", 'Z'),
];

fn fold_accent(c: char) -> char {
    if c.is_ascii() {
        return c;
    }
    ACCENTS.iter().find(|(from, _)| from.contains(c)).map_or(c, |&(_, plain)| plain)
}

/// An opponent's name as it may reach the terminal. The name comes from the session, and
/// over a network from another process: control characters (escape sequences that would
/// act on the player's terminal) and anything not printable ASCII are dropped, the length
/// is capped, and an empty result becomes "Opponent".
pub(crate) fn clean_name(raw: &str) -> String {
    let name: String = raw.chars().map(fold_accent).filter(|c| c.is_ascii_graphic() || *c == ' ').take(NAME_MAX).collect();
    let name = name.trim();
    if name.is_empty() { "Opponent".to_string() } else { name.to_string() }
}

/// A message of plain text lines.
fn text(lines: &[&str]) -> Option<Banner> {
    Some(Banner { above: lines.iter().map(|l| l.to_string()).collect(), ..Banner::default() })
}

/// A message with big words (round number, result), and text around them.
fn big(above: &[&str], words: &[&str], color: u8, below: Vec<String>) -> Option<Banner> {
    Some(Banner {
        above: above.iter().map(|l| l.to_string()).collect(),
        words: words.iter().map(|w| w.to_string()).collect(),
        color,
        below,
    })
}

/// The message that covers the board, when the session asks something of the player,
/// waits for something, or is between two rounds. The countdown does not cover it: see
/// `Countdown`. Every interruption a session can publish has its message.
pub(crate) fn banner(snapshot: &SessionSnapshot) -> Option<Banner> {
    match snapshot.interruption {
        SessionInterruption::ReadyPrompt { opponent_present: false, .. } | SessionInterruption::Disconnected => {
            text(&["Opponent left", "", "Q: menu"])
        }
        SessionInterruption::ReadyPrompt { round_number, .. } => {
            let mut below = Vec::new();
            if let Some(display) = snapshot.vs_display.as_ref().filter(|_| round_number > 1) {
                below.push(format!("You {} - {} {}", display.session_wins, display.session_losses, clean_name(&display.opponent_name)));
            }
            below.push(if round_number > 1 { "Enter: play again" } else { "Enter: start" }.to_string());
            below.push("Q: menu".to_string());
            big(&["Round"], &[&round_number.to_string()], ROUND_COLOR, below)
        }
        SessionInterruption::Paused { is_network: false, .. } => text(&["PAUSE", "P: resume", "Q: quit"]),
        // A pause both players agree to: each says when ready to resume.
        SessionInterruption::Paused { is_network: true, local_ready, remote_ready } => {
            let mut lines = vec!["PAUSE"];
            if local_ready {
                lines.push("Waiting for the opponent");
            } else {
                if remote_ready {
                    lines.push("The opponent is ready");
                }
                lines.push("P: ready to resume");
            }
            lines.extend(["", "Q: menu"]);
            text(&lines)
        }
        SessionInterruption::WaitingRemoteReady { .. } => text(&["Waiting for the opponent", "", "Q: menu"]),
        SessionInterruption::ConnectionLost { .. } => {
            text(&["Connection lost", "Waiting for the opponent", "", "Q: menu"])
        }
        SessionInterruption::AwaitingRoundVerdict => text(&["Round over", "Waiting for the verdict"]),
        SessionInterruption::ResumeSync { .. } => text(&["Resuming"]),
        SessionInterruption::ScenePreparing | SessionInterruption::None => match (snapshot.lifecycle, snapshot.result) {
            (SessionLifecycle::ShowResult, SessionResult::RoundWin) => big(&[], &["YOU", "WIN"], WIN_COLOR, Vec::new()),
            (SessionLifecycle::ShowResult, SessionResult::RoundLose) => big(&[], &["YOU", "LOSE"], LOSE_COLOR, Vec::new()),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crackattack_core::vs::{GameSession, GridOpponent, VsComputerGameSession};
    use crackattack_core::ai_difficulty::AIDifficulty;

    fn session() -> VsComputerGameSession {
        let mut s = VsComputerGameSession::new(7, GridOpponent::new(AIDifficulty::Easy, 7));
        s.set_needs_ready_prompt(true);
        s
    }

    #[test]
    fn a_new_duel_asks_for_ready_before_round_one() {
        let snapshot = session().snapshot();
        let shown = banner(&snapshot).expect("no ready prompt");
        // "Round" in text, the number in big blocks, the key to press under it.
        assert_eq!((shown.above.as_slice(), shown.words.as_slice()), (&["Round".to_string()][..], &["1".to_string()][..]));
        assert!(shown.below.iter().any(|l| l.contains("Enter: start")));
    }

    #[test]
    fn the_preview_has_a_fixed_size_and_a_missing_board_is_dots() {
        let versus = Versus::from_snapshot(&session().snapshot()).unwrap();
        for row in 0..PREVIEW_LINES {
            assert_eq!(super::super::visible_width(&versus.preview_line(row)), PREVIEW_COLUMNS);
        }
        let none = Versus { preview: None, ..versus };
        for row in 0..PREVIEW_LINES {
            let line = none.preview_line(row);
            assert!(!line.contains('█') && !line.contains('▒'));
        }
    }

    /// The color numbers used by a preview line.
    fn colors(line: &str) -> Vec<u8> {
        let mut found: Vec<u8> = line.split("\x1b[38;5;").skip(1).filter_map(|p| p.split('m').next()?.parse().ok()).collect();
        found.dedup();
        found
    }

    #[test]
    fn the_whole_stack_is_green_gray_while_low_and_red_gray_at_the_top_and_garbage_stays_apart() {
        let stacked = |row: usize, byte: u8| {
            let mut bytes = OpponentBoardView::empty().to_bytes();
            bytes[0] = 1;                         // a block at the bottom, always
            bytes[row * PLAY_WIDTH + 1] = byte;   // and what we test, at `row`
            OpponentBoardView::from_bytes(&bytes).unwrap()
        };
        let versus = |preview| Versus { preview: Some(preview), ..Versus::default() };
        // Low: the blocks are green-gray, the garbage keeps its own color.
        let low = versus(stacked(0, 2));
        let bottom = colors(&low.preview_line(PREVIEW_LINES - 1));
        assert!(bottom.contains(&OPPONENT_BLOCK_COLOR) && bottom.contains(&GARBAGE_COLOR), "{:?}", bottom);
        // Something in the top rows: every block of the stack turns red-gray, even far below.
        let high = versus(stacked(PREVIEW_LINES - 1, 1));
        let bottom = colors(&high.preview_line(PREVIEW_LINES - 1));
        assert!(bottom.contains(&OPPONENT_DANGER_COLOR) && !bottom.contains(&OPPONENT_BLOCK_COLOR), "{:?}", bottom);
        assert!(OPPONENT_BLOCK_COLOR != GARBAGE_COLOR && OPPONENT_DANGER_COLOR != GARBAGE_COLOR);
    }

    fn snapshot_with(interruption: SessionInterruption) -> SessionSnapshot {
        SessionSnapshot { interruption, ..session().snapshot() }
    }

    fn text(banner: Option<Banner>) -> String {
        let banner = banner.expect("no message");
        [banner.above, banner.words, banner.below].concat().join("\n")
    }

    #[test]
    fn a_name_from_another_process_cannot_reach_the_terminal_as_control_codes() {
        let hostile = "\x1b]0;pwned\x07\x1b[2JEve\r\n\u{202e}\u{1f600}";
        let clean = clean_name(hostile);
        assert!(clean.chars().all(|c| c.is_ascii_graphic() || c == ' '), "{:?}", clean);
        assert!(clean.contains("Eve"));
        // Capped, never empty, and a plain name is left alone.
        assert_eq!(clean_name(&"x".repeat(100)).len(), NAME_MAX);
        assert_eq!(clean_name("\x1b\x07"), "Opponent");
        assert_eq!(clean_name("  "), "Opponent");
        assert_eq!(clean_name("Sigma"), "Sigma");
        // Accents are folded, other scripts are dropped.
        assert_eq!(clean_name("José Zoë Çağlar"), "Jose Zoe Caglar");
        assert_eq!(clean_name("Игрок"), "Opponent");
        // Both places the name is written use it.
        let mut snapshot = session().snapshot();
        snapshot.vs_display.as_mut().unwrap().opponent_name = hostile.to_string();
        let versus = Versus::from_snapshot(&snapshot).unwrap();
        assert!(!versus.title().replace("\x1b[1m", "").replace("\x1b[22m", "").contains('\x1b'));
        snapshot.interruption = SessionInterruption::ReadyPrompt { round_number: 2, remote_ready: false, opponent_present: true };
        assert!(!text(banner(&snapshot)).replace("\x1b[1m", "").replace("\x1b[22m", "").contains('\x1b'));
    }

    #[test]
    fn every_interruption_a_session_can_publish_has_its_message() {
        use SessionInterruption::*;
        let waits = [
            (WaitingRemoteReady { started_tick: 0 }, "Waiting for the opponent"),
            (ConnectionLost { started_tick: 0 }, "Connection lost"),
            (AwaitingRoundVerdict, "Waiting for the verdict"),
            (ResumeSync { started_tick: 0 }, "Resuming"),
            (Disconnected, "Opponent left"),
            (ReadyPrompt { round_number: 3, remote_ready: false, opponent_present: false }, "Opponent left"),
            (Paused { local_ready: false, remote_ready: false, is_network: false }, "P: resume"),
            (Paused { local_ready: false, remote_ready: true, is_network: true }, "The opponent is ready"),
            (Paused { local_ready: true, remote_ready: false, is_network: true }, "Waiting for the opponent"),
        ];
        for (interruption, expected) in waits {
            let shown = text(banner(&snapshot_with(interruption)));
            assert!(shown.contains(expected), "{:?}: {:?}", interruption, shown);
        }
        // An opponent who left offers no "play again", only the way out.
        let left = text(banner(&snapshot_with(Disconnected)));
        assert!(left.contains("Q: menu") && !left.contains("Enter"));
        // A network pause asks for consent to resume; nothing is shown while a scene is prepared.
        assert!(text(banner(&snapshot_with(Paused { local_ready: false, remote_ready: false, is_network: true }))).contains("P: ready to resume"));
        assert!(banner(&snapshot_with(ScenePreparing)).is_none());
    }

    #[test]
    fn the_title_shows_the_opponent_and_the_tally() {
        let versus = Versus { name: "Sigma".to_string(), wins: 2, losses: 1, ..Versus::default() };
        let title = versus.title();
        assert!(title.contains("Sigma") && title.contains("2-1"));
    }
}
