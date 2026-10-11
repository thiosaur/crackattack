//! Screen-surface overlay pass.
//!
//! Draws curtains, veils and tints that must cover the whole window, margins
//! included, outside the square game viewport. Board overlays
//! (`world_ui_batches`: HUD and gameplay messages) stay in the square pass.
//!
//! A small dedicated pipeline draws them:
//! - vertices are already in clip space (NDC, y up), so there is no projection uniform;
//! - plain alpha blending, no depth, single sample (direct pass on the swapchain,
//!   after the MSAA resolve);
//! - the shader is embedded here and does not depend on `ShaderModules`.

use bytemuck::{Pod, Zeroable};

// `pub(crate)` so the WGSL validation test can compile it.
pub(crate) const SCREEN_OVERLAY_WGSL: &str = r#"
struct VsIn {
    @location(0) pos: vec2f,   // already in clip space (NDC)
    @location(1) color: vec4f,
}
struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) color: vec4f,
}
@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4f(in.pos, 0.0, 1.0);
    out.color = in.color;
    return out;
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    return in.color;
}
"#;

/// Screen overlay vertex: clip-space position and RGBA color (alpha is opacity).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ScreenOverlayVertex {
    pub pos: [f32; 2],
    pub color: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<ScreenOverlayVertex>() == 24);
// The `color` offset is hard-coded in the `VertexBufferLayout` below (offset: 8).
const _: () = assert!(std::mem::offset_of!(ScreenOverlayVertex, color) == 8);

/// Initial vertex buffer capacity, in vertices. The buffer grows on demand.
const INITIAL_CAPACITY_VERTS: u64 = 64;

/// Renders screen-surface overlays: collects clip-space quads, then draws them in
/// one pass that covers the whole surface.
pub struct ScreenOverlayRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    capacity_verts: u64,
    /// CPU-side `screen_overlay_batches` vertices, cleared and refilled every frame.
    verts: Vec<ScreenOverlayVertex>,
}

impl ScreenOverlayRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("screen_overlay_shader"),
            source: wgpu::ShaderSource::Wgsl(SCREEN_OVERLAY_WGSL.into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("screen_overlay_pipeline_layout"),
            bind_group_layouts: &[], // no uniforms
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("screen_overlay_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ScreenOverlayVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 8, shader_location: 1 },
                    ],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(crate::renderer::gpu::pipeline::ALPHA_BLEND),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,         // direct swapchain pass, no depth
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen_overlay_vertex_buffer"),
            size: INITIAL_CAPACITY_VERTS * std::mem::size_of::<ScreenOverlayVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self { pipeline, vertex_buffer, capacity_verts: INITIAL_CAPACITY_VERTS, verts: Vec::new() }
    }

    /// Clears the vertices (call at the start of a frame).
    pub fn clear(&mut self) {
        self.verts.clear();
    }

    /// Adds a rectangle in clip space (NDC, -1..1, y up) with an RGBA color.
    pub fn push_rect_clip(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) {
        let v = |x: f32, y: f32| ScreenOverlayVertex { pos: [x, y], color };
        self.verts.extend_from_slice(&[
            v(x0, y0), v(x1, y0), v(x0, y1),
            v(x1, y0), v(x1, y1), v(x0, y1),
        ]);
    }

    /// Adds a rectangle that covers the whole surface.
    pub fn push_fullscreen(&mut self, color: [f32; 4]) {
        self.push_rect_clip(-1.0, -1.0, 1.0, 1.0, color);
    }

    /// Uploads the vertices, growing the buffer if needed, and returns their count.
    /// Call it outside a render pass: `write_buffer` is not allowed during one.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> u32 {
        let n = self.verts.len() as u64;
        if n == 0 {
            return 0;
        }
        if n > self.capacity_verts {
            let new_cap = n.next_power_of_two();
            self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("screen_overlay_vertex_buffer"),
                size: new_cap * std::mem::size_of::<ScreenOverlayVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.capacity_verts = new_cap;
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.verts));
        self.verts.len() as u32
    }

    /// Draws after `upload`, in a pass whose viewport covers the whole surface.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, vertex_count: u32) {
        if vertex_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..vertex_count, 0..1);
    }
}
