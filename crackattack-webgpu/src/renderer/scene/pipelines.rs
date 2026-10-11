//! 3D scene pipeline resources — immutable after init.

use crate::renderer::gpu::shaders::ShaderModules;
use crate::renderer::scene::meshdata::GarbageShellRanges;

/// Read-only GPU resources: pipelines, bind groups, buffers, shader modules.
/// Immutable after init — can be borrowed as `&` while `executor` is `&mut`.
pub struct ScenePipelines {
    // MSAA and depth buffers
    pub render_targets: crate::renderer::gpu::targets::RenderTargets,

    // Block rendering
    pub block_pipeline: wgpu::RenderPipeline,
    pub block_vertex_buffer: wgpu::Buffer,
    pub block_vertex_count: u32,
    pub instance_buffer: wgpu::Buffer,
    pub instance_bind_group: wgpu::BindGroup,
    pub block_material_bind_group: wgpu::BindGroup,

    // Garbage rendering
    pub garbage_pipeline: wgpu::RenderPipeline,
    pub garbage_shell_vertex_buffer: wgpu::Buffer,
    pub garbage_shell_ranges: GarbageShellRanges,
    pub garbage_instance_buffer: wgpu::Buffer,
    pub garbage_instance_bind_group: wgpu::BindGroup,
    pub garbage_uniform_bind_group: wgpu::BindGroup,
    pub garbage_flavor_bind_groups: Vec<wgpu::BindGroup>,

    // Level light rendering
    pub level_light_pipeline: wgpu::RenderPipeline,
    pub level_light_vertex_buffer: wgpu::Buffer,

    // Frame-level GPU resources
    pub frame_bind_group: wgpu::BindGroup,
    pub uniform_buffer: wgpu::Buffer,
    /// CPU copy of the contents of `uniform_buffer`, updated on resize.
    /// RenderDump hashes this copy, so it matches what the GPU receives.
    pub transform_uniforms: crate::renderer::gpu::uniforms::TransformUniformsGPU,
    pub lighting_buffer: wgpu::Buffer,

    // Shader modules
    pub shader_modules: ShaderModules,
}
