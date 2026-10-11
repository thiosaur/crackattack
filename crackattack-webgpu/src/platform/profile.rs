//! Runtime platform profile (Desktop or SteamDeck).
//!
//! Single source of truth for defaults that depend on the platform (currently fullscreen).
//!
//! Decision order:
//!   1. An explicit override wins (CLI flag `--deck` or `--desktop`).
//!   2. Otherwise, detection (never the only source of truth):
//!      - hardware: DMI `product_name` is `Jupiter` or `Galileo`. Reliable, independent of
//!        how the game was launched.
//!      - environment: `SteamDeck`, `SteamGamepadUI` or `SteamOS`. Depends on the launch channel.
//!
//! A persisted user preference (e.g. fullscreen) still wins at the call site.

use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformProfile {
    Desktop,
    SteamDeck,
}

// Once resolved, `RESOLVED` cannot be reset within the process (it is a `OnceLock`).
// This is fine for the app, because the override is set before the first read. A test
// binary cannot cover several --deck/--desktop cases through this global.
static RESOLVED: OnceLock<PlatformProfile> = OnceLock::new();
static OVERRIDE: Mutex<Option<PlatformProfile>> = Mutex::new(None);

/// Forces the profile (CLI flag). Call it before the first call to `profile()`.
pub fn set_override(p: PlatformProfile) {
    *crate::sync_ext::lock_ignore_poison(&OVERRIDE) = Some(p);
}

/// Resolves the profile and caches it. Safe to call repeatedly.
pub fn profile() -> PlatformProfile {
    *RESOLVED.get_or_init(detect)
}

/// Resolves the profile now. Call it after `set_override`, from `cli::apply`.
pub fn init() {
    let _ = profile();
}

pub fn is_steamdeck() -> bool {
    profile() == PlatformProfile::SteamDeck
}

/// Fullscreen default, used only when no user preference is persisted.
pub fn fullscreen_default() -> bool {
    is_steamdeck()
}

fn detect() -> PlatformProfile {
    // 1. The CLI override wins.
    if let Some(p) = *crate::sync_ext::lock_ignore_poison(&OVERRIDE) {
        log::info!("PlatformProfile: {:?} (override CLI)", p);
        return p;
    }
    // 2. Detection: hardware first, then environment.
    if detect_steamdeck_hardware() {
        log::info!("PlatformProfile: SteamDeck (DMI hardware)");
        return PlatformProfile::SteamDeck;
    }
    if detect_steam_env() {
        log::info!("PlatformProfile: SteamDeck (Steam environment variables)");
        return PlatformProfile::SteamDeck;
    }
    log::info!("PlatformProfile: Desktop");
    PlatformProfile::Desktop
}

/// Reliable signal that does not depend on the launch channel: the Steam Deck DMI product name.
/// `Jupiter` is the LCD model, `Galileo` the OLED model. Linux only (the DMI sysfs exists
/// only there); returns `false` on other OSes.
#[cfg(target_os = "linux")]
fn detect_steamdeck_hardware() -> bool {
    std::fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
        .map(|s| {
            let name = s.trim();
            name == "Jupiter" || name == "Galileo"
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn detect_steamdeck_hardware() -> bool {
    false
}

/// Steam environment hints. They can be missing when launched outside Steam (itch, Flatpak).
fn detect_steam_env() -> bool {
    std::env::var_os("SteamDeck").is_some()
        || std::env::var_os("SteamGamepadUI").is_some()
        || std::env::var_os("SteamOS").is_some()
}
