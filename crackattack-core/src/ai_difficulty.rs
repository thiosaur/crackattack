//! The levels of the grid-playing AI (VS Computer opponent and autoplay).

/// AI difficulty level: Logic, Sigma, Cortex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIDifficulty {
    Easy,
    Medium,
    Hard,
}

impl AIDifficulty {
    /// Display name for UI ("Logic", "Sigma", "Cortex")
    pub fn name(self) -> &'static str {
        match self {
            Self::Easy => "Logic",
            Self::Medium => "Sigma",
            Self::Hard => "Cortex",
        }
    }
}
