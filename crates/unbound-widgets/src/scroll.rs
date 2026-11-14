//! 滚动容器与虚拟列表回放。
//!
//! [`ScrollViewDesc`] 描述视口；[`ScrollState`] 持有滚动偏移量及其弹簧状态。
//! 两者均不持有子控件：子元素由调用方输出，并由 [`ScrollViewOutput::clip`] 裁剪、
//! 由 [`ScrollViewOutput::transform`] 平移。这正是亚像素滚动零开销的原因——
//! 滚动仅修改单个 `GpuTransform`，而不会修改任何子图元。
//!
//!
//! 虚拟列表通过 [`update_virtual_list`] 接入：每帧重算一次可见窗口，
//! 当仅亚行偏移发生变化时返回 `WindowUpdate::offset_only`，实现**零**重新绑定。
//!

use unbound_anim::{Spring2, SpringPreset};
use unbound_gpu::data::{GpuClipNode, GpuPrimitiveInstance, GpuTransform, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;
use unbound_world::virtualize::VirtualList;

use crate::primitive::surface;
use crate::{ControlState, SceneCollector, WidgetDesc, WidgetOutput};

/// 滚动视图所在的界面层。
pub const SCROLL_LAYER: u8 = 1;
/// 滚动条宽度（逻辑像素）。
pub const SCROLLBAR_WIDTH: f32 = 8.0;
/// 滚动条滑块最小长度（逻辑像素）。
pub const MIN_THUMB: f32 = 24.0;
/// 最后一次滚动后滚动条保持可见的时长（秒）。
pub const SCROLLBAR_HOLD: f32 = 1.0;
/// 保持时间到期后的淡出时长。
pub const SCROLLBAR_FADE: f32 = 0.25;
/// 惯性甩动将释放速度投射到弹簧目标的时长系数。
pub const FLING_PROJECTION: f32 = 0.25;

/// 滚动视图允许滚动的坐标轴。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ScrollAxis {
    #[default]
    Vertical,
    Horizontal,
    Both,
}

impl ScrollAxis {
    fn vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }
}

/// 轨道上的滑块长度；由绘制与拖拽共用，确保两条路径下的滑块行程计算完全一致。
fn thumb_length(track_length: f32, viewport: f32, content: f32) -> f32 {
    let ratio = (viewport / content).clamp(0.0, 1.0);
    (track_length * ratio).max(MIN_THUMB).min(track_length)
}

/// 滚动视口的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollViewDesc {
    /// 视口矩形。
    pub rect: Rect,
    /// 完整内容尺寸。
    pub content_size: Vec2,
    /// 允许滚动的坐标轴。
    pub axis: ScrollAxis,
    /// 绘制顺序键。
    pub z: u16,
    /// 视口裁剪的圆角半径。
    pub radius: f32,
    /// 滚动条宽度。
    pub scrollbar_width: f32,
}

impl ScrollViewDesc {
    #[must_use]
    pub fn new(rect: Rect, content_size: Vec2) -> Self {
        Self {
            rect,
            content_size,
            axis: ScrollAxis::Vertical,
            z: 700,
            radius: 0.0,
            scrollbar_width: SCROLLBAR_WIDTH,
        }
    }

    /// 仍能使内容覆盖视口的最大偏移量。
    #[must_use]
    pub fn max_offset(&self) -> Vec2 {
        Vec2::new(
            (self.content_size.x - self.rect.width()).max(0.0),
            (self.content_size.y - self.rect.height()).max(0.0),
        )
    }

    /// 将提议值夹紧后的合法偏移量。
    #[must_use]
    pub fn clamp_offset(&self, offset: Vec2) -> Vec2 {
        let max = self.max_offset();
        Vec2::new(
            if self.axis.horizontal() {
                offset.x.clamp(0.0, max.x)
            } else {
                0.0
            },
            if self.axis.vertical() {
                offset.y.clamp(0.0, max.y)
            } else {
                0.0
            },
        )
    }

    /// 单轴上的滚动条轨道矩形。
    #[must_use]
    pub fn track_rect(&self, horizontal: bool) -> Rect {
        if horizontal {
            Rect::new(
                self.rect.origin.x,
                self.rect.max().y - self.scrollbar_width - 2.0,
                self.rect.width() - self.scrollbar_width - 4.0,
                self.scrollbar_width,
            )
        } else {
            Rect::new(
                self.rect.max().x - self.scrollbar_width - 2.0,
                self.rect.origin.y,
                self.scrollbar_width,
                self.rect.height() - self.scrollbar_width - 4.0,
            )
        }
    }

    /// 给定偏移量对应的滚动条滑块矩形。
    #[must_use]
    pub fn thumb_rect(&self, offset: Vec2, horizontal: bool) -> Rect {
        let track = self.track_rect(horizontal);
        let (viewport, content, position) = if horizontal {
            (self.rect.width(), self.content_size.x, offset.x)
        } else {
            (self.rect.height(), self.content_size.y, offset.y)
        };
        let track_length = if horizontal {
            track.width()
        } else {
            track.height()
        };
        if content <= viewport || track_length <= 0.0 {
            return track;
        }
        let thumb = thumb_length(track_length, viewport, content);
        let max_scroll = (content - viewport).max(0.0);
        let progress = if max_scroll > 0.0 {
            (position / max_scroll).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let travel = track_length - thumb;
        if horizontal {
            Rect::new(
                track.origin.x + travel * progress,
                track.origin.y,
                thumb,
                track.height(),
            )
        } else {
            Rect::new(
                track.origin.x,
                track.origin.y + travel * progress,
                track.width(),
                thumb,
            )
        }
    }
}

/// 正在被拖拽的滚动条方向。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

impl ScrollbarAxis {
    fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal)
    }
}

/// 路由至滚动视图滚动条的指针手势。
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ScrollPointerEvent {
    /// 在屏幕空间点处按下。
    Press(Vec2),
    /// 捕获滑块后指针移动。
    Drag(Vec2),
    /// 指针释放。
    Release,
}

/// 单次滚动条拖拽状态。
#[derive(Copy, Clone, Debug)]
struct Drag {
    axis: ScrollbarAxis,
    start_pointer: f32,
    start_offset: Vec2,
}

/// 实时滚动偏移量及其弹簧状态。
#[derive(Clone, Debug)]
pub struct ScrollState {
    spring: Spring2,
    offset: Vec2,
    drag: Option<Drag>,
    bar_opacity: f32,
    idle: f32,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            spring: Spring2::new(SpringPreset::Precise, Vec2::ZERO),
            offset: Vec2::ZERO,
            drag: None,
            bar_opacity: 0.0,
            idle: 0.0,
        }
    }

    /// 当前视觉偏移量。
    #[must_use]
    pub fn offset(&self) -> Vec2 {
        self.offset
    }

    /// 弹簧正在趋向的目标偏移量。
    #[must_use]
    pub fn target(&self) -> Vec2 {
        Vec2::new(self.spring.x.target, self.spring.y.target)
    }

    /// 弹簧是否仍在运动。
    #[must_use]
    pub fn is_animating(&self) -> bool {
        !self.spring.is_settled()
    }

    /// 无动画直接跳转到指定偏移量。
    pub fn jump_to(&mut self, desc: &ScrollViewDesc, offset: Vec2) {
        let offset = desc.clamp_offset(offset);
        self.spring.snap_to(offset);
        self.offset = offset;
        self.reveal_scrollbar();
    }

    /// 累加增量（滚轮格数或拖拽）并夹紧。
    pub fn scroll_by(&mut self, desc: &ScrollViewDesc, delta: Vec2) {
        let target = desc.clamp_offset(self.target() + delta);
        self.spring.set_target(target);
        self.reveal_scrollbar();
    }

    /// 滚动到绝对偏移量并夹紧。
    pub fn scroll_to(&mut self, desc: &ScrollViewDesc, offset: Vec2) {
        let target = desc.clamp_offset(offset);
        self.spring.set_target(target);
        self.reveal_scrollbar();
    }

    /// 滚动条不透明度（`0..=1`）。
    #[must_use]
    pub fn scrollbar_opacity(&self) -> f32 {
        self.bar_opacity
    }

    /// 立即显示滚动条（用于滚轮、悬停或拖拽事件）。
    pub fn reveal_scrollbar(&mut self) {
        self.bar_opacity = 1.0;
        self.idle = 0.0;
    }

    /// 推进滚动条淡出动画。
    ///
    /// 当指针位于视图上方、正在拖拽或弹簧仍在运动时，滚动条保持完全不透明。
    /// 之后保持 [`SCROLLBAR_HOLD`] 时长并在 [`SCROLLBAR_FADE`] 内淡出。
    ///
    /// 返回不透明度是否发生变化。
    pub fn tick_scrollbar(&mut self, dt: f32, hovering: bool) -> bool {
        let before = self.bar_opacity;
        let active = hovering || self.drag.is_some() || self.is_animating();
        if active {
            self.reveal_scrollbar();
        } else {
            self.idle += dt.max(0.0);
            if self.idle > SCROLLBAR_HOLD {
                self.bar_opacity = (self.bar_opacity - dt.max(0.0) / SCROLLBAR_FADE).max(0.0);
            }
        }
        (self.bar_opacity - before).abs() > f32::EPSILON
    }

    /// 推进弹簧物理步进。仍在运动时返回 `true`。
    ///
    /// 每步都会夹紧视觉偏移：猛烈甩动时弹簧可能过冲一两像素，
    /// 而内容绝不应在首行上方露出一道空隙。
    pub fn step(&mut self, desc: &ScrollViewDesc, dt: f32) -> bool {
        let moving = self.spring.step(dt);
        self.offset = desc.clamp_offset(self.spring.value());
        moving
    }

    /// 开始拖拽滚动条滑块。
    pub fn begin_drag(&mut self, axis: ScrollbarAxis, pointer: f32) {
        self.drag = Some(Drag {
            axis,
            start_pointer: pointer,
            start_offset: self.offset,
        });
        self.reveal_scrollbar();
    }

    /// 更新滚动条拖拽。
    ///
    /// 滑块在轨道空间移动，内容在内容空间按相同比例移动，使滑块始终紧跟指针。
    pub fn drag_to(&mut self, desc: &ScrollViewDesc, pointer: f32) {
        let Some(drag) = self.drag else {
            return;
        };
        let horizontal = drag.axis.horizontal();
        let track = desc.track_rect(horizontal);
        let track_length = if horizontal {
            track.width()
        } else {
            track.height()
        };
        let content = if horizontal {
            desc.content_size.x
        } else {
            desc.content_size.y
        };
        let viewport = if horizontal {
            desc.rect.width()
        } else {
            desc.rect.height()
        };
        let max_scroll = (content - viewport).max(0.0);
        if track_length <= 0.0 || max_scroll <= 0.0 {
            return;
        }
        // 滑块实际行程仅为 `track_length - thumb_length` 而非整条轨道：
        // 若除以整条轨道长度会导致滑块滞后于指针且无法拉到底部。
        // 此处镜像 `thumb_rect` 的尺寸计算，使滑块紧贴指针并精确抵达 `max_scroll`。
        //
        let thumb_length = thumb_length(track_length, viewport, content);
        let travel = (track_length - thumb_length).max(1.0);
        let ratio = max_scroll / travel;
        let delta = (pointer - drag.start_pointer) * ratio;
        let mut offset = drag.start_offset;
        if horizontal {
            offset.x += delta;
        } else {
            offset.y += delta;
        }
        self.jump_to(desc, offset);
    }

    /// 结束滚动条拖拽。
    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    /// 是否正在拖拽滚动条。
    #[must_use]
    pub fn dragging(&self) -> Option<ScrollbarAxis> {
        self.drag.map(|drag| drag.axis)
    }

    /// 滚动条按下点隐含的目标偏移量（点击轨道将跳转）。
    pub fn jump_from_track(&mut self, desc: &ScrollViewDesc, pointer: Vec2) {
        let horizontal = desc.axis.horizontal()
            && pointer.x >= desc.rect.origin.x
            && pointer.x <= desc.rect.max().x;
        let vertical = desc.axis.vertical()
            && pointer.y >= desc.rect.origin.y
            && pointer.y <= desc.rect.max().y;
        if horizontal {
            let track = desc.track_rect(true);
            if track.contains_point(pointer) {
                let max_scroll = (desc.content_size.x - desc.rect.width()).max(0.0);
                let progress =
                    ((pointer.x - track.origin.x) / track.width().max(1.0)).clamp(0.0, 1.0);
                self.scroll_to(desc, Vec2::new(max_scroll * progress, self.target().y));
                return;
            }
        }
        if vertical {
            let track = desc.track_rect(false);
            if track.contains_point(pointer) {
                let max_scroll = (desc.content_size.y - desc.rect.height()).max(0.0);
                let progress =
                    ((pointer.y - track.origin.y) / track.height().max(1.0)).clamp(0.0, 1.0);
                self.scroll_to(desc, Vec2::new(self.target().x, max_scroll * progress));
            }
        }
    }

    /// 当点击滑块外部的滚动条**轨道**时，按一页视口大小翻页。
    ///
    /// 点击滑块上方/左侧向前翻页，点击下方/右侧向后翻页。返回是否触发了翻页。
    pub fn page_at_track_click(&mut self, desc: &ScrollViewDesc, click_pos: Vec2) -> bool {
        let horizontal = desc.axis.horizontal() && desc.track_rect(true).contains_point(click_pos);
        let vertical = desc.axis.vertical() && desc.track_rect(false).contains_point(click_pos);
        let (horizontal, thumb) = if horizontal {
            (true, desc.thumb_rect(self.offset(), true))
        } else if vertical {
            (false, desc.thumb_rect(self.offset(), false))
        } else {
            return false;
        };
        if thumb.contains_point(click_pos) {
            return false;
        }
        let viewport = if horizontal {
            desc.rect.width()
        } else {
            desc.rect.height()
        };
        let direction = if horizontal {
            if click_pos.x < thumb.origin.x {
                -1.0
            } else {
                1.0
            }
        } else if click_pos.y < thumb.origin.y {
            -1.0
        } else {
            1.0
        };
        let mut target = self.target();
        if horizontal {
            target.x += direction * viewport;
        } else {
            target.y += direction * viewport;
        }
        self.scroll_to(desc, target);
        true
    }

    /// 滚动最小距离以使 `child_rect` 可见。
    ///
    /// `child_rect` 处于内容坐标系：原点在内容左上角，偏移 `o` 对应显示窗口 `[o, o + viewport]`。
    /// 此处调整弹簧目标而非直接修改视觉偏移，使视口外获焦的子元素平滑动画滚入。
    /// 返回目标是否发生移动。
    pub fn scroll_into_view(&mut self, desc: &ScrollViewDesc, child_rect: Rect) -> bool {
        let before = self.target();
        let mut target = before;
        if desc.axis.vertical() {
            let height = desc.rect.height();
            if child_rect.origin.y < target.y {
                target.y = child_rect.origin.y;
            } else if child_rect.max().y > target.y + height {
                target.y = child_rect.max().y - height;
            }
        }
        if desc.axis.horizontal() {
            let width = desc.rect.width();
            if child_rect.origin.x < target.x {
                target.x = child_rect.origin.x;
            } else if child_rect.max().x > target.x + width {
                target.x = child_rect.max().x - width;
            }
        }
        if target == before {
            return false;
        }
        self.scroll_to(desc, target);
        true
    }

    /// 将释放时的惯性速度投射到弹簧目标。
    ///
    /// `velocity` 单位为逻辑像素/秒；按 [`FLING_PROJECTION`] 秒外推并夹紧到内容边界，
    /// 使快速轻扫先滑行一段距离，随后精确停在内容末尾而不会越界。
    pub fn fling(&mut self, desc: &ScrollViewDesc, velocity: Vec2) {
        self.scroll_to(desc, self.offset() + velocity * FLING_PROJECTION);
    }

    /// 将指针手势归约为滚动条状态变化。
    ///
    /// 按下滑块将其就地捕获；按下轨道翻动一页；拖拽移动已捕获的滑块；释放结束拖拽。
    /// 返回偏移量或捕获状态是否变化。这是宿主接入指针事件的唯一入口，
    /// 确保滑块抓取、轨道点击与释放清理不会各自漂移。
    pub fn reduce_pointer(&mut self, desc: &ScrollViewDesc, event: ScrollPointerEvent) -> bool {
        match event {
            ScrollPointerEvent::Press(pos) => {
                if desc.axis.vertical() && desc.content_size.y > desc.rect.height() {
                    let thumb = desc.thumb_rect(self.offset(), false);
                    if thumb.contains_point(pos) {
                        self.begin_drag(ScrollbarAxis::Vertical, pos.y);
                        return true;
                    }
                }
                if desc.axis.horizontal() && desc.content_size.x > desc.rect.width() {
                    let thumb = desc.thumb_rect(self.offset(), true);
                    if thumb.contains_point(pos) {
                        self.begin_drag(ScrollbarAxis::Horizontal, pos.x);
                        return true;
                    }
                }
                self.page_at_track_click(desc, pos)
            }
            ScrollPointerEvent::Drag(pos) => {
                let Some(axis) = self.dragging() else {
                    return false;
                };
                let pointer = if axis.horizontal() { pos.x } else { pos.y };
                self.drag_to(desc, pointer);
                true
            }
            ScrollPointerEvent::Release => {
                let was_dragging = self.drag.is_some();
                self.end_drag();
                was_dragging
            }
        }
    }
}

impl ScrollViewOutput {
    /// 在视口裁剪与滚动平移作用域内收集子元素。
    ///
    /// 这是 [`emit_scroll_view`] 与 [`SceneCollector::push_clip`] / [`SceneCollector::push_transform`] 的桥梁：
    /// 闭包推入的每个图元都会继承这两者，且即使闭包未输出任何图元也会正确弹出作用域。
    pub fn collect_children<F>(&self, collector: &mut SceneCollector, viewport: Rect, children: F)
    where
        F: FnOnce(&mut SceneCollector),
    {
        collector.push_clip(self.clip, viewport);
        collector.push_transform(self.transform);
        children(collector);
        collector.pop_transform();
        collector.pop_clip();
    }

    /// 向收集器注册所有滚动条滑块的命中区域。
    ///
    /// [`SceneCollector::push`] 仅收集控件自身的单个 `hit`，
    /// 因此位于 [`WidgetOutput`] 之外的滑块若不注册，每帧都会被静默丢弃而无法拖拽。
    pub fn register_hits(&self, collector: &mut SceneCollector) {
        for hit in &self.hits {
            collector.push_hit(hit.clone());
        }
    }

    /// 一次性推入滚动视图自身输出**以及**滑块命中区域，防止调用方遗漏滑块。
    pub fn push_into(self, collector: &mut SceneCollector, key: u64) {
        self.register_hits(collector);
        collector.push(key, self.output);
    }
}

/// 滚动视图的输出结果。
///
/// 裁剪与变换与 [`WidgetOutput`] 分离，因为它们作用于**子元素**；
/// 滚动视图自身仅绘制滚动条。
#[derive(Clone, Debug)]
pub struct ScrollViewOutput {
    /// 滚动条及滚轮命中区域。
    pub output: WidgetOutput,
    /// 所有子图元必须引用的裁剪区域。
    pub clip: GpuClipNode,
    /// 所有子图元必须引用的平移变换。
    pub transform: GpuTransform,
    /// 滚动条滑块命中区域（先垂直后水平，每个可见滚动条各一个）。
    /// 请通过 [`Self::register_hits`] 注册，以便路由器拾取并拖拽滑块。
    pub hits: Vec<HitRegion>,
}

/// 绑定了节点标识的滚动视图描述符。
#[derive(Clone, Debug)]
pub struct ScrollViewWidget {
    /// 控件标识。
    pub node: u64,
    /// 描述符。
    pub desc: ScrollViewDesc,
    /// 实时偏移量。
    pub state: ScrollState,
}

impl WidgetDesc for ScrollViewWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_scroll_view(self.node, &self.desc, &self.state, state).output
    }
}

/// 输出滚动条、裁剪区域与子元素变换。
#[must_use]
pub fn emit_scroll_view(
    node: u64,
    desc: &ScrollViewDesc,
    scroll: &ScrollState,
    state: ControlState,
) -> ScrollViewOutput {
    let rect = desc.rect;
    let offset = scroll.offset();
    let mut primitives: Vec<GpuPrimitiveInstance> = Vec::with_capacity(4);

    let track_token = if state.disabled {
        Token::DISABLED
    } else {
        Token::CONTROL_TRACK
    };
    let thumb_token = if state.phase == crate::PointerPhase::Hovered || scroll.dragging().is_some()
    {
        Token::CONTROL_THUMB
    } else {
        Token::OUTLINE_STRONG
    };

    let mut hits: Vec<HitRegion> = Vec::with_capacity(2);
    let opacity = scroll.scrollbar_opacity();
    if desc.axis.vertical() && desc.content_size.y > rect.height() {
        let track = desc.track_rect(false);
        let mut track_primitive = surface(track, desc.scrollbar_width * 0.5, track_token, desc.z);
        set_opacity(&mut track_primitive, opacity);
        primitives.push(track_primitive);
        let thumb = desc.thumb_rect(offset, false);
        let mut thumb_primitive = surface(thumb, desc.scrollbar_width * 0.5, thumb_token, desc.z);
        set_opacity(&mut thumb_primitive, opacity);
        primitives.push(thumb_primitive);
        hits.push(
            HitRegion::new(node ^ (1 << 63), SCROLL_LAYER, u32::from(desc.z) + 1, thumb)
                .with_cursor(CursorKind::Pointer),
        );
    }
    if desc.axis.horizontal() && desc.content_size.x > rect.width() {
        let track = desc.track_rect(true);
        let mut track_primitive = surface(track, desc.scrollbar_width * 0.5, track_token, desc.z);
        set_opacity(&mut track_primitive, opacity);
        primitives.push(track_primitive);
        let thumb = desc.thumb_rect(offset, true);
        let mut thumb_primitive = surface(thumb, desc.scrollbar_width * 0.5, thumb_token, desc.z);
        set_opacity(&mut thumb_primitive, opacity);
        primitives.push(thumb_primitive);
        hits.push(
            HitRegion::new(node ^ (1 << 62), SCROLL_LAYER, u32::from(desc.z) + 1, thumb)
                .with_cursor(CursorKind::Pointer),
        );
    }

    let hit = Some(HitRegion::new(node, SCROLL_LAYER, u32::from(desc.z), rect));
    let mut a11y =
        unbound_a11y::A11yNode::new(accesskit::Role::ScrollView, "", rect).disabled(state.disabled);
    a11y.actions
        .insert(unbound_a11y::A11yAction::ScrollIntoView);
    a11y.actions.insert(unbound_a11y::A11yAction::Focus);

    let clip = GpuClipNode::from_aabb([rect.min().x, rect.min().y, rect.max().x, rect.max().y]);
    let mut transform = GpuTransform::IDENTITY;
    transform.translation = [-offset.x, -offset.y];
    if offset != Vec2::ZERO {
        transform.flags = unbound_gpu::data::TransformFlags::ANIMATED.bits();
    }

    ScrollViewOutput {
        output: WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        clip,
        transform,
        hits,
    }
}

/// 单帧之后虚拟列表窗口的状态。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct VirtualWindow {
    /// 来自 [`VirtualList::update`] 的槽位/绑定统计。
    pub update: unbound_world::virtualize::WindowUpdate,
    /// **整个**列表的变换（含亚行偏移）。
    pub transform: GpuTransform,
    /// 当前已实例化的首个条目索引。
    pub first_item: usize,
    /// 当前存活的槽位数量。
    pub slots: usize,
}

/// 根据滚动视图更新虚拟列表。
///
/// 亚行滚动仅修改 `transform`：`update.rebound == 0` 且不重新绑定数据，
/// 这正是实现“10 万行 @ 120 FPS”的关键路径。
#[must_use]
pub fn update_virtual_list(
    list: &mut VirtualList,
    desc: &ScrollViewDesc,
    scroll: &ScrollState,
) -> VirtualWindow {
    let offset = scroll.offset();
    let update = list.update(offset.y, desc.rect.height(), list_total(list, desc));
    let mut transform = GpuTransform::IDENTITY;
    // `VirtualList` 将亚行余量报告为 `offset`，但调用方定位槽位时还需要整行偏移。
    //
    transform.translation = [-offset.x, -offset.y];
    VirtualWindow {
        update,
        transform,
        first_item: list.first_item(),
        slots: list.slot_count(),
    }
}

/// 列表当前报告的总行数。
///
/// `VirtualList` 维护自身的 `total`，而滚动描述符持有内容总高度。
/// 通过行高反向换算可保持单一数据源（描述符），无需重复维护条目数。
fn list_total(list: &VirtualList, desc: &ScrollViewDesc) -> usize {
    let rows = (desc.content_size.y / list.row_height()).round();
    if rows.is_finite() && rows >= 0.0 {
        rows as usize
    } else {
        0
    }
}

fn set_opacity(primitive: &mut GpuPrimitiveInstance, opacity: f32) {
    primitive.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(primitive.depth_u16(), opacity);
}

/// 虚拟列表中单行在视口空间的位置。
#[must_use]
pub fn virtual_row_rect(
    desc: &ScrollViewDesc,
    scroll: &ScrollState,
    row_index: usize,
    row_height: f32,
) -> Rect {
    Rect::new(
        desc.rect.origin.x,
        desc.rect.origin.y + row_index as f32 * row_height - scroll.offset().y,
        desc.rect.width(),
        row_height,
    )
}

/// 将滚动条滑块绘制为独立图元（供测试与工具使用）。
#[must_use]
pub fn scrollbar_thumb_primitive(
    desc: &ScrollViewDesc,
    offset: Vec2,
    horizontal: bool,
) -> GpuPrimitiveInstance {
    let mut primitive = surface(
        desc.thumb_rect(offset, horizontal),
        desc.scrollbar_width * 0.5,
        Token::CONTROL_THUMB,
        desc.z,
    );
    primitive.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Background as u16,
    );
    primitive
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ScrollViewDesc {
        ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(400.0, 3000.0))
    }

    #[test]
    fn wheel_scrolling_is_clamped_to_the_content() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.scroll_by(&desc, Vec2::new(0.0, 120.0));
        state.step(&desc, 1.0 / 60.0);
        assert!(state.offset().y > 0.0);

        state.scroll_by(&desc, Vec2::new(0.0, 1_000_000.0));
        assert_eq!(state.target().y, 2700.0);
        for _ in 0..240 {
            if !state.step(&desc, 1.0 / 60.0) {
                break;
            }
        }
        assert!(
            (state.offset().y - 2700.0).abs() < 0.5,
            "{}",
            state.offset().y
        );

        state.scroll_by(&desc, Vec2::new(0.0, -1_000_000.0));
        for _ in 0..240 {
            if !state.step(&desc, 1.0 / 60.0) {
                break;
            }
        }
        assert_eq!(state.offset().y, 0.0);
    }

    #[test]
    fn a_vertical_only_view_never_scrolls_horizontally() {
        let desc = ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(800.0, 3000.0));
        let mut state = ScrollState::new();
        state.scroll_by(&desc, Vec2::new(200.0, 100.0));
        state.step(&desc, 1.0 / 60.0);
        assert_eq!(state.offset().x, 0.0);
        assert!(state.offset().y > 0.0);
    }

    #[test]
    fn the_thumb_ratio_tracks_the_viewport() {
        let desc = ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(400.0, 600.0));
        let track = desc.track_rect(false);
        let thumb = desc.thumb_rect(Vec2::ZERO, false);
        assert!((thumb.height() - track.height() * 0.5).abs() < 1e-3);

        assert_eq!(
            desc.thumb_rect(Vec2::new(0.0, 300.0), false).max().y,
            track.max().y
        );
        assert_eq!(
            desc.thumb_rect(Vec2::new(0.0, 300.0), false).height(),
            thumb.height()
        );
    }

    #[test]
    fn the_thumb_is_never_smaller_than_the_minimum() {
        let desc = ScrollViewDesc::new(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            Vec2::new(400.0, 100_000.0),
        );
        assert!(desc.thumb_rect(Vec2::ZERO, false).height() >= MIN_THUMB);
    }

    #[test]
    fn dragging_the_thumb_moves_the_content_by_the_track_ratio() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.begin_drag(ScrollbarAxis::Vertical, 10.0);
        assert_eq!(state.dragging(), Some(ScrollbarAxis::Vertical));
        state.drag_to(&desc, 110.0);
        assert!(state.offset().y > 0.0);
        state.end_drag();
        assert!(state.dragging().is_none());
    }

    #[test]
    fn a_thumb_drag_is_clamped_at_both_ends() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.begin_drag(ScrollbarAxis::Vertical, 100.0);
        state.drag_to(&desc, 100_000.0);
        assert_eq!(state.offset().y, 2700.0);
        state.drag_to(&desc, -100_000.0);
        assert_eq!(state.offset().y, 0.0);
    }

    #[test]
    fn clicking_the_track_jumps_towards_the_click() {
        let desc = desc();
        let mut state = ScrollState::new();
        let track = desc.track_rect(false);
        state.jump_from_track(
            &desc,
            Vec2::new(track.center().x, track.origin.y + track.height() * 0.5),
        );
        assert!(state.target().y > 1000.0);
    }

    #[test]
    fn the_scrollbar_fades_after_the_hold_expires() {
        let desc = desc();
        let mut state = ScrollState::new();
        assert_eq!(state.scrollbar_opacity(), 0.0);

        state.scroll_by(&desc, Vec2::new(0.0, 60.0));
        assert_eq!(state.scrollbar_opacity(), 1.0);

        // 弹簧仍在运动，因此滚动条保持可见。
        state.tick_scrollbar(0.016, false);
        assert_eq!(state.scrollbar_opacity(), 1.0);

        // 立即跳转稳定，随后保持一秒。
        state.jump_to(&desc, Vec2::new(0.0, 120.0));
        assert_eq!(state.scrollbar_opacity(), 1.0);
        state.tick_scrollbar(SCROLLBAR_HOLD + 0.01, false);
        assert!(state.scrollbar_opacity() < 1.0);
        state.tick_scrollbar(SCROLLBAR_FADE, false);
        assert_eq!(state.scrollbar_opacity(), 0.0);
    }

    #[test]
    fn hovering_reveals_the_scrollbar_without_scrolling() {
        let mut state = ScrollState::new();
        assert_eq!(state.scrollbar_opacity(), 0.0);
        assert!(state.tick_scrollbar(0.016, true));
        assert_eq!(state.scrollbar_opacity(), 1.0);
    }

    #[test]
    fn scrolling_reveals_the_bar_and_the_emitter_writes_its_opacity() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.jump_to(&desc, Vec2::new(0.0, 60.0));
        let output = emit_scroll_view(9, &desc, &state, ControlState::default());
        assert_eq!(output.output.primitives.len(), 2, "track and thumb");
        assert!(output
            .output
            .primitives
            .iter()
            .all(|primitive| primitive.opacity() > 0.99));
    }

    #[test]
    fn the_clip_covers_the_viewport_and_the_transform_owns_the_offset() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.jump_to(&desc, Vec2::new(0.0, 120.0));
        let output = emit_scroll_view(7, &desc, &state, ControlState::default());
        assert_eq!(output.clip.aabb_clip, [0.0, 0.0, 400.0, 300.0]);
        assert_eq!(output.transform.translation, [0.0, -120.0]);
        assert!(output
            .output
            .hit
            .as_ref()
            .is_some_and(|hit| hit.bounds.contains_rect(&desc.rect)));
    }

    #[test]
    fn a_sub_row_scroll_rebinds_nothing() {
        let desc = desc();
        let mut state = ScrollState::new();
        let mut list = VirtualList::new(28.0, 1.0);
        let first = update_virtual_list(&mut list, &desc, &state);
        assert!(first.slots > 0);

        state.jump_to(&desc, Vec2::new(0.0, 7.5));
        let second = update_virtual_list(&mut list, &desc, &state);
        assert!(second.update.offset_only, "{:?}", second.update);
        assert_eq!(second.update.rebound, 0);
        assert_eq!(second.update.offset, -7.5);
        assert_eq!(second.transform.translation, [0.0, -7.5]);
    }

    #[test]
    fn a_virtual_list_keeps_its_entity_budget_across_a_long_scroll() {
        let desc = ScrollViewDesc::new(
            Rect::new(0.0, 0.0, 400.0, 800.0),
            Vec2::new(400.0, 28.0 * 100_000.0),
        );
        let mut state = ScrollState::new();
        let mut list = VirtualList::new(28.0, 1.0);
        let _ = update_virtual_list(&mut list, &desc, &state);
        let budget = list.slot_count();
        for offset in [28.0, 2_800.0, 280_000.0, 2_800_000.0] {
            state.jump_to(&desc, Vec2::new(0.0, offset));
            let _ = update_virtual_list(&mut list, &desc, &state);
            assert_eq!(list.slot_count(), budget);
        }
        assert!((28..=120).contains(&budget), "{budget}");
    }

    #[test]
    fn dragging_thumb_across_travel_reaches_exact_max_scroll() {
        let desc = desc();
        let track = desc.track_rect(false);
        let thumb = desc.thumb_rect(Vec2::ZERO, false);
        let travel = track.height() - thumb.height();
        let mut state = ScrollState::new();
        let start = track.origin.y + thumb.height() * 0.5;
        state.begin_drag(ScrollbarAxis::Vertical, start);
        state.drag_to(&desc, start + travel);
        assert!(
            (state.offset().y - desc.max_offset().y).abs() < 1e-3,
            "{} should equal {}",
            state.offset().y,
            desc.max_offset().y
        );
    }

    #[test]
    fn both_scrollbar_thumbs_are_pickable_and_registered() {
        let mut desc =
            ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(800.0, 3000.0));
        desc.axis = ScrollAxis::Both;
        let state = ScrollState::new();
        let output = emit_scroll_view(90, &desc, &state, ControlState::default());
        assert_eq!(output.hits.len(), 2, "vertical and horizontal thumbs");

        let mut collector = SceneCollector::new(1);
        output.push_into(&mut collector, 90);
        assert_eq!(collector.regions.len(), 3, "viewport hit plus two thumbs");
        assert!(collector.regions.iter().any(|r| r.node == 90 ^ (1 << 63)));
        assert!(collector.regions.iter().any(|r| r.node == 90 ^ (1 << 62)));
    }

    #[test]
    fn reduce_pointer_grabs_the_thumb_and_drags_to_the_end() {
        let desc = desc();
        let mut state = ScrollState::new();
        let thumb = desc.thumb_rect(Vec2::ZERO, false);
        let press = thumb.center();
        assert!(state.reduce_pointer(&desc, ScrollPointerEvent::Press(press)));
        assert_eq!(state.dragging(), Some(ScrollbarAxis::Vertical));

        let track = desc.track_rect(false);
        let travel = track.height() - thumb.height();
        assert!(state.reduce_pointer(
            &desc,
            ScrollPointerEvent::Drag(Vec2::new(press.x, press.y + travel))
        ));
        assert!((state.offset().y - desc.max_offset().y).abs() < 1e-3);
        assert!(state.reduce_pointer(&desc, ScrollPointerEvent::Release));
        assert!(state.dragging().is_none());
    }

    #[test]
    fn reduce_pointer_pages_on_a_track_click() {
        let desc = desc();
        let mut state = ScrollState::new();
        let track = desc.track_rect(false);
        let below = Vec2::new(track.center().x, track.max().y - 2.0);
        assert!(state.reduce_pointer(&desc, ScrollPointerEvent::Press(below)));
        assert_eq!(state.target().y, desc.rect.height());
        assert!(state.dragging().is_none(), "a track click does not capture");
    }
}
