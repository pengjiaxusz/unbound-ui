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

/// 把 sRGB 空间里的 alpha 字节换算成线性空间里的等效 alpha 字节。
///
/// 渲染管线的颜色附件是 `B8G8R8A8_SRGB`：GPU 的 alpha 混合发生在**线性**空间，
/// 而 WinUI 的半透明画刷在 **sRGB** 空间混合。同一份「白色 12%」在两种空间里
/// 差得很远——线性混合会把 `#202020` 抬到 `#464646`，比 Win11 的 `#2A2A2A`
/// 亮了三倍。卡片于是像 Material 的实色块，而不是 Win11 的薄纱，
/// 分隔线、悬停态、导航选中态也会统统「发白」。
///
/// 这里按 WinUI 在 sRGB 空间的结果反解线性 alpha，把观感拉回原生。
/// 参照底色取深色基准面 `#202020`；底色更亮时偏差略增，但仍在肉眼难辨的范围内。
fn veil(alpha: u8) -> u8 {
    const BASE: f32 = 32.0;
    let base_linear = unbound_math::color::srgb_to_linear(BASE / 255.0);
    let target = BASE + (255.0 - BASE) * (f32::from(alpha) / 255.0);
    let target_linear = unbound_math::color::srgb_to_linear(target / 255.0);
    let linear = ((target_linear - base_linear) / (1.0 - base_linear)).clamp(0.0, 1.0);
    (linear * 255.0).round() as u8
}

/// 浅色基准面上的黑色薄纱换算（原理见 [`veil`]）。
///
/// 浅色主题的薄纱是「压暗」而不是「提亮」：线性空间里黑色薄纱会显得比
/// WinUI 更淡，所以这里按同一套规则反解 alpha。
fn light_base_veil(alpha: u8) -> u8 {
    const BASE: f32 = 243.0;
    let base_linear = unbound_math::color::srgb_to_linear(BASE / 255.0);
    let target = BASE * (1.0 - f32::from(alpha) / 255.0);
    let target_linear = unbound_math::color::srgb_to_linear(target / 255.0);
    let linear = ((base_linear - target_linear) / base_linear).clamp(0.0, 1.0);
    (linear * 255.0).round() as u8
}

/// Windows 11 深色调色板。
///
/// 基准面 `#202020` 即 `SolidBackgroundFillColorBase` 的深色值；卡片、输入框、
/// 悬停态统一为「白色 N%」的薄纱，叠在云母背板上得到 Win11 的层叠灰阶。
#[must_use]
pub fn dark() -> Palette {
    let mut entries: Vec<(Token, Color)> = vec![
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
        (Token::DIVIDER, c(255, 255, 255, veil(14))),
        (Token::CARD_BORDER, c(255, 255, 255, veil(13))),
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
        (Token::HOVER, c(255, 255, 255, veil(12))),
        (Token::PRESSED, c(255, 255, 255, veil(20))),
        (Token::DISABLED, c(255, 255, 255, veil(15))),
        (Token::ON_DISABLED, c(255, 255, 255, 93)),
        (Token::CONTROL_TRACK, c(107, 107, 107, 255)),
        // 关闭态开关轨道的描边：WinUI 的 `ControlStrongStrokeColorDefault`
        // 深色值 `#8BFFFFFF`（54.5% 白）。实测本机 Win11 关闭态开关的轨道
        // 内部与卡片同色（#2C2932），只有一圈 1px 淡灰描边。
        // 关闭态的轨道是**透明底 + 1px 描边**，不是实心灰块——
        // 实心灰块会让开关看起来像一坨积灰的胶囊。
        (Token::CONTROL_TRACK_STROKE, c(255, 255, 255, veil(139))),
        // 开启态旋钮是纯黑：Win11 的开关靠「亮色轨道 + 黑旋钮」读出开启。
        // 对应 WinUI 的 `TextOnAccentFillColorPrimary` 深色值 `#000000`。
        (Token::CONTROL_THUMB_ON, c(0, 0, 0, 255)),
        // 关闭态旋钮：WinUI 的 `TextFillColorSecondary` 深色值 `#C5FFFFFF`。
        // 实测本机 Win11 关闭态旋钮像素为 `#CECDCF`（(206,205,207)），
        // 与「197/255 白薄纱叠在轨道底 #282530 上」完全吻合。
        (Token::SWITCH_THUMB_OFF, c(255, 255, 255, veil(197))),
        (Token::CONTROL_THUMB, c(255, 255, 255, 255)),
        (Token::CONTROL_HOVER, c(45, 45, 45, 255)),
        (Token::CONTROL_DISABLED_BG, c(255, 255, 255, veil(10))),
        (Token::CONTROL_DISABLED_BORDER, c(255, 255, 255, veil(10))),
        (Token::CONTROL_DISABLED_THUMB, c(255, 255, 255, 93)),
        // ---- 外壳容器（云母之上不再加第二层不透明底） --------------------
        // 标题栏与导航面板不另加底色：Win11 的导航窗格本身就是云母。
        (Token::HEADER_BG, c(32, 32, 32, 0)),
        (Token::NAV_BG, c(32, 32, 32, 0)),
        (Token::NAV_HOVER, c(255, 255, 255, veil(12))),
        (Token::NAV_SELECTED, c(255, 255, 255, veil(16))),
        (Token::NAV_TEXT, c(255, 255, 255, 255)),
        (Token::NAV_TEXT_MUTED, c(197, 197, 197, 255)),
        (Token::CARD_BG, c(255, 255, 255, veil(12))),
        (Token::OVERLAY_BG, c(44, 44, 44, 255)),
        (Token::HUD_BG, c(44, 44, 44, 240)),
        (Token::HUD_TEXT, c(255, 255, 255, 255)),
        // ---- 输入 -------------------------------------------------------
        (Token::INPUT_BG, c(255, 255, 255, veil(12))),
        // 控件表面（下拉框/输入框）：WinUI 的 `ControlFillColorDefault` 是
        // 叠在卡片之上的 6% 白色薄纱，实测比卡片亮 13 级（#37373C vs #2A2A31）。
        // 薄纱的 alpha 字节按 sRGB 空间的百分比给定，而卡片比薄纱的参照底色
        // #202020 更亮，因此这里取 19（≈6% 在更亮底色上的线性等价值）。
        (Token::CONTROL_FILL, c(255, 255, 255, veil(19))),
        // 控件描边（`ControlStrokeColorDefault`）：6% 白色薄纱。
        // 实测 Win11 下拉框左沿只比填充亮 3 级，几乎不可见——
        // 因此不能拿 `OUTLINE`（92,92,92）去描边，那会让下拉框变成一个「黑框」。
        (Token::CONTROL_STROKE, c(255, 255, 255, veil(15))),
        (Token::INPUT_BORDER_HOVER, c(255, 255, 255, veil(51))),
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
        (Token::GRID, c(255, 255, 255, veil(30))),
        (Token::GRID_DOT, c(255, 255, 255, veil(30))),
        (Token::HERO_START, c(96, 205, 255, 255)),
        (Token::HERO_END, c(120, 150, 255, 255)),
        (Token::ON_HERO, c(0, 20, 32, 255)),
        (Token::HERO_SCRIM, c(0, 0, 0, 180)),
    ];
    entries.extend(swatch_ramp(true));
    Palette::new(ThemeKind::Dark, &entries)
}

/// Windows 11 浅色调色板。
///
/// 基准面 `#F3F3F3` 即浅色云母的等效色调；卡片是 70% 白色薄纱（
/// `CardBackgroundFillColorDefault`），描边为 6% 黑色。
#[must_use]
pub fn light() -> Palette {
    let mut entries: Vec<(Token, Color)> = vec![
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
        (Token::DIVIDER, c(0, 0, 0, light_base_veil(18))),
        (Token::CARD_BORDER, c(0, 0, 0, light_base_veil(15))),
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
        (Token::HOVER, c(0, 0, 0, light_base_veil(12))),
        (Token::PRESSED, c(0, 0, 0, light_base_veil(20))),
        (Token::DISABLED, c(0, 0, 0, light_base_veil(12))),
        (Token::ON_DISABLED, c(26, 26, 26, 90)),
        (Token::CONTROL_TRACK, c(138, 138, 138, 255)),
        // 浅色下的关闭态轨道描边是 `ControlStrongStrokeColorDefault` 的
        // 浅色值 `#72000000`（44.7% 黑），靠它把透明轨道从白卡片上「抠」出来。
        (
            Token::CONTROL_TRACK_STROKE,
            c(0, 0, 0, light_base_veil(114)),
        ),
        // 开启态旋钮在浅色下是**白色**：`TextOnAccentFillColorPrimary`
        // 跟着主题翻转（浅 `#FFFFFF` / 深 `#000000`）。
        (Token::CONTROL_THUMB_ON, c(255, 255, 255, 255)),
        // 关闭态旋钮是次级文字色 `TextFillColorSecondary` 浅色值 `#9E000000`。
        // 若沿用深色的白色旋钮，浅色下旋钮会与白色轨道糊成一片，开关直接消失。
        (Token::SWITCH_THUMB_OFF, c(0, 0, 0, light_base_veil(158))),
        (Token::CONTROL_THUMB, c(255, 255, 255, 255)),
        (Token::CONTROL_HOVER, c(245, 245, 245, 255)),
        (Token::CONTROL_DISABLED_BG, c(0, 0, 0, light_base_veil(8))),
        (
            Token::CONTROL_DISABLED_BORDER,
            c(0, 0, 0, light_base_veil(12)),
        ),
        (Token::CONTROL_DISABLED_THUMB, c(0, 0, 0, 60)),
        (Token::HEADER_BG, c(243, 243, 243, 0)),
        (Token::NAV_BG, c(243, 243, 243, 0)),
        (Token::NAV_HOVER, c(0, 0, 0, light_base_veil(8))),
        (Token::NAV_SELECTED, c(0, 0, 0, light_base_veil(14))),
        (Token::NAV_TEXT, c(26, 26, 26, 255)),
        (Token::NAV_TEXT_MUTED, c(92, 92, 92, 255)),
        (Token::CARD_BG, c(255, 255, 255, 178)),
        (Token::OVERLAY_BG, c(249, 249, 249, 255)),
        (Token::HUD_BG, c(249, 249, 249, 246)),
        (Token::HUD_TEXT, c(26, 26, 26, 255)),
        (Token::INPUT_BG, c(255, 255, 255, 178)),
        // 浅色下的控件表面是纯白（WinUI 浅色下拉框为不透明白，靠描边区分）。
        (Token::CONTROL_FILL, c(255, 255, 255, 255)),
        // 浅色下的控件描边是 6% 黑色薄纱：白控件靠这圈灰线从卡片上「抠」出来。
        (Token::CONTROL_STROKE, c(0, 0, 0, light_base_veil(15))),
        (
            Token::INPUT_BORDER_HOVER,
            c(26, 26, 26, light_base_veil(90)),
        ),
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
    ];
    entries.extend(swatch_ramp(false));
    Palette::new(ThemeKind::Light, &entries)
}

/// 10 套品牌色板在给定明暗模式下的主强调色。
///
/// 「颜色网格」控件（Win11 设置 → 个性化 → 颜色）需要逐格上色，
/// 因此每套品牌各占一个调色板槽位，而不是在图元里塞字面颜色。
fn swatch_ramp(dark_mode: bool) -> Vec<(Token, Color)> {
    crate::AccentBrand::ALL
        .iter()
        .enumerate()
        .map(|(idx, brand)| (Token::accent_swatch(idx), accent_ramp(*brand, dark_mode).0))
        .collect()
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
