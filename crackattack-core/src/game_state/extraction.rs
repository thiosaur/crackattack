// Frame data extraction - readonly snapshot of game state for rendering

use super::{GameState, GameStatus};
use crate::constants::*;
use crate::frame_data::{
    FrameData, BlockSnapshot, GarbageSnapshot, SwapperSnapshot,
    CellDebugInfo, UIState, GamePhase,
};

impl GameState {
    /// Read-only snapshot, used while the game is frozen (pause, countdown, network wait).
    ///
    /// It carries no transient events. `gameplay_events` and `gravity_impacts` are edges
    /// ("this happened at THIS step"), not levels. Only `step()` delivers them, through
    /// `extract_frame_with_events`. Serving them here would replay the last step's effects
    /// (for example looping sound effects) on every frozen frame.
    pub fn frame_data(&self) -> FrameData {
        self.extract_frame_readonly()
    }

    /// Snapshot of the current step, including the step's events. Reserved for `step()`.
    pub(crate) fn extract_frame_with_events(&self) -> FrameData {
        let mut frame = self.extract_frame_readonly();
        frame.gameplay_events = self.gameplay_events.clone();
        frame.gravity_impacts = self.gravity_impacts.clone();
        frame
    }

    fn extract_frame_readonly(&self) -> FrameData {
        let blocks: Vec<BlockSnapshot> = self.blocks.iter_active()
            .filter(|b| b.grid_y <= crate::constants::SAFE_HEIGHT as i32)
            .map(|b| {
                let combo_mult = b.combo_id
                    .and_then(|cid| self.combos.get_including_dead(crate::combo::ComboId(cid)))
                    .map(|c| c.multiplier)
                    .unwrap_or(1);
                BlockSnapshot::from_block_with_chain_level(b, combo_mult)
            })
            .collect();

        let time_step = self.time_step;
        let garbage: Vec<GarbageSnapshot> = self.garbage.iter_active()
            .map(|g| GarbageSnapshot::from_garbage(g, time_step))
            .collect();

        let debug_left_cell = self.extract_cell_debug(self.swapper.x, self.swapper.y);
        let debug_right_cell = self.extract_cell_debug(self.swapper.x + 1, self.swapper.y);

        let ui = self.extract_ui_state();

        FrameData {
            time_step: self.time_step,
            play_offset_y: self.creep.visual_offset(),
            swapper: SwapperSnapshot::from_swapper(&self.swapper),
            blocks,
            garbage,
            next_row: self.next_row,
            score: self.score.value(),
            game_over: self.status != GameStatus::Playing,
            stats: self.stats.clone(),
            ui,
            debug_left_cell,
            debug_right_cell,
            // Edge events: empty here, filled by `extract_frame_with_events`.
            gameplay_events: Vec::new(),
            gravity_impacts: Vec::new(),
        }
    }

    fn extract_ui_state(&self) -> UIState {
        // Determine game phase and message.
        // Pause and countdown are session concerns — GameState doesn't know about them.
        let phase = match self.status {
            GameStatus::Playing => GamePhase::Playing,
            GameStatus::Won | GameStatus::Lost => GamePhase::Finished,
        };

        // Calculate danger height based on highest occupied row
        // SAFE_HEIGHT is the threshold, anything above is danger
        let mut highest_y = 0;
        for block in self.blocks.iter_active() {
            if block.grid_y > highest_y {
                highest_y = block.grid_y;
            }
        }
        for garbage in self.garbage.iter_active() {
            let top = garbage.grid_y + garbage.height - 1;
            if top > highest_y {
                highest_y = top;
            }
        }

        // Danger starts at SAFE_HEIGHT - 2, critical at SAFE_HEIGHT
        let danger_threshold = SAFE_HEIGHT as i32 - 2;
        let danger_height = if highest_y <= danger_threshold {
            0.0
        } else {
            ((highest_y - danger_threshold) as f32 / 2.0).clamp(0.0, 1.0)
        };

        UIState {
            phase,
            // Displayed clock: starts at GO (see `clock_base`), never negative.
            clock_time: (self.time_step - self.clock_base).max(0),
            score: self.score.value(),
            ticks_since_finished: match self.finished_tick {
                Some(ft) => self.time_step - ft,
                None => 0,
            },
            level: highest_y + 1,  // Stack height (0-based y + 1)
            danger_height,
            creep_freeze: self.creep.creep_freeze,
            loss_countdown: self.creep.loss_countdown,
            top_effective_row: self.grid.top_effective_row,
            rows_to_loss: self.creep.rows_to_loss(self.grid.top_effective_row),
            seconds_per_row: self.creep.seconds_per_row(),
        }
    }

    fn extract_cell_debug(&self, x: i32, y: i32) -> CellDebugInfo {
        use crate::grid::CellType;

        let cell = self.grid.get(x, y);
        let cell_type = match cell.cell_type {
            CellType::Empty => 0,
            CellType::Block => 1,
            CellType::Garbage => 2,
        };

        // Get block state bits if it's a block
        let block_state = if let Some(id) = cell.block_id() {
            self.blocks.get(id).map(|b| b.physics_flags.bits()).unwrap_or(0)
        } else {
            0
        };

        CellDebugInfo {
            cell_type,
            phase: cell.phase,
            block_state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Edge/level invariant: extracting outside a step (`frame_data`, the frozen path)
    /// never re-serves the last step's events. Otherwise a pause would loop sound effects.
    #[test]
    fn frame_data_reextraction_serves_no_edge_events() {
        let mut gs = GameState::new(42);
        gs.gameplay_events.push(crate::gameplay_event::GameplayEvent::SwapPerformed);
        gs.gravity_impacts.push((2, 3));

        // The step path delivers the edge events...
        let stepped = gs.extract_frame_with_events();
        assert_eq!(stepped.gameplay_events.len(), 1);
        assert_eq!(stepped.gravity_impacts, vec![(2, 3)]);

        // ...the read-only path never does, however often it is called.
        for _ in 0..3 {
            let frozen = gs.frame_data();
            assert!(frozen.gameplay_events.is_empty());
            assert!(frozen.gravity_impacts.is_empty());
        }
    }
}
