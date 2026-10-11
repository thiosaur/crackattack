//! Update check: the remote half of the system, compiled only with the `update-check`
//! feature. It reads the latest GitHub release (one field, size bounds, parsing) and, at
//! runtime, runs the checker (HTTP client and scheduling). The distribution profile lives
//! in `platform::distribution`, which is always compiled.

pub mod checker;
pub mod release;

use crate::platform::distribution::{Distribution, FetchPolicy, PUBLIC_PROJECT};

/// Where to read the release and where to send the player. This is the single answer to
/// "can this binary check for updates?". The checker, the Options checkbox, the "Check now"
/// button and the notification rebuild all go through [`update_source`], so no one of them
/// can be enabled without the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSource {
    /// "Latest release" API that is queried.
    pub api_url: String,
    /// Page opened by "See the download". It comes from the distribution profile.
    pub open_url: String,
}

/// Chain of authority: public identity, then an `AutoDaily` profile, then the update
/// destination. Only in `dev` builds, [`DEV_OVERRIDE_VAR`] replaces it for a manual test
/// against a local server.
pub fn update_source() -> Option<UpdateSource> {
    #[cfg(feature = "dev")]
    if let Some(source) = std::env::var(DEV_OVERRIDE_VAR).ok().and_then(|u| dev_override(&u)) {
        return Some(source);
    }
    let project = PUBLIC_PROJECT.as_ref()?;
    let profile = Distribution::current().profile();
    if profile.fetch() != FetchPolicy::AutoDaily {
        return None;
    }
    let open_url = profile.update_url(Some(project))?;
    Some(UpdateSource { api_url: project.latest_release_api.to_string(), open_url })
}

/// Environment variable for local testing, read only with the `dev` feature.
#[cfg(any(feature = "dev", test))]
pub const DEV_OVERRIDE_VAR: &str = "CRACKATTACK_UPDATE_URL";

/// Local test source. The URL must be `http://` on loopback, never a real address
/// (`url_allowed` would accept any `https://`). The destination is the root of the same
/// server: an empty `PUBLIC_PROJECT` has no repository page to offer.
#[cfg(any(feature = "dev", test))]
fn dev_override(url: &str) -> Option<UpdateSource> {
    if !url.starts_with("http://") || !checker::url_allowed(url) {
        log::warn!("{DEV_OVERRIDE_VAR} ignored: only http:// on loopback is accepted");
        return None;
    }
    let authority = url["http://".len()..].split(['/', '?', '#']).next().unwrap_or("");
    Some(UpdateSource { api_url: url.to_string(), open_url: format!("http://{authority}/") })
}

/// Status line of the Info page. Derived from the persisted state and the current
/// attempt, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Checking,
    Never,
    /// The last attempt failed. `last_success` is the last successful read, if any.
    Failed { last_success: Option<u64> },
    Available(String),
    /// A newer version exists but the player ignored it. Never report "up to date" here.
    Ignored(String),
    UpToDate { checked_at: u64 },
}

/// `last_check` and `last_success` are Unix seconds, `0` meaning never. `available` is the
/// persisted version (`""` for none), already purged if it no longer exceeds the installed one.
pub fn update_status(
    checking: bool,
    last_check: u64,
    last_success: u64,
    available: &str,
    ignored: &str,
) -> UpdateStatus {
    if checking {
        return UpdateStatus::Checking;
    }
    if last_check == 0 {
        return UpdateStatus::Never;
    }
    // An attempt newer than the last success means it failed.
    if last_success == 0 || last_check > last_success {
        return UpdateStatus::Failed { last_success: (last_success != 0).then_some(last_success) };
    }
    match available {
        "" => UpdateStatus::UpToDate { checked_at: last_success },
        v if v == ignored => UpdateStatus::Ignored(v.to_string()),
        v => UpdateStatus::Available(v.to_string()),
    }
}

/// Whether the persisted version is still valid, meaning newer than the installed one.
/// Otherwise (the player installed the update, or the value is unreadable) it must be cleared.
pub fn still_newer(available: &str, current: &str) -> bool {
    match (available.parse::<semver::Version>(), current.parse::<semver::Version>()) {
        (Ok(a), Ok(c)) => a > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_absente_aujourd_hui() {
        // Empty public identity and unstamped binary: nothing to check.
        if std::env::var(DEV_OVERRIDE_VAR).is_err() {
            assert!(update_source().is_none());
        }
    }

    #[test]
    fn override_loopback_seulement() {
        let s = dev_override("http://127.0.0.1:8080/repos/o/r/releases/latest").unwrap();
        assert_eq!(s.api_url, "http://127.0.0.1:8080/repos/o/r/releases/latest");
        assert_eq!(s.open_url, "http://127.0.0.1:8080/");
        assert!(dev_override("http://[::1]:9000/latest").is_some());
        // Never a real address, even over HTTPS.
        assert!(dev_override("https://api.github.com/repos/o/r/releases/latest").is_none());
        assert!(dev_override("http://example.org/latest").is_none());
        assert!(dev_override("http://127.0.0.1@evil.tld/latest").is_none());
    }

    #[test]
    fn ligne_d_etat() {
        use UpdateStatus::*;
        assert_eq!(update_status(true, 0, 0, "", ""), Checking);
        assert_eq!(update_status(false, 0, 0, "", ""), Never);
        // never succeeded, or an attempt newer than the last success: failed.
        assert_eq!(update_status(false, 100, 0, "", ""), Failed { last_success: None });
        assert_eq!(update_status(false, 200, 100, "2.1.0", ""), Failed { last_success: Some(100) });
        // succeeded: up to date, available, or ignored (never "up to date" when ignored).
        assert_eq!(update_status(false, 100, 100, "", ""), UpToDate { checked_at: 100 });
        assert_eq!(update_status(false, 100, 100, "2.1.0", ""), Available("2.1.0".into()));
        assert_eq!(update_status(false, 100, 100, "2.1.0", "2.1.0"), Ignored("2.1.0".into()));
        assert_eq!(update_status(false, 100, 100, "2.2.0", "2.1.0"), Available("2.2.0".into()));
    }

    #[test]
    fn version_rattrapee_a_effacer() {
        assert!(still_newer("2.1.0", "2.0.0"));
        assert!(!still_newer("2.1.0", "2.1.0"));
        assert!(!still_newer("2.0.0", "2.1.0"));
        assert!(!still_newer("", "2.0.0"));
        assert!(!still_newer("garbage", "2.0.0"));
    }
}
