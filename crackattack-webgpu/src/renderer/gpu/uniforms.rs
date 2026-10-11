//! GPU Uniform Structures

use bytemuck::{Pod, Zeroable};

/// UBO alignment for dynamic offsets
/// WebGPU minimum is 256; 512 provides a safe margin for dynamic-offset alignment.
pub const UBO_ALIGNMENT: u64 = 512;

// TransformUniformsGPU - 352 bytes (Binding 0, DYNAMIC OFFSET)
//
// CRITICAL NOTES:
// - normalMatrix is mat4x4f in WGSL (not mat3x3) - ignore w
// - clipEnabled is vec4f (NOT u32!) with special encoding

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct TransformUniformsGPU {
    /// Model-View-Projection matrix (column-major)
    pub mvp: [[f32; 4]; 4], // 64 bytes

    /// Model-View matrix (for lighting in eye space)
    pub model_view: [[f32; 4]; 4], // 64 bytes

    /// Normal matrix (transpose of inverse model-view, stored as mat4x4)
    pub normal_matrix: [[f32; 4]; 4], // 64 bytes

    /// Unused: no shader reads it. The slot is kept to preserve the 352-byte layout.
    pub tex_matrix: [[f32; 4]; 4], // 64 bytes

    /// Unused: no shader reads it, and it is always the identity matrix.
    /// The slot is kept to preserve the layout.
    pub tex_matrix1: [[f32; 4]; 4], // 64 bytes

    /// Clip plane (a, b, c, d) OR glow color (rgb, _) in dissolve mode
    pub clip_plane: [f32; 4], // 16 bytes

    /// Clip/dissolve encoding:
    /// .x: 0.0 = none, 1.0 = clip plane, 2.0 = dissolve
    /// .y: dissolve progress (0-1) when mode dissolve
    /// .z: edge glow intensity when mode dissolve
    /// .w: unused
    pub clip_enabled: [f32; 4], // 16 bytes
}

// Compile-time size verification
const _: () = assert!(std::mem::size_of::<TransformUniformsGPU>() == 352);

impl Default for TransformUniformsGPU {
    fn default() -> Self {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        Self {
            mvp: identity,
            model_view: identity,
            normal_matrix: identity,
            tex_matrix: identity,
            tex_matrix1: identity,
            clip_plane: [0.0, 0.0, 0.0, 0.0],
            clip_enabled: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

// LightingUniformsGPU - 96 bytes (Binding 1, NOT dynamic); see the size_of assert below
//
// NOTE: lighting enable is encoded as float (not int) for GPU compatibility

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct LightingUniformsGPU {
    /// Ambient light color
    pub ambient: [f32; 4], // 16 bytes

    /// Diffuse light color
    pub diffuse: [f32; 4], // 16 bytes

    /// Specular light color
    pub specular: [f32; 4], // 16 bytes

    /// Light position in eye space (w=1 point, w=0 directional)
    pub light_pos: [f32; 4], // 16 bytes

    /// Specular shininess exponent
    pub shininess: f32, // 4 bytes

    /// Panic intensity (0.0 = normal, 1.0 = full panic), derived from
    /// `free_rows`. No shader reads this slot, but the CPU uses the value:
    /// `renderer_runtime/mod.rs` low-pass filters it, then modulates
    /// `diffuse` to produce the panic flashing light. Do not freeze this
    /// field without moving that state elsewhere.
    pub panic_intensity: f32, // 4 bytes

    /// Reserved: neither shaders nor the CPU read it, and it stays at 0.
    /// The slot is kept to preserve the 96-byte layout.
    pub game_time: f32, // 4 bytes

    /// Garbage warm gradient intensity (0.0=none, 1.0=full).
    pub garbage_warm: f32, // 4 bytes

    /// Bounding box of currently dying blocks,
    /// in world coordinates. Layout: `[min_x, max_x, min_y, max_y]`.
    /// Updated each frame from the CPU pass that scans `frame.blocks`
    /// for blocks in `DyingShrink` state. BLOCK_SHADER's varied
    /// dying animation reads this to normalize a block's position
    /// to a local `[-1, +1]` range *relative to the dissolution
    /// zone*, so a 3-block chain gets the full coherent gradient
    /// instead of a tiny slice of the playfield-wide one.
    pub dying_zone: [f32; 4], // 16 bytes
}

// Compile-time size verification: 80 + 16 = 96
const _: () = assert!(std::mem::size_of::<LightingUniformsGPU>() == 96);
// `dying_zone` is read by BLOCK_SHADER's varied dying path. Its offset is
// locked so that a layout change fails to compile instead of breaking rendering.
const _: () = assert!(std::mem::offset_of!(LightingUniformsGPU, dying_zone) == 80);

impl Default for LightingUniformsGPU {
    fn default() -> Self {
        Self {
            ambient: [0.2, 0.2, 0.2, 1.0],
            diffuse: [1.0, 1.0, 1.0, 1.0],
            specular: [1.0, 1.0, 1.0, 1.0],
            // Headlight position: (0, 4.2, -35) relative to camera
            light_pos: [0.0, 4.2, -35.0, 1.0], // positional headlight
            shininess: 10.0,
            panic_intensity: 0.0,
            game_time: 0.0,
            garbage_warm: 0.25,
            // Neutral dying zone — when no blocks are dying,
            // the field is irrelevant (the varied path only runs
            // for blocks in DyingShrink state). Width / height of
            // 1.0 each just to avoid divide-by-zero in shader if
            // it ever reads with zero-sized blocks.
            dying_zone: [-0.5, 0.5, -0.5, 0.5],
        }
    }
}

// MaterialUniformsGPU - 96 bytes (Binding 2, DYNAMIC OFFSET); see the size_of/offset_of asserts below
//
// Shader usage:
//   - read: `diffuse` and `emission` (blocks). The level lights use WGSL
//     constants and do not declare this struct.
//   - unused (no shader reads them): `ambient`, `specular`, `shininess`,
//     `use_texture`, `use_emission`, `vertex_color`. Before reusing an unused
//     slot, grep the remaining WGSL declaration (block.wgsl).

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MaterialUniformsGPU {
    /// Diffuse color (.a = alpha), read by BLOCK_SHADER.
    pub diffuse: [f32; 4], // 16 bytes

    /// Unused: no shader reads it.
    pub ambient: [f32; 4], // 16 bytes

    /// Unused: the level lights use WGSL constants.
    pub specular: [f32; 4], // 16 bytes

    /// Emission color, read by BLOCK_SHADER.
    pub emission: [f32; 4], // 16 bytes

    /// Unused, like `specular`.
    pub shininess: f32, // 4 bytes

    /// Unused: no shader reads it. The UI texture variants are dedicated pipelines.
    pub use_texture: f32, // 4 bytes

    /// Unused: no shader reads it.
    pub use_emission: f32, // 4 bytes

    /// Alignment padding: `vertex_color` is a vec4f aligned to 16 bytes.
    pub _pad0: f32, // 4 bytes

    /// Unused: no shader reads it. Last field; its offset is locked below.
    pub vertex_color: [f32; 4], // 16 bytes
}

// Compile-time size verification
const _: () = assert!(std::mem::size_of::<MaterialUniformsGPU>() == 96);

/// The single, constant material used by all blocks.
pub fn block_material_uniforms() -> MaterialUniformsGPU {
    MaterialUniformsGPU {
        diffuse: [1.0, 1.0, 1.0, 1.0],
        ambient: [0.0, 0.0, 0.0, 1.0],
        specular: [0.5, 0.5, 0.5, 1.0],
        emission: [0.0, 0.0, 0.0, 1.0],
        shininess: 10.0,
        use_texture: 0.0,
        use_emission: 0.0,
        _pad0: 0.0,
        vertex_color: [1.0, 1.0, 1.0, 1.0],
    }
}
// `vertex_color` is the last field. Its offset is locked so that a layout
// change fails to compile instead of breaking the WGSL layout.
const _: () = assert!(std::mem::offset_of!(MaterialUniformsGPU, vertex_color) == 80);

impl Default for MaterialUniformsGPU {
    fn default() -> Self {
        Self {
            diffuse: [1.0, 1.0, 1.0, 1.0],
            ambient: [0.2, 0.2, 0.2, 1.0],
            specular: [0.3, 0.3, 0.3, 1.0],
            emission: [0.0, 0.0, 0.0, 1.0],
            shininess: 32.0,
            use_texture: 0.0,
            use_emission: 0.0,
            _pad0: 0.0,
            vertex_color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

// FrameUniforms (CPU-side, passed to the renderer each frame)

#[derive(Copy, Clone, Debug)]
pub struct FrameUniforms {
    /// Game::time_step - frame counter
    pub time_step: i32,

    /// Vertical scroll offset combining static play offset, current creep, and spring displacement.
    pub play_offset_y: f32,
}

impl Default for FrameUniforms {
    fn default() -> Self {
        Self {
            time_step: 0,
            play_offset_y: 0.0,
        }
    }
}
