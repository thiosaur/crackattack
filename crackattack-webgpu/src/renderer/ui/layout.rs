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
pub const LOGO_Y: f32 = 14.1;
/// Logo half-width and half-height (869×740 = 1.17:1 aspect), 80% of the original 4.2.
pub const LOGO_HW: f32 = 4.2 * 0.8; // 3.36
pub const LOGO_HH: f32 = LOGO_HW * 740.0 / 869.0; // ≈ 2.86

/// Logo float (menu and in-game logo), after the web page's but half as wide: a 6 s
/// loop that lifts the logo by about 1% of its width and tilts it by 0.3° to the left
/// halfway, eased in and out.
pub const LOGO_FLOAT_PERIOD: f64 = 6.0;
pub const LOGO_FLOAT_LIFT: f32 = 4.5 / 420.0;
pub const LOGO_FLOAT_TILT_DEG: f32 = -0.3;

/// Logo float at `time` (seconds), in screen convention: (vertical offset as a fraction
/// of the logo width, negative = up; rotation in radians, negative = to the left).
pub fn logo_float(time: f64) -> (f32, f32) {
    let phase = (time.rem_euclid(LOGO_FLOAT_PERIOD) / LOGO_FLOAT_PERIOD) as f32;
    // Up during the first half, down during the second, each eased (smoothstep, close to
    // CSS `ease-in-out`).
    let t = if phase < 0.5 { phase * 2.0 } else { (1.0 - phase) * 2.0 };
    let s = t * t * (3.0 - 2.0 * t);
    (-LOGO_FLOAT_LIFT * s, LOGO_FLOAT_TILT_DEG.to_radians() * s)
}

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
    fn logo_rests_at_the_loop_ends_and_peaks_halfway() {
        assert_eq!(logo_float(0.0), (0.0, 0.0));
        assert_eq!(logo_float(LOGO_FLOAT_PERIOD), (0.0, 0.0));
        let (lift, tilt) = logo_float(LOGO_FLOAT_PERIOD / 2.0);
        assert!((lift + LOGO_FLOAT_LIFT).abs() < 1e-6);
        assert!((tilt - LOGO_FLOAT_TILT_DEG.to_radians()).abs() < 1e-6);
    }

    #[test]
    fn logo_float_is_symmetric_and_bounded() {
        for i in 0..60 {
            let t = i as f64 * 0.1;
            let (a, _) = logo_float(t);
            let (b, _) = logo_float(LOGO_FLOAT_PERIOD - t);
            assert!((a - b).abs() < 1e-5);
            assert!((-LOGO_FLOAT_LIFT..=0.0).contains(&a));
        }
    }

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
