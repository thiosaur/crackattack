#![allow(clippy::needless_range_loop)] // Indexed access is intentional for grid traversal

use super::snapshot::*;
use super::danger::*;

pub(crate) trait AiHeuristic {
    fn evaluate(
        &self,
        grid: &GridSnapshot,
        base: &SwapGrid,
        danger: &DangerContext,
        danger_priority: &DangerPriorityGrid,
    ) -> SwapGrid;
}

pub(crate) struct ModuleOnePlyMatch;

impl ModuleOnePlyMatch {
    fn is_swappable_block(g: &GridSnapshot, x: i32, y: i32) -> bool {
        if x < 0 || x >= GRID_WIDTH as i32 || y < 0 || y >= GRID_HEIGHT as i32 {
            return false;
        }
        let c = g.cell_at(x, y);
        (c.flags & CF_BLOCK) != 0
            && (c.flags & CF_UNSWAPPABLE) == 0
            && g.get_flavor_at(x, y) >= 0
    }

    fn is_garbage_cell(g: &GridSnapshot, x: i32, y: i32) -> bool {
        if x < 0 || x >= GRID_WIDTH as i32 || y < 0 || y >= GRID_HEIGHT as i32 {
            return false;
        }
        (g.cell_at(x, y).flags & CF_GARBAGE) != 0
    }

    fn is_empty_cell(g: &GridSnapshot, x: i32, y: i32) -> bool {
        if x < 0 || x >= GRID_WIDTH as i32 || y < 0 || y >= GRID_HEIGHT as i32 {
            return false;
        }
        g.cell_at(x, y).flags == CF_EMPTY
    }

    fn drop_y<F>(x: i32, start_y: i32, virt_empty: &F) -> i32
    where
        F: Fn(i32, i32) -> bool,
    {
        let mut y = start_y;
        while y > 1 && virt_empty(x, y - 1) {
            y -= 1;
        }
        y
    }

    fn count_h<HB, FL>(has_b: &HB, flv: &FL, x: i32, y: i32) -> i32
    where
        HB: Fn(i32, i32) -> bool,
        FL: Fn(i32, i32) -> i32,
    {
        if !has_b(x, y) {
            return 0;
        }
        let f = flv(x, y);
        let mut c = 1;
        let mut i = x - 1;
        while i >= 0 {
            if has_b(i, y) && flv(i, y) == f {
                c += 1;
            } else {
                break;
            }
            i -= 1;
        }
        i = x + 1;
        while i < GRID_WIDTH as i32 {
            if has_b(i, y) && flv(i, y) == f {
                c += 1;
            } else {
                break;
            }
            i += 1;
        }
        c
    }

    fn count_v<HB, FL>(has_b: &HB, flv: &FL, x: i32, y: i32) -> i32
    where
        HB: Fn(i32, i32) -> bool,
        FL: Fn(i32, i32) -> i32,
    {
        if !has_b(x, y) {
            return 0;
        }
        let f = flv(x, y);
        let mut c = 1;
        let mut j = y - 1;
        while j >= 1 {
            if has_b(x, j) && flv(x, j) == f {
                c += 1;
            } else {
                break;
            }
            j -= 1;
        }
        j = y + 1;
        while j < GRID_HEIGHT as i32 {
            if has_b(x, j) && flv(x, j) == f {
                c += 1;
            } else {
                break;
            }
            j += 1;
        }
        c
    }

    fn count_matches_after_swap_and_drop(grid: &GridSnapshot, sx: i32, sy: i32) -> i32 {
        let y0 = sy + 1;
        if sx < 0 || sx + 1 >= GRID_WIDTH as i32 || y0 < 1 || y0 >= GRID_HEIGHT as i32 {
            return 0;
        }

        let lb = Self::is_swappable_block(grid, sx, y0);
        let rb = Self::is_swappable_block(grid, sx + 1, y0);
        let lf = if lb { grid.get_flavor_at(sx, y0) } else { -1 };
        let rf = if rb { grid.get_flavor_at(sx + 1, y0) } else { -1 };

        if !lb && !rb {
            return 0;
        }

        // Virtual view after swap
        let virt_has_block = |x: i32, y: i32| -> bool {
            if y == y0 {
                if x == sx { return rb; }
                if x == sx + 1 { return lb; }
            }
            Self::is_swappable_block(grid, x, y)
        };
        let virt_flavor = |x: i32, y: i32| -> i32 {
            if y == y0 {
                if x == sx { return rf; }
                if x == sx + 1 { return lf; }
            }
            grid.get_flavor_at(x, y)
        };
        let virt_empty = |x: i32, y: i32| -> bool {
            !virt_has_block(x, y) && Self::is_empty_cell(grid, x, y)
        };

        // Compute final positions after drop
        let (ax, mut ay) = (sx, y0);
        let (bx, mut by) = (sx + 1, y0);

        let a_exists = virt_has_block(sx, y0);
        let b_exists = virt_has_block(sx + 1, y0);

        if a_exists && y0 > 1 && virt_empty(sx, y0 - 1) {
            ay = Self::drop_y(sx, y0, &virt_empty);
        }
        if b_exists && y0 > 1 && virt_empty(sx + 1, y0 - 1) {
            by = Self::drop_y(sx + 1, y0, &virt_empty);
        }

        // Virtual view after swap + drop
        let has_b2 = |x: i32, y: i32| -> bool {
            if a_exists && x == sx && y == y0 { return false; }
            if b_exists && x == sx + 1 && y == y0 { return false; }
            if a_exists && x == ax && y == ay { return true; }
            if b_exists && x == bx && y == by { return true; }
            if y == y0 && (x == sx || x == sx + 1) { return false; }
            Self::is_swappable_block(grid, x, y)
        };
        let flv2 = |x: i32, y: i32| -> i32 {
            if a_exists && x == ax && y == ay { return virt_flavor(sx, y0); }
            if b_exists && x == bx && y == by { return virt_flavor(sx + 1, y0); }
            grid.get_flavor_at(x, y)
        };

        let mut matches = 0;

        if a_exists {
            let h = Self::count_h(&has_b2, &flv2, ax, ay);
            let v = Self::count_v(&has_b2, &flv2, ax, ay);
            if h >= 3 { matches += h; }
            if v >= 3 { matches += v; }
        }
        if b_exists {
            let h = Self::count_h(&has_b2, &flv2, bx, by);
            let v = Self::count_v(&has_b2, &flv2, bx, by);
            if h >= 3 { matches += h; }
            if v >= 3 { matches += v; }
        }

        matches
    }

    fn touches_garbage_after_swap_and_drop(grid: &GridSnapshot, sx: i32, sy: i32) -> bool {
        let y0 = sy + 1;
        if sx < 0 || sx + 1 >= GRID_WIDTH as i32 || y0 < 1 || y0 >= GRID_HEIGHT as i32 {
            return false;
        }

        let lb = Self::is_swappable_block(grid, sx, y0);
        let rb = Self::is_swappable_block(grid, sx + 1, y0);
        if !lb && !rb {
            return false;
        }

        let virt_has_block = |x: i32, y: i32| -> bool {
            if y == y0 {
                if x == sx { return rb; }
                if x == sx + 1 { return lb; }
            }
            Self::is_swappable_block(grid, x, y)
        };
        let virt_empty = |x: i32, y: i32| -> bool {
            !virt_has_block(x, y) && Self::is_empty_cell(grid, x, y)
        };

        let mut ay = y0;
        let mut by = y0;
        let a_exists = virt_has_block(sx, y0);
        let b_exists = virt_has_block(sx + 1, y0);

        if a_exists && y0 > 1 && virt_empty(sx, y0 - 1) {
            ay = Self::drop_y(sx, y0, &virt_empty);
        }
        if b_exists && y0 > 1 && virt_empty(sx + 1, y0 - 1) {
            by = Self::drop_y(sx + 1, y0, &virt_empty);
        }

        let check_around = |cx: i32, cy: i32| -> bool {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 { continue; }
                    let nx = cx + dx;
                    let ny = cy + dy;
                    if nx < 0 || nx >= GRID_WIDTH as i32 || ny < 0 || ny >= GRID_HEIGHT as i32 {
                        continue;
                    }
                    if Self::is_garbage_cell(grid, nx, ny) {
                        return true;
                    }
                }
            }
            false
        };

        if a_exists && check_around(sx, ay) { return true; }
        if b_exists && check_around(sx + 1, by) { return true; }

        false
    }
}

impl AiHeuristic for ModuleOnePlyMatch {
    fn evaluate(
        &self,
        grid: &GridSnapshot,
        base: &SwapGrid,
        _danger: &DangerContext,
        _danger_priority: &DangerPriorityGrid,
    ) -> SwapGrid {
        let mut result = SwapGrid::default();

        for x in 0..SWAP_WIDTH as i32 {
            for y in 0..SWAP_HEIGHT as i32 {
                if base.cell_at(x, y) == swap_base::IMPOSSIBLE {
                    continue;
                }
                let match_count = Self::count_matches_after_swap_and_drop(grid, x, y);
                if match_count >= 3 {
                    let score;
                    if Self::touches_garbage_after_swap_and_drop(grid, x, y) {
                        score = swap_priority::CRITICAL;
                    } else if match_count >= 6 {
                        score = swap_priority::COMBO;
                    } else if match_count >= 4 {
                        score = swap_priority::MATCH_4;
                    } else {
                        score = swap_priority::MATCH;
                    }
                    result.set(x, y, score);
                }
            }
        }

        result
    }
}

pub(crate) struct ModuleDangerDrop;

impl ModuleDangerDrop {
    fn simulate_drop_gain(grid: &GridSnapshot, sx: i32, sy: i32) -> i32 {
        let grid_y = sy + 1;
        let mut left_floor = 0;
        let mut right_floor = 0;

        for j in (0..grid_y).rev() {
            if grid.has_block(sx, j) || (grid.cell_at(sx, j).flags & CF_GARBAGE) != 0 {
                left_floor = j + 1;
                break;
            }
        }

        for j in (0..grid_y).rev() {
            if grid.has_block(sx + 1, j) || (grid.cell_at(sx + 1, j).flags & CF_GARBAGE) != 0 {
                right_floor = j + 1;
                break;
            }
        }

        let flavor_l = grid.get_flavor_at(sx, grid_y);
        let flavor_r = grid.get_flavor_at(sx + 1, grid_y);

        let mut gain = 0;
        // Left block moves to right column
        if flavor_l >= 0 && right_floor < grid_y {
            gain += grid_y - right_floor;
        }
        // Right block moves to left column
        if flavor_r >= 0 && left_floor < grid_y {
            gain += grid_y - left_floor;
        }

        gain
    }
}

impl AiHeuristic for ModuleDangerDrop {
    fn evaluate(
        &self,
        grid: &GridSnapshot,
        base: &SwapGrid,
        danger: &DangerContext,
        _danger_priority: &DangerPriorityGrid,
    ) -> SwapGrid {
        let mut result = SwapGrid::default();

        if danger.urgency_level() == 0 {
            return result;
        }

        for x in 0..SWAP_WIDTH as i32 {
            for y in 0..SWAP_HEIGHT as i32 {
                if base.cell_at(x, y) == swap_base::IMPOSSIBLE {
                    continue;
                }
                let drop_gain = Self::simulate_drop_gain(grid, x, y);
                if drop_gain > 0 {
                    let score = if drop_gain >= 4 {
                        swap_priority::SETUP_HIGH
                    } else if drop_gain >= 2 {
                        swap_priority::SETUP
                    } else {
                        swap_priority::SETUP_LOW
                    };
                    result.set(x, y, score);
                }
            }
        }

        result
    }
}

pub(crate) struct ModulePyramid;

impl ModulePyramid {
    fn longest_segment_row(grid: &GridSnapshot, y: i32, lo: i32, hi: i32) -> (i32, i32) {
        let lo = lo.clamp(0, GRID_WIDTH as i32 - 1);
        let hi = hi.clamp(0, GRID_WIDTH as i32 - 1);
        if lo > hi {
            return (0, -1);
        }

        let mut best_l = 0;
        let mut best_r: i32 = -1;
        let mut best_len = 0;
        let mut cur_l: i32 = -1;

        for x in lo..=hi {
            if grid.has_block(x, y) {
                if cur_l < 0 {
                    cur_l = x;
                }
            } else {
                if cur_l >= 0 {
                    let cur_r = x - 1;
                    let len = cur_r - cur_l + 1;
                    if len > best_len {
                        best_len = len;
                        best_l = cur_l;
                        best_r = cur_r;
                    }
                    cur_l = -1;
                }
            }
        }
        if cur_l >= 0 {
            let cur_r = hi;
            let len = cur_r - cur_l + 1;
            if len > best_len {
                best_l = cur_l;
                best_r = cur_r;
            }
        }

        (best_l, best_r)
    }

    fn collect_color_xs(grid: &GridSnapshot, y: i32, seg_left: i32, seg_right: i32, color: i32) -> Vec<i32> {
        let mut xs = Vec::new();
        if seg_right < seg_left {
            return xs;
        }
        for x in seg_left..=seg_right {
            if grid.has_block(x, y) && grid.get_flavor_at(x, y) == color {
                xs.push(x);
            }
        }
        xs
    }

    fn pick_closest_x(xs: &[i32], target_x: i32) -> i32 {
        let mut best_x = xs[0];
        let mut best_d = (best_x - target_x).abs();
        for &x in xs {
            let d = (x - target_x).abs();
            if d < best_d {
                best_d = d;
                best_x = x;
            }
        }
        best_x
    }
}

impl AiHeuristic for ModulePyramid {
    fn evaluate(
        &self,
        grid: &GridSnapshot,
        base: &SwapGrid,
        danger: &DangerContext,
        _danger_priority: &DangerPriorityGrid,
    ) -> SwapGrid {
        if danger.urgency_level() >= 3 {
            return SwapGrid::default();
        }

        let mut result = SwapGrid::default();

        // Build pyramid segments from bottom up
        let mut pyr: Vec<(i32, i32)> = Vec::with_capacity(SWAP_HEIGHT);

        let seg = Self::longest_segment_row(grid, 1, 0, GRID_WIDTH as i32 - 1);
        if seg.1 < seg.0 {
            return result;
        }
        pyr.push(seg);

        let mut prev_seg = seg;
        for row in 2..=SWAP_HEIGHT as i32 {
            let next = Self::longest_segment_row(grid, row, prev_seg.0, prev_seg.1);
            if next.1 < next.0 {
                break;
            }
            pyr.push(next);
            prev_seg = next;
        }

        if pyr.len() < 3 {
            return result;
        }

        const MAXC: i32 = 8;

        for i in 0..pyr.len().saturating_sub(2) {
            let y0 = (i + 1) as i32;
            let y1 = (i + 2) as i32;
            let y2 = (i + 3) as i32;

            let s0 = pyr[i];
            let s1 = pyr[i + 1];
            let s2 = pyr[i + 2];

            let center_x = (pyr[0].0 + pyr[0].1) / 2;

            for c in 0..MAXC {
                let xs0 = Self::collect_color_xs(grid, y0, s0.0, s0.1, c);
                let xs1 = Self::collect_color_xs(grid, y1, s1.0, s1.1, c);
                let xs2 = Self::collect_color_xs(grid, y2, s2.0, s2.1, c);

                if xs0.is_empty() || xs1.is_empty() || xs2.is_empty() {
                    continue;
                }

                let mut x0 = Self::pick_closest_x(&xs0, center_x);
                let mut x1 = Self::pick_closest_x(&xs1, center_x);
                let mut x2 = Self::pick_closest_x(&xs2, center_x);

                let mut avg_x = (x0 + x1 + x2) / 3;

                x0 = Self::pick_closest_x(&xs0, avg_x);
                x1 = Self::pick_closest_x(&xs1, avg_x);
                x2 = Self::pick_closest_x(&xs2, avg_x);
                avg_x = (x0 + x1 + x2) / 3;

                let d0 = (x0 - avg_x).abs();
                let d1 = (x1 - avg_x).abs();
                let d2 = (x2 - avg_x).abs();

                // Choose outlier: furthest from average
                let mut out_x = x0;
                let mut out_y = y0;
                let mut out_d = d0;

                if d1 > out_d || (d1 == out_d && x1 < out_x) {
                    out_d = d1;
                    out_x = x1;
                    out_y = y1;
                }
                if d2 > out_d || (d2 == out_d && x2 < out_x) {
                    out_x = x2;
                    out_y = y2;
                }

                if out_d == 0 {
                    continue;
                }

                let sy = out_y - 1;

                if out_x < avg_x {
                    // move right
                    let sx = out_x;
                    if sx >= 0 && sx < SWAP_WIDTH as i32 && base.cell_at(sx, sy) != swap_base::IMPOSSIBLE {
                        result.set(sx, sy, 8);
                    }
                } else {
                    // move left
                    let sx = out_x - 1;
                    if sx >= 0 && sx < SWAP_WIDTH as i32 && base.cell_at(sx, sy) != swap_base::IMPOSSIBLE {
                        result.set(sx, sy, 8);
                    }
                }
            }
        }

        result
    }
}

pub(crate) struct ModuleDepth2Chain;

// SimBoard for Depth2Chain
#[derive(Clone)]
struct D2SimBoard {
    cell: [[i8; GRID_HEIGHT]; GRID_WIDTH],
}

const D2_EMPTY: i8 = -1;

impl D2SimBoard {
    fn load_from(grid: &GridSnapshot) -> Self {
        let mut board = D2SimBoard {
            cell: [[D2_EMPTY; GRID_HEIGHT]; GRID_WIDTH],
        };
        for y in 1..GRID_HEIGHT {
            for x in 0..GRID_WIDTH {
                if grid.has_block(x as i32, y as i32) {
                    board.cell[x][y] = grid.get_flavor_at(x as i32, y as i32) as i8;
                }
            }
        }
        board
    }

    fn in_b(x: i32, y: i32) -> bool {
        x >= 0 && x < GRID_WIDTH as i32 && y >= 1 && y < GRID_HEIGHT as i32
    }

    fn apply_swap(&mut self, sx: i32, sy: i32) {
        let y = sy + 1;
        if !Self::in_b(sx, y) || !Self::in_b(sx + 1, y) { return; }
        let tmp = self.cell[sx as usize][y as usize];
        self.cell[sx as usize][y as usize] = self.cell[(sx + 1) as usize][y as usize];
        self.cell[(sx + 1) as usize][y as usize] = tmp;
    }

    fn max_height(&self) -> i32 {
        let mut mh = 0;
        for x in 0..GRID_WIDTH {
            for y in (1..GRID_HEIGHT).rev() {
                if self.cell[x][y] != D2_EMPTY {
                    if y as i32 > mh { mh = y as i32; }
                    break;
                }
            }
        }
        mh
    }

    fn find_matches(&self, mark: &mut [[bool; GRID_HEIGHT]; GRID_WIDTH]) -> i32 {
        for x in 0..GRID_WIDTH {
            for y in 0..GRID_HEIGHT {
                mark[x][y] = false;
            }
        }

        // Horizontal
        for y in 1..GRID_HEIGHT {
            let mut run = 0i32;
            let mut val: i8 = D2_EMPTY;
            for x in 0..GRID_WIDTH {
                let v = self.cell[x][y];
                if v != D2_EMPTY && v == val {
                    run += 1;
                } else {
                    val = v;
                    run = if v == D2_EMPTY { 0 } else { 1 };
                }
                if run >= 3 {
                    mark[x][y] = true;
                    mark[x - 1][y] = true;
                    mark[x - 2][y] = true;
                }
            }
        }

        // Vertical
        for x in 0..GRID_WIDTH {
            let mut run = 0i32;
            let mut val: i8 = D2_EMPTY;
            for y in 1..GRID_HEIGHT {
                let v = self.cell[x][y];
                if v != D2_EMPTY && v == val {
                    run += 1;
                } else {
                    val = v;
                    run = if v == D2_EMPTY { 0 } else { 1 };
                }
                if run >= 3 {
                    mark[x][y] = true;
                    mark[x][y - 1] = true;
                    mark[x][y - 2] = true;
                }
            }
        }

        let mut total = 0;
        for y in 1..GRID_HEIGHT {
            for x in 0..GRID_WIDTH {
                if mark[x][y] {
                    total += 1;
                }
            }
        }
        total
    }

    fn remove_marked(&mut self, mark: &[[bool; GRID_HEIGHT]; GRID_WIDTH]) {
        for y in 1..GRID_HEIGHT {
            for x in 0..GRID_WIDTH {
                if mark[x][y] {
                    self.cell[x][y] = D2_EMPTY;
                }
            }
        }
    }

    fn gravity(&mut self) {
        for x in 0..GRID_WIDTH {
            let mut write_y = 1;
            for y in 1..GRID_HEIGHT {
                if self.cell[x][y] != D2_EMPTY {
                    if write_y != y {
                        self.cell[x][write_y] = self.cell[x][y];
                        self.cell[x][y] = D2_EMPTY;
                    }
                    write_y += 1;
                }
            }
        }
    }

    fn resolve(&mut self) -> (i32, i32) {
        let mut mark = [[false; GRID_HEIGHT]; GRID_WIDTH];
        let mut removed = 0;
        let mut waves = 0;

        loop {
            let m = self.find_matches(&mut mark);
            if m == 0 { break; }
            waves += 1;
            removed += m;
            self.remove_marked(&mark);
            self.gravity();
        }
        (removed, waves)
    }
}

// Scoring constants
const D2_SCORE_PER_CELL: i32 = 30;
const D2_BONUS_WAVES: i32 = 80;
const D2_BONUS_LOWER: i32 = 10;
const D2_PENALTY_RAISE: i32 = 8;
const D2_SECOND_MOVE_DISCOUNT_PCT: i32 = 70;
const D2_MIN_CHAIN_SCORE: i32 = 100;
const D2_BEAM_K: usize = 12;
const D2_SECOND_SWAPS_CAP: i32 = 40;

fn d2_score_delta(before: &D2SimBoard, after: &D2SimBoard, removed: i32, waves: i32) -> i32 {
    let mut s = removed * D2_SCORE_PER_CELL;
    if waves > 1 {
        s += (waves - 1) * D2_BONUS_WAVES;
    }
    let h0 = before.max_height();
    let h1 = after.max_height();
    if h1 < h0 {
        s += (h0 - h1) * D2_BONUS_LOWER;
    }
    if h1 > h0 {
        s -= (h1 - h0) * D2_PENALTY_RAISE;
    }
    s
}

struct D2Eval1 {
    sx: i32,
    sy: i32,
    score1: i32,
    removed1: i32,
    after1: D2SimBoard,
}

impl AiHeuristic for ModuleDepth2Chain {
    fn evaluate(
        &self,
        grid: &GridSnapshot,
        base: &SwapGrid,
        _danger: &DangerContext,
        danger_priority: &DangerPriorityGrid,
    ) -> SwapGrid {
        let mut out = SwapGrid::default();

        // Collect candidates
        let mut cands: Vec<(i32, i32)> = Vec::with_capacity(SWAP_WIDTH * SWAP_HEIGHT);
        for sy in 0..SWAP_HEIGHT as i32 {
            for sx in 0..SWAP_WIDTH as i32 {
                if base.cell_at(sx, sy) == swap_base::IMPOSSIBLE { continue; }
                cands.push((sx, sy));
            }
        }
        if cands.is_empty() {
            return out;
        }

        let g0 = D2SimBoard::load_from(grid);

        // 1) Evaluate first move for all candidates, keep top-K beam
        let mut beam: Vec<D2Eval1> = Vec::with_capacity(D2_BEAM_K);

        for &(sx, sy) in &cands {
            let y = sy + 1;
            let fl = grid.get_flavor_at(sx, y);
            let fr = grid.get_flavor_at(sx + 1, y);
            if fl >= 0 && fr >= 0 && fl == fr { continue; }

            let mut g1 = g0.clone();
            g1.apply_swap(sx, sy);
            let (removed1, waves1) = g1.resolve();

            let mut s1 = d2_score_delta(&g0, &g1, removed1, waves1);

            // Boost based on danger priority
            let dp = danger_priority.cell_at(sx, sy);
            if dp >= danger_priority_level::CRITICAL_ZONE {
                s1 += 120;
            } else if dp >= danger_priority_level::GARBAGE_ZONE {
                s1 += 80;
            } else if dp >= danger_priority_level::TALL_COLUMN {
                s1 += 50;
            } else if dp >= danger_priority_level::NEAR_CURSOR {
                s1 += 20;
            }

            // Lower grid = more useful
            s1 += (SWAP_HEIGHT as i32 - 1 - sy) * 6;

            beam.push(D2Eval1 { sx, sy, score1: s1, removed1, after1: g1 });
        }

        if beam.is_empty() {
            return out;
        }

        beam.sort_by(|a, b| {
            b.score1.cmp(&a.score1)
                .then(a.sy.cmp(&b.sy))
                .then(a.sx.cmp(&b.sx))
        });
        beam.truncate(D2_BEAM_K);

        // 2) For each beam node, find best 2nd swap
        for n1 in &beam {
            let mut best_s2 = -1_000_000i32;
            let mut checked = 0;

            for sy in 0..SWAP_HEIGHT as i32 {
                if checked >= D2_SECOND_SWAPS_CAP { break; }
                for sx in 0..SWAP_WIDTH as i32 {
                    if checked >= D2_SECOND_SWAPS_CAP { break; }
                    if base.cell_at(sx, sy) == swap_base::IMPOSSIBLE { continue; }

                    let y = sy + 1;
                    let a = n1.after1.cell[sx as usize][y as usize];
                    let b = n1.after1.cell[(sx + 1) as usize][y as usize];
                    if a != D2_EMPTY && b != D2_EMPTY && a == b { continue; }

                    let mut g2 = n1.after1.clone();
                    g2.apply_swap(sx, sy);
                    let (removed2, waves2) = g2.resolve();

                    let s2 = d2_score_delta(&n1.after1, &g2, removed2, waves2);
                    if s2 > best_s2 { best_s2 = s2; }

                    checked += 1;
                }
            }

            let total = n1.score1 + (best_s2 * D2_SECOND_MOVE_DISCOUNT_PCT) / 100;

            if total >= D2_MIN_CHAIN_SCORE {
                if n1.removed1 >= 3 {
                    out.set(n1.sx, n1.sy, swap_priority::CHAIN);
                } else {
                    out.set(n1.sx, n1.sy, swap_priority::SETUP_HIGH);
                }
            }
        }

        out
    }
}

pub(crate) struct ModuleGravityDropFallback;

const GF_EMPTY: i8 = -1;
const GF_GARB: i8 = -2;

const GF_SCORE_OLD_HAS_EQUAL: i32 = 40;
const GF_SCORE_NEW_HAS_EQUAL: i32 = 60;
const GF_SCORE_FALL_DISTANCE: i32 = 10;

#[derive(Clone, Copy)]
struct GfCellSim {
    v: i8,
    id: i16,
}

impl Default for GfCellSim {
    fn default() -> Self {
        GfCellSim { v: GF_EMPTY, id: -1 }
    }
}

#[derive(Clone)]
struct GfSimBoard {
    c: [[GfCellSim; GRID_HEIGHT]; GRID_WIDTH],
    next_id: i16,
}

impl GfSimBoard {
    fn load_from(grid: &GridSnapshot) -> Self {
        let mut board = GfSimBoard {
            c: [[GfCellSim::default(); GRID_HEIGHT]; GRID_WIDTH],
            next_id: 1,
        };

        for y in 0..GRID_HEIGHT {
            for x in 0..GRID_WIDTH {
                let cell = grid.cell_at(x as i32, y as i32);
                if (cell.flags & CF_GARBAGE) != 0 {
                    board.c[x][y].v = GF_GARB;
                    board.c[x][y].id = -1;
                } else if (cell.flags & CF_BLOCK) != 0 {
                    let f = grid.get_flavor_at(x as i32, y as i32);
                    if f >= 0 {
                        board.c[x][y].v = f as i8;
                        board.c[x][y].id = board.next_id;
                        board.next_id += 1;
                    }
                }
            }
        }

        board
    }

    fn in_b(x: i32, y: i32) -> bool {
        x >= 0 && x < GRID_WIDTH as i32 && y >= 1 && y < GRID_HEIGHT as i32
    }

    fn apply_swap(&mut self, sx: i32, sy: i32) {
        let y = sy + 1;
        if !Self::in_b(sx, y) || !Self::in_b(sx + 1, y) { return; }
        let tmp = self.c[sx as usize][y as usize];
        self.c[sx as usize][y as usize] = self.c[(sx + 1) as usize][y as usize];
        self.c[(sx + 1) as usize][y as usize] = tmp;
    }

    fn gravity(&mut self) {
        for x in 0..GRID_WIDTH {
            let mut y = 1;
            while y < GRID_HEIGHT {
                // Skip garbage
                while y < GRID_HEIGHT && self.c[x][y].v == GF_GARB { y += 1; }
                if y >= GRID_HEIGHT { break; }

                let seg_start = y;
                while y < GRID_HEIGHT && self.c[x][y].v != GF_GARB { y += 1; }
                let seg_end = y - 1;

                // Collect blocks
                let mut blocks: Vec<GfCellSim> = Vec::new();
                for yy in seg_start..=seg_end {
                    if self.c[x][yy].v >= 0 {
                        blocks.push(self.c[x][yy]);
                    }
                }

                // Clear segment
                for yy in seg_start..=seg_end {
                    self.c[x][yy] = GfCellSim::default();
                }

                // Write back packed
                let mut write_y = seg_start;
                for b in &blocks {
                    self.c[x][write_y] = *b;
                    write_y += 1;
                }
            }
        }
    }
}

fn gf_count_neighbors_left_right_down(b: &GfSimBoard, x: i32, y: i32) -> i32 {
    if !GfSimBoard::in_b(x, y) { return 0; }
    let v = b.c[x as usize][y as usize].v;
    if v < 0 { return 0; }

    let mut count = 0;
    if x > 0 && b.c[(x - 1) as usize][y as usize].v == v { count += 1; }
    if x + 1 < GRID_WIDTH as i32 && b.c[(x + 1) as usize][y as usize].v == v { count += 1; }
    if y > 1 && b.c[x as usize][(y - 1) as usize].v == v { count += 1; }
    count
}

fn gf_has_neighbor_left_right_down(b: &GfSimBoard, x: i32, y: i32) -> bool {
    gf_count_neighbors_left_right_down(b, x, y) > 0
}

fn gf_has_diagonal_drop(grid: &GridSnapshot, sx: i32, sy: i32) -> bool {
    let y = sy + 1;
    if y < 2 { return false; }

    let left_is_block = grid.has_block(sx, y);
    let right_is_block = grid.has_block(sx + 1, y);

    if left_is_block
        && grid.cell_at(sx + 1, y - 1).flags == CF_EMPTY {
            return true;
        }
    if right_is_block
        && grid.cell_at(sx, y - 1).flags == CF_EMPTY {
            return true;
        }

    false
}

struct GfSwapFallEval {
    causes_fall: bool,
    score: i32,
}

fn gf_eval_swap_fall(grid: &GridSnapshot, sx: i32, sy: i32) -> GfSwapFallEval {
    let y = sy + 1;

    let mut before = GfSimBoard::load_from(grid);
    before.apply_swap(sx, sy);

    let id_a = before.c[sx as usize][y as usize].id;
    let id_b = before.c[(sx + 1) as usize][y as usize].id;

    struct Tracked { id: i16, x0: i32, y0: i32, valid: bool }
    let tracked = [
        Tracked { id: id_a, x0: sx,     y0: y, valid: id_a > 0 },
        Tracked { id: id_b, x0: sx + 1, y0: y, valid: id_b > 0 },
    ];

    let mut after = before.clone();
    after.gravity();

    let find_id_pos = |id: i16| -> Option<(i32, i32)> {
        if id <= 0 { return None; }
        for yy in 1..GRID_HEIGHT as i32 {
            for xx in 0..GRID_WIDTH as i32 {
                if after.c[xx as usize][yy as usize].id == id {
                    return Some((xx, yy));
                }
            }
        }
        None
    };

    let mut best_score = -1_000_000i32;
    let mut any_fall = false;

    for t in &tracked {
        if !t.valid { continue; }

        let Some((x1, y1)) = find_id_pos(t.id) else { continue };

        let fall_dist = t.y0 - y1;
        if fall_dist < 1 { continue; }

        any_fall = true;

        let mut score = 0;
        if gf_has_neighbor_left_right_down(&after, t.x0, t.y0) {
            score += GF_SCORE_OLD_HAS_EQUAL;
        }
        if gf_has_neighbor_left_right_down(&after, x1, y1) {
            score += GF_SCORE_NEW_HAS_EQUAL;
        }
        score += fall_dist * GF_SCORE_FALL_DISTANCE;
        score += (GRID_HEIGHT as i32 - 1 - y1) * 2;

        if score > best_score {
            best_score = score;
        }
    }

    GfSwapFallEval {
        causes_fall: any_fall,
        score: if any_fall { best_score } else { -1_000_000 },
    }
}

impl AiHeuristic for ModuleGravityDropFallback {
    fn evaluate(
        &self,
        grid: &GridSnapshot,
        base: &SwapGrid,
        _danger: &DangerContext,
        _danger_priority: &DangerPriorityGrid,
    ) -> SwapGrid {
        let mut result = SwapGrid::default();

        let mut best_score = -1_000_000i32;
        let mut best_x: i32 = -1;
        let mut best_y: i32 = -1;

        for x in 0..SWAP_WIDTH as i32 {
            for y in 0..SWAP_HEIGHT as i32 {
                if base.cell_at(x, y) == swap_base::IMPOSSIBLE { continue; }
                if !gf_has_diagonal_drop(grid, x, y) { continue; }

                let e = gf_eval_swap_fall(grid, x, y);
                if !e.causes_fall { continue; }

                if e.score > best_score {
                    best_score = e.score;
                    best_x = x;
                    best_y = y;
                }
            }
        }

        if best_x >= 0 {
            result.set(best_x, best_y, swap_priority::SETUP);
        }

        result
    }
}
