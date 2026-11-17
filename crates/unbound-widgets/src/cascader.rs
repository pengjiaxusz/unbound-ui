//! 多级联动选择器（Cascader）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{curve_stroke, focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_LEVELS: usize = 3;

/// [`CascaderDesc`] 的层级选项节点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CascaderOption {
    /// 选项值/ID。
    pub value: String,
    /// 显示标签。
    pub label: String,
    /// 嵌套子选项。
    pub children: Vec<CascaderOption>,
}

impl CascaderOption {
    /// 创建叶子选项。
    #[must_use]
    pub fn leaf(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            children: Vec::new(),
        }
    }

    /// 创建带子选项的分支选项。
    #[must_use]
    pub fn branch(
        value: impl Into<String>,
        label: impl Into<String>,
        children: Vec<CascaderOption>,
    ) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            children,
        }
    }
}

/// [`CascaderDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CascaderState {
    /// 跨层级选中的值路径。
    pub path: Vec<String>,
    /// 多列弹出面板是否展开。
    pub open: bool,
}

/// [`Cascader`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CascaderDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 根级选项列表。
    pub options: Vec<CascaderOption>,
    /// 当前选中路径。
    pub path: Vec<String>,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl CascaderDesc {
    /// 创建级联选择器描述符。
    #[must_use]
    pub fn new(rect: Rect, options: Vec<CascaderOption>, path: Vec<String>) -> Self {
        Self {
            rect,
            options,
            path,
            label: "Cascader".to_owned(),
            z: 600,
        }
    }
}

/// 级联选择器面包屑层级胶囊（`0..3`）的确定性子节点 ID。
#[must_use]
pub const fn cascader_level_node(node: u64, level: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0xA00 + level as u64)
}

/// 发射固定形状的级联选择器（1 触发器 + 3 层级胶囊 + 1 箭头 + 1 焦点环 = 6 个图元）。
#[must_use]
pub fn emit_cascader(
    node: u64,
    desc: &CascaderDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(6 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    // 无原生 WinUI 对应控件；触发器对齐 ComboBox：
    // CornerRadius = ControlCornerRadius = 4，底色 ControlFillColorDefault。
    primitives.push(outlined(
        desc.rect,
        4.0,
        if state.disabled {
            // ComboBoxBackgroundDisabled = ControlFillColorDisabled。
            Token::CONTROL_DISABLED_BG
        } else if hovered {
            // ComboBoxBackgroundPointerOver = ControlFillColorSecondary。
            Token::CONTROL_HOVER
        } else {
            Token::CONTROL_FILL
        },
        if state.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if state.focused {
            // ComboBoxBackgroundBorderBrushFocused = FocusStrokeColorOuter。
            Token::OUTLINE_STRONG
        } else {
            // ComboBoxBorderBrush = ControlElevationBorderBrush。
            Token::CONTROL_STROKE
        },
        1.0,
        0.0,
        desc.z,
    ));

    let pill_w = ((desc.rect.width() - 36.0) / MAX_LEVELS as f32).max(16.0);
    let pill_h = (desc.rect.height() - 10.0).max(12.0);
    for level in 0..MAX_LEVELS {
        let r = Rect::new(
            desc.rect.origin.x + 6.0 + level as f32 * (pill_w + 4.0),
            desc.rect.origin.y + 5.0,
            pill_w,
            pill_h,
        );
        let active = level < desc.path.len();
        let mut inst = surface(
            r,
            4.0,
            if active {
                Token::PRIMARY_SOFT
            } else {
                Token::SURFACE_SUNKEN
            },
            desc.z + 1,
        );
        if !active && level > 0 {
            inst.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(inst.depth_u16(), 0.0);
        }
        primitives.push(inst);
    }

    // 展开箭头：与 ComboBox 一致，12×12 图标距右沿 14px，故中心在右沿内 20px。
    let chev_cx = desc.rect.max().x - 20.0;
    let chev_cy = desc.rect.origin.y + desc.rect.height() * 0.5;
    curve_stroke(
        [chev_cx - 4.0, chev_cy - 2.0],
        [chev_cx, chev_cy + 2.0],
        1.5,
        Token::TEXT_SECONDARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );
    curve_stroke(
        [chev_cx, chev_cy + 2.0],
        [chev_cx + 4.0, chev_cy - 2.0],
        1.5,
        Token::TEXT_SECONDARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    primitives.push(focus_ring(
        desc.rect,
        4.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y =
        A11yNode::new(Role::ComboBox, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for (level, seg) in desc.path.iter().take(MAX_LEVELS).enumerate() {
            let r = Rect::new(
                desc.rect.origin.x + 6.0 + level as f32 * (pill_w + 4.0),
                desc.rect.origin.y + 5.0,
                pill_w,
                pill_h,
            );
            let key = cascader_level_node(node, level);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, seg.clone(), r).clickable(),
                true,
            );
        }
    }

    out
}
