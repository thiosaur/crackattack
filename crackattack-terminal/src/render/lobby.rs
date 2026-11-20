// The lobby on the screen: who is connected, and who just arrived or left.

use super::palette::{COUNTDOWN_ONE_COLOR, HINT_COLOR};
use super::versus::NAME_MAX;

/// What the screen shows of the lobby (`--lobby`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LobbyView {
    /// This player's pseudo.
    pub name: String,
    /// The others, by name, with what each is doing.
    pub others: Vec<(String, &'static str)>,
    /// An arrival or a departure, for a few seconds.
    pub notice: Option<String>,
    /// `--invite-command`: what to tell a friend who has no access yet (a server's games only).
    pub invite: Option<String>,
}

impl LobbyView {
    /// The block of at most `room` lines: who we are, the others (as many as fit, then how
    /// many more), and the notice last. Empty when `room` is 0.
    pub(crate) fn lines(&self, room: usize) -> Vec<String> {
        if room == 0 {
            return Vec::new();
        }
        let mut out = vec![format!("Online: \x1b[1m{}\x1b[22m", self.name)];
        let notice_rows = usize::from(self.notice.is_some() && room > 1);
        let slots = room.saturating_sub(1 + notice_rows);
        let shown = if self.others.len() > slots { slots.saturating_sub(1) } else { self.others.len() };
        for (name, state) in self.others.iter().take(shown) {
            out.push(format!("  {:<width$} \x1b[38;5;{}m{}\x1b[39m", name, HINT_COLOR, state, width = NAME_MAX));
        }
        if shown < self.others.len() && slots > 0 {
            out.push(format!("  \x1b[38;5;{}m+{} more\x1b[39m", HINT_COLOR, self.others.len() - shown));
        }
        if let (Some(notice), true) = (&self.notice, notice_rows == 1) {
            out.push(self.notice_line(notice));
        }
        out
    }

    /// The greeting of the menu, without styling: its width is what is centered.
    pub(crate) fn greeting(&self) -> String {
        format!("Welcome {}", self.name)
    }

    /// How many are connected, this player included: one line for the menu.
    pub(crate) fn online(&self) -> String {
        match self.others.len() + 1 {
            1 => "1 player online".to_string(),
            n => format!("{} players online", n),
        }
    }

    /// The notice alone, as a line.
    pub(crate) fn notice_line(&self, notice: &str) -> String {
        format!("\x1b[38;5;{}m{}\x1b[39m", COUNTDOWN_ONE_COLOR, notice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(others: usize, notice: bool) -> LobbyView {
        LobbyView {
            name: "Pixel".to_string(),
            others: (0..others).map(|i| (format!("Player{}", i), "menu")).collect(),
            notice: notice.then(|| "Zed joined".to_string()),
            invite: None,
        }
    }

    #[test]
    fn the_block_never_exceeds_its_room_and_says_how_many_are_left_out() {
        for room in 0..9 {
            for others in 0..12 {
                for notice in [false, true] {
                    assert!(view(others, notice).lines(room).len() <= room, "room {} others {}", room, others);
                }
            }
        }
        let lines = view(9, true).lines(5).join("\n");
        assert!(lines.contains("Pixel") && lines.contains("more") && lines.contains("Zed joined"), "{}", lines);
    }

    #[test]
    fn the_count_includes_this_player_and_is_singular_for_one() {
        assert_eq!(view(0, false).online(), "1 player online");
        assert_eq!(view(3, false).online(), "4 players online");
    }

    #[test]
    fn everyone_is_listed_when_there_is_room() {
        let lines = view(3, false).lines(8).join("\n");
        assert!(lines.contains("Player0") && lines.contains("Player2") && !lines.contains("more"));
    }
}
