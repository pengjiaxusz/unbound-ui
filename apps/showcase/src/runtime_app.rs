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
    AppContext, AppEvent, AppKey, Application, DirtyFlags, EventOutcome, Locale,
    Modifiers as AppModifiers, ViewBuilder,
};
use unbound_backdrop::{BackdropSource, MaterialKind};
use unbound_input::FocusCause;
use unbound_platform::{Modifiers, PlatformKey};
use unbound_theme::ThemeKind;

use crate::shell::{ClipboardRequest, GalleryApp, Interaction};
use crate::state::ThemeMode;
use crate::target;

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
        // 初始化即解析默认后景计划，后续每帧再跟随选择变化。
        self.sync_backdrop_plan();
        ctx.theme = crate::theme::kind(self.state.theme == ThemeMode::Dark);
        ctx.set_locale_enum(self.state.locale);
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::PointerMove { pos, .. } => self.pointer_move(pos),
            AppEvent::PointerDown {
                pos,
                target,
                button,
                ..
            } => self.pointer_down(pos, target, button),
            AppEvent::PointerUp { pos, .. } => self.pointer_up(pos),
            // 释放仅结束拖拽；运行时会单独上报点击，且仅当按压从未转为拖拽时才上报。
            // 第三次及之后的连击由 `count` 标出，此时按“全选”处理而不是重新定位插入符。
            AppEvent::Click { node, pos, count } => {
                if count >= 3 {
                    if let Some(target) = target::decode(node) {
                        self.triple_click_editor(target);
                    }
                } else {
                    self.click(node, pos);
                }
                // 上下文菜单的剪切/复制/粘贴由点击触发，必须在此同步排空
                // 宿主剪贴板请求，否则复制不会写入、粘贴要等到下一次按键。
                self.drain_clipboard(ctx);
            }
            // 一旦指针越过拖拽阈值（drag slop），路由器就会停止发送 `PointerMove`，
            // 改为发送 `DragStart`/`Drag`。它们携带相同的位置，因此外壳的捕获拖拽
            // （滚动条抓取、滑块、画布平移、框选、卡片重排）会像越过阈值前一样继续跟随指针。
            //
            //
            AppEvent::DragStart { pos, .. } | AppEvent::Drag { pos, .. } => self.pointer_move(pos),
            AppEvent::DoubleClick { node, pos } => {
                if let Some(target) = target::decode(node) {
                    self.double_click_editor(target, pos);
                }
            }
            AppEvent::ContextMenu { node, pos } => {
                if let Some(target) = target::decode(node) {
                    let clipboard_has_text = ctx.clipboard_get().is_some();
                    self.open_editor_menu(target, pos, clipboard_has_text);
                }
            }
            AppEvent::Shortcut { .. } => {
                // 外壳尚未注册按键映射绑定；运行时会为已注册的应用解析快捷键。
                //
            }
            AppEvent::Wheel { pos, delta, .. } => {
                let menu_was_open = self.editor_menu_open();
                self.pointer_move(pos);
                self.wheel(delta.y);
                // 菜单打开时滚轮只关闭菜单，不得再滚动其下方的页面。
                //
                if !menu_was_open {
                    ctx.consume_scroll(delta);
                }
            }
            AppEvent::Key {
                key,
                pressed,
                modifiers,
            } => {
                if pressed {
                    let key = platform_key(key);
                    let modifiers = platform_modifiers(modifiers);
                    self.key(&key, modifiers, ctx.focused);
                    self.drain_clipboard(ctx);
                }
            }
            AppEvent::Text { text } => self.type_text(&text),
            AppEvent::Ime { update } => {
                self.apply_ime(&update);
                // IME 事件可能同时改变编辑缓冲区，需要同步排空宿主剪贴板请求。
                //
                self.drain_clipboard(ctx);
            }
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
            AppEvent::UiScaleChanged { scale } => {
                // 缩放档位由 AppContext 统一吸附并激活 OSD 提示，随后强制重建场景。
                ctx.set_ui_scale(scale);
                self.state.bump();
            }
            AppEvent::LocaleChanged { locale } => {
                let parsed = Locale::from_tag(&locale);
                self.state.set_locale(parsed);
                ctx.set_locale_enum(parsed);
            }
            // 云母纹理已生成：下一帧 `view` 会把它提交给资产中心并绘制。
            AppEvent::Custom { id, .. } if unbound_app::is_mica_ready_event(id) => {
                self.state.bump();
            }
            AppEvent::Timer { .. }
            | AppEvent::FileHover(_)
            | AppEvent::FileDrop(_)
            | AppEvent::FileHoverCancelled
            | AppEvent::Custom { .. } => {}
        }

        // 若外壳内部点击（如标题栏语言下拉菜单或 i18n 沙盒按钮）切换了语言，将其同步至 AppContext。
        if ctx.locale() != self.state.locale {
            ctx.set_locale_enum(self.state.locale);
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
        // 云母纹理在后台生成、解码：先提交/绘制，再用最新可见性决定根背景。
        // 可见性翻转本身必须触发重建，否则不透明底色会一直盖着已经就绪的纹理。
        // 后景计划先于绘制同步：选择变化会在同一帧写入实际生效组合并请求重建。
        self.sync_backdrop_plan();
        let plan = self.state.backdrop_plan.clone();
        // 只有自绘路径才需要生成、同步并绘制壁纸纹理；原生路径与「无后景」跳过，
        // 避免无意义的 CPU 与内存开销。
        let draws_self_drawn =
            plan.material_kind != MaterialKind::None && plan.source == BackdropSource::SelfDrawn;
        self.mica.set_enabled(draws_self_drawn);
        let was_mica_visible = self.mica.visible();
        let uv_changed = if draws_self_drawn {
            self.mica.sync(plan.texture, plan.uv, view.event_sender())
        } else {
            false
        };
        let mica_visible = self
            .mica
            .draw(view, unbound_app::ui_key!("showcase_mica_backdrop"));
        if mica_visible != was_mica_visible {
            // 纹理就绪会改变计划里的根背景透明性；先刷新计划再重建，避免
            // 用上一帧的结论把刚可见的后景重新盖住。
            self.sync_backdrop_plan();
        }
        if mica_visible != was_mica_visible
            || uv_changed
            || self.needs_rebuild(viewport, &inter, view.ctx.force)
        {
            self.rebuild(viewport, inter);
        }
        let collector = view.collector();
        // 外壳场景自带从 0 起算的载荷 / 变换 / 裁剪索引；收集器里已经压入了
        // 云母背板等图元，必须重定位后再拼接，否则字形的 UV 会整体错位。
        collector.append_scene(
            &self.scene.primitives,
            &self.scene.transforms,
            &self.scene.clips,
            &self.scene.payloads,
        );
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
    /// 执行外壳排队的剪贴板动作。
    fn drain_clipboard(&mut self, ctx: &mut AppContext) {
        while let Some(request) = self.take_clipboard_request() {
            match request {
                ClipboardRequest::Write(text) => ctx.clipboard_set(&text),
                ClipboardRequest::Read => {
                    if let Some(text) = ctx.clipboard_get() {
                        self.paste_text(&text);
                    }
                }
            }
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
