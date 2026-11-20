//! 带快捷预设档位的数字输入框（`PresetNumberInput`）。
//!
//! 直接复用 [`crate::number_input::NumberInputDesc`] 与 [`crate::number_input::NumberInputState`]
//! 的步进、小数精度、单位后缀与范围钳制逻辑，并在其上集成常用预设档位（支持通过
//! [`crate::overlay::OverlayStack`] 弹出下拉菜单或内联胶囊高亮当前命中档位）。

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yActions, A11yNode};
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::number_input::{NumberInputDesc, NumberInputState, NUMBER_RADIUS};
use crate::overlay::{place_anchored_popup_detailed, OverlayEntry, OverlayStack, Placement};
use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 单个数值预设档位。
#[derive(Clone, Debug, PartialEq)]
pub struct NumberPreset {
    /// 预设显示标签（例如 `"60 FPS"`、`"1024 px"`）。
    pub label: String,
    /// 对应的数值。
    pub value: f64,
    /// 可选补充说明（例如 `"Standard refresh"`）。
    pub description: Option<String>,
}

impl NumberPreset {
    /// 创建数值预设档位。
    #[must_use]
    pub fn new(label: impl Into<String>, value: f64) -> Self {
        Self {
            label: label.into(),
            value,
            description: None,
        }
    }

    /// 附加说明文案。
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// 预设档位的呈现模式。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PresetDisplayMode {
    /// 通过 `OverlayStack` 弹出下拉档位菜单（默认）。
    #[default]
    PopupMenu,
    /// 在输入框右侧/下方渲染内联预设胶囊并高亮命中项。
    InlinePills,
}

/// 预设下拉触发按钮的确定性子节点 ID。
#[must_use]
pub const fn preset_dropdown_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA401
}

/// 预设档位项的确定性子节点 ID。
#[must_use]
pub const fn preset_item_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA480 ^ (index as u64)
}

/// 带预设档位的数字输入框状态机（组合复用 [`NumberInputState`]）。
#[derive(Clone, Debug)]
pub struct PresetNumberInputState {
    /// 复用的底层数字输入状态机。
    pub number: NumberInputState,
    /// 弹出菜单是否打开。
    pub popup_open: bool,
    /// 键盘导航高亮的预设索引。
    pub highlighted_preset: Option<usize>,
}

impl PresetNumberInputState {
    /// 创建初始值为 `value` 的预设数字输入状态。
    #[must_use]
    pub fn new(value: f64) -> Self {
        Self {
            number: NumberInputState::new(value),
            popup_open: false,
            highlighted_preset: None,
        }
    }

    /// 当前数值。
    #[must_use]
    pub fn value(&self) -> f64 {
        self.number.value()
    }

    /// 查找与当前数值匹配的预设档位索引（按容差 `1e-6` 比较）。
    #[must_use]
    pub fn matched_preset_index(&self, presets: &[NumberPreset]) -> Option<usize> {
        let current = self.number.value();
        presets
            .iter()
            .position(|preset| (preset.value - current).abs() <= 1e-6)
    }

    /// 选中指定索引的预设档位（通过 `NumberInputState::set_value` 执行步进/范围钳制）并关闭弹出层。
    pub fn select_preset(&mut self, index: usize, desc: &PresetNumberInputDesc) -> bool {
        let Some(preset) = desc.presets.get(index) else {
            return false;
        };
        let changed = self.number.set_value(preset.value, &desc.number);
        self.popup_open = false;
        self.highlighted_preset = Some(index);
        changed
    }

    /// 打开预设弹出层并初始化高亮索引。
    pub fn open_popup(&mut self, presets: &[NumberPreset]) {
        if presets.is_empty() {
            return;
        }
        self.popup_open = true;
        self.highlighted_preset = self.matched_preset_index(presets).or(Some(0));
    }

    /// 关闭预设弹出层。
    pub fn close_popup(&mut self) {
        self.popup_open = false;
    }

    /// 切换预设弹出层开关。
    pub fn toggle_popup(&mut self, presets: &[NumberPreset]) {
        if self.popup_open {
            self.close_popup();
        } else {
            self.open_popup(presets);
        }
    }

    /// 响应键盘按键：
    /// - 菜单打开时：`ArrowDown`/`ArrowUp` 循环高亮预设，`Enter` 选中高亮预设，`Escape` 关闭菜单；
    /// - 菜单关闭时：`ArrowUp`/`ArrowDown` 复用 `NumberInputState::step_by` 微调数值，`Alt+ArrowDown` 打开菜单。
    pub fn handle_named_key(
        &mut self,
        key: &str,
        alt: bool,
        desc: &PresetNumberInputDesc,
    ) -> bool {
        if self.popup_open {
            let len = desc.presets.len();
            if len == 0 {
                self.popup_open = false;
                return false;
            }
            match key {
                "ArrowDown" => {
                    let next = self.highlighted_preset.map_or(0, |i| (i + 1) % len);
                    self.highlighted_preset = Some(next);
                    true
                }
                "ArrowUp" => {
                    let prev = self
                        .highlighted_preset
                        .map_or(len - 1, |i| if i == 0 { len - 1 } else { i - 1 });
                    self.highlighted_preset = Some(prev);
                    true
                }
                "Enter" => {
                    if let Some(idx) = self.highlighted_preset {
                        self.select_preset(idx, desc);
                        true
                    } else {
                        false
                    }
                }
                "Escape" => {
                    self.close_popup();
                    true
                }
                _ => false,
            }
        } else {
            match key {
                "ArrowDown" if alt => {
                    self.open_popup(&desc.presets);
                    self.popup_open
                }
                "ArrowUp" => self.number.step_by(1, &desc.number),
                "ArrowDown" => self.number.step_by(-1, &desc.number),
                _ => false,
            }
        }
    }

    /// 将预设下拉菜单挂载到 [`OverlayStack`]（关闭或非菜单模式时自动卸载）。
    pub fn sync_overlay(
        &self,
        stack: &mut OverlayStack,
        popup_node: u64,
        anchor_node: u64,
        desc: &PresetNumberInputDesc,
        viewport: Rect,
    ) -> Option<Rect> {
        if !self.popup_open || desc.mode != PresetDisplayMode::PopupMenu || desc.presets.is_empty()
        {
            stack.close(popup_node);
            return None;
        }
        let popup_rect = desc.popup_rect(viewport);
        stack.push(OverlayEntry::menu(popup_node, anchor_node, popup_rect));
        Some(popup_rect)
    }
}

/// 带快捷预设档位的数字输入框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PresetNumberInputDesc {
    /// 复用的底层数字输入框描述符（持有范围、步长、精度、单位后缀等约束）。
    pub number: NumberInputDesc,
    /// 可选预设档位列表。
    pub presets: Vec<NumberPreset>,
    /// 预设呈现模式（弹出菜单或内联胶囊）。
    pub mode: PresetDisplayMode,
    /// 弹出菜单是否展开。
    pub popup_open: bool,
    /// 当前命中或高亮的预设索引。
    pub active_preset: Option<usize>,
    /// 当前格式化值文本（供无障碍播报与展示）。
    pub value_text: String,
    /// 无障碍标签。
    pub label: String,
    /// 绘制 z-index。
    pub z: u16,
}

impl PresetNumberInputDesc {
    /// 创建带范围约束与预设档位的描述符。
    #[must_use]
    pub fn new(rect: Rect, min: f64, max: f64, presets: Vec<NumberPreset>) -> Self {
        Self {
            number: NumberInputDesc::new(rect, min, max),
            presets,
            mode: PresetDisplayMode::PopupMenu,
            popup_open: false,
            active_preset: None,
            value_text: String::new(),
            label: "Preset Number".to_owned(),
            z: 20,
        }
    }

    /// 设置当前格式化值文本。
    #[must_use]
    pub fn with_value_text(mut self, value_text: impl Into<String>) -> Self {
        self.value_text = value_text.into();
        self
    }

    /// 设置呈现模式（弹出菜单或内联胶囊）。
    #[must_use]
    pub const fn with_mode(mut self, mode: PresetDisplayMode) -> Self {
        self.mode = mode;
        self
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 同步底层 [`NumberInputDesc`] 配置（步长、小数精度、单位后缀等）。
    #[must_use]
    pub fn with_number_desc(mut self, number: NumberInputDesc) -> Self {
        self.number = number;
        self
    }

    /// 设置弹出层展开与高亮状态。
    #[must_use]
    pub const fn with_popup_state(mut self, open: bool, active: Option<usize>) -> Self {
        self.popup_open = open;
        self.active_preset = active;
        self
    }

    /// 右侧下拉触发按钮矩形。
    #[must_use]
    pub fn dropdown_button_rect(&self) -> Rect {
        let r = self.number.rect;
        let w = 28.0_f32.min(r.width() * 0.25).max(20.0);
        Rect::new(r.max().x - w - 2.0, r.origin.y + 2.0, w, (r.height() - 4.0).max(16.0))
    }

    /// 计算预设下拉菜单在视口内的停靠矩形。
    #[must_use]
    pub fn popup_rect(&self, viewport: Rect) -> Rect {
        let rows = self.presets.len().clamp(1, 8) as f32;
        let size = Vec2::new(self.number.rect.width().max(140.0), rows * 30.0 + 8.0);
        place_anchored_popup_detailed(
            self.number.rect,
            size,
            viewport,
            Placement::Bottom,
        )
        .rect
    }
}

/// 发射带预设档位的数字输入框（24 态恒定图元数：主输入框 + 底部强调线 + 下拉触发器 + 2 条箭头线段 + 焦点环 = 6 个固定图元，再加内联胶囊）。
#[must_use]
pub fn emit_preset_number_input(
    node: u64,
    desc: &PresetNumberInputDesc,
    state: ControlState,
    text_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let r = desc.number.rect;
    let z = desc.z;
    let pill_count = if desc.mode == PresetDisplayMode::InlinePills {
        desc.presets.len()
    } else {
        0
    };
    let mut primitives = Vec::with_capacity(6 + pill_count + text_glyphs.len());
    let mut payloads = Vec::with_capacity(2 + text_glyphs.len());

    let (fill, border) = if state.disabled {
        (Token::CONTROL_DISABLED_BG, Token::CONTROL_DISABLED_BORDER)
    } else if state.phase == PointerPhase::Hovered {
        (Token::INPUT_BG, Token::INPUT_BORDER_HOVER)
    } else {
        (Token::INPUT_BG, Token::CONTROL_STROKE)
    };
    primitives.push(outlined(r, NUMBER_RADIUS, fill, border, 1.0, 0.0, z));

    // 聚焦底部强调线。
    let mut accent = surface(
        Rect::new(r.origin.x + 1.0, r.max().y - 2.0, (r.width() - 2.0).max(0.0), 2.0),
        0.0,
        Token::PRIMARY,
        z + 1,
    );
    accent.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z + 1,
        if !state.disabled && (state.focused || desc.popup_open) {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(accent);

    // 下拉触发按钮。
    let btn_r = desc.dropdown_button_rect();
    let btn_fill = if state.disabled {
        Token::CONTROL_DISABLED_BG
    } else if desc.popup_open || state.phase == PointerPhase::Pressed {
        Token::PRESSED
    } else if state.phase == PointerPhase::Hovered {
        Token::HOVER
    } else {
        Token::CONTROL_FILL
    };
    primitives.push(outlined(btn_r, 4.0, btn_fill, Token::CONTROL_STROKE, 1.0, 0.0, z + 1));

    let chevron_tok = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    let cx = btn_r.origin.x + btn_r.width() * 0.5;
    let cy = btn_r.origin.y + btn_r.height() * 0.5;
    curve_stroke([cx - 3.5, cy - 1.5], [cx, cy + 2.0], 1.5, chevron_tok, z + 2, &mut primitives, &mut payloads);
    curve_stroke([cx, cy + 2.0], [cx + 3.5, cy - 1.5], 1.5, chevron_tok, z + 2, &mut primitives, &mut payloads);

    // 内联胶囊预设行（仅 InlinePills 模式，数量由 desc.presets 决定，与 24 态无关）。
    let mut inline_pill_rects: Vec<Rect> = Vec::new();
    if desc.mode == PresetDisplayMode::InlinePills && !desc.presets.is_empty() {
        let mut px = r.origin.x;
        let py = r.max().y + 6.0;
        for (idx, preset) in desc.presets.iter().enumerate() {
            let pw = (preset.label.len() as f32 * 7.0 + 16.0).max(40.0);
            let pill_r = Rect::new(px, py, pw, 24.0);
            inline_pill_rects.push(pill_r);
            let is_active = desc.active_preset == Some(idx);
            let (p_fill, p_border) = if is_active {
                (Token::PRIMARY_SOFT, Token::PRIMARY)
            } else {
                (Token::CONTROL_FILL, Token::CONTROL_STROKE)
            };
            primitives.push(outlined(pill_r, 4.0, p_fill, p_border, 1.0, 0.0, z + 1));
            px += pw + 6.0;
        }
    }

    primitives.push(focus_ring(
        r.inflate(3.0),
        NUMBER_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        state.focused && !state.disabled,
    ));

    if !text_glyphs.is_empty() {
        let text_tok = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };
        emit_glyphs(text_glyphs, text_tok, z + 4, &mut primitives, &mut payloads);
    }

    let mut actions = A11yActions::NONE;
    actions.insert(A11yAction::Focus);
    actions.insert(A11yAction::SetValue);
    actions.insert(A11yAction::Increment);
    actions.insert(A11yAction::Decrement);
    let a11y = A11yNode::new(Role::ComboBox, desc.label.clone(), r)
        .with_value(desc.value_text.clone())
        .with_actions(actions)
        .expanded(desc.popup_open)
        .disabled(state.disabled);

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), r)
            .with_shape(Shape::Rounded {
                radii: [NUMBER_RADIUS; 4],
            })
            .with_cursor(CursorKind::Text)
    });

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        let drop_id = preset_dropdown_node(node);
        out.with_sub_control(
            drop_id,
            HitRegion::new(drop_id, CONTROL_LAYER, u32::from(z) + 1, btn_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Toggle Presets", btn_r)
                .expanded(desc.popup_open)
                .clickable(),
            false,
        );
        for (idx, pill_r) in inline_pill_rects.into_iter().enumerate() {
            if let Some(preset) = desc.presets.get(idx) {
                let item_id = preset_item_node(node, idx);
                let mut item_a11y = A11yNode::new(Role::Button, preset.label.clone(), pill_r)
                    .selected(desc.active_preset == Some(idx))
                    .clickable();
                if let Some(note) = &preset.description {
                    item_a11y = item_a11y.with_description(note.clone());
                }
                out.with_sub_control(
                    item_id,
                    HitRegion::new(item_id, CONTROL_LAYER, u32::from(z) + 1, pill_r)
                        .with_cursor(CursorKind::Pointer),
                    item_a11y,
                    true,
                );
            }
        }
    }

    out
}

/// 发射挂载在 [`OverlayStack`] 中的预设档位下拉菜单。
#[must_use]
pub fn emit_preset_number_popup(
    node: u64,
    desc: &PresetNumberInputDesc,
    viewport: Rect,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let popup_r = desc.popup_rect(viewport);
    let z = desc.z.saturating_add(800);
    let mut primitives = Vec::with_capacity(1 + desc.presets.len() + label_glyphs.len());
    let payloads = Vec::with_capacity(label_glyphs.len());

    primitives.push(outlined(
        popup_r,
        8.0,
        Token::OVERLAY_BG,
        Token::CARD_BORDER,
        1.0,
        12.0,
        z,
    ));

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!state.disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), popup_r)
                .with_cursor(CursorKind::Default)
        }),
        a11y: Some(A11yNode::new(Role::ListBox, "Preset Options", popup_r).disabled(state.disabled)),
        focusable: false,
        ..Default::default()
    };

    for (idx, preset) in desc.presets.iter().enumerate() {
        let row_r = Rect::new(
            popup_r.origin.x + 4.0,
            popup_r.origin.y + 4.0 + idx as f32 * 30.0,
            (popup_r.width() - 8.0).max(24.0),
            28.0,
        );
        let is_active = desc.active_preset == Some(idx);
        let fill = if is_active {
            Token::PRIMARY_SOFT
        } else {
            Token::NONE
        };
        out.primitives.push(surface(row_r, 4.0, fill, z + 1));
        if !state.disabled {
            let item_id = preset_item_node(node, idx);
            let mut item_a11y = A11yNode::new(Role::ListBoxOption, preset.label.clone(), row_r)
                .selected(is_active)
                .clickable();
            if let Some(note) = &preset.description {
                item_a11y = item_a11y.with_description(note.clone());
            }
            out.with_sub_control(
                item_id,
                HitRegion::new(item_id, CONTROL_LAYER, u32::from(z) + 1, row_r)
                    .with_cursor(CursorKind::Pointer),
                item_a11y,
                true,
            );
        }
    }

    if !label_glyphs.is_empty() {
        emit_glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    fn sample_presets() -> Vec<NumberPreset> {
        vec![
            NumberPreset::new("30 FPS", 30.0).with_description("Power saving"),
            NumberPreset::new("60 FPS", 60.0).with_description("Standard"),
            NumberPreset::new("120 FPS", 120.0).with_description("High refresh"),
        ]
    }

    #[test]
    fn preset_number_input_selection_keyboard_and_overlay() {
        let rect = Rect::new(20.0, 20.0, 200.0, 32.0);
        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let desc = PresetNumberInputDesc::new(rect, 10.0, 240.0, sample_presets());
        let mut st = PresetNumberInputState::new(60.0);
        assert_eq!(st.matched_preset_index(&desc.presets), Some(1));

        // Alt+ArrowDown 打开菜单并同步到 OverlayStack。
        assert!(st.handle_named_key("ArrowDown", true, &desc));
        assert!(st.popup_open);
        assert_eq!(st.highlighted_preset, Some(1));

        let mut stack = OverlayStack::new();
        assert!(st.sync_overlay(&mut stack, 500, 10, &desc, viewport).is_some());
        assert!(stack.contains(500));

        // ArrowDown -> 120 FPS -> Enter 选中并关闭弹出层。
        assert!(st.handle_named_key("ArrowDown", false, &desc));
        assert_eq!(st.highlighted_preset, Some(2));
        assert!(st.handle_named_key("Enter", false, &desc));
        assert!(!st.popup_open);
        assert!((st.value() - 120.0).abs() < 1e-6);

        assert!(st.sync_overlay(&mut stack, 500, 10, &desc, viewport).is_none());
        assert!(!stack.contains(500));
    }

    #[test]
    fn preset_number_input_a11y_stable_shape_and_headless() {
        let rect = Rect::new(20.0, 20.0, 220.0, 32.0);
        let desc = PresetNumberInputDesc::new(rect, 0.0, 1000.0, sample_presets())
            .with_mode(PresetDisplayMode::InlinePills)
            .with_popup_state(false, Some(1));
        let out = emit_preset_number_input(1, &desc, ControlState::default(), &[]);
        assert_eq!(out.a11y.as_ref().unwrap().role, Role::ComboBox);

        let base = out.primitives.len();
        for phase in [PointerPhase::Idle, PointerPhase::Hovered, PointerPhase::Pressed] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let st = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        assert_eq!(
                            emit_preset_number_input(1, &desc, st, &[]).primitives.len(),
                            base
                        );
                    }
                }
            }
        }

        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "PresetNumberInput",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }
}
