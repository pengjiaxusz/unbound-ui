//! L2 无头交互与 L3 离屏渲染共用的统一测试驱动模块。

#[cfg(feature = "offscreen-test")]
mod offscreen;

#[cfg(feature = "offscreen-test")]
pub use offscreen::{OffscreenInitError, OffscreenTestHarness, RgbaImage};

use unbound_input::PointerButton;
use unbound_math::Vec2;

use crate::{AppKey, AppRuntime, Application, DirtyFlags, FrameReport, Modifiers};

/// 拖拽默认步进后的防抖与动画推进时长（秒）。
const DRAG_STEP_DT_SECS: f32 = 1.0 / 60.0;

/// L2 无头运行时与 L3 离屏渲染测试宿主共用的高层交互驱动原语。
pub trait InteractionDriver {
    /// 在 `pos` 处执行一次左键点击（移动指针 → 按下 → 释放）。
    fn click(&mut self, pos: Vec2) -> DirtyFlags {
        self.click_button(pos, PointerButton::Left)
    }

    /// 在 `pos` 处执行一次指定鼠标按键点击（移动指针 → 按下 → 释放）。
    fn click_button(&mut self, pos: Vec2, button: PointerButton) -> DirtyFlags;

    /// 从 `from` 拖拽至 `to`（自动补齐移动、按下、分段移动跨过拖拽阈值、时间推进与抬起序列）。
    fn drag(&mut self, from: Vec2, to: Vec2, steps: usize) -> DirtyFlags {
        self.drag_button(from, to, PointerButton::Left, steps)
    }

    /// 使用指定鼠标按键从 `from` 拖拽至 `to`。
    fn drag_button(
        &mut self,
        from: Vec2,
        to: Vec2,
        button: PointerButton,
        steps: usize,
    ) -> DirtyFlags;

    /// 在 `pos` 处发送一次滚轮事件。
    fn wheel(&mut self, pos: Vec2, delta: Vec2) -> DirtyFlags;

    /// 发送一次完整按键敲击（按下并立即释放）。
    fn key_tap(&mut self, key: AppKey, modifiers: Modifiers) -> DirtyFlags;

    /// 发送一段文本输入。
    fn text_input(&mut self, text: impl Into<String>) -> DirtyFlags;

    /// 调整测试视口逻辑尺寸（像素）。
    fn resize(&mut self, size: Vec2) -> DirtyFlags;

    /// 连续推进并渲染/重建 `count` 帧，每帧步进 `dt` 秒。
    fn step_frames(&mut self, count: usize, dt: f32);
}

impl<A: Application> InteractionDriver for AppRuntime<A> {
    fn click_button(&mut self, pos: Vec2, button: PointerButton) -> DirtyFlags {
        let mut dirty = self.pointer_move(pos);
        dirty = dirty.merge(self.pointer_down(pos, button));
        dirty = dirty.merge(self.pointer_up(pos, button));
        dirty
    }

    fn drag_button(
        &mut self,
        from: Vec2,
        to: Vec2,
        button: PointerButton,
        steps: usize,
    ) -> DirtyFlags {
        let steps = steps.max(2);
        let mut dirty = self.pointer_move(from);
        dirty = dirty.merge(self.pointer_down(from, button));
        self.advance(DRAG_STEP_DT_SECS);
        for i in 1..=steps {
            let t = (i as f32) / (steps as f32);
            let pos = Vec2::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            dirty = dirty.merge(self.pointer_move(pos));
            self.advance(DRAG_STEP_DT_SECS);
        }
        dirty = dirty.merge(self.pointer_up(to, button));
        dirty
    }

    fn wheel(&mut self, pos: Vec2, delta: Vec2) -> DirtyFlags {
        AppRuntime::wheel(self, pos, delta)
    }

    fn key_tap(&mut self, key: AppKey, modifiers: Modifiers) -> DirtyFlags {
        let down = self.key(key, true, modifiers);
        let up = self.key(key, false, modifiers);
        down.merge(up)
    }

    fn text_input(&mut self, text: impl Into<String>) -> DirtyFlags {
        self.text(text)
    }

    fn resize(&mut self, size: Vec2) -> DirtyFlags {
        AppRuntime::resize(self, size)
    }

    fn step_frames(&mut self, count: usize, dt: f32) {
        for _ in 0..count {
            let _ = self.tick(dt);
            let _ = self.frame(0.0);
        }
    }
}

/// L2 无头测试宿主：零 GPU、零窗口依赖，与 [`OffscreenTestHarness`] 共享同一套交互原语与帧前钩子语义。
/// 每帧构建前执行的钩子闭包类型。
type BeforeRenderHookFn<A> = Box<dyn FnMut(&mut AppRuntime<A>) + Send>;

pub struct HeadlessTestHarness<A: Application> {
    runtime: AppRuntime<A>,
    on_before_render: Option<BeforeRenderHookFn<A>>,
    last_report: Option<FrameReport>,
}

impl<A: Application> HeadlessTestHarness<A> {
    /// 构建指定逻辑视口尺寸的 L2 无头测试宿主。
    #[must_use]
    pub fn new(app: A, width: u32, height: u32) -> Self {
        let viewport = Vec2::new(width.max(1) as f32, height.max(1) as f32);
        Self {
            runtime: AppRuntime::new(app, viewport),
            on_before_render: None,
            last_report: None,
        }
    }

    /// 注册每帧构建前执行的钩子（链式构造）。
    #[must_use]
    pub fn with_on_before_render(
        mut self,
        hook: impl FnMut(&mut AppRuntime<A>) + Send + 'static,
    ) -> Self {
        self.on_before_render = Some(Box::new(hook));
        self
    }

    /// 设置或替换每帧构建前执行的钩子。
    pub fn set_on_before_render(&mut self, hook: impl FnMut(&mut AppRuntime<A>) + Send + 'static) {
        self.on_before_render = Some(Box::new(hook));
        self.runtime.invalidate();
    }

    /// 只读借用内部 [`AppRuntime`]。
    #[must_use]
    pub const fn runtime(&self) -> &AppRuntime<A> {
        &self.runtime
    }

    /// 可变借用内部 [`AppRuntime`]。
    pub fn runtime_mut(&mut self) -> &mut AppRuntime<A> {
        &mut self.runtime
    }

    /// 获取最近一次 [`Self::step_frame`] 产出的 [`FrameReport`]。
    #[must_use]
    pub const fn last_report(&self) -> Option<&FrameReport> {
        self.last_report.as_ref()
    }

    /// 推进一帧：触发 `on_before_render`、`tick(dt)` 与 `frame(0.0)`。
    pub fn step_frame(&mut self, dt: f32) -> &FrameReport {
        if let Some(hook) = self.on_before_render.as_mut() {
            hook(&mut self.runtime);
        }
        let _ = self.runtime.tick(dt);
        let report = self.runtime.frame(0.0);
        self.last_report = Some(report);
        self.last_report.as_ref().expect("last_report just set")
    }
}

impl<A: Application> InteractionDriver for HeadlessTestHarness<A> {
    fn click_button(&mut self, pos: Vec2, button: PointerButton) -> DirtyFlags {
        self.runtime.click_button(pos, button)
    }

    fn drag_button(
        &mut self,
        from: Vec2,
        to: Vec2,
        button: PointerButton,
        steps: usize,
    ) -> DirtyFlags {
        self.runtime.drag_button(from, to, button, steps)
    }

    fn wheel(&mut self, pos: Vec2, delta: Vec2) -> DirtyFlags {
        self.runtime.wheel(pos, delta)
    }

    fn key_tap(&mut self, key: AppKey, modifiers: Modifiers) -> DirtyFlags {
        self.runtime.key_tap(key, modifiers)
    }

    fn text_input(&mut self, text: impl Into<String>) -> DirtyFlags {
        self.runtime.text_input(text)
    }

    fn resize(&mut self, size: Vec2) -> DirtyFlags {
        self.runtime.resize(size)
    }

    fn step_frames(&mut self, count: usize, dt: f32) {
        for _ in 0..count {
            let _ = self.step_frame(dt);
        }
    }
}
