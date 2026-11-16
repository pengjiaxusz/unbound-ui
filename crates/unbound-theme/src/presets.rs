//! 双产品主题预设（`Dunting` 与 `RenderDebuggerLauncher`）、10 套 OKLCH 品牌强调色与 7 档规范 UI 缩放阶梯（阶段 1 WP3）。
//!
//! 严格遵循 `unbound-theme` 的核心不变量：
//! * 所有预设最终生成标准的 512 槽 RGBA16F（4 KiB）颜色查找表 [`ThemeLut`]；
//! * 切换产品预设或切换 10 套 OKLCH 品牌强调色仅更新 4 KiB LUT，**0 字节图元重传、0 重新布局**。

use unbound_math::color::{Color, Oklch};

use crate::{Palette, ThemeKind, ThemeSet, Token};

/// 7 档规范 UI 缩放步进（75% ~ 200%）。
pub const CANONICAL_SCALE_STEPS: [f32; 7] = [0.75, 0.875, 1.0, 1.125, 1.25, 1.5, 2.0];

/// 将任意缩放因子吸附到最近的规范缩放档位。
#[must_use]
pub fn snap_ui_scale(scale: f32) -> f32 {
    let mut best = CANONICAL_SCALE_STEPS[2];
    let mut best_diff = f32::INFINITY;
    for step in CANONICAL_SCALE_STEPS {
        let diff = (scale - step).abs();
        if diff < best_diff {
            best_diff = diff;
            best = step;
        }
    }
    best
}

/// 按步进方向（`delta > 0` 放大一档，`delta < 0` 缩小一档，`delta == 0` 重置为 `1.0`）计算下一档 UI 缩放。
#[must_use]
pub fn step_ui_scale(current: f32, delta: i32) -> f32 {
    if delta == 0 {
        return 1.0;
    }
    let snapped = snap_ui_scale(current);
    let idx = CANONICAL_SCALE_STEPS
        .iter()
        .position(|&s| (s - snapped).abs() < 1e-4)
        .unwrap_or(2);
    let next_idx = (idx as i32 + delta.signum()).clamp(0, (CANONICAL_SCALE_STEPS.len() - 1) as i32);
    CANONICAL_SCALE_STEPS[next_idx as usize]
}

/// 宿主产品主题风格预设。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProductPreset {
    /// Windows 11 Fluent（WinUI 3）云母灰阶风格——`unbound-ui` 的**默认**风格。
    ///
    /// 卡片与控件填充是叠在云母背板上的白色薄纱（半透明令牌），
    /// 因此宿主应在窗口下方绘制「壁纸派生云母」背板，才能得到完整的层次感。
    #[default]
    Win11,
    /// Unbound 标准 Material 风格。
    Standard,
    /// `dunting` 数字资产管理器深邃暗房灰阶风格（低干扰中性灰底，突出贴图与模型固有色彩）。
    Dunting,
    /// `chahu-render-debugger` 启动器与调试面板高对比度工程冷灰风格。
    RenderDebuggerLauncher,
}

/// 10 套 OKLCH 品牌强调色预设。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum AccentBrand {
    #[default]
    Blue,
    Indigo,
    Violet,
    Emerald,
    Teal,
    Amber,
    Rose,
    Crimson,
    Cyan,
    Slate,
}

impl AccentBrand {
    /// 全部 10 套品牌强调色。
    pub const ALL: [Self; 10] = [
        Self::Blue,
        Self::Indigo,
        Self::Violet,
        Self::Emerald,
        Self::Teal,
        Self::Amber,
        Self::Rose,
        Self::Crimson,
        Self::Cyan,
        Self::Slate,
    ];

    /// 品牌色在 OKLCH 空间中的基准色相角（度）与色度 `(hue_deg, chroma)`。
    #[must_use]
    pub const fn oklch_hue_chroma(self) -> (f32, f32) {
        match self {
            Self::Blue => (255.0, 0.17),
            Self::Indigo => (275.0, 0.18),
            Self::Violet => (298.0, 0.18),
            Self::Emerald => (155.0, 0.16),
            Self::Teal => (190.0, 0.14),
            Self::Amber => (75.0, 0.16),
            Self::Rose => (12.0, 0.18),
            Self::Crimson => (25.0, 0.19),
            Self::Cyan => (218.0, 0.15),
            Self::Slate => (250.0, 0.05),
        }
    }

    /// 生成亮色模式下的 `(primary, hover, pressed, soft)` 线性颜色四元组。
    #[must_use]
    pub fn light_ramp(self) -> (Color, Color, Color, Color) {
        let (h, c) = self.oklch_hue_chroma();
        let primary = Oklch::new(0.56, c, h, 1.0).to_linear();
        let hover = Oklch::new(0.62, c, h, 1.0).to_linear();
        let pressed = Oklch::new(0.49, c, h, 1.0).to_linear();
        let soft = Oklch::new(0.94, (c * 0.25).min(0.05), h, 1.0).to_linear();
        (primary, hover, pressed, soft)
    }

    /// 生成暗色模式下的 `(primary, hover, pressed, soft)` 线性颜色四元组。
    #[must_use]
    pub fn dark_ramp(self) -> (Color, Color, Color, Color) {
        let (h, c) = self.oklch_hue_chroma();
        let primary = Oklch::new(0.68, c, h, 1.0).to_linear();
        let hover = Oklch::new(0.74, c, h, 1.0).to_linear();
        let pressed = Oklch::new(0.60, c, h, 1.0).to_linear();
        let soft = Oklch::new(0.28, (c * 0.45).min(0.08), h, 1.0).to_linear();
        (primary, hover, pressed, soft)
    }
}

impl ThemeSet {
    /// 根据宿主产品预设与 OKLCH 品牌强调色生成完整 3 主题（Light / Dark / HighContrast）调色板集合。
    #[must_use]
    pub fn for_product(preset: ProductPreset, accent: AccentBrand) -> Self {
        let base = Self::default_set();
        // Win11 预设自带完整八套调色板与专用强调色阶梯（WinUI 的强调填充更亮、
        // 且深色下配深色前景），因此以它为底再套品牌色；其余预设以 Material 为底。
        let (seed_light, seed_dark) = match preset {
            ProductPreset::Win11 => (crate::win11::light(), crate::win11::dark()),
            _ => (
                base.palette(ThemeKind::Light).clone(),
                base.palette(ThemeKind::Dark).clone(),
            ),
        };

        let mut light_entries: Vec<(Token, Color)> = crate::TOKENS
            .iter()
            .map(|&t| (t, seed_light.color(t)))
            .collect();
        let mut dark_entries: Vec<(Token, Color)> = crate::TOKENS
            .iter()
            .map(|&t| (t, seed_dark.color(t)))
            .collect();

        let set_tok = |list: &mut Vec<(Token, Color)>, tok: Token, col: Color| {
            if let Some(slot) = list.iter_mut().find(|(t, _)| *t == tok) {
                slot.1 = col;
            }
        };

        let (lp, lh, lpr, lsoft, lon) = if preset == ProductPreset::Win11 {
            crate::win11::accent_ramp(accent, false)
        } else {
            let (p, h, pr, soft) = accent.light_ramp();
            (p, h, pr, soft, Color::WHITE)
        };
        let (dp, dh, dpr, dsoft, don) = if preset == ProductPreset::Win11 {
            crate::win11::accent_ramp(accent, true)
        } else {
            let (p, h, pr, soft) = accent.dark_ramp();
            (p, h, pr, soft, Color::WHITE)
        };

        for (entries, (p, h, pr, soft, on)) in [
            (&mut light_entries, (lp, lh, lpr, lsoft, lon)),
            (&mut dark_entries, (dp, dh, dpr, dsoft, don)),
        ] {
            set_tok(entries, Token::PRIMARY, p);
            set_tok(entries, Token::PRIMARY_HOVER, h);
            set_tok(entries, Token::PRIMARY_PRESSED, pr);
            set_tok(entries, Token::PRIMARY_SOFT, soft);
            set_tok(entries, Token::FOCUS_RING, h);
            set_tok(entries, Token::SELECTION, p.with_alpha(0.22));
            set_tok(entries, Token::INDETERMINATE_FILL, p);
            set_tok(entries, Token::PREEDIT_UNDERLINE, p);
            if preset == ProductPreset::Win11 {
                set_tok(entries, Token::ON_PRIMARY, on);
            }
        }

        // 10 套品牌色板：每格填入对应品牌的主强调色，供「颜色网格」逐格上色。
        for (idx, brand) in crate::AccentBrand::ALL.iter().enumerate() {
            let light = if preset == ProductPreset::Win11 {
                crate::win11::accent_ramp(*brand, false).0
            } else {
                brand.light_ramp().0
            };
            let dark = if preset == ProductPreset::Win11 {
                crate::win11::accent_ramp(*brand, true).0
            } else {
                brand.dark_ramp().0
            };
            set_tok(&mut light_entries, Token::accent_swatch(idx), light);
            set_tok(&mut dark_entries, Token::accent_swatch(idx), dark);
        }

        match preset {
            ProductPreset::Win11 | ProductPreset::Standard => {}
            ProductPreset::Dunting => {
                // Dunting 采用中性暗房灰底，消除蓝色偏色对贴图/材质审视的干扰
                set_tok(
                    &mut dark_entries,
                    Token::SURFACE,
                    Color::from_srgb8(20, 20, 22, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::SURFACE_RAISED,
                    Color::from_srgb8(28, 28, 31, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::SURFACE_SUNKEN,
                    Color::from_srgb8(13, 13, 15, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::CARD_BG,
                    Color::from_srgb8(26, 26, 29, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::CANVAS_BACKDROP,
                    Color::from_srgb8(15, 15, 17, 255),
                );
            }
            ProductPreset::RenderDebuggerLauncher => {
                // RenderDebugger Launcher 对齐 Showcase Gallery 的深邃石板层次配色体系
                set_tok(
                    &mut dark_entries,
                    Token::SURFACE,
                    Color::from_srgb8(18, 19, 24, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::SURFACE_RAISED,
                    Color::from_srgb8(26, 28, 35, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::SURFACE_SUNKEN,
                    Color::from_srgb8(13, 14, 19, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::CARD_BG,
                    Color::from_srgb8(25, 27, 34, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::CARD_BORDER,
                    Color::from_srgb8(44, 48, 59, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::HEADER_BG,
                    Color::from_srgb8(17, 18, 23, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::NAV_BG,
                    Color::from_srgb8(21, 22, 28, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::NAV_HOVER,
                    Color::from_srgb8(33, 36, 45, 255),
                );
                set_tok(
                    &mut dark_entries,
                    Token::NAV_SELECTED,
                    Color::from_srgb8(34, 52, 86, 255),
                );
            }
        }

        Self::from_palettes(
            Palette::new(ThemeKind::Light, &light_entries),
            Palette::new(ThemeKind::Dark, &dark_entries),
            base.palette(ThemeKind::HighContrast).clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThemeLut;

    #[test]
    fn canonical_scale_steps_snap_and_step_cleanly() {
        assert_eq!(snap_ui_scale(1.02), 1.0);
        assert_eq!(snap_ui_scale(1.48), 1.5);
        assert_eq!(step_ui_scale(1.0, 1), 1.125);
        assert_eq!(step_ui_scale(1.0, -1), 0.875);
        assert_eq!(step_ui_scale(2.0, 1), 2.0);
        assert_eq!(step_ui_scale(0.75, -1), 0.75);
        assert_eq!(step_ui_scale(1.5, 0), 1.0);
    }

    #[test]
    fn all_ten_oklch_accents_and_product_presets_produce_valid_4kib_luts() {
        for preset in [
            ProductPreset::Win11,
            ProductPreset::Standard,
            ProductPreset::Dunting,
            ProductPreset::RenderDebuggerLauncher,
        ] {
            for accent in AccentBrand::ALL {
                let set = ThemeSet::for_product(preset, accent);
                for kind in ThemeKind::ALL {
                    let lut = ThemeLut::from_palette(set.palette(kind));
                    assert_eq!(lut.bytes().len(), 4096);
                    for token in crate::TOKENS {
                        let c = lut.color(token);
                        assert!(c.is_finite());
                        assert!(c.a > 0.0, "token {:?} transparent in {:?}", token, kind);
                    }
                }
            }
        }
    }
}
