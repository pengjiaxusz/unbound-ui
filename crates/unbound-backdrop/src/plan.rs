//! 后景计划：把模式选择、平台能力与屏幕几何收敛成一份纯值。
//!
//! 宿主不再各自推导「该用哪条路径 / 清屏 alpha 多少 / 纹理多大 / 取哪块 UV」，
//! 而是调用 [`BackdropPlan::derive`] 一次算清，再让启动、运行期切换与自绘生成
//! 三处消费同一份结果。该模块不做任何系统调用，输入必须由调用方提供，因此
//! 可以在单元测试里穷举四种模式与各种几何。

use crate::mica::{DEFAULT_MAX_WIDTH, DEFAULT_SIGMA};
use crate::mode::{BackdropMode, ResolvedBackdrop};

/// 屏幕坐标系中的物理像素矩形。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    /// 左上角物理 X 坐标。
    pub x: i32,
    /// 左上角物理 Y 坐标。
    pub y: i32,
    /// 物理宽度。
    pub width: u32,
    /// 物理高度。
    pub height: u32,
}

impl PixelRect {
    /// 构造一个物理矩形。
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 是否具有可用的面积。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// 右边界（物理像素，不含）。
    #[must_use]
    pub const fn right(&self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    /// 下边界（物理像素，不含）。
    #[must_use]
    pub const fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }
}

/// 归一化 UV 子矩形，取值 `0..1`。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UvRect {
    /// 左边界。
    pub x: f32,
    /// 上边界。
    pub y: f32,
    /// 宽度。
    pub width: f32,
    /// 高度。
    pub height: f32,
}

impl UvRect {
    /// 整张纹理。
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    /// 构造一个归一化子矩形。
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

impl Default for UvRect {
    fn default() -> Self {
        Self::FULL
    }
}

/// 表面合成方式。
///
/// 与用户选择和后景路径无关，只由平台能力决定：原生材质需要预乘 alpha，
/// 不支持时只能用不透明表面。同一窗口生命周期内必须保持不变。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum BackdropCompositeAlpha {
    /// 不透明表面：清屏 alpha 为 1，窗口永远遮住桌面。
    #[default]
    Opaque,
    /// 预乘 alpha 表面：清屏 alpha 可为 0，让 DWM 材质透出。
    PreMultiplied,
}

/// 需要向 DWM 请求的系统材质。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum DwmMaterial {
    /// 不请求任何材质（关闭后景或走自绘路径）。
    #[default]
    None,
    /// 系统 Mica 材质。
    Mica,
}

/// 自绘后景的纹理规格。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WallpaperTexture {
    /// 纹理覆盖区域的物理宽。
    pub width: u32,
    /// 纹理覆盖区域的物理高。
    pub height: u32,
    /// 主屏物理宽，用于把窗口映射进壁纸空间。
    pub screen_width: u32,
    /// 主屏物理高，用于把窗口映射进壁纸空间。
    pub screen_height: u32,
    /// 纹理最大宽度（0 视为默认值）。
    pub max_width: u32,
    /// 磨砂 sigma。
    pub sigma: f32,
    /// 是否使用深色主题色调。
    pub dark: bool,
}

impl WallpaperTexture {
    /// 纹理最大宽度，已把 0 归一化为默认值。
    #[must_use]
    pub const fn effective_max_width(&self) -> u32 {
        if self.max_width == 0 {
            DEFAULT_MAX_WIDTH
        } else {
            self.max_width
        }
    }

    /// 磨砂 sigma，已把负值归一化为 0。
    #[must_use]
    pub fn effective_sigma(&self) -> f32 {
        self.sigma.max(0.0)
    }
}

/// 后景计划的输入。全部是值，不含系统查询。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BackdropInputs {
    /// 用户选择。
    pub choice: BackdropMode,
    /// 平台是否承载原生材质。
    pub native_available: bool,
    /// 客户区屏幕物理矩形；未知为 `None`。
    pub window_screen_rect: Option<PixelRect>,
    /// 主屏物理矩形；未知为 `None`。
    pub primary_screen_rect: Option<PixelRect>,
    /// 是否深色主题。
    pub dark: bool,
    /// 自绘纹理是否已经就绪（用于根背景透明性）。
    pub self_drawn_ready: bool,
}

/// 后景计划：启动、切换与生成共同消费的唯一结果。
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropPlan {
    /// 解析后的实际路径。
    pub resolved: ResolvedBackdrop,
    /// 固定合成方式，只由平台能力决定。
    pub composition: BackdropCompositeAlpha,
    /// 清屏 alpha：原生路径 0，其余路径 1。
    pub clear_alpha: f32,
    /// 根背景是否应为透明，交给后景透出。
    pub root_background_transparent: bool,
    /// 需要请求的 DWM 材质。
    pub material: DwmMaterial,
    /// 自绘纹理规格；非自绘路径或几何未知时为 `None`。
    pub texture: Option<WallpaperTexture>,
    /// 自绘纹理的归一化采样区域。
    pub uv: UvRect,
}

impl Default for BackdropPlan {
    fn default() -> Self {
        Self {
            resolved: ResolvedBackdrop::Off,
            composition: BackdropCompositeAlpha::Opaque,
            clear_alpha: 1.0,
            root_background_transparent: false,
            material: DwmMaterial::None,
            texture: None,
            uv: UvRect::FULL,
        }
    }
}

impl BackdropPlan {
    /// 由输入推导完整计划。
    #[must_use]
    pub fn derive(inputs: BackdropInputs) -> Self {
        let resolved = ResolvedBackdrop::resolve(inputs.choice, inputs.native_available);
        let native = resolved.uses_native_path();
        let composition = if inputs.native_available {
            BackdropCompositeAlpha::PreMultiplied
        } else {
            BackdropCompositeAlpha::Opaque
        };
        let texture = resolved
            .draws_self_drawn_texture()
            .then(|| wallpaper_texture(inputs))
            .flatten();
        Self {
            resolved,
            composition,
            clear_alpha: if native { 0.0 } else { 1.0 },
            root_background_transparent: native
                || (resolved.draws_self_drawn_texture() && inputs.self_drawn_ready),
            material: if native {
                DwmMaterial::Mica
            } else {
                DwmMaterial::None
            },
            texture,
            // 当前纹理只覆盖窗口，整张采样；切换到壁纸空间纹理后才映射子矩形。
            uv: UvRect::FULL,
        }
    }
}

fn wallpaper_texture(inputs: BackdropInputs) -> Option<WallpaperTexture> {
    let window = inputs.window_screen_rect?;
    let screen = inputs.primary_screen_rect?;
    if window.is_empty() || screen.is_empty() {
        return None;
    }
    Some(WallpaperTexture {
        width: window.width.min(screen.width).max(1),
        height: window.height.min(screen.height).max(1),
        screen_width: screen.width.max(1),
        screen_height: screen.height.max(1),
        max_width: DEFAULT_MAX_WIDTH,
        sigma: DEFAULT_SIGMA,
        dark: inputs.dark,
    })
}

/// 把窗口物理矩形映射为主屏纹理的归一化采样区域。
///
/// 部分越界时按主屏边界钳制；完全无交集或几何未知时退回整张纹理，
/// 保证采样区域始终有效。主屏原点参与计算，副屏坐标不会被误当成 `0..1`。
#[must_use]
pub fn sample_uv(window: Option<PixelRect>, screen: Option<PixelRect>) -> UvRect {
    let (Some(window), Some(screen)) = (window, screen) else {
        return UvRect::FULL;
    };
    if window.is_empty() || screen.is_empty() {
        return UvRect::FULL;
    }
    let left = window.x.max(screen.x);
    let top = window.y.max(screen.y);
    let right = window.right().min(screen.right());
    let bottom = window.bottom().min(screen.bottom());
    if right <= left || bottom <= top {
        return UvRect::FULL;
    }
    UvRect::new(
        (left - screen.x) as f32 / screen.width as f32,
        (top - screen.y) as f32 / screen.height as f32,
        (right - left) as f32 / screen.width as f32,
        (bottom - top) as f32 / screen.height as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: PixelRect = PixelRect::new(0, 0, 1920, 1080);

    fn inputs(choice: BackdropMode, native_available: bool) -> BackdropInputs {
        BackdropInputs {
            choice,
            native_available,
            window_screen_rect: Some(PixelRect::new(400, 300, 800, 600)),
            primary_screen_rect: Some(SCREEN),
            dark: false,
            self_drawn_ready: false,
        }
    }

    #[test]
    fn four_modes_keep_the_old_resolution_and_host_effects() {
        let native = BackdropPlan::derive(inputs(BackdropMode::Auto, true));
        assert_eq!(native.resolved, ResolvedBackdrop::Native);
        assert_eq!(native.clear_alpha, 0.0);
        assert_eq!(native.material, DwmMaterial::Mica);
        assert!(native.root_background_transparent);
        assert!(native.texture.is_none());

        let fallback = BackdropPlan::derive(inputs(BackdropMode::Native, false));
        assert_eq!(fallback.resolved, ResolvedBackdrop::SelfDrawn);
        assert_eq!(fallback.clear_alpha, 1.0);
        assert_eq!(fallback.material, DwmMaterial::None);
        assert!(!fallback.root_background_transparent);
        assert!(fallback.texture.is_some());

        let self_drawn = BackdropPlan::derive(inputs(BackdropMode::SelfDrawn, true));
        assert_eq!(self_drawn.resolved, ResolvedBackdrop::SelfDrawn);
        assert_eq!(self_drawn.clear_alpha, 1.0);

        let off = BackdropPlan::derive(inputs(BackdropMode::Off, true));
        assert_eq!(off.resolved, ResolvedBackdrop::Off);
        assert_eq!(off.clear_alpha, 1.0);
        assert_eq!(off.material, DwmMaterial::None);
        assert!(!off.root_background_transparent);
        assert!(off.texture.is_none());
    }

    #[test]
    fn root_transparency_follows_texture_readiness_on_the_self_drawn_path() {
        let mut input = inputs(BackdropMode::SelfDrawn, true);
        assert!(!BackdropPlan::derive(input).root_background_transparent);
        input.self_drawn_ready = true;
        assert!(BackdropPlan::derive(input).root_background_transparent);
    }

    #[test]
    fn composition_only_depends_on_native_capability() {
        for choice in BackdropMode::ALL {
            let supported = BackdropPlan::derive(inputs(choice, true));
            assert_eq!(
                supported.composition,
                BackdropCompositeAlpha::PreMultiplied,
                "{choice:?} must keep a premultiplied surface when the platform supports it"
            );
            let unsupported = BackdropPlan::derive(inputs(choice, false));
            assert_eq!(
                unsupported.composition,
                BackdropCompositeAlpha::Opaque,
                "{choice:?} must stay opaque without native capability"
            );
        }
    }

    #[test]
    fn composition_survives_a_full_mode_round_trip() {
        let sequence = [
            BackdropMode::Auto,
            BackdropMode::SelfDrawn,
            BackdropMode::Off,
            BackdropMode::Native,
            BackdropMode::Auto,
        ];
        let expected = BackdropPlan::derive(inputs(BackdropMode::Auto, true)).composition;
        for choice in sequence {
            let plan = BackdropPlan::derive(inputs(choice, true));
            assert_eq!(
                plan.composition, expected,
                "switching to {choice:?} must not reconfigure the surface"
            );
        }
    }

    #[test]
    fn texture_geometry_keeps_the_window_size_for_now() {
        let plan = BackdropPlan::derive(inputs(BackdropMode::SelfDrawn, true));
        let texture = plan.texture.expect("self-drawn path needs a texture");
        assert_eq!((texture.width, texture.height), (800, 600));
        assert_eq!((texture.screen_width, texture.screen_height), (1920, 1080));
        assert_eq!(plan.uv, UvRect::FULL);
    }

    #[test]
    fn missing_geometry_skips_the_texture_but_keeps_the_path() {
        let mut input = inputs(BackdropMode::SelfDrawn, false);
        input.window_screen_rect = None;
        let plan = BackdropPlan::derive(input);
        assert_eq!(plan.resolved, ResolvedBackdrop::SelfDrawn);
        assert!(plan.texture.is_none());
        assert_eq!(plan.uv, UvRect::FULL);
    }

    #[test]
    fn uv_maps_a_window_inside_the_primary_screen() {
        let uv = sample_uv(Some(PixelRect::new(480, 270, 960, 540)), Some(SCREEN));
        assert_eq!(uv, UvRect::new(0.25, 0.25, 0.5, 0.5));
    }

    #[test]
    fn uv_clamps_a_partially_offscreen_window() {
        let uv = sample_uv(Some(PixelRect::new(-100, -50, 300, 200)), Some(SCREEN));
        assert_eq!(uv, UvRect::new(0.0, 0.0, 200.0 / 1920.0, 150.0 / 1080.0));
    }

    #[test]
    fn uv_falls_back_to_the_whole_texture_without_overlap() {
        assert_eq!(
            sample_uv(Some(PixelRect::new(4000, 3000, 800, 600)), Some(SCREEN)),
            UvRect::FULL
        );
        assert_eq!(sample_uv(None, Some(SCREEN)), UvRect::FULL);
        assert_eq!(sample_uv(Some(SCREEN), None), UvRect::FULL);
    }

    #[test]
    fn a_window_larger_than_the_screen_is_clamped_to_the_full_texture() {
        assert_eq!(
            sample_uv(Some(PixelRect::new(-500, -400, 4000, 3000)), Some(SCREEN)),
            UvRect::FULL
        );
    }
}
