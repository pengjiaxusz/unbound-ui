//! 三个正交的缩放层级（`docs` §4.5）。
//!
//! 混用缩放因子是“文字模糊但面板清晰”类缺陷的经典根源。本设计将三层严格分离：
//!
//!
//! 1. **OS scale（系统缩放）** — 物理显示缩放比。决定交换链分辨率、
//!    字形光栅化尺寸以及用于静态吸附的 1 物理像素网格。
//!
//! 2. **App zoom（应用缩放）** — 用户的 UI 缩放比例。缩放外围界面框架的
//!    *逻辑*样式度量（Taffy 尺寸、标准控件字号）。
//!
//! 3. **Canvas zoom（画布缩放）** — 相机缩放倍率。仅影响画布图元的投影及
//!    其 MTSDF 采样；绝不触动 UI 树的逻辑尺寸。
//!
//! 每一层都通过独立的访问器暴露，使调用方不可能误用错误层级。
//!

use unbound_math::clamp;

/// 严格正交的三层缩放模型。
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
    /// 构建三层缩放配置。
    ///
    /// 所有因子均被钳制在合理的正数范围内：零或负缩放会导致投影矩阵奇异。
    ///
    #[must_use]
    pub fn new(os_scale: f32, app_zoom: f32, canvas_zoom: f32) -> Self {
        Self {
            os_scale: positive(os_scale),
            app_zoom: positive(app_zoom),
            canvas_zoom: positive(canvas_zoom),
        }
    }

    /// 物理显示缩放比（第 1 层）。
    #[must_use]
    pub const fn os_scale(self) -> f32 {
        self.os_scale
    }

    /// 应用 UI 缩放比例（第 2 层）。
    #[must_use]
    pub const fn app_zoom(self) -> f32 {
        self.app_zoom
    }

    /// 画布相机缩放倍率（第 3 层）。
    #[must_use]
    pub const fn canvas_zoom(self) -> f32 {
        self.canvas_zoom
    }

    /// 将 UI 逻辑单位转换为物理像素的因子。
    ///
    /// 仅系统缩放与应用缩放参与计算：画布缩放被刻意排除。
    ///
    #[must_use]
    pub fn ui_scale(self) -> f32 {
        self.os_scale * self.app_zoom
    }

    /// 将画布世界单位转换为物理像素的因子。
    #[must_use]
    pub fn canvas_scale(self) -> f32 {
        self.os_scale * self.canvas_zoom
    }

    /// 以 UI 逻辑单位表示的 1 个物理像素大小。
    ///
    /// 静态几何体吸附到此网格，从而在任意系统缩放下都能保持 1 px 边框锐利。
    ///
    #[must_use]
    pub fn physical_pixel(self) -> f32 {
        1.0 / self.ui_scale()
    }

    /// 以画布世界单位表示的 1 个物理像素大小。
    #[must_use]
    pub fn canvas_pixel(self) -> f32 {
        1.0 / self.canvas_scale()
    }

    /// 将 UI 逻辑长度转换为物理像素。
    #[must_use]
    pub fn to_physical(self, logical: f32) -> f32 {
        logical * self.ui_scale()
    }

    /// 将物理像素数量转换为 UI 逻辑单位。
    #[must_use]
    pub fn to_logical(self, physical: f32) -> f32 {
        physical / self.ui_scale()
    }

    /// 将画布世界长度转换为物理像素。
    #[must_use]
    pub fn world_to_physical(self, world: f32) -> f32 {
        world * self.canvas_scale()
    }

    /// 将物理像素数量转换为画布世界单位。
    #[must_use]
    pub fn physical_to_world(self, physical: f32) -> f32 {
        physical / self.canvas_scale()
    }

    /// 应用缩放比例，钳制在用户界面允许的范围内。
    #[must_use]
    pub fn clamped_app_zoom(&self) -> f32 {
        clamp(self.app_zoom, 0.25, 4.0)
    }

    /// 画布缩放倍率，钳制在画布支持的范围内。
    ///
    /// 设计允许 10% … 6400%；超出该范围后分块坐标系统会损失精度。
    ///
    #[must_use]
    pub fn clamped_canvas_zoom(&self) -> f32 {
        clamp(self.canvas_zoom, 0.1, 64.0)
    }
}

/// 确保缩放因子为有限且严格正值。
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
        // UI 缩放绝不可受画布缩放影响。
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
        // 画布缩放不会移动 UI 吸附网格：系统缩放翻倍会使 1 物理像素对应的逻辑尺寸减半。
        //
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
