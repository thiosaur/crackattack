//! Debug and QA state of the binary: exit request, screenshots (F12 and
//! miniscript), dev panel, modal guard.

/// Debug, test and developer fields.
pub(crate) struct DebugState {
    pub(crate) should_exit: bool,
    /// Path of the active miniscript (prefix of named screenshots, final report).
    /// Set from `BootConfig`.
    pub(crate) script_path: Option<String>,
    /// Exit after N frames (0 = unlimited). Set from `BootConfig`.
    pub(crate) max_frames: u32,
    /// Seed given explicitly at launch (`None` = clock). The miniscript final
    /// report prints 0 when it is absent.
    pub(crate) explicit_seed: Option<u64>,
    /// Pending screenshot request (consumed by `frame_submit`). `Manual` is F12;
    /// `Named` comes from the miniscript (final file name, prefixed by the script name).
    pub(crate) screenshot_request: Option<crate::platform::screenshot::ScreenshotRequest>,
    /// Number of named screenshots requested by the script (final report).
    pub(crate) script_captures_demanded: u32,
    /// Set when a miniscript assertion (`assert …`) failed; the process then exits with code 1.
    pub(crate) script_assert_failed: bool,
    #[cfg(feature = "dev")]
    pub(crate) dev_panel: crate::dev_tuning::DevPanelState,
    /// Debug guard: true while the modals are taken out in `handle_redraw`
    /// (between take and restore). `apply_app_command` must not run then,
    /// because its effects would be overwritten on restore.
    #[cfg(debug_assertions)]
    pub(crate) modals_taken: bool,
}

impl DebugState {
    pub(crate) fn new() -> Self {
        Self {
            should_exit: false,
            script_path: None,
            max_frames: 0,
            explicit_seed: None,
            screenshot_request: None,
            script_captures_demanded: 0,
            script_assert_failed: false,
            #[cfg(feature = "dev")]
            dev_panel: crate::dev_tuning::DevPanelState::default(),
            #[cfg(debug_assertions)]
            modals_taken: false,
        }
    }
}
