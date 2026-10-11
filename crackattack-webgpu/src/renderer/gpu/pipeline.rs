//! Pipeline creation and caching for WebGPU
//!
//! - `UIPipelineCache`: variant-keyed cache for UI pipelines (textured, alpha_text, untextured)
//! - `create_3d_pipeline()`: factory for instanced 3D pipelines (block, garbage, level light, particle)

use std::collections::HashMap;

use crate::renderer::gpu::targets::DEFAULT_DEPTH_FORMAT as DEPTH_FORMAT;
use crate::renderer::gpu::shaders::ShaderModules;

use crate::renderer::gpu::targets::DEFAULT_MSAA_SAMPLE_COUNT;

pub const ALPHA_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

pub const ADDITIVE_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

/// UI pipeline variants
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UIPipelineVariant {
    /// RGBA texture modulate with vertex color
    Textured,
    /// Alpha-only texture (fonts, messages)
    AlphaText,
    /// No texture (solid vertex color)
    Untextured,
    /// RGBA texture with additive blending (particles, glows)
    TexturedAdditive,
    /// Alpha texture with additive blending
    AlphaTextAdditive,
}

/// Pipeline cache for UI rendering
pub struct UIPipelineCache {
    pipelines: HashMap<UIPipelineVariant, wgpu::RenderPipeline>,
    layout: wgpu::PipelineLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    /// Whether pipelines bind a depth attachment. false for swapchain-direct passes; true for MSAA scene passes.
    with_depth: bool,
}

impl UIPipelineCache {
    pub fn new(
        device: &wgpu::Device,
        ui_bind_group_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        Self::with_msaa(device, ui_bind_group_layout, surface_format, DEFAULT_MSAA_SAMPLE_COUNT as u32)
    }

    pub fn with_msaa(
        device: &wgpu::Device,
        ui_bind_group_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        Self::with_config(device, ui_bind_group_layout, surface_format, sample_count, true)
    }

    /// Build a cache for a single-sample swapchain pass with no depth attachment.
    pub fn for_swapchain_no_depth(
        device: &wgpu::Device,
        ui_bind_group_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        Self::with_config(device, ui_bind_group_layout, surface_format, 1, false)
    }

    fn with_config(
        device: &wgpu::Device,
        ui_bind_group_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
        with_depth: bool,
    ) -> Self {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui_pipeline_layout"),
            bind_group_layouts: &[ui_bind_group_layout],
            push_constant_ranges: &[],
        });

        Self {
            pipelines: HashMap::new(),
            layout,
            surface_format,
            sample_count,
            with_depth,
        }
    }

    pub fn get_or_create(
        &mut self,
        device: &wgpu::Device,
        shaders: &ShaderModules,
        variant: UIPipelineVariant,
    ) -> &wgpu::RenderPipeline {
        if !self.pipelines.contains_key(&variant) {
            let pipeline = self.create_pipeline(device, shaders, variant);
            self.pipelines.insert(variant, pipeline);
        }
        self.pipelines.get(&variant).unwrap()
    }

    /// Returns an already created pipeline through a shared borrow.
    ///
    /// Lets a pass fetch a pipeline that [`Self::get_or_create`] built outside the pass.
    pub fn get(&self, variant: UIPipelineVariant) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.get(&variant)
    }

    fn create_pipeline(
        &self,
        device: &wgpu::Device,
        shaders: &ShaderModules,
        variant: UIPipelineVariant,
    ) -> wgpu::RenderPipeline {
        // One UI shader module: the variant selects the fragment entry point.
        // Additive blending is independent of the entry point.
        let shader = &shaders.ui;
        let fs_entry = match variant {
            UIPipelineVariant::Textured | UIPipelineVariant::TexturedAdditive => "fs_textured",
            UIPipelineVariant::AlphaText | UIPipelineVariant::AlphaTextAdditive => "fs_alpha_text",
            UIPipelineVariant::Untextured => "fs_untextured",
        };

        let blend_state = match variant {
            UIPipelineVariant::TexturedAdditive | UIPipelineVariant::AlphaTextAdditive => ADDITIVE_BLEND,
            _ => ALPHA_BLEND,
        };

        let label = format!("ui_pipeline_{:?}", variant);

        let vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: 36,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 12,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 20,
                    shader_location: 2,
                },
            ],
        }];

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&label),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                buffers: &vertex_buffers,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs_entry),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.surface_format,
                    blend: Some(blend_state),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            // UI reads depth with CompareFunction::Always. Swapchain-direct passes have no depth attachment.
            depth_stencil: if self.with_depth {
                Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                })
            } else {
                None
            },
            multisample: wgpu::MultisampleState {
                count: self.sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        })
    }

    /// Get pipeline variant for a given texture mode and blend mode
    pub fn variant_for(texture_mode: crate::renderer::gpu::types::UITextureMode, additive: bool) -> UIPipelineVariant {
        use crate::renderer::gpu::types::UITextureMode;
        match (texture_mode, additive) {
            (UITextureMode::None, _) => UIPipelineVariant::Untextured,
            (UITextureMode::AlphaText, false) => UIPipelineVariant::AlphaText,
            (UITextureMode::AlphaText, true) => UIPipelineVariant::AlphaTextAdditive,
            (UITextureMode::Modulate, false) => UIPipelineVariant::Textured,
            (UITextureMode::Modulate, true) => UIPipelineVariant::TexturedAdditive,
        }
    }
}


/// Configuration for creating a 3D render pipeline.
pub struct Pipeline3DConfig<'a> {
    pub label: &'a str,
    pub shader: &'a wgpu::ShaderModule,
    pub layout: &'a wgpu::PipelineLayout,
    pub vertex_buffers: &'a [wgpu::VertexBufferLayout<'a>],
    /// Color target format: always the surface format, which is only known at runtime.
    pub format: wgpu::TextureFormat,
    pub cull_mode: Option<wgpu::Face>,
    pub depth_write: bool,
    pub blend: Option<wgpu::BlendState>,
    pub alpha_to_coverage: bool,
}

/// Create a 3D render pipeline from a configuration.
pub fn create_3d_pipeline(device: &wgpu::Device, config: &Pipeline3DConfig) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(config.label),
        layout: Some(config.layout),
        vertex: wgpu::VertexState {
            module: config.shader,
            entry_point: Some("vs_main"),
            buffers: config.vertex_buffers,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: config.shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: config.format,
                blend: config.blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: config.cull_mode,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: config.depth_write,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            // All 3D pipelines render into the single MSAA target.
            count: crate::renderer::gpu::targets::DEFAULT_MSAA_SAMPLE_COUNT,
            mask: !0,
            alpha_to_coverage_enabled: config.alpha_to_coverage,
        },
        multiview: None,
        cache: None,
    })
}
