//! Developer tuning panel state — feature-gated under `dev`.

#![cfg(feature = "dev")]

/// The live dev-tuning state.
#[derive(Clone, Debug)]
pub struct DevTuning {
    pub freeze_simulation: bool,
    pub debug_drop_garbage_height: i32,
    pub debug_drop_garbage_flavor: i32,
}

impl Default for DevTuning {
    fn default() -> Self {
        Self {
            freeze_simulation: false,
            debug_drop_garbage_height: 0,
            debug_drop_garbage_flavor: 0,
        }
    }
}

/// Runtime state for the developer tuning panel.
#[derive(Clone, Debug, Default)]
pub struct DevPanelState {
    pub visible: bool,
    pub tuning: DevTuning,
}
