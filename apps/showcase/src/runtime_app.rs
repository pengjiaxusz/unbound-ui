//! The dogfooded shell as a generic [`Application`].
//!
//! The showcase is not a special case any more: `GalleryApp` *is* the
//! application, and the runtime owns the pointer router, the focus manager
//! and the scene collector. The adapter that used to sit in between — and the
//! second router that came with it — is gone. This module only translates the
//! runtime's resolved events into the shell's methods and publishes the
//! finished scene into the runtime's collector, so the showcase runs through
//! exactly the same frame pipeline a third-party application would.

use unbound_app::{
    AppContext, AppEvent, AppKey, Application, DirtyFlags, EventOutcome, ImeUpdate,
    Modifiers as AppModifiers, ViewBuilder,
};
use unbound_input::FocusCause;
use unbound_platform::{Modifiers, PlatformKey};
use unbound_theme::ThemeKind;

use crate::shell::{GalleryApp, Interaction};
use crate::state::ThemeMode;

/// The showcase shell, driven by [`unbound_app::AppRuntime`].
///
/// The alias keeps the window host and the tests reading the same name while
/// the type itself is the shell.
pub type GalleryRuntime = GalleryApp;

impl GalleryApp {
    /// The shell, for the window host.
    #[must_use]
    pub fn shell(&self) -> &Self {
        self
    }

    /// The shell, mutably, for the parts the runtime does not own.
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
            // The release only ends a drag; the runtime reports the click
            // separately, and only when the press never became one.
            AppEvent::Click { node, pos, .. } => self.click(node, pos),
            // Once the pointer crosses the drag slop the router stops emitting
            // `PointerMove` and emits `DragStart`/`Drag` instead. They carry the
            // same position, so the shell's captured drags (scrollbar grabs,
            // sliders, canvas pan, marquee, card reorder) keep following the
            // pointer exactly as they did before the slop was crossed.
            AppEvent::DragStart { pos, .. } | AppEvent::Drag { pos, .. } => self.pointer_move(pos),
            AppEvent::DoubleClick { .. } => {}
            AppEvent::ContextMenu { .. } => {
                // The context-menu widget lands with the P2 container batch;
                // until then the shell has nothing to open.
            }
            AppEvent::Shortcut { .. } => {
                // The shell registers no keymap bindings yet; the runtime
                // resolves shortcuts for applications that do.
            }
            AppEvent::Wheel { pos, delta, .. } => {
                self.pointer_move(pos);
                self.wheel(delta.y);
                // The page scrolls as far as it can; a nested scroll area
                // would consume first and let the remainder bubble.
                ctx.consume_scroll(delta);
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
            AppEvent::Timer { .. } => {}
        }

        if let Some(node) = self.take_focus_request() {
            ctx.request_focus = Some((node, FocusCause::Programmatic));
        }
        if self.take_blur_request() {
            ctx.request_blur = true;
        }
        // The shell paints every frame it is asked for; `view` decides from
        // the interaction snapshot whether the scene has to be rebuilt.
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
        // The regions are what the runtime's router picks from next frame;
        // the shell never routes on its own. They are copied rather than
        // moved: a caller that inspects the last frame still sees them.
        collector.regions.extend_from_slice(&self.regions);
        collector.focus_order.extend_from_slice(&self.focus_order);
        collector
            .node_spans
            .extend(self.node_spans().iter().map(|(k, v)| (*k, *v)));
        // The shell rebuilds the semantic tree from scratch every frame, so
        // handing a copy over leaves the last frame inspectable.
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

/// Translate the runtime's key vocabulary back into the platform one the
/// shell already understands.
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
