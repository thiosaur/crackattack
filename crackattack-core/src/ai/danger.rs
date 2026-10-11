// AI danger analysis: DangerContext, DangerPriorityGrid, and analysis functions.

use super::snapshot::*;

// DangerPriorityGrid levels

pub(crate) mod danger_priority_level {
    pub(crate) const NONE: i8          = 0;
    pub(crate) const NEAR_CURSOR: i8   = 2;
    pub(crate) const TALL_COLUMN: i8   = 5;
    pub(crate) const ISOLATED_COL: i8  = 8;
    pub(crate) const GARBAGE_ZONE: i8  = 10;
    pub(crate) const CRITICAL_ZONE: i8 = 15;
}

// DangerContext

pub(crate) const HEIGHT_WARNING: i32 = 9;
pub(crate) const HEIGHT_CRITICAL: i32 = 11;
pub(crate) const ISOLATED_MIN_HEIGHT: i32 = 3;

#[derive(Clone)]
pub(crate) struct DangerContext {
    pub(crate) screen_blocking: bool,
    pub(crate) max_column_height: i32,
    pub(crate) critical_columns: i32,
    pub(crate) has_garbage_bar: bool,
    pub(crate) garbage_rows: i32,
    pub(crate) lowest_garbage_y: i32,
    pub(crate) has_isolated_column: bool,
    pub(crate) isolated_column_count: i32,
    pub(crate) isolated_column_x: [i32; GRID_WIDTH],
    pub(crate) isolated_column_top: [i32; GRID_WIDTH],
}

impl Default for DangerContext {
    fn default() -> Self {
        DangerContext {
            screen_blocking: false,
            max_column_height: 0,
            critical_columns: 0,
            has_garbage_bar: false,
            garbage_rows: 0,
            lowest_garbage_y: GRID_HEIGHT as i32,
            has_isolated_column: false,
            isolated_column_count: 0,
            isolated_column_x: [-1; GRID_WIDTH],
            isolated_column_top: [0; GRID_WIDTH],
        }
    }
}

impl DangerContext {
    pub(crate) fn urgency_level(&self) -> i32 {
        if self.screen_blocking || self.max_column_height >= HEIGHT_CRITICAL {
            return 3;
        }
        if self.has_garbage_bar && self.lowest_garbage_y >= HEIGHT_WARNING {
            return 2;
        }
        if self.has_isolated_column || self.max_column_height >= HEIGHT_WARNING {
            return 1;
        }
        0
    }
}

// DangerPriorityGrid

#[derive(Clone)]
pub(crate) struct DangerPriorityGrid {
    pub(crate) cells: [[i8; SWAP_HEIGHT]; SWAP_WIDTH],
}

impl Default for DangerPriorityGrid {
    fn default() -> Self {
        DangerPriorityGrid {
            cells: [[0i8; SWAP_HEIGHT]; SWAP_WIDTH],
        }
    }
}

impl DangerPriorityGrid {
    pub(crate) fn cell_at(&self, x: i32, y: i32) -> i8 {
        if x < 0 || x >= SWAP_WIDTH as i32 || y < 0 || y >= SWAP_HEIGHT as i32 {
            return 0;
        }
        self.cells[x as usize][y as usize]
    }

    pub(crate) fn set(&mut self, x: i32, y: i32, value: i8) {
        if x >= 0 && x < SWAP_WIDTH as i32 && y >= 0 && y < SWAP_HEIGHT as i32 {
            self.cells[x as usize][y as usize] = value;
        }
    }
}

// DangerAnalyzer

fn analyze_column_heights(grid: &GridSnapshot, ctx: &mut DangerContext) {
    ctx.max_column_height = 0;
    ctx.critical_columns = 0;
    ctx.screen_blocking = false;

    for x in 0..GRID_WIDTH as i32 {
        let mut col_height = 0;
        for y in (1..GRID_HEIGHT as i32).rev() {
            let c = grid.cell_at(x, y);
            if (c.flags & CF_BLOCK) != 0 || (c.flags & CF_GARBAGE) != 0 {
                col_height = y;
                break;
            }
        }
        if col_height > ctx.max_column_height {
            ctx.max_column_height = col_height;
        }
        if col_height >= HEIGHT_CRITICAL {
            ctx.critical_columns += 1;
            ctx.screen_blocking = true;
        }
    }
}

fn analyze_garbage(grid: &GridSnapshot, ctx: &mut DangerContext) {
    ctx.has_garbage_bar = false;
    ctx.garbage_rows = 0;
    ctx.lowest_garbage_y = GRID_HEIGHT as i32;

    for y in 1..GRID_HEIGHT as i32 {
        let mut row_has_garbage = false;
        for x in 0..GRID_WIDTH as i32 {
            if (grid.cell_at(x, y).flags & CF_GARBAGE) != 0 {
                row_has_garbage = true;
                break;
            }
        }
        if row_has_garbage {
            ctx.has_garbage_bar = true;
            ctx.garbage_rows += 1;
            if y < ctx.lowest_garbage_y {
                ctx.lowest_garbage_y = y;
            }
        }
    }
}

fn analyze_isolated_columns(grid: &GridSnapshot, ctx: &mut DangerContext) {
    ctx.has_isolated_column = false;
    ctx.isolated_column_count = 0;

    for x in 0..GRID_WIDTH as i32 {
        let mut top_y = 0;
        for y in (1..GRID_HEIGHT as i32).rev() {
            if grid.has_block(x, y) {
                top_y = y;
                break;
            }
        }
        if top_y < ISOLATED_MIN_HEIGHT {
            continue;
        }

        let mut stack_height = 0;
        for y in (1..=top_y).rev() {
            if grid.has_block(x, y) {
                stack_height += 1;
            } else {
                break;
            }
        }
        if stack_height < ISOLATED_MIN_HEIGHT {
            continue;
        }

        let mut isolated = true;
        let check_from = top_y;
        let check_to = top_y - ISOLATED_MIN_HEIGHT + 1;
        for y in (check_to.max(1)..=check_from).rev() {
            if x > 0 && grid.has_block(x - 1, y) {
                isolated = false;
                break;
            }
            if x < GRID_WIDTH as i32 - 1 && grid.has_block(x + 1, y) {
                isolated = false;
                break;
            }
        }

        if isolated {
            ctx.has_isolated_column = true;
            let idx = ctx.isolated_column_count as usize;
            if idx < GRID_WIDTH {
                ctx.isolated_column_x[idx] = x;
                ctx.isolated_column_top[idx] = top_y;
            }
            ctx.isolated_column_count += 1;
        }
    }
}

pub(crate) fn analyze_danger(grid: &GridSnapshot) -> DangerContext {
    let mut ctx = DangerContext::default();
    analyze_column_heights(grid, &mut ctx);
    analyze_garbage(grid, &mut ctx);
    analyze_isolated_columns(grid, &mut ctx);
    ctx
}

pub(crate) fn build_danger_priority_grid(
    grid: &GridSnapshot,
    danger: &DangerContext,
    cursor_x: i32,
    cursor_y: i32,
) -> DangerPriorityGrid {
    let mut result = DangerPriorityGrid::default();
    let grid_cursor_y = cursor_y - 1;

    // Compute column heights
    let mut column_heights = [0i32; GRID_WIDTH];
    for x in 0..GRID_WIDTH as i32 {
        for y in (1..GRID_HEIGHT as i32).rev() {
            let c = grid.cell_at(x, y);
            if (c.flags & CF_BLOCK) != 0 || (c.flags & CF_GARBAGE) != 0 {
                column_heights[x as usize] = y;
                break;
            }
        }
    }

    for sx in 0..SWAP_WIDTH as i32 {
        for sy in 0..SWAP_HEIGHT as i32 {
            let mut priority: i8 = danger_priority_level::NONE;

            // 1) Near cursor bonus
            let dist = (sx - cursor_x).abs() + (sy - grid_cursor_y).abs();
            if dist <= 2 {
                priority = danger_priority_level::NEAR_CURSOR;
            }

            // 2) Critical zone
            if danger.screen_blocking {
                let left_height = column_heights[sx as usize];
                let right_height = column_heights[(sx + 1) as usize];
                let local_max = left_height.max(right_height);

                if local_max >= HEIGHT_CRITICAL {
                    let grid_y = sy + 1;
                    if grid_y >= local_max - 3 {
                        priority = danger_priority_level::CRITICAL_ZONE;
                    }
                }
            }

            // 3) Tall column zone
            if danger.max_column_height >= HEIGHT_WARNING {
                let left_height = column_heights[sx as usize];
                let right_height = column_heights[(sx + 1) as usize];
                let local_max = left_height.max(right_height);

                if local_max >= danger.max_column_height - 1 {
                    let grid_y = sy + 1;
                    if grid_y >= local_max - 2
                        && priority < danger_priority_level::TALL_COLUMN
                    {
                        priority = danger_priority_level::TALL_COLUMN;
                    }
                }
            }

            // 4) Near garbage zone
            if danger.has_garbage_bar {
                let grid_y = sy + 1;
                if grid_y >= danger.lowest_garbage_y - 2
                    && grid_y <= danger.lowest_garbage_y + 1
                    && priority < danger_priority_level::GARBAGE_ZONE
                {
                    priority = danger_priority_level::GARBAGE_ZONE;
                }
            }

            // 5) Near isolated columns
            for i in 0..danger.isolated_column_count.min(GRID_WIDTH as i32) {
                let iso_x = danger.isolated_column_x[i as usize];
                let iso_top = danger.isolated_column_top[i as usize];
                if (sx == iso_x || sx == iso_x - 1) && (sy + 1) >= iso_top - 2
                    && priority < danger_priority_level::ISOLATED_COL {
                        priority = danger_priority_level::ISOLATED_COL;
                    }
            }

            result.set(sx, sy, priority);
        }
    }

    result
}
