//! Matrix utilities: type alias, identity, normal matrix extraction.
//!
//! Conventions:
//! - Matrices are 4x4, stored column-major (OpenGL convention)
//! - Multiplication order: M * v (matrix * vector)
//! - Angles in radians

/// 4x4 matrix stored column-major (16 floats)
///
/// Layout: `m[col * 4 + row]`
/// - `m[0..4]`   = column 0 (x-axis)
/// - `m[4..8]`   = column 1 (y-axis)
/// - `m[8..12]`  = column 2 (z-axis)
/// - `m[12..16]` = column 3 (translation)
pub type Matrix4 = [f32; 16];

/// Identity matrix constant (convention reference; used by the tests)
#[cfg(test)]
pub const IDENTITY: Matrix4 = [
    1.0, 0.0, 0.0, 0.0, // column 0
    0.0, 1.0, 0.0, 0.0, // column 1
    0.0, 0.0, 1.0, 0.0, // column 2
    0.0, 0.0, 0.0, 1.0, // column 3
];

/// Extract normal matrix from Matrix4 (flat column-major array).
///
/// The normal matrix is the transpose of the inverse of the modelView matrix.
/// For uniform scale + rotation only (no non-uniform scaling), we can simply
/// extract the upper-left 3x3 and pad to 4x4.
pub fn normal_matrix_from_mat16(mv: &Matrix4) -> [[f32; 4]; 4] {
    // Column-major: m[col * 4 + row]
    [
        [mv[0], mv[1], mv[2], 0.0],   // Column 0 of upper-left 3x3
        [mv[4], mv[5], mv[6], 0.0],   // Column 1 of upper-left 3x3
        [mv[8], mv[9], mv[10], 0.0],  // Column 2 of upper-left 3x3
        [0.0,   0.0,   0.0,    1.0],  // Column 3 (identity)
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_matrix_from_mat16_identity() {
        let normal = normal_matrix_from_mat16(&IDENTITY);
        let expected = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        assert_eq!(normal, expected);
    }

    #[test]
    fn test_normal_matrix_from_mat16_translation_ignored() {
        // Translation (column 3) must NOT appear in the normal matrix
        let mut trans = IDENTITY;
        trans[12] = 10.0;
        trans[13] = 20.0;
        trans[14] = 30.0;
        let normal = normal_matrix_from_mat16(&trans);
        assert_eq!(normal[3][0], 0.0);
        assert_eq!(normal[3][1], 0.0);
        assert_eq!(normal[3][2], 0.0);
        assert_eq!(normal[3][3], 1.0);
    }
}
