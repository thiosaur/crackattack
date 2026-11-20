// Colors of the terminal version (256-color palette), shared by the board, the side
// panel and the capture legend (`screen.rs`).

/// Block colors, indexed by flavor: purple, blue, green, yellow, orange, gray. Close to
/// the colors of the graphical version.
pub(crate) const COLORS: [u8; 6] = [127, 62, 34, 184, 208, 244];
/// Rising row (row 0), still being pushed up: dark versions of the same colors.
pub(crate) const DIM_COLORS: [u8; 6] = [53, 17, 22, 58, 94, 238];
/// Dying block (about to vanish): a small white square at the center of its cell.
pub(crate) const DYING_COLOR: u8 = 255;
/// Garbage: normal (red) and gray.
pub(crate) const GARBAGE_COLOR: u8 = 124;
pub(crate) const GARBAGE_GRAY_COLOR: u8 = 242;
/// Brighter garbage, the moment it is hit (start of its shatter).
pub(crate) const GARBAGE_FLASH_COLOR: u8 = 167;
pub(crate) const GARBAGE_GRAY_FLASH_COLOR: u8 = 250;
/// Dots marking the playable area.
pub(crate) const DOT_COLOR: u8 = 240;
/// Cursor brackets.
pub(crate) const CURSOR_COLOR: u8 = 231;
/// Danger bars on each side of the board (final countdown): purple, then red for the last
/// second; the dark shades while fading out.
pub(crate) const DANGER_PURPLE: [u8; 2] = [129, 54];
pub(crate) const DANGER_RED: [u8; 2] = [196, 88];
/// Chain popup in the side panel.
pub(crate) const CHAIN_COLOR: u8 = 220;
/// Colors of the two words of the logo, as in `logo-terminal-compact.ansi`: red CRACK,
/// purple ATTACK!.
pub(crate) const LOGO_COLORS: [u8; 2] = [124, 127];
/// The opponent's preview in a duel: all its blocks are gray-green while the stack is low and
/// gray-red once it reaches the top (garbage keeps the garbage color of the board, a
/// stronger red, so the two are not mistaken for each other).
pub(crate) const OPPONENT_BLOCK_COLOR: u8 = 108;
pub(crate) const OPPONENT_DANGER_COLOR: u8 = 138;
/// Key reminder at the bottom of the side panel.
pub(crate) const HINT_COLOR: u8 = 244;
/// Countdown digits (3 and 2 reuse the danger red and the orange block), and the dark outline
/// that makes them readable over the board.
pub(crate) const COUNTDOWN_ONE_COLOR: u8 = 220;
pub(crate) const COUNTDOWN_GO_COLOR: u8 = 46;
pub(crate) const COUNTDOWN_OUTLINE_COLOR: u8 = 16;
