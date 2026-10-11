//! Instance buffer structures for GPU instancing
//!
//! These are stored in STORAGE BUFFERS, not vertex buffers.

use bytemuck::{Pod, Zeroable};

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockAnimState {
    /// No animation
    Static = 0,

    /// Flash towards white
    DyingFlash = 1,

    /// Shrink + rotation
    DyingShrink = 2,

    /// Rotation around Y axis
    Swapping = 3,

    /// Pop rotation + scale
    AwakingPop = 4,
}

impl BlockAnimState {
    /// Convert to float for GPU
    pub fn as_f32(self) -> f32 {
        self as u32 as f32
    }
}

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct BlockInstance {
    /// World position
    pub position: [f32; 3], // 12 bytes

    /// Animation state (BlockAnimState as float)
    pub state: f32, // 4 bytes

    /// Block color (RGBA)
    pub color: [f32; 4], // 16 bytes

    /// Animation progress (0→1)
    pub anim_progress: f32, // 4 bytes

    /// Animation axis X / direction / pop_dir
    pub anim_axis_x: f32, // 4 bytes

    /// Animation axis Y
    pub anim_axis_y: f32, // 4 bytes

    /// Animation pivot offset (swap)
    pub anim_pivot_offset: f32, // 4 bytes

    /// Per-block light position
    pub light_pos: [f32; 3], // 12 bytes

    /// Visual flags bitmask, animation flags only (bits 0-2).
    ///
    /// A native `u32`: integers are legal in a storage buffer. Reinterpreting the bits as
    /// `f32` would give subnormals, which WGSL allows a driver to flush to zero.
    pub visual_flags: u32, // 4 bytes

    /// Block flavor index (0-5) for texture atlas selection
    pub flavor_idx: f32, // 4 bytes

    /// Padding to 80 bytes (next 16-byte aligned size)
    pub _pad: [f32; 3], // 12 bytes
}

const _: () = assert!(std::mem::size_of::<BlockInstance>() == 80);

impl Default for BlockInstance {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            state: BlockAnimState::Static.as_f32(),
            color: [1.0, 1.0, 1.0, 1.0],
            anim_progress: 0.0,
            anim_axis_x: 0.0,
            anim_axis_y: 0.0,
            anim_pivot_offset: 0.0,
            light_pos: [0.0, 1.0, 0.0],
            visual_flags: 0,
            flavor_idx: 0.0,
            _pad: [0.0; 3],
        }
    }
}

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GarbagePieceInstance {
    /// World position
    pub position: [f32; 3], // 12 bytes

    /// Flags: bit 0 = clipEnabled
    pub flags: u32, // 4 bytes

    /// Piece color (RGBA)
    pub color: [f32; 4], // 16 bytes

    /// UV transform: [scaleX, scaleY, offsetX, offsetY]
    pub uv_transform: [f32; 4], // 16 bytes

    /// Clip plane in eye space (a, b, c, d)
    pub clip_plane_eye: [f32; 4], // 16 bytes
}

const _: () = assert!(std::mem::size_of::<GarbagePieceInstance>() == 64);

impl Default for GarbagePieceInstance {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            flags: 0,
            color: [1.0, 1.0, 1.0, 1.0],
            uv_transform: [1.0, 1.0, 0.0, 0.0], // identity
            clip_plane_eye: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ParticleInstance {
    /// World position
    pub position: [f32; 3], // 12 bytes

    /// Rotation around Z axis (radians)
    pub angle: f32, // 4 bytes

    /// Particle color (RGBA)
    pub color: [f32; 4], // 16 bytes

    /// Scale X
    pub scale_x: f32, // 4 bytes

    /// Scale Y (negative for flip)
    pub scale_y: f32, // 4 bytes

    /// Padding
    pub _pad: [f32; 2], // 8 bytes
}

const _: () = assert!(std::mem::size_of::<ParticleInstance>() == 48);

impl Default for ParticleInstance {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            angle: 0.0,
            color: [1.0, 1.0, 1.0, 1.0],
            scale_x: 1.0,
            scale_y: 1.0,
            _pad: [0.0, 0.0],
        }
    }
}
