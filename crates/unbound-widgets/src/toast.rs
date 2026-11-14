//! 角落通知堆栈。
//!
//! Toast 与警告横幅相反：它是瞬态的、悬浮于页面之上且必须自动消失。
//! 因此堆栈持有两个无状态横幅不需要的时钟：
//!
//!
//! * 每个条目的**倒计时**——当指针停留在堆栈任意位置时暂停（阅读消息绝不应变成与计时器赛跑）；
//!
//! * 每个条目的**弹簧**——用于滑入与淡入淡出动画。
//!
//!
//! 控件保持纯函数特性：[`ToastStack::tick`] 推进时钟并返回过期的 ID，
//! [`emit_toast`] 则仅根据弹簧当前报告的值进行绘制。

use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, PORTAL_LAYER};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::alert::Severity;
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetOutput};

use accesskit::Role;

pub const TOAST_DURATION_MS: f32 = 4000.0;
/// Toast 在屏幕上停留的默认时长。
pub const TOAST_SLIDE_IN: f32 = 16.0;
/// 新 Toast 向上滑入的起始距离。
pub const TOAST_CLOSE_SIZE: f32 = 18.0;
/// 关闭按钮正方形尺寸。
pub const TOAST_ACTION_SIZE: (f32, f32) = (76.0, 24.0);
/// 操作按钮尺寸。
pub const TOAST_RADIUS: f32 = 8.0;
const TOAST_SHADOW: f32 = 18.0;

/// Toast 圆角半径。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ToastSpring {
    /// 标量的临界阻尼弹簧。
    pub value: f32,
    /// 当前值。
    pub velocity: f32,
    /// 当前速度。
    pub target: f32,
}

impl ToastSpring {
    /// 弹簧趋向的静止目标值。
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self {
            value,
            velocity: 0.0,
            target: value,
        }
    }

    /// 在 `value` 处静止的弹簧。
    pub fn step(&mut self, dt: f32) -> bool {
        if dt <= 0.0 {
            return false;
        }
        let dt = dt.min(0.05);
        let before = self.value;
        let force = (self.target - self.value) * 260.0;
        self.velocity = (self.velocity + force * dt) * (-26.0 * dt).exp();
        self.value += self.velocity * dt;
        if (self.target - self.value).abs() < 0.001 && self.velocity.abs() < 0.01 {
            self.value = self.target;
            self.velocity = 0.0;
        }
        (self.value - before).abs() > f32::EPSILON
    }

    /// 推进弹簧步进；返回数值是否发生了移动。
    #[must_use]
    pub fn settled(&self) -> bool {
        (self.target - self.value).abs() < 0.001 && self.velocity.abs() < 0.01
    }
}

/// 弹簧是否已静止在目标值上。
#[derive(Clone, Debug, PartialEq)]
pub struct ToastEntry {
    /// 堆栈中的单条通知。
    pub id: u64,
    /// 稳定标识。
    pub title: String,
    /// 标题。
    pub message: String,
    /// 正文文本。
    pub severity: Severity,
    /// 严重级别分类。
    pub duration_ms: f32,
    /// 总生命周期；`0.0` 表示常驻不自动消失。
    pub remaining_ms: f32,
    /// 剩余生命周期。
    pub action_label: Option<String>,
    /// 可选的操作按钮标签。
    pub y_spring: ToastSpring,
    /// 滑入偏移量。
    pub opacity_spring: ToastSpring,
}

impl ToastEntry {
    /// 淡入淡出不透明度。
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>, severity: Severity) -> Self {
        Self {
            id,
            title: title.into(),
            message: String::new(),
            severity,
            duration_ms: TOAST_DURATION_MS,
            remaining_ms: TOAST_DURATION_MS,
            action_label: None,
            y_spring: ToastSpring {
                value: TOAST_SLIDE_IN,
                velocity: 0.0,
                target: 0.0,
            },
            opacity_spring: ToastSpring {
                value: 0.0,
                velocity: 0.0,
                target: 1.0,
            },
        }
    }

    /// 创建从下方淡入的 Toast。
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// 附加正文文本。
    #[must_use]
    pub fn with_action(mut self, label: impl Into<String>) -> Self {
        self.action_label = Some(label.into());
        self
    }

    /// 附加操作按钮。
    #[must_use]
    pub fn with_duration(mut self, duration_ms: f32) -> Self {
        self.duration_ms = duration_ms.max(0.0);
        self.remaining_ms = self.duration_ms;
        self
    }

    /// 设置显式生命周期。
    #[must_use]
    pub fn persistent(mut self) -> Self {
        self.duration_ms = 0.0;
        self.remaining_ms = 0.0;
        self
    }

    /// 创建永不自动过期的常驻 Toast。
    #[must_use]
    pub fn is_persistent(&self) -> bool {
        self.duration_ms <= 0.0
    }

    /// 该 Toast 是否忽略倒计时。
    #[must_use]
    pub fn progress(&self) -> f32 {
        if self.is_persistent() {
            return 0.0;
        }
        (1.0 - self.remaining_ms / self.duration_ms).clamp(0.0, 1.0)
    }

    /// 生命周期已流逝的比例（`0.0..=1.0`）。
    pub fn tick(&mut self, dt_ms: f32) -> bool {
        if self.is_persistent() || dt_ms <= 0.0 {
            return false;
        }
        self.remaining_ms = (self.remaining_ms - dt_ms).max(0.0);
        self.remaining_ms <= 0.0
    }

    /// 推进倒计时；返回 Toast 是否已过期。
    #[must_use]
    pub fn close_rect(&self, rect: Rect) -> Rect {
        Rect::new(
            rect.max().x - 8.0 - TOAST_CLOSE_SIZE,
            rect.origin.y + 8.0,
            TOAST_CLOSE_SIZE,
            TOAST_CLOSE_SIZE,
        )
    }

    /// 关闭按钮矩形。
    #[must_use]
    pub fn action_rect(&self, rect: Rect) -> Option<Rect> {
        self.action_label.as_ref().map(|_| {
            Rect::new(
                rect.max().x - 8.0 - TOAST_ACTION_SIZE.0,
                rect.max().y - 8.0 - TOAST_ACTION_SIZE.1,
                TOAST_ACTION_SIZE.0,
                TOAST_ACTION_SIZE.1,
            )
        })
    }

    /// 操作按钮矩形（当 Toast 带有操作标签时）。
    #[must_use]
    pub fn content_rect(&self, rect: Rect) -> Rect {
        let left = rect.origin.x + 16.0;
        let right = self.close_rect(rect).origin.x - 6.0;
        Rect::new(
            left,
            rect.origin.y + 10.0,
            (right - left).max(0.0),
            (rect.size.y - 20.0).max(0.0),
        )
    }
}

/// 文本区域矩形（位于强调条内侧且避开按钮）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToastTick {
    /// 单次 [`ToastStack::tick`] 的输出结果。
    pub expired: Vec<u64>,
    /// 已过期并被移除的 ID 列表。
    pub animating: bool,
}

/// 是否仍有弹簧处于运动状态。
#[derive(Clone, Debug, Default)]
pub struct ToastStack {
    entries: Vec<ToastEntry>,
    hovered: bool,
}

impl ToastStack {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 通知堆栈。
    pub fn push(&mut self, entry: ToastEntry) {
        self.entries.push(entry);
    }

    /// 追加一条 Toast。
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        before != self.entries.len()
    }

    /// 按 ID 移除一条 Toast。
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 清空所有 Toast。
    #[must_use]
    pub fn entries(&self) -> &[ToastEntry] {
        &self.entries
    }

    /// 按最旧优先顺序遍历条目。
    #[must_use]
    pub fn entry(&self, id: u64) -> Option<&ToastEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// 按 ID 查找条目。
    #[must_use]
    pub fn paused(&self) -> bool {
        self.hovered
    }

    /// 倒计时是否处于暂停状态。
    pub fn set_hovered(&mut self, hovered: bool) -> bool {
        let changed = self.hovered != hovered;
        self.hovered = hovered;
        changed
    }

    /// 暂停或恢复倒计时；返回标志位是否变化。
    pub fn tick(&mut self, dt_ms: f32) -> ToastTick {
        let dt = dt_ms.max(0.0);
        let dt_s = dt / 1000.0;
        let mut animating = false;
        for entry in &mut self.entries {
            if entry.y_spring.step(dt_s) {
                animating = true;
            }
            if entry.opacity_spring.step(dt_s) {
                animating = true;
            }
            if !entry.y_spring.settled() || !entry.opacity_spring.settled() {
                animating = true;
            }
        }
        let mut expired = Vec::new();
        if !self.hovered {
            for entry in &mut self.entries {
                if entry.tick(dt) {
                    expired.push(entry.id);
                }
            }
        }
        if !expired.is_empty() {
            self.entries.retain(|entry| !expired.contains(&entry.id));
        }
        ToastTick { expired, animating }
    }
}

/// 将所有时钟推进 `dt_ms` 毫秒，并丢弃过期的 Toast。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToastLayout {
    /// 等尺寸 Toast 堆栈的布局位置。
    pub viewport: Rect,
    /// 堆栈保持在内的视口矩形。
    pub width: f32,
    /// Toast 宽度。
    pub height: f32,
    /// Toast 高度。
    pub gap: f32,
    /// 两条 Toast 之间的垂直间距。
    pub margin: f32,
}

impl ToastLayout {
    /// 距离视口角落的边距。
    #[must_use]
    pub fn new(viewport: Rect, width: f32, height: f32) -> Self {
        Self {
            viewport,
            width,
            height,
            gap: 8.0,
            margin: 16.0,
        }
    }

    /// 创建右下角堆栈布局。
    /// 第 `index` 个槽位的矩形（索引 `0` 为最新 Toast，最靠近角落）。
    #[must_use]
    pub fn slot(&self, index: usize) -> Rect {
        let x = (self.viewport.max().x - self.margin - self.width).max(self.viewport.origin.x);
        let y = self.viewport.max().y
            - self.margin
            - self.height
            - index as f32 * (self.height + self.gap);
        Rect::new(x, y.max(self.viewport.origin.y), self.width, self.height)
    }

    /// `count` 条 Toast 的总包围盒。
    #[must_use]
    pub fn stack_rect(&self, count: usize) -> Rect {
        if count == 0 {
            return Rect::new(
                self.viewport.max().x - self.margin,
                self.viewport.max().y - self.margin,
                0.0,
                0.0,
            );
        }
        let first = self.slot(0);
        let last = self.slot(count - 1);
        Rect::new(
            first.origin.x,
            last.origin.y,
            first.size.x,
            (first.max().y - last.origin.y).max(0.0),
        )
    }
}

/// 单条 Toast 的输出结果。
#[derive(Clone, Debug)]
pub struct ToastOutput {
    /// 主体部分：背景、强调条、字形与关闭叉号。
    pub output: WidgetOutput,
    /// 关闭按钮。
    pub close: HitRegion,
    /// 操作按钮（若有）。
    pub action: Option<HitRegion>,
}

/// 输出 Toast 卡片及其命中区域。
#[must_use]
pub fn emit_toast(
    node: u64,
    entry: &ToastEntry,
    rect: Rect,
    toast_glyphs: &[GlyphQuad],
    control: ControlState,
) -> ToastOutput {
    let opacity = entry.opacity_spring.value.clamp(0.0, 1.0);
    let offset_y = entry.y_spring.value;
    let rect = Rect::new(
        rect.origin.x,
        rect.origin.y + offset_y,
        rect.size.x,
        rect.size.y,
    );
    let accent = entry.severity.accent();

    let mut payloads = Vec::with_capacity(2 + toast_glyphs.len());
    let mut primitives_raw = vec![
        instance(
            rect,
            [TOAST_RADIUS; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            TOAST_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            560,
        ),
        surface(rect, TOAST_RADIUS, Token::OVERLAY_BG, 560),
        instance(
            rect,
            [TOAST_RADIUS; 4],
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            560,
        ),
        surface(
            Rect::new(rect.origin.x, rect.origin.y + 10.0, 3.0, rect.size.y - 20.0),
            1.5,
            accent,
            561,
        ),
    ];

    let close = entry.close_rect(rect);
    let inset = 5.0;
    stroke(
        (close.origin.x + inset, close.origin.y + inset),
        (close.max().x - inset, close.max().y - inset),
        1.5,
        Token::TEXT_SECONDARY,
        562,
        &mut primitives_raw,
        &mut payloads,
    );
    stroke(
        (close.max().x - inset, close.origin.y + inset),
        (close.origin.x + inset, close.max().y - inset),
        1.5,
        Token::TEXT_SECONDARY,
        562,
        &mut primitives_raw,
        &mut payloads,
    );

    let mut glyph_primitives = Vec::with_capacity(toast_glyphs.len());
    crate::primitive::glyphs(
        toast_glyphs,
        Token::TEXT_PRIMARY,
        562,
        &mut glyph_primitives,
        &mut payloads,
    );

    let action = entry.action_rect(rect);
    if let Some(action) = action {
        primitives_raw.push(surface(action, 6.0, Token::PRIMARY_SOFT, 561));
    }

    primitives_raw.extend(glyph_primitives);
    for primitive in &mut primitives_raw {
        primitive.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
            primitive.depth_u16(),
            opacity * if control.disabled { 0.6 } else { 1.0 },
        );
    }

    let mut a11y = A11yNode::new(Role::Alert, entry.title.clone(), rect);
    a11y.description = entry.message.clone();
    a11y.value = entry.severity.label().to_owned();

    ToastOutput {
        output: WidgetOutput {
            primitives: primitives_raw,
            payloads,
            // 主体命中区域用于在指针停留在堆栈上时暂停倒计时。
            //
            hit: Some(
                HitRegion::new(node, PORTAL_LAYER, 560, rect).with_cursor(CursorKind::Default),
            ),
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        close: HitRegion::new(node ^ (1 << 62), PORTAL_LAYER, 562, close)
            .with_cursor(CursorKind::Pointer),
        action: action.map(|action| {
            HitRegion::new(node ^ (1 << 61), PORTAL_LAYER, 562, action)
                .with_cursor(CursorKind::Pointer)
        }),
    }
}

/// 编码为退化二次曲线的直线描边。
#[allow(clippy::too_many_arguments)]
fn stroke(
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let origin = Rect::new(
        from.0.min(to.0),
        from.1.min(to.1),
        (from.0 - to.0).abs().max(1.0),
        (from.1 - to.1).abs().max(1.0),
    );
    let payload_idx = payloads.len() as u32;
    payloads.push(GpuPayload::curve(
        [from.0 - origin.origin.x, from.1 - origin.origin.y],
        [
            (from.0 + to.0) * 0.5 - origin.origin.x,
            (from.1 + to.1) * 0.5 - origin.origin.y,
        ],
        [to.0 - origin.origin.x, to.1 - origin.origin.y],
        width,
    ));
    let mut primitive = instance(
        origin,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    // 每条关闭线描边拥有独立的载荷槽位；索引随实例传递，使两条曲线不会互相覆盖。
    //
    primitive.extra_payload_idx = payload_idx;
    primitives.push(primitive);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64) -> ToastEntry {
        ToastEntry::new(id, "Saved", Severity::Success).with_message("All changes stored.")
    }

    #[test]
    fn pushing_and_removing_keeps_insertion_order() {
        let mut stack = ToastStack::new();
        stack.push(entry(1));
        stack.push(entry(2));
        assert_eq!(
            stack
                .entries()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(stack.remove(1));
        assert!(!stack.remove(1));
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn the_countdown_expires_and_drops_the_toast() {
        let mut stack = ToastStack::new();
        stack.push(entry(7).with_duration(100.0));
        assert!(stack.tick(60.0).expired.is_empty());
        assert_eq!(stack.tick(60.0).expired, vec![7]);
        assert!(stack.is_empty());
    }

    #[test]
    fn hovering_the_stack_pauses_every_countdown() {
        let mut stack = ToastStack::new();
        stack.push(entry(7).with_duration(100.0));
        assert!(stack.set_hovered(true));
        assert!(stack.paused());
        assert!(stack.tick(500.0).expired.is_empty());
        assert_eq!(stack.entry(7).unwrap().remaining_ms, 100.0);
        stack.set_hovered(false);
        assert_eq!(stack.tick(500.0).expired, vec![7]);
    }

    #[test]
    fn a_persistent_toast_never_expires() {
        let mut stack = ToastStack::new();
        stack.push(entry(3).persistent());
        assert!(stack.tick(60_000.0).expired.is_empty());
        assert!(stack.entry(3).unwrap().is_persistent());
    }

    #[test]
    fn the_spring_settles_on_its_target() {
        let mut spring = ToastSpring::new(16.0);
        spring.target = 0.0;
        for _ in 0..240 {
            spring.step(1.0 / 60.0);
        }
        assert!(spring.settled());
        assert_eq!(spring.value, 0.0);
        assert_eq!(spring.velocity, 0.0);
    }

    #[test]
    fn the_slots_stack_upward_inside_the_viewport() {
        let layout = ToastLayout::new(Rect::new(0.0, 0.0, 1280.0, 720.0), 320.0, 64.0);
        let first = layout.slot(0);
        let second = layout.slot(1);
        assert!(second.origin.y < first.origin.y);
        assert_eq!(first.max().x, 1280.0 - 16.0);
        assert_eq!(first.max().y, 720.0 - 16.0);
        let bounds = layout.stack_rect(3);
        assert!(Rect::new(0.0, 0.0, 1280.0, 720.0).contains_rect(&bounds));
    }

    #[test]
    fn the_toast_reports_its_severity_and_buttons() {
        let mut toast = entry(4).with_action("Undo");
        toast.y_spring.value = 0.0;
        let rect = Rect::new(900.0, 600.0, 320.0, 64.0);
        let output = emit_toast(11, &toast, rect, &[], ControlState::default());
        assert_eq!(output.close.bounds, toast.close_rect(rect));
        assert_eq!(
            output.action.unwrap().bounds,
            toast.action_rect(rect).unwrap()
        );
        let a11y = output.output.a11y.expect("toast");
        assert_eq!(a11y.role, Role::Alert);
        assert_eq!(a11y.value, "成功");
        assert!(!output.output.primitives.is_empty());
    }

    #[test]
    fn the_fade_applies_to_every_primitive() {
        let toast = entry(4);
        let rect = Rect::new(0.0, 0.0, 320.0, 64.0);
        let mut faded = toast.clone();
        faded.opacity_spring.value = 0.5;
        let output = emit_toast(11, &faded, rect, &[], ControlState::default());
        for primitive in &output.output.primitives {
            // UNORM16 舍入会将 0.5 推高至约 0.50001。
            assert!(primitive.opacity() <= 0.501);
        }
    }
}
