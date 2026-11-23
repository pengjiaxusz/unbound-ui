//! 小型、可复用的展厅控件。
//!
//! 它们并非第二套控件框架：每个函数都是基于 [`Canvas`] 的纯渲染器。
//! 标识、悬停、按压与焦点均由调用方传入，与最初按钮确立的契约完全一致。
//!

use accesskit::Role;
use unbound_input::CursorKind;
use unbound_input::HitRegion;
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
use unbound_widgets::select::{emit_select_with_glyphs, SelectDesc, SelectState};
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
    let start = canvas.span_start();
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
    canvas.record_span(node, start);
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
    let start = canvas.span_start();
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
    canvas.widget_output(WidgetCatalogId::ToggleSwitch, node, &output);
    canvas.record_span(node, start);
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
    let start = canvas.span_start();
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
    canvas.widget_output(WidgetCatalogId::CheckBox, node, &output);
    canvas.record_span(node, start);
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
    let start = canvas.span_start();
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
    canvas.widget_output(WidgetCatalogId::RadioButton, node, &output);
    canvas.record_span(node, start);
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
    let start = canvas.span_start();
    let state = canvas.control_state(node, false, false);
    let desc = SliderDesc::new(rect, "Slider", value, min, max);
    let output = emit_slider(node, &desc, state, &[], SliderTokens::standard());
    canvas.widget_output(WidgetCatalogId::Slider, node, &output);
    canvas.record_span(node, start);
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
    canvas.widget_output(WidgetCatalogId::SelectorBar, 0, &frame);
    for index in 0..labels.len() {
        let seg_start = canvas.span_start();
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
        canvas.record_span(ids[index], seg_start);
    }
}

/// 单个分段槽位的扩展配置（支持逐槽禁用、自定义字号与无障碍覆盖）。
pub struct SegmentedSlot<'a> {
    /// 槽位节点键。
    pub node: u64,
    /// 槽位显示标签。
    pub label: &'a str,
    /// 该槽位是否禁用交互。
    pub disabled: bool,
    /// 可选自定义无障碍标签（`None` 时沿用 `label`）。
    pub a11y_label: Option<&'a str>,
    /// 可选自定义无障碍角色（`None` 时沿用分段控件默认 `Role::Tab`）。
    pub a11y_role: Option<Role>,
    /// 可选单槽字号覆盖（`None` 时沿用整组默认字号）。
    pub font_size: Option<f32>,
    /// 可选标签排布矩形与水平对齐方式（`None` 时居中排布于整个槽位）。
    pub label_box: Option<(Rect, AlignX)>,
}

impl<'a> SegmentedSlot<'a> {
    /// 构造标准启用态分段槽位。
    #[must_use]
    pub const fn new(node: u64, label: &'a str) -> Self {
        Self {
            node,
            label,
            disabled: false,
            a11y_label: None,
            a11y_role: None,
            font_size: None,
            label_box: None,
        }
    }
}

/// 渲染支持逐槽禁用、自定义宽列与无障碍覆盖的 `unbound-widgets` 分段控件。
///
/// 统一经 [`emit_segmented_frame`] 与 [`emit_segmented_segment`] 消费
/// [`SegmentedTokens::standard`]（选中背景 `Token::PRIMARY`、选中文字 `Token::ON_PRIMARY`），
/// 并返回每个槽位实际布局矩形供调用方叠加附加徽标或说明文本。
pub fn segmented_slots(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    slots: &[SegmentedSlot<'_>],
    selected: usize,
    font_size: f32,
    item_widths: Option<Vec<f32>>,
) -> Vec<Rect> {
    let labels: Vec<&str> = slots.iter().map(|slot| slot.label).collect();
    let mut desc = SegmentedDesc::new(rect, &labels).with_selected(selected);
    if let Some(widths) = item_widths {
        desc = desc.with_item_widths(widths);
    }
    for (index, slot) in slots.iter().enumerate() {
        if slot.disabled {
            desc = desc.with_disabled(index, true);
        }
    }
    let tokens = SegmentedTokens::standard();
    let frame = emit_segmented_frame(&desc, tokens);
    canvas.widget_output(WidgetCatalogId::SelectorBar, 0, &frame);
    let mut segment_rects = Vec::with_capacity(slots.len());
    for (index, slot) in slots.iter().enumerate() {
        let seg_start = canvas.span_start();
        let seg_rect = desc.segment_rect(index);
        segment_rects.push(seg_rect);
        let state = canvas.control_state(slot.node, slot.disabled, index == selected);
        let (text_rect, align_x) = slot.label_box.unwrap_or((seg_rect, AlignX::Center));
        let (quads, _) = canvas.layout_text(
            slot.label,
            text_rect,
            slot.font_size.unwrap_or(font_size),
            align_x,
            AlignY::Center,
        );
        let mut output = emit_segmented_segment(slot.node, &desc, index, state, &quads, tokens);
        if slot.disabled && output.hit.is_none() {
            // 为禁用槽位保留被动命中包围盒（指针不呈手型且语义标记 disabled），
            // 便于无头回归测试定位并验证点击禁用项不产生状态变化。
            output.hit = Some(
                HitRegion::new(slot.node, 1, u32::from(desc.z), seg_rect)
                    .with_cursor(CursorKind::Default),
            );
        }
        if let Some(a11y) = output.a11y.as_mut() {
            if let Some(custom_label) = slot.a11y_label {
                a11y.label = custom_label.to_owned();
            }
            if let Some(role) = slot.a11y_role {
                a11y.role = role;
            }
        }
        canvas.control_output(slot.node, &output);
        canvas.record_span(slot.node, seg_start);
    }
    segment_rects
}

/// 紧凑型下拉选择触发器（用于标题栏语言选择与窗口缩放大小下拉框）。
///
/// 复用引擎层的 [`emit_select_with_glyphs`]，在所有指针状态下发射恒定数量的图元
/// （背景表面 + 边框 + 两条箭头曲线段 + 字形 + 焦点环），保证悬停仅产生局部切片修补。
pub fn select_trigger(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    display: &str,
    a11y_label: &str,
    open: bool,
) -> Visual {
    let start = canvas.span_start();
    let state = canvas.control_state(node, false, open);
    let mut desc = SelectDesc::new(rect, vec![display.to_string()]);
    desc.label = a11y_label.to_string();
    let mut select_state = SelectState::new();
    select_state.selected = Some(0);
    select_state.open = open;
    let text_rect = Rect::new(
        rect.origin.x + 10.0,
        rect.origin.y,
        (rect.size.x - 28.0).max(0.0),
        rect.size.y,
    );
    let (quads, _) = canvas.layout_text(display, text_rect, 11.5, AlignX::Start, AlignY::Center);
    let output = emit_select_with_glyphs(node, &desc, &select_state, state, &quads);
    canvas.widget_output(WidgetCatalogId::ComboBox, node, &output);
    canvas.record_span(node, start);
    visual_of(state)
}

/// 标题栏下拉菜单的行高（绘制与无障碍语义节点共用，避免两处魔数漂移）。
pub const MENU_ROW_HEIGHT: f32 = 30.0;

/// 标题栏锚定下拉菜单浮层（绘制在 Portal 层级 2，确保覆盖下方页面内容并优先拾取）。
///
/// `panel` 由调用方经 `shell::dropdown_panel_rect` 统一求解，使绘制几何与无障碍语义节点
/// 共用同一份面板矩形，避免两处重复推导而产生漂移。
pub fn dropdown_menu(
    canvas: &mut Canvas<'_>,
    panel: Rect,
    options: &[(&str, Target)],
    selected: usize,
) {
    let palette = *canvas.palette;
    let row_h = MENU_ROW_HEIGHT;
    canvas.rect(
        panel,
        10.0,
        palette.overlay_bg,
        palette.card_border,
        1.0,
        20.0,
    );
    for (idx, &(label, target)) in options.iter().enumerate() {
        let node = crate::target::id(target);
        let start = canvas.span_start();
        let row = Rect::new(
            panel.origin.x + 4.0,
            panel.origin.y + 4.0 + idx as f32 * row_h,
            panel.size.x - 8.0,
            row_h,
        );
        let is_selected = idx == selected;
        let visual = canvas.visual(node, false);
        let bg = if is_selected {
            palette.primary_soft
        } else if visual.is_hovered() {
            palette.control_hover
        } else {
            palette.overlay_bg
        };
        canvas.rect(row, 6.0, bg, 0, 0.0, 0.0);
        canvas.rect_faded(
            Rect::new(
                row.origin.x + 6.0,
                row.origin.y + 8.0,
                3.0,
                row.size.y - 16.0,
            ),
            1.5,
            palette.primary,
            if is_selected { 1.0 } else { 0.0 },
        );
        canvas.text(
            label,
            Rect::new(
                row.origin.x + 16.0,
                row.origin.y,
                row.size.x - 24.0,
                row.size.y,
            ),
            11.5,
            if is_selected {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
        canvas.control_on_layer(
            node,
            row,
            Role::MenuItem,
            label,
            CursorKind::Pointer,
            false,
            is_selected,
            2,
        );
        canvas.record_span(node, start);
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

pub use unbound_widgets::shell::{
    emit_shell_icon_primitives, AppBrandBadgeDesc, FeatureCardDesc, HeroBackdropDesc,
    IconButtonDesc, ShellIcon as Icon, SidebarItemDesc, SidebarSectionHeaderDesc,
};

/// 将图标居中绘制在 `rect` 中。
pub fn icon(canvas: &mut Canvas<'_>, kind: Icon, rect: Rect, color: u16, background: u16) {
    let mut primitives = Vec::new();
    let mut payloads = Vec::new();
    emit_shell_icon_primitives(
        kind,
        rect,
        Token::new(color),
        Token::new(background),
        canvas.z(),
        &mut primitives,
        &mut payloads,
    );
    canvas.control_output(
        0,
        &unbound_widgets::WidgetOutput {
            primitives,
            payloads,
            ..Default::default()
        },
    );
}

/// 应用品牌徽标（基于 [`AppBrandBadgeDesc`] 组件描述符）。
pub fn brand_badge(canvas: &mut Canvas<'_>, rect: Rect, letter: &str) {
    let palette = *canvas.palette;
    let desc = AppBrandBadgeDesc::new(letter).size(rect.size.x).radius(8.0);
    canvas.rect(rect, desc.radius, palette.primary, 0, 0.0, 4.0);
    canvas.text(
        &desc.symbol,
        rect,
        15.0,
        palette.on_primary,
        AlignX::Center,
        AlignY::Center,
    );
}

/// 侧边栏分组大写小标题（基于 [`SidebarSectionHeaderDesc`] 组件描述符）。
pub fn sidebar_section_header(canvas: &mut Canvas<'_>, rect: Rect, label: &str, active: bool) {
    let palette = *canvas.palette;
    let desc = SidebarSectionHeaderDesc::new(label).active(active);
    let upper = desc.title.to_uppercase();
    canvas.text(
        &upper,
        rect,
        9.5,
        if desc.active {
            palette.primary
        } else {
            palette.nav_text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
}

/// 侧边栏胶囊导航项（基于 [`SidebarItemDesc`] 组件描述符，保持恒定图元跨度）。
pub fn sidebar_item(
    canvas: &mut Canvas<'_>,
    node: u64,
    rect: Rect,
    label: &str,
    kind: Icon,
    selected: bool,
) {
    let palette = *canvas.palette;
    let start = canvas.span_start();
    let hover = canvas.visual(node, false);
    let desc = SidebarItemDesc::new(label, kind)
        .selected(selected)
        .with_rect(rect);
    let background = if desc.selected {
        palette.nav_selected
    } else if hover.is_hovered() {
        palette.nav_hover
    } else {
        palette.nav_bg
    };
    canvas.rect(desc.rect, desc.radius, background, 0, 0.0, 0.0);
    canvas.rect_faded(
        desc.indicator_rect(),
        1.5,
        palette.primary,
        if desc.selected { 1.0 } else { 0.0 },
    );
    icon(
        canvas,
        desc.icon,
        desc.icon_rect(),
        if desc.selected {
            palette.primary
        } else {
            palette.nav_text_muted
        },
        palette.nav_bg,
    );
    canvas.text(
        &desc.label,
        desc.label_rect(0.0),
        12.0,
        if desc.selected {
            palette.nav_text
        } else {
            palette.nav_text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
    canvas.control(
        node,
        desc.rect,
        Role::Link,
        &desc.label,
        CursorKind::Pointer,
        false,
        desc.selected,
    );
    canvas.record_span(node, start);
}

/// 展厅特性入口卡片（基于 [`FeatureCardDesc`] 组件描述符）。
#[allow(clippy::too_many_arguments)]
pub fn feature_card(
    canvas: &mut Canvas<'_>,
    node: u64,
    cell: Rect,
    kind: Icon,
    title: &str,
    subtitle: &str,
    a11y_label: &str,
    accent_index: usize,
) {
    let palette = *canvas.palette;
    let desc = FeatureCardDesc::new(title, subtitle, kind).with_rect(cell);
    let hover = canvas.visual(node, false);
    canvas.rect(
        desc.rect,
        desc.radius,
        if hover.is_hovered() {
            palette.control_hover
        } else {
            palette.card_bg
        },
        palette.card_border,
        1.0,
        4.0,
    );
    let icon_box = desc.icon_box_rect();
    canvas.rect(
        icon_box,
        9.0,
        match accent_index {
            0 => palette.primary_soft,
            1 => palette.success_soft,
            2 => palette.warning_soft,
            _ => palette.secondary_soft,
        },
        0,
        0.0,
        0.0,
    );
    icon(
        canvas,
        desc.icon,
        icon_box,
        match accent_index {
            0 => palette.primary,
            1 => palette.success,
            2 => palette.warning,
            _ => palette.secondary,
        },
        palette.card_bg,
    );
    canvas.text(
        &desc.title,
        desc.title_rect(),
        14.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    canvas.text_wrapped(
        &desc.subtitle,
        desc.subtitle_rect(),
        11.5,
        palette.text_muted,
    );
    canvas.text(
        "→",
        desc.arrow_rect(),
        15.0,
        palette.primary,
        AlignX::Center,
        AlignY::Center,
    );
    let _ = a11y_label;
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
    let start = canvas.span_start();
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
    canvas.stroke_faded(
        rect.inflate(1.0),
        9.0,
        palette.focus_ring,
        2.0,
        if canvas.focused(node) { 1.0 } else { 0.0 },
    );
    canvas.control(
        node,
        rect,
        Role::Button,
        label,
        CursorKind::Pointer,
        false,
        selected,
    );
    canvas.record_span(node, start);
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
