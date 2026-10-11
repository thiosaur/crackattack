//! Particle systems and GPU resources (compute sparkles).

use crate::renderer::gpu::compute_sparkles::ComputeSparkles;

pub struct ParticleRuntime {
    pub compute_sparkles: ComputeSparkles,
    pub particle_uniform_bind_group: wgpu::BindGroup,
    pub particle_pipeline: wgpu::RenderPipeline,
}
