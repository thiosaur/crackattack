//! Core rendering types: vertices, texture modes.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshVertex {
    pub pos: [f32; 3],        // 12 bytes - position xyz
    pub normal: [f32; 3],     // 12 bytes - normal xyz
    pub uv: [f32; 3],         // 12 bytes - mesh UV (uvw); no block or garbage shader reads it
    pub uv_diffuse: [f32; 2], //  8 bytes - diffuse texture UV
}

const _: () = assert!(std::mem::size_of::<MeshVertex>() == 44);

impl MeshVertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3,  // pos
        1 => Float32x3,  // normal
        2 => Float32x3,  // uv
        3 => Float32x2,  // uv_diffuse
    ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct UIVertex {
    pub x: f32,
    pub y: f32,
    pub z: f32, // 12 bytes - world position
    pub u: f32,
    pub v: f32, // 8 bytes - texture coords
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32, // 16 bytes - color
}

const _: () = assert!(std::mem::size_of::<UIVertex>() == 36);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UITextureMode {
    /// No texture (solid color).
    None,
    /// Alpha-only texture (messages, fonts).
    AlphaText,
    /// RGB/RGBA texture (logo, bars).
    Modulate,
}
