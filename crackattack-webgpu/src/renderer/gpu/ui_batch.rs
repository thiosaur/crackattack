//! `UIBatch`: a collection of UI primitives drawn together.
//!
//! GPU-facing data consumed by `render_command.rs` and the UI renderer.

use std::borrow::Cow;
use crate::renderer::gpu::types::{UIVertex, UITextureMode};

/// A batch of UI vertices to render with a single texture
#[derive(Debug, Clone)]
pub struct UIBatch {
    /// Vertex data
    pub vertices: Vec<UIVertex>,
    /// Triangle indices
    pub indices: Vec<u16>,
    /// Texture name (key in TextureCache). All texture names are
    /// compile-time literals — no heap allocation per batch.
    pub texture: Option<Cow<'static, str>>,
    /// Texture mode (determines shader variant)
    pub texture_mode: UITextureMode,
    /// Blend mode
    pub blend_mode: UIBlendMode,
}

/// Blend mode for UI rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UIBlendMode {
    #[default]
    Opaque,
    /// Alpha blending for textures with transparency
    Alpha,
    /// Additive blending for glowing effects
    Additive,
}

impl UIBatch {
    /// Create a new empty batch
    pub fn new(texture: Option<Cow<'static, str>>, blend_mode: UIBlendMode) -> Self {
        let texture_mode = if texture.is_some() {
            UITextureMode::Modulate
        } else {
            UITextureMode::None
        };
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            texture,
            texture_mode,
            blend_mode,
        }
    }

    /// Create a new batch with explicit texture mode
    pub fn with_texture_mode(
        texture: Option<Cow<'static, str>>,
        texture_mode: UITextureMode,
        blend_mode: UIBlendMode,
    ) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            texture,
            texture_mode,
            blend_mode,
        }
    }

    /// Create a batch for alpha-text rendering (messages, fonts)
    pub fn alpha_text(texture: &'static str) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            texture: Some(Cow::Borrowed(texture)),
            texture_mode: UITextureMode::AlphaText,
            blend_mode: UIBlendMode::Alpha,
        }
    }

    /// Create a batch for modulate texture (logo, images)
    pub fn modulate(texture: &'static str) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            texture: Some(Cow::Borrowed(texture)),
            texture_mode: UITextureMode::Modulate,
            blend_mode: UIBlendMode::Alpha,
        }
    }

    /// Push a quad as 2 triangles
    /// Vertex order: BL, BR, TL, TR
    pub fn push_quad(&mut self, bl: UIVertex, br: UIVertex, tl: UIVertex, tr: UIVertex) {
        let base = self.vertices.len() as u16;
        self.vertices.push(bl);
        self.vertices.push(br);
        self.vertices.push(tl);
        self.vertices.push(tr);

        self.indices.push(base);
        self.indices.push(base + 1);
        self.indices.push(base + 2);

        self.indices.push(base + 2);
        self.indices.push(base + 1);
        self.indices.push(base + 3);
    }

}
