//! 小型、可复用的展厅控件。
//!
//! 它们并非第二套控件框架：每个函数都是基于 [`Canvas`] 的纯渲染器。
//! 标识、悬停、按压与焦点均由调用方传入，与最初按钮确立的契约完全一致。
//!

use accesskit::Role;
use unbound_input::CursorKind;
use unbound_math::Rect;
use unbound_theme::Token;
use unbound_widgets::button::{
    emit_button_with_tokens, ButtonDesc, ButtonTokens, ButtonVariant as EngineButtonVariant,
};
use unbound_widgets::checkbox::{emit_checkbox, CheckboxDesc, CheckboxTokens};
use unbound_widgets::radio::{emit_radio, RadioDesc, RadioTokens};
use unbound_widgets::segmented::{
    emit_segmented_frame, emit_segmented_segment, SegmentedDesc, SegmentedTokens,
};
use unbound_widgets::slider::{emit_slider, SliderDesc, SliderTokens};
use unbound_widgets::switch::{emit_switch, SwitchDesc, SwitchTokens};
use unbound_widgets::{ControlState, WidgetCatalogId};

use crate::canvas::{quads_bounds, AlignX, AlignY, Canvas, Visual};
use crate::target::Target;

/// 按钮的视觉权重。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Subtle,
    Icon,
    /// 高饱和度首屏表面上的主行动号召按钮。
    ///
    /// 蓝色渐变背景上的实心蓝色按钮几乎没有对比度，因此主操作采用带品牌色标签的
    /// 纯白胶囊，次操作采用带细边框的深色半透明胶囊。
    ///
    ///
    OnHero,
    /// 首屏表面上的次要行动号召按钮。
    OnHeroSecondary,
}

/// 展厅按钮。返回其实际绘制的视觉状态。
///
/// 控件本身位于引擎层：外壳仅提供几何信息、塑形后的标签以及各变体的颜色槽位，
/// 从而使五种状态和焦点环均来自 [`emit_button_with_tokens`]，而非第二套手绘实现。
///
///
pub fn button(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    variant: ButtonVariant,
    enabled: bool,
) -> Visual {
    let state = canvas.control_state(node, !enabled, false);
    let font_size = if variant == ButtonVariant::Icon {
        15.0
    } else {
        13.0
    };
    let (mut quads, _) = canvas.layout_text(label, rect, font_size, AlignX::Center, AlignY::Center);
    let mut desc = ButtonDesc::new(rect, label)
        .with_variant(carrier_variant(variant))
        .with_shadow(0.0)
        .with_centre_label(false)
        .with_focus_ring_gap(2.0);
    desc.font_size = font_size;
    let output = emit_button_with_tokens(node, &desc, state, button_tokens(variant), &mut quads);
    canvas.widget_output(WidgetCatalogId::Button, node, &output);
    if !enabled {
        // 引擎会丢弃禁用按钮的命中区域；展厅则像手绘版本那样，
        // 让禁用按钮继续吸收点击。
        //
        canvas.control(
            node,
            rect,
            Role::Button,
            label,
            CursorKind::Pointer,
            true,
            false,
        );
    }
    visual_of(state)
}

/// 用作载体的引擎变体。
///
/// 颜色槽位是显式传入的，因此载体从不决定调色板；它仅用于保持描述符可读。
///
fn carrier_variant(variant: ButtonVariant) -> EngineButtonVariant {
    match variant {
        ButtonVariant::Primary => EngineButtonVariant::Primary,
        ButtonVariant::Secondary | ButtonVariant::OnHeroSecondary => EngineButtonVariant::Secondary,
        ButtonVariant::Subtle => EngineButtonVariant::Subtle,
        ButtonVariant::Icon => EngineButtonVariant::Icon,
        ButtonVariant::OnHero => EngineButtonVariant::OnHero,
    }
}

/// 各展厅变体绘制时使用的颜色槽位。
///
/// 这些槽位与手绘按钮完全一致，从而保证迁移后的页面像素级一致：
/// 引擎自身的默认值在三处有所不同（Primary 的边框、所有的 `label_disabled`、
/// 以及 Secondary 的禁用填充），此处予以覆盖。
///
fn button_tokens(variant: ButtonVariant) -> ButtonTokens {
    let focus_ring = Token::FOCUS_RING;
    match variant {
        ButtonVariant::Primary => ButtonTokens {
            fill: Token::PRIMARY,
            fill_hover: Token::PRIMARY_HOVER,
            fill_pressed: Token::PRIMARY_PRESSED,
            fill_disabled: Token::DISABLED,
            border: Token::NONE,
            label: Token::ON_PRIMARY,
            label_disabled: Token::TEXT_SECONDARY,
            focus_ring,
        },
        ButtonVariant::Secondary => ButtonTokens {
            fill: Token::CARD_BG,
            fill_hover: Token::CONTROL_HOVER,
            fill_pressed: Token::CONTROL_HOVER,
            fill_disabled: Token::CARD_BG,
            border: Token::CARD_BORDER,
            label: Token::TEXT_PRIMARY,
            label_disabled: Token::TEXT_SECONDARY,
            focus_ring,
        },
        ButtonVariant::Subtle | ButtonVariant::Icon => ButtonTokens {
            fill: Token::NONE,
            fill_hover: Token::CONTROL_HOVER,
            fill_pressed: Token::NAV_SELECTED,
            fill_disabled: Token::NONE,
            border: Token::NONE,
            label: Token::PRIMARY,
            label_disabled: Token::TEXT_SECONDARY,
            focus_ring,
        },
        ButtonVariant::OnHero => ButtonTokens {
            fill: Token::ON_HERO,
            fill_hover: Token::ON_HERO,
            fill_pressed: Token::PRIMARY_SOFT,
            fill_disabled: Token::ON_HERO,
            border: Token::NONE,
            label: Token::PRIMARY,
            label_disabled: Token::PRIMARY,
            focus_ring,
        },
        ButtonVariant::OnHeroSecondary => ButtonTokens {
            fill: Token::HERO_SCRIM,
            fill_hover: Token::HERO_SCRIM,
            fill_pressed: Token::HERO_SCRIM,
            fill_disabled: Token::HERO_SCRIM,
            border: Token::ON_HERO,
            label: Token::ON_HERO,
            label_disabled: Token::ON_HERO,
            focus_ring,
        },
    }
}

/// 带标签和开关的行。
///
/// 控件本身位于引擎层（[`unbound_widgets::switch`]）：
/// 外壳仅提供行几何信息与塑形后的标签。
///
pub fn toggle_switch(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    on: bool,
    disabled: bool,
) -> Visual {
    let state = canvas.control_state(node, disabled, on);
    let desc = SwitchDesc::new(rect, label).with_on(on);
    let (quads, _) = canvas.layout_text(
        label,
        Rect::new(
            rect.origin.x,
            rect.origin.y,
            (rect.size.x - 56.0).max(0.0),
            rect.size.y,
        ),
        13.0,
        AlignX::Start,
        AlignY::Center,
    );
    let output = emit_switch(node, &desc, state, &quads, SwitchTokens::standard());
    canvas.widget_output(WidgetCatalogId::Switch, node, &output);
    visual_of(state)
}

/// 将控件状态转换为外壳自身的视觉词汇。
fn visual_of(state: ControlState) -> Visual {
    if state.disabled {
        return Visual::Disabled;
    }
    match state.phase {
        unbound_widgets::PointerPhase::Idle => Visual::Idle,
        unbound_widgets::PointerPhase::Hovered => Visual::Hover,
        unbound_widgets::PointerPhase::Pressed => Visual::Pressed,
    }
}

/// 复选框行。
pub fn checkbox(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    checked: bool,
    disabled: bool,
) -> Visual {
    let state = canvas.control_state(node, disabled, checked);
    let desc = CheckboxDesc::new(rect, label).with_checked(checked);
    let (mark, _) = canvas.layout_text("✓", desc.box_rect(), 12.0, AlignX::Center, AlignY::Center);
    let (quads, _) = canvas.layout_text(
        label,
        desc.label_rect(),
        13.0,
        AlignX::Start,
        AlignY::Center,
    );
    let output = emit_checkbox(
        node,
        &desc,
        state,
        &mark,
        &quads,
        CheckboxTokens::standard(),
    );
    canvas.widget_output(WidgetCatalogId::Checkbox, node, &output);
    visual_of(state)
}

/// 单选按钮行。
pub fn radio(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    selected: bool,
    disabled: bool,
) -> Visual {
    let state = canvas.control_state(node, disabled, selected);
    let desc = RadioDesc::new(rect, label).with_selected(selected);
    let (quads, _) = canvas.layout_text(
        label,
        desc.label_rect(),
        13.0,
        AlignX::Start,
        AlignY::Center,
    );
    let output = emit_radio(node, &desc, state, &quads, RadioTokens::standard());
    canvas.widget_output(WidgetCatalogId::Radio, node, &output);
    visual_of(state)
}

/// 水平滑块。`value` 位于 `[min, max]` 区间内。
pub fn slider(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    value: f32,
    min: f32,
    max: f32,
) -> Visual {
    let state = canvas.control_state(node, false, false);
    let desc = SliderDesc::new(rect, "Slider", value, min, max);
    let output = emit_slider(node, &desc, state, &[], SliderTokens::standard());
    canvas.widget_output(WidgetCatalogId::Slider, node, &output);
    visual_of(state)
}

/// 分段胶囊控件。
///
/// 各槽位构成均匀网格，高亮区域四边各内缩两像素，使标签绝不触碰边框：
/// 43 px 的槽位可容纳 `"100%"`，并在两侧保留七像素的呼吸空间。
///
pub fn segmented(
    canvas: &mut Canvas<'_>,
    ids: &[u64],
    rect: Rect,
    labels: &[&str],
    selected: usize,
    disabled: bool,
    thumb: Option<f32>,
) {
    let mut desc = SegmentedDesc::new(rect, labels).with_selected(selected);
    if let Some(thumb) = thumb {
        desc = desc.with_thumb(thumb);
    }
    let tokens = SegmentedTokens::standard();
    let frame = emit_segmented_frame(&desc, tokens);
    canvas.widget_output(WidgetCatalogId::Segmented, 0, &frame);
    for index in 0..labels.len() {
        let state = canvas.control_state(ids[index], disabled, index == selected);
        let (quads, _) = canvas.layout_text(
            labels[index],
            desc.segment_rect(index),
            12.0,
            AlignX::Center,
            AlignY::Center,
        );
        let output = emit_segmented_segment(ids[index], &desc, index, state, &quads, tokens);
        canvas.control_output(ids[index], &output);
    }
}

/// 导航行：选中胶囊、标签以及可选的尾部提示。
pub fn nav_row(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    selected: bool,
    indent: f32,
) {
    let start = canvas.span_start();
    let palette = *canvas.palette;
    let visual = canvas.visual(node, false);
    // 无论处于何种状态，该行始终绘制两个图元——背景与强调条。
    // 空闲时背景槽位采用导航窗格自身的颜色，强调条淡出，
    // 因此两种状态都不会改变该行所拥有的图元数量。
    //
    let background = if selected {
        palette.nav_selected
    } else if visual.is_hovered() {
        palette.nav_hover
    } else {
        palette.nav_bg
    };
    canvas.rect(rect, 8.0, background, 0, 0.0, 0.0);
    canvas.rect_faded(
        Rect::new(
            rect.origin.x - 10.0,
            rect.origin.y + 8.0,
            3.0,
            rect.size.y - 16.0,
        ),
        2.0,
        palette.primary,
        if selected { 1.0 } else { 0.0 },
    );
    canvas.text(
        label,
        Rect::new(
            rect.origin.x + indent,
            rect.origin.y,
            rect.size.x - indent,
            rect.size.y,
        ),
        13.0,
        if selected {
            palette.nav_text
        } else {
            palette.nav_text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
    canvas.control(
        node,
        rect,
        Role::Link,
        label,
        CursorKind::Pointer,
        false,
        selected,
    );
    canvas.record_span(node, start);
}

/// 小号胶囊徽章。
pub fn badge(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    label: &str,
    foreground: u16,
    background: u16,
) -> Rect {
    badge_sized(canvas, rect, label, foreground, background, 11.0)
}

/// 指定字号的胶囊徽章。
///
/// 宽度始终依据文本绘制时所使用的同一字号计算，
/// 以便调用方按返回的矩形推进游标，避免胶囊之间发生重叠。
///
pub fn badge_sized(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    label: &str,
    foreground: u16,
    background: u16,
    size: f32,
) -> Rect {
    let measured = canvas.labels.measure(label, size);
    let width = (measured.x + 16.0).max(28.0);
    let rect = Rect::new(rect.origin.x, rect.origin.y, width, rect.size.y);
    canvas.rect(rect, rect.size.y * 0.5, background, 0, 0.0, 0.0);
    canvas.text(
        label,
        rect,
        size,
        foreground,
        AlignX::Center,
        AlignY::Center,
    );
    rect
}

/// 外壳以几何方式绘制的图标词汇表。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    Menu,
    Search,
    Sun,
    Moon,
    Snap,
    Chart,
    Grid,
    Layers,
    ChevronDown,
    ChevronRight,
    Check,
    Close,
    Plus,
    Play,
    Undo,
    Redo,
}

/// 将图标居中绘制在 `rect` 中。
pub fn icon(canvas: &mut Canvas<'_>, kind: Icon, rect: Rect, color: u16, background: u16) {
    let c = rect.center();
    match kind {
        Icon::Menu => {
            for i in 0..3 {
                canvas.rect(
                    Rect::new(c.x - 8.0, c.y - 6.0 + i as f32 * 5.0, 16.0, 2.0),
                    1.0,
                    color,
                    0,
                    0.0,
                    0.0,
                );
            }
        }
        Icon::Search => {
            canvas.stroke(Rect::new(c.x - 9.0, c.y - 9.0, 13.0, 13.0), 6.5, color, 2.0);
            canvas.rect(
                Rect::new(c.x + 3.0, c.y + 3.0, 6.0, 2.0),
                1.0,
                color,
                0,
                0.0,
                0.0,
            );
        }
        Icon::Sun => {
            canvas.rect(
                Rect::new(c.x - 6.0, c.y - 6.0, 12.0, 12.0),
                6.0,
                color,
                0,
                0.0,
                0.0,
            );
            for (dx, dy, w, h) in [
                (0.0, -10.0, 2.0, 4.0),
                (0.0, 6.0, 2.0, 4.0),
                (-10.0, 0.0, 4.0, 2.0),
                (6.0, 0.0, 4.0, 2.0),
            ] {
                canvas.rect(Rect::new(c.x + dx, c.y + dy, w, h), 1.0, color, 0, 0.0, 0.0);
            }
        }
        Icon::Moon => {
            canvas.rect(
                Rect::new(c.x - 8.0, c.y - 8.0, 16.0, 16.0),
                8.0,
                color,
                0,
                0.0,
                0.0,
            );
            canvas.rect(
                Rect::new(c.x - 2.0, c.y - 9.0, 11.0, 14.0),
                7.0,
                background,
                0,
                0.0,
                0.0,
            );
        }
        Icon::Snap => {
            for row in 0..3 {
                for col in 0..3 {
                    canvas.rect(
                        Rect::new(
                            c.x - 8.0 + col as f32 * 6.0,
                            c.y - 8.0 + row as f32 * 6.0,
                            3.0,
                            3.0,
                        ),
                        1.0,
                        color,
                        0,
                        0.0,
                        0.0,
                    );
                }
            }
        }
        Icon::Chart => {
            for (i, height) in [6.0f32, 12.0, 9.0].iter().enumerate() {
                canvas.rect(
                    Rect::new(c.x - 8.0 + i as f32 * 6.0, c.y + 6.0 - height, 4.0, *height),
                    1.5,
                    color,
                    0,
                    0.0,
                    0.0,
                );
            }
        }
        Icon::Grid => {
            for row in 0..2 {
                for col in 0..2 {
                    canvas.rect(
                        Rect::new(
                            c.x - 8.0 + col as f32 * 9.0,
                            c.y - 8.0 + row as f32 * 9.0,
                            7.0,
                            7.0,
                        ),
                        2.0,
                        color,
                        0,
                        0.0,
                        0.0,
                    );
                }
            }
        }
        Icon::Layers => {
            for i in 0..3 {
                canvas.rect(
                    Rect::new(
                        c.x - 8.0 + i as f32 * 2.0,
                        c.y - 6.0 + i as f32 * 5.0,
                        12.0,
                        4.0,
                    ),
                    2.0,
                    color,
                    0,
                    0.0,
                    0.0,
                );
            }
        }
        Icon::ChevronDown => {
            canvas.text("⌄", rect, 16.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::ChevronRight => {
            canvas.text("›", rect, 16.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Check => {
            canvas.text("✓", rect, 14.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Close => {
            canvas.text("×", rect, 16.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Plus => {
            canvas.text("+", rect, 16.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Play => {
            canvas.text("▶", rect, 12.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Undo => {
            canvas.text("↶", rect, 17.0, color, AlignX::Center, AlignY::Center);
        }
        Icon::Redo => {
            canvas.text("↷", rect, 17.0, color, AlignX::Center, AlignY::Center);
        }
    }
}

/// 可点击的图标按钮。
pub fn icon_button(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    kind: Icon,
    label: &str,
    selected: bool,
) {
    let palette = *canvas.palette;
    let visual = canvas.visual(node, false);
    let fill = match visual {
        Visual::Hover => palette.control_hover,
        Visual::Pressed => palette.nav_selected,
        _ => {
            if selected {
                palette.nav_selected
            } else {
                0
            }
        }
    };
    canvas.rect(rect, 8.0, fill, 0, 0.0, 0.0);
    icon(canvas, kind, rect, palette.text, palette.card_bg);
    if canvas.focused(node) {
        canvas.stroke(rect.inflate(1.0), 9.0, palette.focus_ring, 2.0);
    }
    canvas.control(
        node,
        rect,
        Role::Button,
        label,
        CursorKind::Pointer,
        false,
        selected,
    );
}

/// 字形序列的图元计数，供遥测页面使用。
#[must_use]
pub fn glyph_count(quads: &[unbound_text::GlyphQuad]) -> usize {
    quads_bounds(quads).map_or(0, |_| quads.len())
}

/// 演示部件标识辅助函数。
#[must_use]
pub fn demo_id(page: u8, card: u8, part: u16) -> u64 {
    crate::target::id(Target::Demo(page, card, part))
}
