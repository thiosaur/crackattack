//! Render Command Buffer
//!
//! Deferred command recording for rendering.
//!
//! Architecture:
//! - `RenderCommandBuffer` is a separate layer above the renderer.
//! - Draw calls record commands; the buffer is cleared and reused each frame.

use crate::renderer::gpu::mesh::BuiltinMesh;
use crate::renderer::gpu::ui_batch::UIBatch;


/// Per-frame rendering statistics
#[derive(Clone, Debug, Default)]
pub struct FrameStats {
    pub num_draw_ui_batch: u32,
    pub num_draw_instanced: u32,
    pub num_draw_garbage_instanced: u32,
    pub num_commands: u32,
}

/// A recorded render command
#[derive(Clone, Debug)]
pub enum RenderCommand {
    /// Begin a new frame
    BeginFrame,

    /// Draw instanced blocks
    DrawInstanced {
        instance_count: u32,
    },

    /// Draw instanced garbage pieces
    DrawGarbageInstanced {
        mesh: BuiltinMesh,
        instance_count: u32,
        first_instance: u32,
        texture: u32,
    },

    /// Draw a UI batch
    DrawUIBatch(UIBatch),

    /// End the current frame
    EndFrame,
}

/// Buffer for recording render commands
#[derive(Clone, Debug, Default)]
pub struct RenderCommandBuffer {
    commands: Vec<RenderCommand>,
    stats: FrameStats,
    #[cfg(debug_assertions)]
    frame_started: bool,
    #[cfg(debug_assertions)]
    frame_ended: bool,
}

impl RenderCommandBuffer {
    /// Create a new empty command buffer
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a new frame (must be first command)
    pub fn begin_frame(&mut self) {
        #[cfg(debug_assertions)]
        {
            assert!(!self.frame_started, "begin_frame called twice without end_frame");
            self.frame_started = true;
            self.frame_ended = false;
        }

        self.commands.push(RenderCommand::BeginFrame);
        self.stats.num_commands += 1;
    }

    /// Record an instanced draw call (blocks)
    pub fn draw_instanced(&mut self, instance_count: u32) {
        #[cfg(debug_assertions)]
        {
            assert!(self.frame_started, "draw_instanced called before begin_frame");
            assert!(!self.frame_ended, "draw_instanced called after end_frame");
        }

        self.commands.push(RenderCommand::DrawInstanced { instance_count });
        self.stats.num_draw_instanced += 1;
        self.stats.num_commands += 1;
    }

    /// Record an instanced garbage draw call
    pub fn draw_garbage_instanced(
        &mut self,
        mesh: BuiltinMesh,
        instance_count: u32,
        first_instance: u32,
        texture: u32,
    ) {
        #[cfg(debug_assertions)]
        {
            assert!(self.frame_started, "draw_garbage_instanced called before begin_frame");
            assert!(!self.frame_ended, "draw_garbage_instanced called after end_frame");
        }

        self.commands.push(RenderCommand::DrawGarbageInstanced {
            mesh,
            instance_count,
            first_instance,
            texture,
        });
        self.stats.num_draw_garbage_instanced += 1;
        self.stats.num_commands += 1;
    }

    /// Record a UI batch draw call
    pub fn draw_ui_batch(&mut self, batch: UIBatch) {
        #[cfg(debug_assertions)]
        {
            assert!(self.frame_started, "draw_ui_batch called before begin_frame");
            assert!(!self.frame_ended, "draw_ui_batch called after end_frame");
        }

        self.commands.push(RenderCommand::DrawUIBatch(batch));
        self.stats.num_draw_ui_batch += 1;
        self.stats.num_commands += 1;
    }

    /// End the current frame (must be last command)
    pub fn end_frame(&mut self) {
        #[cfg(debug_assertions)]
        {
            assert!(self.frame_started, "end_frame called before begin_frame");
            assert!(!self.frame_ended, "end_frame called twice");
            self.frame_ended = true;
        }

        self.commands.push(RenderCommand::EndFrame);
        self.stats.num_commands += 1;
    }

    /// Clear all recorded commands (reuse buffer for next frame)
    pub fn clear(&mut self) {
        self.commands.clear();
        self.stats = FrameStats::default();
        #[cfg(debug_assertions)]
        {
            self.frame_started = false;
            self.frame_ended = false;
        }
    }

    /// Get recorded commands for execution
    pub fn commands(&self) -> &[RenderCommand] {
        &self.commands
    }

    /// Get frame statistics (test-only accessor)
    #[cfg(test)]
    pub fn stats(&self) -> &FrameStats {
        &self.stats
    }

    /// Check if buffer is empty (test-only accessor)
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Get command count (test-only accessor)
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Get total draw calls (instanced + UI batches)
    pub fn total_draws(&self) -> u32 {
        self.stats.num_draw_instanced
            + self.stats.num_draw_garbage_instanced
            + self.stats.num_draw_ui_batch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_buffer_basic() {
        let mut buffer = RenderCommandBuffer::new();

        buffer.begin_frame();
        buffer.draw_instanced(42);
        buffer.end_frame();

        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.stats().num_draw_instanced, 1);
        assert_eq!(buffer.total_draws(), 1);
    }

    #[test]
    fn test_command_buffer_clear() {
        let mut buffer = RenderCommandBuffer::new();

        buffer.begin_frame();
        buffer.end_frame();
        assert!(!buffer.is_empty());

        buffer.clear();
        assert!(buffer.is_empty());
        assert_eq!(buffer.stats().num_commands, 0);
    }

}
