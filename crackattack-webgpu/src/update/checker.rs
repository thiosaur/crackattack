//! HTTP client and scheduling of the update check. A dedicated thread sends the
//! already-validated result over an mpsc channel. Every error is silent for the player
//! (debug log only). The fetcher is a trait: unit tests inject a fake one, and integration
//! tests point the real client at a local server.

use std::io::Read;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use super::release::{parse_latest_release, MAX_RELEASE_BYTES};
use crate::platform::distribution::FetchPolicy;

/// At most one attempt per 24 hours, counted from the last attempt, successful or not.
/// Offline, the game does not retry at every launch.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 3600;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
pub enum FetchError {
    /// HTTPS is required (the only exception is loopback, see [`url_allowed`]).
    ForbiddenScheme,
    TooLarge,
    /// Non-2xx HTTP status. This includes 3xx, because redirects are disabled.
    HttpStatus(u16),
    Network(String),
}

pub trait ReleaseFetcher: Send + 'static {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, FetchError>;
}

/// HTTPS is required. The only exception is loopback, for integration tests against a
/// temporary local HTTP server. Production has no URL override: the URL comes from
/// `PUBLIC_PROJECT`.
pub fn url_allowed(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        // Bracketed IPv6 (`[::1]:port`): the port's `:` follows the `]`.
        let host = match authority.strip_prefix('[') {
            Some(v6) => {
                return v6.strip_prefix("::1]").is_some_and(|r| r.is_empty() || r.starts_with(':'))
            }
            None => authority.split(':').next().unwrap_or(""),
        };
        return host == "127.0.0.1" || host == "localhost";
    }
    false
}

/// Minimal User-Agent. GitHub requires one; nothing but the version is sent.
fn user_agent() -> String {
    format!("crackattack/{}", env!("CARGO_PKG_VERSION"))
}

/// The real client (ureq/rustls): short timeout, no redirects, bounded response.
pub struct HttpFetcher {
    timeout: Duration,
}

impl HttpFetcher {
    pub fn new() -> Self {
        Self { timeout: DEFAULT_TIMEOUT }
    }
    /// For tests: a short timeout keeps the suite fast.
    #[cfg(test)]
    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl ReleaseFetcher for HttpFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        if !url_allowed(url) {
            return Err(FetchError::ForbiddenScheme);
        }
        let agent = ureq::AgentBuilder::new().timeout(self.timeout).redirects(0).build();
        let request = agent
            .get(url)
            .set("User-Agent", &user_agent())
            .set("Accept", "application/json");
        match request.call() {
            Ok(resp) => {
                // With `redirects(0)`, ureq returns a 3xx response as `Ok` (it only errors
                // on 4xx/5xx). Require a 2xx explicitly: redirects are refused, not followed.
                if !(200..300).contains(&resp.status()) {
                    return Err(FetchError::HttpStatus(resp.status()));
                }
                let mut buf = Vec::new();
                let mut r = resp.into_reader().take(MAX_RELEASE_BYTES as u64 + 1);
                r.read_to_end(&mut buf).map_err(|e| FetchError::Network(e.to_string()))?;
                if buf.len() > MAX_RELEASE_BYTES {
                    return Err(FetchError::TooLarge);
                }
                Ok(buf)
            }
            Err(ureq::Error::Status(code, _)) => Err(FetchError::HttpStatus(code)),
            Err(e) => Err(FetchError::Network(e.to_string())),
        }
    }
}

/// Pure decision: should a check run now? If the clock is inconsistent (`last > now`), it
/// retries, so the check is never postponed forever. The manual button bypasses this delay
/// and does not call this function.
pub fn should_check_now(
    policy: FetchPolicy,
    enabled: bool,
    last_attempt: Option<u64>,
    now: u64,
) -> bool {
    if policy != FetchPolicy::AutoDaily || !enabled {
        return false;
    }
    match last_attempt {
        None => true,
        Some(last) if last > now => true, // inconsistent clock
        Some(last) => now - last >= CHECK_INTERVAL_SECS,
    }
}

/// An available update, ready to display: the version and the URL to open. The URL comes
/// from the distribution profile, never from the remote document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAvailable {
    pub version: String,
    pub open_url: String,
}

/// Result of an attempt, already validated in the thread. The caller compares it with the
/// installed and ignored versions. A successful read counts even when the remote version
/// is not newer (it feeds `last_update_success`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Version announced by the latest published release.
    Remote(semver::Version),
    /// The read failed (network, status, size or invalid document). Already logged.
    Failed,
}

/// Runs one full attempt in a dedicated thread. The thread only touches remote data.
pub fn spawn_check(fetcher: impl ReleaseFetcher, release_url: String) -> Receiver<CheckOutcome> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            let _ = tx.send(run_check(&fetcher, &release_url));
        })
        .expect("spawn update-check");
    rx
}

fn run_check(fetcher: &dyn ReleaseFetcher, url: &str) -> CheckOutcome {
    let bytes = match fetcher.fetch(url) {
        Ok(b) => b,
        Err(e) => {
            log::debug!("update-check: fetch failed ({e:?})");
            return CheckOutcome::Failed;
        }
    };
    match parse_latest_release(&bytes) {
        Ok(v) => CheckOutcome::Remote(v),
        Err(e) => {
            log::debug!("update-check: release rejected ({e:?})");
            CheckOutcome::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_obligatoire_sauf_loopback() {
        assert!(url_allowed("https://example.org/releases/latest"));
        assert!(url_allowed("http://127.0.0.1:8080/releases/latest"));
        assert!(url_allowed("http://[::1]:8080/releases/latest"));
        assert!(url_allowed("http://[::1]/releases/latest"));
        assert!(!url_allowed("http://[::2]:8080/releases/latest"));
        assert!(!url_allowed("http://[::1.evil.tld]/x"));
        assert!(!url_allowed("http://[::1]@evil.tld/x"));
        assert!(!url_allowed("http://127.0.0.1@evil.tld/x"));
        assert!(url_allowed("http://localhost/m.json"));
        assert!(!url_allowed("http://example.org/releases/latest"));
        assert!(!url_allowed("http://127.0.0.1.evil.tld/m.json"));
        assert!(!url_allowed("ftp://127.0.0.1/m.json"));
    }

    #[test]
    fn cadence_quotidienne() {
        use FetchPolicy::*;
        let now = 1_000_000_000;
        // never with Off, nor with the option unchecked.
        assert!(!should_check_now(Off, true, None, now));
        assert!(!should_check_now(AutoDaily, false, None, now));
        // first time, delay elapsed, inconsistent clock: yes.
        assert!(should_check_now(AutoDaily, true, None, now));
        assert!(should_check_now(AutoDaily, true, Some(now - CHECK_INTERVAL_SECS), now));
        assert!(should_check_now(AutoDaily, true, Some(now + 999), now));
        // recent attempt (even a failed one: the attempt is what counts): no.
        assert!(!should_check_now(AutoDaily, true, Some(now - 60), now));
    }

    struct FakeFetcher(Result<Vec<u8>, FetchError>);
    impl ReleaseFetcher for FakeFetcher {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>, FetchError> {
            match &self.0 {
                Ok(b) => Ok(b.clone()),
                Err(e) => Err(match e {
                    FetchError::ForbiddenScheme => FetchError::ForbiddenScheme,
                    FetchError::TooLarge => FetchError::TooLarge,
                    FetchError::HttpStatus(c) => FetchError::HttpStatus(*c),
                    FetchError::Network(s) => FetchError::Network(s.clone()),
                }),
            }
        }
    }

    const DOC: &[u8] = br#"{ "tag_name": "v2.1.0", "html_url": "https://evil.test/" }"#;

    fn remote(s: &str) -> CheckOutcome {
        CheckOutcome::Remote(s.parse().unwrap())
    }

    #[test]
    fn cycle_complet_via_fetcher_injecte() {
        let rx = spawn_check(FakeFetcher(Ok(DOC.to_vec())), "https://example.test/latest".into());
        assert_eq!(rx.recv().unwrap(), remote("2.1.0"));
    }

    /// A release that is not newer is still a successful read: the caller needs it to
    /// show "you have the latest version".
    #[test]
    fn version_ancienne_reste_une_reussite() {
        let rx = spawn_check(
            FakeFetcher(Ok(br#"{ "tag_name": "v0.0.1" }"#.to_vec())),
            "https://example.test/latest".into(),
        );
        assert_eq!(rx.recv().unwrap(), remote("0.0.1"));
    }

    #[test]
    fn erreurs_silencieuses_donnent_failed() {
        for f in [
            FakeFetcher(Err(FetchError::Network("down".into()))),
            FakeFetcher(Err(FetchError::HttpStatus(404))),
            FakeFetcher(Ok(b"{ not json".to_vec())),
            FakeFetcher(Ok(br#"{ "tag_name": "2.1" }"#.to_vec())),
            FakeFetcher(Ok(br#"{ "name": "v2.1.0" }"#.to_vec())),
        ] {
            let rx = spawn_check(f, "https://x/latest".into());
            assert_eq!(rx.recv().unwrap(), CheckOutcome::Failed);
        }
    }

    // -- Integration: the real HTTP client against a temporary local server. The only
    // HTTPS exception is this loopback, so no public URL is needed. --

    /// Serves `response` (raw bytes, headers included) to the first connection, then stops.
    /// Returns the loopback URL to query and the request received (to check the headers
    /// sent).
    fn serve_once(response: Vec<u8>) -> (String, Receiver<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).unwrap_or(0); // the GET request
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = sock.write_all(&response);
            }
        });
        (format!("http://127.0.0.1:{port}/repos/o/r/releases/latest"), rx)
    }

    fn http_ok(body: &[u8]) -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(body);
        r
    }

    #[test]
    fn integration_cas_nominal() {
        let (url, _) = serve_once(http_ok(DOC));
        let got = HttpFetcher::new().fetch(&url).unwrap();
        assert_eq!(got, DOC);
        // And the full chain (fetch, then parse):
        let (url, _) = serve_once(http_ok(DOC));
        let rx = spawn_check(HttpFetcher::new(), url);
        assert_eq!(rx.recv().unwrap(), remote("2.1.0"));
    }

    #[test]
    fn integration_en_tetes_envoyes() {
        let (url, request) = serve_once(http_ok(DOC));
        HttpFetcher::new().fetch(&url).unwrap();
        let request = request.recv().unwrap().to_ascii_lowercase();
        let ua = format!("user-agent: crackattack/{}", env!("CARGO_PKG_VERSION"));
        assert!(request.contains(&ua), "{request}");
        assert!(request.contains("accept: application/json"), "{request}");
    }

    #[test]
    fn integration_reponse_trop_grande() {
        let big = vec![b'a'; MAX_RELEASE_BYTES + 100];
        let (url, _) = serve_once(http_ok(&big));
        assert_eq!(HttpFetcher::new().fetch(&url), Err(FetchError::TooLarge));
    }

    #[test]
    fn integration_redirection_refusee() {
        let (url, _) = serve_once(
            b"HTTP/1.1 301 Moved Permanently\r\nLocation: https://evil.test/\r\nContent-Length: 0\r\n\r\n"
                .to_vec(),
        );
        assert_eq!(HttpFetcher::new().fetch(&url), Err(FetchError::HttpStatus(301)));
    }

    #[test]
    fn integration_pas_de_release() {
        let (url, _) = serve_once(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        );
        assert_eq!(HttpFetcher::new().fetch(&url), Err(FetchError::HttpStatus(404)));
    }

    #[test]
    fn integration_serveur_absent() {
        // Bind a port, then release it: the connection is refused.
        let url = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://127.0.0.1:{}/latest", l.local_addr().unwrap().port())
        };
        assert!(matches!(HttpFetcher::new().fetch(&url), Err(FetchError::Network(_))));
    }

    #[test]
    fn integration_timeout() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf);
                std::thread::sleep(Duration::from_secs(3)); // never answers in time
            }
        });
        let f = HttpFetcher::with_timeout(Duration::from_millis(300));
        let start = std::time::Instant::now();
        assert!(matches!(
            f.fetch(&format!("http://127.0.0.1:{port}/latest")),
            Err(FetchError::Network(_))
        ));
        // The server closes after 3 s and would also give `Network`: only the elapsed time
        // proves the timeout (300 ms) is what ended the call.
        assert!(start.elapsed() < Duration::from_secs(2), "timeout not applied");
    }

    #[test]
    fn integration_http_non_loopback_jamais_contacte() {
        // Refused before any connection. The domain does not exist, so a network attempt
        // would give `Network`, not `ForbiddenScheme`.
        let f = HttpFetcher::with_timeout(Duration::from_millis(100));
        assert_eq!(
            f.fetch("http://release-en-clair.invalid/latest"),
            Err(FetchError::ForbiddenScheme)
        );
    }

    #[test]
    fn integration_json_invalide_silencieux() {
        let (url, _) = serve_once(http_ok(b"{ truncated"));
        let rx = spawn_check(HttpFetcher::new(), url);
        assert_eq!(rx.recv().unwrap(), CheckOutcome::Failed);
    }

    /// Inertness: for every distribution without fetch, the scheduling decision is always
    /// negative, even with the option checked and the delay elapsed. Together with the
    /// identity gate (tested in `distribution`), no network path is reachable.
    #[test]
    fn inertie_profils_sans_fetch() {
        use crate::platform::distribution::{Distribution, ALL_DISTRIBUTIONS};
        for d in ALL_DISTRIBUTIONS {
            let p = d.profile();
            // Only the direct download may fetch: a store or distro profile that drifts to
            // `AutoDaily` must fail here, not be skipped.
            if d != Distribution::Direct {
                assert_eq!(p.fetch(), FetchPolicy::Off, "{d:?}");
            }
            if p.fetch() == FetchPolicy::Off {
                assert!(!should_check_now(p.fetch(), true, None, 1_000_000_000), "{d:?}");
            }
        }
    }
}
