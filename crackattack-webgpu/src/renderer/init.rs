//! Initialization utilities for renderer resources.
//!
//! Pure functions for projection math, asset discovery, texture loading,
//! and bind group construction. No dependency on `RendererRuntime`: these
//! are used by both the bootstrap (`app_bootstrap.rs`) and the runtime
//! (`renderer_runtime`).

use std::path::{Path, PathBuf};
use bytemuck::{Pod, Zeroable};

use crate::renderer::gpu::texture::TextureCache;
use crate::renderer::effects::sparkle_texture::load_sparkle_textures;
use crate::renderer::scene::garbage_flavor::GARBAGE_FLAVOR_COUNT;

/// Uniform buffer for UI projection matrix.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct UIUniforms {
    pub projection: [[f32; 4]; 4],
}

/// Create projection for UI rendering.
/// Uses the same frustum projection as the 3D game so UI elements in world
/// space align correctly.
pub fn create_ui_projection(width: u32, height: u32) -> UIUniforms {
    let n = 38.0_f32;   // near
    let f = 65.0_f32;   // far
    let b = -6.0_f32;   // bottom
    let t = 18.0_f32;   // top
    // The horizontal extent widens with the aspect ratio, as in
    // `create_projection_matrix`, so world-space UI stays aligned with the 3D scene.
    // The vertical extent is unchanged, so nothing is stretched.
    let aspect_eff = (width.max(1) as f32 / height.max(1) as f32).clamp(1.0, USEFUL_ZONE_MAX_ASPECT);
    let center_x = (-15.6_f32 + 8.4_f32) * 0.5;
    let half_w = (t - b) * 0.5 * aspect_eff;
    let l = center_x - half_w; // left
    let r = center_x + half_w; // right

    let proj = [
        [2.0 * n / (r - l),      0.0,                    0.0,                        0.0],
        [0.0,                    2.0 * n / (t - b),      0.0,                        0.0],
        [(r + l) / (r - l),      (t + b) / (t - b),      f / (n - f),                -1.0],
        [0.0,                    0.0,                    n * f / (n - f),            0.0],
    ];

    UIUniforms { projection: proj }
}

/// Compute effective viewport for letterboxing.
/// Game has 1:1 aspect ratio — uses "Contain" mode.
pub fn compute_effective_viewport(window_width: u32, window_height: u32) -> (u32, u32, u32, u32) {
    const GAME_ASPECT: f32 = 1.0;
    let window_aspect = window_width as f32 / window_height as f32;

    let (vp_width, vp_height) = if window_aspect > GAME_ASPECT {
        let h = window_height;
        let w = (window_height as f32 * GAME_ASPECT) as u32;
        (w, h)
    } else {
        let w = window_width;
        let h = (window_width as f32 / GAME_ASPECT) as u32;
        (w, h)
    };

    let vp_x = (window_width - vp_width) / 2;
    let vp_y = (window_height - vp_height) / 2;

    (vp_x, vp_y, vp_width, vp_height)
}

/// Maximum aspect ratio of the useful zone (16:9).
pub const USEFUL_ZONE_MAX_ASPECT: f32 = 16.0 / 9.0;

/// Horizontal offsets of the element groups when the layout reflows for wide windows.
pub struct ReflowOffsets {
    /// Info column (logo, nameplate, clock, score, stars). Negative: moves left.
    pub info_dx: f32,
    /// Playfield (gameplay group). Positive: moves right.
    pub play_dx: f32,
    /// Level lights: offset toward the edge, inset by the outer margin `o`.
    pub light_dx: f32,
    /// Danger bar: full-edge offset (`E/2`). It is an edge glow, so no outer margin.
    pub edge_dx: f32,
}

/// Computes the reflow offsets for the window aspect ratio.
///
/// The extra world width revealed by the widened frustum,
/// `E = REF_WIDTH * (aspect_eff - 1)`, is split over 5 margins with weights
/// `[outer, 2, 1, 2, outer]`: two side margins (lights/info and game/lights),
/// one central margin (info/game) and two outer margins (edge/lights). The outer
/// weight grows from 0 (square) to 0.5 (16:9), which insets the lights and danger
/// bar slightly so the margins look smaller. All offsets are 0 for square or
/// portrait windows.
pub fn reflow_offsets(window_width: u32, window_height: u32) -> ReflowOffsets {
    let aspect = (window_width.max(1) as f32 / window_height.max(1) as f32)
        .clamp(1.0, USEFUL_ZONE_MAX_ASPECT);
    let e = crate::renderer::ui::constants::REF_WIDTH * (aspect - 1.0);
    // Outer margin weight: linear from 0 at aspect 1 to 0.5 at 16:9.
    let outer_w = if USEFUL_ZONE_MAX_ASPECT > 1.0 {
        0.5 * (aspect - 1.0) / (USEFUL_ZONE_MAX_ASPECT - 1.0)
    } else { 0.0 };
    let total_w = 5.0 + 2.0 * outer_w;          // outer + 2 + 1 + 2 + outer
    let u = if total_w > 0.0 { e / total_w } else { 0.0 }; // width per unit of weight
    let o = outer_w * u;                         // outer margin (each side)
    ReflowOffsets {
        // Cumulative margins from the left edge, which moves out by -E/2.
        info_dx: -e / 2.0 + (outer_w + 2.0) * u, // outer + side margin
        play_dx: -e / 2.0 + (outer_w + 3.0) * u, // + central margin
        light_dx: e / 2.0 - o,                   // level lights inset by `o` from the edge
        edge_dx: e / 2.0,                        // danger bar: full widened edge
    }
}

/// Computes the useful zone: the centered rectangle with aspect `clamp(w/h, 1, 16/9)`,
/// as large as fits in the window.
///
/// It is square in portrait. In landscape up to 16:9 it fills the window; beyond
/// 16:9 it is a centered 16:9 rectangle. Game and UI layout live inside it; the
/// rest of the window only shows background and curtains.
/// Returns `(x, y, width, height)` in pixels.
pub fn compute_useful_zone(window_width: u32, window_height: u32) -> (u32, u32, u32, u32) {
    let w = window_width.max(1) as f32;
    let h = window_height.max(1) as f32;
    let aspect = (w / h).clamp(1.0, USEFUL_ZONE_MAX_ASPECT);
    // Largest rectangle of the given aspect that fits in the window.
    let (zw, zh) = if w / h >= aspect {
        (h * aspect, h) // limited by height (landscape)
    } else {
        (w, w / aspect) // limited by width (portrait: square, aspect = 1)
    };
    let zx = ((w - zw) * 0.5) as u32;
    let zy = ((h - zh) * 0.5) as u32;
    (zx, zy, zw as u32, zh as u32)
}

/// Asset directory candidates, in search order.
///
/// Candidates relative to the executable come first, so a double-click or
/// `.desktop` launch works from any working directory. Candidates relative to the
/// working directory follow, for development. `../Resources/assets` is the macOS
/// app bundle layout (`App.app/Contents/MacOS/<exe>` to
/// `App.app/Contents/Resources/assets`).
fn assets_dir_candidates(exe_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = exe_dir {
        candidates.push(dir.join("assets"));
        candidates.push(dir.join("../assets"));
        candidates.push(dir.join("../Resources/assets"));
    }
    candidates.extend([
        PathBuf::from("crackattack-webgpu/assets"),
        PathBuf::from("assets"),
        PathBuf::from("../crackattack-webgpu/assets"),
    ]);
    candidates
}

/// Find the data directory containing textures.
pub fn find_assets_dir() -> PathBuf {
    let exe = std::env::current_exe().ok();
    let candidates = assets_dir_candidates(exe.as_deref().and_then(Path::parent));
    // Require a directory and a sentinel asset (logo.png). Otherwise fail early and
    // clearly, instead of accepting an empty or incomplete directory and ending up
    // with silent white squares.
    for path in &candidates {
        if path.is_dir() && path.join("textures/ui/logo.png").is_file() {
            // Log the chosen candidate: it reveals a stale assets directory next to a
            // dev binary (the executable's directory wins over the working directory).
            log::info!("Assets: {}", path.display());
            return path.clone();
        }
    }
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    eprintln!();
    eprintln!("FATAL ERROR: assets folder not found (or incomplete).");
    eprintln!("  Current directory: {cwd}");
    eprintln!("  Candidates tried (must contain textures/ui/logo.png):");
    for c in &candidates {
        let abs = std::env::current_dir().map(|d| d.join(c)).unwrap_or_else(|_| c.clone());
        eprintln!("    - {}  ->  {}", c.display(), abs.display());
    }
    eprintln!("  Run the game from the workspace root (rust/) or from crackattack-webgpu/,");
    eprintln!("  or put assets/ next to the executable (Contents/Resources/assets in a macOS .app).");
    eprintln!("  (Without assets, the game only draws white squares.)");
    eprintln!();
    crate::platform::diagnostics::fatal(
        "The game data (assets folder) was not found next to the executable. Reinstall the game from its package.",
    )
}

/// Load UI textures: clock digits, logo, countdown, messages, signs, nameplates, menu items.
pub fn load_ui_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui_dir: &std::path::Path,
    fonts_dir: &std::path::Path,
    player_name: &str,
    texture_cache: &mut TextureCache,
) {
    for i in 0..10 {
        let name = format!("clock_{}", i);
        let path = ui_dir.join(format!("{}.png", name));
        if let Err(e) = texture_cache.load(device, queue, &name, &path) {
            log::warn!("Failed to load {}: {}", name, e);
        }
    }
    {
        let path = ui_dir.join("clock_extra.png");
        if let Err(e) = texture_cache.load(device, queue, "clock_extra", &path) {
            log::warn!("Failed to load clock_extra: {}", e);
        }
    }

    if let Err(e) = texture_cache.load(device, queue, "logo", &ui_dir.join("logo.png")) {
        log::warn!("Failed to load logo: {}", e);
    }

    for name in &["count_down_3", "count_down_2", "count_down_1", "count_down_go",
                  "message_game_over", "message_winner",
                  "message_round_win", "message_round_lose"] {
        let path = ui_dir.join(format!("{}.png", name));
        if let Err(e) = texture_cache.load(device, queue, name, &path) {
            log::warn!("Failed to load {}: {}", name, e);
        }
    }

    let (np_w, np_h, np_data) =
        crate::renderer::ui::text_texture::generate_nameplate_texture_from_assets(
            fonts_dir, player_name
        ).unwrap_or_else(|e| {
            log::warn!("Failed to load font assets, falling back to bitmap: {}", e);
            crate::renderer::ui::text_texture::generate_nameplate_texture()
        });
    if let Err(e) = texture_cache.load_raw(device, queue, "nameplate", np_w, np_h, &np_data) {
        log::warn!("Failed to generate nameplate texture: {}", e);
    }
    for diff_name in &["Logic", "Sigma", "Cortex"] {
        let tex_name = format!("nameplate_vs_{}", diff_name.to_lowercase());
        if let Ok((w, h, data)) = crate::renderer::ui::text_texture::generate_vs_nameplate_from_assets(
            fonts_dir, player_name, diff_name
        ) {
            let _ = texture_cache.load_raw(device, queue, &tex_name, w, h, &data);
        }
    }

    log::debug!("Loaded {} UI textures", texture_cache.len());
}

/// Load game textures: garbage atlas, garbage flavors, sparkles.
pub fn load_game_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    garbage_dir: &std::path::Path,
    texture_cache: &mut TextureCache,
) {
    {
        let red_path = garbage_dir.join("red_texture.png");
        let gray_path = garbage_dir.join("gray_texture.png");
        if let (Ok(red_img), Ok(gray_img)) = (image::open(&red_path), image::open(&gray_path)) {
            let tile_w = 512u32;
            let tile_h = 768u32;
            let red_tile = image::imageops::resize(&red_img.to_rgba8(), tile_w, tile_h, image::imageops::FilterType::Lanczos3);
            let gray_tile = image::imageops::resize(&gray_img.to_rgba8(), tile_w, tile_h, image::imageops::FilterType::Lanczos3);
            let mut atlas = image::RgbaImage::new(tile_w, tile_h * 2);
            image::imageops::overlay(&mut atlas, &red_tile, 0, 0);
            image::imageops::overlay(&mut atlas, &gray_tile, 0, tile_h as i64);
            let (aw, ah) = (atlas.width(), atlas.height());
            if let Err(e) = texture_cache.load_raw_with_options(
                device, queue, "garbage_flavor", aw, ah, &atlas.into_raw(),
                crate::renderer::gpu::texture::TextureLoadOptions { generate_mipmaps: true, ..Default::default() },
            ) {
                log::error!("Failed to load garbage atlas: {}", e);
            } else {
                log::debug!("Built garbage texture atlas: {}x{} (red + gray)", aw, ah);
            }
        } else {
            let fallback = garbage_dir.join("garbage_flavor_000.png");
            if let Err(e) = texture_cache.load(device, queue, "garbage_flavor", &fallback) {
                log::warn!("Failed to load garbage texture: {}", e);
            }
        }
    }

    let flavors_dir = garbage_dir.join("flavors");
    for i in 0..GARBAGE_FLAVOR_COUNT {
        let name = format!("garbage_flavor_{}", i);
        let path = flavors_dir.join(format!("flavor_{}.png", i));
        if let Err(e) = texture_cache.load(device, queue, &name, &path) {
            log::warn!("Failed to load {}: {}", name, e);
        }
    }

    if let Err(e) = load_sparkle_textures(texture_cache, device, queue) {
        log::warn!("Failed to generate sparkle textures: {}", e);
    }
}

/// Create a single UI bind group for one texture.
pub fn create_ui_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    texture_view: &wgpu::TextureView,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(texture_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

/// Build one bind group per cached texture for the UI pipeline.
pub fn build_ui_bind_groups(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    texture_cache: &TextureCache,
) -> std::collections::HashMap<String, wgpu::BindGroup> {
    let mut groups = std::collections::HashMap::new();
    for name in texture_cache.names() {
        if name == "_dummy" { continue; }
        if let Some(view) = texture_cache.get_view(name) {
            let bg = create_ui_bind_group(device, layout, uniform_buffer, sampler, view,
                &format!("ui_bind_group_{}", name));
            groups.insert(name.clone(), bg);
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_candidates_exe_first_then_cwd() {
        let exe_dir = Path::new("/Applications/Crack Attack.app/Contents/MacOS");
        let c = assets_dir_candidates(Some(exe_dir));
        assert_eq!(c[..3], [
            exe_dir.join("assets"),
            exe_dir.join("../assets"),
            exe_dir.join("../Resources/assets"),
        ]);
        assert_eq!(c[3..], [
            PathBuf::from("crackattack-webgpu/assets"),
            PathBuf::from("assets"),
            PathBuf::from("../crackattack-webgpu/assets"),
        ]);
    }

    #[test]
    fn assets_candidates_without_exe_are_cwd_only() {
        assert_eq!(assets_dir_candidates(None).len(), 3);
    }
}
