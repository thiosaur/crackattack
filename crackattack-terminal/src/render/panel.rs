// Side panel, right of the board: game info, the chain just reached, the logo and the
// keys.

use crackattack_core::frame_data::FrameData;

use super::palette::{CHAIN_COLOR, HINT_COLOR, LOGO_COLORS};
use super::raster::LOWER_EIGHTHS;
use super::Hud;

/// The game's compact terminal logo (`logo.txt`): one block per `#`, the two words
/// separated by a blank line. Shown between the game info and the key hints.
pub(super) const LOGO: &str = include_str!("../logo.txt");
/// What the logo turns into when the game is lost: same layout (two words, same colors).
pub(super) const GAME_OVER: &str = include_str!("../game-over.txt");
/// Panel line where the logo starts: just below the chain popup.
pub(super) const LOGO_FIRST_LINE: usize = 7;
/// Game over: the logo turns into GAME OVER column after column, from the left, each
/// flipping like a block of a shattered garbage (it flattens, then the new one grows).
/// Ticks between two columns, and for one column's flip.
const FLIP_STAGGER: i32 = 2;
const FLIP_TICKS: i32 = 8;

/// A game key, as `--help` lists it and as the side panel reminds it.
pub(crate) struct KeyHelp {
    /// Every key that does it, for `--help`.
    pub keys: &'static str,
    pub action: &'static str,
    /// Short forms for the side panel.
    pub hint_key: &'static str,
    pub hint_action: &'static str,
}

/// The game's keys (mapped in `main.rs`): the one list `--help` and the side panel show.
pub(crate) const KEYS: [KeyHelp; 5] = [
    KeyHelp { keys: "Arrows / WASD", action: "Move cursor", hint_key: "Arrows", hint_action: "move" },
    KeyHelp { keys: "Space", action: "Swap blocks", hint_key: "Space", hint_action: "swap" },
    KeyHelp { keys: "Enter", action: "Raise the stack", hint_key: "Enter", hint_action: "raise" },
    KeyHelp { keys: "P", action: "Pause / resume", hint_key: "P", hint_action: "pause" },
    KeyHelp { keys: "Q / Escape", action: "Quit", hint_key: "Q", hint_action: "quit" },
];

/// Side panel text for line `i` (0 = top) of `lines`: game info at the top, the chain
/// just reached below it, the keys at the bottom (once the game is lost and GAME OVER
/// shown, a prompt instead).
pub(super) fn panel_line(i: usize, lines: usize, frame: &FrameData, hud: &Hud) -> Option<String> {
    let seconds = frame.ui.clock_time / crate::TICKS_PER_SECOND;
    let first_hint = lines.checked_sub(KEYS.len())?;
    match i {
        0 => Some(format!("\x1b[1mScore  {}\x1b[22m", frame.score)),
        2 => Some(format!("Time   {}:{:02}", seconds / 60, seconds % 60)),
        3 => Some(match frame.stats.max_chain_level {
            n if n >= 2 => format!("Best   x{}", n),
            _ => "Best   -".to_string(),
        }),
        6 => hud.chain.map(|n| format!("\x1b[1;38;5;{}mChain x{}!\x1b[22;39m", CHAIN_COLOR, n)),
        _ if (LOGO_FIRST_LINE..LOGO_FIRST_LINE + art().colors.len()).contains(&i) => {
            Some(logo_line(i - LOGO_FIRST_LINE, hud.game_over))
        }
        _ if hud.game_over.is_some_and(|t| t >= game_over_ticks()) => {
            (i == first_hint).then(|| "\x1b[1mPress any key\x1b[22m".to_string())
        }
        _ => {
            let key = KEYS.get(i.checked_sub(first_hint)?)?;
            Some(format!("\x1b[38;5;{}m{:<7}{}\x1b[39m", HINT_COLOR, key.hint_key, key.hint_action))
        }
    }
}

/// The logo and GAME OVER, cell by cell (`true` = a block), with the color of each line.
struct Art {
    logo: Vec<Vec<bool>>,
    game_over: Vec<Vec<bool>>,
    colors: Vec<u8>,
}

fn art() -> &'static Art {
    static ART: std::sync::OnceLock<Art> = std::sync::OnceLock::new();
    ART.get_or_init(|| {
        let cells = |text: &str| -> Vec<Vec<bool>> {
            text.lines().map(|line| line.chars().map(|c| c == '#').collect()).collect()
        };
        // The words are separated by a blank line; each has its color.
        let mut word = 0;
        let colors = LOGO.lines()
            .map(|line| {
                if line.trim().is_empty() {
                    word = (word + 1).min(LOGO_COLORS.len() - 1);
                }
                LOGO_COLORS[word]
            })
            .collect();
        Art { logo: cells(LOGO), game_over: cells(GAME_OVER), colors }
    })
}

/// Ticks from the loss until GAME OVER is fully shown.
pub(crate) fn game_over_ticks() -> i32 {
    let art = art();
    let width = art.logo.iter().chain(&art.game_over).map(Vec::len).max().unwrap_or(0);
    (width as i32 - 1).max(0) * FLIP_STAGGER + FLIP_TICKS
}

/// Line `row` of the logo; `game_over` ticks after a loss, turning into GAME OVER.
pub(super) fn logo_line(row: usize, game_over: Option<i32>) -> String {
    let art = art();
    let filled = |cells: &[Vec<bool>], col: usize| cells.get(row).and_then(|l| l.get(col)).copied().unwrap_or(false);
    let width = |cells: &[Vec<bool>]| cells.get(row).map_or(0, Vec::len);
    let glyphs: String = match game_over {
        None => (0..width(&art.logo)).map(|col| if filled(&art.logo, col) { '█' } else { ' ' }).collect(),
        Some(t) => (0..width(&art.logo).max(width(&art.game_over)))
            .map(|col| {
                // Into its flip: the logo's cell shrinks to the bottom (8, 6, 4, 2 eighths),
                // then GAME OVER's grows (2, 4, 6, 8).
                let p = t - col as i32 * FLIP_STAGGER;
                let (on, eighths) = if p < FLIP_TICKS / 2 {
                    (filled(&art.logo, col), 8 - 2 * p.max(0))
                } else {
                    (filled(&art.game_over, col), (2 * (p - FLIP_TICKS / 2 + 1)).min(8))
                };
                match (on, eighths) {
                    (false, _) => ' ',
                    (true, 8) => '█',
                    (true, n) => LOWER_EIGHTHS[n as usize - 1],
                }
            })
            .collect(),
    };
    if glyphs.trim().is_empty() {
        return String::new();
    }
    format!("\x1b[38;5;{}m{}\x1b[39m", art.colors[row], glyphs)
}
