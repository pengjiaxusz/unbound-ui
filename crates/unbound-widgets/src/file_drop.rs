//! 文件拖放区（FileDropZone）：操作系统拖拽反馈与路径传递。
//!
//! 引擎从不关心文件来自何处。操作系统拖拽以携带 [`PayloadArena`] 载荷 ID 的 [`DragOver`] 形式到达；
//! 本控件仅回答两个问题——“指针是否悬停在我上方”以及“我收到了哪些路径”——
//! 因此从资源管理器拖入的文件与在应用内部拖拽的文件走完全相同的路径。
//!
//! 路径以 `Vec<PathBuf>` 跨越控件边界，而非作为动作载荷：
//! 一次拖放一百个文件也不会占用撤销历史。

use std::path::PathBuf;

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, DragOver, HitRegion, PayloadArena, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 虚线轮廓的内边距。
pub const DROP_INSET: f32 = 8.0;
/// 单段虚线长度。
pub const DROP_DASH: f32 = 10.0;
/// 虚线之间的间隔。
pub const DROP_GAP: f32 = 7.0;
/// 拖放区圆角半径。
pub const DROP_RADIUS: f32 = 10.0;

/// 文件拖放区的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct FileDropDesc {
    /// 区域矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 区域内显示的提示文案。
    pub hint: String,
    /// 是否允许单次拖放多个文件。
    pub accept_multiple: bool,
    /// 区域是否禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl FileDropDesc {
    /// 创建可接收多个文件的拖放区。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            hint: String::new(),
            accept_multiple: true,
            disabled: false,
            z: 745,
        }
    }
}

/// 拖放区的实时状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDropState {
    hovered: bool,
    payload: Option<u64>,
}

impl FileDropState {
    /// 闲置状态的拖放区。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否有兼容的拖拽悬停在区域上方。
    #[must_use]
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// 当前悬停的载荷（若有）。
    #[must_use]
    pub fn payload(&self) -> Option<u64> {
        self.payload
    }

    /// 根据操作系统拖拽更新悬停状态。
    ///
    /// 返回区域视觉状态是否发生变化。
    pub fn drag_over(
        &mut self,
        node: u64,
        desc: &FileDropDesc,
        over: &DragOver,
        arena: &PayloadArena,
    ) -> bool {
        let over_us = !desc.disabled
            && over
                .target
                .map(|target| u64::from(target.0))
                .is_some_and(|target| target == node)
            && desc.rect.contains_point(over.position);
        let compatible = arena
            .get(over.payload_id)
            .is_some_and(|payload| !payload.is_empty());
        let hovered = over_us && compatible;
        let payload = hovered.then_some(over.payload_id);
        let changed = self.hovered != hovered || self.payload != payload;
        self.hovered = hovered;
        self.payload = payload;
        changed
    }

    /// 完成拖拽，返回待传递的文件列表。
    ///
    /// 单文件区域会截断列表而非拒绝拖放：第一条路径是用户最先抓取的文件，这是最符合直觉的选择。
    pub fn drop_files(
        &mut self,
        desc: &FileDropDesc,
        over: &DragOver,
        arena: &PayloadArena,
    ) -> Option<Vec<PathBuf>> {
        let accepted = self.hovered && self.payload == Some(over.payload_id);
        self.leave();
        if !accepted {
            return None;
        }
        let mut paths = arena.get(over.payload_id)?.paths.clone();
        if paths.is_empty() {
            return None;
        }
        if !desc.accept_multiple {
            paths.truncate(1);
        }
        Some(paths)
    }

    /// 拖拽已离开区域或窗口。
    pub fn leave(&mut self) -> bool {
        let changed = self.hovered || self.payload.is_some();
        self.hovered = false;
        self.payload = None;
        changed
    }
}

/// 使用线段图元绘制的虚线矩形。
fn dash_rect(rect: Rect, mut out: impl FnMut(Rect, PrimType)) {
    let step = DROP_DASH + DROP_GAP;
    let mut x = rect.origin.x;
    while x < rect.max().x {
        let width = DROP_DASH.min(rect.max().x - x);
        out(Rect::new(x, rect.origin.y, width, 1.0), PrimType::Line);
        out(Rect::new(x, rect.max().y - 1.0, width, 1.0), PrimType::Line);
        x += step;
    }
    let mut y = rect.origin.y;
    while y < rect.max().y {
        let height = DROP_DASH.min(rect.max().y - y);
        out(Rect::new(rect.origin.x, y, 1.0, height), PrimType::Line);
        out(
            Rect::new(rect.max().x - 1.0, y, 1.0, height),
            PrimType::Line,
        );
        y += step;
    }
}

/// 发射区域框架、虚线轮廓与语义节点。
#[must_use]
pub fn emit_file_drop(
    node: u64,
    desc: &FileDropDesc,
    state: &FileDropState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let mut primitives = Vec::new();
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if state.hovered || control.phase == crate::PointerPhase::Hovered {
        Token::PRIMARY_SOFT
    } else {
        Token::SURFACE_SUNKEN
    };
    primitives.push(surface(rect, DROP_RADIUS, fill, desc.z));

    let inner = rect.inflate(-DROP_INSET);
    let mark = if control.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if state.hovered || control.focused {
        Token::PRIMARY
    } else {
        Token::OUTLINE_STRONG
    };
    dash_rect(inner, |dash, kind| {
        primitives.push(instance(
            dash,
            [0.0; 4],
            mark,
            Token::NONE,
            0.0,
            0.0,
            kind,
            SubElement::Content,
            desc.z,
        ));
    });

    // 简单的“托盘”图标：两段描边加一个圆点，无需字体。
    let cx = rect.center().x;
    let cy = rect.center().y - 6.0;
    for (dx, width) in [(-8.0, 16.0), (-5.0, 1.0)] {
        primitives.push(instance(
            Rect::new(cx + dx, cy, width, 1.5),
            [0.0; 4],
            mark,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            desc.z,
        ));
    }
    primitives.push(surface(
        Rect::new(cx - 5.0, cy + 5.0, 10.0, 8.0),
        2.0,
        mark,
        desc.z,
    ));

    emit_glyphs(
        glyphs,
        if control.disabled {
            Token::TEXT_DISABLED
        } else if state.hovered {
            Token::PRIMARY
        } else {
            Token::TEXT_SECONDARY
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );

    primitives.push(focus_ring(
        rect.inflate(3.0),
        DROP_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded {
                radii: [DROP_RADIUS; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Group, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.description = desc.hint.clone();

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// 拖拽悬停的点坐标，供测试与应用命中测试使用。
#[must_use]
pub fn drop_point(rect: Rect) -> Vec2 {
    rect.center()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_abi::desc::RawNodeId;
    use unbound_input::DragPayload;

    fn desc() -> FileDropDesc {
        FileDropDesc::new(Rect::new(0.0, 0.0, 200.0, 100.0))
    }

    fn over(node: u64, payload_id: u64, position: Vec2) -> DragOver {
        DragOver {
            position,
            target: Some(RawNodeId(u32::try_from(node).expect("test node fits"))),
            payload_id,
        }
    }

    #[test]
    fn hovering_requires_the_zone_to_be_the_hit_target() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![PathBuf::from("a.txt")]));
        let mut state = FileDropState::new();

        assert!(!state.drag_over(1, &desc, &over(2, id, Vec2::new(50.0, 50.0)), &arena));
        assert!(!state.is_hovered());
        assert!(state.drag_over(1, &desc, &over(1, id, Vec2::new(50.0, 50.0)), &arena));
        assert!(state.is_hovered());
        assert_eq!(state.payload(), Some(id));
    }

    #[test]
    fn a_position_outside_the_rect_is_rejected() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![PathBuf::from("a.txt")]));
        let mut state = FileDropState::new();
        assert!(!state.drag_over(1, &desc, &over(1, id, Vec2::new(500.0, 50.0)), &arena));
        assert!(!state.is_hovered());
    }

    #[test]
    fn an_empty_payload_never_lights_the_zone() {
        let desc = desc();
        let arena = PayloadArena::new();
        let mut state = FileDropState::new();
        assert!(!state.drag_over(1, &desc, &over(1, 99, Vec2::new(10.0, 10.0)), &arena));
    }

    #[test]
    fn dropping_delivers_the_paths_and_clears_the_hover() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![
            PathBuf::from("a.txt"),
            PathBuf::from("b.txt"),
        ]));
        let mut state = FileDropState::new();
        let drag = over(1, id, Vec2::new(20.0, 20.0));
        state.drag_over(1, &desc, &drag, &arena);
        assert_eq!(
            state.drop_files(&desc, &drag, &arena),
            Some(vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")])
        );
        assert!(!state.is_hovered());
    }

    #[test]
    fn a_single_file_zone_truncates_the_drop() {
        let mut desc = desc();
        desc.accept_multiple = false;
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![
            PathBuf::from("a.txt"),
            PathBuf::from("b.txt"),
        ]));
        let mut state = FileDropState::new();
        let drag = over(1, id, Vec2::new(20.0, 20.0));
        state.drag_over(1, &desc, &drag, &arena);
        assert_eq!(
            state.drop_files(&desc, &drag, &arena),
            Some(vec![PathBuf::from("a.txt")])
        );
    }

    #[test]
    fn dropping_without_a_hover_delivers_nothing() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![PathBuf::from("a.txt")]));
        let mut state = FileDropState::new();
        assert_eq!(
            state.drop_files(&desc, &over(1, id, Vec2::new(20.0, 20.0)), &arena),
            None
        );
    }

    #[test]
    fn leaving_clears_the_state_once() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![PathBuf::from("a.txt")]));
        let mut state = FileDropState::new();
        state.drag_over(1, &desc, &over(1, id, Vec2::new(20.0, 20.0)), &arena);
        assert!(state.leave());
        assert!(!state.leave());
    }

    #[test]
    fn the_emitter_reports_a_pointer_cursor_and_the_hint() {
        let desc = desc();
        let state = FileDropState::new();
        let output = emit_file_drop(4, &desc, &state, ControlState::default(), &mut []);
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Group);
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Pointer))
        );
        assert!(output.primitives.len() > 20, "dashed outline is emitted");
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let mut counts = Vec::new();
        for drag_hovered in [false, true] {
            let mut state = FileDropState::new();
            state.hovered = drag_hovered;
            for phase in [
                crate::PointerPhase::Idle,
                crate::PointerPhase::Hovered,
                crate::PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let output = emit_file_drop(4, &desc, &state, control, &mut []);
                            counts.push(output.primitives.len());
                        }
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}
