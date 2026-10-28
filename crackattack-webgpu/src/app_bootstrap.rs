//! `AppState::new()` and init helper functions.
//!
//! Contains all GPU resource creation (meshes, particles, UI) and
//! the full `AppState` constructor.


use wgpu::util::DeviceExt;

use crate::renderer::scene::transforms::{
    create_transform_uniforms, create_lighting_uniforms,
};
use crate::renderer::scene::meshdata::build_garbage_shell_buffer;
use crate::renderer::gpu::texture::TextureCache;
use crate::renderer::gpu::types::UIVertex;
use crate::renderer::gpu::backend::RenderDumpCursor;
use crate::renderer::gpu::render_command::RenderCommandBuffer;
use crate::renderer::gpu::shaders::ShaderModules;
use crate::renderer::gpu::pipeline::{
    UIPipelineCache, Pipeline3DConfig, create_3d_pipeline, ADDITIVE_BLEND,
};
use crate::renderer::ui::ui_renderer::UIRenderer;
use crate::platform::persist::UserConfig;
use crate::platform::sound::AudioOutput;

use super::{AppState, GarbageFlavorImageState, GARBAGE_FLAVOR_COUNT, BLOCK_SHADER, SessionRuntime};
use crate::renderer_runtime::RendererRuntime;
use crate::renderer::gpu::context::GpuContext;
use crate::renderer::scene::pipelines::ScenePipelines;
use crate::renderer::scene::scene_renderer::SceneRenderer;
use crate::renderer::ui::ui_runtime::UiRuntime;
use crate::renderer::effects::particle_runtime::ParticleRuntime;
use crate::platform::screenshot::ScreenshotCapture;
use crate::scheduling::SchedulingState;
use crate::debug_state::DebugState;
use crate::renderer::init::{
    create_ui_projection, compute_effective_viewport, find_assets_dir,
    load_ui_textures, load_game_textures, build_ui_bind_groups,
};

/// Resources produced by `build_mesh_resources`: block GPU buffers.
struct MeshResources {
    block_vertex_buffer: wgpu::Buffer,
    block_vertex_count: u32,
}

/// Resources produced by `build_particle_resources`: particle pipelines + compute sparkles.
struct ParticleResources {
    particle_pipeline: wgpu::RenderPipeline,
    compute_sparkles: crate::renderer::gpu::compute_sparkles::ComputeSparkles,
    particle_uniform_bind_group_layout: wgpu::BindGroupLayout,
}

/// Resources produced by `build_ui_resources`: 2D overlay infrastructure + egui.
struct UiResources {
    ui_pipeline_cache: UIPipelineCache,
    screen_overlay: crate::renderer::gpu::screen_overlay::ScreenOverlayRenderer,
    screen_ui_pipeline_cache: UIPipelineCache,
    screen_ui_vertex_buffer: wgpu::Buffer,
    screen_ui_index_buffer: wgpu::Buffer,
    ui_uniform_buffer: wgpu::Buffer,
    ui_bind_group_layout: wgpu::BindGroupLayout,
    ui_vertex_buffer: wgpu::Buffer,
    ui_index_buffer: wgpu::Buffer,
    ui_sampler: wgpu::Sampler,
    ui_renderer: UIRenderer,
    egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
}

impl<P: crate::platform_shell::PlatformShell> AppState<P> {
    pub(crate) fn new(
        platform: P,
        audio_output: Option<AudioOutput>,
        boot: crate::platform::boot_config::BootConfig,
    ) -> Self {
        // Load user config (player name, high scores)
        let mut user_config = UserConfig::load();
        // Apply the `--set config:…` overrides before anything reads the config
        // (log below, nameplate, session), so the effective value is seen everywhere.
        // `save()` strips the overrides, so they are never written to disk.
        user_config.apply_cli_set_overrides(&boot.set_overrides);
        log::info!("Player: {}, highest score: {}", user_config.player_name, user_config.highest_score());
        // Extract values needed before user_config moves into SessionRuntime.
        let player_name = user_config.player_name.clone();
        let key_bindings_config = user_config.key_bindings.clone();
        let _highest_score = user_config.highest_score();

        // The platform layer (SDL3) creates the surface and drives egui input
        // (`ingest`/`poll_events`). There is no direct window dependency here.
        let mut gpu = GpuContext::new(|inst| platform.create_surface(inst), platform.pixel_size());
        let device = &gpu.device;
        let queue = &gpu.queue;
        let config = &gpu.config;
        let surface_format = config.format;

        // Create MSAA color + depth render targets (scene pass).
        // Managed as a single bundle via renderer::gpu::targets::RenderTargets.
        let render_targets = crate::renderer::gpu::targets::RenderTargets::new(
            device,
            (config.width, config.height),
            crate::renderer::gpu::targets::DEFAULT_MSAA_SAMPLE_COUNT,
            config.format,
            crate::renderer::gpu::targets::DEFAULT_DEPTH_FORMAT,
        );

        // Create shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("block_shader"),
            source: wgpu::ShaderSource::Wgsl(BLOCK_SHADER.into()),
        });

        // Uniform buffers: @group(0) bindings 0–2

        // Create transform uniform buffer (binding 0)
        let transform_uniforms = create_transform_uniforms(config.width, config.height);
        log::trace!("MVP matrix: {:?}", transform_uniforms.mvp);
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("transform_uniform_buffer"),
            contents: bytemuck::bytes_of(&transform_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create lighting uniform buffer (binding 1)
        let lighting_uniforms = create_lighting_uniforms();
        let lighting_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lighting_uniform_buffer"),
            contents: bytemuck::bytes_of(&lighting_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Material ring buffer — blocks use dynamic offsets.
        // Budget: 1 DrawInstanced per frame typical,
        // 16 slots gives headroom for future draw variants.
        use crate::renderer::gpu::ring_buffer::UniformRingBuffer;
        let material_ring = UniformRingBuffer::with_default_alignment(&device, 16, "material_ring_buffer");

        // Dummy texture: magenta/black checkerboard in dev (makes missing textures
        // immediately visible), white 1x1 in release (safe fallback).
        #[cfg(feature = "dev")]
        let (dummy_w, dummy_h, dummy_pixels) = (2u32, 2u32, vec![
            255u8, 0, 255, 255,   0, 0, 0, 255,  // magenta, black
            0,     0,   0, 255, 255, 0, 255, 255,  // black, magenta
        ]);
        #[cfg(not(feature = "dev"))]
        let (dummy_w, dummy_h, dummy_pixels) = (1u32, 1u32, vec![255u8, 255, 255, 255]);
        let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy_texture"),
            size: wgpu::Extent3d { width: dummy_w, height: dummy_h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &dummy_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &dummy_pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dummy_w * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d { width: dummy_w, height: dummy_h, depth_or_array_layers: 1 },
        );
        let dummy_texture_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create sampler for textures
        let block_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("block_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Used by the block pipeline and reused by the level lights.
        // Bindings 0-1 are transform + lighting, as declared by the shaders.
        let frame_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame_bind_group_layout"),
            entries: &[
                // binding 0: TransformUniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // binding 1: LightingUniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });

        // Block @group(2): the material UBO only, with a dynamic offset.
        let material_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(96), // sizeof(MaterialUniformsGPU)
                    },
                    count: None,
                },
            ],
        });

        let mut texture_cache = TextureCache::new();

        // Built once, shared by all block/swapper draws in a frame.
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame_bind_group"),
            layout: &frame_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: lighting_buffer.as_entire_binding() },
            ],
        });

        // Block @group(2): the material ring only (dynamic offset).
        let block_material_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("block_material_bind_group"),
            layout: &material_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: material_ring.buffer(),
                    offset: 0,
                    size: std::num::NonZeroU64::new(96), // sizeof(MaterialUniformsGPU)
                }) },
            ],
        });

        // Create instance buffer (storage buffer for block instances)
        let max_instances = 256;
        use crate::renderer::gpu::instance::BlockInstance;
        let instance_buffer_size = max_instances * std::mem::size_of::<BlockInstance>();
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: instance_buffer_size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create instance bind group layout
        let instance_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instance_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        // Create instance bind group
        let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("instance_bind_group"),
            layout: &instance_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: instance_buffer.as_entire_binding(),
            }],
        });

        // Create pipeline layout
        // Block pipeline uses the separated bind group layout:
        //   @group(0) = frame uniforms (transform + lighting)
        //   @group(1) = instance storage buffer
        //   @group(2) = material UBO only (dynamic offset)
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("block_pipeline_layout"),
            bind_group_layouts: &[
                &frame_bind_group_layout,
                &instance_bind_group_layout,
                &material_bind_group_layout,
            ],
            push_constant_ranges: &[],
        });

        // Create block pipeline (LDR — direct swapchain path)
        let block_pipeline = create_3d_pipeline(device, &Pipeline3DConfig {
            label: "block_pipeline",
            shader: &shader,
            layout: &pipeline_layout,
            vertex_buffers: &[crate::renderer::gpu::types::MeshVertex::desc()],
            format: surface_format,
            cull_mode: Some(wgpu::Face::Back),
            depth_write: true,
            blend: None,
            alpha_to_coverage: false,
        });

        // Block mesh GPU resources
        let mesh_res = build_mesh_resources(device);
        let MeshResources {
            block_vertex_buffer, block_vertex_count,
        } = mesh_res;

        // Create all shader modules (from shaders.rs)
        let shader_modules = ShaderModules::new(device);
        log::debug!("Created shader modules");

        // Level Light Pipeline (3D lit, emission mode)

        // Reuses the block frame layout (transform + lighting), which already matches
        // exactly what the shader declares.
        let level_light_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("level_light_pipeline_layout"),
            bind_group_layouts: &[&frame_bind_group_layout],
            push_constant_ranges: &[],
        });

        let level_light_pipeline = create_3d_pipeline(device, &Pipeline3DConfig {
            label: "level_light_pipeline",
            shader: &shader_modules.level_light_emission,
            layout: &level_light_pipeline_layout,
            vertex_buffers: &[crate::renderer::gpu::types::MeshVertex::desc()],
            format: surface_format,
            cull_mode: None,
            depth_write: true,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            alpha_to_coverage: false,
        });

        // Level light vertex buffer, sized for `LEVEL_LIGHT_MAX_VERTICES`
        let ll_max_bytes = crate::renderer::ui::constants::LEVEL_LIGHT_MAX_VERTICES * std::mem::size_of::<crate::renderer::gpu::types::MeshVertex>();
        let level_light_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("level_light_vertex_buffer"),
            size: ll_max_bytes as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Particle pipelines and compute sparkles
        let particle_res = build_particle_resources(device, surface_format, &shader_modules);
        let ParticleResources {
            particle_pipeline,
            compute_sparkles, particle_uniform_bind_group_layout,
        } = particle_res;

        // UI overlay infrastructure and egui
        let ui_res = build_ui_resources(device, surface_format, config.width, config.height);
        let UiResources {
            ui_pipeline_cache, screen_overlay,
            screen_ui_pipeline_cache, screen_ui_vertex_buffer, screen_ui_index_buffer,
            ui_uniform_buffer, ui_bind_group_layout,
            ui_vertex_buffer, ui_index_buffer, ui_sampler,
            ui_renderer, egui_ctx, egui_renderer: egui_render,
        } = ui_res;

        // Load UI and game textures
        let data_dir = find_assets_dir();
        let ui_dir = data_dir.join("textures/ui");
        let fonts_dir = data_dir.join("textures/fonts");
        let garbage_dir = data_dir.join("textures/garbage");
        log::info!("Loading assets from: {}", data_dir.display());

        load_ui_textures(device, queue, &ui_dir, &fonts_dir, &player_name, &mut texture_cache);
        load_game_textures(device, queue, &garbage_dir, &mut texture_cache);

        let garbage_texture_view = texture_cache.get_view("garbage_flavor")
            .unwrap_or(&dummy_texture_view); // fallback to dummy if not found
        let sparkle_texture_view = texture_cache.get_view("sparkle")
            .unwrap_or(&dummy_texture_view); // fallback to dummy if not found
        log::debug!("Garbage and sparkle textures loaded");

        // Particle uniform bind group (with sparkle texture)
        let particle_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle_uniform_bind_group"),
            layout: &particle_uniform_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(sparkle_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&block_sampler),
                },
            ],
        });

        // Uses INSTANCED_GARBAGE shader with per-instance UV transform and clip plane

        // Garbage instance buffer (GarbagePieceInstance is 64 bytes)
        let max_garbage_instances = 256; // Max garbage cells per frame (doubled for background planes)
        let garbage_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("garbage_instance_buffer"),
            size: (max_garbage_instances * 64) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Garbage instance bind group (same layout as block instances)
        let garbage_instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("garbage_instance_bind_group"),
            layout: &instance_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: garbage_instance_buffer.as_entire_binding(),
            }],
        });

        // Dedicated garbage layout: transform + lighting + flavor texture + sampler.
        // Bindings are 0, 1, 3 and 4 (binding 2 is unused), matching the garbage shader.
        let garbage_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("garbage_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });

        // Garbage uniform bind group (garbage flavor texture at binding 3)
        let garbage_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("garbage_uniform_bind_group"),
            layout: &garbage_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: lighting_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(garbage_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&block_sampler),
                },
            ],
        });

        // Per-flavor bind groups: same layout as garbage_uniform but with individual flavor texture at binding 3
        let garbage_flavor_bind_groups: Vec<wgpu::BindGroup> = (0..GARBAGE_FLAVOR_COUNT)
            .filter_map(|i| {
                let name = format!("garbage_flavor_{}", i);
                texture_cache.get_view(&name).map(|flavor_view| {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(&format!("garbage_flavor_bind_group_{}", i)),
                        layout: &garbage_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: lighting_buffer.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(flavor_view) },
                            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&block_sampler) },
                        ],
                    })
                })
            })
            .collect();
        log::debug!("Created {} per-flavor garbage bind groups", garbage_flavor_bind_groups.len());

        log::trace!("Bind group texture map:");

        log::trace!("  garbage_uniform_bind_group → @group(0) binding 3: garbage_flavor");

        // Garbage shader (from shaders.rs INSTANCED_GARBAGE)
        let garbage_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("garbage_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                crate::renderer::gpu::shaders::INSTANCED_GARBAGE
            )),
        });

        // Garbage pipeline layout: dedicated @group(0) layout.
        let garbage_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("garbage_pipeline_layout"),
            bind_group_layouts: &[
                &garbage_bind_group_layout,
                &instance_bind_group_layout,
            ],
            push_constant_ranges: &[],
        });

        // Garbage pipeline (cull_mode: None for mirrored shells)
        let garbage_pipeline = create_3d_pipeline(device, &Pipeline3DConfig {
            label: "garbage_pipeline",
            shader: &garbage_shader,
            layout: &garbage_pipeline_layout,
            vertex_buffers: &[crate::renderer::gpu::types::MeshVertex::desc()],
            format: surface_format,
            cull_mode: None,
            depth_write: true,
            blend: Some(wgpu::BlendState::REPLACE),
            alpha_to_coverage: true,
        });

        log::debug!("Created garbage pipeline with INSTANCED_GARBAGE shader");

        // Create garbage shell vertex buffer (6 beveled meshes concatenated)
        let (garbage_shell_vertices, garbage_shell_ranges) = build_garbage_shell_buffer();
        let garbage_shell_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("garbage_shell_vertex_buffer"),
            contents: bytemuck::cast_slice(&garbage_shell_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        log::debug!(
            "Created garbage shell vertex buffer: {} vertices ({} bytes), ranges: corner={}, edge={}, middle={}, thin_cap={}, thin_mid={}, small={}",
            garbage_shell_vertices.len(),
            garbage_shell_vertices.len() * std::mem::size_of::<crate::renderer::gpu::types::MeshVertex>(),
            garbage_shell_ranges.thick_corner.vertex_count,
            garbage_shell_ranges.thick_edge.vertex_count,
            garbage_shell_ranges.thick_middle.vertex_count,
            garbage_shell_ranges.thin_cap.vertex_count,
            garbage_shell_ranges.thin_middle.vertex_count,
            garbage_shell_ranges.small.vertex_count,
        );

        // Unified dummy texture for UI fallback (same as 3D dummy: magenta/black in dev, white in release)
        #[cfg(feature = "dev")]
        let (ui_dummy_w, ui_dummy_h, ui_dummy_data) = (2u32, 2u32, vec![
            255u8, 0, 255, 255,   0, 0, 0, 255,
            0,     0,   0, 255, 255, 0, 255, 255,
        ]);
        #[cfg(not(feature = "dev"))]
        let (ui_dummy_w, ui_dummy_h, ui_dummy_data) = (1u32, 1u32, vec![255u8, 255, 255, 255]);
        texture_cache.load_raw(device, queue, "_dummy", ui_dummy_w, ui_dummy_h, &ui_dummy_data).unwrap();

        // Create UI bind groups (combined: MVP uniform + texture + sampler)
        // This matches the UI shader layout in shaders.rs.
        let dummy_view = texture_cache.get_view("_dummy").unwrap();
        let solid_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("solid_bind_group"),
            layout: &ui_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ui_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(dummy_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&ui_sampler),
                },
            ],
        });

        let ui_bind_groups = build_ui_bind_groups(
            device, &ui_bind_group_layout, &ui_uniform_buffer, &ui_sampler, &texture_cache,
        );

        log::trace!("UI bind groups created: {:?}", ui_bind_groups.keys().collect::<Vec<_>>());

        // Compute initial effective viewport
        {
            let (vx, vy, vw, vh) = compute_effective_viewport(gpu.config.width, gpu.config.height);
            gpu.viewport_x = vx;
            gpu.viewport_y = vy;
            gpu.viewport_width = vw;
            gpu.viewport_height = vh;
            log::info!("Effective viewport: {}x{} at ({}, {})", vw, vh, vx, vy);
        }

        Self {
            platform,
            gpu,
            renderer: RendererRuntime {
            scene: SceneRenderer {
                pipelines: ScenePipelines {
                    render_targets,
                    block_pipeline,
                    block_vertex_buffer,
                    block_vertex_count,
                    instance_buffer,
                    instance_bind_group,
                    block_material_bind_group,
                    garbage_pipeline,
                    garbage_shell_vertex_buffer,
                    garbage_shell_ranges,
                    garbage_instance_buffer,
                    garbage_instance_bind_group,
                    garbage_uniform_bind_group,
                    garbage_flavor_bind_groups,
                    level_light_pipeline,
                    level_light_vertex_buffer,
                    frame_bind_group,
                    uniform_buffer,
                    transform_uniforms,
                    lighting_buffer,
                    shader_modules,
                },
                dump_cursor: RenderDumpCursor::new(),
                command_buffer: RenderCommandBuffer::new(),
                material_ring,
                garbage_flavor_state: GarbageFlavorImageState::default(),
                garbage_flavor_instance_idx: None,
                block_impact_phases: std::collections::HashMap::new(),
                danger_bar_animator: crate::renderer::effects::game_animators::DangerBarAnimator::new(),
                level_light_animator: crate::renderer::effects::game_animators::LevelLightAnimator::new(),
                opponent_board_animator: crate::renderer::effects::game_animators::OpponentBoardAnimator::new(),
                logo_shake_animator: crate::renderer::effects::game_animators::LogoShakeAnimator::new(),
                menu_backdrop: crate::renderer::effects::menu_backdrop::MenuBackdrop::new(0x5eed_b10c),
            },
            ui: UiRuntime {
                ui_pipeline_cache,
                screen_overlay,
                screen_ui_pipeline_cache,
                screen_ui_vertex_buffer,
                screen_ui_index_buffer,
                screen_result_batch: None,
                ui_uniform_buffer,
                ui_bind_group_layout,
                ui_vertex_buffer,
                ui_index_buffer,
                ui_sampler,
                texture_cache,
                ui_renderer,
                ui_bind_groups,
                solid_bind_group,
                egui_ctx,
                egui_renderer: egui_render,
                pending_raw_input: None,
                focused: true,
                pending_egui_outputs: None,
            },
            particles: ParticleRuntime {
                compute_sparkles,
                particle_uniform_bind_group,
                particle_pipeline,
            },
            screenshot: ScreenshotCapture::new(boot.capture_dir.clone()),
            render_groups: crate::app::render_groups::RenderGroups::new(),
            }, // RendererRuntime
            scheduling: {
                let mut sched = SchedulingState::new();
                sched.fixed_step = boot.fixed_step;
                sched
            },
            runtime: SessionRuntime::new(
                super::session_runtime::SessionConfig {
                    seed: {
                        // An explicit seed is `Some(_)`; `Some(0)` is valid (`--seed 0`).
                        match boot.seed {
                            Some(seed) => seed as u32,
                            None => std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs() as u32,
                        }
                    },
                    autoplay: boot.autoplay,
                    autoplay_ai: boot.autoplay_ai,
                    autoplay_max_rounds: {
                        if boot.autoplay_rounds > 0 { Some(boot.autoplay_rounds) } else { None }
                    },
                    vs_mode: boot.vs_computer,
                    ai_difficulty: boot.ai_difficulty,
                    network: boot.network.clone(),
                    open_targets: boot.open_targets.clone(),
                    set_overrides: boot.set_overrides.clone(),
                    sounds_dir: boot.sounds_dir.as_ref().map(std::path::PathBuf::from),
                    music_dir: boot.music_dir.as_ref().map(std::path::PathBuf::from),
                    rungs_dir: boot.rungs_dir.as_ref().map(std::path::PathBuf::from),
                    music_debug: boot.music_debug,
                },
                user_config, audio_output, &data_dir,
            ),
            debug: {
                let mut debug = DebugState::new();
                debug.script_path = boot.script.clone();
                debug.max_frames = boot.max_frames.unwrap_or(0);
                debug.explicit_seed = boot.seed;
                debug
            },
            cached_has_modal: false,
            cached_grid_cleared: false,
            key_bindings: crate::keycodes::CompiledBindings::compile(&key_bindings_config),
            occluded: false,
            text_input_on: false,
            // Test miniscript: load and parse it if `--script` or `session:qa.script` is given.
            // An error is fatal (a broken script makes the test invalid). It fails cleanly
            // like any user input error: message on stderr and exit code 1, no panic.
            script: boot.script.clone().map(|path| {
                std::fs::read_to_string(&path)
                    .map_err(|e| format!("unreadable: {e}"))
                    .and_then(|src| crate::script::ScriptRunner::parse(&src)
                        .map_err(|e| format!("invalid: {e}")))
                    .unwrap_or_else(|e| {
                        eprintln!("Error: miniscript '{path}' {e}");
                        std::process::exit(1);
                    })
            }),
            // Recorder: captures the seed and the effective command line for the `.mini`
            // header (a replay needs the same start-up options, so they are recorded).
            recorder: boot.record.clone().map(|path| {
                // `.mini` header: the explicit seed, or 0 if none. A recording made
                // without a seed cannot be replayed identically anyway; fixed step alone
                // is not enough.
                let seed = boot.seed.unwrap_or(0);
                // Minimal POSIX shell quoting. The "launched with" header is only a
                // diagnostic: it can be replayed as is under sh/zsh, but needs adapting
                // under cmd/PowerShell (Windows quoting differs).
                let cmdline = std::env::args().skip(1)
                    .map(|a| if a.contains(|c: char| c.is_whitespace() || c == '"') {
                        format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\""))
                    } else { a })
                    .collect::<Vec<_>>().join(" ");
                crate::script::Recorder::new(path, seed, cmdline)
            }),
        }
    }
}

/// Build the block mesh GPU resources (beveled cube, 504 vertices).
fn build_mesh_resources(
    device: &wgpu::Device,
) -> MeshResources {
    use crate::renderer::scene::beveled_block::BEVELED_BLOCK_VERTICES;
    let block_vertex_count = BEVELED_BLOCK_VERTICES.len() as u32;

    let block_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("block_vertex_buffer"),
        contents: bytemuck::cast_slice(&BEVELED_BLOCK_VERTICES),
        usage: wgpu::BufferUsages::VERTEX,
    });

    MeshResources {
        block_vertex_buffer,
        block_vertex_count,
    }
}

/// Build particle render pipelines and compute sparkle system.
fn build_particle_resources(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    shader_modules: &ShaderModules,
) -> ParticleResources {
    // Particle uniform bind group layout (transform + texture + sampler)
    let particle_uniform_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle_uniform_bind_group_layout"),
            entries: &[
                // Transform uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Particle texture
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Particle sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    // Particle instance bind group layout (storage buffer)
    let particle_instance_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle_instance_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

    // Particle pipeline layout
    let particle_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle_pipeline_layout"),
            bind_group_layouts: &[
                &particle_uniform_bind_group_layout,
                &particle_instance_bind_group_layout,
            ],
            push_constant_ranges: &[],
        });

    // Particle pipeline — additive blending (src*srcAlpha + dst*1)
    // Used for compute sparkles.
    let particle_pipeline = create_3d_pipeline(device, &Pipeline3DConfig {
        label: "particle_pipeline",
        shader: &shader_modules.instanced_particle,
        layout: &particle_pipeline_layout,
        vertex_buffers: &[],
        format: surface_format,
        cull_mode: None,
        depth_write: false,
        blend: Some(ADDITIVE_BLEND),
        alpha_to_coverage: false,
    });

    // Compute-driven sparkle system
    let compute_sparkles = crate::renderer::gpu::compute_sparkles::ComputeSparkles::new(
        device,
        &particle_instance_bind_group_layout,
    );

    log::debug!("Created particle pipeline, sparkle system, and mote system");

    ParticleResources {
        particle_pipeline,
        compute_sparkles,
        particle_uniform_bind_group_layout,
    }
}

/// Build 2D UI overlay infrastructure and egui context.
fn build_ui_resources(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    config_width: u32,
    config_height: u32,
) -> UiResources {
    // Create UI uniform buffer (orthographic projection)
    let ui_uniforms = create_ui_projection(config_width, config_height);
    let ui_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ui_uniform_buffer"),
        contents: bytemuck::bytes_of(&ui_uniforms),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    // Create sampler for UI textures
    let ui_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("ui_sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    // Combined UI bind group layout (matching shaders.rs UI shader):
    // binding 0: MVP uniform
    // binding 1: texture
    // binding 2: sampler
    let ui_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ui_bind_group_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    // Scene-pass UI cache: MSAA + with depth.
    let ui_pipeline_cache = UIPipelineCache::new(
        device,
        &ui_bind_group_layout,
        surface_format,
    );
    log::debug!("Created UI pipeline cache (scene-pass MSAA)");

    // Screen-surface overlay pass: a small dedicated clip-space pipeline.
    let screen_overlay =
        crate::renderer::gpu::screen_overlay::ScreenOverlayRenderer::new(device, surface_format);

    // Swapchain UI pipeline (single-sample, no depth) plus small dedicated buffers,
    // used to redraw the result message above the curtain in the overlay pass.
    let screen_ui_pipeline_cache =
        UIPipelineCache::for_swapchain_no_depth(device, &ui_bind_group_layout, surface_format);
    let screen_ui_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screen_ui_vertex_buffer"),
        size: 256 * std::mem::size_of::<UIVertex>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let screen_ui_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screen_ui_index_buffer"),
        size: 512 * std::mem::size_of::<u16>() as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Consolidated UI buffers have a fixed capacity and are never resized. Filling
    // is guarded against overflow in `renderer_runtime::dispatch_render_commands`.
    let ui_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui_vertex_buffer"),
        size: (crate::renderer_runtime::UI_VERTEX_CAPACITY * std::mem::size_of::<UIVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let ui_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui_index_buffer"),
        size: (crate::renderer_runtime::UI_INDEX_CAPACITY * std::mem::size_of::<u16>()) as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let ui_renderer = UIRenderer::new();

    // egui overlay
    let egui_ctx = egui::Context::default();
    // Lucide icon font (family "lucide", ISC license); see `renderer::ui::icons`.
    crate::renderer::ui::icons::install(&egui_ctx);
    let egui_renderer = egui_wgpu::Renderer::new(device, surface_format, None, 1, false);

    UiResources {
        ui_pipeline_cache,
        screen_overlay,
        screen_ui_pipeline_cache,
        screen_ui_vertex_buffer,
        screen_ui_index_buffer,
        ui_uniform_buffer,
        ui_bind_group_layout,
        ui_vertex_buffer,
        ui_index_buffer,
        ui_sampler,
        ui_renderer,
        egui_ctx,
        egui_renderer,
    }
}
