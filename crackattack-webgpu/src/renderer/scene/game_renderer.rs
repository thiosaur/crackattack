//! Game rendering helpers — converts `FrameData` into GPU-ready data.
//!
//! This module hosts free functions that transform a `FrameData` snapshot into
//! the values the WebGPU renderer needs (instances, uniforms, mesh helpers,
//! garbage/swapper/particle batching). There is no stateful renderer struct:
//! all functions are pure or write only through the `&wgpu::Queue` / buffers
//! the caller passes in.
//!
//! Entry point for the frame instances:
//! - `generate_frame_instances(&FrameData) -> Vec<BlockInstance>` — block
//!   instances plus next-row preview. Garbage goes through its own path
//!   (see `assemble_garbage_draws`). The swapper cursor is drawn as a UI
//!   silhouette by the renderer runtime, not here.

use crate::renderer::gpu::instance::{BlockInstance, BlockAnimState, GarbagePieceInstance};
use crate::renderer::gpu::mesh::BuiltinMesh;
use crackattack_core::block::BlockState;
use crackattack_core::frame_data::{FrameData, BlockSnapshot, GarbageSnapshot};
use crackattack_core::constants::{PLAY_WIDTH, STEPS_PER_GRID, DYING_DELAY, INTERNAL_POP_DELAY};

/// Duration in frames of the white flash at the start of a block's death animation.
///
/// Presentation constant. The original Crack Attack! 1.1.14 hard-codes this
/// window in its renderer.
const DYING_FLASH_TIME: i32 = 5;

/// Grid element length in world units.
pub const GRID_ELEMENT_LENGTH: f32 = 2.0;

/// Play area width in world units (PLAY_WIDTH * GRID_ELEMENT_LENGTH)
pub const PLAY_AREA_WIDTH: f32 = crackattack_core::constants::PLAY_WIDTH as f32 * GRID_ELEMENT_LENGTH;

/// Camera height ratio for the vertical play-area offset.
pub const CAMERA_HEIGHT_RATIO: f32 = 0.25;

/// Play area height in world units.
/// Note: PLAY_OFFSET_Y = -1 - PLAY_AREA_HEIGHT * CAMERA_HEIGHT_RATIO = -7.0
pub const PLAY_AREA_HEIGHT: f32 = 24.0;

/// Play area offset X (base, square window).
///
/// The play area center is at 0.8 instead of 1.0: it is shifted 0.2 wu to the
/// left so it does not overlap the level lights on the right.
pub const PLAY_OFFSET_X: f32 = 0.8 - PLAY_AREA_WIDTH / 2.0;

/// Horizontal reflow offset of the play area, as `f32` bits.
///
/// Set on resize by [`set_play_reflow_dx`]: `+E/6` when the window gets wider
/// (to equalize the three margins), 0 for square or portrait windows. It is
/// the single source for the whole gameplay group (blocks, garbage, cursor,
/// sparkles), so the value is not passed through every call.
static PLAY_REFLOW_DX_BITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Sets the play area reflow offset (world units). Called on resize.
pub fn set_play_reflow_dx(dx: f32) {
    PLAY_REFLOW_DX_BITS.store(dx.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Effective play area X offset: base plus reflow.
///
/// Use it, not the raw `PLAY_OFFSET_X`, to position any play-area element in X.
#[inline]
pub fn play_offset_x() -> f32 {
    PLAY_OFFSET_X + f32::from_bits(PLAY_REFLOW_DX_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

/// Play area offset Y
pub const PLAY_OFFSET_Y: f32 = -1.0 - PLAY_AREA_HEIGHT * CAMERA_HEIGHT_RATIO;

/// Play area offset Z (depth into screen)
pub const PLAY_OFFSET_Z: f32 = -42.5;
/// Z position for sparkle particles (slightly in front of blocks)
pub const SPARK_Z: f32 = PLAY_OFFSET_Z + 0.6;

/// Position of a single nucleon on its orbit at a given instant.
pub struct NucleonPosition {
    /// Offset from center (world units)
    pub dx: f32,
    pub dy: f32,
}

/// Compute positions for `n` nucleons orbiting a center point.
///
/// Each nucleon sits on its own elliptical orbit, uniformly tilted
/// so that the orbit orientations are evenly spread over 180°
/// (2 nucleons: 90° apart, 3: 60°, 4: 45°). The trajectories stay
/// visually distinct whatever the count.
///
/// Parameters:
/// - `n`: number of nucleons (0 returns empty)
/// - `semi_major`, `semi_minor`: ellipse axes
/// - `phase_base`: angular position on the orbit (radians, animated externally)
/// - `offset_angle`: global rotation applied to all orbit orientations
///
/// Returns one `NucleonPosition` per nucleon, in angular order.
pub fn nucleon_orbit_positions(
    n: u32,
    semi_major: f32,
    semi_minor: f32,
    phase_base: f32,
    offset_angle: f32,
) -> Vec<NucleonPosition> {
    (0..n).map(|i| {
        // Each orbit is tilted by i * PI / n so orientations are
        // maximally spread. PI (not TAU) because an ellipse has
        // 180° symmetry — tilting by PI/n gives the widest visual
        // separation between orbit paths.
        let orbit_tilt = offset_angle
            + (i as f32) * std::f32::consts::PI / n as f32;

        // Phase offset per nucleon so they don't cluster at the
        // same point on their respective orbits.
        let phase = phase_base
            + (i as f32) * std::f32::consts::TAU / n as f32;

        // Ellipse point in local frame
        let ex = semi_major * phase.cos();
        let ey = semi_minor * phase.sin();

        // Rotate into the orbit's tilted frame
        let (cos_t, sin_t) = (orbit_tilt.cos(), orbit_tilt.sin());
        NucleonPosition {
            dx: ex * cos_t - ey * sin_t,
            dy: ex * sin_t + ey * cos_t,
        }
    }).collect()
}

/// Block flavor colors (RGB), indexed by flavor (0=purple … 5=gray).
pub const BLOCK_COLORS: [[f32; 3]; 6] = [
    [0.73, 0.0,  0.73],  // 0: Purple (BF_PURPLE)
    [0.2,  0.2,  0.8 ],  // 1: Blue   (BF_BLUE)
    [0.0,  0.6,  0.05],  // 2: Green  (BF_GREEN)
    [0.85, 0.85, 0.0 ],  // 3: Yellow (BF_YELLOW)
    [1.0,  0.4,  0.0 ],  // 4: Orange (BF_ORANGE)
    [0.4,  0.4,  0.4 ],  // 5: Gray   (BF_GRAY)
];

/// Creep row colors (25% brightness of the main palette).
/// Used for blocks at y=0 (preparation row rising from below)
pub const CREEP_COLORS: [[f32; 3]; 6] = [
    [0.25 * 0.73, 0.25 * 0.0,  0.25 * 0.73],  // 0: Purple
    [0.25 * 0.2,  0.25 * 0.2,  0.25 * 0.8 ],  // 1: Blue
    [0.25 * 0.0,  0.25 * 0.6,  0.25 * 0.05],  // 2: Green
    [0.25 * 0.85, 0.25 * 0.85, 0.25 * 0.0 ],  // 3: Yellow
    [0.25 * 1.0,  0.25 * 0.4,  0.25 * 0.0 ],  // 4: Orange
    [0.25 * 0.4,  0.25 * 0.4,  0.25 * 0.4 ],  // 5: Gray
];

/// Pop rotation duration in frames.
pub const POP_ROTATE_TIME: i32 = 4 * INTERNAL_POP_DELAY / 5;

/// Determine animation state from block snapshot
pub fn block_anim_state_from_snapshot(block: &BlockSnapshot) -> BlockAnimState {
    let state = BlockState::from_bits_truncate(block.state);

    if state.contains(BlockState::SWAPPING) {
        BlockAnimState::Swapping
    } else if state.contains(BlockState::DYING) {
        // Flash or shrink, depending on the remaining countdown.
        if block.animation_countdown > DYING_DELAY - DYING_FLASH_TIME {
            BlockAnimState::DyingFlash
        } else {
            BlockAnimState::DyingShrink
        }
    } else if state.contains(BlockState::AWAKING) {
        BlockAnimState::AwakingPop
    } else {
        // Static and Falling both use Static animation state
        BlockAnimState::Static
    }
}

/// Generate BlockInstance data from a block snapshot
pub fn generate_block_instance(
    block: &BlockSnapshot,
    play_offset_y: f32,
    swap_factor: f32,
    time_step: i32,
) -> BlockInstance {
    let anim_state = block_anim_state_from_snapshot(block);

    let base_x = play_offset_x() + (block.grid_x as f32) * GRID_ELEMENT_LENGTH;
    let base_y = PLAY_OFFSET_Y
        + play_offset_y * GRID_ELEMENT_LENGTH  // Convert to world units
        + (block.grid_y as f32) * GRID_ELEMENT_LENGTH
        + (block.sub_grid_y as f32 / STEPS_PER_GRID as f32) * GRID_ELEMENT_LENGTH;
    let base_z = PLAY_OFFSET_Z;

    // Adjust position for swapping animation
    let (pos_x, pos_y, pos_z) = if anim_state == BlockAnimState::Swapping {
        // During swap, blocks move in an arc
        let state = BlockState::from_bits_truncate(block.state);
        let direction = if state.contains(BlockState::SWAP_DIRECTION) { 1.0 } else { -1.0 };

        // swap_factor goes from 0.0 to 1.0 during the swap.
        // The block animation progress is the complement (1.0 at start, 0.0 at end) to match
        // the block shader STATE_SWAPPING semantic (angle = -180 * (1 - t)).
        let progress = 1.0 - swap_factor;
        let offset_x = direction * progress * GRID_ELEMENT_LENGTH;

        (base_x + offset_x * 0.5, base_y, base_z)
    } else {
        (base_x, base_y, base_z)
    };

    // Animation progress
    let anim_progress = match anim_state {
        BlockAnimState::Swapping => 1.0 - swap_factor,
        BlockAnimState::DyingFlash => {
            let time_in_flash = (DYING_DELAY as f32) - (block.animation_countdown as f32);
            (time_in_flash / DYING_FLASH_TIME as f32).clamp(0.0, 1.0)
        }
        BlockAnimState::DyingShrink => {
            let shrink_time = (DYING_DELAY - DYING_FLASH_TIME) as f32;
            let time_in_shrink = shrink_time - (block.animation_countdown as f32);
            (time_in_shrink / shrink_time).clamp(0.0, 1.0)
        }
        BlockAnimState::AwakingPop => {
            // pop_countdown == 0 means the pop is already consumed: animation complete.
            if block.pop_countdown == 0 {
                1.0 // Pop finished — stay at full size
            } else if block.pop_countdown - time_step >= POP_ROTATE_TIME {
                0.0 // Not yet started
            } else {
                let p = (block.pop_countdown - time_step) as f32 / POP_ROTATE_TIME as f32;
                (1.0 - p).clamp(0.0, 1.0)
            }
        }
        // Grid blocks never drift: that state belongs to the menu backdrop.
        BlockAnimState::Static | BlockAnimState::Drifting => 0.0,
    };

    // Blocks at y=0 (creep row) use the dimmer color palette.
    let flavor_idx = (block.flavor as usize).min(BLOCK_COLORS.len() - 1);
    let final_color = if block.grid_y == 0 {
        CREEP_COLORS[flavor_idx]
    } else {
        BLOCK_COLORS[flavor_idx]
    };

    // Awaking blocks emerge from the garbage color and transition
    // to their final block color during the pop animation.
    let color = if anim_state == BlockAnimState::AwakingPop {
        let garbage_color_idx = (block.pop_color as usize).min(GARBAGE_COLORS.len() - 1);
        let garbage_col = GARBAGE_COLORS[garbage_color_idx];
        // anim_progress: 0.0 = start (garbage color), 1.0 = done (block color)
        // Use smoothstep for a more polished transition than linear
        let t = anim_progress * anim_progress * (3.0 - 2.0 * anim_progress);
        [
            garbage_col[0] * (1.0 - t) + final_color[0] * t,
            garbage_col[1] * (1.0 - t) + final_color[1] * t,
            garbage_col[2] * (1.0 - t) + final_color[2] * t,
        ]
    } else {
        final_color
    };

    // Visual flags: boolean bits for shader conditionals
    let state_bits = BlockState::from_bits_truncate(block.state);
    let swap_direction = state_bits.contains(BlockState::SWAP_DIRECTION);
    // FLAG_SWAP_BOUNCE=1, FLAG_DYING_WOBBLE=2, FLAG_AWAKING_EASE_OUT=4
    let mut visual_flags_bits: u32 = 0;
    if anim_state == BlockAnimState::Swapping {
        visual_flags_bits |= 1; // FLAG_SWAP_BOUNCE - always enabled for blocks
    }
    if anim_state == BlockAnimState::DyingShrink {
        visual_flags_bits |= 2; // FLAG_DYING_WOBBLE - always enabled for blocks
    }
    if anim_state == BlockAnimState::AwakingPop {
        visual_flags_bits |= 4; // FLAG_AWAKING_EASE_OUT
    }

    // DyingShrink contract: the shader builds the rotation axis
    // (axis_x, axis_y, sin(nx*2.5)*0.8 + 0.0001) and normalizes it. The 0.0001
    // guard is not absolute: if axis_x = axis_y = 0 and the sine term vanishes
    // (block exactly at the center of the area), the axis is near zero and
    // normalizing gives NaN. The core must therefore provide a non-zero axis
    // for dying blocks.
    let anim_axis_x = match anim_state {
        BlockAnimState::DyingShrink => block.axis_x,
        BlockAnimState::Swapping => if swap_direction { 1.0 } else { -1.0 },
        BlockAnimState::AwakingPop => block.pop_direction as f32,
        _ => 0.0,
    };

    // Awaking blocks are visibly translucent until they become STATIC.
    // No gradual fade — the contrast must be unambiguous.
    let alpha = if anim_state == BlockAnimState::AwakingPop { 0.45 } else { 1.0 };

    BlockInstance {
        position: [pos_x, pos_y, pos_z],
        state: anim_state.as_f32(),
        color: [color[0], color[1], color[2], alpha],
        anim_progress,
        anim_axis_x,
        anim_axis_y: if anim_state == BlockAnimState::DyingShrink { block.axis_y } else { 0.0 },
        anim_pivot_offset: if anim_state == BlockAnimState::Swapping { GRID_ELEMENT_LENGTH / 2.0 } else { 0.0 },
        // `light_pos` is not read by any shader. The field is kept with a fixed
        // value to preserve the instance layout.
        light_pos: [0.0, 1.0, 0.0],
        visual_flags: visual_flags_bits,
        flavor_idx: flavor_idx as f32,
        // Pure padding.
        _pad: [0.0; 3],
    }
}


// Awaking state value, taken from the core enum so the two cannot drift apart.
const GARBAGE_STATE_AWAKING: u8 = crackattack_core::garbage::GarbageState::Awaking as u8;

/// Garbage colors by flavor (0=normal red, 1=gray).
pub const GARBAGE_COLORS: [[f32; 3]; 2] = [
    [1.0,  0.0,  0.0 ],   // 0: GARBAGE_FLAVOR_NORMAL (red)
    [0.4,  0.4,  0.4 ],   // 1: GARBAGE_FLAVOR_GRAY
];

/// Shatter flash duration in frames.
pub const SHATTER_FLASH_TIME: i32 = 12;

/// Total shatter animation duration in frames.
///
/// Re-exported from the core: a local copy could drift out of sync with the
/// flash window.
pub use crackattack_core::constants::SHATTER_TIME;

/// Compute flash intensity during garbage shatter (sine pulse over the flash window).
fn compute_garbage_flash_intensity(shatter_progress: f32) -> f32 {
    let flash_ratio = SHATTER_FLASH_TIME as f32 / SHATTER_TIME as f32;
    if shatter_progress >= flash_ratio {
        return 0.0;
    }
    let t = shatter_progress / flash_ratio;
    (2.0 * std::f32::consts::PI * t).sin().abs()
}

/// Pre-computed render parameters for a garbage block, shared by all
/// instance generation paths (shell, piece, cell).
pub struct GarbageRenderParams {
    pub base_x: f32,
    pub base_y: f32,
    pub base_z: f32,
    pub color: [f32; 4],
    pub clip_plane: [f32; 4],
    pub clip_flag: u32,
    pub flavor_idx: usize,
}

/// Compute all shared render parameters for a garbage block.
pub fn compute_garbage_render_params(
    garbage: &GarbageSnapshot,
    play_offset_y: f32,
) -> GarbageRenderParams {
    let gl = GRID_ELEMENT_LENGTH;

    let base_x = play_offset_x() + (garbage.grid_x as f32) * gl;
    let base_y = PLAY_OFFSET_Y
        + play_offset_y * gl
        + (garbage.grid_y as f32) * gl
        + (garbage.sub_grid_y as f32 / STEPS_PER_GRID as f32) * gl;
    let base_z = PLAY_OFFSET_Z;

    let flavor_idx = (garbage.flavor as usize).min(GARBAGE_COLORS.len() - 1);
    let base_color = GARBAGE_COLORS[flavor_idx];
    let flash = compute_garbage_flash_intensity(garbage.shatter_progress);
    let color = [
        base_color[0] + flash * (1.0 - base_color[0]),
        base_color[1] + flash * (1.0 - base_color[1]),
        base_color[2] + flash * (1.0 - base_color[2]),
        1.0,
    ];

    let is_shattering = garbage.shatter_progress > 0.0;
    let is_awaking = garbage.state == GARBAGE_STATE_AWAKING
        && garbage.awaking_progress > 0.0
        && garbage.awaking_progress < 1.0;

    let smoothstep = |t: f32| -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };

    let h = garbage.height as f32;
    let clip_plane = if is_shattering {
        let start_w = gl * 0.5;
        let end_w = -h * gl;
        let eased = smoothstep(garbage.shatter_progress);
        let w_local = start_w + (end_w - start_w) * eased;
        [0.0_f32, 1.0, 0.0, w_local - base_y]
    } else if is_awaking {
        let start_w = gl * 0.5;
        let end_w = -h * gl;
        let eased = smoothstep(garbage.awaking_progress);
        let w_local = end_w + (start_w - end_w) * eased;
        [0.0_f32, 1.0, 0.0, w_local - base_y]
    } else {
        [0.0, 0.0, 0.0, 0.0]
    };

    let clip_flag = if is_shattering || is_awaking { 1 } else { 0 }; // bit 0 = clipEnabled

    GarbageRenderParams {
        base_x, base_y, base_z, color,
        clip_plane, clip_flag, flavor_idx,
    }
}

/// Flag bits for GarbagePieceInstance (matches shader interpretation)
const FLAG_MIRROR_X: u32 = 8;
const FLAG_MIRROR_Y: u32 = 16;
const FLAG_MIRROR_Z: u32 = 32;
const FLAG_SWAP_XY: u32 = 64;

/// Garbage shell instances grouped by mesh type for separate draw calls.
pub struct GarbageShellInstances {
    pub thick_corner: Vec<GarbagePieceInstance>,
    pub thick_edge: Vec<GarbagePieceInstance>,
    pub thick_middle: Vec<GarbagePieceInstance>,
    pub thin_cap: Vec<GarbagePieceInstance>,
    pub thin_middle: Vec<GarbagePieceInstance>,
    pub small: Vec<GarbagePieceInstance>,
    /// One frontal quad per garbage — a single quad covering the
    /// entire W×H face with UV [0,1]². Vertex shader scales from
    /// unit quad to garbage size using W/H in flags bits 16-23.
    pub frontal: Vec<GarbagePieceInstance>,
}

impl Default for GarbageShellInstances {
    fn default() -> Self {
        Self::new()
    }
}

impl GarbageShellInstances {
    pub fn new() -> Self {
        Self {
            thick_corner: Vec::new(),
            thick_edge: Vec::new(),
            thick_middle: Vec::new(),
            thin_cap: Vec::new(),
            thin_middle: Vec::new(),
            small: Vec::new(),
            frontal: Vec::new(),
        }
    }
}

/// Flatten garbage shell instances into a single buffer with draw ranges per mesh type.
/// Returns (combined_instances, draw_commands) where each draw command is (mesh, count, first_instance).
pub fn assemble_garbage_draws(
    shell: &GarbageShellInstances,
) -> (Vec<GarbagePieceInstance>, Vec<(BuiltinMesh, u32, u32)>) {
    let mut instances: Vec<GarbagePieceInstance> = Vec::new();
    let mut draws: Vec<(BuiltinMesh, u32, u32)> = Vec::new();

    let groups: [(BuiltinMesh, &[GarbagePieceInstance]); 7] = [
        (BuiltinMesh::GarbageThickCorner, &shell.thick_corner),
        (BuiltinMesh::GarbageThickEdge, &shell.thick_edge),
        (BuiltinMesh::GarbageThickMiddle, &shell.thick_middle),
        (BuiltinMesh::GarbageThinCap, &shell.thin_cap),
        (BuiltinMesh::GarbageThinMiddle, &shell.thin_middle),
        (BuiltinMesh::GarbageSmall, &shell.small),
        (BuiltinMesh::GarbageFrontal, &shell.frontal),
    ];

    for (mesh, group) in &groups {
        if !group.is_empty() {
            let first = instances.len() as u32;
            let count = group.len() as u32;
            instances.extend_from_slice(group);
            draws.push((*mesh, count, first));
        }
    }

    (instances, draws)
}

/// Generate garbage shell instances for a single garbage block.
///
/// Produces corner, edge, and middle shell pieces that form a beveled 3D shell.
pub fn generate_garbage_shell_instances(
    garbage: &GarbageSnapshot,
    play_offset_y: f32,
    out: &mut GarbageShellInstances,
) {
    let w = garbage.width;
    let h = garbage.height;
    let gl = GRID_ELEMENT_LENGTH; // 2.0

    let p = compute_garbage_render_params(garbage, play_offset_y);
    let base_x = p.base_x;
    let base_y = p.base_y;
    let base_z = p.base_z;
    let color = p.color;
    let clip_plane = p.clip_plane;
    let clip_flag = p.clip_flag;
    let flavor_idx = p.flavor_idx;

    // Garbage flavor in bit 7 (matches block flavorIdx convention: CPU prepares, shader consumes)
    let flavor_flag = (flavor_idx as u32 & 1) << 7;

    // Per-garbage texture offset seed (bits 24-31 of flags)
    // Based on garbage.id (stable for lifetime) not position (changes when falling)
    let id_seed = garbage.id as f32 * 12.9898;
    let hash_u = id_seed.sin().abs().fract();
    let hash_v = (id_seed + 1.0).sin().abs().fract();
    let hash_u_bits = ((hash_u * 15.0) as u32).min(15) << 24;
    let hash_v_bits = ((hash_v * 15.0) as u32).min(15) << 28;
    let seed_flags = hash_u_bits | hash_v_bits;

    // Helper to create a piece instance
    let make_piece = |dx: f32, dy: f32, extra_flags: u32| -> GarbagePieceInstance {
        // Identity uvTransform. The flavor quads, the only real readers of
        // uvTransform, set their own in the scene recording.
        let piece_uv = [1.0, 1.0, 0.0, 0.0];
        // Encode cell position + garbage dims in bits 8-23 for
        // continuous procedural noise (same encoding as piece cubes).
        let mut flags = clip_flag | extra_flags | seed_flags | flavor_flag;
        flags |= ((dx as u32) & 0xF) << 8;
        flags |= ((dy as u32) & 0xF) << 12;
        flags |= ((w as u32) & 0xF) << 16;
        flags |= ((h as u32) & 0xF) << 20;
        GarbagePieceInstance {
            position: [base_x + dx * gl, base_y + dy * gl, base_z],
            flags,
            color,
            uv_transform: piece_uv,
            clip_plane_eye: clip_plane,
        }
    };

    if h == 1 {
        // Thin garbage (height = 1)
        if w == 1 {
            // Small: single piece
            out.small.push(make_piece(0.0, 0.0, 0));
        } else {
            // Left cap
            out.thin_cap.push(make_piece(0.0, 0.0, 0));
            // Middle sections
            for i in 1..(w - 1) {
                out.thin_middle.push(make_piece(i as f32, 0.0, 0));
            }
            // Right cap (mirrored: scale [-1, 1, -1])
            out.thin_cap.push(make_piece((w - 1) as f32, 0.0, FLAG_MIRROR_X | FLAG_MIRROR_Z));
        }
    } else {
        // Thick garbage (height >= 2)

        // Bottom row (y=0)
        let bottom_flags = FLAG_MIRROR_Y | FLAG_MIRROR_Z;

        // Bottom-left corner
        out.thick_corner.push(make_piece(0.0, 0.0, bottom_flags));
        // Bottom edges
        for i in 1..(w - 1) {
            out.thick_edge.push(make_piece(i as f32, 0.0, bottom_flags));
        }
        // Bottom-right corner: mirror flags combine to scale(-1,-1,1).
        out.thick_corner.push(make_piece((w - 1) as f32, 0.0, FLAG_MIRROR_X | FLAG_MIRROR_Y));

        // Middle rows, from y=1 to y=h-2.
        for row in 1..(h - 1) {
            let mid_base_flags = FLAG_SWAP_XY;

            // Right edge (first after swap, at x = w-1)
            out.thick_edge.push(make_piece((w - 1) as f32, row as f32, mid_base_flags));
            // Middle sections (going from right to left after swap)
            for col in 1..(w - 1) {
                // After xy_swap, pieces go from x=w-2 down to x=1
                out.thick_middle.push(make_piece((w - 1 - col) as f32, row as f32, mid_base_flags));
            }
            // Left edge: swap XY then mirror Y and Z.
            out.thick_edge.push(make_piece(0.0, row as f32, mid_base_flags | FLAG_MIRROR_Y | FLAG_MIRROR_Z));
        }

        // Top row (y=h-1)
        // Top-left corner
        out.thick_corner.push(make_piece(0.0, (h - 1) as f32, 0));
        // Top edges
        for i in 1..(w - 1) {
            out.thick_edge.push(make_piece(i as f32, (h - 1) as f32, 0));
        }
        // Top-right corner (mirrored X and Z)
        out.thick_corner.push(make_piece((w - 1) as f32, (h - 1) as f32, FLAG_MIRROR_X | FLAG_MIRROR_Z));
    }

    // The frontal quad is not emitted: it would z-fight with the internal cubes.
    // Enabling it would require replacing those cubes.
}


/// Generate all instances for a frame (blocks + next-row preview).
///
/// Pure function: all data comes from `frame`, no state is carried between
/// calls.
///
/// Garbage is not included here. It has its own path, driven by
/// `assemble_garbage_draws` and `generate_garbage_shell_instances`. The
/// swapper cursor is drawn as a UI silhouette by the renderer runtime.
pub fn generate_frame_instances(
    frame: &FrameData,
    impact_phases: &std::collections::HashMap<i32, f32>,
) -> Vec<BlockInstance> {
    let capacity = frame.blocks.len();
    let mut instances = Vec::with_capacity(capacity);

    for block in &frame.blocks {
        let mut instance = generate_block_instance(
            block,
            frame.play_offset_y,
            frame.swapper.swap_factor,
            frame.time_step,
        );

        // Darken the block as it dies. The shrink phase already pulls the
        // scale toward 0, but the color staying bright read as "still in
        // play right up to the last frame"; dimming it lets the piece feel
        // like it's "fading out with the scale", matching the idea that it
        // disappears into the gap it's making. Linear ramp over the shrink
        // anim_progress (0 at the start of shrink, 1 at full retract),
        // saturated to a 70 % dim so the silhouette stays readable.
        if instance.state == BlockAnimState::DyingShrink.as_f32() {
            let p = instance.anim_progress.clamp(0.0, 1.0);
            // sqrt curve: darkening ramps up fast at the start then
            // eases toward the final 15 % so the block reads as
            // "already going away" well before the shrink ends.
            let k = 1.0 - p.sqrt() * 0.85;
            instance.color[0] *= k;
            instance.color[1] *= k;
            instance.color[2] *= k;
        }
        // Inject squash & stretch impact phase into param0 for Static blocks.
        // The `generate_block_instance`
        // above sets `param0 = 0.0` when `anim_state == Static` so
        // overwriting it is safe; for non-Static blocks the param0
        // slot is already occupied by an animation-specific value
        // (axis_x for Dying, direction for Swapping, pop_dir for
        // Awaking) and the impact phase stays dormant — the shader's
        // Static branch is the only place that reads it anyway.
        let is_static = instance.state == BlockAnimState::Static.as_f32();
        if is_static {
            if let Some(&phase) = impact_phases.get(&block.id) {
                instance.anim_axis_x = phase;
            }
        }
        instances.push(instance);
    }

    // When awaking_progress > 0, the shell handles rendering; skip these cells.
    for garbage in &frame.garbage {
        if garbage.state != GARBAGE_STATE_AWAKING || garbage.awaking_progress > 0.0 {
            continue;
        }
        let base_y = PLAY_OFFSET_Y
            + frame.play_offset_y * GRID_ELEMENT_LENGTH
            + (garbage.grid_y as f32) * GRID_ELEMENT_LENGTH
            + (garbage.sub_grid_y as f32 / STEPS_PER_GRID as f32) * GRID_ELEMENT_LENGTH;

        let flavor_color = GARBAGE_COLORS[(garbage.flavor as usize).min(GARBAGE_COLORS.len() - 1)];
        let pop_color = GARBAGE_COLORS[(garbage.pop_color as usize).min(GARBAGE_COLORS.len() - 1)];

        // Each cell has 3 states: already popped, popping, not yet popped.
        let mut counter = garbage.sections_popped;
        for h in 0..garbage.height {
            for w in 0..garbage.width {
                let px = play_offset_x() + (garbage.grid_x + w) as f32 * GRID_ELEMENT_LENGTH;
                let py = base_y + h as f32 * GRID_ELEMENT_LENGTH;

                let (state, color, progress, pop_dir) = if counter > 0 {
                    (BlockAnimState::Static, flavor_color, 0.0, 0.0)
                } else if counter == 0 && garbage.pop_progress > 0.0 {
                    (BlockAnimState::AwakingPop, pop_color, garbage.pop_progress, garbage.pop_direction as f32)
                } else {
                    (BlockAnimState::AwakingPop, pop_color, 0.0, garbage.pop_direction as f32)
                };
                counter -= 1;

                instances.push(BlockInstance {
                    position: [px, py, PLAY_OFFSET_Z],
                    state: state.as_f32(),
                    color: [color[0], color[1], color[2], 0.45],
                    anim_progress: progress,
                    anim_axis_x: pop_dir,
                    ..BlockInstance::default()
                });
            }
        }
    }

    // Next row preview at y=-1 (fills visual gap below creep row)
    for x in 0..PLAY_WIDTH {
        let flavor = frame.next_row[x] as usize;
        let creep_table = &CREEP_COLORS;
        let flavor_clamped = flavor.min(CREEP_COLORS.len() - 1);
        let color = creep_table[flavor_clamped];
        let base_x = play_offset_x() + (x as f32) * GRID_ELEMENT_LENGTH;
        let base_y = PLAY_OFFSET_Y
            + frame.play_offset_y * GRID_ELEMENT_LENGTH
            - GRID_ELEMENT_LENGTH;
        instances.push(BlockInstance {
            position: [base_x, base_y, PLAY_OFFSET_Z],
            state: BlockAnimState::Static.as_f32(),
            color: [color[0], color[1], color[2], 1.0],
            flavor_idx: flavor_clamped as f32,
            ..BlockInstance::default()
        });
    }

    instances
}

