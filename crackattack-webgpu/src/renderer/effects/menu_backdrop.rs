//! Menu backdrop: big game blocks drifting down in the dark behind the menu.
//!
//! Each block has a depth: the far ones are smaller, darker and slower, which gives
//! the scene its volume. A block that leaves through the bottom comes back from the
//! top at a new place. The blocks are drawn by the block pipeline, with the
//! `Drifting` state (free rotation and scale).

use crate::renderer::gpu::instance::{BlockAnimState, BlockInstance};
use crate::renderer::scene::game_renderer::BLOCK_COLORS;

/// Number of drifting blocks.
const BLOCK_COUNT: usize = 8;
/// How far a block may stray from the middle of its lane, as a fraction of the lane width.
const LANE_JITTER: f32 = 0.3;
/// Distance to the camera of the nearest and farthest blocks (world units, the camera
/// looks down -Z). The play field sits at 42.5.
const NEAR_DISTANCE: f32 = 47.0;
const FAR_DISTANCE: f32 = 60.0;
/// Scale of the farthest and nearest blocks (1 = a block of the grid).
const FAR_SCALE: f32 = 1.4;
const NEAR_SCALE: f32 = 2.6;
/// Fall speed of the farthest and nearest blocks (world units per second).
const FAR_SPEED: f32 = 0.1;
const NEAR_SPEED: f32 = 0.28;
/// Largest rotation speed around each axis (radians per second).
const MAX_SPIN: f32 = 0.1;
/// Brightness of the farthest and nearest blocks: kept low, the backdrop must stay
/// in the background.
const FAR_BRIGHTNESS: f32 = 0.08;
const NEAR_BRIGHTNESS: f32 = 0.2;
/// Half-diagonal of a grid block (half-side 1): how far a tumbling block reaches from
/// its center, so it enters and leaves the view whole and without delay.
const BLOCK_REACH: f32 = 1.75;
/// Largest extra wait above the top before a block shows up again (world units).
const RESPAWN_SPREAD: f32 = 1.0;
/// Colored flavors only (no gray), in `BLOCK_COLORS` order.
const FLAVORS: [usize; 5] = [0, 1, 2, 3, 4];

/// Visible area at distance 1 from the camera, from the game frustum (near plane 38):
/// vertical bounds -6..18, horizontal center -3.6, half-height 12.
const VIEW_BOTTOM: f32 = -6.0 / 38.0;
const VIEW_TOP: f32 = 18.0 / 38.0;
const VIEW_CENTER_X: f32 = -3.6 / 38.0;
const VIEW_HALF_HEIGHT: f32 = 12.0 / 38.0;
/// Widest aspect the frustum shows (it widens up to 16:9).
const MAX_ASPECT: f32 = 16.0 / 9.0;

#[derive(Clone, Copy, Debug)]
struct Drifter {
    /// 0 = farthest, 1 = nearest.
    depth: f32,
    /// Vertical lane: the width is shared into one lane per block, so the blocks
    /// stay spaced apart.
    lane: usize,
    /// Horizontal place, as a fraction of the visible half-width (-1 = left edge,
    /// 1 = right edge): blocks follow the window when its shape changes.
    column: f32,
    y: f32,
    angle_x: f32,
    angle_y: f32,
    spin_x: f32,
    spin_y: f32,
    flavor: usize,
}

impl Drifter {
    fn distance(&self) -> f32 {
        FAR_DISTANCE + (NEAR_DISTANCE - FAR_DISTANCE) * self.depth
    }

    fn scale(&self) -> f32 {
        scale_at(self.depth)
    }
}

fn scale_at(depth: f32) -> f32 {
    FAR_SCALE + (NEAR_SCALE - FAR_SCALE) * depth
}

pub struct MenuBackdrop {
    blocks: Vec<Drifter>,
    rng: u32,
}

impl MenuBackdrop {
    pub fn new(seed: u32) -> Self {
        let mut backdrop = Self { blocks: Vec::with_capacity(BLOCK_COUNT), rng: seed.max(1) };
        for lane in 0..BLOCK_COUNT {
            // Neighbouring lanes start at distant heights (3 is prime with the count).
            let height = ((lane * 3) % BLOCK_COUNT) as f32 / BLOCK_COUNT as f32;
            let block = backdrop.spawn(lane, FLAVORS[lane % FLAVORS.len()], Some(height));
            backdrop.blocks.push(block);
        }
        backdrop
    }

    /// xorshift32: deterministic, enough for scattering blocks.
    fn next(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// A new block in `lane`, at a random depth. With `height` (fraction of the visible
    /// height, from the bottom) it starts inside the view (first frame); otherwise it
    /// starts just above the top.
    fn spawn(&mut self, lane: usize, flavor: usize, height: Option<f32>) -> Drifter {
        let depth = self.next();
        let d = FAR_DISTANCE + (NEAR_DISTANCE - FAR_DISTANCE) * depth;
        let lane_width = 2.0 / BLOCK_COUNT as f32;
        let column = -1.0 + lane_width * (lane as f32 + 0.5 + (self.next() * 2.0 - 1.0) * LANE_JITTER);
        let (bottom, top) = (VIEW_BOTTOM * d, VIEW_TOP * d);
        let y = match height {
            Some(h) => bottom + (h + self.next() / BLOCK_COUNT as f32) * (top - bottom),
            // Just out of sight above the top: it comes back as soon as it left.
            None => top + scale_at(depth) * BLOCK_REACH + self.next() * RESPAWN_SPREAD,
        };
        Drifter {
            depth,
            lane,
            column,
            y,
            angle_x: self.next() * std::f32::consts::TAU,
            angle_y: self.next() * std::f32::consts::TAU,
            spin_x: (self.next() * 2.0 - 1.0) * MAX_SPIN,
            spin_y: (self.next() * 2.0 - 1.0) * MAX_SPIN,
            flavor,
        }
    }

    /// Advances the drift by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        for i in 0..self.blocks.len() {
            let b = &mut self.blocks[i];
            b.y -= (FAR_SPEED + (NEAR_SPEED - FAR_SPEED) * b.depth) * dt;
            b.angle_x = (b.angle_x + b.spin_x * dt) % std::f32::consts::TAU;
            b.angle_y = (b.angle_y + b.spin_y * dt) % std::f32::consts::TAU;
            if b.y < VIEW_BOTTOM * b.distance() - b.scale() * BLOCK_REACH {
                let (lane, flavor) = (b.lane, b.flavor);
                self.blocks[i] = self.spawn(lane, flavor, None);
            }
        }
    }

    /// Block instances for the frame, far blocks first, spread over the width shown at
    /// `aspect` (width / height of the viewport).
    pub fn instances(&self, aspect: f32) -> Vec<BlockInstance> {
        let aspect = aspect.clamp(1.0, MAX_ASPECT);
        let mut out: Vec<BlockInstance> = self.blocks.iter().map(|b| {
            let d = b.distance();
            let x = VIEW_CENTER_X * d + b.column * VIEW_HALF_HEIGHT * aspect * d;
            let color = BLOCK_COLORS[b.flavor];
            let light = FAR_BRIGHTNESS + (NEAR_BRIGHTNESS - FAR_BRIGHTNESS) * b.depth;
            BlockInstance {
                position: [x, b.y, -d],
                state: BlockAnimState::Drifting.as_f32(),
                color: [color[0], color[1], color[2], 1.0],
                anim_progress: light,
                anim_axis_x: b.angle_x,
                anim_axis_y: b.angle_y,
                anim_pivot_offset: b.scale(),
                ..BlockInstance::default()
            }
        }).collect();
        out.sort_by(|a, b| a.position[2].total_cmp(&b.position[2]));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_start_inside_the_view() {
        let backdrop = MenuBackdrop::new(7);
        assert_eq!(backdrop.blocks.len(), BLOCK_COUNT);
        for b in &backdrop.blocks {
            let d = b.distance();
            assert!((NEAR_DISTANCE..=FAR_DISTANCE).contains(&d));
            assert!(b.y >= VIEW_BOTTOM * d && b.y <= VIEW_TOP * d);
        }
    }

    #[test]
    fn blocks_fall_and_come_back_from_the_top() {
        let mut backdrop = MenuBackdrop::new(7);
        let before = backdrop.blocks[0].y;
        backdrop.step(0.1);
        assert!(backdrop.blocks[0].y < before);
        // Long enough for every block to leave through the bottom at least once.
        for _ in 0..2000 {
            backdrop.step(0.05);
        }
        for b in &backdrop.blocks {
            let d = b.distance();
            let reach = b.scale() * BLOCK_REACH;
            assert!(b.y >= VIEW_BOTTOM * d - reach);
            assert!(b.y <= VIEW_TOP * d + reach + RESPAWN_SPREAD);
        }
    }

    #[test]
    fn far_blocks_are_drawn_first_smaller_and_darker() {
        let backdrop = MenuBackdrop::new(3);
        let inst = backdrop.instances(1.0);
        assert_eq!(inst.len(), BLOCK_COUNT);
        assert!(inst.windows(2).all(|w| w[0].position[2] <= w[1].position[2]));
        let (far, near) = (inst.first().unwrap(), inst.last().unwrap());
        assert!(far.anim_pivot_offset < near.anim_pivot_offset);
        assert!(inst.iter().all(|i| i.state == BlockAnimState::Drifting.as_f32()));
    }

    #[test]
    fn each_block_keeps_its_own_lane() {
        let mut backdrop = MenuBackdrop::new(9);
        for _ in 0..2000 {
            backdrop.step(0.5);
        }
        let mut columns: Vec<f32> = backdrop.blocks.iter().map(|b| b.column).collect();
        columns.sort_by(f32::total_cmp);
        let gap = 2.0 / BLOCK_COUNT as f32 * (1.0 - 2.0 * LANE_JITTER);
        assert!(columns.windows(2).all(|w| w[1] - w[0] >= gap - 1e-4));
    }

    #[test]
    fn a_wider_window_spreads_the_blocks() {
        let backdrop = MenuBackdrop::new(5);
        let width = |aspect: f32| {
            let inst = backdrop.instances(aspect);
            let xs = inst.iter().map(|i| i.position[0]);
            xs.clone().fold(f32::MIN, f32::max) - xs.fold(f32::MAX, f32::min)
        };
        assert!(width(16.0 / 9.0) > width(1.0) * 1.5);
    }

    #[test]
    fn the_same_seed_gives_the_same_backdrop() {
        let (a, b) = (MenuBackdrop::new(11).instances(1.0), MenuBackdrop::new(11).instances(1.0));
        assert!(a.iter().zip(&b).all(|(x, y)| x.position == y.position));
    }
}
