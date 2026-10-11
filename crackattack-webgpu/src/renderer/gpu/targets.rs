//! Render target management for the native WGPU renderer.
//!
//! Owns the MSAA color attachment, the depth attachment, their underlying
//! textures (kept alive so the views don't dangle) and the current size.
//! Resizing recreates both textures.

use wgpu::TextureFormat;

/// Standard MSAA sample count; call sites may pass a different value to RenderTargets::new.
pub const DEFAULT_MSAA_SAMPLE_COUNT: u32 = 4;

/// Depth texture format used by the scene render pass.
pub const DEFAULT_DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;

/// Scene render targets for the main rendering pipeline.
///
/// Holds MSAA + depth attachments and keeps their underlying textures
/// alive so the views don't dangle. Exposes the views via public fields
/// for direct use in render pass descriptors.
pub struct RenderTargets {
    /// MSAA color attachment view — use as `color_attachments[0].view`.
    /// The resolve target is typically the final swapchain view.
    pub msaa_color_view: wgpu::TextureView,

    /// Underlying MSAA color texture — kept alive so the view above
    /// stays valid. Not exposed directly to callers.
    msaa_color_tex: wgpu::Texture,

    /// Depth attachment view — use as `depth_stencil_attachment.view`.
    pub depth_view: wgpu::TextureView,

    /// Underlying depth texture — kept alive so the view above stays
    /// valid. Not exposed directly to callers.
    depth_tex: wgpu::Texture,

    size: (u32, u32),
    sample_count: u32,
    color_format: TextureFormat,
    depth_format: TextureFormat,
}

impl RenderTargets {
    /// Create a new set of render targets at the given size.
    ///
    /// `color_format` should match the surface configuration format of
    /// the swapchain (so the MSAA target and the final resolve target
    /// are compatible). `sample_count` must be a value supported by the
    /// underlying WGPU adapter — typically 4 for the Crack Attack scene.
    pub fn new(
        device: &wgpu::Device,
        size: (u32, u32),
        sample_count: u32,
        color_format: TextureFormat,
        depth_format: TextureFormat,
    ) -> Self {
        let (msaa_color_tex, msaa_color_view) =
            create_msaa_color(device, size, sample_count, color_format);
        let (depth_tex, depth_view) =
            create_depth(device, size, sample_count, depth_format);

        Self {
            msaa_color_view,
            msaa_color_tex,
            depth_view,
            depth_tex,
            size,
            sample_count,
            color_format,
            depth_format,
        }
    }

    /// Resize the targets to a new dimension. No-op if the size is
    /// unchanged or if either dimension is zero (zero-size windows can
    /// occur on some platforms during minimization and must not trigger
    /// a texture recreation).
    ///
    /// Recreates both the MSAA color and depth textures and their
    /// views. Callers holding copies of the old views (via the public
    /// fields) must re-read them after this call.
    pub fn resize(&mut self, device: &wgpu::Device, new_size: (u32, u32)) {
        if new_size == self.size || new_size.0 == 0 || new_size.1 == 0 {
            return;
        }
        let (msaa_color_tex, msaa_color_view) =
            create_msaa_color(device, new_size, self.sample_count, self.color_format);
        let (depth_tex, depth_view) =
            create_depth(device, new_size, self.sample_count, self.depth_format);

        self.msaa_color_tex = msaa_color_tex;
        self.msaa_color_view = msaa_color_view;
        self.depth_tex = depth_tex;
        self.depth_view = depth_view;
        self.size = new_size;
    }

}

fn create_msaa_color(
    device: &wgpu::Device,
    size: (u32, u32),
    sample_count: u32,
    format: TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rendering::RenderTargets::msaa_color"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn create_depth(
    device: &wgpu::Device,
    size: (u32, u32),
    sample_count: u32,
    format: TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rendering::RenderTargets::depth"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}
