//! The three orthogonal zoom layers (`docs` §4.5).
//!
//! Mixing zoom factors is the classic source of "the text is blurry but the
//! panel is crisp" bugs. The design keeps the three layers strictly separate:
//!
//! 1. **OS scale** — the physical display scale. It drives the swapchain
//!    resolution, the glyph rasterisation size and the 1 physical pixel grid
//!    used for static snapping.
//! 2. **App zoom** — the user's UI zoom. It scales the *logical* style
//!    metrics (Taffy sizes, standard control font size) of the outer chrome.
//! 3. **Canvas zoom** — the camera zoom. It only affects the projection of
//!    canvas primitives and their MTSDF sampling; it never touches the UI
//!    tree's logical sizes.
//!
//! Each layer is exposed through its own accessor so that a call site cannot
//! accidentally use the wrong one.

use unbound_math::clamp;

/// The three zoom layers, kept strictly orthogonal.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ZoomLayers {
    os_scale: f32,
    app_zoom: f32,
    canvas_zoom: f32,
}

impl Default for ZoomLayers {
    fn default() -> Self {
        Self::new(1.0, 1.0, 1.0)
    }
}

impl ZoomLayers {
    /// Build the three layers.
    ///
    /// All factors are clamped to a sane positive range: a zero or negative
    /// scale would make the projection matrix singular.
    #[must_use]
    pub fn new(os_scale: f32, app_zoom: f32, canvas_zoom: f32) -> Self {
        Self {
            os_scale: positive(os_scale),
            app_zoom: positive(app_zoom),
            canvas_zoom: positive(canvas_zoom),
        }
    }

    /// The physical display scale (layer 1).
    #[must_use]
    pub const fn os_scale(self) -> f32 {
        self.os_scale
    }

    /// The application UI zoom (layer 2).
    #[must_use]
    pub const fn app_zoom(self) -> f32 {
        self.app_zoom
    }

    /// The canvas camera zoom (layer 3).
    #[must_use]
    pub const fn canvas_zoom(self) -> f32 {
        self.canvas_zoom
    }

    /// The factor that turns UI logical units into physical pixels.
    ///
    /// Only the OS scale and the app zoom participate: the canvas zoom is
    /// deliberately excluded.
    #[must_use]
    pub fn ui_scale(self) -> f32 {
        self.os_scale * self.app_zoom
    }

    /// The factor that turns canvas world units into physical pixels.
    #[must_use]
    pub fn canvas_scale(self) -> f32 {
        self.os_scale * self.canvas_zoom
    }

    /// The size of one physical pixel expressed in UI logical units.
    ///
    /// Static geometry snaps to this grid, which is what keeps 1 px borders
    /// crisp at any OS scale.
    #[must_use]
    pub fn physical_pixel(self) -> f32 {
        1.0 / self.ui_scale()
    }

    /// The size of one physical pixel expressed in canvas world units.
    #[must_use]
    pub fn canvas_pixel(self) -> f32 {
        1.0 / self.canvas_scale()
    }

    /// Convert a UI logical length into physical pixels.
    #[must_use]
    pub fn to_physical(self, logical: f32) -> f32 {
        logical * self.ui_scale()
    }

    /// Convert a physical pixel count into UI logical units.
    #[must_use]
    pub fn to_logical(self, physical: f32) -> f32 {
        physical / self.ui_scale()
    }

    /// Convert a canvas world length into physical pixels.
    #[must_use]
    pub fn world_to_physical(self, world: f32) -> f32 {
        world * self.canvas_scale()
    }

    /// Convert a physical pixel count into canvas world units.
    #[must_use]
    pub fn physical_to_world(self, physical: f32) -> f32 {
        physical / self.canvas_scale()
    }

    /// The app zoom, clamped to the range a user interface should allow.
    #[must_use]
    pub fn clamped_app_zoom(&self) -> f32 {
        clamp(self.app_zoom, 0.25, 4.0)
    }

    /// The canvas zoom, clamped to the range the canvas supports.
    ///
    /// The design allows 10 % … 6400 %; outside that range the chunked
    /// coordinate system loses precision.
    #[must_use]
    pub fn clamped_canvas_zoom(&self) -> f32 {
        clamp(self.canvas_zoom, 0.1, 64.0)
    }
}

/// Keep a scale factor finite and strictly positive.
fn positive(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_layers_stay_orthogonal() {
        let layers = ZoomLayers::new(2.0, 1.5, 8.0);
        // The UI scale must not see the canvas zoom.
        assert_eq!(layers.ui_scale(), 3.0);
        assert_eq!(layers.canvas_scale(), 16.0);
        assert_ne!(layers.ui_scale(), layers.canvas_scale());
    }

    #[test]
    fn a_static_pixel_snap_grid_scales_with_the_os_and_app_zoom_only() {
        let a = ZoomLayers::new(1.0, 1.0, 4.0);
        let b = ZoomLayers::new(2.0, 1.0, 4.0);
        assert_eq!(a.physical_pixel(), 1.0);
        assert_eq!(b.physical_pixel(), 0.5);
        // The canvas zoom does not move the UI snap grid: doubling the OS
        // scale halves the logical size of one physical pixel.
        assert_eq!(a.physical_pixel(), b.physical_pixel() * 2.0);
    }

    #[test]
    fn conversions_round_trip() {
        let layers = ZoomLayers::new(1.5, 1.25, 3.0);
        assert!((layers.to_logical(layers.to_physical(37.0)) - 37.0).abs() < 1e-3);
        assert!((layers.physical_to_world(layers.world_to_physical(12.5)) - 12.5).abs() < 1e-3);
    }

    #[test]
    fn degenerate_scales_fall_back_to_one() {
        let layers = ZoomLayers::new(0.0, -2.0, f32::NAN);
        assert_eq!(layers.os_scale(), 1.0);
        assert_eq!(layers.app_zoom(), 1.0);
        assert_eq!(layers.canvas_zoom(), 1.0);
    }

    #[test]
    fn user_zoom_is_clamped_to_a_usable_range() {
        let layers = ZoomLayers::new(1.0, 100.0, 1000.0);
        assert_eq!(layers.clamped_app_zoom(), 4.0);
        assert_eq!(layers.clamped_canvas_zoom(), 64.0);
        let small = ZoomLayers::new(1.0, 0.01, 0.001);
        assert_eq!(small.clamped_app_zoom(), 0.25);
        assert_eq!(small.clamped_canvas_zoom(), 0.1);
    }

    #[test]
    fn the_canvas_supports_the_documented_ten_to_sixty_four_hundred_percent() {
        let layers = ZoomLayers::new(1.0, 1.0, 64.0);
        assert_eq!(layers.clamped_canvas_zoom(), 64.0);
        assert_eq!(layers.clamped_canvas_zoom() * 100.0, 6400.0);
    }
}
