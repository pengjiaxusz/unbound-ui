//! 自定义离屏 3D 视口渲染 Pass 协议、轨道相机约束与物理分辨率计算。
//!
//! 统一封装 `unbound-gpu` 离屏 3D 视口渲染管线的公共契约：
//! * [`ViewportId`]：多实例 3D 视口的稳定强类型标识；
//! * [`ViewportMsaa`] 与 [`ViewportDepthMode`]：采样数（1x / 4x MSAA）与深度模式（Standard / Reverse-Z）；
//! * [`ViewportCamera3D`] 与 [`ViewportCameraConstraints`]：带俯仰/距离钳制与视图平面平移的轨道相机；
//! * [`ViewportRenderPassDesc`]、[`ViewportPassContext`] 与 [`CustomViewportPass`]：自定义录制回调契约；
//! * [`ViewportReadback`]：离屏 RGBA8 像素回读结果载体。

use ash::vk;
use unbound_math::{Rect, Vec2};

/// 3D 视口最小渲染分辨率缩放倍率（防止过低采样导致退化）。
pub const MIN_VIEWPORT_RENDER_SCALE: f32 = 0.25;

/// 3D 视口最大渲染分辨率缩放倍率（原生物理像素 1:1）。
pub const MAX_VIEWPORT_RENDER_SCALE: f32 = 1.0;

/// 3D 视口强类型唯一标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ViewportId(pub u64);

impl ViewportId {
    /// 以原始 `u64` 数值构造视口标识。
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// 返回底层原始 `u64` 数值。
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// 离屏 3D 视口多重采样抗锯齿（MSAA）级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ViewportMsaa {
    /// 单采样（关闭 MSAA）。
    #[default]
    X1,
    /// 4x MSAA（硬件不支持时由渲染器安全回退至 1x）。
    X4,
}

impl ViewportMsaa {
    /// 转换为 Vulkan `VkSampleCountFlags`。
    #[must_use]
    pub const fn vk_sample_count(self) -> vk::SampleCountFlags {
        match self {
            Self::X1 => vk::SampleCountFlags::TYPE_1,
            Self::X4 => vk::SampleCountFlags::TYPE_4,
        }
    }
}

/// 离屏 3D 视口深度缓冲约定模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ViewportDepthMode {
    /// 标准深度范围 `[0.0, 1.0]`，远平面为 `1.0`。
    #[default]
    Standard,
    /// Reverse-Z 深度范围 `[1.0, 0.0]`，远平面为 `0.0`，显著提升远距离浮点精度。
    ReverseZ,
}

impl ViewportDepthMode {
    /// 返回对应深度模式下的清屏深度值。
    #[must_use]
    pub const fn clear_depth(self) -> f32 {
        match self {
            Self::Standard => 1.0,
            Self::ReverseZ => 0.0,
        }
    }
}

/// 3D 轨道相机的俯仰角与观察距离约束。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportCameraConstraints {
    /// 允许的最小俯仰角（弧度）。
    pub min_pitch_rad: f32,
    /// 允许的最大俯仰角（弧度）。
    pub max_pitch_rad: f32,
    /// 允许的最小相机距离。
    pub min_distance: f32,
    /// 允许的最大相机距离。
    pub max_distance: f32,
}

impl Default for ViewportCameraConstraints {
    fn default() -> Self {
        let limit = 89.0_f32.to_radians();
        Self {
            min_pitch_rad: -limit,
            max_pitch_rad: limit,
            min_distance: 0.01,
            max_distance: 100_000.0,
        }
    }
}

impl ViewportCameraConstraints {
    /// 按场景包围球半径与最小/最大距离倍率推导轨道相机约束。
    ///
    /// 当 `radius`、`min_factor` 或 `max_factor` 非正或非有限值时自动回退至安全正值。
    #[must_use]
    pub fn for_bounding_radius(radius: f32, min_factor: f32, max_factor: f32) -> Self {
        let safe_radius = if radius.is_finite() && radius > 0.0 {
            radius
        } else {
            1.0
        };
        let safe_min_factor = if min_factor.is_finite() && min_factor > 0.0 {
            min_factor
        } else {
            0.01
        };
        let safe_max_factor = if max_factor.is_finite() && max_factor > 0.0 {
            max_factor
        } else {
            100.0
        };
        let min_distance = (safe_radius * safe_min_factor).max(1e-4);
        let max_distance = (safe_radius * safe_max_factor).max(min_distance);
        Self {
            min_distance,
            max_distance,
            ..Self::default()
        }
    }

    /// 消毒俯仰角与距离上下界：过滤 `NaN`/非有限值/非正距离并修正倒置区间，防止 `f32::clamp` 触发 panic。
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let defaults = Self::default();
        let raw_min_pitch = if self.min_pitch_rad.is_finite() {
            self.min_pitch_rad
        } else {
            defaults.min_pitch_rad
        };
        let raw_max_pitch = if self.max_pitch_rad.is_finite() {
            self.max_pitch_rad
        } else {
            defaults.max_pitch_rad
        };
        let min_pitch_rad = raw_min_pitch.min(raw_max_pitch);
        let max_pitch_rad = raw_min_pitch.max(raw_max_pitch);

        let raw_min_dist = if self.min_distance.is_finite() && self.min_distance > 0.0 {
            self.min_distance
        } else {
            defaults.min_distance
        };
        let raw_max_dist = if self.max_distance.is_finite() && self.max_distance > 0.0 {
            self.max_distance
        } else {
            defaults.max_distance
        };
        let min_distance = raw_min_dist.min(raw_max_dist).max(1e-6);
        let max_distance = raw_min_dist.max(raw_max_dist).max(min_distance);

        Self {
            min_pitch_rad,
            max_pitch_rad,
            min_distance,
            max_distance,
        }
    }

    /// 按消毒后的俯仰约束安全钳制角度（自动处理 `NaN` 与上下界倒置）。
    #[must_use]
    fn clamp_pitch(self, pitch: f32) -> f32 {
        let safe = self.sanitized();
        if !pitch.is_finite() {
            return 0.0_f32.clamp(safe.min_pitch_rad, safe.max_pitch_rad);
        }
        pitch.clamp(safe.min_pitch_rad, safe.max_pitch_rad)
    }

    /// 按消毒后的距离约束安全钳制观察距离（自动处理 `NaN` 与上下界倒置）。
    #[must_use]
    fn clamp_distance(self, distance: f32) -> f32 {
        let safe = self.sanitized();
        if !distance.is_finite() {
            return safe.min_distance;
        }
        distance.clamp(safe.min_distance, safe.max_distance)
    }
}

/// 3D 视口轨道相机参数（用于 3D 模型/材质球预览与网格检视）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportCamera3D {
    /// 水平旋转角（弧度）。
    pub orbit_yaw: f32,
    /// 俯仰角（弧度，受 [`Self::constraints`] 钳制）。
    pub orbit_pitch: f32,
    /// 相机到目标点的距离（受 [`Self::constraints`] 钳制）。
    pub distance: f32,
    /// 观察目标中心点。
    pub target: [f32; 3],
    /// 垂直视场角（弧度）。
    pub fov_y_rad: f32,
    /// 近裁剪面。
    pub near: f32,
    /// 远裁剪面。
    pub far: f32,
    /// 俯仰角与距离约束。
    pub constraints: ViewportCameraConstraints,
}

impl Default for ViewportCamera3D {
    fn default() -> Self {
        Self {
            orbit_yaw: 0.6,
            orbit_pitch: 0.35,
            distance: 3.5,
            target: [0.0, 0.0, 0.0],
            fov_y_rad: core::f32::consts::FRAC_PI_4,
            near: 0.05,
            far: 1000.0,
            constraints: ViewportCameraConstraints::default(),
        }
    }
}

impl ViewportCamera3D {
    /// 以链式风格绑定自定义约束并立即重钳制俯仰角与距离。
    #[must_use]
    pub fn with_constraints(mut self, c: ViewportCameraConstraints) -> Self {
        self.set_constraints(c);
        self
    }

    /// 更新轨道相机约束（先执行非有限值与倒置区间消毒），并立即重新钳制 `orbit_pitch` 与 `distance`。
    pub fn set_constraints(&mut self, c: ViewportCameraConstraints) {
        self.constraints = c.sanitized();
        self.orbit_pitch = self.constraints.clamp_pitch(self.orbit_pitch);
        self.distance = self.constraints.clamp_distance(self.distance);
    }

    /// 根据鼠标拖拽增量旋转轨道相机（俯仰角按约束钳制）。
    pub fn orbit(&mut self, delta: Vec2, sensitivity: f32) {
        self.orbit_yaw += delta.x * sensitivity;
        self.orbit_pitch = self
            .constraints
            .clamp_pitch(self.orbit_pitch + delta.y * sensitivity);
    }

    /// 沿当前相机视图平面的右向量与上向量平移观察目标点（按距离缩放）。
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let cp = self.orbit_pitch.cos();
        let sp = self.orbit_pitch.sin();
        let cy = self.orbit_yaw.cos();
        let sy = self.orbit_yaw.sin();

        // 与 `eye_position` 保持右手系一致：
        // forward = [-cp * sy, -sp, -cp * cy]
        // right   = normalize(forward × world_up) = [cy, 0, -sy]
        // up      = right × forward = [-sp * sy, cp, -sp * cy]
        let right = [cy, 0.0, -sy];
        let up = [-sp * sy, cp, -sp * cy];

        let scale = self.distance.max(1e-3);
        let rx = dx * scale;
        let ry = dy * scale;
        self.target[0] += right[0] * rx + up[0] * ry;
        self.target[1] += right[1] * rx + up[1] * ry;
        self.target[2] += right[2] * rx + up[2] * ry;
    }

    /// 根据滚轮缩放系数调整观察距离（按约束钳制）。
    pub fn dolly(&mut self, zoom_factor: f32) {
        self.distance = self.constraints.clamp_distance(self.distance * zoom_factor);
    }

    /// 计算相机在世界空间中的眼睛坐标。
    #[must_use]
    pub fn eye_position(&self) -> [f32; 3] {
        let cp = self.orbit_pitch.cos();
        let sp = self.orbit_pitch.sin();
        let cy = self.orbit_yaw.cos();
        let sy = self.orbit_yaw.sin();
        [
            self.target[0] + self.distance * cp * sy,
            self.target[1] + self.distance * sp,
            self.target[2] + self.distance * cp * cy,
        ]
    }
}

/// 将视口渲染分辨率缩放倍率钳制至 `[0.25, 1.0]`（非有限值回退为 `1.0`）。
#[must_use]
pub fn clamp_render_scale(scale: f32) -> f32 {
    if !scale.is_finite() {
        return MAX_VIEWPORT_RENDER_SCALE;
    }
    scale.clamp(MIN_VIEWPORT_RENDER_SCALE, MAX_VIEWPORT_RENDER_SCALE)
}

/// 按逻辑尺寸、有效 DPI 缩放与渲染分辨率倍率计算离屏物理像素尺寸。
///
/// 当宽、高或有效缩放非正/非有限，或取整后任意一维小于 `1` 像素时返回 `None`。
#[must_use]
pub fn compute_viewport_physical_extent(
    logical_width: f32,
    logical_height: f32,
    effective_scale: f32,
    render_scale: f32,
) -> Option<(u32, u32)> {
    if !logical_width.is_finite()
        || !logical_height.is_finite()
        || !effective_scale.is_finite()
        || logical_width <= 0.0
        || logical_height <= 0.0
        || effective_scale <= 0.0
    {
        return None;
    }
    let scale = effective_scale * clamp_render_scale(render_scale);
    let w = (logical_width * scale).round();
    let h = (logical_height * scale).round();
    if !w.is_finite() || !h.is_finite() || w < 1.0 || h < 1.0 {
        return None;
    }
    if w > u32::MAX as f32 || h > u32::MAX as f32 {
        return None;
    }
    Some((w as u32, h as u32))
}

/// 嵌入 UI 布局矩形中的自定义 3D 视口渲染 Pass 描述符。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportRenderPassDesc {
    /// 视口唯一标识。
    pub viewport_id: ViewportId,
    /// 请求的多重采样级别（1x 或 4x MSAA）。
    pub sample_count: ViewportMsaa,
    /// 深度缓冲约定模式（Standard 或 Reverse-Z）。
    pub depth_mode: ViewportDepthMode,
    /// 离屏颜色附件清屏色（RGBA，支持 `alpha = 0.0` 透明合成）。
    pub clear_color: [f32; 4],
    /// 相对物理分辨率的渲染缩放倍率（钳制至 `[0.25, 1.0]`）。
    pub render_scale: f32,
    /// UI 布局分配的屏幕矩形。
    pub bounds: Rect,
    /// 绑定的外部颜色纹理 Bindless 槽位。
    pub bindless_slot: u32,
    /// 3D 轨道相机参数。
    pub camera: ViewportCamera3D,
    /// 是否显示线框叠加。
    pub wireframe_overlay: bool,
    /// 是否显示世界坐标网格。
    pub show_grid: bool,
}

impl ViewportRenderPassDesc {
    /// 按指定 [`ViewportId`] 构造默认视口渲染 Pass 描述符。
    #[must_use]
    pub fn new(viewport_id: ViewportId) -> Self {
        Self {
            viewport_id,
            sample_count: ViewportMsaa::default(),
            depth_mode: ViewportDepthMode::default(),
            clear_color: [0.0, 0.0, 0.0, 1.0],
            render_scale: MAX_VIEWPORT_RENDER_SCALE,
            bounds: Rect::ZERO,
            bindless_slot: 0,
            camera: ViewportCamera3D::default(),
            wireframe_overlay: false,
            show_grid: false,
        }
    }

    /// 返回经过安全钳制后的渲染分辨率缩放倍率。
    #[must_use]
    pub fn clamped_render_scale(&self) -> f32 {
        clamp_render_scale(self.render_scale)
    }
}

/// 自定义 3D 视口 Pass 录制阶段的 Vulkan 上下文参数。
pub struct ViewportPassContext<'a> {
    /// Vulkan 逻辑设备引用。
    pub device: &'a ash::Device,
    /// 当前正在录制的图形命令缓冲区（已处于 Dynamic Rendering Pass 内）。
    pub cmd: vk::CommandBuffer,
    /// 当前离屏渲染目标物理像素尺寸 `(width, height)`。
    pub extent: (u32, u32),
    /// 颜色附件格式（`R8G8B8A8_UNORM`）。
    pub color_format: vk::Format,
    /// 深度附件格式（`D32_SFLOAT`）。
    pub depth_format: vk::Format,
    /// 实际生效的采样数（若硬件不支持 4x 则已回退至 1x）。
    pub sample_count: ViewportMsaa,
    /// 本帧同步的视口渲染 Pass 描述符。
    pub desc: &'a ViewportRenderPassDesc,
}

impl core::fmt::Debug for ViewportPassContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewportPassContext")
            .field("cmd", &self.cmd)
            .field("extent", &self.extent)
            .field("color_format", &self.color_format)
            .field("depth_format", &self.depth_format)
            .field("sample_count", &self.sample_count)
            .field("desc", &self.desc)
            .finish_non_exhaustive()
    }
}

/// 嵌入 `unbound-ui` 帧管线的自定义离屏 3D 视口渲染钩子。
pub trait CustomViewportPass: Send + Sync + core::fmt::Debug {
    /// 返回视口唯一标识。
    fn viewport_id(&self) -> ViewportId;

    /// 返回初始或默认渲染 Pass 描述符。
    fn render_pass_desc(&self) -> ViewportRenderPassDesc {
        ViewportRenderPassDesc::new(self.viewport_id())
    }

    /// 在离屏 3D Dynamic Rendering Pass 内录制自定义绘制命令。
    fn record_pass(&mut self, ctx: &ViewportPassContext<'_>);
}

/// 离屏 3D 视口颜色目标的 CPU RGBA8 像素回读结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportReadback {
    /// 图像宽度（像素）。
    pub width: u32,
    /// 图像高度（像素）。
    pub height: u32,
    /// 单行字节步长（紧凑排列为 `width * 4`）。
    pub stride: u32,
    /// 紧凑排列的 RGBA8 像素字节数组。
    pub rgba8: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_id_msaa_and_depth_mode_helpers_match_contract() {
        let id = ViewportId::new(42);
        assert_eq!(id.raw(), 42);
        assert_eq!(ViewportId::default().raw(), 0);
        assert!(ViewportId::new(1) < ViewportId::new(2));

        assert_eq!(ViewportMsaa::default(), ViewportMsaa::X1);
        assert_eq!(
            ViewportMsaa::X1.vk_sample_count(),
            vk::SampleCountFlags::TYPE_1
        );
        assert_eq!(
            ViewportMsaa::X4.vk_sample_count(),
            vk::SampleCountFlags::TYPE_4
        );

        assert_eq!(ViewportDepthMode::default(), ViewportDepthMode::Standard);
        assert_eq!(ViewportDepthMode::Standard.clear_depth(), 1.0);
        assert_eq!(ViewportDepthMode::ReverseZ.clear_depth(), 0.0);
    }

    #[test]
    fn camera_orbit_dolly_and_custom_constraints_clamp_properly() {
        let mut cam = ViewportCamera3D::default();
        let default_limit = 89.0_f32.to_radians();
        cam.orbit(Vec2::new(1.5, 1000.0), 0.1);
        assert!((cam.orbit_pitch - default_limit).abs() < 1e-5);
        cam.orbit(Vec2::new(0.0, -2000.0), 0.1);
        assert!((cam.orbit_pitch + default_limit).abs() < 1e-5);

        cam.dolly(1e-9);
        assert!((cam.distance - 0.01).abs() < 1e-6);
        cam.dolly(1e12);
        assert!((cam.distance - 100_000.0).abs() < 1e-3);

        // 自定义约束必须立即重钳制当前俯仰与距离。
        let custom = ViewportCameraConstraints {
            min_pitch_rad: -0.5,
            max_pitch_rad: 0.4,
            min_distance: 2.0,
            max_distance: 10.0,
        };
        let clamped = cam.with_constraints(custom);
        assert!((clamped.orbit_pitch - (-0.5)).abs() < 1e-6);
        assert!((clamped.distance - 10.0).abs() < 1e-6);

        let mut cam2 = clamped;
        cam2.set_constraints(ViewportCameraConstraints {
            min_pitch_rad: 0.1,
            max_pitch_rad: 0.3,
            min_distance: 3.0,
            max_distance: 5.0,
        });
        assert!((cam2.orbit_pitch - 0.1).abs() < 1e-6);
        assert!((cam2.distance - 5.0).abs() < 1e-6);
    }

    #[test]
    fn camera_pan_moves_target_along_view_plane_basis() {
        // yaw = 0, pitch = 0 时相机在 +Z 看向原点：right = +X，up = +Y。
        let mut cam = ViewportCamera3D {
            orbit_yaw: 0.0,
            orbit_pitch: 0.0,
            distance: 4.0,
            target: [0.0, 0.0, 0.0],
            ..ViewportCamera3D::default()
        };
        cam.pan(0.5, -0.25);
        assert!((cam.target[0] - 2.0).abs() < 1e-5);
        assert!((cam.target[1] - (-1.0)).abs() < 1e-5);
        assert!(cam.target[2].abs() < 1e-5);

        // 旋转 90°（yaw = pi/2）时，相机在 +X 看向原点：right = -Z，up = +Y。
        let mut cam_rot = ViewportCamera3D {
            orbit_yaw: core::f32::consts::FRAC_PI_2,
            orbit_pitch: 0.0,
            distance: 2.0,
            target: [1.0, 2.0, 3.0],
            ..ViewportCamera3D::default()
        };
        cam_rot.pan(1.0, 0.5);
        assert!((cam_rot.target[0] - 1.0).abs() < 1e-5);
        assert!((cam_rot.target[1] - 3.0).abs() < 1e-5);
        assert!((cam_rot.target[2] - 1.0).abs() < 1e-5);

        let eye = cam_rot.eye_position();
        assert!(eye.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn constraints_for_bounding_radius_handles_valid_and_invalid_inputs() {
        let c = ViewportCameraConstraints::for_bounding_radius(5.0, 0.5, 10.0);
        assert!((c.min_distance - 2.5).abs() < 1e-5);
        assert!((c.max_distance - 50.0).abs() < 1e-5);

        // 非法或倒置倍率安全回退。
        let fallback = ViewportCameraConstraints::for_bounding_radius(f32::NAN, -1.0, 0.0);
        assert!(fallback.min_distance.is_finite() && fallback.min_distance > 0.0);
        assert!(fallback.max_distance >= fallback.min_distance);

        // `NaN`、无穷大与倒置约束在 `sanitized` / `set_constraints` / `orbit` / `dolly` 下不得 panic。
        let broken = ViewportCameraConstraints {
            min_pitch_rad: f32::NAN,
            max_pitch_rad: f32::INFINITY,
            min_distance: -5.0,
            max_distance: f32::NAN,
        };
        let sanitized = broken.sanitized();
        assert!(sanitized.min_pitch_rad.is_finite() && sanitized.max_pitch_rad.is_finite());
        assert!(sanitized.min_pitch_rad <= sanitized.max_pitch_rad);
        assert!(sanitized.min_distance.is_finite() && sanitized.min_distance > 0.0);
        assert!(
            sanitized.max_distance.is_finite() && sanitized.max_distance >= sanitized.min_distance
        );

        let mut cam = ViewportCamera3D::default().with_constraints(broken);
        cam.constraints = ViewportCameraConstraints {
            min_pitch_rad: 0.8,
            max_pitch_rad: -0.4,
            min_distance: 10.0,
            max_distance: 2.0,
        };
        cam.orbit(Vec2::new(1.0, f32::NAN), 0.1);
        cam.dolly(f32::NAN);
        assert!(cam.orbit_pitch.is_finite());
        assert!(cam.distance.is_finite() && cam.distance >= 2.0 && cam.distance <= 10.0);
    }

    #[test]
    fn clamp_render_scale_and_physical_extent_handle_edge_cases() {
        assert_eq!(clamp_render_scale(f32::NAN), MAX_VIEWPORT_RENDER_SCALE);
        assert_eq!(clamp_render_scale(f32::INFINITY), MAX_VIEWPORT_RENDER_SCALE);
        assert_eq!(
            clamp_render_scale(f32::NEG_INFINITY),
            MAX_VIEWPORT_RENDER_SCALE
        );
        assert_eq!(clamp_render_scale(0.1), MIN_VIEWPORT_RENDER_SCALE);
        assert_eq!(clamp_render_scale(2.5), MAX_VIEWPORT_RENDER_SCALE);
        assert_eq!(clamp_render_scale(0.5), 0.5);

        assert_eq!(
            compute_viewport_physical_extent(100.0, 50.0, 2.0, 0.5),
            Some((100, 50))
        );
        assert_eq!(
            compute_viewport_physical_extent(100.0, 50.0, 1.5, f32::NAN),
            Some((150, 75))
        );
        assert_eq!(compute_viewport_physical_extent(0.0, 50.0, 1.0, 1.0), None);
        assert_eq!(
            compute_viewport_physical_extent(100.0, -10.0, 1.0, 1.0),
            None
        );
        assert_eq!(
            compute_viewport_physical_extent(100.0, 50.0, 0.0, 1.0),
            None
        );
        assert_eq!(
            compute_viewport_physical_extent(f32::NAN, 50.0, 1.0, 1.0),
            None
        );
        assert_eq!(compute_viewport_physical_extent(0.2, 50.0, 1.0, 0.25), None);

        let mut desc = ViewportRenderPassDesc::new(ViewportId::new(7));
        desc.render_scale = 0.1;
        assert_eq!(desc.clamped_render_scale(), MIN_VIEWPORT_RENDER_SCALE);
    }
}
