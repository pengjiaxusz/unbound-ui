//! 框架级主题过渡状态机（ADR 0034）。
//!
//! 本模块在纯值层实现基于 4 KiB 语义颜色 LUT 的逐帧插值：
//! - 默认时长 240ms，采用与 `unbound-anim` 一致的 `CubicBezier::EASE_OUT` 曲线；
//! - RGB 在 Oklab 感知均匀空间插值，Alpha 线性插值；
//! - `Token::NONE`（槽位 0）恒为全透明，单侧 `alpha == 0` 端点沿用非透明端 RGB 避免黑边；
//! - 过渡中途重入时冻结当前视觉调色板为新起点，反向折返按已走进度按比例缩短时长；
//! - 首帧、减弱动画、时长为 0 或无头/差分模式均走 `snap_to` 强制瞬切。

use unbound_math::color::Color;

use crate::{Palette, ThemeKind, ThemeLut, Token, MAX_TOKENS};

/// 框架默认主题过渡时长（秒）：240ms，兼顾暗光护眼与桌面响应速度。
pub const DEFAULT_THEME_TRANSITION_DURATION_SECS: f32 = 0.24;

/// 主题过渡配置。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ThemeTransitionConfig {
    /// 过渡时长（秒）；`<= 0.0` 表示关闭过渡并直接瞬切。
    pub duration_secs: f32,
    /// 是否启用运行期主题过渡。
    pub enabled: bool,
}

impl Default for ThemeTransitionConfig {
    fn default() -> Self {
        Self {
            duration_secs: DEFAULT_THEME_TRANSITION_DURATION_SECS,
            enabled: true,
        }
    }
}

impl ThemeTransitionConfig {
    /// 禁用主题过渡（强制瞬切）。
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            duration_secs: 0.0,
            enabled: false,
        }
    }

    /// 按自定义毫秒数构造主题过渡配置（`0` 表示瞬切）。
    #[must_use]
    pub fn from_millis(ms: u32) -> Self {
        if ms == 0 {
            Self::disabled()
        } else {
            Self {
                duration_secs: ms as f32 / 1000.0,
                enabled: true,
            }
        }
    }

    /// 结合系统「减弱动画」偏好解析最终生效的过渡时长（秒）；返回 `0.0` 表示强制瞬切。
    #[must_use]
    pub fn effective_duration_secs(&self, reduced_motion: bool) -> f32 {
        if !self.enabled
            || reduced_motion
            || !self.duration_secs.is_finite()
            || self.duration_secs <= 0.0
        {
            0.0
        } else {
            self.duration_secs
        }
    }
}

/// 与 `unbound_anim::CubicBezier::EASE_OUT`（`0.0, 0.0, 0.58, 1.0`）完全一致的缓动求值。
#[must_use]
pub fn ease_out_cubic_bezier(t: f32) -> f32 {
    let u = t.clamp(0.0, 1.0);
    if u <= 0.0 {
        return 0.0;
    }
    if u >= 1.0 {
        return 1.0;
    }
    const X1: f32 = 0.0;
    const Y1: f32 = 0.0;
    const X2: f32 = 0.58;
    const Y2: f32 = 1.0;

    fn bezier(s: f32, p1: f32, p2: f32) -> f32 {
        let inv = 1.0 - s;
        3.0 * inv * inv * s * p1 + 3.0 * inv * s * s * p2 + s * s * s
    }

    fn bezier_deriv(s: f32, p1: f32, p2: f32) -> f32 {
        let inv = 1.0 - s;
        3.0 * inv * inv * p1 + 6.0 * inv * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
    }

    let mut guess = u;
    for _ in 0..8 {
        let x = bezier(guess, X1, X2) - u;
        let dx = bezier_deriv(guess, X1, X2);
        if dx.abs() < 1e-6 {
            break;
        }
        guess = (guess - x / dx).clamp(0.0, 1.0);
    }
    bezier(guess, Y1, Y2).clamp(0.0, 1.0)
}

/// 在 Oklab 感知空间插值 RGB、线性插值 Alpha，并对单侧全透明端点做防黑边处理。
#[must_use]
pub fn interpolate_color_oklab(from: Color, to: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return from;
    }
    if t >= 1.0 {
        return to;
    }
    const ALPHA_EPS: f32 = 1e-6;
    let from_transparent = from.a <= ALPHA_EPS;
    let to_transparent = to.a <= ALPHA_EPS;
    if from_transparent && to_transparent {
        return Color::TRANSPARENT;
    }
    if from_transparent {
        return Color::new(to.r, to.g, to.b, to.a * t);
    }
    if to_transparent {
        return Color::new(from.r, from.g, from.b, from.a * (1.0 - t));
    }
    let rgb = from.lerp_oklab(to, t);
    let alpha = from.a + (to.a - from.a) * t;
    Color::new(rgb.r, rgb.g, rgb.b, alpha)
}

/// 将颜色数组打包为 4 KiB [`ThemeLut`]（确保 `Token::NONE` 恒定透明）。
fn pack_lut(colors: &[Color; MAX_TOKENS]) -> ThemeLut {
    let mut entries = Vec::with_capacity(MAX_TOKENS);
    let mut bytes = Vec::with_capacity(crate::LUT_BYTES);
    for (idx, color) in colors.iter().enumerate() {
        let c = if idx == Token::NONE.index() {
            Color::TRANSPARENT
        } else {
            *color
        };
        let packed = c.to_rgba16f();
        entries.push(packed);
        for ch in packed {
            bytes.extend_from_slice(&ch.to_le_bytes());
        }
    }
    ThemeLut::from_raw(entries, bytes)
}

fn palette_to_array(palette: &Palette) -> [Color; MAX_TOKENS] {
    let mut arr = [Color::TRANSPARENT; MAX_TOKENS];
    for (dst, src) in arr.iter_mut().zip(palette.entries().iter()) {
        *dst = *src;
    }
    arr[Token::NONE.index()] = Color::TRANSPARENT;
    arr
}

fn colors_approx_eq(a: &[Color; MAX_TOKENS], b: &[Color; MAX_TOKENS]) -> bool {
    const EPS: f32 = 1e-5;
    a.iter().zip(b.iter()).all(|(x, y)| {
        (x.r - y.r).abs() <= EPS
            && (x.g - y.g).abs() <= EPS
            && (x.b - y.b).abs() <= EPS
            && (x.a - y.a).abs() <= EPS
    })
}

#[derive(Clone, Debug)]
struct ActiveTransition {
    from_colors: [Color; MAX_TOKENS],
    from_kind: ThemeKind,
    /// 本次过渡启动前的原始起点，用于识别连续反向折返。
    origin_colors: [Color; MAX_TOKENS],
    start_time_secs: f64,
    duration_secs: f32,
    raw_progress: f32,
}

/// 框架级主题过渡状态机。
///
/// 维护起止调色板快照与当前已打包的 4 KiB [`ThemeLut`]，支持多窗口共享时间戳推进、
/// 中途平滑折返以及原生后景中点翻转判定。
#[derive(Clone, Debug)]
pub struct ThemeTransition {
    current_colors: [Color; MAX_TOKENS],
    target_colors: [Color; MAX_TOKENS],
    target_kind: ThemeKind,
    active: Option<ActiveTransition>,
    lut: ThemeLut,
}

impl ThemeTransition {
    /// 以初始调色板构建静止态过渡器（首帧无动画）。
    #[must_use]
    pub fn new(initial: &Palette) -> Self {
        let colors = palette_to_array(initial);
        let lut = pack_lut(&colors);
        Self {
            current_colors: colors,
            target_colors: colors,
            target_kind: initial.kind(),
            active: None,
            lut,
        }
    }

    /// 强制瞬切到目标调色板并清空进行中的过渡（用于首帧、减弱动画、无头/差分模式与 `Suspended` 恢复）。
    pub fn snap_to(&mut self, target: &Palette) {
        let colors = palette_to_array(target);
        self.current_colors = colors;
        self.target_colors = colors;
        self.target_kind = target.kind();
        self.active = None;
        self.lut = pack_lut(&colors);
    }

    /// 启动一次向 `target` 的主题过渡。
    ///
    /// 返回 `true` 表示目标色板发生了有效变更（需要至少上传一帧 LUT）；
    /// 返回 `false` 表示目标与当前完全一致（无操作，不唤醒帧调度器）。
    pub fn begin(&mut self, target: &Palette, now_secs: f64, duration_secs: f32) -> bool {
        let next_colors = palette_to_array(target);
        let next_kind = target.kind();

        // 目标与当前终态一致且未处于过渡中：无操作。
        if self.active.is_none()
            && self.target_kind == next_kind
            && colors_approx_eq(&self.target_colors, &next_colors)
        {
            return false;
        }

        // 时长无效或为 0：直接强制瞬切。
        if !duration_secs.is_finite() || duration_secs <= 0.0 {
            self.snap_to(target);
            return true;
        }

        // 若当前已有进行中的过渡，先推进到 `now_secs` 冻结当前视觉调色板作为新起点。
        let (from_kind, scaled_duration) = if self.active.is_some() {
            let _ = self.advance(now_secs);
            if let Some(prev) = &self.active {
                let is_reversal = colors_approx_eq(&next_colors, &prev.origin_colors)
                    || (next_kind == prev.from_kind && next_kind != self.target_kind);
                let dur = if is_reversal {
                    (duration_secs * prev.raw_progress.clamp(0.05, 1.0)).max(0.016)
                } else {
                    duration_secs
                };
                // 中途折返时保留原起点语义供原生后景判断。
                let eff_from_kind = if prev.raw_progress < 0.5 {
                    prev.from_kind
                } else {
                    self.target_kind
                };
                (eff_from_kind, dur)
            } else {
                (self.target_kind, duration_secs)
            }
        } else {
            (self.target_kind, duration_secs)
        };

        let origin_colors = self.current_colors;
        self.active = Some(ActiveTransition {
            from_colors: self.current_colors,
            from_kind,
            origin_colors,
            start_time_secs: now_secs,
            duration_secs: scaled_duration,
            raw_progress: 0.0,
        });
        self.target_colors = next_colors;
        self.target_kind = next_kind;
        true
    }

    /// 将过渡状态推进至 `now_secs` 并更新当前 4 KiB LUT。
    ///
    /// 返回 `true` 表示过渡仍在进行中（需要继续请求下一帧）；
    /// 返回 `false` 表示已收敛到达终态（调度器可回落 0% 占用）。
    pub fn advance(&mut self, now_secs: f64) -> bool {
        let Some(active) = &mut self.active else {
            return false;
        };
        let elapsed = (now_secs - active.start_time_secs).max(0.0) as f32;
        let raw = if active.duration_secs <= 0.0 {
            1.0
        } else {
            (elapsed / active.duration_secs).clamp(0.0, 1.0)
        };
        active.raw_progress = raw;

        if raw >= 1.0 {
            self.current_colors = self.target_colors;
            self.active = None;
            self.lut = pack_lut(&self.current_colors);
            return false;
        }

        let eased = ease_out_cubic_bezier(raw);
        let from_colors = active.from_colors;
        let mut blended = [Color::TRANSPARENT; MAX_TOKENS];
        for i in 1..MAX_TOKENS {
            blended[i] = interpolate_color_oklab(from_colors[i], self.target_colors[i], eased);
        }
        blended[Token::NONE.index()] = Color::TRANSPARENT;
        self.current_colors = blended;
        self.lut = pack_lut(&blended);
        true
    }

    /// 是否正在执行主题过渡。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// 当前线性时间进度（`0.0..=1.0`，静止时恒为 `1.0`）。
    #[must_use]
    pub fn progress(&self) -> f32 {
        self.active.as_ref().map_or(1.0, |a| a.raw_progress)
    }

    /// 当前缓动进度（`0.0..=1.0`，静止时恒为 `1.0`）。
    #[must_use]
    pub fn eased_progress(&self) -> f32 {
        self.active
            .as_ref()
            .map_or(1.0, |a| ease_out_cubic_bezier(a.raw_progress))
    }

    /// 当前过渡在 `now_secs` 时刻的剩余时长（秒）；静止时为 `0.0`。
    #[must_use]
    pub fn remaining_secs(&self, now_secs: f64) -> f32 {
        let Some(active) = &self.active else {
            return 0.0;
        };
        let elapsed = (now_secs - active.start_time_secs).max(0.0) as f32;
        (active.duration_secs - elapsed).max(0.0)
    }

    /// 目标主题种类。
    #[must_use]
    pub const fn target_kind(&self) -> ThemeKind {
        self.target_kind
    }

    /// 原生后景（如 DWM `DWMWA_USE_IMMERSIVE_DARK_MODE`）在当前时刻应采用的主题。
    ///
    /// 过渡进度未过中点（`progress < 0.5`）时维持起点主题，跨越中点（`>= 0.5`）后
    /// 一次性翻转至目标主题，避免背景先于前景猛闪。
    #[must_use]
    pub fn effective_backdrop_theme(&self) -> ThemeKind {
        match &self.active {
            Some(active) if active.raw_progress < 0.5 => active.from_kind,
            _ => self.target_kind,
        }
    }

    /// 当前帧可直接上传至 GPU 的 4 KiB 主题 LUT。
    #[must_use]
    pub const fn lut(&self) -> &ThemeLut {
        &self.lut
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThemeSet;

    #[test]
    fn default_config_is_240ms_and_respects_reduced_motion() {
        let cfg = ThemeTransitionConfig::default();
        assert!((cfg.duration_secs - 0.24).abs() < 1e-6);
        assert!((cfg.effective_duration_secs(false) - 0.24).abs() < 1e-6);
        assert_eq!(cfg.effective_duration_secs(true), 0.0);
        assert_eq!(
            ThemeTransitionConfig::from_millis(0).effective_duration_secs(false),
            0.0
        );
        assert!(
            (ThemeTransitionConfig::from_millis(300).effective_duration_secs(false) - 0.3).abs()
                < 1e-6
        );
    }

    #[test]
    fn oklab_interpolation_keeps_none_transparent_and_avoids_dark_fringe() {
        let transparent_black = Color::new(0.0, 0.0, 0.0, 0.0);
        let white = Color::new(1.0, 1.0, 1.0, 1.0);
        let mid = interpolate_color_oklab(transparent_black, white, 0.5);
        // 单侧透明端点必须沿用非透明端 RGB，仅插值 Alpha。
        assert!((mid.r - 1.0).abs() < 1e-5);
        assert!((mid.g - 1.0).abs() < 1e-5);
        assert!((mid.b - 1.0).abs() < 1e-5);
        assert!((mid.a - 0.5).abs() < 1e-5);

        let mid_rev = interpolate_color_oklab(white, transparent_black, 0.5);
        assert!((mid_rev.r - 1.0).abs() < 1e-5);
        assert!((mid_rev.a - 0.5).abs() < 1e-5);
    }

    #[test]
    fn transition_converges_monotonically_and_flips_backdrop_at_midpoint() {
        let set = ThemeSet::default_set();
        let light = set.palette(ThemeKind::Light);
        let dark = set.palette(ThemeKind::Dark);

        let mut tr = ThemeTransition::new(light);
        assert!(!tr.is_active());
        assert_eq!(tr.effective_backdrop_theme(), ThemeKind::Light);

        // 切到相同主题是无操作。
        assert!(!tr.begin(light, 1.0, 0.24));
        assert!(!tr.is_active());

        // 启动 Light -> Dark 过渡。
        assert!(tr.begin(dark, 1.0, 0.24));
        assert!(tr.is_active());

        // t = 0.25 (< 0.5)：后景仍保持 Light。
        assert!(tr.advance(1.06));
        assert_eq!(tr.effective_backdrop_theme(), ThemeKind::Light);
        assert_eq!(tr.lut().color(Token::NONE), Color::TRANSPARENT);
        let mid_surface = tr.lut().color(Token::SURFACE);
        let light_surface = light.color(Token::SURFACE);
        let dark_surface = dark.color(Token::SURFACE);
        assert!(mid_surface.r < light_surface.r && mid_surface.r > dark_surface.r);

        // t = 0.50：后景翻转至 Dark。
        assert!(tr.advance(1.12));
        assert_eq!(tr.effective_backdrop_theme(), ThemeKind::Dark);

        // t = 1.00：收敛静止，色值精确到达目标。
        assert!(!tr.advance(1.24));
        assert!(!tr.is_active());
        assert_eq!(tr.progress(), 1.0);
        let end_surface = tr.lut().color(Token::SURFACE);
        assert!((end_surface.r - dark_surface.r).abs() < 1e-2);
    }

    #[test]
    fn mid_transition_reversal_scales_duration_and_preserves_continuity() {
        let set = ThemeSet::default_set();
        let light = set.palette(ThemeKind::Light);
        let dark = set.palette(ThemeKind::Dark);

        let mut tr = ThemeTransition::new(light);
        assert!(tr.begin(dark, 10.0, 0.24));
        // 推进 40%（0.096s）。
        assert!(tr.advance(10.096));
        let visual_at_reversal = tr.lut().color(Token::SURFACE);

        // 中途反向切回 Light：应从当前视觉色平滑折返，且时长按 0.4 比例缩短至约 0.096s。
        assert!(tr.begin(light, 10.096, 0.24));
        let rem = tr.remaining_secs(10.096);
        assert!(
            rem < 0.12 && rem > 0.08,
            "reversed duration should scale with elapsed progress, got {rem}"
        );
        // 折返瞬间（dt = 0）颜色不得跳变。
        assert!(tr.advance(10.096));
        let visual_after_reversal = tr.lut().color(Token::SURFACE);
        assert!((visual_after_reversal.r - visual_at_reversal.r).abs() < 1e-2);
        // 经过缩减后的时长应收敛回 Light。
        assert!(!tr.advance(10.096 + rem as f64 + 0.005));
        assert!(!tr.is_active());
    }

    #[test]
    fn zero_duration_snaps_immediately() {
        let set = ThemeSet::default_set();
        let light = set.palette(ThemeKind::Light);
        let dark = set.palette(ThemeKind::Dark);

        let mut tr = ThemeTransition::new(light);
        assert!(tr.begin(dark, 1.0, 0.0));
        assert!(!tr.is_active());
        assert_eq!(tr.effective_backdrop_theme(), ThemeKind::Dark);
        let end = tr.lut().color(Token::SURFACE);
        let expected = dark.color(Token::SURFACE);
        assert!((end.r - expected.r).abs() < 1e-2);
    }
}
