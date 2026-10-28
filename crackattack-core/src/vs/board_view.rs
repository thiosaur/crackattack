//! Opponent board preview: a simplified picture of a board, and the rule that decides when its
//! owner publishes a new one.
//!
//! The picture keeps only what the HUD preview shows: which cells hold a block and which hold
//! garbage, over the playable rows. No colors, no cursor, no motion. Each view is a complete
//! picture, never a difference with the previous one: a view may be skipped or lost and the next
//! one still replaces everything. Its fixed size lets it travel later in a network control
//! message of fixed size.

use crate::constants::{GARBAGE_STORE_SIZE, PLAY_WIDTH, SAFE_HEIGHT};
use crate::game_state::GameState;
use crate::gameplay_event::GameplayEvent;
use crate::grid::CellType;

/// Rows of the view: grid rows 1 to `SAFE_HEIGHT`. Row 0, the one rising from below, is not
/// shown, and garbage above the top row is cut off.
pub const BOARD_VIEW_ROWS: usize = SAFE_HEIGHT;

/// Size of a view in bytes.
pub const BOARD_VIEW_LEN: usize = PLAY_WIDTH * BOARD_VIEW_ROWS;

/// Longest stretch of played ticks without a published view (the network communication period).
pub const BOARD_VIEW_PERIOD: i32 = 32;

/// Shortest gap between two published views, so a chain breaking several slabs in a row does
/// not send a burst.
pub const BOARD_VIEW_MIN_GAP: i32 = 8;

const CELL_EMPTY: u8 = 0;
const CELL_BLOCK: u8 = 1;
/// Garbage cells store their arena index plus this offset.
const CELL_GARBAGE_BASE: u8 = 2;
/// Highest value a cell may hold: the garbage arena has `GARBAGE_STORE_SIZE` slots.
const CELL_MAX: u8 = CELL_GARBAGE_BASE + GARBAGE_STORE_SIZE as u8 - 1;
const _: () = assert!(CELL_GARBAGE_BASE as usize + GARBAGE_STORE_SIZE - 1 <= u8::MAX as usize);

/// One cell of a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardCell {
    Empty,
    Block,
    /// Garbage, with a small identifier shared by all the cells of the same slab. A slab whose
    /// identifier was absent from the previous view has just appeared.
    Garbage(u8),
}

/// A simplified picture of a board, as the opponent preview shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpponentBoardView {
    /// Row-major from the bottom playable row (grid row 1) upward.
    cells: [u8; BOARD_VIEW_LEN],
}

impl OpponentBoardView {
    pub fn empty() -> Self {
        Self { cells: [CELL_EMPTY; BOARD_VIEW_LEN] }
    }

    pub fn from_game(game: &GameState) -> Self {
        let mut view = Self::empty();
        for row in 0..BOARD_VIEW_ROWS {
            for x in 0..PLAY_WIDTH {
                let cell = game.grid.get(x as i32, row as i32 + 1);
                view.cells[row * PLAY_WIDTH + x] = match cell.cell_type {
                    CellType::Empty => CELL_EMPTY,
                    // Every block counts, whatever it is doing (falling, swapping, dying).
                    CellType::Block => CELL_BLOCK,
                    CellType::Garbage => {
                        // The arena holds 2 × PLAY_HEIGHT slabs: the index always fits.
                        let id = u8::try_from(cell.id.max(0)).unwrap_or(u8::MAX - CELL_GARBAGE_BASE);
                        id.saturating_add(CELL_GARBAGE_BASE)
                    }
                };
            }
        }
        view
    }

    /// The view as sent over the network.
    pub fn to_bytes(&self) -> [u8; BOARD_VIEW_LEN] {
        self.cells
    }

    /// A view received over the network. `None` when the length is wrong or a cell holds a
    /// value no board can produce: a corrupt view is dropped, never drawn.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let cells: [u8; BOARD_VIEW_LEN] = bytes.try_into().ok()?;
        cells.iter().all(|&v| v <= CELL_MAX).then_some(Self { cells })
    }

    /// The cell at column `x` and view row `row` (0 = bottom playable row).
    pub fn cell(&self, x: usize, row: usize) -> BoardCell {
        match self.cells[row * PLAY_WIDTH + x] {
            CELL_EMPTY => BoardCell::Empty,
            CELL_BLOCK => BoardCell::Block,
            v => BoardCell::Garbage(v - CELL_GARBAGE_BASE),
        }
    }

    /// Whether some cell holds the garbage slab `id`.
    pub fn has_garbage(&self, id: u8) -> bool {
        self.cells.contains(&id.saturating_add(CELL_GARBAGE_BASE))
    }
}

/// Events after which the owner publishes at once (within the minimal gap): what the receiver
/// should see without waiting for the regular view.
fn is_board_event(event: &GameplayEvent) -> bool {
    matches!(
        event,
        GameplayEvent::GarbageLanded(_)
            | GameplayEvent::GarbageShattering(_)
            | GameplayEvent::CreepShift
            | GameplayEvent::GameOver { .. }
    )
}

/// Decides when the owner of a board publishes a new view.
///
/// A view goes out after a board event or after `BOARD_VIEW_PERIOD` ticks without one; any
/// publication restarts that count. Two publications are at least `BOARD_VIEW_MIN_GAP` ticks
/// apart: an event during the gap goes out at its end. A view identical to the last one is not
/// published. The end of the game publishes at once, gap or not: nothing is stepped after it.
/// While suspended (link in trouble), nothing is published and nothing is queued; the
/// first tick after resuming publishes a fresh view, unless it is the one last published (the
/// receiver already has it).
#[derive(Debug, Clone)]
pub struct BoardViewPublisher {
    last: Option<OpponentBoardView>,
    ticks_since_publish: i32,
    event_pending: bool,
    suspended: bool,
    resync: bool,
}

impl Default for BoardViewPublisher {
    fn default() -> Self {
        Self::new()
    }
}

impl BoardViewPublisher {
    pub fn new() -> Self {
        Self { last: None, ticks_since_publish: 0, event_pending: false, suspended: false, resync: false }
    }

    /// A new round: forgets the past and returns the view of the starting board. A suspension
    /// carries over; the view is then due again at the first tick after resuming.
    pub fn start(&mut self, game: &GameState) -> OpponentBoardView {
        let view = OpponentBoardView::from_game(game);
        let suspended = self.suspended;
        *self = Self { last: Some(view.clone()), suspended, resync: suspended, ..Self::new() };
        view
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Suspends or resumes publication.
    pub fn set_suspended(&mut self, suspended: bool) {
        if self.suspended && !suspended {
            self.resync = true;
        }
        self.suspended = suspended;
        if suspended {
            self.event_pending = false;
        }
    }

    /// One played tick, after the game stepped. Returns the view to publish, if any.
    pub fn step(&mut self, game: &GameState) -> Option<OpponentBoardView> {
        if self.suspended {
            return None;
        }
        self.ticks_since_publish += 1;
        self.event_pending |= game.gameplay_events.iter().any(is_board_event);
        // The game stops stepping after its end: the final picture cannot wait for the gap.
        let game_over = game.gameplay_events.iter().any(|e| matches!(e, GameplayEvent::GameOver { .. }));
        let due = self.resync
            || game_over
            || (self.event_pending && self.ticks_since_publish >= BOARD_VIEW_MIN_GAP)
            || self.ticks_since_publish >= BOARD_VIEW_PERIOD;
        if !due {
            return None;
        }
        self.ticks_since_publish = 0;
        self.event_pending = false;
        self.resync = false;
        let view = OpponentBoardView::from_game(game);
        if self.last.as_ref() == Some(&view) {
            return None;
        }
        self.last = Some(view.clone());
        Some(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay_event::GarbageLandedEvent;
    use crate::grid::CellRef;
    use crate::score::ScoringMode;

    fn empty_game() -> GameState {
        let mut game = GameState::with_scoring(1, ScoringMode::Classic);
        for x in 0..PLAY_WIDTH as i32 {
            for y in 0..crate::constants::PLAY_HEIGHT as i32 {
                game.grid.set(x, y, CellRef::empty());
            }
        }
        game
    }

    fn put(game: &mut GameState, x: i32, y: i32, cell_type: CellType, id: i16) {
        game.grid.set(x, y, CellRef { id, cell_type, phase: 0 });
    }

    fn landed() -> GameplayEvent {
        GameplayEvent::GarbageLanded(GarbageLandedEvent { grid_x: 0, grid_y: 1, width: 6, height: 1, initial: true })
    }

    #[test]
    fn bytes_round_trip_and_reject_impossible_values() {
        let mut game = empty_game();
        put(&mut game, 0, 1, CellType::Block, 0);
        put(&mut game, 1, 1, CellType::Garbage, (GARBAGE_STORE_SIZE - 1) as i16);
        let view = OpponentBoardView::from_game(&game);
        assert_eq!(OpponentBoardView::from_bytes(&view.to_bytes()), Some(view.clone()));

        let mut bytes = view.to_bytes();
        bytes[5] = CELL_MAX + 1;
        assert_eq!(OpponentBoardView::from_bytes(&bytes), None);
        assert_eq!(OpponentBoardView::from_bytes(&view.to_bytes()[1..]), None);
    }

    #[test]
    fn empty_grid_gives_empty_view() {
        let view = OpponentBoardView::from_game(&empty_game());
        assert_eq!(view, OpponentBoardView::empty());
    }

    #[test]
    fn rising_row_is_not_shown_and_row_one_is_the_bottom() {
        let mut game = empty_game();
        put(&mut game, 2, 0, CellType::Block, 0);
        put(&mut game, 3, 1, CellType::Block, 1);
        let view = OpponentBoardView::from_game(&game);
        assert_eq!(view.cell(2, 0), BoardCell::Empty);
        assert_eq!(view.cell(3, 0), BoardCell::Block);
    }

    #[test]
    fn garbage_above_the_top_row_is_cut_off() {
        let mut game = empty_game();
        for y in 10..16 {
            put(&mut game, 0, y, CellType::Garbage, 4);
        }
        let view = OpponentBoardView::from_game(&game);
        assert_eq!(view.cell(0, 9), BoardCell::Garbage(4));
        assert_eq!(view.cell(0, BOARD_VIEW_ROWS - 1), BoardCell::Garbage(4));
        let shown = (0..BOARD_VIEW_ROWS).filter(|&r| view.cell(0, r) != BoardCell::Empty).count();
        assert_eq!(shown, BOARD_VIEW_ROWS - 9);
    }

    #[test]
    fn slabs_keep_distinct_identifiers() {
        let mut game = empty_game();
        put(&mut game, 0, 1, CellType::Garbage, 0);
        put(&mut game, 0, 2, CellType::Garbage, 7);
        let view = OpponentBoardView::from_game(&game);
        assert_eq!(view.cell(0, 0), BoardCell::Garbage(0));
        assert_eq!(view.cell(0, 1), BoardCell::Garbage(7));
        assert!(view.has_garbage(7));
        assert!(!view.has_garbage(3));
    }

    /// Steps the publisher `ticks` times, changing one cell before each tick so a due view always
    /// differs from the last, with a board event on the ticks `events_at` selects. Returns the
    /// ticks (1-based) at which a view went out.
    fn run(
        publisher: &mut BoardViewPublisher,
        game: &mut GameState,
        ticks: i32,
        mut events_at: impl FnMut(i32) -> bool,
    ) -> Vec<i32> {
        let mut published = Vec::new();
        for t in 1..=ticks {
            // A different board every tick: the dedup never hides a due view.
            let x = t % PLAY_WIDTH as i32;
            let row = 1 + (t / PLAY_WIDTH as i32) % BOARD_VIEW_ROWS as i32;
            let cur = game.grid.get(x, row);
            let next = if cur.cell_type == CellType::Block { CellType::Empty } else { CellType::Block };
            put(game, x, row, next, if next == CellType::Block { 0 } else { -1 });
            game.gameplay_events.clear();
            if events_at(t) {
                game.gameplay_events.push(landed());
            }
            if publisher.step(game).is_some() {
                published.push(t);
            }
        }
        published
    }

    #[test]
    fn without_events_a_view_every_period() {
        let mut game = empty_game();
        let mut publisher = BoardViewPublisher::new();
        publisher.start(&game);
        let published = run(&mut publisher, &mut game, 100, |_| false);
        assert_eq!(published, vec![32, 64, 96]);
    }

    #[test]
    fn an_event_publishes_and_restarts_the_period() {
        let mut game = empty_game();
        let mut publisher = BoardViewPublisher::new();
        publisher.start(&game);
        // Event at tick 20: out at once (gap satisfied), then the next regular view at 52.
        let published = run(&mut publisher, &mut game, 60, |t| t == 20);
        assert_eq!(published, vec![20, 52]);
    }

    #[test]
    fn an_event_inside_the_gap_waits_for_its_end() {
        let mut game = empty_game();
        let mut publisher = BoardViewPublisher::new();
        publisher.start(&game);
        let published = run(&mut publisher, &mut game, 20, |t| t == 3);
        assert_eq!(published, vec![BOARD_VIEW_MIN_GAP]);
    }

    #[test]
    fn a_burst_never_exceeds_one_view_per_gap() {
        let mut game = empty_game();
        let mut publisher = BoardViewPublisher::new();
        publisher.start(&game);
        let published = run(&mut publisher, &mut game, 64, |_| true);
        assert_eq!(published.len(), 64 / BOARD_VIEW_MIN_GAP as usize);
        assert!(published.windows(2).all(|w| w[1] - w[0] >= BOARD_VIEW_MIN_GAP));
    }

    #[test]
    fn an_unchanged_view_is_not_published() {
        let game = empty_game();
        let mut publisher = BoardViewPublisher::new();
        publisher.start(&game);
        assert!((0..100).all(|_| publisher.step(&game).is_none()));
    }

    #[test]
    fn suspended_publishes_nothing_then_resyncs_at_once() {
        let mut game = empty_game();
        let mut publisher = BoardViewPublisher::new();
        publisher.start(&game);
        publisher.set_suspended(true);
        assert!(run(&mut publisher, &mut game, 100, |_| true).is_empty());
        publisher.set_suspended(false);
        let published = run(&mut publisher, &mut game, 40, |_| false);
        assert_eq!(published, vec![1, 33]);
    }
}
