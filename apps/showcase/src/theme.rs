//! 展台的品牌配色。
//!
//! 这里**不**再手写 512 槽的颜色表：颜色以
//! `(语义 token, sRGB)` 的形式列出，由 [`unbound_theme`] 统一打包成 4 KiB
//! 的查找表。图元永远只携带 token 下标，因此切换主题不重传任何图元。
//!
//! 亮 / 暗两套配色共用同一份 token 布局 —— 变的只有颜色值，这正是
//! "主题切换只写一次颜色表" 的前提。

use unbound_theme::{Palette, ThemeKind, ThemeLut, ThemeSet, Token};

/// 亮色主题。
const LIGHT: &[(Token, [u8; 4])] = &[
    (Token::SHADOW, [20, 22, 30, 60]),
    (Token::SURFACE, [245, 246, 250, 255]),
    (Token::SURFACE_RAISED, [255, 255, 255, 255]),
    (Token::OUTLINE, [222, 224, 232, 255]),
    (Token::PRIMARY, [52, 100, 220, 255]),
    (Token::SECONDARY, [120, 80, 210, 255]),
    (Token::SUCCESS, [30, 160, 100, 255]),
    (Token::WARNING, [220, 155, 40, 255]),
    (Token::ERROR, [220, 60, 65, 255]),
    (Token::TEXT_PRIMARY, [28, 30, 38, 255]),
    (Token::TEXT_SECONDARY, [120, 124, 140, 255]),
    (Token::PRIMARY_HOVER, [70, 120, 240, 255]),
    (Token::PRIMARY_PRESSED, [36, 78, 190, 255]),
    (Token::ON_PRIMARY, [255, 255, 255, 255]),
    (Token::FOCUS_RING, [28, 90, 210, 255]),
    (Token::DISABLED, [200, 204, 214, 255]),
    (Token::ON_DISABLED, [120, 124, 140, 255]),
    (Token::HEADER_BG, [255, 255, 255, 255]),
    (Token::NAV_BG, [247, 248, 251, 255]),
    (Token::NAV_HOVER, [236, 238, 244, 255]),
    (Token::NAV_SELECTED, [228, 235, 253, 255]),
    (Token::CARD_BG, [255, 255, 255, 255]),
    (Token::CARD_BORDER, [228, 230, 238, 255]),
    (Token::CANVAS_BACKDROP, [245, 246, 250, 255]),
    (Token::GRID, [227, 230, 239, 255]),
    (Token::CONTROL_TRACK, [228, 231, 239, 255]),
    (Token::CONTROL_THUMB, [255, 255, 255, 255]),
    (Token::CONTROL_HOVER, [236, 239, 246, 255]),
    (Token::CODE_BG, [28, 32, 41, 255]),
    (Token::CODE_TEXT, [215, 220, 234, 255]),
    (Token::SCRIM, [15, 18, 26, 90]),
    (Token::OVERLAY_BG, [255, 255, 255, 255]),
    (Token::PRIMARY_SOFT, [232, 238, 254, 255]),
    (Token::SUCCESS_SOFT, [228, 246, 236, 255]),
    (Token::WARNING_SOFT, [252, 242, 223, 255]),
    (Token::ERROR_SOFT, [253, 233, 234, 255]),
    (Token::HERO_START, [59, 110, 246, 255]),
    (Token::HERO_END, [138, 92, 246, 255]),
    (Token::ON_HERO, [255, 255, 255, 255]),
    (Token::SELECTION, [59, 110, 246, 40]),
    (Token::DIVIDER, [234, 236, 242, 255]),
    (Token::HUD_BG, [255, 255, 255, 246]),
    (Token::HUD_TEXT, [28, 30, 38, 255]),
    (Token::ACCENT_SOFT, [255, 240, 232, 255]),
    (Token::GRID_DOT, [216, 220, 232, 255]),
    (Token::SECONDARY_SOFT, [240, 234, 254, 255]),
    (Token::NAV_TEXT, [42, 47, 58, 255]),
    (Token::NAV_TEXT_MUTED, [122, 128, 144, 255]),
    (Token::HERO_SCRIM, [15, 18, 28, 200]),
];

/// 暗色主题。
const DARK: &[(Token, [u8; 4])] = &[
    (Token::SHADOW, [0, 0, 0, 160]),
    (Token::SURFACE, [18, 18, 22, 255]),
    (Token::SURFACE_RAISED, [32, 33, 40, 255]),
    (Token::OUTLINE, [58, 60, 72, 255]),
    (Token::PRIMARY, [88, 140, 255, 255]),
    (Token::SECONDARY, [150, 110, 240, 255]),
    (Token::SUCCESS, [70, 190, 130, 255]),
    (Token::WARNING, [235, 175, 70, 255]),
    (Token::ERROR, [235, 90, 95, 255]),
    (Token::TEXT_PRIMARY, [238, 239, 245, 255]),
    (Token::TEXT_SECONDARY, [140, 144, 160, 255]),
    (Token::PRIMARY_HOVER, [110, 160, 255, 255]),
    (Token::PRIMARY_PRESSED, [64, 110, 220, 255]),
    (Token::ON_PRIMARY, [12, 16, 26, 255]),
    (Token::FOCUS_RING, [150, 195, 255, 255]),
    (Token::DISABLED, [70, 72, 82, 255]),
    (Token::ON_DISABLED, [140, 144, 160, 255]),
    (Token::HEADER_BG, [23, 26, 33, 255]),
    (Token::NAV_BG, [20, 23, 29, 255]),
    (Token::NAV_HOVER, [35, 40, 51, 255]),
    (Token::NAV_SELECTED, [38, 51, 79, 255]),
    (Token::CARD_BG, [27, 31, 39, 255]),
    (Token::CARD_BORDER, [44, 50, 64, 255]),
    (Token::CANVAS_BACKDROP, [23, 26, 33, 255]),
    (Token::GRID, [38, 43, 54, 255]),
    (Token::CONTROL_TRACK, [45, 51, 65, 255]),
    (Token::CONTROL_THUMB, [237, 241, 250, 255]),
    (Token::CONTROL_HOVER, [56, 64, 82, 255]),
    (Token::CODE_BG, [14, 16, 21, 255]),
    (Token::CODE_TEXT, [201, 210, 228, 255]),
    (Token::SCRIM, [0, 0, 0, 150]),
    (Token::OVERLAY_BG, [32, 37, 47, 255]),
    (Token::PRIMARY_SOFT, [34, 48, 79, 255]),
    (Token::SUCCESS_SOFT, [28, 58, 42, 255]),
    (Token::WARNING_SOFT, [59, 47, 27, 255]),
    (Token::ERROR_SOFT, [61, 34, 38, 255]),
    (Token::HERO_START, [76, 125, 255, 255]),
    (Token::HERO_END, [160, 107, 255, 255]),
    (Token::ON_HERO, [255, 255, 255, 255]),
    (Token::SELECTION, [120, 165, 255, 48]),
    (Token::DIVIDER, [38, 44, 56, 255]),
    (Token::HUD_BG, [8, 10, 14, 224]),
    (Token::HUD_TEXT, [220, 227, 242, 255]),
    (Token::ACCENT_SOFT, [58, 42, 34, 255]),
    (Token::GRID_DOT, [51, 58, 72, 255]),
    (Token::SECONDARY_SOFT, [46, 37, 66, 255]),
    (Token::NAV_TEXT, [228, 233, 243, 255]),
    (Token::NAV_TEXT_MUTED, [138, 147, 166, 255]),
    (Token::HERO_SCRIM, [8, 10, 16, 200]),
];

/// 高对比度主题：纯黑底、纯白字，强调色拉到极限。
const HIGH_CONTRAST: &[(Token, [u8; 4])] = &[
    (Token::SHADOW, [0, 0, 0, 255]),
    (Token::SURFACE, [0, 0, 0, 255]),
    (Token::SURFACE_RAISED, [12, 12, 12, 255]),
    (Token::OUTLINE, [255, 255, 255, 255]),
    (Token::PRIMARY, [255, 230, 0, 255]),
    (Token::SECONDARY, [0, 220, 255, 255]),
    (Token::SUCCESS, [0, 255, 120, 255]),
    (Token::WARNING, [255, 170, 0, 255]),
    (Token::ERROR, [255, 60, 60, 255]),
    (Token::TEXT_PRIMARY, [255, 255, 255, 255]),
    (Token::TEXT_SECONDARY, [230, 230, 230, 255]),
    (Token::PRIMARY_HOVER, [255, 245, 120, 255]),
    (Token::PRIMARY_PRESSED, [200, 180, 0, 255]),
    (Token::ON_PRIMARY, [0, 0, 0, 255]),
    (Token::FOCUS_RING, [255, 230, 0, 255]),
    (Token::DISABLED, [60, 60, 60, 255]),
    (Token::ON_DISABLED, [200, 200, 200, 255]),
    (Token::HEADER_BG, [0, 0, 0, 255]),
    (Token::NAV_BG, [8, 8, 8, 255]),
    (Token::NAV_HOVER, [48, 48, 48, 255]),
    (Token::NAV_SELECTED, [255, 230, 0, 255]),
    (Token::CARD_BG, [10, 10, 10, 255]),
    (Token::CARD_BORDER, [255, 255, 255, 255]),
    (Token::CANVAS_BACKDROP, [0, 0, 0, 255]),
    (Token::GRID, [80, 80, 80, 255]),
    (Token::CONTROL_TRACK, [60, 60, 60, 255]),
    (Token::CONTROL_THUMB, [255, 255, 255, 255]),
    (Token::CONTROL_HOVER, [64, 64, 64, 255]),
    (Token::CODE_BG, [0, 0, 0, 255]),
    (Token::CODE_TEXT, [255, 255, 255, 255]),
    (Token::SCRIM, [0, 0, 0, 255]),
    (Token::OVERLAY_BG, [0, 0, 0, 255]),
    (Token::PRIMARY_SOFT, [60, 58, 0, 255]),
    (Token::SUCCESS_SOFT, [0, 60, 32, 255]),
    (Token::WARNING_SOFT, [60, 48, 0, 255]),
    (Token::ERROR_SOFT, [66, 16, 16, 255]),
    (Token::HERO_START, [0, 120, 255, 255]),
    (Token::HERO_END, [120, 0, 255, 255]),
    (Token::ON_HERO, [0, 0, 0, 255]),
    (Token::SELECTION, [255, 230, 0, 96]),
    (Token::DIVIDER, [255, 255, 255, 255]),
    (Token::HUD_BG, [0, 0, 0, 255]),
    (Token::HUD_TEXT, [255, 255, 255, 255]),
    (Token::ACCENT_SOFT, [60, 32, 16, 255]),
    (Token::GRID_DOT, [80, 80, 80, 255]),
    (Token::SECONDARY_SOFT, [0, 58, 66, 255]),
    (Token::NAV_TEXT, [255, 255, 255, 255]),
    (Token::NAV_TEXT_MUTED, [220, 220, 220, 255]),
    (Token::HERO_SCRIM, [0, 0, 0, 255]),
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
        Palette::from_srgb8(ThemeKind::Light, LIGHT),
        Palette::from_srgb8(ThemeKind::Dark, DARK),
        Palette::from_srgb8(ThemeKind::HighContrast, HIGH_CONTRAST),
    )
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
        assert_eq!(light, dark, "亮暗主题必须共用同一份槽位布局");

        let contrast: Vec<u16> = HIGH_CONTRAST.iter().map(|(token, _)| token.raw()).collect();
        assert_eq!(light, contrast);
    }

    #[test]
    fn no_token_is_listed_twice() {
        for set in [LIGHT, DARK, HIGH_CONTRAST] {
            let mut seen = Vec::new();
            for (token, _) in set {
                assert!(!seen.contains(&token.raw()), "token 重复");
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
            "主色在两个主题里都必须不透明"
        );
    }
}
