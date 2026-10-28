//! Declarative menu tree.
//!
//! The menu structure lives here, once, as static data. `MenuState` (in `meta_state`)
//! is only a cursor (`path` + `selected`) on this tree.
//!
//! Design:
//! - Terminal actions are typed (`MenuAction`): the meaning is in the action, not in
//!   an index. `OpenOptions(OptionsTab)` carries the tab directly.
//! - Labels are resolved against the current locale (`LabelKey::localized`); they are
//!   not a fixed field of the node.
//! - An `enabled` extension point is planned (see `MenuState::labels`) but not used yet.
//! - No closures and no generic engine.

use crate::app::meta_state::OptionsTab;
use crackattack_core::ai_difficulty::AIDifficulty;

/// Section of the About/Info screen (Info submenu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoSection {
    Rules,
    Credits,
    /// "Updates" page: version, check status, download, setting.
    Updates,
    /// "Network rank" page (own rank card and game history, from the match journal).
    NetworkRank,
    /// "Opponents" page (last known rank and quick reconnect).
    NetworkOpponents,
}

/// Action triggered when a menu leaf is confirmed.
#[derive(Debug, Clone, Copy)]
pub enum MenuAction {
    StartSolo,
    StartVsComputer(AIDifficulty),
    /// Network: automatic discovery (mDNS).
    OpenNetwork,
    /// Network: manual connection (IP/port entry).
    OpenNetworkManual,
    OpenOptions(OptionsTab),
    /// Opens a section of the Info screen. The Info entry is a submenu.
    OpenInfo(InfoSection),
    /// Quits the game (drawn as a discreet icon).
    Quit,
    /// Returns to the parent menu (drawn as a discreet icon).
    Back,
}

/// Label key, resolved against the current locale at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelKey {
    Solo,
    VsComputer,
    VsNetwork,
    /// Network submenu: "Auto search" entry.
    NetworkAuto,
    /// Network submenu: "Manual connection" entry.
    NetworkManual,
    /// Network submenu: "Network rank" entry (local ranking page).
    NetworkRank,
    /// Network submenu: "Opponents" entry (last known rank and reconnect).
    NetworkOpponents,
    Options,
    /// Info / About. Not shown as text (drawn as an icon).
    Info,
    /// Info submenu: "Rules" entry.
    Rules,
    /// Info submenu: "Credits" entry.
    Credits,
    /// Info submenu: "Updates" entry.
    Updates,
    Easy,
    Medium,
    Hard,
    Player,
    Sound,
    Keyboard,
    Gamepad,
    /// Quit. Not shown as text (drawn as an icon); the label is mainly for accessibility.
    Quit,
    /// Back. Not shown as text (drawn as an icon).
    Back,
}

impl LabelKey {
    /// Text in the current locale. Not a static property of the node: it depends on
    /// `locale::t()`, so it is recomputed at each render.
    pub fn localized(self) -> &'static str {
        let t = crate::app::locale::t();
        match self {
            LabelKey::Solo => t.menu_solo,
            LabelKey::VsComputer => t.menu_vs_computer,
            LabelKey::VsNetwork => t.menu_vs_network,
            LabelKey::NetworkAuto => t.network_auto,
            LabelKey::NetworkManual => t.manual_connection,
            LabelKey::NetworkRank => t.menu_network_rank,
            LabelKey::NetworkOpponents => t.menu_network_opponents,
            LabelKey::Options => t.menu_options,
            LabelKey::Info => t.menu_info,
            LabelKey::Rules => t.menu_rules,
            LabelKey::Credits => t.menu_credits,
            LabelKey::Updates => t.menu_updates,
            LabelKey::Easy => t.menu_easy,
            LabelKey::Medium => t.menu_medium,
            LabelKey::Hard => t.menu_hard,
            LabelKey::Player => t.player_tab,
            LabelKey::Sound => t.sound_tab,
            LabelKey::Keyboard => t.keyboard_tab,
            LabelKey::Gamepad => t.gamepad_tab,
            LabelKey::Quit => t.menu_quit,
            LabelKey::Back => t.menu_back,
        }
    }
}

/// Menu node: a branch (submenu) or a leaf (terminal action).
pub enum MenuNode {
    Branch {
        label: LabelKey,
        /// Index selected by default when entering the branch (for example Medium).
        default: usize,
        children: &'static [MenuNode],
    },
    Leaf {
        label: LabelKey,
        action: MenuAction,
    },
}

impl MenuNode {
    pub fn label(&self) -> LabelKey {
        match self {
            MenuNode::Branch { label, .. } | MenuNode::Leaf { label, .. } => *label,
        }
    }
}

/// The menu definition, in one place.
pub static MENU_ROOT: &[MenuNode] = &[
    MenuNode::Leaf { label: LabelKey::Solo, action: MenuAction::StartSolo },
    MenuNode::Branch {
        label: LabelKey::VsComputer,
        default: 1, // Medium
        children: &[
            MenuNode::Leaf { label: LabelKey::Easy, action: MenuAction::StartVsComputer(AIDifficulty::Easy) },
            MenuNode::Leaf { label: LabelKey::Medium, action: MenuAction::StartVsComputer(AIDifficulty::Medium) },
            MenuNode::Leaf { label: LabelKey::Hard, action: MenuAction::StartVsComputer(AIDifficulty::Hard) },
            MenuNode::Leaf { label: LabelKey::Back, action: MenuAction::Back },
        ],
    },
    MenuNode::Branch {
        label: LabelKey::VsNetwork,
        default: 0, // Auto search
        children: &[
            MenuNode::Leaf { label: LabelKey::NetworkAuto, action: MenuAction::OpenNetwork },
            MenuNode::Leaf { label: LabelKey::NetworkManual, action: MenuAction::OpenNetworkManual },
            MenuNode::Leaf { label: LabelKey::NetworkRank, action: MenuAction::OpenInfo(InfoSection::NetworkRank) },
            MenuNode::Leaf { label: LabelKey::NetworkOpponents, action: MenuAction::OpenInfo(InfoSection::NetworkOpponents) },
            MenuNode::Leaf { label: LabelKey::Back, action: MenuAction::Back },
        ],
    },
    MenuNode::Branch {
        label: LabelKey::Options,
        default: 0,
        children: &[
            MenuNode::Leaf { label: LabelKey::Player, action: MenuAction::OpenOptions(OptionsTab::Player) },
            MenuNode::Leaf { label: LabelKey::Sound, action: MenuAction::OpenOptions(OptionsTab::Sound) },
            MenuNode::Leaf { label: LabelKey::Keyboard, action: MenuAction::OpenOptions(OptionsTab::Keyboard) },
            MenuNode::Leaf { label: LabelKey::Gamepad, action: MenuAction::OpenOptions(OptionsTab::Gamepad) },
            MenuNode::Leaf { label: LabelKey::Back, action: MenuAction::Back },
        ],
    },
    // Bottom icon bar (options, info, quit), drawn as a horizontal row.
    // Info is a submenu (Rules / Credits / Updates); its icon enters it like the Options gear.
    MenuNode::Branch {
        label: LabelKey::Info,
        default: 0,
        children: &[
            MenuNode::Leaf { label: LabelKey::Rules, action: MenuAction::OpenInfo(InfoSection::Rules) },
            MenuNode::Leaf { label: LabelKey::Credits, action: MenuAction::OpenInfo(InfoSection::Credits) },
            MenuNode::Leaf { label: LabelKey::Updates, action: MenuAction::OpenInfo(InfoSection::Updates) },
            MenuNode::Leaf { label: LabelKey::Back, action: MenuAction::Back },
        ],
    },
    MenuNode::Leaf { label: LabelKey::Quit, action: MenuAction::Quit },
];

/// Whether `path` goes through a branch with this label (for example, "am I in the
/// VS Network submenu?", where the network services run).
pub fn path_contains_label(path: &[usize], label: LabelKey) -> bool {
    let mut nodes: &'static [MenuNode] = MENU_ROOT;
    for &i in path {
        match nodes.get(i) {
            Some(MenuNode::Branch { label: l, children, .. }) => {
                if *l == label {
                    return true;
                }
                nodes = children;
            }
            _ => return false,
        }
    }
    false
}

/// Children visible at `path` (empty = root). An invalid path gives an empty slice.
pub fn children_at(path: &[usize]) -> &'static [MenuNode] {
    let mut nodes: &'static [MenuNode] = MENU_ROOT;
    for &i in path {
        match nodes.get(i) {
            Some(MenuNode::Branch { children, .. }) => nodes = children,
            _ => return &[],
        }
    }
    nodes
}
