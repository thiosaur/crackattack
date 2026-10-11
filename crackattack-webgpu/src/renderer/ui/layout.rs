//! Centralized layout constants for UI element placement.
//!
//! Declarative layout based on named zones and relative positioning.
//!
//! The coordinate system is world units at Z=-42.5 (the UI reference plane),
//! projected through the same perspective frustum as the game blocks.
//!
//! Frustum: near=38, far=65, left=-15.6, right=8.4, bottom=-6, top=18
//! At Z=-42.5: scale = 42.5/38.0 = 1.11842×
//!   visible width  = 24.0 × 42.5/38.0 = 510/19 ≈ 26.842
//!   visible height = same (1:1 aspect)
//!   center = (-3.6, 6.0) × 42.5/38.0 ≈ (-4.03, 6.71). This is not the world origin,
//!   because the frustum is asymmetric.

/// A rectangular region in UI world space.
/// All coordinates are at the UI reference plane (Z = UI_Z).
#[derive(Debug, Clone, Copy)]
pub struct LayoutBox {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
}

impl LayoutBox {
    pub const fn new(x_min: f32, x_max: f32, y_min: f32, y_max: f32) -> Self {
        Self { x_min, x_max, y_min, y_max }
    }

    pub fn center_x(&self) -> f32 { (self.x_min + self.x_max) * 0.5 }
    pub fn center_y(&self) -> f32 { (self.y_min + self.y_max) * 0.5 }

    /// Width of the box (test-only accessor)
    #[cfg(test)]
    pub fn width(&self) -> f32 { self.x_max - self.x_min }
    /// Height of the box (test-only accessor)
    #[cfg(test)]
    pub fn height(&self) -> f32 { self.y_max - self.y_min }
}

/// UI reference plane depth (all flat UI elements).
pub const UI_Z: f32 = -42.5;

/// Countdown is rendered slightly more in front.
pub const UI_Z_FRONT: f32 = -42.3;

// The frustum and the reference rectangle have a single source: `constants.rs`. The center
// must be scaled by |z|/near too, otherwise the rectangle would not match the real
// projection (danger bars are anchored on `REF_*`, backdrop and HUD on this module).
use super::constants::{REF_LEFT, REF_RIGHT, REF_BOTTOM, REF_TOP};

/// Full visible screen at Z=-42.5.
pub const SCREEN: LayoutBox = LayoutBox::new(
    REF_LEFT,   // -17.45
    REF_RIGHT,  //   9.39
    REF_BOTTOM, //  -6.71
    REF_TOP,    //  20.13
);

// Left column positions

/// Center X of the left info column
pub const LEFT_COL_X: f32 = -10.5;

/// Logo Y position
pub const LOGO_Y: f32 = 13.5;
/// Logo half-width and half-height (869×740 = 1.17:1 aspect).
pub const LOGO_HW: f32 = 4.2;
pub const LOGO_HH: f32 = 4.2 * 740.0 / 869.0; // ≈ 3.58

/// Nameplate Y position
pub const NAME_Y: f32 = 7.0;
/// Nameplate half-width
pub const NAME_HW: f32 = 4.4;
/// Nameplate half-height (2:1 aspect)
pub const NAME_HH: f32 = 2.2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screen_dimensions() {
        let w = SCREEN.width();
        let h = SCREEN.height();
        assert!((w - 26.842).abs() < 0.01, "width = {}", w);
        assert!((h - 26.842).abs() < 0.01, "height = {}", h);
    }

    #[test]
    fn test_screen_center() {
        // Frustum center projected on the z = -42.5 plane: (-3.6, 6.0) × 42.5/38.
        assert!((SCREEN.center_x() - (-4.026)).abs() < 0.01, "center_x = {}", SCREEN.center_x());
        assert!((SCREEN.center_y() - 6.711).abs() < 0.01, "center_y = {}", SCREEN.center_y());
    }

}
