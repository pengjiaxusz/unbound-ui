//! Windows 11（WinUI 3）Fluent 设计语言的语义调色板。
//!
//! 取值对齐 WinUI 3 的公开语义画刷（`SolidBackgroundFillColorBase`、
//! `CardBackgroundFillColorDefault`、`TextFillColorPrimary`、`DividerStrokeColorDefault`
//! 等），并保留其**半透明**语义：卡片与控件填充是叠加在云母背板上的白色薄纱，
//! 而非不透明实色。这正是 Win11 观感的核心——层次来自「同一张云母上叠几层薄纱」，
//! 而不是为每一层发明一个新灰色。
//!
//! 两套调色板都刻意让 [`crate::Token::SURFACE`] 保持不透明：它同时充当
//! GPU 硬件清屏色（交换链为 `OPAQUE` 合成），透明度会让未覆盖区域变黑。

use unbound_math::color::{Color, Oklch};

use crate::{Palette, ThemeKind, Token};

/// 从 8 位 sRGB 分量构造调色板表项。
fn c(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_srgb8(r, g, b, a)
}

/// Windows 11 深色调色板。
///
/// 基准面 `#202020` 即 `SolidBackgroundFillColorBase` 的深色值；卡片、输入框、
/// 悬停态统一为「白色 N%」的薄纱，叠在云母背板上得到 Win11 的层叠灰阶。
#[must_use]
pub fn dark() -> Palette {
    Palette::new(
        ThemeKind::Dark,
        &[
            // ---- 表面与文本 -------------------------------------------------
            (Token::SURFACE, c(32, 32, 32, 255)),
            (Token::SURFACE_RAISED, c(43, 43, 43, 255)),
            (Token::SURFACE_SUNKEN, c(26, 26, 26, 255)),
            (Token::ON_SURFACE, c(255, 255, 255, 255)),
            (Token::ON_SURFACE_MUTED, c(197, 197, 197, 255)),
            (Token::TEXT_PRIMARY, c(255, 255, 255, 255)),
            (Token::TEXT_SECONDARY, c(197, 197, 197, 255)),
            (Token::TEXT_DISABLED, c(255, 255, 255, 93)),
            // ---- 描边 -------------------------------------------------------
            (Token::OUTLINE, c(58, 58, 58, 255)),
            (Token::OUTLINE_STRONG, c(92, 92, 92, 255)),
            (Token::DIVIDER, c(255, 255, 255, 14)),
            (Token::CARD_BORDER, c(255, 255, 255, 13)),
            // ---- 语义状态 ---------------------------------------------------
            (Token::ERROR, c(255, 153, 164, 255)),
            (Token::ON_ERROR, c(0, 0, 0, 255)),
            (Token::WARNING, c(252, 225, 0, 255)),
            (Token::SUCCESS, c(108, 203, 95, 255)),
            (Token::ERROR_SOFT, c(58, 34, 36, 255)),
            (Token::WARNING_SOFT, c(51, 41, 12, 255)),
            (Token::SUCCESS_SOFT, c(35, 43, 30, 255)),
            (Token::SHADOW, c(0, 0, 0, 140)),
            (Token::SCRIM, c(0, 0, 0, 128)),
            // ---- 强调色（WinUI 深色强调填充 = #60CDFF，前景为黑） -------------
            (Token::PRIMARY, c(96, 205, 255, 255)),
            (Token::ON_PRIMARY, c(0, 20, 32, 255)),
            (Token::PRIMARY_HOVER, c(130, 216, 255, 255)),
            (Token::PRIMARY_PRESSED, c(76, 194, 255, 255)),
            (Token::PRIMARY_SOFT, c(10, 46, 66, 255)),
            (Token::ACCENT, c(153, 235, 255, 255)),
            (Token::ON_ACCENT, c(0, 20, 32, 255)),
            (Token::ACCENT_SOFT, c(10, 46, 66, 255)),
            (Token::SECONDARY, c(153, 235, 255, 255)),
            (Token::SECONDARY_SOFT, c(27, 42, 51, 255)),
            (Token::FOCUS_RING, c(255, 255, 255, 255)),
            (Token::INDETERMINATE_FILL, c(96, 205, 255, 255)),
            (Token::PREEDIT_UNDERLINE, c(96, 205, 255, 255)),
            // ---- 选区与控件状态（Win11 一律是白色薄纱） ----------------------
            (Token::SELECTION, c(96, 205, 255, 90)),
            (Token::ON_SELECTION, c(255, 255, 255, 255)),
            (Token::HOVER, c(255, 255, 255, 12)),
            (Token::PRESSED, c(255, 255, 255, 20)),
            (Token::DISABLED, c(255, 255, 255, 15)),
            (Token::ON_DISABLED, c(255, 255, 255, 93)),
            (Token::CONTROL_TRACK, c(107, 107, 107, 255)),
            (Token::CONTROL_THUMB, c(255, 255, 255, 255)),
            (Token::CONTROL_HOVER, c(45, 45, 45, 255)),
            (Token::CONTROL_DISABLED_BG, c(255, 255, 255, 10)),
            (Token::CONTROL_DISABLED_BORDER, c(255, 255, 255, 10)),
            (Token::CONTROL_DISABLED_THUMB, c(255, 255, 255, 93)),
            // ---- 外壳容器（云母之上不再加第二层不透明底） --------------------
            // 标题栏与导航面板不另加底色：Win11 的导航窗格本身就是云母。
            (Token::HEADER_BG, c(32, 32, 32, 0)),
            (Token::NAV_BG, c(32, 32, 32, 0)),
            (Token::NAV_HOVER, c(255, 255, 255, 12)),
            (Token::NAV_SELECTED, c(255, 255, 255, 16)),
            (Token::NAV_TEXT, c(255, 255, 255, 255)),
            (Token::NAV_TEXT_MUTED, c(197, 197, 197, 255)),
            (Token::CARD_BG, c(255, 255, 255, 12)),
            (Token::OVERLAY_BG, c(44, 44, 44, 255)),
            (Token::HUD_BG, c(44, 44, 44, 240)),
            (Token::HUD_TEXT, c(255, 255, 255, 255)),
            // ---- 输入 -------------------------------------------------------
            (Token::INPUT_BG, c(255, 255, 255, 12)),
            (Token::INPUT_BORDER_HOVER, c(255, 255, 255, 51)),
            (Token::INPUT_BORDER_ERROR, c(255, 153, 164, 255)),
            (Token::INPUT_BORDER_WARNING, c(252, 225, 0, 255)),
            (Token::SUBMENU_ARROW, c(197, 197, 197, 255)),
            // ---- 代码 / 画布 / 主视觉 ----------------------------------------
            (Token::CODE_BG, c(26, 26, 26, 255)),
            (Token::CODE_TEXT, c(214, 214, 214, 255)),
            (Token::CANVAS_BACKDROP, c(26, 26, 26, 255)),
            (Token::CANVAS_NODE, c(43, 43, 43, 255)),
            (Token::CANVAS_NODE_SELECTED, c(96, 205, 255, 255)),
            (Token::SNAP_GUIDE, c(255, 153, 164, 255)),
            (Token::GRID, c(255, 255, 255, 30)),
            (Token::GRID_DOT, c(255, 255, 255, 30)),
            (Token::HERO_START, c(96, 205, 255, 255)),
            (Token::HERO_END, c(120, 150, 255, 255)),
            (Token::ON_HERO, c(0, 20, 32, 255)),
            (Token::HERO_SCRIM, c(0, 0, 0, 180)),
        ],
    )
}

/// Windows 11 浅色调色板。
///
/// 基准面 `#F3F3F3` 即浅色云母的等效色调；卡片是 70% 白色薄纱（
/// `CardBackgroundFillColorDefault`），描边为 6% 黑色。
#[must_use]
pub fn light() -> Palette {
    Palette::new(
        ThemeKind::Light,
        &[
            (Token::SURFACE, c(243, 243, 243, 255)),
            (Token::SURFACE_RAISED, c(255, 255, 255, 255)),
            (Token::SURFACE_SUNKEN, c(233, 233, 233, 255)),
            (Token::ON_SURFACE, c(26, 26, 26, 255)),
            (Token::ON_SURFACE_MUTED, c(92, 92, 92, 255)),
            (Token::TEXT_PRIMARY, c(26, 26, 26, 255)),
            (Token::TEXT_SECONDARY, c(92, 92, 92, 255)),
            (Token::TEXT_DISABLED, c(26, 26, 26, 90)),
            (Token::OUTLINE, c(209, 209, 209, 255)),
            (Token::OUTLINE_STRONG, c(134, 134, 134, 255)),
            (Token::DIVIDER, c(0, 0, 0, 18)),
            (Token::CARD_BORDER, c(0, 0, 0, 15)),
            (Token::ERROR, c(196, 43, 28, 255)),
            (Token::ON_ERROR, c(255, 255, 255, 255)),
            (Token::WARNING, c(157, 93, 0, 255)),
            (Token::SUCCESS, c(15, 123, 15, 255)),
            (Token::ERROR_SOFT, c(253, 231, 233, 255)),
            (Token::WARNING_SOFT, c(252, 240, 214, 255)),
            (Token::SUCCESS_SOFT, c(223, 246, 221, 255)),
            (Token::SHADOW, c(0, 0, 0, 36)),
            (Token::SCRIM, c(0, 0, 0, 90)),
            (Token::PRIMARY, c(0, 95, 184, 255)),
            (Token::ON_PRIMARY, c(255, 255, 255, 255)),
            (Token::PRIMARY_HOVER, c(25, 118, 210, 255)),
            (Token::PRIMARY_PRESSED, c(0, 78, 152, 255)),
            (Token::PRIMARY_SOFT, c(219, 235, 250, 255)),
            (Token::ACCENT, c(0, 103, 192, 255)),
            (Token::ON_ACCENT, c(255, 255, 255, 255)),
            (Token::ACCENT_SOFT, c(232, 240, 250, 255)),
            (Token::SECONDARY, c(0, 103, 192, 255)),
            (Token::SECONDARY_SOFT, c(232, 240, 250, 255)),
            (Token::FOCUS_RING, c(26, 26, 26, 255)),
            (Token::INDETERMINATE_FILL, c(0, 95, 184, 255)),
            (Token::PREEDIT_UNDERLINE, c(0, 95, 184, 255)),
            (Token::SELECTION, c(0, 95, 184, 64)),
            (Token::ON_SELECTION, c(255, 255, 255, 255)),
            (Token::HOVER, c(0, 0, 0, 12)),
            (Token::PRESSED, c(0, 0, 0, 20)),
            (Token::DISABLED, c(0, 0, 0, 12)),
            (Token::ON_DISABLED, c(26, 26, 26, 90)),
            (Token::CONTROL_TRACK, c(138, 138, 138, 255)),
            (Token::CONTROL_THUMB, c(255, 255, 255, 255)),
            (Token::CONTROL_HOVER, c(245, 245, 245, 255)),
            (Token::CONTROL_DISABLED_BG, c(0, 0, 0, 8)),
            (Token::CONTROL_DISABLED_BORDER, c(0, 0, 0, 12)),
            (Token::CONTROL_DISABLED_THUMB, c(0, 0, 0, 60)),
            (Token::HEADER_BG, c(243, 243, 243, 0)),
            (Token::NAV_BG, c(243, 243, 243, 0)),
            (Token::NAV_HOVER, c(0, 0, 0, 8)),
            (Token::NAV_SELECTED, c(0, 0, 0, 14)),
            (Token::NAV_TEXT, c(26, 26, 26, 255)),
            (Token::NAV_TEXT_MUTED, c(92, 92, 92, 255)),
            (Token::CARD_BG, c(255, 255, 255, 178)),
            (Token::OVERLAY_BG, c(249, 249, 249, 255)),
            (Token::HUD_BG, c(249, 249, 249, 246)),
            (Token::HUD_TEXT, c(26, 26, 26, 255)),
            (Token::INPUT_BG, c(255, 255, 255, 178)),
            (Token::INPUT_BORDER_HOVER, c(26, 26, 26, 90)),
            (Token::INPUT_BORDER_ERROR, c(196, 43, 28, 255)),
            (Token::INPUT_BORDER_WARNING, c(157, 93, 0, 255)),
            (Token::SUBMENU_ARROW, c(92, 92, 92, 255)),
            (Token::CODE_BG, c(255, 255, 255, 255)),
            (Token::CODE_TEXT, c(26, 26, 26, 255)),
            (Token::CANVAS_BACKDROP, c(233, 233, 233, 255)),
            (Token::CANVAS_NODE, c(255, 255, 255, 255)),
            (Token::CANVAS_NODE_SELECTED, c(0, 95, 184, 255)),
            (Token::SNAP_GUIDE, c(196, 43, 28, 255)),
            (Token::GRID, c(0, 0, 0, 26)),
            (Token::GRID_DOT, c(0, 0, 0, 26)),
            (Token::HERO_START, c(0, 95, 184, 255)),
            (Token::HERO_END, c(0, 120, 212, 255)),
            (Token::ON_HERO, c(255, 255, 255, 255)),
            (Token::HERO_SCRIM, c(0, 0, 0, 160)),
        ],
    )
}

/// Win11 强调色四元组 `(primary, hover, pressed, soft)` 与前景色。
///
/// WinUI 的强调填充比 Material 更亮更淡（深色下接近 `#60CDFF`），
/// 且在深色主题里配**深色**前景，与 Material 的「亮底白字」习惯相反。
#[must_use]
pub fn accent_ramp(
    brand: crate::AccentBrand,
    dark_mode: bool,
) -> (Color, Color, Color, Color, Color) {
    let (hue, chroma) = brand.oklch_hue_chroma();
    if dark_mode {
        let chroma = chroma.min(0.10);
        (
            Oklch::new(0.80, chroma, hue, 1.0).to_linear(),
            Oklch::new(0.86, chroma, hue, 1.0).to_linear(),
            Oklch::new(0.73, chroma, hue, 1.0).to_linear(),
            Oklch::new(0.30, chroma * 0.45, hue, 1.0).to_linear(),
            Oklch::new(0.16, 0.02, hue, 1.0).to_linear(),
        )
    } else {
        let chroma = chroma.min(0.12);
        (
            Oklch::new(0.52, chroma, hue, 1.0).to_linear(),
            Oklch::new(0.58, chroma, hue, 1.0).to_linear(),
            Oklch::new(0.45, chroma, hue, 1.0).to_linear(),
            Oklch::new(0.94, chroma * 0.18, hue, 1.0).to_linear(),
            Color::WHITE,
        )
    }
}
