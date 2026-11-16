//! 颜色色块（Color Swatch）：Win11 设置应用「个性化 > 颜色」里的颜色网格单元。
//!
//! Win11 的颜色选择器是一格格**填满品牌色**的圆角方块，选中项外侧套一圈
//! 强调色细环。颜色本身来自调色板槽位（[`unbound_theme::Token`]）而非图元里的
//! 字面颜色，因此切换品牌强调色只需重写查找表，色块图元本身不必重传。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::{focus_ring, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认色块边长（逻辑像素）。
///
/// 实测 Win11「个性化 > 颜色」里的色块是 **50px** 见方、相邻间距 3px，
/// 整排 10 块。这里取 48px 见方 + 4px 间距，视觉密度与原生一致，
/// 同时给外侧 3px 选中环留出余量，避免相邻色块的环互相压上。
pub const SWATCH_SIZE: f32 = unbound_theme::typography::geometry::COLOR_SWATCH_SIZE;

/// 相邻色块之间的间距。
pub const SWATCH_GAP: f32 = unbound_theme::typography::geometry::COLOR_SWATCH_GAP;

/// 默认色块圆角半径。
///
/// Win11 的色块是近乎方角的 4px 圆角，与设置页的分组卡片保持同一套几何。
pub const SWATCH_RADIUS: f32 = 4.0;

/// 选中环向色块外延伸的距离。
const RING_GAP: f32 = 3.0;

/// 颜色色块的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ColorSwatchDesc {
    /// 色块矩形。
    pub rect: Rect,
    /// 填充色令牌（通常是品牌色板槽位）。
    pub fill: Token,
    /// 是否为当前选中项。
    pub selected: bool,
    /// 无障碍标签。
    pub label: String,
    /// 绘制顺序键。
    pub z: u16,
}

impl ColorSwatchDesc {
    /// 创建颜色色块。
    #[must_use]
    pub fn new(rect: Rect, fill: Token, label: impl Into<String>) -> Self {
        Self {
            rect,
            fill,
            selected: false,
            label: label.into(),
            z: 702,
        }
    }

    /// 设置选中状态。
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 选中环矩形。
    #[must_use]
    pub fn ring_rect(&self) -> Rect {
        self.rect.inflate(RING_GAP)
    }
}

/// 发射颜色色块：色块本体 + 悬停描边 + 选中环 + 焦点环。
///
/// 图元数量在悬停 / 按下 / 聚焦下**恒定**：状态只改属性（透明度），
/// 因此色块不会因交互瞬态触发图元池重传。
#[must_use]
pub fn emit_color_swatch(node: u64, desc: &ColorSwatchDesc, control: ControlState) -> WidgetOutput {
    let z = desc.z;
    let radius = SWATCH_RADIUS;
    let mut primitives = Vec::with_capacity(4);

    // 1. 色块本体。
    primitives.push(surface(desc.rect, radius, desc.fill, z));

    // 2. 悬停描边：Win11 的色块悬停时外沿浮起一圈淡描边。
    let hover_opacity = if control.phase == PointerPhase::Hovered && !control.disabled {
        1.0
    } else {
        0.0
    };
    let mut hover = outlined(
        desc.rect,
        radius,
        Token::NONE,
        Token::OUTLINE_STRONG,
        1.0,
        0.0,
        z + 1,
    );
    hover.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z + 1, hover_opacity);
    primitives.push(hover);

    // 3. 选中环：强调色细环套在色块外侧。
    let ring = desc.ring_rect();
    let mut selection = outlined(
        ring,
        radius + RING_GAP,
        Token::NONE,
        Token::PRIMARY,
        2.0,
        0.0,
        z + 2,
    );
    selection.z_and_opacity =
        GpuPrimitiveInstance::pack_z_opacity(z + 2, if desc.selected { 1.0 } else { 0.0 });
    primitives.push(selection);

    // 4. 焦点环。
    primitives.push(focus_ring(
        ring.inflate(2.0),
        radius + RING_GAP + 2.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded { radii: [radius; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Button, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(control.disabled);
    a11y.selected = desc.selected;

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ColorSwatchDesc {
        ColorSwatchDesc::new(
            Rect::new(0.0, 0.0, SWATCH_SIZE, SWATCH_SIZE),
            Token::ACCENT_SWATCH_0,
            "工程蓝",
        )
    }

    #[test]
    fn the_swatch_paints_its_own_fill() {
        let output = emit_color_swatch(1, &desc(), ControlState::default());
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            Token::ACCENT_SWATCH_0.raw() as u32
        );
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let idle = emit_color_swatch(1, &desc(), ControlState::default());
        let hovered = emit_color_swatch(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
        );
        let selected = emit_color_swatch(1, &desc().selected(true), ControlState::default());
        assert_eq!(idle.primitives.len(), 4);
        assert_eq!(hovered.primitives.len(), 4);
        assert_eq!(selected.primitives.len(), 4);
    }

    #[test]
    fn a_disabled_swatch_leaves_the_hit_set() {
        let output = emit_color_swatch(
            1,
            &desc(),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }
}
