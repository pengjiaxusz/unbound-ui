//! 展台的品牌配色。
//!
//! 这里**不**再手写 512 槽的颜色表：颜色以
//! `(语义 token, sRGB)` 的形式列出，由 [`unbound_theme`] 统一打包成 4 KiB
//! 的查找表。图元永远只携带 token 下标，因此切换主题不重传任何图元。
//!
//! 亮 / 暗 / 高对比度三套配色共用同一份 token 布局，且 100% 覆盖
//! [`unbound_theme::TOKENS`] 中的全部 73 个语义令牌（单元测试对此作强制门禁），
//! 杜绝任何组件因未定义令牌而回退为全透明 `[0, 0, 0, 0]`。

use unbound_theme::{Palette, ThemeKind, ThemeLut, ThemeSet, Token};

/// 亮色主题（完整覆盖全部 73 个引擎语义令牌）。
const LIGHT: &[(Token, [u8; 4])] = &[
    (Token::SHADOW, [20, 22, 30, 60]),
    (Token::SURFACE, [245, 246, 250, 255]),
    (Token::SURFACE_RAISED, [255, 255, 255, 255]),
    (Token::SURFACE_SUNKEN, [235, 238, 245, 255]),
    (Token::ON_SURFACE, [28, 30, 38, 255]),
    (Token::ON_SURFACE_MUTED, [108, 114, 130, 255]),
    (Token::OUTLINE, [214, 218, 228, 255]),
    (Token::OUTLINE_STRONG, [148, 155, 170, 255]),
    (Token::PRIMARY, [52, 100, 220, 255]),
    (Token::ON_PRIMARY, [255, 255, 255, 255]),
    (Token::ACCENT, [245, 110, 60, 255]),
    (Token::ON_ACCENT, [255, 255, 255, 255]),
    (Token::SECONDARY, [120, 80, 210, 255]),
    (Token::SUCCESS, [30, 160, 100, 255]),
    (Token::WARNING, [220, 155, 40, 255]),
    (Token::ERROR, [220, 60, 65, 255]),
    (Token::ON_ERROR, [255, 255, 255, 255]),
    (Token::SELECTION, [59, 110, 246, 48]),
    (Token::ON_SELECTION, [18, 24, 36, 255]),
    (Token::HOVER, [20, 24, 34, 18]),
    (Token::PRESSED, [20, 24, 34, 34]),
    (Token::DISABLED, [200, 204, 214, 255]),
    (Token::ON_DISABLED, [120, 124, 140, 255]),
    (Token::GRID, [227, 230, 239, 255]),
    (Token::CANVAS_BACKDROP, [245, 246, 250, 255]),
    (Token::CANVAS_NODE, [255, 255, 255, 255]),
    (Token::CANVAS_NODE_SELECTED, [52, 100, 220, 255]),
    (Token::SNAP_GUIDE, [240, 60, 100, 255]),
    (Token::TEXT_PRIMARY, [28, 30, 38, 255]),
    (Token::TEXT_SECONDARY, [110, 115, 132, 255]),
    (Token::TEXT_DISABLED, [160, 165, 178, 255]),
    (Token::SCRIM, [15, 18, 26, 110]),
    (Token::PRIMARY_HOVER, [70, 120, 240, 255]),
    (Token::PRIMARY_PRESSED, [36, 78, 190, 255]),
    (Token::PRIMARY_SOFT, [228, 236, 254, 255]),
    (Token::SECONDARY_SOFT, [238, 232, 254, 255]),
    (Token::SUCCESS_SOFT, [224, 246, 234, 255]),
    (Token::WARNING_SOFT, [253, 241, 218, 255]),
    (Token::ERROR_SOFT, [253, 230, 232, 255]),
    (Token::FOCUS_RING, [48, 110, 235, 255]),
    (Token::CONTROL_TRACK, [222, 226, 236, 255]),
    (Token::CONTROL_THUMB, [255, 255, 255, 255]),
    (Token::CONTROL_HOVER, [234, 238, 246, 255]),
    (Token::CONTROL_FILL, [246, 248, 252, 255]),
    (Token::CONTROL_STROKE, [205, 210, 222, 255]),
    (Token::CONTROL_TRACK_STROKE, [186, 192, 206, 255]),
    (Token::CONTROL_THUMB_ON, [255, 255, 255, 255]),
    (Token::SWITCH_THUMB_OFF, [110, 115, 132, 255]),
    (Token::HEADER_BG, [255, 255, 255, 255]),
    (Token::NAV_BG, [247, 248, 251, 255]),
    (Token::NAV_HOVER, [236, 238, 244, 255]),
    (Token::NAV_SELECTED, [228, 235, 253, 255]),
    (Token::DIVIDER, [226, 229, 238, 255]),
    (Token::CODE_BG, [22, 26, 34, 255]),
    (Token::CODE_TEXT, [224, 230, 242, 255]),
    (Token::OVERLAY_BG, [255, 255, 255, 255]),
    (Token::HERO_START, [59, 110, 246, 255]),
    (Token::HERO_END, [138, 92, 246, 255]),
    (Token::ON_HERO, [255, 255, 255, 255]),
    (Token::HERO_SCRIM, [15, 18, 28, 200]),
    (Token::HUD_BG, [255, 255, 255, 246]),
    (Token::HUD_TEXT, [28, 30, 38, 255]),
    (Token::ACCENT_SOFT, [255, 238, 228, 255]),
    (Token::GRID_DOT, [210, 215, 228, 255]),
    (Token::NAV_TEXT, [42, 47, 58, 255]),
    (Token::NAV_TEXT_MUTED, [122, 128, 144, 255]),
    (Token::CARD_BG, [255, 255, 255, 255]),
    (Token::CARD_BORDER, [222, 226, 236, 255]),
    (Token::CONTROL_DISABLED_BG, [238, 240, 245, 255]),
    (Token::CONTROL_DISABLED_BORDER, [212, 216, 224, 255]),
    (Token::CONTROL_DISABLED_THUMB, [192, 196, 206, 255]),
    (Token::INDETERMINATE_FILL, [52, 100, 220, 255]),
    (Token::INPUT_BG, [255, 255, 255, 255]),
    (Token::INPUT_BORDER_HOVER, [152, 160, 178, 255]),
    (Token::INPUT_BORDER_ERROR, [220, 60, 65, 255]),
    (Token::INPUT_BORDER_WARNING, [220, 155, 40, 255]),
    (Token::PREEDIT_UNDERLINE, [52, 100, 220, 255]),
    (Token::SUBMENU_ARROW, [108, 114, 130, 255]),
];

/// 暗色主题（完整覆盖全部 73 个引擎语义令牌，`ON_PRIMARY` 采用高对比度纯白）。
const DARK: &[(Token, [u8; 4])] = &[
    (Token::SHADOW, [0, 0, 0, 160]),
    (Token::SURFACE, [18, 19, 24, 255]),
    (Token::SURFACE_RAISED, [32, 35, 44, 255]),
    (Token::SURFACE_SUNKEN, [13, 15, 20, 255]),
    (Token::ON_SURFACE, [238, 240, 246, 255]),
    (Token::ON_SURFACE_MUTED, [156, 163, 180, 255]),
    (Token::OUTLINE, [64, 70, 84, 255]),
    (Token::OUTLINE_STRONG, [108, 116, 134, 255]),
    (Token::PRIMARY, [88, 140, 255, 255]),
    (Token::ON_PRIMARY, [255, 255, 255, 255]),
    (Token::ACCENT, [255, 136, 84, 255]),
    (Token::ON_ACCENT, [255, 255, 255, 255]),
    (Token::SECONDARY, [156, 116, 248, 255]),
    (Token::SUCCESS, [72, 196, 134, 255]),
    (Token::WARNING, [240, 178, 72, 255]),
    (Token::ERROR, [240, 92, 96, 255]),
    (Token::ON_ERROR, [255, 255, 255, 255]),
    (Token::SELECTION, [96, 148, 255, 64]),
    (Token::ON_SELECTION, [242, 246, 255, 255]),
    (Token::HOVER, [238, 240, 246, 22]),
    (Token::PRESSED, [238, 240, 246, 40]),
    (Token::DISABLED, [62, 66, 78, 255]),
    (Token::ON_DISABLED, [134, 140, 156, 255]),
    (Token::GRID, [42, 48, 60, 255]),
    (Token::CANVAS_BACKDROP, [21, 24, 31, 255]),
    (Token::CANVAS_NODE, [36, 40, 50, 255]),
    (Token::CANVAS_NODE_SELECTED, [88, 140, 255, 255]),
    (Token::SNAP_GUIDE, [255, 92, 126, 255]),
    (Token::TEXT_PRIMARY, [240, 242, 248, 255]),
    (Token::TEXT_SECONDARY, [154, 160, 178, 255]),
    (Token::TEXT_DISABLED, [104, 110, 126, 255]),
    (Token::SCRIM, [4, 6, 10, 155]),
    (Token::PRIMARY_HOVER, [112, 162, 255, 255]),
    (Token::PRIMARY_PRESSED, [64, 112, 224, 255]),
    (Token::PRIMARY_SOFT, [36, 52, 86, 255]),
    (Token::SECONDARY_SOFT, [48, 38, 72, 255]),
    (Token::SUCCESS_SOFT, [28, 60, 44, 255]),
    (Token::WARNING_SOFT, [62, 48, 26, 255]),
    (Token::ERROR_SOFT, [64, 34, 38, 255]),
    (Token::FOCUS_RING, [136, 184, 255, 255]),
    (Token::CONTROL_TRACK, [46, 52, 66, 255]),
    (Token::CONTROL_THUMB, [242, 245, 252, 255]),
    (Token::CONTROL_HOVER, [54, 62, 78, 255]),
    (Token::CONTROL_FILL, [40, 45, 58, 255]),
    (Token::CONTROL_STROKE, [78, 85, 102, 255]),
    (Token::CONTROL_TRACK_STROKE, [96, 104, 122, 255]),
    (Token::CONTROL_THUMB_ON, [255, 255, 255, 255]),
    (Token::SWITCH_THUMB_OFF, [154, 160, 178, 255]),
    (Token::HEADER_BG, [23, 26, 33, 255]),
    (Token::NAV_BG, [20, 23, 29, 255]),
    (Token::NAV_HOVER, [35, 40, 51, 255]),
    (Token::NAV_SELECTED, [38, 51, 79, 255]),
    (Token::DIVIDER, [46, 52, 66, 255]),
    (Token::CODE_BG, [14, 16, 22, 255]),
    (Token::CODE_TEXT, [216, 224, 240, 255]),
    (Token::OVERLAY_BG, [30, 35, 45, 255]),
    (Token::HERO_START, [76, 125, 255, 255]),
    (Token::HERO_END, [160, 107, 255, 255]),
    (Token::ON_HERO, [255, 255, 255, 255]),
    (Token::HERO_SCRIM, [8, 10, 16, 200]),
    (Token::HUD_BG, [18, 21, 28, 240]),
    (Token::HUD_TEXT, [232, 238, 248, 255]),
    (Token::ACCENT_SOFT, [62, 42, 32, 255]),
    (Token::GRID_DOT, [54, 62, 76, 255]),
    (Token::NAV_TEXT, [228, 233, 243, 255]),
    (Token::NAV_TEXT_MUTED, [138, 147, 166, 255]),
    (Token::CARD_BG, [27, 31, 39, 255]),
    (Token::CARD_BORDER, [50, 56, 70, 255]),
    (Token::CONTROL_DISABLED_BG, [38, 42, 52, 255]),
    (Token::CONTROL_DISABLED_BORDER, [64, 70, 82, 255]),
    (Token::CONTROL_DISABLED_THUMB, [96, 102, 116, 255]),
    (Token::INDETERMINATE_FILL, [88, 140, 255, 255]),
    (Token::INPUT_BG, [24, 28, 36, 255]),
    (Token::INPUT_BORDER_HOVER, [110, 118, 138, 255]),
    (Token::INPUT_BORDER_ERROR, [240, 92, 96, 255]),
    (Token::INPUT_BORDER_WARNING, [240, 178, 72, 255]),
    (Token::PREEDIT_UNDERLINE, [88, 140, 255, 255]),
    (Token::SUBMENU_ARROW, [156, 163, 180, 255]),
];

/// 高对比度主题：纯黑底、纯白字，强调色拉到极限（完整覆盖全部 73 个引擎语义令牌）。
const HIGH_CONTRAST: &[(Token, [u8; 4])] = &[
    (Token::SHADOW, [0, 0, 0, 255]),
    (Token::SURFACE, [0, 0, 0, 255]),
    (Token::SURFACE_RAISED, [12, 12, 12, 255]),
    (Token::SURFACE_SUNKEN, [0, 0, 0, 255]),
    (Token::ON_SURFACE, [255, 255, 255, 255]),
    (Token::ON_SURFACE_MUTED, [235, 235, 235, 255]),
    (Token::OUTLINE, [255, 255, 255, 255]),
    (Token::OUTLINE_STRONG, [255, 255, 255, 255]),
    (Token::PRIMARY, [255, 230, 0, 255]),
    (Token::ON_PRIMARY, [0, 0, 0, 255]),
    (Token::ACCENT, [0, 235, 255, 255]),
    (Token::ON_ACCENT, [0, 0, 0, 255]),
    (Token::SECONDARY, [0, 220, 255, 255]),
    (Token::SUCCESS, [0, 255, 120, 255]),
    (Token::WARNING, [255, 170, 0, 255]),
    (Token::ERROR, [255, 60, 60, 255]),
    (Token::ON_ERROR, [0, 0, 0, 255]),
    (Token::SELECTION, [255, 230, 0, 110]),
    (Token::ON_SELECTION, [0, 0, 0, 255]),
    (Token::HOVER, [255, 255, 255, 48]),
    (Token::PRESSED, [255, 255, 255, 96]),
    (Token::DISABLED, [60, 60, 60, 255]),
    (Token::ON_DISABLED, [200, 200, 200, 255]),
    (Token::GRID, [96, 96, 96, 255]),
    (Token::CANVAS_BACKDROP, [0, 0, 0, 255]),
    (Token::CANVAS_NODE, [16, 16, 16, 255]),
    (Token::CANVAS_NODE_SELECTED, [255, 230, 0, 255]),
    (Token::SNAP_GUIDE, [255, 0, 128, 255]),
    (Token::TEXT_PRIMARY, [255, 255, 255, 255]),
    (Token::TEXT_SECONDARY, [230, 230, 230, 255]),
    (Token::TEXT_DISABLED, [180, 180, 180, 255]),
    (Token::SCRIM, [0, 0, 0, 255]),
    (Token::PRIMARY_HOVER, [255, 245, 120, 255]),
    (Token::PRIMARY_PRESSED, [200, 180, 0, 255]),
    (Token::PRIMARY_SOFT, [60, 58, 0, 255]),
    (Token::SECONDARY_SOFT, [0, 58, 66, 255]),
    (Token::SUCCESS_SOFT, [0, 60, 32, 255]),
    (Token::WARNING_SOFT, [60, 48, 0, 255]),
    (Token::ERROR_SOFT, [66, 16, 16, 255]),
    (Token::FOCUS_RING, [255, 230, 0, 255]),
    (Token::CONTROL_TRACK, [60, 60, 60, 255]),
    (Token::CONTROL_THUMB, [255, 255, 255, 255]),
    (Token::CONTROL_HOVER, [64, 64, 64, 255]),
    (Token::CONTROL_FILL, [0, 0, 0, 255]),
    (Token::CONTROL_STROKE, [255, 255, 255, 255]),
    (Token::CONTROL_TRACK_STROKE, [138, 138, 138, 255]),
    (Token::CONTROL_THUMB_ON, [0, 0, 0, 255]),
    (Token::SWITCH_THUMB_OFF, [255, 255, 255, 255]),
    (Token::HEADER_BG, [0, 0, 0, 255]),
    (Token::NAV_BG, [8, 8, 8, 255]),
    (Token::NAV_HOVER, [48, 48, 48, 255]),
    (Token::NAV_SELECTED, [255, 230, 0, 255]),
    (Token::DIVIDER, [255, 255, 255, 255]),
    (Token::CODE_BG, [0, 0, 0, 255]),
    (Token::CODE_TEXT, [255, 255, 255, 255]),
    (Token::OVERLAY_BG, [0, 0, 0, 255]),
    (Token::HERO_START, [0, 120, 255, 255]),
    (Token::HERO_END, [120, 0, 255, 255]),
    (Token::ON_HERO, [0, 0, 0, 255]),
    (Token::HERO_SCRIM, [0, 0, 0, 255]),
    (Token::HUD_BG, [0, 0, 0, 255]),
    (Token::HUD_TEXT, [255, 255, 255, 255]),
    (Token::ACCENT_SOFT, [60, 32, 16, 255]),
    (Token::GRID_DOT, [80, 80, 80, 255]),
    (Token::NAV_TEXT, [255, 255, 255, 255]),
    (Token::NAV_TEXT_MUTED, [220, 220, 220, 255]),
    (Token::CARD_BG, [10, 10, 10, 255]),
    (Token::CARD_BORDER, [255, 255, 255, 255]),
    (Token::CONTROL_DISABLED_BG, [32, 32, 32, 255]),
    (Token::CONTROL_DISABLED_BORDER, [255, 255, 255, 255]),
    (Token::CONTROL_DISABLED_THUMB, [200, 200, 200, 255]),
    (Token::INDETERMINATE_FILL, [255, 230, 0, 255]),
    (Token::INPUT_BG, [0, 0, 0, 255]),
    (Token::INPUT_BORDER_HOVER, [255, 230, 0, 255]),
    (Token::INPUT_BORDER_ERROR, [255, 60, 60, 255]),
    (Token::INPUT_BORDER_WARNING, [255, 170, 0, 255]),
    (Token::PREEDIT_UNDERLINE, [255, 230, 0, 255]),
    (Token::SUBMENU_ARROW, [255, 255, 255, 255]),
];

/// 展台使用的三套主题。
#[must_use]
pub fn theme_set() -> ThemeSet {
    let (light, dark, high_contrast) = palettes();
    ThemeSet::from_palettes(light, dark, high_contrast)
}

/// 三套配色。
fn palettes() -> (Palette, Palette, Palette) {
    (
        with_swatch(ThemeKind::Light, LIGHT),
        with_swatch(ThemeKind::Dark, DARK),
        with_swatch(ThemeKind::HighContrast, HIGH_CONTRAST),
    )
}

/// 给一套配色补上 10 格品牌色板。
///
/// 「颜色网格」这类控件按槽位逐格上色，因此每套品牌各占一个调色板槽位。
/// 静态表只写死界面外壳的颜色，色板由 [`AccentBrand`] 的 OKLCH 阶梯生成，
/// 与框架侧 [`unbound_theme`] 的槽位顺序严格一致。
fn with_swatch(kind: ThemeKind, base: &[(Token, [u8; 4])]) -> Palette {
    let mut entries: Vec<(Token, [u8; 4])> = base.to_vec();
    for (idx, brand) in unbound_theme::AccentBrand::ALL.iter().enumerate() {
        let ramp = if kind == ThemeKind::Dark {
            brand.dark_ramp().0
        } else {
            brand.light_ramp().0
        };
        let [r, g, b, _a] = ramp.to_array();
        entries.push((
            Token::accent_swatch(idx),
            [
                linear_to_srgb8(r),
                linear_to_srgb8(g),
                linear_to_srgb8(b),
                255,
            ],
        ));
    }
    Palette::from_srgb8(kind, &entries)
}

/// 线性分量转 8 位 sRGB。
fn linear_to_srgb8(v: f32) -> u8 {
    let srgb = unbound_math::color::linear_to_srgb(v).clamp(0.0, 1.0);
    (srgb * 255.0).round() as u8
}

/// 一套主题的颜色表。
#[must_use]
pub fn lut(dark: bool) -> ThemeLut {
    ThemeLut::from_palette(&palette(dark))
}

/// 一套主题的颜色。
#[must_use]
pub fn palette(dark: bool) -> Palette {
    let (light, dark_palette, _) = palettes();
    if dark {
        dark_palette
    } else {
        light
    }
}

/// 主题对应的语义槽位。
#[must_use]
pub fn kind(dark: bool) -> ThemeKind {
    if dark {
        ThemeKind::Dark
    } else {
        ThemeKind::Light
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_themes_define_the_same_tokens() {
        let light: Vec<u16> = LIGHT.iter().map(|(token, _)| token.raw()).collect();
        let dark: Vec<u16> = DARK.iter().map(|(token, _)| token.raw()).collect();
        assert_eq!(
            light, dark,
            "light and dark themes must share the same slot layout"
        );

        let contrast: Vec<u16> = HIGH_CONTRAST.iter().map(|(token, _)| token.raw()).collect();
        assert_eq!(light, contrast);
    }

    #[test]
    fn every_engine_token_is_defined_and_non_transparent_in_showcase_palettes() {
        let set = theme_set();
        for kind in ThemeKind::ALL {
            let pal = set.palette(kind);
            for token in unbound_theme::TOKENS {
                let c = pal.color(token);
                assert!(
                    c.a > 0.0,
                    "token {:?} (raw={}) is transparent in showcase {:?}",
                    token,
                    token.raw(),
                    kind
                );
            }
        }
    }

    #[test]
    fn no_token_is_listed_twice() {
        for set in [LIGHT, DARK, HIGH_CONTRAST] {
            let mut seen = Vec::new();
            for (token, _) in set {
                assert!(!seen.contains(&token.raw()), "duplicate token");
                seen.push(token.raw());
            }
        }
    }

    #[test]
    fn the_lut_is_exactly_four_kibibytes() {
        assert_eq!(lut(false).bytes().len(), 4096);
        assert_eq!(lut(true).bytes().len(), 4096);
    }

    #[test]
    fn the_two_themes_differ_where_they_should() {
        let light = lut(false);
        let dark = lut(true);
        assert_ne!(light.color(Token::PRIMARY), dark.color(Token::PRIMARY));
        assert_eq!(
            light.color(Token::PRIMARY).to_array()[3],
            dark.color(Token::PRIMARY).to_array()[3],
            "the primary color must stay opaque in both themes"
        );
    }
}
