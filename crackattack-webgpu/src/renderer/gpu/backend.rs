//! RenderDump cursor: draw-call counter plus `frame_start` / `frame_end` forwarding.
//!
//! Command execution itself lives in `renderer_runtime::dispatch_render_commands`.

use crate::renderer::gpu::render_dump;
use crate::renderer::gpu::uniforms::FrameUniforms;

/// Draw-call counter for the current frame, with RenderDump logging.
#[derive(Default)]
pub struct RenderDumpCursor {
    draw_idx: u32,
}

impl RenderDumpCursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets the counter and logs the start of the frame.
    pub fn begin_frame(&mut self, uniforms: &FrameUniforms, frame_id: u32) {
        self.draw_idx = 0;
        if render_dump::is_enabled() {
            render_dump::frame_start(frame_id, uniforms);
        }
    }

    /// Logs the end of the frame.
    pub fn end_frame(&self, frame_id: u32, total_draws: u32) {
        if render_dump::is_enabled() {
            render_dump::frame_end(frame_id, total_draws);
        }
    }

    /// Index of the current draw in the frame.
    pub fn draw_idx(&self) -> u32 {
        self.draw_idx
    }

    /// Advances to the next draw.
    pub fn increment_draw_idx(&mut self) {
        self.draw_idx += 1;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_transform_uniforms_non_zero() {
        use crate::renderer::scene::transforms::create_transform_uniforms;
        let u = create_transform_uniforms(800, 600);
        // MVP diagonal should be non-zero (projection * view * model)
        assert_ne!(u.mvp[0][0], 0.0);
        assert_ne!(u.mvp[1][1], 0.0);
        assert_ne!(u.mvp[2][2], 0.0);
    }
}
