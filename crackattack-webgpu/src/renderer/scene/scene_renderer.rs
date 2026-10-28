//! Scene renderer module: owns GPU pipelines, execution state, and visual animators.

use crate::renderer::gpu::backend::RenderDumpCursor;
use crate::renderer::gpu::render_command::RenderCommandBuffer;
use crate::renderer::gpu::ring_buffer::UniformRingBuffer;
use crate::renderer::scene::pipelines::ScenePipelines;
use crate::renderer::scene::garbage_flavor::GarbageFlavorImageState;
use crate::renderer::effects::game_animators::{DangerBarAnimator, LevelLightAnimator, LogoShakeAnimator, OpponentBoardAnimator};

pub struct SceneRenderer {
    /// GPU pipelines and buffers — immutable after init.
    pub pipelines: ScenePipelines,

    /// Draw counter and RenderDump log. Mutable during rendering.
    pub dump_cursor: RenderDumpCursor,
    pub command_buffer: RenderCommandBuffer,

    /// Material ring buffer for dynamic uniform offsets.
    pub material_ring: UniformRingBuffer,

    /// Per-frame mutable visual state.
    pub garbage_flavor_state: GarbageFlavorImageState,
    pub garbage_flavor_instance_idx: Option<u32>,
    pub block_impact_phases: std::collections::HashMap<i32, f32>,

    /// Renderer-side animators.
    pub danger_bar_animator: DangerBarAnimator,
    pub level_light_animator: LevelLightAnimator,
    pub opponent_board_animator: OpponentBoardAnimator,
    pub logo_shake_animator: LogoShakeAnimator,
    /// Blocks drifting behind the menu.
    pub menu_backdrop: crate::renderer::effects::menu_backdrop::MenuBackdrop,
}

impl SceneRenderer {
    /// Reset visual state when returning to menu (animators, impacts, flavor).
    pub fn reset_for_menu(&mut self) {
        self.level_light_animator = LevelLightAnimator::new();
        self.danger_bar_animator = DangerBarAnimator::new();
        self.opponent_board_animator = OpponentBoardAnimator::new();
        self.logo_shake_animator = LogoShakeAnimator::new();
        self.block_impact_phases.clear();
        self.garbage_flavor_state = GarbageFlavorImageState::default();
        self.garbage_flavor_instance_idx = None;
    }
}
