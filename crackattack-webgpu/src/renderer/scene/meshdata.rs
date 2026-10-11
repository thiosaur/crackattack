//! Hardcoded mesh vertex data
//!
//! This module provides static mesh data for all builtin meshes.
//! The data is hardcoded to match the original mesh geometry exactly.

use crate::renderer::gpu::types::MeshVertex;

// The garbage shell meshes below are triangle strips with 3D texture coordinates.
// CoordConverter = -1 / (2.0 * 6) = -0.0833333
// The flavor quad is built inline in `build_garbage_shell_buffer`.

/// Lightmap coordinate converter for garbage meshes
/// Formula: -1.0 / (GRID_ELEMENT_LENGTH * GC_PLAY_WIDTH)
// Single definition (in `lightmap`), so the mesh and instance UV scales cannot diverge.
pub use crate::renderer::scene::lightmap::GARBAGE_LIGHTMAP_COORD_CONVERTER;

// All 6 garbage shell meshes with data split into separate arrays.

/// A single garbage mesh as a triangle list with separated attributes
pub struct GarbageShellMesh {
    /// Vertex positions (3 floats per vertex)
    pub positions: Vec<[f32; 3]>,
    /// Vertex normals (3 floats per vertex)
    pub normals: Vec<[f32; 3]>,
    /// Vertex UVs (3 floats per vertex, derived from positions * -1/12)
    pub uvs: Vec<[f32; 3]>,
    /// Diffuse texture UVs (2 floats per vertex) — per-face projection
    pub uv_diffuse: Vec<[f32; 2]>,
}

/// All 6 garbage shell meshes
pub struct GarbageShellMeshes {
    pub thick_corner: GarbageShellMesh,
    pub thick_edge: GarbageShellMesh,
    pub thick_middle: GarbageShellMesh,
    pub thin_cap: GarbageShellMesh,
    pub thin_middle: GarbageShellMesh,
    pub small: GarbageShellMesh,
}

/// Convert a triangle strip (positions + normals) into a triangle list GarbageShellMesh.
/// UVs are computed as position * (-1/12).
/// Compute diffuse UV for a garbage shell vertex based on its normal orientation.
/// Front/back (Z dominant): UV from (X, Y) — main face, most visible
/// Top/bottom (Y dominant): UV from (X, Z) — horizontal faces
/// Sides (X dominant): UV from (Z, Y) — lateral faces
/// Bevels get a smooth transition since their normals are diagonal.
/// All UVs are in cell-local space [-1,1] → [0,1].
fn diffuse_uv_for_vertex(pos: [f32; 3], normal: [f32; 3]) -> [f32; 2] {
    let ax = normal[0].abs();
    let ay = normal[1].abs();
    let az = normal[2].abs();
    if az >= ax && az >= ay {
        // Front/back face: project XY
        [(pos[0] + 1.0) / 2.0, (pos[1] + 1.0) / 2.0]
    } else if ay >= ax {
        // Top/bottom face: project XZ
        [(pos[0] + 1.0) / 2.0, (pos[2] + 1.0) / 2.0]
    } else {
        // Side face: project ZY
        [(pos[2] + 1.0) / 2.0, (pos[1] + 1.0) / 2.0]
    }
}

fn shell_strip_to_triangles(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
) -> GarbageShellMesh {
    let cc = GARBAGE_LIGHTMAP_COORD_CONVERTER;
    let mut out_pos = Vec::new();
    let mut out_norm = Vec::new();
    let mut out_uv = Vec::new();
    let mut out_uv_diffuse = Vec::new();

    if positions.len() < 3 {
        return GarbageShellMesh { positions: out_pos, normals: out_norm, uvs: out_uv, uv_diffuse: out_uv_diffuse };
    }

    for i in 0..(positions.len() - 2) {
        let (i0, i1, i2) = if i % 2 == 0 {
            (i, i + 1, i + 2)
        } else {
            (i + 1, i, i + 2)
        };

        let v0 = positions[i0];
        let v1 = positions[i1];
        let v2 = positions[i2];

        if (v0[0] == v1[0] && v0[1] == v1[1] && v0[2] == v1[2])
            || (v1[0] == v2[0] && v1[1] == v2[1] && v1[2] == v2[2])
            || (v0[0] == v2[0] && v0[1] == v2[1] && v0[2] == v2[2])
        {
            continue;
        }

        for &idx in &[i0, i1, i2] {
            let p = positions[idx];
            let n = normals[idx];
            out_pos.push(p);
            out_norm.push(n);
            out_uv.push([p[0] * cc, p[1] * cc, p[2] * cc]);
            out_uv_diffuse.push(diffuse_uv_for_vertex(p, n));
        }
    }

    GarbageShellMesh { positions: out_pos, normals: out_norm, uvs: out_uv, uv_diffuse: out_uv_diffuse }
}

/// Merge multiple GarbageShellMesh results into one by concatenation.
fn merge_shell_meshes(meshes: Vec<GarbageShellMesh>) -> GarbageShellMesh {
    let total: usize = meshes.iter().map(|m| m.positions.len()).sum();
    let mut out = GarbageShellMesh {
        positions: Vec::with_capacity(total),
        normals: Vec::with_capacity(total),
        uvs: Vec::with_capacity(total),
        uv_diffuse: Vec::with_capacity(total),
    };
    for m in meshes {
        out.positions.extend(m.positions);
        out.normals.extend(m.normals);
        out.uvs.extend(m.uvs);
        out.uv_diffuse.extend(m.uv_diffuse);
    }
    out
}

/// Generate all 6 garbage shell meshes as triangle lists with separated attributes.
pub fn generate_garbage_shell_meshes() -> GarbageShellMeshes {
    GarbageShellMeshes {
        thick_corner: generate_shell_thick_corner(),
        thick_edge: generate_shell_thick_edge(),
        thick_middle: generate_shell_thick_middle(),
        thin_cap: generate_shell_thin_cap(),
        thin_middle: generate_shell_thin_middle(),
        small: generate_shell_small(),
    }
}

impl GarbageShellMesh {
    /// Convert to Vec<MeshVertex> for GPU upload
    pub fn to_mesh_vertices(&self) -> Vec<MeshVertex> {
        self.positions.iter()
            .zip(self.normals.iter())
            .zip(self.uvs.iter())
            .zip(self.uv_diffuse.iter())
            .map(|(((p, n), u), ud)| MeshVertex { pos: *p, normal: *n, uv: *u, uv_diffuse: *ud })
            .collect()
    }
}

/// Vertex ranges for each garbage shell mesh type in a concatenated buffer.
/// Each draw call uses the range of its mesh type.
#[derive(Debug, Clone, Copy)]
pub struct GarbageShellRange {
    pub vertex_offset: u32,
    pub vertex_count: u32,
}

/// All garbage shell mesh ranges in a concatenated vertex buffer.
#[derive(Debug, Clone, Copy)]
pub struct GarbageShellRanges {
    pub thick_corner: GarbageShellRange,
    pub thick_edge: GarbageShellRange,
    pub thick_middle: GarbageShellRange,
    pub thin_cap: GarbageShellRange,
    pub thin_middle: GarbageShellRange,
    pub small: GarbageShellRange,
    /// Flat 2×2-cell quad for garbage flavor image overlay.
    pub flavor_quad: GarbageShellRange,
}

/// Build a concatenated vertex buffer and return (vertices, ranges).
pub fn build_garbage_shell_buffer() -> (Vec<MeshVertex>, GarbageShellRanges) {
    let meshes = generate_garbage_shell_meshes();
    let mut all_vertices = Vec::new();
    let mut offset = 0u32;

    let mut add_mesh = |mesh: &GarbageShellMesh| -> GarbageShellRange {
        let verts = mesh.to_mesh_vertices();
        let count = verts.len() as u32;
        let range = GarbageShellRange { vertex_offset: offset, vertex_count: count };
        all_vertices.extend(verts);
        offset += count;
        range
    };

    let thick_corner = add_mesh(&meshes.thick_corner);
    let thick_edge = add_mesh(&meshes.thick_edge);
    let thick_middle = add_mesh(&meshes.thick_middle);
    let thin_cap = add_mesh(&meshes.thin_cap);
    let thin_middle = add_mesh(&meshes.thin_middle);
    let small = add_mesh(&meshes.small);

    // Flat 2×2-cell quad for flavor image overlay.
    // Centered at origin, extends ±2 in X/Y (= 2 cells of GRID_ELEMENT_LENGTH=2).
    // Z=0, normal (0,0,1). UV [0,1]² across the full quad.
    let flavor_quad_verts: Vec<MeshVertex> = {
        let n = [0.0_f32, 0.0, 1.0];
        vec![
            MeshVertex { pos: [-2.0, -2.0, 0.0], normal: n, uv: [0.0, 0.0, 0.0], uv_diffuse: [0.0, 0.0] },
            MeshVertex { pos: [ 2.0, -2.0, 0.0], normal: n, uv: [1.0, 0.0, 0.0], uv_diffuse: [0.0, 0.0] },
            MeshVertex { pos: [ 2.0,  2.0, 0.0], normal: n, uv: [1.0, 1.0, 0.0], uv_diffuse: [0.0, 0.0] },
            MeshVertex { pos: [-2.0, -2.0, 0.0], normal: n, uv: [0.0, 0.0, 0.0], uv_diffuse: [0.0, 0.0] },
            MeshVertex { pos: [ 2.0,  2.0, 0.0], normal: n, uv: [1.0, 1.0, 0.0], uv_diffuse: [0.0, 0.0] },
            MeshVertex { pos: [-2.0,  2.0, 0.0], normal: n, uv: [0.0, 1.0, 0.0], uv_diffuse: [0.0, 0.0] },
        ]
    };
    let fq_count = flavor_quad_verts.len() as u32;
    let flavor_quad = GarbageShellRange { vertex_offset: offset, vertex_count: fq_count };
    all_vertices.extend(flavor_quad_verts);

    let ranges = GarbageShellRanges {
        thick_corner, thick_edge, thick_middle,
        thin_cap, thin_middle, small, flavor_quad,
    };

    (all_vertices, ranges)
}

fn generate_shell_thick_corner() -> GarbageShellMesh {
    // Strip 0: main wrap (12 verts)
    #[rustfmt::skip]
    let pos0: [[f32; 3]; 12] = [
        [1.0, -1.0, -1.0],  [-0.8, -1.0, -1.0],
        [1.0, 0.8, -1.0],   [-0.8, 0.8, -1.0],
        [1.0, 1.0, -0.8],   [-0.8, 1.0, -0.8],
        [1.0, 1.0, 0.8],    [-0.8, 1.0, 0.8],
        [1.0, 0.8, 1.0],    [-0.8, 0.8, 1.0],
        [1.0, -1.0, 1.0],   [-0.8, -1.0, 1.0],
    ];
    #[rustfmt::skip]
    let norm0: [[f32; 3]; 12] = [
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
    ];

    // Strip 1: left side bevel (8 verts)
    #[rustfmt::skip]
    let pos1: [[f32; 3]; 8] = [
        [-0.8, -1.0, -1.0], [-1.0, -1.0, -0.8],
        [-0.8, 0.8, -1.0],  [-1.0, 0.8, -0.8],
        [-0.8, 1.0, -0.8],  [-1.0, 0.8, 0.8],
        [-0.8, 1.0, 0.8],   [-0.8, 0.8, 1.0],
    ];
    #[rustfmt::skip]
    let norm1: [[f32; 3]; 8] = [
        [0.0, 0.0, -1.0],  [-1.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],  [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],   [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],   [0.0, 0.0, 1.0],
    ];

    // Strip 2: left face + back bevel (6 verts)
    #[rustfmt::skip]
    let pos2: [[f32; 3]; 6] = [
        [-1.0, 0.8, -0.8],  [-1.0, -1.0, -0.8],
        [-1.0, 0.8, 0.8],   [-1.0, -1.0, 0.8],
        [-0.8, 0.8, 1.0],   [-0.8, -1.0, 1.0],
    ];
    #[rustfmt::skip]
    let norm2: [[f32; 3]; 6] = [
        [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
    ];

    merge_shell_meshes(vec![
        shell_strip_to_triangles(&pos0, &norm0),
        shell_strip_to_triangles(&pos1, &norm1),
        shell_strip_to_triangles(&pos2, &norm2),
    ])
}

fn generate_shell_thick_edge() -> GarbageShellMesh {
    // Single strip (12 verts)
    #[rustfmt::skip]
    let pos: [[f32; 3]; 12] = [
        [1.0, -1.0, -1.0],  [-1.0, -1.0, -1.0],
        [1.0, 0.8, -1.0],   [-1.0, 0.8, -1.0],
        [1.0, 1.0, -0.8],   [-1.0, 1.0, -0.8],
        [1.0, 1.0, 0.8],    [-1.0, 1.0, 0.8],
        [1.0, 0.8, 1.0],    [-1.0, 0.8, 1.0],
        [1.0, -1.0, 1.0],   [-1.0, -1.0, 1.0],
    ];
    #[rustfmt::skip]
    let norm: [[f32; 3]; 12] = [
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
    ];

    shell_strip_to_triangles(&pos, &norm)
}

fn generate_shell_thick_middle() -> GarbageShellMesh {
    // Strip 0: front face (Z-)
    #[rustfmt::skip]
    let pos0: [[f32; 3]; 4] = [
        [1.0, -1.0, -1.0], [-1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],  [-1.0, 1.0, -1.0],
    ];
    #[rustfmt::skip]
    let norm0: [[f32; 3]; 4] = [
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
    ];

    // Strip 1: back face (Z+)
    #[rustfmt::skip]
    let pos1: [[f32; 3]; 4] = [
        [-1.0, -1.0, 1.0], [1.0, -1.0, 1.0],
        [-1.0, 1.0, 1.0],  [1.0, 1.0, 1.0],
    ];
    #[rustfmt::skip]
    let norm1: [[f32; 3]; 4] = [
        [0.0, 0.0, 1.0], [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0], [0.0, 0.0, 1.0],
    ];

    merge_shell_meshes(vec![
        shell_strip_to_triangles(&pos0, &norm0),
        shell_strip_to_triangles(&pos1, &norm1),
    ])
}

fn generate_shell_thin_cap() -> GarbageShellMesh {
    // Strip 0: main tube with cap (18 verts)
    #[rustfmt::skip]
    let pos0: [[f32; 3]; 18] = [
        [-0.8, -0.8, -1.0], [1.0, -0.8, -1.0],
        [-0.8, -1.0, -0.8], [1.0, -1.0, -0.8],
        [-0.8, -1.0, 0.8],  [1.0, -1.0, 0.8],
        [-0.8, -0.8, 1.0],  [1.0, -0.8, 1.0],
        [-0.8, 0.8, 1.0],   [1.0, 0.8, 1.0],
        [-0.8, 1.0, 0.8],   [1.0, 1.0, 0.8],
        [-0.8, 1.0, -0.8],  [1.0, 1.0, -0.8],
        [-0.8, 0.8, -1.0],  [1.0, 0.8, -1.0],
        [-0.8, -0.8, -1.0], [1.0, -0.8, -1.0],
    ];
    #[rustfmt::skip]
    let norm0: [[f32; 3]; 18] = [
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, -1.0, 0.0], [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0], [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
    ];

    // Strip 1: left side bevel (8 verts)
    #[rustfmt::skip]
    let pos1: [[f32; 3]; 8] = [
        [-0.8, -0.8, -1.0], [-1.0, -0.8, -0.8],
        [-0.8, 0.8, -1.0],  [-1.0, 0.8, -0.8],
        [-0.8, 1.0, -0.8],  [-1.0, 0.8, 0.8],
        [-0.8, 1.0, 0.8],   [-0.8, 0.8, 1.0],
    ];
    #[rustfmt::skip]
    let norm1: [[f32; 3]; 8] = [
        [0.0, 0.0, -1.0],  [-1.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],  [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],   [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],   [0.0, 0.0, 1.0],
    ];

    // Strip 2: left side back bevel (8 verts)
    #[rustfmt::skip]
    let pos2: [[f32; 3]; 8] = [
        [-0.8, 0.8, 1.0],   [-1.0, 0.8, 0.8],
        [-0.8, -0.8, 1.0],  [-1.0, -0.8, 0.8],
        [-0.8, -1.0, 0.8],  [-1.0, -0.8, -0.8],
        [-0.8, -1.0, -0.8], [-0.8, -0.8, -1.0],
    ];
    #[rustfmt::skip]
    let norm2: [[f32; 3]; 8] = [
        [0.0, 0.0, 1.0],  [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],  [-1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0], [-1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0], [0.0, 0.0, -1.0],
    ];

    // Strip 3: left flat face (4 verts)
    #[rustfmt::skip]
    let pos3: [[f32; 3]; 4] = [
        [-1.0, 0.8, 0.8],  [-1.0, 0.8, -0.8],
        [-1.0, -0.8, 0.8], [-1.0, -0.8, -0.8],
    ];
    #[rustfmt::skip]
    let norm3: [[f32; 3]; 4] = [
        [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0],
    ];

    merge_shell_meshes(vec![
        shell_strip_to_triangles(&pos0, &norm0),
        shell_strip_to_triangles(&pos1, &norm1),
        shell_strip_to_triangles(&pos2, &norm2),
        shell_strip_to_triangles(&pos3, &norm3),
    ])
}

fn generate_shell_thin_middle() -> GarbageShellMesh {
    // Single strip (18 verts)
    #[rustfmt::skip]
    let pos: [[f32; 3]; 18] = [
        [-1.0, -0.8, -1.0], [1.0, -0.8, -1.0],
        [-1.0, -1.0, -0.8], [1.0, -1.0, -0.8],
        [-1.0, -1.0, 0.8],  [1.0, -1.0, 0.8],
        [-1.0, -0.8, 1.0],  [1.0, -0.8, 1.0],
        [-1.0, 0.8, 1.0],   [1.0, 0.8, 1.0],
        [-1.0, 1.0, 0.8],   [1.0, 1.0, 0.8],
        [-1.0, 1.0, -0.8],  [1.0, 1.0, -0.8],
        [-1.0, 0.8, -1.0],  [1.0, 0.8, -1.0],
        [-1.0, -0.8, -1.0], [1.0, -0.8, -1.0],
    ];
    #[rustfmt::skip]
    let norm: [[f32; 3]; 18] = [
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, -1.0, 0.0], [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0], [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
    ];

    shell_strip_to_triangles(&pos, &norm)
}

fn generate_shell_small() -> GarbageShellMesh {
    // Strip 0: main tube (18 verts, beveled on both X ends)
    #[rustfmt::skip]
    let pos0: [[f32; 3]; 18] = [
        [-0.8, -0.8, -1.0], [0.8, -0.8, -1.0],
        [-0.8, -1.0, -0.8], [0.8, -1.0, -0.8],
        [-0.8, -1.0, 0.8],  [0.8, -1.0, 0.8],
        [-0.8, -0.8, 1.0],  [0.8, -0.8, 1.0],
        [-0.8, 0.8, 1.0],   [0.8, 0.8, 1.0],
        [-0.8, 1.0, 0.8],   [0.8, 1.0, 0.8],
        [-0.8, 1.0, -0.8],  [0.8, 1.0, -0.8],
        [-0.8, 0.8, -1.0],  [0.8, 0.8, -1.0],
        [-0.8, -0.8, -1.0], [0.8, -0.8, -1.0],
    ];
    #[rustfmt::skip]
    let norm0: [[f32; 3]; 18] = [
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, -1.0, 0.0], [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0], [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],  [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],  [0.0, 1.0, 0.0],
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
    ];

    // Strip 1: left side top chamfer (8 verts, same as ThinCap strip 1)
    #[rustfmt::skip]
    let pos1: [[f32; 3]; 8] = [
        [-0.8, -0.8, -1.0], [-1.0, -0.8, -0.8],
        [-0.8, 0.8, -1.0],  [-1.0, 0.8, -0.8],
        [-0.8, 1.0, -0.8],  [-1.0, 0.8, 0.8],
        [-0.8, 1.0, 0.8],   [-0.8, 0.8, 1.0],
    ];
    #[rustfmt::skip]
    let norm1: [[f32; 3]; 8] = [
        [0.0, 0.0, -1.0],  [-1.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],  [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],   [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],   [0.0, 0.0, 1.0],
    ];

    // Strip 2: left side back chamfer (8 verts, same as ThinCap strip 2)
    #[rustfmt::skip]
    let pos2: [[f32; 3]; 8] = [
        [-0.8, 0.8, 1.0],   [-1.0, 0.8, 0.8],
        [-0.8, -0.8, 1.0],  [-1.0, -0.8, 0.8],
        [-0.8, -1.0, 0.8],  [-1.0, -0.8, -0.8],
        [-0.8, -1.0, -0.8], [-0.8, -0.8, -1.0],
    ];
    #[rustfmt::skip]
    let norm2: [[f32; 3]; 8] = [
        [0.0, 0.0, 1.0],  [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],  [-1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0], [-1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0], [0.0, 0.0, -1.0],
    ];

    // Strip 3: left flat face (4 verts, same as ThinCap strip 3)
    #[rustfmt::skip]
    let pos3: [[f32; 3]; 4] = [
        [-1.0, 0.8, 0.8],  [-1.0, 0.8, -0.8],
        [-1.0, -0.8, 0.8], [-1.0, -0.8, -0.8],
    ];
    #[rustfmt::skip]
    let norm3: [[f32; 3]; 4] = [
        [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0],
    ];

    // Strip 4: right flat face (4 verts)
    #[rustfmt::skip]
    let pos4: [[f32; 3]; 4] = [
        [1.0, 0.8, 0.8],  [1.0, -0.8, 0.8],
        [1.0, 0.8, -0.8], [1.0, -0.8, -0.8],
    ];
    #[rustfmt::skip]
    let norm4: [[f32; 3]; 4] = [
        [1.0, 0.0, 0.0], [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0], [1.0, 0.0, 0.0],
    ];

    // Strip 5: right top chamfer (8 verts)
    #[rustfmt::skip]
    let pos5: [[f32; 3]; 8] = [
        [0.8, 1.0, 0.8],   [1.0, 0.8, 0.8],
        [0.8, 1.0, -0.8],  [1.0, 0.8, -0.8],
        [0.8, 0.8, -1.0],  [1.0, -0.8, -0.8],
        [0.8, -0.8, -1.0], [0.8, -1.0, -0.8],
    ];
    #[rustfmt::skip]
    let norm5: [[f32; 3]; 8] = [
        [0.0, 1.0, 0.0],  [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],  [1.0, 0.0, 0.0],
        [0.0, 0.0, -1.0], [1.0, 0.0, 0.0],
        [0.0, 0.0, -1.0], [0.0, -1.0, 0.0],
    ];

    // Strip 6: right bottom chamfer (8 verts)
    #[rustfmt::skip]
    let pos6: [[f32; 3]; 8] = [
        [0.8, -1.0, -0.8],  [1.0, -0.8, -0.8],
        [0.8, -1.0, 0.8],   [1.0, -0.8, 0.8],
        [0.8, -0.8, 1.0],   [1.0, 0.8, 0.8],
        [0.8, 0.8, 1.0],    [0.8, 1.0, 0.8],
    ];
    #[rustfmt::skip]
    let norm6: [[f32; 3]; 8] = [
        [0.0, -1.0, 0.0], [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0], [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],  [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],  [0.0, 1.0, 0.0],
    ];

    merge_shell_meshes(vec![
        shell_strip_to_triangles(&pos0, &norm0),
        shell_strip_to_triangles(&pos1, &norm1),
        shell_strip_to_triangles(&pos2, &norm2),
        shell_strip_to_triangles(&pos3, &norm3),
        shell_strip_to_triangles(&pos4, &norm4),
        shell_strip_to_triangles(&pos5, &norm5),
        shell_strip_to_triangles(&pos6, &norm6),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_buffer_flavor_quad() {
        // The flavor quad in the shell buffer: 6 vertices, one full 2x2 cell.
        let (_verts, ranges) = build_garbage_shell_buffer();
        assert_eq!(ranges.flavor_quad.vertex_count, 6);
    }
}
