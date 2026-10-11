//! VS mode: opponents, sessions, round lifecycle.

pub mod opponent;
pub mod computer;
pub mod computer_session;
pub mod game_session;
pub mod session;
pub mod round;

pub use opponent::Opponent;
pub use computer::ComputerOpponent;
pub use computer_session::VsComputerGameSession;
pub use game_session::{GameSession, SoloSession};
pub use session::VsSession;
pub use round::{RoundOutcome, SessionPhase};

