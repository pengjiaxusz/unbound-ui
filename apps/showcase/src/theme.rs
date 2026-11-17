//! 展台主题：直接沿用框架的 Windows 11 默认主题。
//!
//! 展台不再维护自己的 Material 色板——那会让「框架默认长什么样」和
//! 「展台长什么样」分叉，演示反而掩盖了 Win11 的层次。这里只做一层薄适配：
//! 把 `unbound_theme::ThemeSet::default_set()`（Win11 Fluent + 蓝色强调色）
//! 打包成 GPU 查找表，供页面按亮 / 暗模式取用。
//!
//! 颜色仍以语义 token 表达，图元只携带 token 下标，因此切换主题不重传图元。

use unbound_theme::{Palette, ThemeKind, ThemeLut, ThemeSet};

/// 框架默认主题集（Windows 11 Fluent + 蓝色强调色）。
#[must_use]
pub fn theme_set() -> ThemeSet {
    ThemeSet::default_set()
}

/// 亮 / 暗模式对应的主题类型。
#[must_use]
pub fn kind(dark: bool) -> ThemeKind {
    if dark {
        ThemeKind::Dark
    } else {
        ThemeKind::Light
    }
}

/// 亮 / 暗模式对应的调色板。
#[must_use]
pub fn palette(dark: bool) -> Palette {
    theme_set().palette(kind(dark)).clone()
}

/// 亮 / 暗模式对应的 4 KiB 颜色查找表。
#[must_use]
pub fn lut(dark: bool) -> ThemeLut {
    ThemeLut::from_palette(&palette(dark))
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_math::color::linear_to_srgb;
    use unbound_theme::Token;

    fn alpha(lut: &ThemeLut, token: Token) -> f32 {
        lut.color(token).to_array()[3]
    }

    fn srgb8(value: f32) -> f32 {
        linear_to_srgb(value.clamp(0.0, 1.0)) * 255.0
    }

    #[test]
    fn the_lut_is_exactly_four_kibibytes() {
        assert_eq!(lut(false).bytes().len(), 4096);
        assert_eq!(lut(true).bytes().len(), 4096);
    }

    #[test]
    fn the_showcase_uses_the_win11_base_surfaces() {
        let light = lut(false);
        let dark = lut(true);
        // Win11 `SolidBackgroundFillColorBase`：浅色 #F3F3F3，深色 #202020。
        let light_surface = light.color(Token::SURFACE).to_array();
        let dark_surface = dark.color(Token::SURFACE).to_array();
        assert!((srgb8(light_surface[0]) - 243.0).abs() < 1.0);
        assert!((srgb8(dark_surface[0]) - 32.0).abs() < 1.0);
    }

    #[test]
    fn the_header_and_navigation_are_transparent_so_mica_shows_through() {
        for dark in [false, true] {
            let lut = lut(dark);
            assert_eq!(alpha(&lut, Token::HEADER_BG), 0.0);
            assert_eq!(alpha(&lut, Token::NAV_BG), 0.0);
        }
    }

    #[test]
    fn every_engine_token_is_defined_in_both_themes() {
        for dark in [false, true] {
            let lut = lut(dark);
            for token in unbound_theme::TOKENS {
                let color = lut.color(token);
                assert!(
                    color.to_array().iter().all(|channel| channel.is_finite()),
                    "token {} is not finite in {} theme",
                    token.raw(),
                    if dark { "dark" } else { "light" }
                );
            }
        }
    }
}
