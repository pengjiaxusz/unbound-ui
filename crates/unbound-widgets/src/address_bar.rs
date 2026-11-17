//! 双模路径导航地址栏（`AddressBar`）：支持分段路径面包屑与全路径文本编辑双态切换。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::text_input::focus_underline;
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 地址栏圆角。对齐 WinUI `ControlCornerRadius = 4`（AutoSuggestBox）。
const BAR_RADIUS: f32 = 4.0;
/// 内嵌导航按钮圆角。对齐 WinUI `ControlCornerRadius = 4`。
const INNER_BUTTON_RADIUS: f32 = 4.0;
/// 路径分段胶囊圆角。对齐 WinUI `ControlCornerRadius = 4`。
const SEGMENT_RADIUS: f32 = 4.0;

/// 双模地址栏描述符（平时显示分段路径面包屑，点击空白或按下快捷键切换为完整路径编辑框）。
#[derive(Clone, Debug, PartialEq)]
pub struct AddressBarDesc {
    pub rect: Rect,
    pub segments: Vec<String>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub z: u16,
}

impl AddressBarDesc {
    #[must_use]
    pub fn new(rect: Rect, segments: Vec<String>) -> Self {
        Self {
            rect,
            segments,
            can_go_back: true,
            can_go_forward: false,
            z: 10,
        }
    }

    /// 完整路径字符串（以 `/` 连接）。
    #[must_use]
    pub fn full_path(&self) -> String {
        if self.segments.is_empty() {
            "/".to_string()
        } else {
            self.segments.join("/")
        }
    }
}

/// 双模地址栏交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressBarState {
    /// 是否处于全路径文本编辑模式。
    pub editing: bool,
    /// 编辑缓冲区内容。
    pub edit_buffer: String,
    /// 历史后退栈。
    pub back_stack: Vec<Vec<String>>,
    /// 历史前进栈。
    pub forward_stack: Vec<Vec<String>>,
}

impl AddressBarState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 切换进入全路径编辑态。
    pub fn begin_edit(&mut self, current_path: &str) {
        self.editing = true;
        self.edit_buffer = current_path.to_owned();
    }

    /// 提交编辑路径并解析为分段列表。
    pub fn commit_edit(&mut self) -> Vec<String> {
        self.editing = false;
        self.edit_buffer
            .split(['/', '\\'])
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// 发射双模地址栏（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_address_bar(
    node: u64,
    desc: &AddressBarDesc,
    state: &AddressBarState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(6 + desc.segments.len() + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let bg = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else {
        // `ControlFillColorDefault`：AutoSuggestBox / 地址栏的控件表面填充。
        Token::CONTROL_FILL
    };
    let border = if control.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if control.phase == PointerPhase::Hovered {
        // `TextControlBorderBrushPointerOver` 档的浅描边。
        Token::OUTLINE_STRONG
    } else {
        // `ControlStrokeColorDefault`：1px（`TextControlBorderThemeThickness = 1`）。
        Token::CONTROL_STROKE
    };
    primitives.push(outlined(r, BAR_RADIUS, bg, border, 1.0, 0.0, z));

    // 聚焦态底部 2px 强调条（`TextControlBorderThemeThicknessFocused = 1,1,1,2`）：
    // 图元始终存在，进入编辑或获得焦点时改为不透明，保证图元数恒定。
    primitives.push(focus_underline(
        r,
        BAR_RADIUS,
        z,
        if !control.disabled && (state.editing || control.focused) {
            1.0
        } else {
            0.0
        },
    ));

    // 后退、前进、向上、刷新四个导航槽位背景（恒定 4 个图元）
    for i in 0..4usize {
        let btn_r = Rect::new(
            r.origin.x + 4.0 + i as f32 * 26.0,
            r.origin.y + 4.0,
            22.0,
            (r.size.y - 8.0).max(16.0),
        );
        // 导航按钮是「微妙」按钮：静息透明，悬停才出现 `SubtleFillColorSecondary`。
        let fill = if !control.disabled && control.phase == PointerPhase::Hovered && i == 0 {
            Token::HOVER
        } else {
            Token::NONE
        };
        primitives.push(surface(btn_r, INNER_BUTTON_RADIUS, fill, z + 1));
    }

    // 路径分段胶囊槽位（对固定 `desc.segments` 恒定发射）
    let mut x = r.origin.x + 114.0;
    for seg in &desc.segments {
        let w = (seg.chars().count() as f32 * 7.5 + 16.0).clamp(36.0, 140.0);
        let seg_r = Rect::new(x, r.origin.y + 4.0, w, (r.size.y - 8.0).max(16.0));
        // 面包屑段落在 WinUI 中静息为 `SubtleFillColorSecondary`（即 HOVER 令牌），
        // 进入整行文本编辑时隐去，避免与编辑框的纯文本冲突。
        let seg_fill = if state.editing || control.disabled {
            Token::NONE
        } else {
            Token::HOVER
        };
        primitives.push(surface(seg_r, SEGMENT_RADIUS, seg_fill, z + 1));
        x += w + 6.0;
    }

    emit_glyphs(
        glyphs,
        if control.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        BAR_RADIUS + 2.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Toolbar, desc.full_path(), r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
}
