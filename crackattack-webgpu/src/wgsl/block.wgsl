// Instanced, lit block shader with full animation support:
// dying (flash/shrink), swapping and awaking.

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
    // Bounding box of the currently dying blocks in world coordinates:
    // `[min_x, max_x, min_y, max_y]`. The dying animation reads it to
    // normalize each block's position to [-1, +1] relative to the
    // dissolution zone, so the gradient covers the actual group and not
    // the entire playfield.
    dyingZone: vec4f,             // offset 80
}

// Shared 96-byte layout (size asserts live in uniforms.rs). This shader reads only
// `diffuse` and `emission`. `ambient`, `useTexture`, `useEmission` and `vertexColor`
// are read by no shader; they stay declared to keep the layout.
struct MaterialUniforms {
    diffuse: vec4f,
    ambient: vec4f,
    specular: vec4f,
    emission: vec4f,
    shininess: f32,
    useTexture: f32,
    useEmission: f32,
    _pad0: f32,
    vertexColor: vec4f,
}

// Per-instance data (80 bytes per instance)
struct BlockInstance {
    position: vec3f,
    state: f32,
    color: vec4f,
    animProgress: f32,
    animAxisX: f32,
    animAxisY: f32,
    animPivotOffset: f32,
    lightPos: vec3f,
    visualFlags: u32,
    flavorIdx: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
}

// Animation state constants (mirror of BlockAnimState in instance.rs).
// State 1 (DyingFlash) exists in the protocol but has no branch here:
// the flash comes from the instance color computed on the CPU.
const STATE_STATIC: f32 = 0.0;
const STATE_DYING_SHRINK: f32 = 2.0;
const STATE_SWAPPING: f32 = 3.0;
const STATE_AWAKING_POP: f32 = 4.0;

// Visual effect flags. Only bit 0 is read here; the CPU also sets bits 1 and 2,
// which this shader ignores.
const VF_SWAP_BOUNCE: u32     = 1u;  // Bit 0

// Helper: check if flag is set
fn hasFlag(flags: u32, flag: u32) -> bool {
    return (flags & flag) != 0u;
}

// Animation timing constants
const DYING_SHRINK_MIN_SIZE: f32 = 0.1;
const PI: f32 = 3.14159265359;
// Degrees-to-radians factor, so the conversion is not recomputed per vertex.
const DEG2RAD: f32 = PI / 180.0;

// @group(0): Frame uniforms (bound once per frame)
@group(0) @binding(0) var<uniform> transform: TransformUniforms;
@group(0) @binding(1) var<uniform> lighting: LightingUniforms;

// @group(1): Instance storage buffer (per batch)
@group(1) @binding(0) var<storage, read> instances: array<BlockInstance>;

// @group(2): the material UBO only. Blocks use no textures.
@group(2) @binding(0) var<uniform> material: MaterialUniforms;

struct VertexInput {
    @location(0) position: vec3f,
    @location(1) normal: vec3f,
    // location(2) is the MeshVertex uv. This shader does not read it, which is
    // allowed: a shader may consume only part of the vertex layout.
    @builtin(instance_index) instanceIdx: u32,
}

// The locations are sparse on purpose: unused outputs were removed without
// renumbering the others.
struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(1) normalEye: vec3f,
    @location(2) posEye: vec3f,
    @location(4) instanceColor: vec4f,
}

// Rodrigues rotation formula: rotate vector v around axis by angle (radians)
fn rotateAroundAxis(v: vec3f, axis: vec3f, angle: f32) -> vec3f {
    let c = cos(angle);
    let s = sin(angle);
    let k = normalize(axis);
    return v * c + cross(k, v) * s + k * dot(k, v) * (1.0 - c);
}

// Easing functions for visual effects
fn easeOutBounce(t: f32) -> f32 {
    let n1 = 7.5625;
    let d1 = 2.75;
    var x = t;
    if (x < 1.0 / d1) {
        return n1 * x * x;
    } else if (x < 2.0 / d1) {
        x = x - 1.5 / d1;
        return n1 * x * x + 0.75;
    } else if (x < 2.5 / d1) {
        x = x - 2.25 / d1;
        return n1 * x * x + 0.9375;
    } else {
        x = x - 2.625 / d1;
        return n1 * x * x + 0.984375;
    }
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let inst = instances[in.instanceIdx];
    let vflags = inst.visualFlags;

    var localPos = in.position;
    var localNormal = in.normal;

    // STATE_DYING_SHRINK: rotation + shrink + optional wobble
    //
    // Varied dissolution: positions normalized to the dissolving
    // group bbox, outward drift, linear rotation, per-block tumble.
    if (abs(inst.state - STATE_DYING_SHRINK) < 0.5) {
        // ---- Varied path (local zone grid) ----
        //
        // All variation parameters derive from a position
        // **normalized to the bounding box of the current
        // dissolution group**, not the absolute playfield
        // coordinates. The CPU pass scans `frame.blocks` for
        // DyingShrink-state blocks each frame and writes
        // `[min_x, max_x, min_y, max_y]` into `lighting.dyingZone`.
        //
        // This way a 3-block chain in the corner of the
        // playfield spans the full [-1, +1] gradient
        // for each parameter, instead of all blocks landing
        // on a tiny slice of the playfield-wide curve.
        //
        // Outcome: blocks at the **left edge of the dissolving
        // group** systematically get fast ease + leftward
        // drift + counter-rotating tumble, blocks at the
        // **right edge** the opposite. Same logic on Y.
        // A row dissolves as a coordinated symmetric burst,
        // a column as a vertical disperse.
        let raw_t = inst.animProgress;
        let zone = lighting.dyingZone;  // x and y: X range; z and w: Y range
        let center_x = (zone.x + zone.y) * 0.5;
        let center_y = (zone.z + zone.w) * 0.5;
        // Half-extent with a floor so a single-block group
        // doesn't divide by zero.
        let half_x = max((zone.y - zone.x) * 0.5, 0.5);
        let half_y = max((zone.w - zone.z) * 0.5, 0.5);
        // Normalized local position in [-1, +1].
        let nx = clamp((inst.position.x - center_x) / half_x, -1.0, 1.0);
        let ny = clamp((inst.position.y - center_y) / half_y, -1.0, 1.0);

        // Speed varies with nx. Left of the group: ease < 1
        // (rapid burst at start). Right: ease > 1 (build-up).
        let ease_exp = max(1.0 + nx * 0.4, 0.4);
        let t = pow(raw_t, ease_exp);

        // Rotation magnitude varies with ny. Top of group
        // rotates harder than bottom. The angle profile is
        // **linear in t** (a quadratic `t²` would pile all the
        // rotation into the second half), so the rotation is
        // progressive across the whole animation. The total is
        // 540° (1.5 turns) because the linear curve makes the
        // late-frame motion feel faster at the same total angle.
        let angle_mult = max(1.0 + ny * 0.4, 0.4);
        let angle = t * 540.0 * DEG2RAD * angle_mult;

        // Tumble axis Z alternates per column inside the
        // group via a sinusoid on nx. Sign(nx) gives a hard
        // split, sin gives a smooth counter-rotation pattern.
        let axis_z = sin(nx * 2.5) * 0.8;
        let axis = vec3f(inst.animAxisX, inst.animAxisY, axis_z + 0.0001);

        localPos = rotateAroundAxis(localPos, axis, angle);
        localNormal = rotateAroundAxis(localNormal, axis, angle);

        // Wobble — frequency varies with nx (left columns
        // slow undulation, right fast), phase with ny so a
        // vertical pair doesn't wobble in lockstep.
        let wobbleFreq = max(13.0 + nx * 6.0, 4.0);
        let wobbleAmp = 0.10 * t;
        let wobblePhase = ny * 2.0;
        let wobble = sin(localPos.y * wobbleFreq + t * 20.0 + wobblePhase) * wobbleAmp;
        localPos.x = localPos.x + wobble;
        localPos.z = localPos.z + wobble * 0.5;

        // Shrink min varies subtly with ny.
        let shrink_min = max(DYING_SHRINK_MIN_SIZE + ny * 0.04, 0.02);
        let scale = mix(1.0, shrink_min, t);
        localPos = localPos * scale;

        // 3D drift — outward from the dissolution zone center.
        // nx and ny already point "away from center", so the
        // X and Y components are direct.
        //
        // **Z is always negative** (away from camera) — never
        // toward the camera. The shrinking block already reads
        // as "smaller", which the eye interprets as "farther".
        // If we then drift it forward (+Z toward camera) it
        // looks like it's *passing in front* of its neighbours
        // while appearing farther — a perceptual contradiction.
        // By always receding, "smaller" and "farther in Z"
        // align so the dissolution reads as a consistent
        // backward dispersion.
        //
        // Magnitude: corner blocks recede more than center
        // blocks (`abs(nx) + abs(ny)` term) so the spread has
        // some depth variation without ever going forward.
        let drift_x = nx * 1.4;
        let drift_y = ny * 0.7;
        let drift_z = -(2.5 + (abs(nx) + abs(ny)) * 1.5);
        let drift = vec3f(drift_x, drift_y, drift_z) * t;
        localPos = localPos + drift;
    }

    // STATE_SWAPPING: rotation around Y + position offset
    if (abs(inst.state - STATE_SWAPPING) < 0.5) {
        var t = inst.animProgress;  // 0→1 during swap (swap_factor)
        let direction = inst.animAxisX;  // -1 or +1
        let pivotOffset = inst.animPivotOffset;  // GRID_ELEMENT_LENGTH / 2

        // Optional bounce easing (VF_SWAP_BOUNCE)
        if (hasFlag(vflags, VF_SWAP_BOUNCE)) {
            t = easeOutBounce(t);
        }

        // Translate to pivot, rotate, translate back
        // This creates the "swing" effect around the midpoint
        let angle = -(1.0 - t) * PI; // -180° * (1 - t), in radians

        // Offset from block center to pivot point
        localPos.x = localPos.x - pivotOffset * direction;
        localPos = rotateAroundAxis(localPos, vec3f(0.0, 1.0, 0.0), angle);
        localPos.x = localPos.x + pivotOffset * direction;

        localNormal = rotateAroundAxis(localNormal, vec3f(0.0, 1.0, 0.0), angle);
    }

    // STATE_AWAKING_POP: rotation + scale
    //
    // Ease-out cubic (1-(1-t)³) gives stronger initial deceleration.
    // Exponent perturbed per-block by world position (2.2..3.8).
    //
    // `t=0 → 0` and `t=1 → 1` are preserved, so the start and
    // end positions are shared and the transition to `Static`
    // is snap-free.
    if (abs(inst.state - STATE_AWAKING_POP) < 0.5) {
        let raw_t = inst.animProgress;
        let popDir = i32(inst.animAxisX);

        // Per-block eased progress
        let exp_delta = sin(inst.position.x * 0.9 + inst.position.y * 0.6) * 0.8;
        let ease_exp = 3.0 + exp_delta;
        let t = 1.0 - pow(1.0 - raw_t, ease_exp);

        {
            let rot_scale = 1.0 + sin(inst.position.x * 1.7 + inst.position.y * 1.1) * 0.08;
            var rotX: f32 = 0.0;
            var rotY: f32 = 0.0;
            if (popDir == 0) {
                rotX = 45.0 * (1.0 - t) * rot_scale;
                rotY = 45.0 * (1.0 - t) * rot_scale;
            } else if (popDir == 1) {
                rotX = 90.0 - 45.0 * (1.0 - t) * rot_scale;
                rotY = 45.0 * (1.0 - t) * rot_scale;
            } else if (popDir == 2) {
                rotX = 90.0 - 45.0 * (1.0 - t) * rot_scale;
                rotY = -45.0 * (1.0 - t) * rot_scale;
            } else {
                rotX = 45.0 * (1.0 - t) * rot_scale;
                rotY = -45.0 * (1.0 - t) * rot_scale;
            }
            localPos = rotateAroundAxis(localPos, vec3f(1.0, 0.0, 0.0), rotX * DEG2RAD);
            localPos = rotateAroundAxis(localPos, vec3f(0.0, 1.0, 0.0), rotY * DEG2RAD);
            localNormal = rotateAroundAxis(localNormal, vec3f(1.0, 0.0, 0.0), rotX * DEG2RAD);
            localNormal = rotateAroundAxis(localNormal, vec3f(0.0, 1.0, 0.0), rotY * DEG2RAD);
            let scale = mix(0.5, 1.0, t);
            localPos = localPos * scale;
        }
    }

    // Squash & stretch on landing or swap settle.
    //
    // When the CPU detects a FALLING->STATIC or SWAPPING->STATIC
    // transition, it writes `1.0` into `animAxisX` and decays the
    // phase toward zero over ~200 ms.
    //
    // The shader converts the phase into time-since-impact
    // `t = (1.0 - phase) * 10` and evaluates a damped cosine
    // envelope: the block squashes vertically, bounces once or
    // twice, then settles. X and Z stretch inversely by
    // `1/sqrt(squash)` to preserve volume. Hard-clamped so the
    // block never leaves its cell.
    //
    // The `phase > 0.0` strict test guards against the static
    // baseline: only blocks with an active impact enter the
    // deformation branch.
    if (abs(inst.state - STATE_STATIC) < 0.5 && inst.animAxisX > 0.0001) {
        let phase = inst.animAxisX;
        let t = (1.0 - phase) * 10.0;
        let envelope = exp(-5.0 * t) * cos(20.0 * t);
        let squash = clamp(1.0 - 0.12 * envelope, 0.88, 1.12);
        let stretch = 1.0 / sqrt(squash);
        localPos.y = localPos.y * squash;
        localPos.x = localPos.x * stretch;
        localPos.z = localPos.z * stretch;
    }

    // Final position: local transformed + instance world position
    let worldPos = localPos + inst.position;
    let pos4 = vec4f(worldPos, 1.0);

    let eyePos = transform.modelView * pos4;
    out.posEye = eyePos.xyz;
    out.position = transform.mvp * pos4;
    out.normalEye = normalize((transform.normalMatrix * vec4f(localNormal, 0.0)).xyz);
    out.instanceColor = inst.color;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    // Clip plane test
    if (transform.clipEnabled.x > 0.5) {
        let clipDist = dot(vec4f(in.posEye, 1.0), transform.clipPlane);
        if (clipDist < 0.0) {
            discard;
        }
    }

    let N = normalize(in.normalEye);
    let V = normalize(-in.posEye);

    // Key light direction: fixed, from the top-front-left, so the cube faces stay readable.
    let L = normalize(vec3f(-0.4, 0.8, 0.5));
    let H = normalize(L + V);

    let NdotL_raw = dot(N, L);
    let NdotH = max(dot(N, H), 0.0);

    let NdotL = max(NdotL_raw, 0.0);

    // Base color from instance tint
    let baseColor = material.diffuse.rgb * in.instanceColor.rgb;

    // Blinn-Phong lighting for better bevel visibility
    let specPower = pow(NdotH, 32.0);  // Tight specular highlights

    let ambientContrib = baseColor * 0.25;  // Slightly higher ambient
    // `lighting.diffuse.rgb` is the panic blink color shift (vec3(1) at default).
    let diffuseContrib = baseColor * NdotL * 0.75 * lighting.diffuse.rgb;
    let specularContrib = vec3f(0.4) * specPower;  // White specular
    let emissionContrib = material.emission.rgb;

    let directContrib = diffuseContrib + specularContrib;

    var finalColor = ambientContrib + directContrib + emissionContrib;

    // lighting.ambient.a is a global brightness multiplier (default 1.0, no-op at release).
    finalColor = finalColor * lighting.ambient.a;

    // Blocks are opaque
    var alpha = 1.0;

    // Apply material/instance alpha
    alpha = alpha * material.diffuse.a * in.instanceColor.a;

    return vec4f(finalColor, alpha);
}
