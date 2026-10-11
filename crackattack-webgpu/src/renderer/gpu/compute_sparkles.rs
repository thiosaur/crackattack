//! Compute-driven sparkle system.
//!
//! Integrates the sparkle physics on the GPU and produces per-slot
//! `ParticleInstance` records for the particle render pipeline, so the
//! CPU never builds the instance buffer itself.
//!
//! ## Data flow
//!
//! ```text
//!     [dying block detection] (CPU, renderer_runtime)
//!                 │
//!                 ▼
//!     ComputeSparkles::push_spawn(request)
//!                 │  (host-side pending Vec<SparkleSpawnGpu>)
//!                 ▼
//!     ComputeSparkles::dispatch(queue, encoder)
//!        1. queue.write_buffer(spawn_buffer, requests)
//!        2. compute pass "spawn"     (one thread per request)
//!        3. compute pass "integrate" (one thread per slot)
//!                 │
//!                 ▼
//!     gpu_render_buffer  — array<ParticleInstance>, storage
//!                 │
//!                 ▼
//!     particle render pipeline
//!     via `ComputeSparkles::render_bind_group()` which is
//!     layout-compatible with the
//!     `particle_instance_bind_group_layout`.
//! ```
//!
//! ## Slot management
//!
//! The CPU pre-assigns the target slot for each spawn via a
//! simple round-robin counter that wraps at `SPARKLE_SLOT_COUNT`.
//! With 8192 slots and ~10 sparkles per match, a full cycle takes
//! ~800 matches — much longer than the ~3-second lifetime of any
//! single sparkle, so overwrite collisions are extremely rare in
//! practice. When they happen, the new spawn silently overwrites
//! a still-active sparkle. Not a bug, just a tradeoff against the
//! complexity of a proper free-list with atomics.
//!
//! The compute pipeline is the only particle path. Idle detection stops dispatching when no particles
//! have been spawned for longer than the max particle lifetime.

use wgpu::{BindGroup, BindGroupLayout, Buffer, ComputePipeline};

/// Max concurrent sparkles supported by the GPU state buffer.
/// Round-robin slot allocation assumes spawns are sparse enough
/// that a full cycle takes much longer than a sparkle lifetime —
/// at 8192 slots a full cycle takes ~800 matches, trivially longer
/// than the ~180-frame max lifetime. At 48 + 64 = 112 bytes per
/// slot, the total GPU footprint is ~900 KB which is negligible.
pub const SPARKLE_SLOT_COUNT: u32 = 8192;

/// Size in bytes of one GPU `SparkleState` slot.
///
/// There is no Rust mirror of the struct: it is GPU-only and the CPU never reads
/// `state_buffer`. This constant is the only link to the WGSL declaration (64 bytes
/// total), so the two must change together.
const SPARKLE_STATE_SIZE: u64 = 64;

/// Max concurrent spawn requests uploaded per frame. Crack Attack
/// chain clears can emit up to ~40 sparkles in a single frame at
/// the highest combo magnitudes, so 64 is a comfortable ceiling.
pub const SPAWN_RING_SIZE: u32 = 64;

/// Host-side spawn event. Packed to 48 bytes to match the WGSL
/// struct below — the first three fields carry the spawn data,
/// the fourth holds the target slot index that the CPU picked
/// round-robin.
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SparkleSpawnGpu {
    pub position: [f32; 3],
    pub initial_angle: f32,
    pub velocity: [f32; 2],
    pub angular_v: f32,
    pub size: f32,
    pub lifetime: f32,
    pub flavor: u32,
    pub target_slot: u32,
    pub _pad: u32,
}

const _: () = assert!(std::mem::size_of::<SparkleSpawnGpu>() == 48);
// Pin the key offsets: reordering fields would keep the size but break the WGSL layout.
// `velocity: vec2f` is 8-aligned in WGSL, hence offset 16.
const _: () = assert!(std::mem::offset_of!(SparkleSpawnGpu, velocity) == 16);
const _: () = assert!(std::mem::offset_of!(SparkleSpawnGpu, lifetime) == 32);
const _: () = assert!(std::mem::offset_of!(SparkleSpawnGpu, target_slot) == 40);

/// Per-dispatch uniforms — how many live spawn requests are in
/// the ring buffer this frame. 16 bytes (std140-aligned scalar).
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct DispatchParamsGpu {
    spawn_count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

const _: () = assert!(std::mem::size_of::<DispatchParamsGpu>() == 16);

/// Owns the compute infrastructure for sparkle physics.
pub struct ComputeSparkles {
    // GPU buffers
    state_buffer: Buffer,
    spawn_buffer: Buffer,
    params_buffer: Buffer,

    // Pipelines
    spawn_pipeline: ComputePipeline,
    integrate_pipeline: ComputePipeline,

    // Bind groups
    compute_bind_group: BindGroup,
    /// Layout-compatible with `particle_instance_bind_group_layout` so the
    /// particle render pipeline can bind it without modification.
    render_bind_group: BindGroup,

    // Host-side state
    pending_spawns: Vec<SparkleSpawnGpu>,
    next_slot: u32,
    ever_spawned: bool,
    /// Set by clear_all(). Next dispatch() will zero the GPU state buffer.
    needs_gpu_clear: bool,
    /// Frames since the last push_spawn(). Used for idle detection:
    /// if no spawn for longer than the max particle lifetime, all
    /// particles have expired and we can stop dispatching.
    frames_since_last_spawn: u32,
}

impl ComputeSparkles {
    pub fn new(
        device: &wgpu::Device,
        particle_instance_bind_group_layout: &BindGroupLayout,
    ) -> Self {
        // ---- Storage buffers ----
        // Physics state, GPU-owned across frames (`SPARKLE_STATE_SIZE` bytes per slot,
        // see `SparkleState` in the shader). Zeroed (inactive) at creation.
        let state_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rendering::ComputeSparkles::state_buffer"),
            size: (SPARKLE_SLOT_COUNT as u64) * SPARKLE_STATE_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        // Explicit zero-init for determinism — wgpu already zeros
        // mapped buffers on most backends, but make it explicit.
        state_buffer
            .slice(..)
            .get_mapped_range_mut()
            .fill(0);
        state_buffer.unmap();

        // Render instances — ParticleInstance layout, 48 bytes per
        // slot. Written by the integrate kernel each frame, read
        // by the particle render pipeline.
        let render_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rendering::ComputeSparkles::render_buffer"),
            size: (SPARKLE_SLOT_COUNT as u64)
                * std::mem::size_of::<crate::renderer::gpu::instance::ParticleInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        render_buffer
            .slice(..)
            .get_mapped_range_mut()
            .fill(0);
        render_buffer.unmap();

        // Spawn ring — CPU writes pending spawn events here each
        // frame via `queue.write_buffer`.
        let spawn_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rendering::ComputeSparkles::spawn_buffer"),
            size: (SPAWN_RING_SIZE as u64) * std::mem::size_of::<SparkleSpawnGpu>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Small uniform buffer carrying the current spawn count.
        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rendering::ComputeSparkles::params_buffer"),
            size: std::mem::size_of::<DispatchParamsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ---- Compute bind group layout ----
        let compute_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rendering::ComputeSparkles::compute_bgl"),
            entries: &[
                // binding 0: state buffer (read_write)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 1: render buffer (read_write — we overwrite each frame)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 2: spawn buffer (read_only — CPU writes, compute reads)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 3: params uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rendering::ComputeSparkles::compute_bind_group"),
            layout: &compute_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: state_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: render_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: spawn_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rendering::ComputeSparkles::pipeline_layout"),
            bind_group_layouts: &[&compute_bgl],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rendering::ComputeSparkles::shader"),
            source: wgpu::ShaderSource::Wgsl(COMPUTE_SPARKLES_SHADER.into()),
        });

        let spawn_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("rendering::ComputeSparkles::spawn_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_spawn"),
            compilation_options: Default::default(),
            cache: None,
        });

        let integrate_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("rendering::ComputeSparkles::integrate_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_integrate"),
            compilation_options: Default::default(),
            cache: None,
        });

        // ---- Render-side bind group ----
        // Same layout as the particle instance bind group
        // so the existing particle render pipeline can bind this
        // group without any shader or pipeline change.
        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rendering::ComputeSparkles::render_bind_group"),
            layout: particle_instance_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: render_buffer.as_entire_binding(),
            }],
        });

        Self {
            state_buffer,
            spawn_buffer,
            params_buffer,
            spawn_pipeline,
            integrate_pipeline,
            compute_bind_group,
            render_bind_group,
            pending_spawns: Vec::with_capacity(SPAWN_RING_SIZE as usize),
            next_slot: 0,
            ever_spawned: false,
            needs_gpu_clear: false,
            frames_since_last_spawn: 0,
        }
    }

    /// Return the render-side bind group so the particle render
    /// pipeline can bind it at `@group(1)`. Layout-compatible
    /// with the `particle_instance_bind_group`.
    pub fn render_bind_group(&self) -> &BindGroup {
        &self.render_bind_group
    }

    pub fn has_active(&self) -> bool {
        self.ever_spawned
    }

    /// Clear all particles — both pending spawns and active GPU state.
    /// The GPU state buffer is zeroed on the next flush_clear() call.
    pub fn clear_all(&mut self) {
        self.pending_spawns.clear();
        self.next_slot = 0;
        self.ever_spawned = false;
        self.needs_gpu_clear = true;
        self.frames_since_last_spawn = 0;
    }

    /// Enqueues a spawn event. Host-side only: nothing reaches the
    /// GPU until `dispatch()` runs at the end of the frame.
    /// Requests beyond `SPAWN_RING_SIZE` in one frame are dropped.
    pub fn push_spawn(&mut self, req: SparkleSpawnGpu) {
        if self.pending_spawns.len() >= SPAWN_RING_SIZE as usize {
            return;
        }
        self.pending_spawns.push(req);
        self.ever_spawned = true;
        self.frames_since_last_spawn = 0;
    }

    /// Pick the next slot via round-robin. Used by the caller to
    /// fill in `SparkleSpawnGpu::target_slot`.
    pub fn next_slot(&mut self) -> u32 {
        let s = self.next_slot;
        self.next_slot = (self.next_slot + 1) % SPARKLE_SLOT_COUNT;
        s
    }

    /// Flushes a deferred GPU clear if needed. Call each frame before `dispatch`,
    /// on the same encoder, so the clear is ordered before the compute passes.
    /// It uses `clear_buffer` rather than uploading 512 KiB of zeros.
    pub fn flush_clear(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.needs_gpu_clear {
            encoder.clear_buffer(&self.state_buffer, 0, None);
            self.needs_gpu_clear = false;
        }
    }

    /// Upload pending spawn events, run the spawn + integrate
    /// compute passes, and clear the host-side queue. Called once
    /// per frame before particle rendering.
    pub fn dispatch(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder) {
        // 1. Upload pending spawn events.
        let spawn_count = self.pending_spawns.len() as u32;
        if spawn_count > 0 {
            queue.write_buffer(
                &self.spawn_buffer,
                0,
                bytemuck::cast_slice(&self.pending_spawns),
            );
        }
        // 2. Update dispatch params.
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::bytes_of(&DispatchParamsGpu {
                spawn_count,
                _pad0: 0,
                _pad1: 0,
                _pad2: 0,
            }),
        );

        // 3. Run spawn pass (only if we have live events to process).
        if spawn_count > 0 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rendering::ComputeSparkles::spawn_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.spawn_pipeline);
            pass.set_bind_group(0, &self.compute_bind_group, &[]);
            // Workgroup size is 32 in the shader; ceil-div.
            let groups = spawn_count.div_ceil(32);
            pass.dispatch_workgroups(groups, 1, 1);
        }

        // 4. Run integrate pass — one thread per slot, walks the
        //    whole buffer.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rendering::ComputeSparkles::integrate_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.integrate_pipeline);
            pass.set_bind_group(0, &self.compute_bind_group, &[]);
            // Workgroup size is 64 in the shader.
            pass.dispatch_workgroups(SPARKLE_SLOT_COUNT / 64, 1, 1);
        }

        // 5. Clear the host-side queue for next frame.
        self.pending_spawns.clear();

        // 6. Idle detection: if no spawn for longer than the max
        //    particle lifetime, all particles have expired. Stop
        //    dispatching to avoid integrating 8192 empty slots.
        //    Max lifetime in spawns: 130.0 ticks (chain rings).
        const MAX_PARTICLE_LIFETIME: u32 = 150; // generous margin
        self.frames_since_last_spawn += 1;
        if self.frames_since_last_spawn > MAX_PARTICLE_LIFETIME {
            self.ever_spawned = false;
        }
    }

}

/// The compute shader hosts both kernels (`cs_spawn` and
/// `cs_integrate`) plus the shared `SparkleState` struct and
/// physics constants. Kept inline so the physics tuning lives next
/// to the Rust-side spawn logic.
// `pub(crate)` so the WGSL validation test can compile it.
pub(crate) const COMPUTE_SPARKLES_SHADER: &str = r#"
// GPU-resident sparkle state (64 bytes)
// GPU-only: there is no Rust mirror of this struct.
// The ParticleInstance render layout lives in a separate buffer.
struct SparkleState {
    position: vec3f,        // 12
    angle: f32,             //  4
    velocity: vec3f,        // 12   (z always 0, just for alignment)
    angular_v: f32,         //  4
    size: f32,              //  4
    lifetime: f32,          //  4
    // max_lifetime: written by cs_spawn but not read yet (the color phases compare
    // lifetime to the FADE/PULSE constants). Kept for phases that need a life ratio.
    max_lifetime: f32,      //  4
    flavor: u32,            //  4
    is_live: u32,           //  4
    _pad0: u32,             //  4
    _pad1: u32,             //  4
    _pad2: u32,             //  4
}
// 64 bytes total

// Render-side instance layout (48 bytes — matches ParticleInstance)
struct ParticleInstance {
    position: vec3f,       // 12
    angle: f32,            //  4
    color: vec4f,          // 16
    scale_x: f32,          //  4
    scale_y: f32,          //  4
    _pad0: f32,            //  4
    _pad1: f32,            //  4
}

// Spawn request (48 bytes)
struct SpawnRequest {
    position: vec3f,       // 12
    initial_angle: f32,    //  4
    velocity: vec2f,       //  8
    angular_v: f32,        //  4
    size: f32,             //  4
    lifetime: f32,         //  4
    flavor: u32,           //  4
    target_slot: u32,      //  4
    _pad: u32,             //  4
}
// 48 bytes — 44 bytes of fields + one 4-byte _pad

struct DispatchParams {
    spawn_count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read_write> state:  array<SparkleState>;
@group(0) @binding(1) var<storage, read_write> render: array<ParticleInstance>;
@group(0) @binding(2) var<storage, read>       spawns: array<SpawnRequest>;
@group(0) @binding(3) var<uniform>             params: DispatchParams;

// Base block flavor colors, matching the Rust BLOCK_COLORS table.
// Codes 6u..=11u are "nucleons" (flavor = 6 + block_flavor). This function
// returns the nominal blue for them; the integrate kernel recovers the block
// flavor (flavor - 6) from the code to compute their color.
fn flavor_color(flavor: u32) -> vec3f {
    // Matches `game_renderer.rs::BLOCK_COLORS` one-for-one so
    // sparkle and nucleon tints agree with the on-screen blocks.
    switch (flavor) {
        case 0u: { return vec3f(0.73, 0.00, 0.73); } // purple
        case 1u: { return vec3f(0.20, 0.20, 0.80); } // blue
        case 2u: { return vec3f(0.00, 0.60, 0.05); } // green
        case 3u: { return vec3f(0.85, 0.85, 0.00); } // yellow
        case 4u: { return vec3f(1.00, 0.40, 0.00); } // orange
        case 5u: { return vec3f(0.40, 0.40, 0.40); } // gray
        default: { return vec3f(0.30, 0.50, 1.00); } // nucleon — fallback blue
    }
}

// Spawn kernel
//
// One thread per live spawn request. Writes the initial state
// into the slot the CPU pre-assigned (`target_slot`). Also writes
// an initial render instance. The integrate pass, dispatched right
// after on the same encoder, overwrites it, so the write is redundant.
// It guarantees the slot is never observable half-initialized.

@compute @workgroup_size(32)
fn cs_spawn(@builtin(global_invocation_id) gid: vec3u) {
    let req_idx = gid.x;
    if (req_idx >= params.spawn_count) {
        return;
    }
    let req = spawns[req_idx];
    let slot = req.target_slot;
    if (slot >= arrayLength(&state)) {
        return;
    }

    var s: SparkleState;
    s.position = req.position;
    s.angle = req.initial_angle;
    s.velocity = vec3f(req.velocity, 0.0);
    s.angular_v = req.angular_v;
    s.size = req.size;
    s.lifetime = req.lifetime;
    s.max_lifetime = req.lifetime;
    s.flavor = req.flavor;
    s.is_live = 1u;
    s._pad0 = 0u;
    s._pad1 = 0u;
    s._pad2 = 0u;
    state[slot] = s;

    // Initial render instance (see kernel header).
    let base = flavor_color(req.flavor);
    var r: ParticleInstance;
    r.position = req.position;
    r.angle = req.initial_angle;
    r.color = vec4f(base, 1.0);
    // scale = 0.4 * size (GRID_ELEMENT_LENGTH = 1.0 in this coordinate system)
    let scale = 0.4 * req.size;
    r.scale_x = scale;
    r.scale_y = scale;
    r._pad0 = 0.0;
    r._pad1 = 0.0;
    render[slot] = r;
}

// Integrate kernel
//
// One thread per slot. If the slot is active, integrates physics
// (gravity + drag on velocity, position + angle update, lifetime
// decay) and computes the 3-phase render color (fade-out at end of
// life, pulse-to-white mid-life, stable flavor color otherwise).

const GRAVITY: f32 = 0.001;
const DRAG: f32 = 0.001;
const FADE_TIME: f32 = 15.0;
const PULSE_TIME: f32 = 6.0;

@compute @workgroup_size(64)
fn cs_integrate(@builtin(global_invocation_id) gid: vec3u) {
    let slot = gid.x;
    if (slot >= arrayLength(&state)) {
        return;
    }

    var s = state[slot];
    if (s.is_live == 0u) {
        // Inactive slot — mark render instance invisible.
        render[slot].color = vec4f(0.0);
        return;
    }

    // Physics step: gravity drag, position integration, angle update, lifetime decay.
    s.velocity.y = s.velocity.y - (GRAVITY + DRAG * s.velocity.y);
    s.velocity.x = s.velocity.x - DRAG * s.velocity.x;
    s.position.x = s.position.x + s.velocity.x;
    s.position.y = s.position.y + s.velocity.y;
    s.angle = s.angle + s.angular_v;
    s.lifetime = s.lifetime - 1.0;

    if (s.lifetime <= 0.0) {
        s.is_live = 0u;
        state[slot] = s;
        render[slot].color = vec4f(0.0);
        return;
    }

    state[slot] = s;

    // 3-phase color: fade-out at end of life, pulse to white mid-life, stable flavor color.
    // Nucleon flavors (6 and above) get a simpler two-phase ramp (no white
    // pulse) so the hue stays steady, and a lower peak alpha so the
    // particles stay discreet.
    let base = flavor_color(s.flavor);
    var rgb: vec3f;
    var alpha: f32;
    if (s.flavor >= 6u) {
        // Nucleon: the dead block's own flavour, but renormalised
        // to the **luminance of the block-price blue** (≈0.49)
        // so every flavour renders at a comparable brightness
        // and the effect stays visually balanced. Hue preserved,
        // only the brightness shifts.
        let block_flavor = s.flavor - 6u;
        let raw = flavor_color(block_flavor);
        let L_TARGET = 0.49;
        let L = dot(raw, vec3f(0.299, 0.587, 0.114));
        rgb = min(raw * (L_TARGET / max(L, 0.01)), vec3f(1.0));
        // Peak alpha kept low (0.35) so the nucleons stay discreet.
        if (s.lifetime < FADE_TIME) {
            alpha = 0.35 * (s.lifetime / FADE_TIME);
        } else {
            alpha = 0.35;
        }
    } else if (s.lifetime < FADE_TIME) {
        // Fade out.
        rgb = base;
        alpha = s.lifetime / FADE_TIME;
    } else if (s.lifetime < FADE_TIME + PULSE_TIME) {
        // Pulse to white.
        var pulse = (s.lifetime - FADE_TIME) * (2.0 / PULSE_TIME);
        if (pulse > 1.0) { pulse = 2.0 - pulse; }
        rgb = vec3f(pulse) + (1.0 - pulse) * base;
        alpha = 1.0;
    } else {
        // Stable flavor color.
        rgb = base;
        alpha = 1.0;
    }

    var r: ParticleInstance;
    r.position = s.position;
    r.angle = s.angle;
    r.color = vec4f(rgb, alpha);
    let scale = 0.4 * s.size;
    r.scale_x = scale;
    r.scale_y = scale;
    r._pad0 = 0.0;
    r._pad1 = 0.0;
    render[slot] = r;
}
"#;
