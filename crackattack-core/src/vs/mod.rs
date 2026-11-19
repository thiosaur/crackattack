//! VS mode: opponents, sessions, round lifecycle.

pub mod opponent;
pub mod board_view;
pub(crate) mod computer_session;
pub(crate) mod grid_ai;
pub(crate) mod grid_opponent;
pub(crate) mod game_session;
pub mod session;
pub mod round;

pub use opponent::Opponent;
pub use board_view::{BoardCell, OpponentBoardView};
pub use grid_ai::GridAutoplay;
pub use grid_opponent::GridOpponent;
pub use computer_session::VsComputerGameSession;
pub use game_session::{GameSession, SoloSession};
pub use session::VsSession;
pub use round::{RoundOutcome, SessionPhase};

