//! 展台主题回归：展台必须与框架默认主题（Windows 11 Fluent）保持一致。
//!
//! 这里不再钉死整张色板快照——色板是 `unbound-theme` 的单一事实源，
//! 快照会随框架调色而漂移。测试改为锁定「展台确实用了 Win11 默认」这一契约：
//! 基准面、云母透明语义、亮暗差异与 4 KiB 查找表结构。

use showcase::theme;
use unbound_math::color::linear_to_srgb;
use unbound_math::Color;
use unbound_theme::{ThemeLut, Token};

fn srgb8(value: f32) -> f32 {
    linear_to_srgb(value.clamp(0.0, 1.0)) * 255.0
}

fn channel(lut: &ThemeLut, token: Token, index: usize) -> f32 {
    srgb8(lut.color(token).to_array()[index])
}

fn alpha(lut: &ThemeLut, token: Token) -> f32 {
    lut.color(token).to_array()[3]
}

#[test]
fn the_lut_is_exactly_four_kibibytes() {
    assert_eq!(theme::lut(false).bytes().len(), 4096);
    assert_eq!(theme::lut(true).bytes().len(), 4096);
}

#[test]
fn the_showcase_uses_the_win11_base_surfaces() {
    let light = theme::lut(false);
    let dark = theme::lut(true);
    // WinUI `SolidBackgroundFillColorBase`：浅色 #F3F3F3、深色 #202020；
    // 卡片 `CardBackgroundFillColorDefault`：浅色 #FFFFFF、深色 #2B2B2B。
    assert!((channel(&light, Token::SURFACE, 0) - 243.0).abs() < 1.0);
    assert!((channel(&dark, Token::SURFACE, 0) - 32.0).abs() < 1.0);
    assert!((channel(&light, Token::SURFACE_RAISED, 0) - 255.0).abs() < 1.0);
    assert!((channel(&dark, Token::SURFACE_RAISED, 0) - 43.0).abs() < 1.0);
}

#[test]
fn the_header_and_navigation_stay_transparent_for_mica() {
    for dark in [false, true] {
        let lut = theme::lut(dark);
        assert_eq!(alpha(&lut, Token::HEADER_BG), 0.0);
        assert_eq!(alpha(&lut, Token::NAV_BG), 0.0);
    }
}

#[test]
fn the_two_themes_differ_where_they_should() {
    let light = theme::lut(false);
    let dark = theme::lut(true);
    let luminance = |color: Color| {
        let channels = color.to_array();
        0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]
    };
    assert!(
        luminance(light.color(Token::SURFACE)) > luminance(dark.color(Token::SURFACE)),
        "light surface must be brighter than dark surface"
    );
    for token in unbound_theme::TOKENS {
        assert!(
            light.color(token).to_array().iter().all(|c| c.is_finite()),
            "token {} missing in light theme",
            token.raw()
        );
        assert!(
            dark.color(token).to_array().iter().all(|c| c.is_finite()),
            "token {} missing in dark theme",
            token.raw()
        );
    }
}

#[test]
fn the_unused_slots_stay_transparent() {
    let lut = theme::lut(false);
    let highest = unbound_theme::TOKENS
        .iter()
        .map(|token| token.raw())
        .max()
        .unwrap_or(0);
    for slot in (highest + 1)..512 {
        assert_eq!(
            lut.color(Token::new(slot)).to_array(),
            [0.0, 0.0, 0.0, 0.0],
            "slot {slot} must stay fully transparent"
        );
    }
}

#[test]
fn slot_zero_is_transparent_and_slot_one_is_the_shadow() {
    let lut = theme::lut(true);
    assert_eq!(lut.color(Token::NONE).to_array(), [0.0, 0.0, 0.0, 0.0]);
    // 引擎着色器从槽位 1 读取阴影；Win11 深色的 SHADOW 是 55% 黑。
    let shadow_alpha = alpha(&lut, Token::SHADOW);
    assert!(
        (shadow_alpha - 140.0 / 255.0).abs() < 1e-3,
        "slot 1 shadow alpha is {shadow_alpha}"
    );
}
