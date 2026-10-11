//! **Pure** model of the manual network connection screen.
//!
//! The typed values (IPv4, IPv6, port) and the two focuses (screen ring and on-screen
//! keyboard grid) are owned by the **session** (`LanScreenState.manual`), not by egui
//! temporary data. This follows the contract of screens with shared focus
//! (`renderer/ui/helpers.rs`):
//! - the renderer (`renderer/ui/network_modal.rs`) **reads** this state and only mutates it
//!   through the intended paths (hover sets a transition, click mutates the session `&mut`
//!   directly);
//! - keyboard **and** gamepad navigation go through the router
//!   (`main.rs::route_lan_intent`, then `AppCommand::Manual*`, then `session_runtime`).

use crate::app::onscreen_keyboard::{osk_activate, osk_focus_to, osk_move, InputFormat, KbOutcome};

/// Editable field of the manual screen (with a matching on-screen keyboard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualField {
    Ipv4,
    Ipv6,
    Port,
}

impl ManualField {
    /// Input format of the field (character set, max length, default).
    pub fn format(self) -> InputFormat {
        match self {
            ManualField::Ipv4 => InputFormat::ip_address(),
            ManualField::Ipv6 => InputFormat::ipv6(),
            ManualField::Port => InputFormat::port(),
        }
    }
    /// Whether the typed value is **well formed** for this field. This is a structural check,
    /// beyond the character filter. An empty string is valid (field left blank).
    pub fn is_valid(self, v: &str) -> bool {
        let v = v.trim();
        if v.is_empty() {
            return true;
        }
        match self {
            ManualField::Ipv4 => v.parse::<std::net::Ipv4Addr>().is_ok(),
            ManualField::Ipv6 => v.parse::<std::net::Ipv6Addr>().is_ok(),
            ManualField::Port => v.parse::<u16>().is_ok(),
        }
    }
    pub fn value(self, st: &ManualConnectState) -> &String {
        match self {
            ManualField::Ipv4 => &st.ipv4,
            ManualField::Ipv6 => &st.ipv6,
            ManualField::Port => &st.port,
        }
    }
    pub fn value_mut(self, st: &mut ManualConnectState) -> &mut String {
        match self {
            ManualField::Ipv4 => &mut st.ipv4,
            ManualField::Ipv6 => &mut st.ipv6,
            ManualField::Port => &mut st.port,
        }
    }
}

/// Focusable item of the manual connection screen (**vertical** navigation).
/// `Copy` only exists when a local address was detected (network present).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ManualFocus {
    Copy,
    #[default]
    Ipv4,
    Ipv6,
    Port,
    Paste,
    Connect,
    Back,
}

impl ManualFocus {
    /// Focusable items in vertical order (top to bottom). `has_copy` includes the
    /// "Copy my address" button (only present when the local network was detected).
    pub fn ring(has_copy: bool) -> &'static [ManualFocus] {
        use ManualFocus::*;
        if has_copy {
            &[Copy, Ipv4, Ipv6, Port, Paste, Connect, Back]
        } else {
            &[Ipv4, Ipv6, Port, Paste, Connect, Back]
        }
    }
    /// Moves the focus by `dir` (+1 or -1) in the ring, **wrapping** (vertical navigation
    /// is cyclic).
    pub fn moved(self, has_copy: bool, dir: i32) -> ManualFocus {
        let ring = Self::ring(has_copy);
        let cur = ring.iter().position(|f| *f == self).unwrap_or(0);
        ring[crate::app::focus::ring_move(cur, ring.len(), dir)]
    }
}

/// **Session** state of the manual connection screen: the three typed values, the field
/// being edited on the on-screen keyboard, and the two focuses. Lives in
/// `LanScreenState.manual` (reset each time the LAN screen opens, like the rest of
/// `LanScreenState`).
#[derive(Debug, Clone, Default)]
pub struct ManualConnectState {
    pub ipv4: String,
    pub ipv6: String,
    pub port: String,
    /// Field being edited on the on-screen keyboard (`None` = none, screen nav is active).
    pub editing: Option<ManualField>,
    /// Focus (row, column) in the on-screen keyboard grid.
    pub kb_focus: (usize, usize),
    /// Target x ("goal-X") for vertical keyboard navigation (see `onscreen_keyboard`).
    pub kb_goal_x: f32,
    /// Focused item for screen navigation (outside the on-screen keyboard).
    pub nav_focus: ManualFocus,
}

impl ManualConnectState {
    // --- Screen navigation (vertical ring) ---

    /// Moves the screen focus by `dir` (+1 or -1), wrapping. No effect while the on-screen
    /// keyboard is open (it captures navigation).
    pub fn nav_move(&mut self, has_copy: bool, dir: i32) {
        if self.editing.is_none() {
            self.nav_focus = self.nav_focus.moved(has_copy, dir);
        }
    }

    // --- On-screen keyboard (same model as `NameInputState` in the options) ---

    /// Opens the on-screen keyboard on `field` (initial focus at the top left).
    pub fn open_editor(&mut self, field: ManualField) {
        self.editing = Some(field);
        osk_focus_to(&mut self.kb_focus, &mut self.kb_goal_x, field.format().charset, 0, 0);
    }

    /// Moves the keyboard focus (vertical moves reproject onto the target x, horizontal moves wrap).
    pub fn kb_move(&mut self, dx: i32, dy: i32) {
        if let Some(field) = self.editing {
            osk_move(&mut self.kb_focus, &mut self.kb_goal_x, field.format().charset, dx, dy);
        }
    }

    /// Activates the focused keyboard key (insert, backspace, clear or OK).
    /// Closes and finalizes if the key is `Done`.
    pub fn kb_activate(&mut self) {
        let Some(field) = self.editing else { return };
        let format = field.format();
        let focus = self.kb_focus;
        if osk_activate(field.value_mut(self), focus, format) == KbOutcome::Close {
            self.kb_close();
        }
    }

    /// Closes the on-screen keyboard and **finalizes** the field (trim plus the format default).
    /// Covers every exit: OK, Enter, Esc/B, click.
    pub fn kb_close(&mut self) {
        if let Some(field) = self.editing.take() {
            let finalized = field.format().finalize(field.value(self));
            *field.value_mut(self) = finalized;
        }
    }

    // --- Paste / prefill ---

    /// Splits a pasted address into the right fields. `ip:port` or `[v6]:port` fills the v4
    /// **or** v6 field (by family) and the port. A bare `ip` (or `[v6]` in brackets) fills the
    /// matching field and leaves the port unchanged.
    pub fn paste(&mut self, s: &str) {
        use std::net::IpAddr;
        let s = s.trim();
        if let Ok(sa) = s.parse::<std::net::SocketAddr>() {
            self.set_ip(sa.ip());
            self.port = sa.port().to_string();
        } else {
            // Bare IP, possibly in brackets (`[2a01:e0a::1]`). Unwrap before parsing: a
            // bracketed IPv6 **without a port** parses as neither `SocketAddr` nor `IpAddr`.
            let bare = s.strip_prefix('[').and_then(|x| x.strip_suffix(']')).unwrap_or(s);
            if let Ok(ip) = bare.parse::<IpAddr>() {
                self.set_ip(ip);
            }
        }
    }

    /// Sets an address in the field of its family (prefill when switching from reconnect to manual).
    pub fn set_ip(&mut self, ip: std::net::IpAddr) {
        match ip {
            std::net::IpAddr::V4(a) => self.ipv4 = a.to_string(),
            std::net::IpAddr::V6(a) => self.ipv6 = a.to_string(),
        }
    }

    // --- Validation / connection target (single source, shared by rendering and activation) ---

    /// Connection target: IPv6 if set and valid, otherwise IPv4.
    pub fn target_ip(&self) -> Option<std::net::IpAddr> {
        self.ipv6.trim().parse::<std::net::Ipv6Addr>().ok()
            .map(std::net::IpAddr::V6)
            .or_else(|| self.ipv4.trim().parse::<std::net::Ipv4Addr>().ok()
                .map(std::net::IpAddr::V4))
    }

    /// Effective port: empty gives the default port, otherwise the parsed value (`None` = out of range).
    pub fn effective_port(&self) -> Option<u16> {
        let t = self.port.trim();
        if t.is_empty() {
            Some(crackattack_net::connection::TCP_PORT_START)
        } else {
            t.parse::<u16>().ok()
        }
    }

    /// Full target (address and port) if the input allows connecting.
    pub fn connect_target(&self) -> Option<(std::net::IpAddr, u16)> {
        Some((self.target_ip()?, self.effective_port()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_wraps_and_respects_copy_presence() {
        // With Copy: the ring wraps from Back to Copy.
        assert_eq!(ManualFocus::Back.moved(true, 1), ManualFocus::Copy);
        assert_eq!(ManualFocus::Copy.moved(true, -1), ManualFocus::Back);
        // Without Copy: Ipv4 is the first item.
        assert_eq!(ManualFocus::Back.moved(false, 1), ManualFocus::Ipv4);
        assert_eq!(ManualFocus::Ipv4.moved(false, -1), ManualFocus::Back);
        // A focus outside the ring (Copy after the network went away) falls back to the start.
        assert_eq!(ManualFocus::Copy.moved(false, 1), ManualFocus::Ipv6);
    }

    #[test]
    fn nav_move_blocked_while_editing() {
        let mut st = ManualConnectState::default();
        st.open_editor(ManualField::Ipv4);
        st.nav_move(true, 1);
        assert_eq!(st.nav_focus, ManualFocus::Ipv4); // unchanged: the on-screen keyboard captures navigation
        st.kb_close();
        st.nav_move(true, 1);
        assert_eq!(st.nav_focus, ManualFocus::Ipv6);
    }

    #[test]
    fn paste_splits_socket_addresses() {
        let mut st = ManualConnectState::default();
        st.paste("192.168.1.20:45678");
        assert_eq!(st.ipv4, "192.168.1.20");
        assert_eq!(st.port, "45678");
        st.paste("[2a01:e0a::1]:8080");
        assert_eq!(st.ipv6, "2a01:e0a::1");
        assert_eq!(st.port, "8080");
        // Bare IP (bracketed or not): matching field, port unchanged.
        st.paste("10.0.0.7");
        assert_eq!(st.ipv4, "10.0.0.7");
        assert_eq!(st.port, "8080");
        st.paste("[fe80::2]");
        assert_eq!(st.ipv6, "fe80::2");
    }

    #[test]
    fn connect_target_prefers_ipv6_and_defaults_port() {
        let mut st = ManualConnectState::default();
        assert_eq!(st.connect_target(), None); // nothing set
        st.ipv4 = "192.168.1.20".into();
        let (ip, port) = st.connect_target().unwrap();
        assert_eq!(ip.to_string(), "192.168.1.20");
        assert_eq!(port, crackattack_net::connection::TCP_PORT_START); // empty port gives the default
        st.ipv6 = "2a01:e0a::1".into(); // a valid v6 takes priority
        assert_eq!(st.connect_target().unwrap().0.to_string(), "2a01:e0a::1");
        st.port = "99999".into(); // out of u16 range: no target
        assert_eq!(st.connect_target(), None);
    }

    #[test]
    fn kb_activate_done_finalizes_and_closes() {
        use crate::app::onscreen_keyboard::{done_focus, CharSet};
        let mut st = ManualConnectState::default();
        st.open_editor(ManualField::Port);
        st.ipv4 = "1.2.3.4".into();
        st.port = "  4567 ".into();
        let (r, c) = done_focus(CharSet::Port);
        st.kb_focus = (r, c);
        st.kb_activate(); // OK key: finalizes (trim) and closes
        assert_eq!(st.editing, None);
        assert_eq!(st.port, "4567");
    }
}
