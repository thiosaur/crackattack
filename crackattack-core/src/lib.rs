// Crack Attack! Core - Pure game logic, no rendering
//
// This crate contains only the deterministic game simulation.
// Renderers (terminal, WebGPU) are in separate crates.

pub mod constants;
pub mod block;
pub mod grid;
pub mod garbage;
pub(crate) mod garbage_generator;
pub(crate) mod garbage_rules;
pub(crate) mod random;
pub mod input;
pub(crate) mod swapper;
pub(crate) mod combo;
pub(crate) mod creep;
pub(crate) mod score;
pub mod frame_data;
pub mod game_state;
pub mod fingerprint;
pub mod gameplay_event;
pub mod ai;
pub mod ai_difficulty;
pub mod vs;
pub mod stats;

// Re-exports for convenience
pub use constants::*;
pub use block::BlockState;
pub use input::{Input, InputSnapshot, INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE};
pub use score::ScoringMode;
pub use frame_data::{FrameData, UIState, GamePhase, CountdownDigit, CountdownInfo};
pub use game_state::{GameState, GameStatus};
pub use fingerprint::Fingerprint;
pub use ai::AIController;
