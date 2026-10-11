//! GPU infrastructure: types, uniforms, shaders, pipelines, buffers, textures.

pub mod backend;
pub mod compute_sparkles;
pub mod context;
pub mod instance;
pub mod mesh;
pub mod pipeline;
pub mod render_command;
pub mod render_dump;
pub mod ring_buffer;
pub mod screen_overlay;
pub mod shaders;
pub mod targets;
pub mod texture;
pub mod types;
pub mod ui_batch;
pub mod uniforms;

#[cfg(test)]
mod wgsl_tests;
