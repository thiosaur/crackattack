//! Startup targets (`--open`).
//!
//! A `BootTarget` is a stable, semantic id (`menu:`, `screen:` or `flow:`). It resolves to
//! the same sequence of `AppCommand`s the menu would produce. The sequence is pushed into
//! `pending_startup`. Ids do not depend on menu positions, so reorganizing the menu breaks
//! no id.
//!
//! Namespaces (UI structure is not the same as product intent):
//! - `menu:`   logical navigation in the menu tree (`menu:info/credits`, `menu:network/opponents`);
//! - `screen:` a screen or overlay opened directly (`screen:lan`);
//! - `flow:`   a product scenario (`flow:play/vs-computer`, `flow:network/host`).

use crackattack_core::virtual_player::AIDifficulty;
use std::net::IpAddr;

use crate::app::command::AppCommand;
use crate::app::menu_tree::InfoSection;
use crate::app::meta_state::{FlowState, GameMode, OptionsTab};

/// Context needed to resolve a target into commands. Supplied by the bootstrap.
#[derive(Clone, Copy)]
pub struct BootContext {
    pub seed: u32,
    pub ranking_entry_score: i32,
}

/// An addressable startup target. Each variant is one stable public id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootTarget {
    // ── flow: (product intent) ───────────────────────────────────────────────────────────
    /// `flow:play/solo`
    PlaySolo,
    /// `flow:play/vs-computer[:easy|medium|hard]`
    PlayVsComputer(AIDifficulty),
    /// `flow:network/host`: open the network screen and listen (optional fixed port, see
    /// `requested_listen_port`).
    NetworkHost { port: Option<u16> },
    /// `flow:network/join`: open the network screen and invite this address.
    NetworkJoin { addr: IpAddr, port: u16 },

    // ── screen: (direct overlay) ─────────────────────────────────────────────────────────
    /// `screen:lan`
    ScreenLan,

    // ── menu: (logical navigation) ───────────────────────────────────────────────────────
    /// `menu:options/<player|sound|keyboard|gamepad>` (Options always has sections).
    MenuOptions(OptionsTab),
    /// `menu:info/<rules|credits|updates>` et `menu:network/<rank|opponents>`.
    MenuInfo(InfoSection),
}

impl BootTarget {
    /// Listen port requested by this target (network host with a fixed port). Otherwise the
    /// dynamic range is used.
    pub fn requested_listen_port(&self) -> Option<u16> {
        match self {
            BootTarget::NetworkHost { port } => *port,
            _ => None,
        }
    }

    /// Resolves the target into the same `AppCommand` sequence as the matching menu action.
    pub fn resolve(&self, ctx: &BootContext) -> Vec<AppCommand> {
        // Starting a game is exactly the menu sequence (SelectMode, CreateGame, transition).
        // See `input_handler::MenuConfirmAction::SelectMode`.
        let launch = |mode: GameMode| -> Vec<AppCommand> {
            vec![
                AppCommand::SelectMode(mode.clone()),
                AppCommand::CreateGame {
                    mode,
                    seed: ctx.seed,
                    ranking_entry_score: ctx.ranking_entry_score,
                },
                AppCommand::StartTransition(FlowState::Playing, 0.5),
            ]
        };
        match self {
            BootTarget::PlaySolo => launch(GameMode::Solo),
            BootTarget::PlayVsComputer(diff) => launch(GameMode::VsComputer(*diff)),
            // Host: open the network screen (listen and announce). The fixed port goes
            // through `requested_listen_port`, which the bootstrap reads.
            BootTarget::NetworkHost { .. } | BootTarget::ScreenLan => {
                vec![AppCommand::OpenLanScreen { manual: false }]
            }
            // Join: open the network screen, then invite the address (the normal invite
            // flow, including the guard against inviting yourself).
            BootTarget::NetworkJoin { addr, port } => vec![
                AppCommand::OpenLanScreen { manual: false },
                AppCommand::ConnectToPeer { addr: *addr, port: *port },
            ],
            BootTarget::MenuOptions(tab) => {
                // Deep link: the public id names a specific section, so carry it.
                vec![AppCommand::OpenOptionsModal { tab: Some(*tab) }]
            }
            BootTarget::MenuInfo(section) => vec![AppCommand::OpenInfoScreen(*section)],
        }
    }

    /// Parses a public id (`menu:`, `screen:`, `flow:`). Returns `Err` with a clear message
    /// otherwise.
    pub fn parse(id: &str) -> Result<BootTarget, String> {
        let (ns, rest) = id.split_once(':')
            .ok_or_else(|| format!("invalid target '{id}' (expected menu:/screen:/flow:...)"))?;
        match ns {
            "flow" => match rest {
                "play/solo" => Ok(BootTarget::PlaySolo),
                "network/host" => Ok(BootTarget::NetworkHost { port: None }),
                _ => {
                    // flow:play/vs-computer[:diff]
                    if let Some(diff) = rest.strip_prefix("play/vs-computer") {
                        let d = match diff {
                            "" | ":medium" => AIDifficulty::Medium,
                            ":easy" => AIDifficulty::Easy,
                            ":hard" => AIDifficulty::Hard,
                            other => return Err(format!("unknown difficulty '{other}' (easy|medium|hard)")),
                        };
                        return Ok(BootTarget::PlayVsComputer(d));
                    }
                    // flow:network/join needs an address, so it cannot be a plain id.
                    // Use the preset (with session:network.host/port).
                    Err(format!("unknown flow '{rest}'"))
                }
            },
            "screen" => match rest {
                "lan" => Ok(BootTarget::ScreenLan),
                _ => Err(format!("unknown screen '{rest}'")),
            },
            "menu" => match rest {
                "options/player" => Ok(BootTarget::MenuOptions(OptionsTab::Player)),
                "options/sound" => Ok(BootTarget::MenuOptions(OptionsTab::Sound)),
                "options/keyboard" => Ok(BootTarget::MenuOptions(OptionsTab::Keyboard)),
                "options/gamepad" => Ok(BootTarget::MenuOptions(OptionsTab::Gamepad)),
                "info/rules" => Ok(BootTarget::MenuInfo(InfoSection::Rules)),
                "info/credits" => Ok(BootTarget::MenuInfo(InfoSection::Credits)),
                "info/updates" => Ok(BootTarget::MenuInfo(InfoSection::Updates)),
                "network/rank" => Ok(BootTarget::MenuInfo(InfoSection::NetworkRank)),
                "network/opponents" => Ok(BootTarget::MenuInfo(InfoSection::NetworkOpponents)),
                _ => Err(format!("unknown menu '{rest}'")),
            },
            _ => Err(format!("unknown namespace '{ns}:' (menu:/screen:/flow:)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> BootContext { BootContext { seed: 42, ranking_entry_score: 1 } }

    #[test]
    fn parse_flow_targets() {
        assert_eq!(BootTarget::parse("flow:play/solo").unwrap(), BootTarget::PlaySolo);
        assert_eq!(BootTarget::parse("flow:play/vs-computer").unwrap(),
            BootTarget::PlayVsComputer(AIDifficulty::Medium));
        assert_eq!(BootTarget::parse("flow:play/vs-computer:hard").unwrap(),
            BootTarget::PlayVsComputer(AIDifficulty::Hard));
        assert_eq!(BootTarget::parse("flow:network/host").unwrap(),
            BootTarget::NetworkHost { port: None });
        // Difficulty suffix: every value, and the default when it is absent.
        assert_eq!(BootTarget::parse("flow:play/vs-computer:easy").unwrap(),
            BootTarget::PlayVsComputer(AIDifficulty::Easy));
        assert_eq!(BootTarget::parse("flow:play/vs-computer:medium").unwrap(),
            BootTarget::PlayVsComputer(AIDifficulty::Medium));
    }

    #[test]
    fn parse_menu_screen_targets() {
        assert_eq!(BootTarget::parse("screen:lan").unwrap(), BootTarget::ScreenLan);
        assert!(matches!(BootTarget::parse("menu:options/sound").unwrap(),
            BootTarget::MenuOptions(OptionsTab::Sound)));
        assert!(matches!(BootTarget::parse("menu:network/opponents").unwrap(),
            BootTarget::MenuInfo(InfoSection::NetworkOpponents)));
        // The other public ids (a wrong arm in the `match` would map one to another target).
        assert!(matches!(BootTarget::parse("menu:options/player").unwrap(),
            BootTarget::MenuOptions(OptionsTab::Player)));
        assert!(matches!(BootTarget::parse("menu:options/keyboard").unwrap(),
            BootTarget::MenuOptions(OptionsTab::Keyboard)));
        assert!(matches!(BootTarget::parse("menu:options/gamepad").unwrap(),
            BootTarget::MenuOptions(OptionsTab::Gamepad)));
        assert!(matches!(BootTarget::parse("menu:info/rules").unwrap(),
            BootTarget::MenuInfo(InfoSection::Rules)));
        assert!(matches!(BootTarget::parse("menu:info/credits").unwrap(),
            BootTarget::MenuInfo(InfoSection::Credits)));
        assert!(matches!(BootTarget::parse("menu:info/updates").unwrap(),
            BootTarget::MenuInfo(InfoSection::Updates)));
        assert!(matches!(BootTarget::parse("menu:network/rank").unwrap(),
            BootTarget::MenuInfo(InfoSection::NetworkRank)));
    }

    #[test]
    fn parse_errors_are_clear() {
        // Each error names the faulty element, so the CLI message is usable.
        for (id, culprit) in [
            ("play/solo", "play/solo"),         // no namespace
            ("flow:play/xxx", "play/xxx"),
            ("nope:foo", "nope"),
            ("flow:play/vs-computer:insane", "insane"),
            ("flow:network/join", "network/join"), // needs an address: preset only
            ("screen:nowhere", "nowhere"),
            ("menu:options/nothing", "options/nothing"),
        ] {
            let err = BootTarget::parse(id).expect_err(id);
            assert!(err.contains(culprit), "'{id}': message '{err}' does not mention '{culprit}'");
        }
    }

    /// Launching resolves the same sequence as the menu (SelectMode, CreateGame, transition).
    #[test]
    fn resolve_launch_matches_menu_sequence() {
        let cmds = BootTarget::PlayVsComputer(AIDifficulty::Hard).resolve(&ctx());
        assert!(matches!(cmds[0], AppCommand::SelectMode(GameMode::VsComputer(AIDifficulty::Hard))));
        assert!(matches!(cmds[1], AppCommand::CreateGame { mode: GameMode::VsComputer(AIDifficulty::Hard), seed: 42, .. }));
        assert!(matches!(cmds[2], AppCommand::StartTransition(FlowState::Playing, _)));
    }

    #[test]
    fn resolve_network_join_opens_then_invites() {
        let addr: IpAddr = "127.0.0.1".parse().unwrap();
        let cmds = BootTarget::NetworkJoin { addr, port: 45690 }.resolve(&ctx());
        assert!(matches!(cmds[0], AppCommand::OpenLanScreen { manual: false }));
        assert!(matches!(cmds[1], AppCommand::ConnectToPeer { port: 45690, .. }));
    }

    #[test]
    fn resolve_options_carries_the_section() {
        // Public id contract: `menu:options/<section>` opens that section (deep link),
        // not the default tab.
        for (id, tab) in [
            ("menu:options/player", OptionsTab::Player),
            ("menu:options/sound", OptionsTab::Sound),
            ("menu:options/keyboard", OptionsTab::Keyboard),
            ("menu:options/gamepad", OptionsTab::Gamepad),
        ] {
            let cmds = BootTarget::parse(id).unwrap().resolve(&ctx());
            assert!(
                matches!(cmds.as_slice(), [AppCommand::OpenOptionsModal { tab: Some(t) }] if *t == tab),
                "{id} → {cmds:?}"
            );
        }
    }

    #[test]
    fn host_requests_fixed_port() {
        assert_eq!(BootTarget::NetworkHost { port: Some(45691) }.requested_listen_port(), Some(45691));
        assert_eq!(BootTarget::PlaySolo.requested_listen_port(), None);
    }
}
