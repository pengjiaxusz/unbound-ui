//! 分割面板：由可拖拽分隔条隔开的两个区域。
//!
//! 该控件仅持有*分割关系*而非面板内容。调用方自行在 [`SplitLayout::first`] 与
//! [`SplitLayout::second`] 内布局内容；分割面板仅保证两个矩形永不重叠、
//! 分隔条可通过调整尺寸光标命中测试，且拖拽时保留最小尺寸约束。
//!
//!
//!
//! 比例保存在状态中，几何信息保存在描述符中：移动分隔条只是纯粹更新 `state.ratio = …`，
//! 因此动画可直接驱动同一字段而无需触碰布局树。
//!

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::{instance, outlined, surface};
use crate::scroll::ScrollAxis;
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认分隔条厚度（逻辑像素）。
pub const SPLIT_HANDLE: f32 = 8.0;

/// 调用方未指定时面板的最小长度。
pub const SPLIT_MIN_PANE: f32 = 48.0;

/// 键盘微调占分割区域的比例。
const KEY_STEP: f32 = 0.02;

/// 折叠操作作用于两个面板中的哪一个。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitSide {
    /// 第一个面板（左侧或顶部）。
    First,
    /// 第二个面板（右侧或底部）。
    Second,
}

/// 分割产生的三个矩形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitLayout {
    /// 第一个面板。
    pub first: Rect,
    /// 可拖拽的分隔条。
    pub gutter: Rect,
    /// 第二个面板。
    pub second: Rect,
}

/// 分割面板的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SplitPaneDesc {
    /// 整个分割区域。
    pub rect: Rect,
    /// `Horizontal` 使面板左右并排，`Vertical` 使面板上下堆叠。
    ///
    /// 变体命名指代面板的排列方向，因此 `Horizontal` 产生带有左右调整光标的垂直分隔条。
    /// `Both` 会回退到 `Horizontal`。
    pub axis: ScrollAxis,
    /// 第一个面板允许的最小长度（像素）。
    pub min_first: f32,
    /// 第二个面板允许的最小长度（像素）。
    pub min_second: f32,
    /// 分隔条厚度（像素）。
    pub handle_size: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl SplitPaneDesc {
    /// 使用默认尺寸创建的分割面板。
    #[must_use]
    pub fn new(rect: Rect, axis: ScrollAxis) -> Self {
        Self {
            rect,
            axis,
            min_first: SPLIT_MIN_PANE,
            min_second: SPLIT_MIN_PANE,
            handle_size: SPLIT_HANDLE,
            z: 400,
        }
    }

    /// 创建左右并排的分割面板。
    #[must_use]
    pub fn horizontal(rect: Rect) -> Self {
        Self::new(rect, ScrollAxis::Horizontal)
    }

    /// 创建上下堆叠的分割面板。
    #[must_use]
    pub fn vertical(rect: Rect) -> Self {
        Self::new(rect, ScrollAxis::Vertical)
    }

    /// 替换最小面板长度约束。
    #[must_use]
    pub fn with_minimums(mut self, first: f32, second: f32) -> Self {
        self.min_first = first.max(0.0);
        self.min_second = second.max(0.0);
        self
    }

    /// 替换分隔条厚度。
    #[must_use]
    pub fn with_handle(mut self, handle: f32) -> Self {
        self.handle_size = handle.max(1.0);
        self
    }

    /// 面板是否按从左到右排列。
    #[must_use]
    pub fn is_horizontal(&self) -> bool {
        !matches!(self.axis, ScrollAxis::Vertical)
    }

    /// 该分割面板使用的分隔条光标。
    #[must_use]
    pub fn cursor(&self) -> CursorKind {
        if self.is_horizontal() {
            CursorKind::ResizeEastWest
        } else {
            CursorKind::ResizeNorthSouth
        }
    }

    /// 扣除分隔条后可用于面板的总长度。
    #[must_use]
    pub fn split_length(&self) -> f32 {
        let total = if self.is_horizontal() {
            self.rect.size.x
        } else {
            self.rect.size.y
        };
        (total - self.handle_size).max(0.0)
    }

    /// 最小面板尺寸允许的比例范围。
    ///
    /// 当两个最小值无法同时满足时，范围会坍缩到中点使两面板均匀收缩：
    /// 完全拒绝分割比违背本就无法满足的约束更糟糕。
    #[must_use]
    pub fn ratio_bounds(&self) -> (f32, f32) {
        let length = self.split_length();
        if length <= 0.0 {
            return (0.0, 0.0);
        }
        let low = (self.min_first / length).clamp(0.0, 1.0);
        let high = (1.0 - self.min_second / length).clamp(0.0, 1.0);
        if low > high {
            let mid = 0.5;
            (mid, mid)
        } else {
            (low, high)
        }
    }

    /// 将比例夹紧到合法范围内。
    #[must_use]
    pub fn clamp_ratio(&self, ratio: f32) -> f32 {
        let (low, high) = self.ratio_bounds();
        ratio.clamp(low, high)
    }

    /// 将指针位置转换为沿分割轴的比例。
    #[must_use]
    pub fn ratio_at(&self, pointer: Vec2) -> f32 {
        let length = self.split_length();
        if length <= 0.0 {
            return 0.0;
        }
        let offset = if self.is_horizontal() {
            pointer.x - self.rect.origin.x
        } else {
            pointer.y - self.rect.origin.y
        };
        self.clamp_ratio(offset / length)
    }
}

/// 分割面板的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct SplitPaneState {
    /// 第一个面板所占分割区域的比例（`0.0..=1.0`）。
    pub ratio: f32,
    /// 分隔条当前是否正在被拖拽。
    pub dragging: bool,
    /// 被折叠为零的面板（若有）。
    pub collapsed: Option<SplitSide>,
    drag_origin: Option<f32>,
}

impl Default for SplitPaneState {
    fn default() -> Self {
        Self::new(0.5)
    }
}

impl SplitPaneState {
    /// 创建指定比例的分割状态。
    #[must_use]
    pub fn new(ratio: f32) -> Self {
        Self {
            ratio: ratio.clamp(0.0, 1.0),
            dragging: false,
            collapsed: None,
            drag_origin: None,
        }
    }

    /// 经夹紧与折叠处理后，面板当前实际使用的比例。
    #[must_use]
    pub fn effective_ratio(&self, desc: &SplitPaneDesc) -> f32 {
        match self.collapsed {
            Some(SplitSide::First) => 0.0,
            Some(SplitSide::Second) => 1.0,
            None => desc.clamp_ratio(self.ratio),
        }
    }

    /// 该状态对应的三个矩形。
    #[must_use]
    pub fn layout(&self, desc: &SplitPaneDesc) -> SplitLayout {
        let length = desc.split_length();
        let first_len = length * self.effective_ratio(desc);
        let handle = desc.handle_size;
        if desc.is_horizontal() {
            let x = desc.rect.origin.x;
            let y = desc.rect.origin.y;
            let h = desc.rect.size.y;
            SplitLayout {
                first: Rect::new(x, y, first_len, h),
                gutter: Rect::new(x + first_len, y, handle, h),
                second: Rect::new(x + first_len + handle, y, (length - first_len).max(0.0), h),
            }
        } else {
            let x = desc.rect.origin.x;
            let y = desc.rect.origin.y;
            let w = desc.rect.size.x;
            SplitLayout {
                first: Rect::new(x, y, w, first_len),
                gutter: Rect::new(x, y + first_len, w, handle),
                second: Rect::new(x, y + first_len + handle, w, (length - first_len).max(0.0)),
            }
        }
    }

    /// 在按下时捕获指针。
    pub fn begin_drag(&mut self, desc: &SplitPaneDesc, pointer: Vec2) {
        self.dragging = true;
        self.drag_origin = Some(if desc.is_horizontal() {
            pointer.x
        } else {
            pointer.y
        });
        // 拖拽已折叠的分割面板意味着将其展开：拒绝移动的分隔条会让用户以为坏掉了，而非“已锁定”。
        //
        self.collapsed = None;
    }

    /// 根据指针位置更新比例。
    ///
    /// 比例直接跟随指针而非累加增量，因此丢帧或合并移动事件绝不会让分隔条落后于光标。
    pub fn drag_to(&mut self, desc: &SplitPaneDesc, pointer: Vec2) {
        if !self.dragging {
            return;
        }
        self.ratio = desc.ratio_at(pointer);
    }

    /// 释放指针。
    pub fn end_drag(&mut self) {
        self.dragging = false;
        self.drag_origin = None;
    }

    /// 折叠指定侧；若已折叠则展开。
    pub fn toggle_collapsed(&mut self, side: SplitSide) {
        self.collapsed = if self.collapsed == Some(side) {
            None
        } else {
            Some(side)
        };
        self.dragging = false;
        self.drag_origin = None;
    }

    /// 展开两个面板。
    pub fn expand(&mut self) {
        self.collapsed = None;
    }

    /// 按指定像素数微调比例。
    pub fn nudge(&mut self, desc: &SplitPaneDesc, pixels: f32) {
        let length = desc.split_length();
        if length <= 0.0 {
            return;
        }
        self.collapsed = None;
        self.ratio = desc.clamp_ratio(self.ratio + pixels / length);
    }

    /// 单次方向键按压产生的比例变化量。
    #[must_use]
    pub fn key_step() -> f32 {
        KEY_STEP
    }

    /// 按下时记录的指针偏移（若正在拖拽）。
    #[must_use]
    pub fn drag_origin(&self) -> Option<f32> {
        self.drag_origin
    }
}

/// 聚焦后的分割面板可识别的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitKey {
    /// 将分隔条向起始侧移动。
    Back,
    /// 将分隔条向末尾侧移动。
    Forward,
    /// 折叠第二个面板。
    Collapse,
    /// 恢复已折叠的面板。
    Expand,
}

/// 将单次按键归约为状态更新。
///
/// 返回状态是否发生变化，供调用方更新脏标记。
pub fn reduce_split_key(desc: &SplitPaneDesc, state: &mut SplitPaneState, key: SplitKey) -> bool {
    match key {
        SplitKey::Back => {
            let before = (state.ratio, state.collapsed);
            state.nudge(desc, -desc.split_length() * KEY_STEP);
            before != (state.ratio, state.collapsed)
        }
        SplitKey::Forward => {
            let before = (state.ratio, state.collapsed);
            state.nudge(desc, desc.split_length() * KEY_STEP);
            before != (state.ratio, state.collapsed)
        }
        SplitKey::Collapse => {
            if state.collapsed == Some(SplitSide::Second) {
                false
            } else {
                state.toggle_collapsed(SplitSide::Second);
                true
            }
        }
        SplitKey::Expand => {
            if state.collapsed.is_some() {
                state.expand();
                true
            } else {
                false
            }
        }
    }
}

/// 分割面板绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitPaneTokens {
    /// 分隔条轨道。
    pub track: Token,
    /// 悬停或拖拽时的分隔条轨道。
    pub track_active: Token,
    /// 抓手把手。
    pub grip: Token,
    /// 悬停或拖拽时的抓手把手。
    pub grip_active: Token,
    /// 键盘焦点环。
    pub focus_ring: Token,
}

impl SplitPaneTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            track: Token::SURFACE_SUNKEN,
            track_active: Token::CONTROL_HOVER,
            grip: Token::OUTLINE,
            grip_active: Token::PRIMARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 输出分隔条：轨道、抓手与调整尺寸命中区域。
///
/// 本函数不绘制两个面板；调用方自行在 [`SplitLayout::first`] 与 [`SplitLayout::second`] 内绘制内容。
#[must_use]
pub fn emit_split_pane(
    node: u64,
    desc: &SplitPaneDesc,
    state: &SplitPaneState,
    control: ControlState,
    tokens: SplitPaneTokens,
) -> WidgetOutput {
    let layout = state.layout(desc);
    let disabled = control.disabled;
    let active =
        matches!(control.phase, PointerPhase::Hovered | PointerPhase::Pressed) || state.dragging;

    let track = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if active {
        tokens.track_active
    } else {
        tokens.track
    };
    let grip_color = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if active {
        tokens.grip_active
    } else {
        tokens.grip
    };

    let mut primitives = Vec::with_capacity(5);
    primitives.push(surface(
        layout.gutter,
        desc.handle_size * 0.5,
        track,
        desc.z,
    ));

    // 沿分隔条的三个圆点无需字形即可提供拖拽视觉提示，因此即使字体缺少“⋮”也能渲染出可用把手。
    //
    let (cx, cy) = (
        layout.gutter.origin.x + layout.gutter.size.x * 0.5,
        layout.gutter.origin.y + layout.gutter.size.y * 0.5,
    );
    let dot = (desc.handle_size * 0.22).clamp(1.0, 2.5);
    for offset in [-1.0f32, 0.0, 1.0] {
        let centre = if desc.is_horizontal() {
            Vec2::new(cx, cy + offset * dot * 3.0)
        } else {
            Vec2::new(cx + offset * dot * 3.0, cy)
        };
        if desc.is_horizontal() && layout.gutter.size.y < dot * 9.0 {
            continue;
        }
        if !desc.is_horizontal() && layout.gutter.size.x < dot * 9.0 {
            continue;
        }
        primitives.push(dot_primitive(centre, dot, grip_color, desc.z));
    }

    // 焦点环始终输出；聚焦仅改变其不透明度。
    primitives.push(crate::primitive::focus_ring(
        layout.gutter.inflate(1.0),
        3.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), layout.gutter)
            .with_cursor(desc.cursor())
    });
    let percent = (state.effective_ratio(desc) * 100.0).round();
    let mut a11y = A11yNode::new(Role::Splitter, "", layout.gutter)
        .clickable()
        .disabled(disabled);
    a11y.value = format!("{percent:.0}%");

    WidgetOutput {
        primitives,
        payloads: Vec::<GpuPayload>::new(),
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 小型圆形抓手圆点。
fn dot_primitive(centre: Vec2, radius: f32, color: Token, z: u16) -> GpuPrimitiveInstance {
    let dot = Rect::new(
        centre.x - radius,
        centre.y - radius,
        radius * 2.0,
        radius * 2.0,
    );
    let mut primitive = instance(
        dot,
        [radius; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Content,
        z,
    );
    primitive.border_token_or_tex = Token::NONE.raw() as u32;
    primitive
}

/// 若调用方希望由分割面板绘制背景，则输出两个面板的底色。
///
/// 多数调用方会自绘面板；提供此辅助函数是为了让裸分割面板
/// 也能显示两侧区域，而非一整块透明矩形。
#[must_use]
pub fn emit_split_pane_panes(
    desc: &SplitPaneDesc,
    state: &SplitPaneState,
    fill: Token,
    surface_token: Token,
) -> WidgetOutput {
    let layout = state.layout(desc);
    let mut primitives = Vec::with_capacity(2);
    if layout.first.size.x > 0.0 && layout.first.size.y > 0.0 {
        primitives.push(outlined(
            layout.first,
            0.0,
            fill,
            surface_token,
            1.0,
            0.0,
            desc.z,
        ));
    }
    if layout.second.size.x > 0.0 && layout.second.size.y > 0.0 {
        primitives.push(outlined(
            layout.second,
            0.0,
            fill,
            surface_token,
            1.0,
            0.0,
            desc.z,
        ));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> SplitPaneDesc {
        SplitPaneDesc::horizontal(Rect::new(0.0, 0.0, 500.0, 300.0))
    }

    #[test]
    fn the_gutter_separates_two_disjoint_panes() {
        let desc = desc();
        let layout = SplitPaneState::new(0.5).layout(&desc);
        assert_eq!(layout.first.origin.x, 0.0);
        assert_eq!(layout.first.size.x, 246.0);
        assert_eq!(layout.gutter.origin.x, 246.0);
        assert_eq!(layout.gutter.size.x, 8.0);
        assert_eq!(layout.second.origin.x, 254.0);
        assert_eq!(layout.second.size.x, 246.0);
    }

    #[test]
    fn a_vertical_split_stacks_along_y() {
        let desc = SplitPaneDesc::vertical(Rect::new(0.0, 0.0, 300.0, 500.0));
        let layout = SplitPaneState::new(0.25).layout(&desc);
        assert_eq!(layout.first.size.y, 123.0);
        assert_eq!(layout.gutter.origin.y, 123.0);
        assert_eq!(layout.gutter.size.y, 8.0);
        assert_eq!(layout.second.origin.y, 131.0);
        assert_eq!(layout.second.size.y, 369.0);
    }

    #[test]
    fn the_minimum_sizes_clamp_the_ratio() {
        let desc = desc().with_minimums(100.0, 100.0);
        let mut state = SplitPaneState::new(0.5);
        assert_eq!(state.effective_ratio(&desc), 0.5);

        state.drag_to(&desc, Vec2::new(0.0, 0.0));
        assert_eq!(state.effective_ratio(&desc), 0.5, "drag_to needs a begin");

        state.begin_drag(&desc, Vec2::new(250.0, 150.0));
        state.drag_to(&desc, Vec2::new(0.0, 150.0));
        assert!((state.effective_ratio(&desc) - 100.0 / 492.0).abs() < 1e-4);

        state.drag_to(&desc, Vec2::new(500.0, 150.0));
        assert!((state.effective_ratio(&desc) - (1.0 - 100.0 / 492.0)).abs() < 1e-4);
    }

    #[test]
    fn dragging_follows_the_pointer_and_ends_on_release() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        state.begin_drag(&desc, Vec2::new(246.0, 150.0));
        assert!(state.dragging);
        state.drag_to(&desc, Vec2::new(123.0, 150.0));
        assert!((state.ratio - 0.25).abs() < 1e-4);
        assert!((state.layout(&desc).first.size.x - 123.0).abs() < 1e-4);
        state.end_drag();
        assert!(!state.dragging);
        state.drag_to(&desc, Vec2::new(400.0, 150.0));
        assert!((state.ratio - 0.25).abs() < 1e-4, "released drag is inert");
    }

    #[test]
    fn double_click_collapses_and_restores_the_second_pane() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        state.toggle_collapsed(SplitSide::Second);
        assert_eq!(state.collapsed, Some(SplitSide::Second));
        let layout = state.layout(&desc);
        assert_eq!(layout.second.size.x, 0.0);
        assert_eq!(layout.first.size.x, 492.0);
        assert_eq!(layout.gutter.origin.x, 492.0);

        state.toggle_collapsed(SplitSide::Second);
        assert_eq!(state.collapsed, None);
        assert!((state.layout(&desc).first.size.x - 246.0).abs() < 1e-4);
    }

    #[test]
    fn collapsing_the_first_pane_moves_the_gutter_to_the_origin() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        state.toggle_collapsed(SplitSide::First);
        let layout = state.layout(&desc);
        assert_eq!(layout.first.size.x, 0.0);
        assert_eq!(layout.gutter.origin.x, 0.0);
        assert_eq!(layout.second.origin.x, 8.0);
        assert_eq!(layout.second.size.x, 492.0);
    }

    #[test]
    fn dragging_a_collapsed_split_expands_it() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        state.toggle_collapsed(SplitSide::Second);
        state.begin_drag(&desc, Vec2::new(492.0, 150.0));
        assert_eq!(state.collapsed, None);
        assert!(state.dragging);
    }

    #[test]
    fn the_gutter_is_pickable_with_the_resize_cursor() {
        let desc = desc();
        let tokens = SplitPaneTokens::standard();
        let output = emit_split_pane(
            7,
            &desc,
            &SplitPaneState::new(0.5),
            ControlState::default(),
            tokens,
        );
        let hit = output.hit.expect("the gutter is always pickable");
        assert_eq!(hit.node, 7);
        assert_eq!(hit.cursor, Some(CursorKind::ResizeEastWest));
        assert!(output.focusable);
        let a11y = output.a11y.expect("a splitter is reachable");
        assert_eq!(a11y.role, Role::Splitter);
        assert_eq!(a11y.value, "50%");
    }

    #[test]
    fn a_vertical_split_uses_the_north_south_cursor() {
        let desc = SplitPaneDesc::vertical(Rect::new(0.0, 0.0, 200.0, 400.0));
        let output = emit_split_pane(
            1,
            &desc,
            &SplitPaneState::new(0.5),
            ControlState::default(),
            SplitPaneTokens::standard(),
        );
        assert_eq!(
            output.hit.expect("pickable").cursor,
            Some(CursorKind::ResizeNorthSouth)
        );
    }

    #[test]
    fn a_hovered_gutter_paints_the_active_tokens() {
        let desc = desc();
        let tokens = SplitPaneTokens::standard();
        let output = emit_split_pane(
            1,
            &desc,
            &SplitPaneState::new(0.5),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.track_active.raw() as u32
        );
    }

    #[test]
    fn a_disabled_split_is_not_pickable() {
        let desc = desc();
        let output = emit_split_pane(
            1,
            &desc,
            &SplitPaneState::new(0.5),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            SplitPaneTokens::standard(),
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }

    #[test]
    fn arrow_keys_nudge_and_saturate_at_the_minimums() {
        let desc = desc().with_minimums(50.0, 50.0);
        let mut state = SplitPaneState::new(0.5);
        assert!(reduce_split_key(&desc, &mut state, SplitKey::Forward));
        assert!((state.ratio - 0.52).abs() < 1e-4);
        for _ in 0..200 {
            reduce_split_key(&desc, &mut state, SplitKey::Forward);
        }
        assert!((state.effective_ratio(&desc) - (1.0 - 50.0 / 492.0)).abs() < 1e-4);
    }

    #[test]
    fn collapse_and_expand_keys_are_idempotent() {
        let desc = desc();
        let mut state = SplitPaneState::new(0.5);
        assert!(reduce_split_key(&desc, &mut state, SplitKey::Collapse));
        assert!(!reduce_split_key(&desc, &mut state, SplitKey::Collapse));
        assert!(reduce_split_key(&desc, &mut state, SplitKey::Expand));
        assert!(!reduce_split_key(&desc, &mut state, SplitKey::Expand));
        state.expand();
        assert!(!reduce_split_key(&desc, &mut state, SplitKey::Expand));
    }

    #[test]
    fn impossible_minimums_collapse_to_the_midpoint() {
        let desc =
            SplitPaneDesc::horizontal(Rect::new(0.0, 0.0, 100.0, 50.0)).with_minimums(200.0, 200.0);
        let (low, high) = desc.ratio_bounds();
        assert_eq!(low, high);
        assert_eq!(desc.clamp_ratio(0.1), 0.5);
    }
}
