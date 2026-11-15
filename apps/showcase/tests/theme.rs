//! 主题收敛后的**像素一致性**回归。
//!
//! 展台不再手写 512 槽颜色表，改为由 [`unbound_theme`] 依据语义 token 打包。
//! 这份用例把阶段 1 色板基线下每个语义槽的**精确颜色**钉死：任何一个
//! token 的槽位映射、颜色值或亮暗对应关系发生变化，测试立刻失败。
//!
//! 期望值与 `src/theme.rs` 的 LIGHT / DARK 表逐行对应（sRGB 十六进制 +
//! 透明度）；色板重调时必须同步刷新本快照。

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

/// 阶段 1 色板基线的完整快照。
const EXPECTED: &[Expected] = &[
    Expected {
        token: Token::SHADOW,
        light: ("#14161E", 60),
        dark: ("#000000", 160),
    },
    Expected {
        token: Token::SURFACE,
        light: ("#F5F6FA", 255),
        dark: ("#121318", 255),
    },
    Expected {
        token: Token::SURFACE_RAISED,
        light: ("#FFFFFF", 255),
        dark: ("#20232C", 255),
    },
    Expected {
        token: Token::OUTLINE,
        light: ("#D6DAE4", 255),
        dark: ("#404654", 255),
    },
    Expected {
        token: Token::PRIMARY,
        light: ("#3464DC", 255),
        dark: ("#588CFF", 255),
    },
    Expected {
        token: Token::SECONDARY,
        light: ("#7850D2", 255),
        dark: ("#9C74F8", 255),
    },
    Expected {
        token: Token::SUCCESS,
        light: ("#1EA064", 255),
        dark: ("#48C486", 255),
    },
    Expected {
        token: Token::WARNING,
        light: ("#DC9B28", 255),
        dark: ("#F0B248", 255),
    },
    Expected {
        token: Token::ERROR,
        light: ("#DC3C41", 255),
        dark: ("#F05C60", 255),
    },
    Expected {
        token: Token::TEXT_PRIMARY,
        light: ("#1C1E26", 255),
        dark: ("#F0F2F8", 255),
    },
    Expected {
        token: Token::TEXT_SECONDARY,
        light: ("#6E7384", 255),
        dark: ("#9AA0B2", 255),
    },
    Expected {
        token: Token::PRIMARY_HOVER,
        light: ("#4678F0", 255),
        dark: ("#70A2FF", 255),
    },
    Expected {
        token: Token::PRIMARY_PRESSED,
        light: ("#244EBE", 255),
        dark: ("#4070E0", 255),
    },
    Expected {
        token: Token::ON_PRIMARY,
        light: ("#FFFFFF", 255),
        dark: ("#FFFFFF", 255),
    },
    Expected {
        token: Token::FOCUS_RING,
        light: ("#306EEB", 255),
        dark: ("#88B8FF", 255),
    },
    Expected {
        token: Token::DISABLED,
        light: ("#C8CCD6", 255),
        dark: ("#3E424E", 255),
    },
    Expected {
        token: Token::ON_DISABLED,
        light: ("#787C8C", 255),
        dark: ("#868C9C", 255),
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
        light: ("#DEE2EC", 255),
        dark: ("#323846", 255),
    },
    Expected {
        token: Token::CANVAS_BACKDROP,
        light: ("#F5F6FA", 255),
        dark: ("#15181F", 255),
    },
    Expected {
        token: Token::GRID,
        light: ("#E3E6EF", 255),
        dark: ("#2A303C", 255),
    },
    Expected {
        token: Token::CONTROL_TRACK,
        light: ("#DEE2EC", 255),
        dark: ("#2E3442", 255),
    },
    Expected {
        token: Token::CONTROL_THUMB,
        light: ("#FFFFFF", 255),
        dark: ("#F2F5FC", 255),
    },
    Expected {
        token: Token::CONTROL_HOVER,
        light: ("#EAEEF6", 255),
        dark: ("#363E4E", 255),
    },
    Expected {
        token: Token::CODE_BG,
        light: ("#161A22", 255),
        dark: ("#0E1016", 255),
    },
    Expected {
        token: Token::CODE_TEXT,
        light: ("#E0E6F2", 255),
        dark: ("#D8E0F0", 255),
    },
    Expected {
        token: Token::SCRIM,
        light: ("#0F121A", 110),
        dark: ("#04060A", 155),
    },
    Expected {
        token: Token::OVERLAY_BG,
        light: ("#FFFFFF", 255),
        dark: ("#1E232D", 255),
    },
    Expected {
        token: Token::PRIMARY_SOFT,
        light: ("#E4ECFE", 255),
        dark: ("#243456", 255),
    },
    Expected {
        token: Token::SUCCESS_SOFT,
        light: ("#E0F6EA", 255),
        dark: ("#1C3C2C", 255),
    },
    Expected {
        token: Token::WARNING_SOFT,
        light: ("#FDF1DA", 255),
        dark: ("#3E301A", 255),
    },
    Expected {
        token: Token::ERROR_SOFT,
        light: ("#FDE6E8", 255),
        dark: ("#402226", 255),
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
        light: ("#3B6EF6", 48),
        dark: ("#6094FF", 64),
    },
    Expected {
        token: Token::DIVIDER,
        light: ("#E2E5EE", 255),
        dark: ("#2E3442", 255),
    },
    Expected {
        token: Token::HUD_BG,
        light: ("#FFFFFF", 246),
        dark: ("#12151C", 240),
    },
    Expected {
        token: Token::HUD_TEXT,
        light: ("#1C1E26", 255),
        dark: ("#E8EEF8", 255),
    },
    Expected {
        token: Token::ACCENT_SOFT,
        light: ("#FFEEE4", 255),
        dark: ("#3E2A20", 255),
    },
    Expected {
        token: Token::GRID_DOT,
        light: ("#D2D7E4", 255),
        dark: ("#363E4C", 255),
    },
    Expected {
        token: Token::SECONDARY_SOFT,
        light: ("#EEE8FE", 255),
        dark: ("#302648", 255),
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

/// 以 sRGB 十六进制加上 8 位 alpha 表示的颜色。
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
fn every_slot_matches_the_palette_baseline() {
    let light = theme::lut(false);
    let dark = theme::lut(true);
    for expected in EXPECTED {
        let actual_light = srgb(light.color(expected.token));
        assert_eq!(
            actual_light,
            (expected.light.0.to_string(), expected.light.1),
            "light-theme token {} changed",
            expected.token.raw()
        );
        let actual_dark = srgb(dark.color(expected.token));
        assert_eq!(
            actual_dark,
            (expected.dark.0.to_string(), expected.dark.1),
            "dark-theme token {} changed",
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
            "token {} must exist in both themes",
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
            "slot {slot} must stay fully transparent"
        );
    }
}

#[test]
fn the_slot_zero_is_transparent_and_slot_one_is_the_shadow() {
    let lut = theme::lut(true);
    assert_eq!(lut.color(Token::NONE).to_array(), [0.0, 0.0, 0.0, 0.0]);
    // 引擎的着色器从槽位 1 读取阴影。颜色表存储半精度浮点数，因此回读的 alpha 会被量化。
    //
    let shadow_alpha = lut.color(Token::SHADOW).to_array()[3];
    assert!(
        (shadow_alpha - 160.0 / 255.0).abs() < 1e-3,
        "slot 1 shadow alpha is {shadow_alpha}"
    );
}
