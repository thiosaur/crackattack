//! Sparkle texture generation.
//!
//! Generates procedural luminance/alpha textures for sparkle particles.
//! Each texture type is a star pattern computed from polar coordinates.


/// Texture size in pixels.
pub const SPARKLE_TEX_SIZE: u32 = 32;

/// Sparkle texture types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparkleType {
    /// 4-pointed star, the only sparkle used by the particle pipeline.
    FourPointedStar,
}

/// Generates sparkle texture data for a given type.
///
/// Returns RGBA data (32x32x4 bytes) where:
/// - R = luminance (brightness)
/// - G = 0 (unused)
/// - B = 0 (unused)
/// - A = alpha (glow/edge fade)
///
/// The shader samples R for luminance and A for alpha.
pub fn generate_sparkle_texture(sparkle_type: SparkleType) -> Vec<u8> {
    let size = SPARKLE_TEX_SIZE as usize;
    let mut data = Vec::with_capacity(size * size * 4);

    for t in 0..size {
        for s in 0..size {
            // Normalized coordinates in [-1, 1].
            let x = s as f32 * (2.0 / (size - 1) as f32) - 1.0;
            let y = t as f32 * (2.0 / (size - 1) as f32) - 1.0;

            // Radius from the center.
            let r = (x * x + y * y).sqrt();
            let (luminance, alpha) = generate_pixel(sparkle_type, x, y, r);

            let lum_byte = (luminance.clamp(0.0, 1.0) * 255.0) as u8;
            let alpha_byte = (alpha.clamp(0.0, 1.0) * 255.0) as u8;

            data.push(lum_byte);  // R = luminance
            data.push(0);         // G = unused
            data.push(0);         // B = unused
            data.push(alpha_byte); // A = alpha
        }
    }

    data
}

/// Returns `(luminance, alpha)` for a single pixel.
fn generate_pixel(sparkle_type: SparkleType, x: f32, y: f32, r: f32) -> (f32, f32) {
    // Avoid division by zero.
    let r_safe = r.max(0.001);
    let sqrt_r = r_safe.sqrt();

    match sparkle_type {
        SparkleType::FourPointedStar => {
            let lum = 0.3 * (1.0 - 2.5 * (x * x - y * y).abs()) * (-r * r).exp() / sqrt_r;
            let alpha = 4.5 * lum * sqrt_r;
            (lum, alpha)
        }
    }
}

/// Loads the sparkle texture into the texture cache.
pub fn load_sparkle_textures(
    cache: &mut crate::renderer::gpu::texture::TextureCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<(), String> {
    use crate::renderer::gpu::texture::{TextureLoadOptions, TextureSemantic};

    let options = TextureLoadOptions {
        semantic: TextureSemantic::Data, // Raw data, no sRGB conversion.
        ..Default::default()
    };

    let spark_data = generate_sparkle_texture(SparkleType::FourPointedStar);
    cache.load_raw_with_options(
        device, queue,
        "sparkle",
        SPARKLE_TEX_SIZE, SPARKLE_TEX_SIZE,
        &spark_data,
        options.clone(),
    )?;

    log::info!("Generated sparkle texture ({}x{})", SPARKLE_TEX_SIZE, SPARKLE_TEX_SIZE);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_texture_size() {
        let data = generate_sparkle_texture(SparkleType::FourPointedStar);
        let expected = SPARKLE_TEX_SIZE as usize * SPARKLE_TEX_SIZE as usize * 4;
        assert_eq!(data.len(), expected);
    }

    #[test]
    fn test_center_bright() {
        let data = generate_sparkle_texture(SparkleType::FourPointedStar);
        let center = (SPARKLE_TEX_SIZE / 2) as usize;
        let idx = (center * SPARKLE_TEX_SIZE as usize + center) * 4;
        assert!(data[idx] > 0, "Center luminance should be non-zero");
    }

}
