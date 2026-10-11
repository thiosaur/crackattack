//! Unprivileged remote document: the latest GitHub release, of which only `tag_name` is
//! read. It can inform but never widen anything: no URL comes out of it (`html_url`,
//! `assets` and the rest are ignored) and no permission is read from it.

/// Hard size limit on the document, checked by the fetcher and again here. A release body
/// holds the release notes and the file list, so it can be large.
pub const MAX_RELEASE_BYTES: usize = 256 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum ReleaseError {
    TooLarge,
    /// Invalid or truncated JSON, not an object, or `tag_name` missing or not a string.
    Malformed,
    /// `tag_name` is present but is not a strict SemVer version once a leading `v` is removed.
    NotSemver(String),
}

#[derive(serde::Deserialize)]
struct RawRelease {
    tag_name: String,
}

/// Version announced by the release: `tag_name`, with or without a leading `v`, strict SemVer.
pub fn parse_latest_release(bytes: &[u8]) -> Result<semver::Version, ReleaseError> {
    if bytes.len() > MAX_RELEASE_BYTES {
        return Err(ReleaseError::TooLarge);
    }
    let raw: RawRelease = serde_json::from_slice(bytes).map_err(|_| ReleaseError::Malformed)?;
    let tag = raw.tag_name.strip_prefix('v').unwrap_or(&raw.tag_name);
    tag.parse::<semver::Version>()
        .map_err(|_| ReleaseError::NotSemver(raw.tag_name.clone()))
}

/// Resolves an update: `Some(remote)` if the release is strictly newer than `current` and
/// the player has not ignored it. An invalid `current` gives `None`, so a doubtful
/// comparison never raises a notification.
pub fn available_update(
    current: &str,
    remote: &semver::Version,
    ignored: Option<&str>,
) -> Option<semver::Version> {
    let current: semver::Version = current.parse().ok()?;
    if *remote <= current {
        return None;
    }
    if ignored.and_then(|s| s.parse::<semver::Version>().ok()).is_some_and(|i| i == *remote) {
        return None;
    }
    Some(remote.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> semver::Version {
        s.parse().unwrap()
    }

    /// Realistic excerpt of a GitHub response: only `tag_name` matters.
    const OK: &[u8] = br#"{ "url": "https://api.github.com/repos/o/r/releases/1",
        "html_url": "https://evil.test/ne-pas-suivre", "tag_name": "v2.1.0",
        "name": "Crack Attack 2.1.0", "draft": false, "prerelease": false,
        "assets": [ { "browser_download_url": "https://evil.test/x.zip" } ],
        "body": "Release notes" }"#;

    #[test]
    fn tag_avec_et_sans_v() {
        assert_eq!(parse_latest_release(OK).unwrap(), v("2.1.0"));
        assert_eq!(parse_latest_release(br#"{ "tag_name": "2.1.0" }"#).unwrap(), v("2.1.0"));
    }

    #[test]
    fn tag_non_semver_rejete() {
        for tag in ["2.1", "v2", "latest", "vv2.1.0", "2.1.0.1", ""] {
            let doc = format!(r#"{{ "tag_name": "{tag}" }}"#);
            assert!(
                matches!(parse_latest_release(doc.as_bytes()), Err(ReleaseError::NotSemver(_))),
                "{tag:?}"
            );
        }
    }

    #[test]
    fn champ_absent_ou_mal_type() {
        for doc in [
            &br#"{ "name": "2.1.0" }"#[..],
            br#"{ "tag_name": 210 }"#,
            br#"{ "tag_name": null }"#,
            br#"[ { "tag_name": "v2.1.0" } ]"#,
            br#""v2.1.0""#,
            b"",
        ] {
            assert_eq!(parse_latest_release(doc), Err(ReleaseError::Malformed));
        }
    }

    #[test]
    fn json_tronque() {
        assert_eq!(parse_latest_release(&OK[..60]), Err(ReleaseError::Malformed));
    }

    #[test]
    fn trop_grand_rejete() {
        let mut doc = br#"{ "tag_name": "v2.1.0", "body": ""#.to_vec();
        doc.extend(std::iter::repeat_n(b'a', MAX_RELEASE_BYTES));
        doc.extend(b"\" }");
        assert_eq!(parse_latest_release(&doc), Err(ReleaseError::TooLarge));
    }

    #[test]
    fn comparaisons_semver() {
        let remote = v("2.1.0");
        // newer: notify; equal or older: no.
        assert_eq!(available_update("2.0.9", &remote, None), Some(v("2.1.0")));
        assert!(available_update("2.1.0", &remote, None).is_none());
        assert!(available_update("3.0.0", &remote, None).is_none());
        // local prerelease 2.1.0-beta < 2.1.0: notify (SemVer ordering, intended).
        assert!(available_update("2.1.0-beta.1", &remote, None).is_some());
        // Betas are published as normal releases: beta.1 is told about beta.2, and beta.2
        // is not told about beta.1.
        assert!(available_update("2.0.0-beta.1", &v("2.0.0-beta.2"), None).is_some());
        assert!(available_update("2.0.0-beta.2", &v("2.0.0-beta.1"), None).is_none());
        assert_eq!(parse_latest_release(br#"{"tag_name": "v2.0.0-beta.2"}"#).unwrap(), v("2.0.0-beta.2"));
        // invalid current version: never notify.
        assert!(available_update("n/a", &remote, None).is_none());
    }

    #[test]
    fn version_ignoree_persistante() {
        let remote = v("2.1.0");
        assert!(available_update("2.0.0", &remote, Some("2.1.0")).is_none());
        // ignoring a different version does not hide this one.
        assert!(available_update("2.0.0", &remote, Some("2.0.5")).is_some());
        // unreadable ignored value: no effect.
        assert!(available_update("2.0.0", &remote, Some("garbage")).is_some());
    }
}
