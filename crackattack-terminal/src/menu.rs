// The start menu: solo, or a duel against the computer at one of its three levels.

use std::time::{Duration, Instant};

use crackattack_core::ai_difficulty::AIDifficulty;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal;

use crate::duel;
use crate::lobby::{Presence, State};
use crate::{present, render, Ending, Limits, Mode};

/// What the menu came to.
pub(crate) enum Outcome {
    Play(Mode),
    /// A duel against the player of the server picked in the list: to be joined.
    Duel(String, Option<duel::Waiting>),
    /// A player of the server picked this one, who waited on the players page.
    Opponent(duel::Opponent),
    Quit,
    /// A limit was reached, or the terminal is gone.
    Ending(Ending),
}

/// The pages of the menu: the first, the opponents of `Vs computer`, and the players of the
/// server who wait for a duel.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Page {
    Top,
    Computer,
    Players,
}

impl Page {
    /// What a page with fixed items offers; `online`: the games of the server see each other
    /// (`--lobby`). The players page is made of whoever waits: see `player_items`.
    fn content(self, online: bool) -> &'static render::MenuPage {
        match self {
            Page::Top if online => &render::MENU_TOP_ONLINE,
            Page::Top | Page::Players => &render::MENU_TOP,
            Page::Computer => &render::MENU_COMPUTER,
        }
    }

    /// The page this one was opened from, and the item that opened it.
    fn parent(self) -> (Page, usize) {
        match self {
            Page::Players => (Page::Top, 2),
            _ => (Page::Top, 1),
        }
    }
}

/// What a key does to the menu.
#[derive(Debug, PartialEq)]
enum Step {
    Select(usize),
    Choose,
    /// Q or Escape: one page back, or out of the menu from the first page.
    Back,
    Nothing,
}

/// What choosing an item does.
#[derive(Debug, PartialEq)]
enum Action {
    Play(Mode),
    Duel(String),
    Open(Page),
    Back,
    Quit,
}

/// What item `index` of `page` does (the players page: see `player_action`).
fn action(page: Page, index: usize, online: bool) -> Action {
    match (page, index) {
        (Page::Top, 0) => Action::Play(Mode::Solo),
        (Page::Top, 1) => Action::Open(Page::Computer),
        (Page::Top, 2) if online => Action::Open(Page::Players),
        (Page::Computer, 0) => Action::Play(Mode::Versus(AIDifficulty::Easy)),
        (Page::Computer, 1) => Action::Play(Mode::Versus(AIDifficulty::Medium)),
        (Page::Computer, 2) => Action::Play(Mode::Versus(AIDifficulty::Hard)),
        (Page::Computer, _) | (Page::Players, _) => Action::Back,
        (Page::Top, _) => Action::Quit,
    }
}

/// The items of the players page: each player who waits (by name), then Back.
fn player_items(waiting: &[(String, std::net::SocketAddr)]) -> Vec<String> {
    let mut items: Vec<String> = waiting.iter().map(|(name, _)| name.clone()).collect();
    items.push("Back".to_string());
    items
}

/// What item `index` of the players page does.
fn player_action(items: &[String], index: usize) -> Action {
    match items.get(index) {
        Some(name) if index + 1 < items.len() => Action::Duel(name.clone()),
        _ => Action::Back,
    }
}

fn step(len: usize, selected: usize, code: KeyCode) -> Step {
    let last = len - 1;
    let code = match code {
        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
        code => code,
    };
    match code {
        KeyCode::Up | KeyCode::Char('w') => Step::Select(selected.checked_sub(1).unwrap_or(last)),
        KeyCode::Down | KeyCode::Char('s') => Step::Select(if selected == last { 0 } else { selected + 1 }),
        KeyCode::Enter | KeyCode::Char(' ') => Step::Choose,
        KeyCode::Char('q') | KeyCode::Esc => Step::Back,
        _ => Step::Nothing,
    }
}

/// Shows the menu until the player chooses, quits, or a limit is reached. The terminal is
/// already taken by the caller.
pub(crate) fn run(limits: &Limits, compact: bool, mut presence: Option<&mut Presence>, seed: u32) -> Outcome {
    let mut renderer = render::Renderer::new();
    let (mut page, mut selected) = (Page::Top, 0);
    // On the players page, once the player has moved: the item chosen, by name, so that it
    // stays chosen while players arrive and leave and the list changes under it.
    let mut selected_name: Option<String> = None;
    let online = presence.is_some();
    // On the players page the game waits for a duel: it can be picked by the others there.
    let mut waiting_room: Option<duel::Waiting> = None;
    let mut last_activity = Instant::now();
    // Keys pressed before the menu came up (the end of a game) do not choose anything.
    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    if let Some(presence) = presence.as_deref() {
        presence.set_state(&State::Menu);
    }
    loop {
        if let Some(presence) = presence.as_deref_mut() {
            presence.poll();
        }
        if let (Some(room), Some(presence)) = (waiting_room.as_mut(), presence.as_deref()) {
            if let Some(opponent) = room.accept(presence.name(), seed) {
                return Outcome::Opponent(opponent);
            }
        }
        let waiting = presence.as_deref().map(|p| p.waiting()).unwrap_or_default();
        let items = player_items(&waiting);
        let len = if page == Page::Players { items.len() } else { page.content(online).items.len() };
        if page == Page::Players {
            // Followed by name once the player has moved; until then the first item stays chosen
            // (the first player to arrive, not the Back that was alone before).
            selected = match &selected_name {
                Some(name) => items.iter().position(|item| item == name).unwrap_or(selected.min(len - 1)),
                None => selected.min(len - 1),
            };
        }
        let size = terminal::size().ok().map(|(columns, rows)| (columns as usize, rows as usize));
        renderer.set_placement(render::place(size, compact));
        let view = presence.as_deref().map(|p| p.view());
        let text = if page == Page::Players {
            // Nobody else on the page yet: the heading says what the game is doing.
            let heading = if items.len() == 1 { "Vs player: waiting for another player" } else { "Vs player" };
            renderer.menu_list(heading, &items, selected, "Arrows: choose   Enter: duel   Q: back", view.as_ref())
        } else {
            renderer.menu(page.content(online), selected, view.as_ref())
        };
        if !present(&text) {
            return Outcome::Ending(Ending::Hangup);
        }
        // Woken up now and then to look at the limits.
        match event::poll(Duration::from_millis(100)) {
            Err(_) => return Outcome::Ending(Ending::Hangup),
            Ok(false) => {}
            Ok(true) => match event::read() {
                Err(_) => return Outcome::Ending(Ending::Hangup),
                Ok(Event::Key(KeyEvent { code, kind: KeyEventKind::Press, .. })) => {
                    last_activity = Instant::now();
                    let go = match step(len, selected, code) {
                        Step::Select(index) => {
                            selected = index;
                            selected_name = items.get(index).cloned();
                            None
                        }
                        Step::Choose if page == Page::Players => Some(player_action(&items, selected)),
                        Step::Choose => Some(action(page, selected, online)),
                        Step::Back => Some(if page == Page::Top { Action::Quit } else { Action::Back }),
                        Step::Nothing => None,
                    };
                    match go {
                        Some(Action::Play(mode)) => return Outcome::Play(mode),
                        // Picking someone: this game stops waiting (the caller joins them).
                        // Two players who pick each other at once must not both stop waiting:
                        // the one whose pseudo comes first keeps its waiting room open.
                        Some(Action::Duel(target)) => {
                            let first = presence.as_deref().is_some_and(|p| p.name() < target.as_str());
                            let room = waiting_room.take().filter(|_| first);
                            // Not waiting any more (the room is closed): nobody must be told to come.
                            if room.is_none() {
                                if let Some(presence) = presence.as_deref() {
                                    presence.set_state(&State::Menu);
                                }
                            }
                            return Outcome::Duel(target, room);
                        }
                        Some(Action::Quit) => return Outcome::Quit,
                        Some(Action::Open(next)) => {
                            (page, selected, selected_name) = (next, 0, None);
                            if next == Page::Players {
                                waiting_room = presence.as_deref().and_then(duel::Waiting::open);
                            }
                        }
                        // Back to the page that opened this one, on the item that did.
                        Some(Action::Back) => {
                            if page == Page::Players {
                                waiting_room = None;
                                if let Some(presence) = presence.as_deref() {
                                    presence.set_state(&State::Menu);
                                }
                            }
                            (page, selected) = page.parent();
                            selected_name = None;
                        }
                        None => {}
                    }
                }
                Ok(_) => {}
            },
        }
        if let Some(ending) = limits.reached(last_activity, Instant::now()) {
            return Outcome::Ending(ending);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrows_wrap_around_the_items_of_each_page() {
        let top_last = render::MENU_TOP.items.len();
        assert_eq!(step(top_last, 0, KeyCode::Up), Step::Select(top_last - 1));
        assert_eq!(step(top_last, top_last - 1, KeyCode::Down), Step::Select(0));
        let computer = render::MENU_COMPUTER.items.len();
        assert_eq!(step(computer, computer - 1, KeyCode::Down), Step::Select(0));
        assert_eq!(step(computer, 1, KeyCode::Char('S')), Step::Select(2));
    }

    #[test]
    fn enter_chooses_and_q_or_escape_go_back() {
        assert_eq!(step(3, 0, KeyCode::Enter), Step::Choose);
        assert_eq!(step(3, 0, KeyCode::Char(' ')), Step::Choose);
        assert_eq!(step(3, 0, KeyCode::Esc), Step::Back);
        assert_eq!(step(4, 0, KeyCode::Char('Q')), Step::Back);
        assert_eq!(step(3, 0, KeyCode::Char('x')), Step::Nothing);
    }

    #[test]
    fn every_item_does_what_its_label_says() {
        let top = render::MENU_TOP.items;
        assert_eq!((top[0], top[1], top[2]), ("Solo", "Vs computer", "Quit"));
        assert_eq!(action(Page::Top, 0, false), Action::Play(Mode::Solo));
        assert_eq!(action(Page::Top, 1, false), Action::Open(Page::Computer));
        assert_eq!(action(Page::Top, 2, false), Action::Quit);
        let levels = render::MENU_COMPUTER.items;
        assert_eq!(levels.len(), 4);
        assert_eq!(action(Page::Computer, 0, false), Action::Play(Mode::Versus(AIDifficulty::Easy)));
        assert_eq!(action(Page::Computer, 1, false), Action::Play(Mode::Versus(AIDifficulty::Medium)));
        assert_eq!(action(Page::Computer, 2, false), Action::Play(Mode::Versus(AIDifficulty::Hard)));
        assert_eq!(action(Page::Computer, 3, false), Action::Back);
        // With a lobby, Vs player (between Vs computer and Quit) opens the list of players.
        let online = render::MENU_TOP_ONLINE.items;
        assert_eq!(online, &["Solo", "Vs computer", "Vs player", "Quit"]);
        assert_eq!(action(Page::Top, 2, true), Action::Open(Page::Players));
        assert_eq!(action(Page::Top, 3, true), Action::Quit);
        // Back returns to the item that opened the page.
        assert_eq!(Page::Computer.parent(), (Page::Top, 1));
        assert_eq!(Page::Players.parent(), (Page::Top, 2));
        assert_eq!(online[Page::Players.parent().1], "Vs player");
        assert_eq!(top[Page::Computer.parent().1], render::MENU_COMPUTER.heading.unwrap());
    }

    #[test]
    fn the_players_page_lists_only_those_who_wait_then_back() {
        assert_eq!(player_items(&[]), ["Back"]);
        let waiting = vec![("Bob".to_string(), "127.0.0.1:4001".parse().unwrap()), ("Cy".to_string(), "127.0.0.1:4002".parse().unwrap())];
        let items = player_items(&waiting);
        assert_eq!(items, ["Bob", "Cy", "Back"]);
        assert_eq!(player_action(&items, 0), Action::Duel("Bob".to_string()));
        assert_eq!(player_action(&items, 1), Action::Duel("Cy".to_string()));
        assert_eq!(player_action(&items, 2), Action::Back);
        assert_eq!(player_action(&player_items(&[]), 0), Action::Back);
    }
}
