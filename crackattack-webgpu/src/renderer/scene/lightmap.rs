//! UV scale constants for the garbage shell meshes.
//!
//! `GARBAGE_LIGHTMAP_COORD_CONVERTER` scales vertex positions into the UV attribute
//! (`MeshVertex.uv`) that `meshdata` generates. The garbage shader does not read that
//! attribute; it is kept so the vertex layout stays unchanged.

use crate::renderer::scene::game_renderer::GRID_ELEMENT_LENGTH;

/// Play field width in cells.
pub const GC_PLAY_WIDTH: i32 = 6;

/// UV coordinate converter: maps a mesh position to the UV attribute.
pub const GARBAGE_LIGHTMAP_COORD_CONVERTER: f32 = -1.0 / (GRID_ELEMENT_LENGTH * GC_PLAY_WIDTH as f32);
