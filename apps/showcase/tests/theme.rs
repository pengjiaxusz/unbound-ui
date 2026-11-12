//! 主题收敛后的**像素一致性**回归。
//!
//! 展台不再手写 512 槽颜色表，改为由 [`unbound_theme`] 依据语义 token 打包。
//! 这份用例把迁移前每个语义槽的**精确颜色**钉死：任何一个 token 的槽位
//! 映射、颜色值或亮暗对应关系发生变化，测试立刻失败。
//!
//! 期望值取自迁移前手写颜色表的每一行（sRGB 十六进制 + 透明度）。

use showcase::theme;
use unbound_math::color::linear_to_srgb;
use unbound_math::Color;
use unbound_theme::Token;

/// 一套主题里某个槽位的期望颜色。
struct Expected {
    token: Token,
    light: (&'static str, u8),
    dark: (&'static str, u8),
}

/// 迁移前手写颜色表的完整快照。
const EXPECTED: &[Expected] = &[
    Expected {
        token: Token::SHADOW,
        light: ("#14161E", 60),
        dark: ("#000000", 160),
    },
    Expected {
        token: Token::SURFACE,
        light: ("#F5F6FA", 255),
        dark: ("#121216", 255),
    },
    Expected {
        token: Token::SURFACE_RAISED,
        light: ("#FFFFFF", 255),
        dark: ("#202128", 255),
    },
    Expected {
        token: Token::OUTLINE,
        light: ("#DEE0E8", 255),
        dark: ("#3A3C48", 255),
    },
    Expected {
        token: Token::PRIMARY,
        light: ("#3464DC", 255),
        dark: ("#588CFF", 255),
    },
    Expected {
        token: Token::SECONDARY,
        light: ("#7850D2", 255),
        dark: ("#966EF0", 255),
    },
    Expected {
        token: Token::SUCCESS,
        light: ("#1EA064", 255),
        dark: ("#46BE82", 255),
    },
    Expected {
        token: Token::WARNING,
        light: ("#DC9B28", 255),
        dark: ("#EBAF46", 255),
    },
    Expected {
        token: Token::ERROR,
        light: ("#DC3C41", 255),
        dark: ("#EB5A5F", 255),
    },
    Expected {
        token: Token::TEXT_PRIMARY,
        light: ("#1C1E26", 255),
        dark: ("#EEEFF5", 255),
    },
    Expected {
        token: Token::TEXT_SECONDARY,
        light: ("#787C8C", 255),
        dark: ("#8C90A0", 255),
    },
    Expected {
        token: Token::PRIMARY_HOVER,
        light: ("#4678F0", 255),
        dark: ("#6EA0FF", 255),
    },
    Expected {
        token: Token::PRIMARY_PRESSED,
        light: ("#244EBE", 255),
        dark: ("#406EDC", 255),
    },
    Expected {
        token: Token::ON_PRIMARY,
        light: ("#FFFFFF", 255),
        dark: ("#0C101A", 255),
    },
    Expected {
        token: Token::FOCUS_RING,
        light: ("#1C5AD2", 255),
        dark: ("#96C3FF", 255),
    },
    Expected {
        token: Token::DISABLED,
        light: ("#C8CCD6", 255),
        dark: ("#464852", 255),
    },
    Expected {
        token: Token::ON_DISABLED,
        light: ("#787C8C", 255),
        dark: ("#8C90A0", 255),
    },
    Expected {
        token: Token::HEADER_BG,
        light: ("#FFFFFF", 255),
        dark: ("#171A21", 255),
    },
    Expected {
        token: Token::NAV_BG,
        light: ("#F7F8FB", 255),
        dark: ("#14171D", 255),
    },
    Expected {
        token: Token::NAV_HOVER,
        light: ("#ECEEF4", 255),
        dark: ("#232833", 255),
    },
    Expected {
        token: Token::NAV_SELECTED,
        light: ("#E4EBFD", 255),
        dark: ("#26334F", 255),
    },
    Expected {
        token: Token::CARD_BG,
        light: ("#FFFFFF", 255),
        dark: ("#1B1F27", 255),
    },
    Expected {
        token: Token::CARD_BORDER,
        light: ("#E4E6EE", 255),
        dark: ("#2C3240", 255),
    },
    Expected {
        token: Token::CANVAS_BACKDROP,
        light: ("#F5F6FA", 255),
        dark: ("#171A21", 255),
    },
    Expected {
        token: Token::GRID,
        light: ("#E3E6EF", 255),
        dark: ("#262B36", 255),
    },
    Expected {
        token: Token::CONTROL_TRACK,
        light: ("#E4E7EF", 255),
        dark: ("#2D3341", 255),
    },
    Expected {
        token: Token::CONTROL_THUMB,
        light: ("#FFFFFF", 255),
        dark: ("#EDF1FA", 255),
    },
    Expected {
        token: Token::CONTROL_HOVER,
        light: ("#ECEFF6", 255),
        dark: ("#384052", 255),
    },
    Expected {
        token: Token::CODE_BG,
        light: ("#1C2029", 255),
        dark: ("#0E1015", 255),
    },
    Expected {
        token: Token::CODE_TEXT,
        light: ("#D7DCEA", 255),
        dark: ("#C9D2E4", 255),
    },
    Expected {
        token: Token::SCRIM,
        light: ("#0F121A", 90),
        dark: ("#000000", 150),
    },
    Expected {
        token: Token::OVERLAY_BG,
        light: ("#FFFFFF", 255),
        dark: ("#20252F", 255),
    },
    Expected {
        token: Token::PRIMARY_SOFT,
        light: ("#E8EEFE", 255),
        dark: ("#22304F", 255),
    },
    Expected {
        token: Token::SUCCESS_SOFT,
        light: ("#E4F6EC", 255),
        dark: ("#1C3A2A", 255),
    },
    Expected {
        token: Token::WARNING_SOFT,
        light: ("#FCF2DF", 255),
        dark: ("#3B2F1B", 255),
    },
    Expected {
        token: Token::ERROR_SOFT,
        light: ("#FDE9EA", 255),
        dark: ("#3D2226", 255),
    },
    Expected {
        token: Token::HERO_START,
        light: ("#3B6EF6", 255),
        dark: ("#4C7DFF", 255),
    },
    Expected {
        token: Token::HERO_END,
        light: ("#8A5CF6", 255),
        dark: ("#A06BFF", 255),
    },
    Expected {
        token: Token::ON_HERO,
        light: ("#FFFFFF", 255),
        dark: ("#FFFFFF", 255),
    },
    Expected {
        token: Token::SELECTION,
        light: ("#3B6EF6", 40),
        dark: ("#78A5FF", 48),
    },
    Expected {
        token: Token::DIVIDER,
        light: ("#EAECF2", 255),
        dark: ("#262C38", 255),
    },
    Expected {
        token: Token::HUD_BG,
        light: ("#FFFFFF", 246),
        dark: ("#080A0E", 224),
    },
    Expected {
        token: Token::HUD_TEXT,
        light: ("#1C1E26", 255),
        dark: ("#DCE3F2", 255),
    },
    Expected {
        token: Token::ACCENT_SOFT,
        light: ("#FFF0E8", 255),
        dark: ("#3A2A22", 255),
    },
    Expected {
        token: Token::GRID_DOT,
        light: ("#D8DCE8", 255),
        dark: ("#333A48", 255),
    },
    Expected {
        token: Token::SECONDARY_SOFT,
        light: ("#F0EAFE", 255),
        dark: ("#2E2542", 255),
    },
    Expected {
        token: Token::NAV_TEXT,
        light: ("#2A2F3A", 255),
        dark: ("#E4E9F3", 255),
    },
    Expected {
        token: Token::NAV_TEXT_MUTED,
        light: ("#7A8090", 255),
        dark: ("#8A93A6", 255),
    },
    Expected {
        token: Token::HERO_SCRIM,
        light: ("#0F121C", 200),
        dark: ("#080A10", 200),
    },
];

/// A colour as sRGB hex plus the 8-bit alpha.
fn srgb(color: Color) -> (String, u8) {
    let channels = color.to_array();
    let channel = |value: f32| (linear_to_srgb(value.clamp(0.0, 1.0)) * 255.0).round() as u8;
    (
        format!(
            "#{:02X}{:02X}{:02X}",
            channel(channels[0]),
            channel(channels[1]),
            channel(channels[2])
        ),
        (channels[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[test]
fn every_slot_kept_its_colour_through_the_migration() {
    let light = theme::lut(false);
    let dark = theme::lut(true);
    for expected in EXPECTED {
        let actual_light = srgb(light.color(expected.token));
        assert_eq!(
            actual_light,
            (expected.light.0.to_string(), expected.light.1),
            "亮色主题的 token {} 变了",
            expected.token.raw()
        );
        let actual_dark = srgb(dark.color(expected.token));
        assert_eq!(
            actual_dark,
            (expected.dark.0.to_string(), expected.dark.1),
            "暗色主题的 token {} 变了",
            expected.token.raw()
        );
    }
}

#[test]
fn the_two_themes_share_one_slot_layout() {
    let light = theme::lut(false);
    let dark = theme::lut(true);
    for expected in EXPECTED {
        assert_eq!(
            light.color(expected.token).to_array()[3] > 0.0,
            dark.color(expected.token).to_array()[3] > 0.0,
            "token {} 在两个主题里的存在性必须一致",
            expected.token.raw()
        );
    }
}

#[test]
fn the_unused_slots_stay_transparent() {
    let lut = theme::lut(false);
    for slot in 100..512 {
        let color = lut.color(Token::new(slot as u16));
        let channels = color.to_array();
        assert_eq!(
            channels,
            [0.0, 0.0, 0.0, 0.0],
            "槽位 {slot} 必须保持完全透明"
        );
    }
}

#[test]
fn the_slot_zero_is_transparent_and_slot_one_is_the_shadow() {
    let lut = theme::lut(true);
    assert_eq!(lut.color(Token::NONE).to_array(), [0.0, 0.0, 0.0, 0.0]);
    // The engine's shader reads the shadow from slot 1. The table stores
    // half floats, so the alpha comes back quantised.
    let shadow_alpha = lut.color(Token::SHADOW).to_array()[3];
    assert!(
        (shadow_alpha - 160.0 / 255.0).abs() < 1e-3,
        "槽位 1 的阴影透明度是 {shadow_alpha}"
    );
}
