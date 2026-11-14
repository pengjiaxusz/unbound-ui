//! 作为通用 [`Application`] 实现的自举外壳。
//!
//! showcase 不再是特例：`GalleryApp` *就是*应用本身，运行时拥有指针路由器、
//! 焦点管理器与场景收集器。曾经夹在中间的适配器——以及随之而来的第二个路由器——已移除。
//! 本模块仅将运行时的已解析事件转换为外壳方法调用，并将完成的场景发布到运行时的收集中，
//! 使 showcase 与第三方应用走完全相同的帧管线。
//!
//!
//!

use unbound_app::{
    AppContext, AppEvent, AppKey, Application, DirtyFlags, EventOutcome, ImeUpdate,
    Modifiers as AppModifiers, ViewBuilder,
};
use unbound_input::FocusCause;
use unbound_platform::{Modifiers, PlatformKey};
use unbound_theme::ThemeKind;

use crate::shell::{GalleryApp, Interaction};
use crate::state::ThemeMode;

/// 由 [`unbound_app::AppRuntime`] 驱动的 showcase 外壳。
///
/// 该别名让窗口宿主和测试保持相同的类型名，而类型本身即外壳。
///
pub type GalleryRuntime = GalleryApp;

impl GalleryApp {
    /// 外壳引用，供窗口宿主使用。
    #[must_use]
    pub fn shell(&self) -> &Self {
        self
    }

    /// 可变外壳引用，用于访问运行时不拥有的部分。
    pub fn shell_mut(&mut self) -> &mut Self {
        self
    }
}

impl Application for GalleryApp {
    type State = ();

    fn init(&mut self, ctx: &mut AppContext) -> Self::State {
        ctx.theme = crate::theme::kind(self.state.theme == ThemeMode::Dark);
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::PointerMove { pos, .. } => self.pointer_move(pos),
            AppEvent::PointerDown { pos, target, .. } => self.pointer_down(pos, target),
            AppEvent::PointerUp { pos, .. } => self.pointer_up(pos),
            // 释放仅结束拖拽；运行时会单独上报点击，且仅当按压从未转为拖拽时才上报。
            //
            AppEvent::Click { node, pos, .. } => self.click(node, pos),
            // 一旦指针越过拖拽阈值（drag slop），路由器就会停止发送 `PointerMove`，
            // 改为发送 `DragStart`/`Drag`。它们携带相同的位置，因此外壳的捕获拖拽
            // （滚动条抓取、滑块、画布平移、框选、卡片重排）会像越过阈值前一样继续跟随指针。
            //
            //
            AppEvent::DragStart { pos, .. } | AppEvent::Drag { pos, .. } => self.pointer_move(pos),
            AppEvent::DoubleClick { .. } => {}
            AppEvent::ContextMenu { .. } => {
                // 上下文菜单控件随 P2 容器批次一起落地；在此之前外壳没有可打开的菜单。
                //
            }
            AppEvent::Shortcut { .. } => {
                // 外壳尚未注册按键映射绑定；运行时会为已注册的应用解析快捷键。
                //
            }
            AppEvent::Wheel { pos, delta, .. } => {
                self.pointer_move(pos);
                self.wheel(delta.y);
                // 页面尽可能滚动；嵌套滚动区域会先消费滚动量，再将剩余部分冒泡。
                //
                ctx.consume_scroll(delta);
            }
            AppEvent::Key {
                key,
                pressed,
                modifiers,
            } => {
                if pressed {
                    if modifiers.ctrl && !modifiers.alt && !modifiers.meta {
                        if let AppKey::Character(ch) = key {
                            let ch = ch.to_ascii_lowercase();
                            if ch == 'c' {
                                if self.state.search_focused && !self.state.search_query.is_empty()
                                {
                                    ctx.clipboard_set(&self.state.search_query);
                                } else if self.state.demo.typing && !self.state.demo.text.is_empty()
                                {
                                    ctx.clipboard_set(&self.state.demo.text);
                                }
                            } else if ch == 'x' {
                                if self.state.search_focused && !self.state.search_query.is_empty()
                                {
                                    ctx.clipboard_set(&self.state.search_query);
                                    self.state.search_query.clear();
                                    self.state.bump();
                                } else if self.state.demo.typing && !self.state.demo.text.is_empty()
                                {
                                    ctx.clipboard_set(&self.state.demo.text);
                                    self.state.demo.text.clear();
                                    self.state.bump();
                                }
                            } else if ch == 'v' {
                                if let Some(pasted) = ctx.clipboard_get() {
                                    if self.state.search_focused {
                                        self.state.search_query.push_str(&pasted);
                                        self.state.bump();
                                    } else if self.state.demo.typing {
                                        self.state.demo.text.push_str(&pasted);
                                        self.state.bump();
                                    }
                                }
                            }
                        }
                    }
                    let key = platform_key(key);
                    let modifiers = platform_modifiers(modifiers);
                    self.key(&key, modifiers, ctx.focused);
                }
            }
            AppEvent::Text { text } => self.type_text(&text),
            AppEvent::Ime { update } => self.apply_ime(&update),
            AppEvent::Resize { size } => {
                ctx.viewport = size;
            }
            AppEvent::Theme { kind } => {
                self.state.theme = if kind == ThemeKind::Dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                };
                self.state.bump();
            }
            AppEvent::A11y {
                target,
                action,
                value,
            } => self.apply_a11y(target, action, value),
            AppEvent::Tick { dt } => {
                self.tick(dt);
            }
            AppEvent::Timer { .. }
            | AppEvent::FileHover(_)
            | AppEvent::FileDrop(_)
            | AppEvent::FileHoverCancelled => {}
        }

        if let Some(node) = self.take_focus_request() {
            ctx.request_focus = Some((node, FocusCause::Programmatic));
        }
        if self.take_blur_request() {
            ctx.request_blur = true;
        }
        // 外壳会绘制被请求的每一帧；`view` 根据交互快照决定是否需要重建场景。
        //
        EventOutcome::new(DirtyFlags::LAYOUT)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
        let viewport = (view.ctx.viewport.x, view.ctx.viewport.y);
        let inter = Interaction {
            hovered: view.ctx.hovered,
            pressed: view.ctx.pressed,
            focused: view.ctx.focused,
        };
        if self.needs_rebuild(viewport, &inter, view.ctx.force) {
            self.rebuild(viewport, inter);
        }
        let collector = view.collector();
        collector
            .primitives
            .extend_from_slice(&self.scene.primitives);
        collector
            .transforms
            .extend_from_slice(&self.scene.transforms);
        collector.clips.extend_from_slice(&self.scene.clips);
        collector.payloads.extend_from_slice(&self.scene.payloads);
        // 这些区域是运行时路由器在下一帧进行拾取的依据；外壳从不自行路由。
        // 这里采用拷贝而非移动，以便检查上一帧的调用方仍能看到它们。
        //
        collector.regions.extend_from_slice(&self.regions);
        collector.focus_order.extend_from_slice(&self.focus_order);
        collector
            .node_spans
            .extend(self.node_spans().iter().map(|(k, v)| (*k, *v)));
        // 外壳每帧都会从头重建语义树，因此交出一份拷贝可使上一帧保持可检查状态。
        //
        collector.semantics = self.semantics.clone();
    }
}

impl GalleryApp {
    fn apply_ime(&mut self, update: &ImeUpdate) {
        if let Some(commit) = &update.commit {
            self.type_text(commit);
        }
        if self.state.ime_preedit != update.preedit {
            self.state.ime_preedit = update.preedit.clone();
            self.state.bump();
        }
    }
}

/// 将运行时的按键词汇表转换回外壳已理解的平台按键词汇表。
///
fn platform_key(key: AppKey) -> PlatformKey {
    match key {
        AppKey::Tab => PlatformKey::Named("Tab".into()),
        AppKey::Escape => PlatformKey::Named("Escape".into()),
        AppKey::Enter => PlatformKey::Named("Enter".into()),
        AppKey::Space => PlatformKey::Character(" ".into()),
        AppKey::Backspace => PlatformKey::Named("Backspace".into()),
        AppKey::Delete => PlatformKey::Named("Delete".into()),
        AppKey::Insert => PlatformKey::Named("Insert".into()),
        AppKey::Left => PlatformKey::Named("ArrowLeft".into()),
        AppKey::Right => PlatformKey::Named("ArrowRight".into()),
        AppKey::Up => PlatformKey::Named("ArrowUp".into()),
        AppKey::Down => PlatformKey::Named("ArrowDown".into()),
        AppKey::Home => PlatformKey::Named("Home".into()),
        AppKey::End => PlatformKey::Named("End".into()),
        AppKey::PageUp => PlatformKey::Named("PageUp".into()),
        AppKey::PageDown => PlatformKey::Named("PageDown".into()),
        AppKey::ContextMenu => PlatformKey::Named("ContextMenu".into()),
        AppKey::CapsLock => PlatformKey::Named("CapsLock".into()),
        AppKey::F(number) => PlatformKey::Named(format!("F{number}")),
        AppKey::Copy => PlatformKey::Named("Copy".into()),
        AppKey::Cut => PlatformKey::Named("Cut".into()),
        AppKey::Paste => PlatformKey::Named("Paste".into()),
        AppKey::Undo => PlatformKey::Named("Undo".into()),
        AppKey::Redo => PlatformKey::Named("Redo".into()),
        AppKey::Character(c) => PlatformKey::Character(c.to_string()),
    }
}

fn platform_modifiers(modifiers: AppModifiers) -> Modifiers {
    Modifiers {
        shift: modifiers.shift,
        control: modifiers.ctrl,
        alt: modifiers.alt,
        meta: modifiers.meta,
    }
}
