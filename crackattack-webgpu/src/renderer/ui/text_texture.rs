//! Text texture generation for UI elements (player names, scores, etc.)
//!
//! Supports two rendering backends:
//! - PNG glyph font: loads 86 individual `font0_*.png` glyphs and composites
//!   them with a diagonal shading gradient
//! - Fallback 8x8 bitmap font: embedded pixel font for when PNG assets aren't available

use std::path::Path;

/// 8x8 bitmap font data for uppercase letters, numbers, and basic punctuation
/// Each character is represented as 8 bytes, one per row, MSB = leftmost pixel
const FONT_8X8: &[(char, [u8; 8])] = &[
    // Letters
    ('A', [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x00]),
    ('B', [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00]),
    ('C', [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0x00]),
    ('D', [0x78, 0x6C, 0x66, 0x66, 0x66, 0x6C, 0x78, 0x00]),
    ('E', [0x7E, 0x60, 0x60, 0x78, 0x60, 0x60, 0x7E, 0x00]),
    ('F', [0x7E, 0x60, 0x60, 0x78, 0x60, 0x60, 0x60, 0x00]),
    ('G', [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3C, 0x00]),
    ('H', [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00]),
    ('I', [0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00]),
    ('J', [0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x6C, 0x38, 0x00]),
    ('K', [0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0x00]),
    ('L', [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0x00]),
    ('M', [0x63, 0x77, 0x7F, 0x6B, 0x63, 0x63, 0x63, 0x00]),
    ('N', [0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0x00]),
    ('O', [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00]),
    ('P', [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x00]),
    ('Q', [0x3C, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x0E, 0x00]),
    ('R', [0x7C, 0x66, 0x66, 0x7C, 0x78, 0x6C, 0x66, 0x00]),
    ('S', [0x3C, 0x66, 0x60, 0x3C, 0x06, 0x66, 0x3C, 0x00]),
    ('T', [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00]),
    ('U', [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00]),
    ('V', [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00]),
    ('W', [0x63, 0x63, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0x00]),
    ('X', [0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0x00]),
    ('Y', [0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x00]),
    ('Z', [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00]),
    // Numbers
    ('0', [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0x00]),
    ('1', [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00]),
    ('2', [0x3C, 0x66, 0x06, 0x0C, 0x30, 0x60, 0x7E, 0x00]),
    ('3', [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00]),
    ('4', [0x06, 0x0E, 0x1E, 0x66, 0x7F, 0x06, 0x06, 0x00]),
    ('5', [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00]),
    ('6', [0x3C, 0x66, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0x00]),
    ('7', [0x7E, 0x66, 0x0C, 0x18, 0x18, 0x18, 0x18, 0x00]),
    ('8', [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0x00]),
    ('9', [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x66, 0x3C, 0x00]),
    // Punctuation
    (' ', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
    (':', [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00]),
    ('.', [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00]),
    ('-', [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00]),
    ('_', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00]),
];

/// Get the bitmap data for a character
fn get_char_bitmap(c: char) -> Option<&'static [u8; 8]> {
    let upper = c.to_ascii_uppercase();
    FONT_8X8.iter()
        .find(|(ch, _)| *ch == upper)
        .map(|(_, data)| data)
}

/// A line of text to render in a fixed-size texture.
pub struct TextLine<'a> {
    pub text: &'a str,
    pub scale: u32,
    /// Vertical position (pixels from top). If `None`, auto-centered.
    pub y: Option<u32>,
    pub r: u8, pub g: u8, pub b: u8, pub a: u8,
}

/// Generate a fixed-size texture with centered text lines.
pub fn generate_fixed_text_texture(width: u32, height: u32, lines: &[TextLine]) -> (u32, u32, Vec<u8>) {
    let mut data = vec![0u8; (width * height * 4) as usize];
    for line in lines {
        let text_width = line.text.len() as u32 * 8 * line.scale;
        let offset_x = (width.saturating_sub(text_width)) / 2;
        let offset_y = line.y.unwrap_or_else(|| (height - 8 * line.scale) / 2);
        render_text_to_buffer(
            &mut data, width, height,
            line.text, line.scale, offset_x, offset_y,
            line.r, line.g, line.b, line.a,
        );
    }
    (width, height, data)
}

/// Generate a nameplate texture for "PLAYER 1" centered in 256x128 texture
pub fn generate_nameplate_texture() -> (u32, u32, Vec<u8>) {
    generate_fixed_text_texture(256, 128, &[
        TextLine { text: "PLAYER 1", scale: 4, y: None, r: 255, g: 255, b: 255, a: 255 },
    ])
}

/// Renders text with the 8x8 bitmap font into an existing RGBA buffer.
fn render_text_to_buffer(
    data: &mut [u8],
    tex_width: u32,
    tex_height: u32,
    text: &str,
    scale: u32,
    offset_x: u32,
    offset_y: u32,
    r: u8, g: u8, b: u8, a: u8,
) {
    for (char_idx, c) in text.chars().enumerate() {
        if let Some(bitmap) = get_char_bitmap(c) {
            let base_x = offset_x + char_idx as u32 * 8 * scale;

            for row in 0..8u32 {
                let row_data = bitmap[row as usize];
                for col in 0..8u32 {
                    let bit = (row_data >> (7 - col)) & 1;
                    if bit == 1 {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                let px = base_x + col * scale + sx;
                                let py = offset_y + row * scale + sy;
                                if px < tex_width && py < tex_height {
                                    let idx = ((py * tex_width + px) * 4) as usize;
                                    data[idx] = r;
                                    data[idx + 1] = g;
                                    data[idx + 2] = b;
                                    data[idx + 3] = a;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Letter texture size in pixels
const LETTER_TEX_LENGTH: u32 = 32;

/// Space width in pixels
const SPACE_WIDTH: u32 = 10;

/// 86 supported characters
const LETTER_MAPPING: &[char] = &[
    '0','1','2','3','4','5','6','7','8','9',
    'A','B','C','D','E','F','G','H','I','J','K','L','M',
    'N','O','P','Q','R','S','T','U','V','W','X','Y','Z',
    'a','b','c','d','e','f','g','h','i','j','k','l','m',
    'n','o','p','q','r','s','t','u','v','w','x','y','z',
    '-',':','.',',','!','@','~','#','$','%','&','(',')','/',
    '<','>','?','=','+','|',';','[','*',']',
];

/// Per-character width in pixels
const LETTER_WIDTHS: &[u32] = &[
    20,12,17,14,16,15,16,19,17,16,
    28,20,17,20,18,20,20,19,11,17,19,17,30,20,20,21,22,22,16,21,20,20,30,25,22,19,
    19,14,13,14,13,14,15,14, 8,12,14,12,22,15,14,14,16,16,12,15,14,15,22,18,17,13,
    20,12,11,10,11,21,23,23,18,19,20,12,13,18,16,16,17,20,23,16,22,22,16,22,
];

/// PNG filename suffixes
const LETTER_FILES: &[&str] = &[
    "0","1","2","3","4","5","6","7","8","9",
    "ca","cb","cc","cd","ce","cf","cg","ch","ci","cj","ck","cl","cm",
    "cn","co","cp","cq","cr","cs","ct","cu","cv","cw","cx","cy","cz",
    "a","b","c","d","e","f","g","h","i","j","k","l","m",
    "n","o","p","q","r","s","t","u","v","w","x","y","z",
    "mn","cln","pe","cma","ep","at","td","pd","ds","ps","and","pl","pr","sl",
    "lt","gt","qm","eq","pu","br","za","zb","zc","zd",
];

/// Default font color: blue
const DEFAULT_COLOR: [f32; 3] = [0.2, 0.2, 1.0];

/// Strip diacritics: map accented characters to their ASCII base.
/// Uses a manual table covering common Latin accented characters.
fn strip_diacritic(c: char) -> char {
    match c {
        'À'|'Á'|'Â'|'Ã'|'Ä'|'Å' => 'A',
        'Æ' => 'A',
        'Ç' => 'C',
        'È'|'É'|'Ê'|'Ë' => 'E',
        'Ì'|'Í'|'Î'|'Ï' => 'I',
        'Ñ' => 'N',
        'Ò'|'Ó'|'Ô'|'Õ'|'Ö' => 'O',
        'Ù'|'Ú'|'Û'|'Ü' => 'U',
        'Ý' => 'Y',
        'à'|'á'|'â'|'ã'|'ä'|'å' => 'a',
        'æ' => 'a',
        'ç' => 'c',
        'è'|'é'|'ê'|'ë' => 'e',
        'ì'|'í'|'î'|'ï' => 'i',
        'ñ' => 'n',
        'ò'|'ó'|'ô'|'õ'|'ö' => 'o',
        'ù'|'ú'|'û'|'ü' => 'u',
        'ý'|'ÿ' => 'y',
        'ß' => 's',
        // Latin Extended-A letters of Polish and Turkish
        'Ą' => 'A', 'Ć' => 'C', 'Ę' => 'E', 'Ł' => 'L', 'Ń' => 'N', 'Ś' => 'S', 'Ź'|'Ż' => 'Z',
        'ą' => 'a', 'ć' => 'c', 'ę' => 'e', 'ł' => 'l', 'ń' => 'n', 'ś' => 's', 'ź'|'ż' => 'z',
        'Ğ' => 'G', 'İ' => 'I', 'Ş' => 'S', 'ğ' => 'g', 'ı' => 'i', 'ş' => 's',
        _ => c,
    }
}

/// Map a character to its index in LETTER_MAPPING, or None if not found.
/// Strips diacritics first so accented characters fall back to their base letter.
fn map_char_to_code(c: char) -> Option<usize> {
    let base = strip_diacritic(c);
    LETTER_MAPPING.iter().position(|&ch| ch == base)
}

/// Load all 86 font letter textures from `font0_*.png` files, cached.
///
/// The PNGs are decoded once per process. Nameplates are regenerated on every keystroke while
/// the name is edited, and they reuse the cache instead of reading and decoding the files again.
/// `data_dir` is constant for a session (assets are resolved once), so the cache ignores it
/// after the first successful load.
fn load_letter_textures(data_dir: &Path) -> Result<&'static [Vec<u8>], String> {
    static CACHE: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();
    if let Some(cached) = CACHE.get() {
        return Ok(cached);
    }
    let loaded = load_letter_textures_impl(data_dir)?;
    // Borrow `&'static` from the `OnceLock`, so the 86 glyphs (about 88 KB, 86 allocations)
    // are not cloned on each nameplate generation.
    Ok(CACHE.get_or_init(|| loaded))
}

/// Load all 86 font letter textures from font0_*.png files.
/// Returns Vec of 32x32 alpha-only buffers.
fn load_letter_textures_impl(data_dir: &Path) -> Result<Vec<Vec<u8>>, String> {
    let mut textures = Vec::with_capacity(LETTER_FILES.len());
    for suffix in LETTER_FILES {
        let path = data_dir.join(format!("font0_{}.png", suffix));
        let img = image::open(&path)
            .map_err(|e| format!("Failed to load font glyph '{}': {}", path.display(), e))?;
        let rgba = img.to_rgba8();
        if rgba.width() != LETTER_TEX_LENGTH || rgba.height() != LETTER_TEX_LENGTH {
            return Err(format!(
                "Font glyph '{}' has unexpected size {}x{}, expected {}x{}",
                path.display(), rgba.width(), rgba.height(),
                LETTER_TEX_LENGTH, LETTER_TEX_LENGTH
            ));
        }
        // Extract alpha channel only (the letter shape is in the alpha channel)
        let alpha: Vec<u8> = rgba.pixels().map(|p| p.0[3]).collect();
        textures.push(alpha);
    }
    Ok(textures)
}

/// Compose a string into an RGBA texture at a given integer scale factor.
/// Scale 1 = normal (32px glyphs), scale 2 = double (64px), etc.
/// Returns (actual_pixel_width, RGBA data) where height = LETTER_TEX_LENGTH * scale.
fn fill_string_texture_scaled(
    text: &str,
    letters: &[Vec<u8>],
    max_width: u32,
    scale: u32,
) -> (u32, Vec<u8>) {
    let scale = scale.max(1);
    let out_h = LETTER_TEX_LENGTH * scale;
    let pixels = (max_width * out_h) as usize;
    let mut texture = vec![0u8; pixels * 4];

    let mut cursor: u32 = 0;
    let base_color = DEFAULT_COLOR;

    for c in text.chars() {
        if cursor >= max_width { break; }
        if c == ' ' { cursor += SPACE_WIDTH * scale; continue; }
        let code = match map_char_to_code(c) { Some(i) => i, None => continue };
        let char_width = LETTER_WIDTHS[code] * scale;
        if cursor + char_width > max_width { break; }
        let glyph = &letters[code];

        for t in 0..LETTER_TEX_LENGTH {
            for s in 0..LETTER_TEX_LENGTH {
                let glyph_alpha = glyph[(t * LETTER_TEX_LENGTH + s) as usize];
                if glyph_alpha == 0 { continue; }

                let vert = LETTER_TEX_LENGTH - t;
                let horiz = if s >= LETTER_WIDTHS[code] { LETTER_TEX_LENGTH } else { s + (LETTER_TEX_LENGTH - LETTER_WIDTHS[code]) };
                let gradient = (vert + horiz) as f32;
                let shade = gradient * 0.5 / LETTER_TEX_LENGTH as f32;

                // Write scale×scale block
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = cursor + s * scale + sx;
                        let py = t * scale + sy;
                        if px >= max_width || py >= out_h { continue; }
                        let idx = ((py * max_width + px) * 4) as usize;
                        let old_alpha = texture[idx + 3] as f32;
                        texture[idx + 3] = (texture[idx + 3] as u32 + glyph_alpha as u32).min(255) as u8;
                        for ch in 0..3 {
                            let color = base_color[ch] + (1.0 - base_color[ch]) * shade;
                            let old_value = texture[idx + ch] as f32;
                            texture[idx + ch] = (255.0 * color + (old_alpha / 255.0) * old_value).min(255.0) as u8;
                        }
                    }
                }
            }
        }
        cursor += char_width;
    }
    (cursor, texture)
}

/// Compute the pixel width of a string using legacy font metrics.
fn legacy_string_width(text: &str, max_width: u32) -> u32 {
    let mut length: u32 = 0;
    for c in text.chars() {
        if length >= max_width {
            break;
        }
        if c == ' ' {
            length += SPACE_WIDTH;
            continue;
        }
        if let Some(code) = map_char_to_code(c) {
            if length + LETTER_WIDTHS[code] > max_width {
                break;
            }
            length += LETTER_WIDTHS[code];
        }
    }
    length
}

/// Generate nameplate texture from font PNG assets (solo mode).
///
/// Returns (256, 128, RGBA data) with the player name centered.
pub fn generate_nameplate_texture_from_assets(
    data_dir: &Path,
    player_name: &str,
) -> Result<(u32, u32, Vec<u8>), String> {
    const TEX_WIDTH: u32 = 256;
    const TEX_HEIGHT: u32 = 128;

    let letters = load_letter_textures(data_dir)?;

    // Compute string width, clamped to texture width
    let str_width = legacy_string_width(player_name, TEX_WIDTH);
    if str_width == 0 {
        return Err("Empty player name".into());
    }

    // Render the string into a tight buffer (width = str_width, height = 32)
    let (actual_width, subtexture) = fill_string_texture_scaled(player_name, &letters, str_width, 1);

    // Create the final 256x128 texture (transparent)
    let mut data = vec![0u8; (TEX_WIDTH * TEX_HEIGHT * 4) as usize];

    // Center the rendered text in the 256x128 texture
    let offset_x = (TEX_WIDTH.saturating_sub(actual_width)) / 2;
    let offset_y = (TEX_HEIGHT - LETTER_TEX_LENGTH) / 2;

    // Copy subtexture into the final texture
    for t in 0..LETTER_TEX_LENGTH {
        for s in 0..actual_width {
            let src_idx = ((t * str_width + s) * 4) as usize;
            let dst_x = offset_x + s;
            let dst_y = offset_y + t;
            if dst_x < TEX_WIDTH && dst_y < TEX_HEIGHT {
                let dst_idx = ((dst_y * TEX_WIDTH + dst_x) * 4) as usize;
                data[dst_idx..dst_idx + 4].copy_from_slice(&subtexture[src_idx..src_idx + 4]);
            }
        }
    }

    log::info!(
        "Generated legacy font nameplate for '{}': string width={}, centered at ({}, {})",
        player_name, actual_width, offset_x, offset_y
    );

    Ok((TEX_WIDTH, TEX_HEIGHT, data))
}

/// Generate VS mode nameplate texture from font0_*.png glyph assets.
/// Layout: 3 centered lines
///   - Top: player name (normal size)
///   - Middle: "VS" (same glyph size)
///   - Bottom: opponent name (normal size)
/// Returns (256, 128, RGBA data).
pub fn generate_vs_nameplate_from_assets(
    data_dir: &Path,
    player_name: &str,
    opponent_name: &str,
) -> Result<(u32, u32, Vec<u8>), String> {
    const TEX_WIDTH: u32 = 256;
    const TEX_HEIGHT: u32 = 128;

    let letters = load_letter_textures(data_dir)?;
    let mut data = vec![0u8; (TEX_WIDTH * TEX_HEIGHT * 4) as usize];

    // Helper: blit a rendered string centered horizontally at vertical offset oy
    let blit = |data: &mut Vec<u8>, text: &str, oy: u32, height: u32, scale: u32| {
        let max_w = TEX_WIDTH;
        let scaled_w = legacy_string_width(text, max_w / scale) * scale;
        if scaled_w == 0 { return; }
        let (actual_w, sub) = fill_string_texture_scaled(text, &letters, scaled_w, scale);
        let ox = TEX_WIDTH.saturating_sub(actual_w) / 2;
        for t in 0..height {
            for s in 0..actual_w {
                let src = ((t * scaled_w + s) * 4) as usize;
                let (dx, dy) = (ox + s, oy + t);
                if dx < TEX_WIDTH && dy < TEX_HEIGHT && src + 3 < sub.len() {
                    let dst = ((dy * TEX_WIDTH + dx) * 4) as usize;
                    if sub[src + 3] > 0 {
                        data[dst..dst + 4].copy_from_slice(&sub[src..src + 4]);
                    }
                }
            }
        }
    };

    // Names at 32px, "VS" at 20px (bilinear downscale), 75% opacity
    let name_h = LETTER_TEX_LENGTH; // 32
    let vs_h: u32 = 20;
    let vs_gap: u32 = 4;  // tight spacing around VS
    let total = 2 * name_h + vs_h + 2 * vs_gap; // 32 + 4 + 20 + 4 + 32 = 92
    let outer = (TEX_HEIGHT - total) / 2; // 18px top/bottom margin
    let y0 = outer;
    let y1 = y0 + name_h + vs_gap;
    let y2 = y1 + vs_h + vs_gap;

    blit(&mut data, player_name, y0, name_h, 1);

    // "VS" downscaled from 32px to 20px with bilinear filtering + 75% opacity
    {
        let w = legacy_string_width("VS", TEX_WIDTH);
        if w > 0 {
            let (src_w, sub) = fill_string_texture_scaled("VS", &letters, w, 1);
            let src_h = LETTER_TEX_LENGTH;
            let dst_w = src_w * vs_h / src_h; // proportional width
            let ox = TEX_WIDTH.saturating_sub(dst_w) / 2;

            for dy in 0..vs_h {
                for dx in 0..dst_w {
                    // Map destination pixel to source (floating point)
                    let sx = dx as f32 * (src_w as f32 / dst_w as f32);
                    let sy = dy as f32 * (src_h as f32 / vs_h as f32);
                    let sx0 = sx as u32;
                    let sy0 = sy as u32;
                    let sx1 = (sx0 + 1).min(src_w - 1);
                    let sy1 = (sy0 + 1).min(src_h - 1);
                    let fx = sx - sx0 as f32;
                    let fy = sy - sy0 as f32;

                    // Bilinear sample 4 channels
                    let sample = |x: u32, y: u32, ch: usize| -> f32 {
                        let i = ((y * w + x) * 4) as usize + ch;
                        if i < sub.len() { sub[i] as f32 } else { 0.0 }
                    };
                    let (px, py_dst) = (ox + dx, y1 + dy);
                    if px >= TEX_WIDTH || py_dst >= TEX_HEIGHT { continue; }
                    let dst = ((py_dst * TEX_WIDTH + px) * 4) as usize;

                    for ch in 0..4 {
                        let v = sample(sx0, sy0, ch) * (1.0 - fx) * (1.0 - fy)
                              + sample(sx1, sy0, ch) * fx * (1.0 - fy)
                              + sample(sx0, sy1, ch) * (1.0 - fx) * fy
                              + sample(sx1, sy1, ch) * fx * fy;
                        let mut val = v as u8;
                        if ch == 3 { val = (val as u32 * 3 / 4) as u8; } // 75% opacity on alpha
                        data[dst + ch] = val;
                    }
                }
            }
        }
    }

    blit(&mut data, opponent_name, y2, name_h, 1);

    Ok((TEX_WIDTH, TEX_HEIGHT, data))
}


#[cfg(test)]
mod tests {
    use super::*;

    // Polish and Turkish names keep a letter on the nameplate for each character.
    #[test]
    fn polish_and_turkish_letters_fold_to_ascii() {
        let folded: String = "Łukasz Żółć Işıl Şeyma Ğİ".chars().map(strip_diacritic).collect();
        assert_eq!(folded, "Lukasz Zolc Isil Seyma GI");
    }

    #[test]
    fn test_generate_nameplate_texture() {
        let (w, h, data) = generate_nameplate_texture();
        assert_eq!(w, 256);
        assert_eq!(h, 128);
        assert_eq!(data.len(), 256 * 128 * 4);
    }
}
