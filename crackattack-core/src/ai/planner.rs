// AI Planner: coordinates heuristic modules to choose the best swap.

use super::snapshot::*;
use super::danger::*;
use super::heuristics::*;

pub(crate) const HISTORY_SIZE: usize = 5;

pub(crate) struct Planner {
    modules: Vec<Box<dyn AiHeuristic>>,
}

impl Planner {
    pub(crate) fn new() -> Self {
        let modules: Vec<Box<dyn AiHeuristic>> = vec![
            Box::new(ModuleOnePlyMatch),
            Box::new(ModuleDangerDrop),
            Box::new(ModulePyramid),
            Box::new(ModuleDepth2Chain),
            Box::new(ModuleGravityDropFallback),
        ];
        Planner { modules }
    }

    pub(crate) fn plan_move(
        &self,
        grid: &GridSnapshot,
        cursor_x: i32,
        cursor_y: i32,
        out_x: &mut i32,
        out_y: &mut i32,
        history_x: &[i32; HISTORY_SIZE],
        history_y: &[i32; HISTORY_SIZE],
    ) -> bool {
        let base_grid = build_swap_grid(grid);
        let danger_ctx = analyze_danger(grid);
        let danger_priority = build_danger_priority_grid(grid, &danger_ctx, cursor_x, cursor_y);

        let mut combined = SwapGrid::default();

        // Collect and merge module responses
        for module in &self.modules {
            let module_grid = module.evaluate(grid, &base_grid, &danger_ctx, &danger_priority);
            combined.merge(&module_grid);
        }

        // Apply danger priority bonus
        for x in 0..SWAP_WIDTH as i32 {
            for y in 0..SWAP_HEIGHT as i32 {
                let module_score = combined.cell_at(x, y);
                if module_score > 0 {
                    let danger_bonus = danger_priority.cell_at(x, y);
                    let c = (module_score as i32 + danger_bonus as i32).min(127);
                    combined.set(x, y, c as i8);
                }
            }
        }

        // Apply history malus
        const HISTORY_MALUS: [i32; HISTORY_SIZE] = [5, 4, 3, 2, 1];
        for i in 0..HISTORY_SIZE {
            let hx = history_x[i];
            let hy = history_y[i];
            if hx >= 0 && hx < SWAP_WIDTH as i32 && hy >= 0 && hy < SWAP_HEIGHT as i32 {
                let score = combined.cell_at(hx, hy) as i32;
                if score > 0 {
                    let new_score = (score - HISTORY_MALUS[i]).max(0);
                    combined.set(hx, hy, new_score as i8);
                }
            }
        }

        // Apply mask
        combined.apply_mask(&base_grid);

        // Find best position
        let grid_cursor_y = cursor_y - 1;

        if combined.find_best(cursor_x, grid_cursor_y, out_x, out_y) {
            return true;
        }

        // Fallback: any valid swap from base
        if base_grid.find_best(cursor_x, grid_cursor_y, out_x, out_y) {
            return true;
        }

        false
    }
}
