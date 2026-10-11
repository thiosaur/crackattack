//! `TextureCache`: loads and caches textures from PNG files.
//!
//! It provides:
//! - PNG image loading using the `image` crate
//! - wgpu::Texture creation from loaded images
//! - HashMap-based caching by texture name
//! - Texture semantic (Color vs Data) for correct color space handling
//! - Version tracking for CPU/GPU synchronization
//!
//! Usage:
//! ```ignore
//! let mut cache = TextureCache::new();
//! cache.load(&device, &queue, "logo", &data_path.join("logo.png"))?;
//! let view = cache.get("logo").unwrap();
//! ```

use std::collections::HashMap;
use std::path::Path;

/// Texture semantic for color space handling
///
/// This determines the texture format used on GPU:
/// - Color: sRGB content (PNG artwork) → auto sRGB→linear conversion on sample
/// - Data: Linear data (masks, MSDF) → no conversion
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextureSemantic {
    /// sRGB content (PNG artwork) - uses Rgba8UnormSrgb format
    /// WebGPU automatically converts sRGB to linear during sampling
    #[default]
    Color,
    /// Linear data (masks, MSDF) - uses Rgba8Unorm format
    /// No color space conversion, raw linear values
    Data,
}

impl TextureSemantic {
    /// Get the wgpu texture format for this semantic
    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Color => wgpu::TextureFormat::Rgba8UnormSrgb,
            Self::Data => wgpu::TextureFormat::Rgba8Unorm,
        }
    }
}

/// A loaded texture and its view with metadata
///
/// Includes version tracking for CPU/GPU synchronization and
/// semantic information for correct color space handling.
pub struct TextureEntry {
    /// The GPU texture
    pub texture: wgpu::Texture,
    /// View for binding to shaders
    pub view: wgpu::TextureView,
    /// Texture dimensions
    pub width: u32,
    pub height: u32,
    /// Version number (incremented on each update)
    /// Used for CPU/GPU synchronization
    pub version: u32,
}

/// Options for loading a texture
#[derive(Clone, Debug, Default)]
pub struct TextureLoadOptions {
    /// Color space semantic (default: Color for sRGB)
    pub semantic: TextureSemantic,
    /// Generate CPU mipmaps (Lanczos3 downscale chain)
    pub generate_mipmaps: bool,
}

/// Cache for loaded textures
///
/// Textures are loaded from PNG files and stored by name for efficient reuse.
/// Supports version tracking for CPU/GPU synchronization.
pub struct TextureCache {
    textures: HashMap<String, TextureEntry>,
    /// Next version number (global counter)
    next_version: u32,
}

impl TextureCache {
    /// Create a new empty texture cache
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            next_version: 1,
        }
    }

    /// Load a texture from a PNG file with default options (Color semantic)
    ///
    /// # Arguments
    /// * `device` - The wgpu device
    /// * `queue` - The wgpu queue for uploading texture data
    /// * `name` - Name to store the texture under
    /// * `path` - Path to the PNG file
    ///
    /// # Returns
    /// Ok(()) on success, Err with message on failure
    pub fn load(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        name: &str,
        path: &Path,
    ) -> Result<(), String> {
        self.load_with_options(device, queue, name, path, TextureLoadOptions::default())
    }

    /// Load a texture from a PNG file with custom options
    pub fn load_with_options(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        name: &str,
        path: &Path,
        options: TextureLoadOptions,
    ) -> Result<(), String> {
        let img = image::open(path)
            .map_err(|e| format!("Failed to load texture '{}': {}", path.display(), e))?;

        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();

        let format = options.semantic.texture_format();

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let version = self.next_version;
        self.next_version += 1;

        self.textures.insert(
            name.to_string(),
            TextureEntry {
                texture,
                view,
                width,
                height,
                version,
            },
        );

        log::debug!(
            "Loaded texture '{}' ({}x{}, {:?}) from {}",
            name, width, height, options.semantic, path.display()
        );
        Ok(())
    }

    /// Load a texture from raw RGBA data
    ///
    /// # Arguments
    /// * `device` - The wgpu device
    /// * `queue` - The wgpu queue
    /// * `name` - Name to store the texture under
    /// * `width` - Texture width
    /// * `height` - Texture height
    /// * `data` - RGBA pixel data (must be width * height * 4 bytes)
    pub fn load_raw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        name: &str,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<(), String> {
        self.load_raw_with_options(
            device,
            queue,
            name,
            width,
            height,
            data,
            TextureLoadOptions::default(),
        )
    }

    /// Load a texture from raw RGBA data with custom options
    pub fn load_raw_with_options(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        name: &str,
        width: u32,
        height: u32,
        data: &[u8],
        options: TextureLoadOptions,
    ) -> Result<(), String> {
        let expected_size = (width * height * 4) as usize;
        if data.len() != expected_size {
            return Err(format!(
                "Texture '{}' data size mismatch: expected {}, got {}",
                name, expected_size, data.len()
            ));
        }

        let format = options.semantic.texture_format();
        let mip_count = if options.generate_mipmaps {
            (width.max(height) as f32).log2().floor() as u32 + 1
        } else {
            1
        };
        // Mips are generated on the CPU and uploaded (COPY_DST). No render pass
        // writes to these textures, so RENDER_ATTACHMENT is not requested.
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        if options.generate_mipmaps && mip_count > 1 {
            let src_img = image::RgbaImage::from_raw(width, height, data.to_vec())
                .ok_or_else(|| format!("Failed to create image for mipmap generation of '{}'", name))?;
            let mut src = src_img;
            let (mut mw, mut mh) = (width, height);
            for level in 1..mip_count {
                let nw = (mw / 2).max(1);
                let nh = (mh / 2).max(1);
                let resized = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Lanczos3);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &resized, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(nw * 4), rows_per_image: None },
                    wgpu::Extent3d { width: nw, height: nh, depth_or_array_layers: 1 },
                );
                src = resized;
                mw = nw;
                mh = nh;
            }
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let version = self.next_version;
        self.next_version += 1;

        self.textures.insert(
            name.to_string(),
            TextureEntry {
                texture,
                view,
                width,
                height,
                version,
            },
        );

        log::debug!("Loaded raw texture '{}' ({}x{}, {:?})", name, width, height, options.semantic);
        Ok(())
    }

    /// Get a texture view by name
    pub fn get_view(&self, name: &str) -> Option<&wgpu::TextureView> {
        self.textures.get(name).map(|e| &e.view)
    }

    /// Removes a texture from the cache and frees its GPU resource.
    ///
    /// Used to evict stale network opponent nameplates. Without eviction,
    /// every distinct name would keep a 256x128 texture alive forever.
    pub fn remove(&mut self, name: &str) -> bool {
        self.textures.remove(name).is_some()
    }

    /// Check if a texture is loaded (test-only accessor)
    #[cfg(test)]
    pub fn contains(&self, name: &str) -> bool {
        self.textures.contains_key(name)
    }

    /// Get the number of loaded textures
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// Check if the cache is empty (test-only accessor)
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    /// List all texture names in the cache
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.textures.keys()
    }

    /// Update texture data (increments version)
    ///
    /// This is used when texture content changes without reloading.
    /// Bind groups using this texture should be invalidated.
    pub fn update_raw(
        &mut self,
        queue: &wgpu::Queue,
        name: &str,
        data: &[u8],
    ) -> Result<u32, String> {
        let entry = self.textures.get_mut(name).ok_or_else(|| {
            format!("Texture '{}' not found in cache", name)
        })?;

        let expected_size = (entry.width * entry.height * 4) as usize;
        if data.len() != expected_size {
            return Err(format!(
                "Texture '{}' data size mismatch: expected {}, got {}",
                name, expected_size, data.len()
            ));
        }

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &entry.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * entry.width),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: entry.width,
                height: entry.height,
                depth_or_array_layers: 1,
            },
        );

        entry.version = self.next_version;
        self.next_version += 1;

        log::debug!("Updated texture '{}' to version {}", name, entry.version);
        Ok(entry.version)
    }
}

impl Default for TextureCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_texture_cache_creation() {
        let cache = TextureCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_texture_cache_contains() {
        let cache = TextureCache::new();
        assert!(!cache.contains("nonexistent"));
    }
}

