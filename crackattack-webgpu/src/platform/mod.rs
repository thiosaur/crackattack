//! Native platform integration (CLI, persistence, audio).

pub mod boot_config;
pub mod distribution;
pub mod boot_target;
pub mod cli;
#[cfg(feature = "native")]
pub mod diagnostics;
pub mod ladder;
pub mod persist;
pub mod preset;
pub mod profile;
pub mod session_param;
#[cfg(feature = "native")]
pub mod screenshot;
#[cfg(feature = "native")]
pub mod adaptive_music;
#[cfg(feature = "native")]
pub mod sound;
#[cfg(test)]
pub(crate) mod test_support;
