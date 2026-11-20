// The start menu: the logo, the choices, the keys.

use super::lobby::LobbyView;
use super::palette::HINT_COLOR;
use super::panel::{logo_line, LOGO};
use super::visible_width;

/// A screen of the menu: what it offers, in order.
pub(crate) struct Page {
    /// Says where we are, on the pages below the first.
    pub heading: Option<&'static str>,
    pub items: &'static [&'static str],
    pub hint: &'static str,
}

/// The first page.
pub(crate) const TOP: Page = Page {
    heading: None,
    items: &["Solo", "Vs computer", "Quit"],
    hint: "Arrows: choose   Enter: select   Q: quit",
};

/// The first page when the games of a server see each other (`--lobby`).
pub(crate) const TOP_ONLINE: Page = Page {
    heading: None,
    items: &["Solo", "Vs computer", "Vs player", "Quit"],
    hint: "Arrows: choose   Enter: select   Q: quit",
};

/// The opponents of `Vs computer`, one per level of the AI.
pub(crate) const COMPUTER: Page = Page {
    heading: Some("Vs computer"),
    items: &["Logic (easy)", "Sigma (medium)", "Cortex (hard)", "Back"],
    hint: "Arrows: choose   Enter: play   Q: back",
};

/// Rows the items of a page may take: every page is laid out with this many, so nothing
/// above or below moves from one page to the next.
const SLOTS: usize = 4;
/// The page area: the heading (or a blank), the item slots, a blank row and the keys.
const PAGE_ROWS: usize = 1 + SLOTS + 1 + 1;
/// The last rows of the zone, kept for the lobby (the greeting, the players online) even
/// without one, so that nothing moves with `--lobby`.
const LOBBY_ROWS: usize = 2;

/// One of the item slots of a page.
struct Slot<'a> {
    /// What stands before the text: the cursor, an arrow saying the list goes on, or blanks.
    lead: &'a str,
    text: &'a str,
    selected: bool,
}

/// The menu `page` for a terminal of `columns` × `rows`, `selected` being the item chosen.
/// Centered. Every row has its place whatever the page: the logo, the heading, the item
/// slots and the keys start where they started, only what they hold changes, and the
/// logo does not depend on `--lobby` either. With a lobby, the last two rows greet the player
/// by pseudo and give the number of players connected. On a terminal too short for the logo,
/// the logo is left out.
pub(crate) fn lines(page: &Page, selected: usize, columns: usize, rows: usize, lobby: Option<&LobbyView>) -> Vec<String> {
    debug_assert!(page.items.len() <= SLOTS);
    let slots: Vec<Slot> = page.items.iter().enumerate()
        .map(|(i, text)| Slot { lead: if i == selected { "> " } else { "  " }, text, selected: i == selected })
        .collect();
    compose(page.heading.map(str::to_string), &slots, page.hint, columns, rows, lobby, false)
}

/// The first item shown of a list of `len` items, `selected` being the one chosen: the list
/// scrolls by itself, keeping the choice on the second row (so there is a row before it that
/// can say the list goes on) except at both ends.
pub(crate) fn window_start(len: usize, selected: usize) -> usize {
    selected.saturating_sub(1).min(len.saturating_sub(SLOTS))
}

/// A page whose items come from outside and may be more than the screen shows: `items`
/// under `heading`, a window of them at a time. An arrow stands before the first item shown
/// when the list goes on above it, and before the last when it goes on below.
pub(crate) fn list_lines(heading: &str, items: &[String], selected: usize, hint: &str, columns: usize, rows: usize, lobby: Option<&LobbyView>) -> Vec<String> {
    let start = window_start(items.len(), selected);
    let shown = &items[start..items.len().min(start + SLOTS)];
    let slots: Vec<Slot> = shown.iter().enumerate()
        .map(|(i, text)| {
            let at = start + i;
            let lead = if at == selected { "> " }
                else if i == 0 && start > 0 { "\u{25b2} " }
                else if i + 1 == shown.len() && start + shown.len() < items.len() { "\u{25bc} " }
                else { "  " };
            Slot { lead, text, selected: at == selected }
        })
        .collect();
    // Where the window is, when it cannot show everything.
    let heading = if items.len() > SLOTS { format!("{}  {}/{}", heading, selected + 1, items.len()) } else { heading.to_string() };
    compose(Some(heading), &slots, hint, columns, rows, lobby, true)
}

/// `invite`: the last two rows tell how to invite a friend, when the lobby has what to say,
/// in place of the greeting and the count.
fn compose(heading: Option<String>, slots: &[Slot], hint: &str, columns: usize, rows: usize, lobby: Option<&LobbyView>, invite: bool) -> Vec<String> {
    let logo_lines = LOGO.lines().count();
    let with_logo = rows >= logo_lines + PAGE_ROWS + LOBBY_ROWS;
    // A blank row between the logo and the heading when the zone has the room for it (not on
    // the compact one, which is exactly as tall as logo, page and lobby).
    let gap = usize::from(with_logo && rows > logo_lines + PAGE_ROWS + LOBBY_ROWS);
    let used = if with_logo { logo_lines + gap } else { 0 } + PAGE_ROWS + LOBBY_ROWS;
    let top = rows.saturating_sub(used) / 2;

    let logo_width = LOGO.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    let item_width = slots.iter().map(|s| s.text.chars().count() + 2).max().unwrap_or(0);
    // Each block of text is centered as a whole, so the items stay aligned with each other
    // and the logo (whose lines have different lengths) keeps its shape.
    let centered = |line: String, width: usize| format!("{}{}", " ".repeat(columns.saturating_sub(width) / 2), line);

    let mut out = vec![String::new(); top];
    if with_logo {
        out.extend((0..logo_lines).map(|row| centered(logo_line(row, None), logo_width)));
        out.extend(std::iter::repeat(String::new()).take(gap));
    }
    out.push(match &heading {
        Some(heading) => centered(format!("\x1b[38;5;{}m{}\x1b[39m", HINT_COLOR, heading), visible_width(heading)),
        None => String::new(),
    });
    for slot in 0..SLOTS {
        out.push(match slots.get(slot) {
            Some(s) if s.selected => centered(format!("\x1b[1m{}{}\x1b[22m", s.lead, s.text), item_width),
            Some(s) => centered(format!("{}{}", s.lead, s.text), item_width),
            None => String::new(),
        });
    }
    out.push(String::new());
    out.push(centered(format!("\x1b[38;5;{}m{}\x1b[39m", HINT_COLOR, hint), hint.chars().count()));
    // The greeting and the players connected, on the last two rows of the zone.
    if let Some(lobby) = lobby.filter(|_| rows >= out.len() + LOBBY_ROWS) {
        out.resize(rows - LOBBY_ROWS, String::new());
        if let (true, Some(command)) = (invite, &lobby.invite) {
            let title = "To invite a friend, give them:";
            out.push(centered(format!("\x1b[38;5;{}m{}\x1b[39m", HINT_COLOR, title), title.chars().count()));
            out.push(centered(format!("\x1b[1m{}\x1b[22m", command), command.chars().count()));
        } else {
            let greeting = lobby.greeting();
            out.push(centered(format!("Welcome \x1b[1m{}\x1b[22m", lobby.name), greeting.chars().count()));
            let online = lobby.online();
            out.push(centered(format!("\x1b[38;5;{}m{}\x1b[39m", HINT_COLOR, online), online.chars().count()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selected_item_is_marked_and_the_others_are_not() {
        let screen = lines(&TOP, 1, 80, 24, None).join("\n");
        assert!(screen.contains("> Vs computer"));
        assert!(!screen.contains("> Solo") && !screen.contains("> Quit"));
        assert!(screen.contains("Quit"));
    }

    #[test]
    fn the_menu_fits_80_by_24_and_a_short_terminal_loses_only_the_logo() {
        assert!(lines(&TOP, 0, 80, 24, None).len() <= 24);
        let short = lines(&TOP, 0, 80, 12, None);
        assert!(short.len() <= 12);
        assert!(short.join("\n").contains("Solo"));
    }

    #[test]
    fn no_line_is_wider_than_the_terminal() {
        for line in lines(&TOP, 1, 80, 24, None) {
            assert!(visible_width(&line) <= 80);
        }
    }
}
