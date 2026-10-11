//! WGSL validation without a GPU.
//!
//! Every WGSL source is parsed and validated by naga (the frontend wgpu embeds),
//! so a syntax or type error, or a read of a removed field, fails `cargo test`
//! instead of `create_shader_module` at startup. The exact set of entry points
//! (name and stage) is checked too: a renamed, missing or wrong-stage entry
//! point would break pipeline creation.
//!
//! `Capabilities::empty()` means the shaders must not need any optional
//! capability, which keeps them compatible with minimal WebGPU.
//!
//! Limit: naga validates the source only, not its compatibility with the
//! `BindGroupLayout` or the device limits. Running the app covers that.

use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::ShaderStage;

fn stage_name(s: ShaderStage) -> &'static str {
    match s {
        ShaderStage::Vertex => "vertex",
        ShaderStage::Fragment => "fragment",
        ShaderStage::Compute => "compute",
    }
}

/// Parses and validates a source, and checks its exact set of entry points (name and stage).
fn validate_source(name: &str, source: &str, expected_entry_points: &[(&str, ShaderStage)]) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{name}: WGSL parse failed:\n{}", e.emit_to_string(source)));

    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: naga validation failed: {e:?}"));

    let mut found: Vec<(String, &'static str)> = module
        .entry_points
        .iter()
        .map(|ep| (ep.name.clone(), stage_name(ep.stage)))
        .collect();
    let mut expected: Vec<(String, &'static str)> = expected_entry_points
        .iter()
        .map(|(n, s)| (n.to_string(), stage_name(*s)))
        .collect();
    found.sort();
    expected.sort();
    assert_eq!(
        found, expected,
        "{name}: unexpected entry points (name, stage), found vs expected"
    );
}

/// Checks the 7 WGSL sources and their 16 entry points: 5 vertex/fragment
/// pairs, the combined UI module with 4 entry points, and 2 compute kernels.
/// Every new WGSL source must be added here.
#[test]
fn all_wgsl_sources_parse_validate_and_expose_expected_entry_points() {
    let vs_fs: &[(&str, ShaderStage)] = &[
        ("vs_main", ShaderStage::Vertex),
        ("fs_main", ShaderStage::Fragment),
    ];
    type Case<'a> = (&'a str, &'a str, &'a [(&'a str, ShaderStage)]);
    let cases: &[Case] = &[
        ("block.wgsl", crate::BLOCK_SHADER, vs_fs),
        ("LEVEL_LIGHT_EMISSION", super::shaders::LEVEL_LIGHT_EMISSION, vs_fs),
        (
            "UI_COMBINED",
            super::shaders::UI_COMBINED,
            &[
                ("vs_main", ShaderStage::Vertex),
                ("fs_textured", ShaderStage::Fragment),
                ("fs_alpha_text", ShaderStage::Fragment),
                ("fs_untextured", ShaderStage::Fragment),
            ],
        ),
        ("INSTANCED_GARBAGE", super::shaders::INSTANCED_GARBAGE, vs_fs),
        ("INSTANCED_PARTICLE", super::shaders::INSTANCED_PARTICLE, vs_fs),
        ("SCREEN_OVERLAY", super::screen_overlay::SCREEN_OVERLAY_WGSL, vs_fs),
        (
            "COMPUTE_SPARKLES",
            super::compute_sparkles::COMPUTE_SPARKLES_SHADER,
            &[
                ("cs_spawn", ShaderStage::Compute),
                ("cs_integrate", ShaderStage::Compute),
            ],
        ),
    ];
    for (name, source, entry_points) in cases {
        validate_source(name, source, entry_points);
    }
}
