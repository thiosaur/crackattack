//! WGSL shaders for Crack Attack! renderer
//!
//! This module contains all WGSL shaders as inline strings.

// Shared uniform struct definitions (for reference)
//
// TransformUniforms (352 bytes, binding 0, DYNAMIC OFFSET):
//   - mvp: mat4x4f
//   - modelView: mat4x4f
//   - normalMatrix: mat4x4f
//   - texMatrix: mat4x4f
//   - texMatrix1: mat4x4f
//   - clipPlane: vec4f
//   - clipEnabled: vec4f
//
// LightingUniforms (96 bytes, binding 1, NOT dynamic) and
// MaterialUniforms (96 bytes, binding 2, DYNAMIC OFFSET):
// the Rust side is the layout authority (structs and compile-time
// size_of/offset_of asserts in uniforms.rs). Every shader below declares
// the same offsets.


/// Level Light Emission shader - per-vertex emission color via UV attribute
///
/// Same lighting as LIT_UNTEXTURED emission mode, but reads emission color
/// from the UV vertex attribute (rgb stored in uv.xyz) instead of a uniform.
/// This allows all 24 level lights to be drawn in a single call with
/// different colors per triangle.
///
/// The material is made of constants (specular 0.25, shininess 2.0). To make
/// it tunable, add a small dedicated uniform.
pub const LEVEL_LIGHT_EMISSION: &str = r#"
// Level Light Emission shader - per-vertex color via UV

struct TransformUniforms {
    mvp: mat4x4f,
    modelView: mat4x4f,
    normalMatrix: mat4x4f,
    texMatrix: mat4x4f,
    texMatrix1: mat4x4f,
    clipPlane: vec4f,
    clipEnabled: vec4f,
}

struct LightingUniforms {
    ambient: vec4f,
    diffuse: vec4f,
    specular: vec4f,
    lightPos: vec4f,
    shininess: f32,
    panicIntensity: f32,
    gameTime: f32,
    garbageWarm: f32,
    // dyingZone is not read here. Every WGSL copy of this struct declares it so
    // that they all describe the same 96-byte buffer (see uniforms.rs).
    dyingZone: vec4f,
}

// The material never changes, so it is a set of constants.
const SPECULAR: vec3f = vec3f(0.25, 0.25, 0.25);
const SHININESS: f32 = 2.0;

// @group(0) reuses the blocks' frame bind group (transform + lighting).
// There is no material UBO and no texture.
@group(0) @binding(0) var<uniform> transform: TransformUniforms;
@group(0) @binding(1) var<uniform> lighting: LightingUniforms;

struct VertexInput {
    @location(0) position: vec3f,
    @location(1) normal: vec3f,
    @location(2) uv: vec3f,
    @location(3) uvDiffuse: vec2f,
}

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) normalEye: vec3f,
    @location(1) posEye: vec3f,
    @location(2) emissionColor: vec3f,
    @location(4) alpha: f32,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let pos4 = vec4f(in.position, 1.0);
    let eyePos = transform.modelView * pos4;
    out.posEye = eyePos.xyz;
    out.normalEye = normalize((transform.normalMatrix * vec4f(in.normal, 0.0)).xyz);
    out.position = transform.mvp * pos4;
    out.emissionColor = in.uv;
    out.alpha = in.uvDiffuse.x; // alpha passed via uvDiffuse.x
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    let N = normalize(in.normalEye);

    var L: vec3f;
    if (lighting.lightPos.w > 0.5) {
        L = normalize(lighting.lightPos.xyz - in.posEye);
    } else {
        L = normalize(lighting.lightPos.xyz);
    }

    let V = normalize(-in.posEye);
    let H = normalize(L + V);

    let NdotH = max(dot(N, H), 0.0);
    let specPower = pow(NdotH, SHININESS);

    let specularContrib = SPECULAR * specPower;
    let finalColor = in.emissionColor + specularContrib;

    return vec4f(finalColor, in.alpha);
}
"#;

/// 2D UI module: one `vs_main` and three fragment entry points.
///
/// - `fs_textured`: RGBA texture modulated by the vertex color (score, messages, HUD).
/// - `fs_alpha_text`: alpha-only textures (text). The RGB comes from the tint and
///   the alpha from the texture.
/// - `fs_untextured`: plain color. The texture bindings exist in the module but
///   this entry point does not read them. The shared layout provides them.
pub const UI_COMBINED: &str = r#"
// UI 2D combined module: one vertex shader, three fragment entry points

struct Uniforms {
    mvp: mat4x4f,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var uiTexture: texture_2d<f32>;
@group(0) @binding(2) var uiSampler: sampler;

struct VertexInput {
    @location(0) position: vec3f,
    @location(1) uv: vec2f,
    @location(2) color: vec4f,
}

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = uniforms.mvp * vec4f(in.position, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_textured(in: VertexOutput) -> @location(0) vec4f {
    let tex = textureSample(uiTexture, uiSampler, in.uv);
    return tex * in.color;
}

@fragment
fn fs_alpha_text(in: VertexOutput) -> @location(0) vec4f {
    let texAlpha = textureSample(uiTexture, uiSampler, in.uv).a;
    return vec4f(in.color.rgb, in.color.a * texAlpha);
}

@fragment
fn fs_untextured(in: VertexOutput) -> @location(0) vec4f {
    return in.color;
}
"#;

/// Instanced garbage shader - texture flavor, UV transform, organic clip plane
///
/// Usage: Garbage pieces with various mesh types
pub const INSTANCED_GARBAGE: &str = r#"
// Instanced garbage shader — texture flavor, UV transform, clip plane

struct TransformUniforms {
    mvp: mat4x4f,
    modelView: mat4x4f,
    normalMatrix: mat4x4f,
    texMatrix: mat4x4f,
    texMatrix1: mat4x4f,
    clipPlane: vec4f,
    clipEnabled: vec4f,
}

struct LightingUniforms {
    ambient: vec4f,
    diffuse: vec4f,
    specular: vec4f,
    lightPos: vec4f,
    shininess: f32,
    panicIntensity: f32,
    gameTime: f32,
    garbageWarm: f32,
    // dyingZone is not read here. Every WGSL copy of this struct declares it so
    // that they all describe the same 96-byte buffer (see uniforms.rs).
    dyingZone: vec4f,
}

// GarbagePieceInstance: 64 bytes
struct GarbagePieceInstance {
    position: vec3f,      // 12 bytes
    flags: u32,           // 4 bytes (bit 0 = clipEnabled)
    color: vec4f,         // 16 bytes
    uvTransform: vec4f,   // 16 bytes [scaleX, scaleY, offsetX, offsetY]
    clipPlaneEye: vec4f,  // 16 bytes
}

// Dedicated garbage layout. Bindings 2, 5 and 6 are unused. The remaining
// binding numbers are kept as they are.
@group(0) @binding(0) var<uniform> transform: TransformUniforms;
@group(0) @binding(1) var<uniform> lighting: LightingUniforms;
@group(0) @binding(3) var diffuseTexture: texture_2d<f32>;
@group(0) @binding(4) var diffuseSampler: sampler;

@group(1) @binding(0) var<storage, read> instances: array<GarbagePieceInstance>;

struct VertexInput {
    @builtin(instance_index) instanceIdx: u32,
    @location(0) position: vec3f,
    @location(1) normal: vec3f,
    @location(2) uv: vec3f,
    // location(3), the `MeshVertex` uvDiffuse, is not consumed by this shader.
    // Reading only part of the vertex layout is allowed.
}

// Varying budget: 12 used out of 16 (the default WebGPU limit on the highest
// location). The locations are sparse. Do not renumber them and do not exceed 15.
// Everything that is constant per instance uses @interpolate(flat). The provoking
// vertex value is taken as is, which saves interpolating three identical copies.
struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
    @location(2) normalEye: vec3f,
    @location(3) posEye: vec3f,
    @location(4) @interpolate(flat) color: vec4f,
    @location(5) @interpolate(flat) clipPlane: vec4f,
    @location(6) @interpolate(flat) clipEnabled: f32,
    @location(7) @interpolate(flat) hasFlavor: f32,
    @location(8) worldPos: vec3f,  // Garbage smooth shatter
    @location(9) localPos: vec3f,  // Garbage-local coords (tint and depth gradients)
    @location(10) @interpolate(flat) cellPhase: f32,  // phase (seed * 2π), computed in the vertex stage
    @location(12) @interpolate(flat) cellInfo: vec4f, // cellX, cellY, garbW, garbH from flags
    @location(13) @interpolate(flat) flavorIdx: f32, // 0=red, 1=gray — matches block flavorIdx convention
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let inst = instances[in.instanceIdx];

    // Flag bits:
    //   0: clipEnabled
    //   1: hasFlavor
    //   2: isBackground (flatten to plane)
    //   3: mirrorX (scale -1 on local X)
    //   4: mirrorY (scale -1 on local Y)
    //   5: mirrorZ (scale -1 on local Z)
    //   6: swapXY (swap x and y, negate z; xy_swap_matrix)
    let isBackground = (inst.flags & 4u) != 0u;
    var localPos = in.position;
    var normal = in.normal;
    var localUv = in.uv;

    // Apply mirror/swap transforms to vertex, normal, and UV
    if ((inst.flags & 8u) != 0u) {
        localPos.x = -localPos.x;
        normal.x = -normal.x;
        localUv.x = -localUv.x;
    }
    if ((inst.flags & 16u) != 0u) {
        localPos.y = -localPos.y;
        normal.y = -normal.y;
        localUv.y = -localUv.y;
    }
    if ((inst.flags & 32u) != 0u) {
        localPos.z = -localPos.z;
        normal.z = -normal.z;
        localUv.z = -localUv.z;
    }
    if ((inst.flags & 64u) != 0u) {
        // swapXY: swap x and y, negate z
        let tmp = localPos.x;
        localPos.x = localPos.y;
        localPos.y = tmp;
        localPos.z = -localPos.z;
        let tn = normal.x;
        normal.x = normal.y;
        normal.y = tn;
        normal.z = -normal.z;
        let tu = localUv.x;
        localUv.x = localUv.y;
        localUv.y = tu;
        localUv.z = -localUv.z;
    }

    if (isBackground) {
        localPos = vec3f(localPos.x * 1.05, localPos.y * 1.05, 0.0);
        normal = vec3f(0.0, 0.0, 1.0);
    }

    // Frontal quad disabled (bit 7 reused for flavorIdx).
    // If frontal quad is re-enabled, it will need a different flag bit.

    let worldPos = localPos + inst.position;
    let pos4 = vec4f(worldPos, 1.0);
    let eyePos = transform.modelView * pos4;

    out.posEye = eyePos.xyz;
    out.position = transform.mvp * pos4;

    // UV handling: the per-instance transform applies to the diffuse/garbage texture.
    let hasFlavor = (inst.flags & 2u) != 0u;
    if (hasFlavor) {
        // Flavor overlay: derive UV from vertex position.
        // Flavor quad vertices range from -2.0 to 2.0 (S=2.0). Normalize to [0,1].
        // V is flipped (S - Y) because texture row 0 = top of image.
        let S = 2.0;
        let posUV = vec2f(
            (in.position.x + S) / (2.0 * S),
            (S - in.position.y) / (2.0 * S)
        );
        out.uv = posUV * inst.uvTransform.xy + inst.uvTransform.zw;
    } else {
        let baseUV = localUv.xy;
        out.uv = baseUV * inst.uvTransform.xy + inst.uvTransform.zw;
    }
    out.normalEye = normalize((transform.normalMatrix * vec4f(normal, 0.0)).xyz);
    out.color = inst.color;
    out.clipPlane = inst.clipPlaneEye;
    out.clipEnabled = f32(inst.flags & 1u);
    out.hasFlavor = f32((inst.flags >> 1u) & 1u);  // bit 1 = has flavor overlay
    out.worldPos = worldPos;  // Needed for organic clip edge noise
    out.localPos = localPos;  // Procedural noise must be in garbage-local space
    // Decode cell position + garbage dims from flags bits 8-23
    let cellX = f32((inst.flags >> 8u) & 0xFu);
    let cellY = f32((inst.flags >> 12u) & 0xFu);
    let garbW = max(f32((inst.flags >> 16u) & 0xFu), 1.0);
    let garbH = max(f32((inst.flags >> 20u) & 0xFu), 1.0);

    // Per-garbage seed from CPU (bits 24-27 = hash_u, 28-31 = hash_v),
    // normalized to [0,1] and converted to a phase here. Every piece of a garbage
    // block gets the same value.
    out.cellPhase = f32((inst.flags >> 24u) & 0xFu) / 15.0 * 6.28;
    out.cellInfo = vec4f(cellX, cellY, garbW, garbH);
    out.flavorIdx = f32((inst.flags >> 7u) & 1u);

    return out;
}

// Multi-frequency pseudo-noise for the organic shatter edge.
// Built from three sines with different
// frequencies and phases so the output looks like "cracks" along
// the world XY plane. Output range roughly [-0.35, +0.35].
fn shatterWave(world: vec3f) -> f32 {
    let n1 = sin(world.x * 1.9 + world.y * 2.3);
    let n2 = sin(world.x * 5.1 - world.y * 3.7 + 1.7);
    let n3 = sin(world.x * 0.6 + world.y * 0.9 + 4.2);
    return (n1 * 0.22 + n2 * 0.10 + n3 * 0.06);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    // Organic cut edge: `clipDist` is perturbed by a multi-sine wave anchored on
    // the world XY position (shatterWave). The edge is wavy and consistent across
    // neighboring fragments, instead of the straight line of the original binary
    // test. The CPU eases `shatter_progress` (smoothstep on the plane W).
    if (in.clipEnabled > 0.5) {
        let rawDist = dot(vec4f(in.posEye, 1.0), in.clipPlane);
        let wave = shatterWave(in.worldPos);
        let clipDist = rawDist + wave;
        if (clipDist < 0.0) {
            discard;
        }
    }

    // Sample textures unconditionally (WebGPU requires uniform control flow for textureSample).
    // Mirrored shells can have negative UVs (ClampToEdge). That is harmless: the
    // result is discarded unless the piece has a flavor, and flavor quads always
    // get UVs in [0,1] (posUV with an identity transform).
    let texColor = textureSample(diffuseTexture, diffuseSampler, in.uv);

    // Garbage material — subtle procedural enrichment
    // All noise is sampled in GARBAGE-LOCAL space (in.localPos) so
    // the pattern moves with the garbage, not with the grid.
    var alpha = in.color.a;

    // Spatial gradient in garbage-local coordinates.
    // Origin = front-left-bottom corner.
    // Z: depth (0=front, 1=back) → darkening
    // Y: height (0=bottom, 1=top) → subtle warm/yellow shift
    let N_face = normalize(in.normalEye);

    // Depth: post-transform Z. The mirror/swap transforms ensure localPos.z > 0
    // always faces the camera. Range: front face ≈ +1.0, back face ≈ -1.0.
    // Map [-1, +1] → [1, 0] so front=0 (bright) and back=1 (dark).
    let depth_t = 0.5 - 0.5 * clamp(in.localPos.z / 1.1, -1.0, 1.0); // 0=front, 1=back

    // Height: Y normalized over the full garbage (not just this cell)
    let ci = in.cellInfo; // cellX, cellY, garbW, garbH
    let cell_y_frac = clamp((in.localPos.y + 1.0) * 0.5, 0.0, 1.0);
    // ci.z and ci.w are already clamped to at least 1 in the vertex stage.
    let height_t = (ci.y + cell_y_frac) / ci.w; // 0=bottom, 1=top

    // 1. Tint = f(X, Y): garbage color at bottom, warm shift at top.
    // The tint onset varies with X via a sinusoidal wave (< 1 period)
    // with a per-garbage random phase offset (cellPhase).
    let gw = lighting.garbageWarm;
    // Absolute target color: red→orange, gray→warm beige
    let warm_target = select(
        vec3f(0.55, 0.50, 0.38),  // gray → warm beige
        vec3f(1.0, 0.55, 0.08),   // red → orange
        in.flavorIdx < 0.5
    );

    // X position normalized over the full garbage width
    let cell_x_frac = clamp((in.localPos.x + 1.0) * 0.5, 0.0, 1.0);
    let x_norm = (ci.x + cell_x_frac) / ci.z;

    // Sinusoidal modulation on the tint onset height (~0.7 period).
    // wave_amp comes from lighting.specular.w, which is repurposed as the wave
    // amplitude. The renderer sets it to 0.25, i.e. ±25% of the height.
    let wave_amp = lighting.specular.w;
    let wave = sin(x_norm * 2.2 + in.cellPhase) * wave_amp;

    // Tint starts at ~1/3 height, modulated by X wave
    let tint_onset = 0.33 + wave;
    let tint_t = clamp((height_t - tint_onset) / (1.0 - tint_onset), 0.0, 1.0);
    // For gray garbages: darken the base at the bottom, lighten at the top.
    // Red stays as-is (already has strong color contrast with orange target).
    let base_with_height = select(
        in.color.rgb * (0.7 + 0.5 * height_t),  // gray: 70% at bottom → 120% at top
        in.color.rgb,                             // red: unchanged
        in.flavorIdx < 0.5
    );
    let tint = mix(base_with_height, warm_target, tint_t * tint_t * gw);

    // 2. Brightness = f(Z only): front = 1.0, back dims toward min.
    // Smooth cubic ease: sides (depth_t=0.5) stay reasonably lit,
    // only the back face (depth_t→1) darkens significantly.
    let min_brightness = 0.25;
    let brightness = mix(1.0, min_brightness, depth_t * depth_t * (3.0 - 2.0 * depth_t));

    var baseColor = tint * brightness;

    // Flavor overlay: texture RGB + alpha for smooth edges via alpha-to-coverage
    if (in.hasFlavor > 0.5) {
        baseColor = texColor.rgb;
        alpha = texColor.a;
    }


    // Lighting: uses lighting.diffuse so the panic flashing light
    // tints garbage too.
    let N = N_face;
    let V = normalize(-in.posEye);
    let L = normalize(lighting.lightPos.xyz - in.posEye);
    let NdotL = max(dot(N, L), 0.0);

    // Ambient from lighting uniforms (includes ambient_tint modulation)
    let ambient = baseColor * lighting.ambient.rgb * 0.3;
    // Diffuse from lighting uniforms (carries the panic pulse)
    let diffuse = baseColor * lighting.diffuse.rgb * NdotL * 0.7;

    let finalColor = ambient + diffuse;
    return vec4f(finalColor, alpha);
}
"#;

/// Instanced particle shader - unlit, textured, with rotation and scale
///
/// Usage: Sparkles (additive), motes (additive), stars (alpha)
pub const INSTANCED_PARTICLE: &str = r#"
// Instanced particle shader - unlit, textured, billboarded with rotation/scale

struct TransformUniforms {
    mvp: mat4x4f,
    modelView: mat4x4f,
    normalMatrix: mat4x4f,
    texMatrix: mat4x4f,
    texMatrix1: mat4x4f,
    clipPlane: vec4f,
    clipEnabled: vec4f,
}

// ParticleInstance: 48 bytes
struct ParticleInstance {
    position: vec3f,  // 12 bytes
    angle: f32,       // 4 bytes (Z rotation in radians)
    color: vec4f,     // 16 bytes
    scaleX: f32,      // 4 bytes
    scaleY: f32,      // 4 bytes (negative for flip)
    _pad0: f32,       // 4 bytes
    _pad1: f32,       // 4 bytes
}

@group(0) @binding(0) var<uniform> transform: TransformUniforms;
@group(0) @binding(1) var particleTexture: texture_2d<f32>;
@group(0) @binding(2) var particleSampler: sampler;

@group(1) @binding(0) var<storage, read> instances: array<ParticleInstance>;

struct VertexInput {
    @builtin(instance_index) instanceIdx: u32,
    @builtin(vertex_index) vertexIdx: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
}

// Quad vertices (2 triangles, 6 vertices per instance), CCW winding.
// Declared at module level so they are not rebuilt per vertex invocation.
const QUAD_POS: array<vec2f, 4> = array<vec2f, 4>(
    vec2f(-0.5, -0.5),  // bottom-left
    vec2f( 0.5, -0.5),  // bottom-right
    vec2f(-0.5,  0.5),  // top-left
    vec2f( 0.5,  0.5)   // top-right
);
const QUAD_UV: array<vec2f, 4> = array<vec2f, 4>(
    vec2f(0.0, 1.0),
    vec2f(1.0, 1.0),
    vec2f(0.0, 0.0),
    vec2f(1.0, 0.0)
);
const QUAD_INDICES: array<u32, 6> = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let inst = instances[in.instanceIdx];

    // The draw is `draw(0..6, 0..N)` (renderer_runtime), so vertex_index is in [0,5].
    let idx = QUAD_INDICES[in.vertexIdx];
    var localPos = QUAD_POS[idx];
    out.uv = QUAD_UV[idx];

    // Apply scale
    localPos.x = localPos.x * inst.scaleX;
    localPos.y = localPos.y * inst.scaleY;

    // Apply rotation around Z
    let c = cos(inst.angle);
    let s = sin(inst.angle);
    let rotated = vec2f(
        localPos.x * c - localPos.y * s,
        localPos.x * s + localPos.y * c
    );

    // Billboard: use view matrix columns for orientation
    let right = vec3f(transform.modelView[0][0], transform.modelView[1][0], transform.modelView[2][0]);
    let up = vec3f(transform.modelView[0][1], transform.modelView[1][1], transform.modelView[2][1]);

    let worldPos = inst.position + right * rotated.x + up * rotated.y;
    out.position = transform.mvp * vec4f(worldPos, 1.0);
    out.color = inst.color;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    let tex = textureSample(particleTexture, particleSampler, in.uv);
    // Luminance-alpha mode: use red channel as luminance
    let luminance = tex.r;
    return vec4f(in.color.rgb * luminance, in.color.a * tex.a);
}
"#;

pub fn create_shader_module(device: &wgpu::Device, source: &str, label: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// All shader modules needed for rendering
pub struct ShaderModules {
    /// Combined UI module: `vs_main` plus `fs_textured`, `fs_alpha_text` and `fs_untextured`.
    pub ui: wgpu::ShaderModule,
    pub instanced_particle: wgpu::ShaderModule,
    pub level_light_emission: wgpu::ShaderModule,
}

impl ShaderModules {
    /// Create all shader modules
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            ui: create_shader_module(device, UI_COMBINED, "ui_combined"),
            instanced_particle: create_shader_module(device, INSTANCED_PARTICLE, "instanced_particle"),
            level_light_emission: create_shader_module(device, LEVEL_LIGHT_EMISSION, "level_light_emission"),
        }
    }
}
