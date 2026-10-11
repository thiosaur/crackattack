//! Presets (`--preset <name>`): named shortcuts for the command line.
//!
//! A preset is a name that expands into `--open` targets, `--set` overrides and/or a network
//! mode, plus optional positional arguments. It holds no game logic: it only rewrites to the
//! `--set` and `--open` mechanisms. Adding a launchable feature means adding one match arm.
//!
//! The legacy network launch aliases (`--vs network-create` / `--vs network-join`) reuse the
//! `net-host` / `net-join` expansion, so host and port are parsed in one place.

use super::cli::NetworkMode;

/// Default network port, shared with the legacy launch aliases.
pub const DEFAULT_NET_PORT: u16 = 45678;

/// Result of expanding a preset: fragments to merge into `CliArgs`.
#[derive(Debug, Default, PartialEq)]
pub struct PresetExpansion {
    /// `--open` targets (e.g. `flow:play/solo`).
    pub open_targets: Vec<String>,
    /// `--set` overrides (e.g. `session:network.host=…`).
    pub set_overrides: Vec<String>,
    /// Network mode (host or join). It has no unambiguous `--set` form.
    pub network: Option<NetworkMode>,
}

/// Returns the positional argument at `idx`, if present and not a flag (does not start with `-`).
fn positional<'a>(rest: &'a [String], idx: usize) -> Option<&'a str> {
    rest.get(idx).map(|s| s.as_str()).filter(|s| !s.starts_with('-'))
}

/// Expands the named preset using the arguments that follow it.
///
/// Returns the expansion and the number of positional arguments consumed, so the caller can
/// advance its argument cursor. Returns a readable error message otherwise.
pub fn expand(name: &str, rest: &[String]) -> Result<(PresetExpansion, usize), String> {
    let mut exp = PresetExpansion::default();
    match name {
        "solo" => {
            exp.open_targets.push("flow:play/solo".to_string());
            Ok((exp, 0))
        }
        "vs-computer" | "vs-ordi" => {
            // Optional positional difficulty. The preset consumes it, so there is no ordering
            // ambiguity with `--difficulty` (unlike `--vs-computer`).
            match positional(rest, 0) {
                Some(d @ ("easy" | "medium" | "hard")) => {
                    exp.open_targets.push(format!("flow:play/vs-computer:{d}"));
                    Ok((exp, 1))
                }
                Some(other) => Err(format!("difficulty \"{other}\" (expected easy|medium|hard)")),
                None => {
                    exp.open_targets.push("flow:play/vs-computer".to_string());
                    Ok((exp, 0))
                }
            }
        }
        "net-host" | "network-host" => {
            // Optional PORT. A lone port is ambiguous with `--set`; here the preset says
            // "host", so the port is a listening port: Create.
            let (port, consumed) = parse_opt_port(rest, 0)?;
            exp.network = Some(NetworkMode::Create { port });
            Ok((exp, consumed))
        }
        "net-join" | "network-join" => {
            // Required HOST, optional PORT.
            let host = positional(rest, 0)
                .ok_or_else(|| "net-join requires HOST (e.g. --preset net-join 192.168.1.5)".to_string())?;
            let (port, consumed) = parse_opt_port(rest, 1)?;
            exp.network = Some(NetworkMode::Join { host: host.to_string(), port });
            Ok((exp, 1 + consumed))
        }
        _ => Err(format!(
            "unknown preset \"{name}\" (solo | vs-computer [diff] | net-host [port] | net-join HOST [port])"
        )),
    }
}

/// Parses the optional port at position `idx`. Returns `(port, positional_args_consumed)`;
/// the default port is used when absent.
fn parse_opt_port(rest: &[String], idx: usize) -> Result<(u16, usize), String> {
    match positional(rest, idx) {
        Some(p) => p.parse::<u16>()
            .map(|port| (port, 1))
            .map_err(|_| format!("port \"{p}\" (integer 0-65535)")),
        None => Ok((DEFAULT_NET_PORT, 0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn solo_opens_flow() {
        let (e, n) = expand("solo", &[]).unwrap();
        assert_eq!(e.open_targets, vec!["flow:play/solo"]);
        assert_eq!(n, 0);
    }

    #[test]
    fn vs_computer_with_and_without_difficulty() {
        let (e, n) = expand("vs-computer", &args(&["hard"])).unwrap();
        assert_eq!(e.open_targets, vec!["flow:play/vs-computer:hard"]);
        assert_eq!(n, 1);
        let (e, n) = expand("vs-computer", &[]).unwrap();
        assert_eq!(e.open_targets, vec!["flow:play/vs-computer"]);
        assert_eq!(n, 0);
        // A following flag is not taken as a difficulty.
        let (e, n) = expand("vs-computer", &args(&["--seed", "5"])).unwrap();
        assert_eq!(e.open_targets, vec!["flow:play/vs-computer"]);
        assert_eq!(n, 0);
        assert!(expand("vs-computer", &args(&["insane"])).is_err());
    }

    #[test]
    fn net_host_resolves_port_ambiguity() {
        let (e, n) = expand("net-host", &args(&["45000"])).unwrap();
        assert_eq!(e.network, Some(NetworkMode::Create { port: 45000 }));
        assert_eq!(n, 1);
        let (e, n) = expand("net-host", &[]).unwrap();
        assert_eq!(e.network, Some(NetworkMode::Create { port: DEFAULT_NET_PORT }));
        assert_eq!(n, 0);
    }

    #[test]
    fn net_join_requires_host() {
        let (e, n) = expand("net-join", &args(&["192.168.1.5", "45000"])).unwrap();
        assert_eq!(e.network, Some(NetworkMode::Join { host: "192.168.1.5".into(), port: 45000 }));
        assert_eq!(n, 2);
        let (e, n) = expand("network-join", &args(&["10.0.0.1"])).unwrap();
        assert_eq!(e.network, Some(NetworkMode::Join { host: "10.0.0.1".into(), port: DEFAULT_NET_PORT }));
        assert_eq!(n, 1);
        assert!(expand("net-join", &[]).is_err());
        assert!(expand("net-join", &args(&["--foo"])).is_err()); // no valid host
    }

    #[test]
    fn unknown_preset_is_error() {
        assert!(expand("teleport", &[]).is_err());
    }

    #[test]
    fn bad_port_is_error() {
        assert!(expand("net-host", &args(&["99999"])).is_err());
    }
}
