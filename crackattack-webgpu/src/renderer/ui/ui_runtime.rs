//! UI rendering runtime: 2D overlay pipeline, texture cache, bind groups, egui.

use crate::renderer::gpu::texture::TextureCache;
use crate::renderer::gpu::pipeline::UIPipelineCache;
use crate::renderer::gpu::screen_overlay::ScreenOverlayRenderer;
use crate::renderer::ui::ui_renderer::UIRenderer;

/// UI rendering resources: 2D overlay pipeline, texture cache, bind groups, egui.
pub struct UiRuntime {
    pub ui_renderer: UIRenderer,
    /// Cached bind groups for UI rendering (texture name -> bind group with MVP + texture + sampler)
    pub ui_bind_groups: std::collections::HashMap<String, wgpu::BindGroup>,
    /// Bind group for solid color rendering (dummy texture)
    pub solid_bind_group: wgpu::BindGroup,
    pub ui_uniform_buffer: wgpu::Buffer,
    pub ui_bind_group_layout: wgpu::BindGroupLayout,
    pub ui_vertex_buffer: wgpu::Buffer,
    pub ui_index_buffer: wgpu::Buffer,
    pub ui_sampler: wgpu::Sampler,
    pub texture_cache: TextureCache,
    /// Scene-pass UI cache. Built MSAA + with depth, matching
    /// the scene render pass attachments.
    pub ui_pipeline_cache: UIPipelineCache,
    /// Screen-surface family (full-window curtains, veils and tints). Drawn in a dedicated
    /// post-scene pass with a small clip-space pipeline.
    pub screen_overlay: ScreenOverlayRenderer,
    /// Swapchain UI pipeline (single-sample, no depth). It redraws a world-space UI batch
    /// (the result message) over the curtain, in the overlay pass (square viewport).
    pub screen_ui_pipeline_cache: UIPipelineCache,
    pub screen_ui_vertex_buffer: wgpu::Buffer,
    pub screen_ui_index_buffer: wgpu::Buffer,
    /// World-space UI batch to draw over the curtain this frame (result/postgame), if any.
    pub screen_result_batch: Option<crate::renderer::gpu::ui_batch::UIBatch>,
    // egui overlay
    pub egui_ctx: egui::Context,
    pub egui_renderer: egui_wgpu::Renderer,
    /// egui input for the frame: the `RawInput` built by the binary from the ordered
    /// `PlatformShell` event stream (via `ui_bridge`) and consumed by `run_egui_pass`.
    /// `None` if no input was drained.
    pub pending_raw_input: Option<egui::RawInput>,
    /// Whether the window has focus. Feeds `RawInput.focused`.
    pub focused: bool,
    /// egui outputs for the frame: extracted from `FullOutput.platform_output` by
    /// `run_egui_pass` and applied by the binary through `PlatformShell` (contextual
    /// cursor, clipboard).
    pub pending_egui_outputs: Option<PendingEguiOutputs>,
}

/// Platform outputs of one egui frame: the subset the app actually emits. These are the
/// contextual cursor (`on_hover_cursor`) and clipboard copy (`ctx.copy_text`, used by the
/// network "copy address" button). There is no `TextEdit`/IME and no `OpenUrl`; links use
/// `open::that` directly.
pub struct PendingEguiOutputs {
    pub cursor_icon: egui::CursorIcon,
    pub copy_text: Option<String>,
}
