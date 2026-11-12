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
    AppContext, AppEvent, AppKey, Application, DirtyFlags, ImeUpdate, Modifiers as AppModifiers,
    ViewBuilder,
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
    ) -> DirtyFlags {
        match event {
            AppEvent::PointerMove { pos, .. } => self.pointer_move(pos),
            AppEvent::PointerDown { pos, target, .. } => self.pointer_down(pos, target),
            AppEvent::PointerUp { pos, .. } => self.pointer_up(pos),
            // The release only ends a drag; the runtime reports the click
            // separately, and only when the press never became one.
            AppEvent::Click { node, pos, .. } => self.click(node, pos),
            AppEvent::DoubleClick { .. } | AppEvent::DragStart { .. } | AppEvent::Drag { .. } => {}
            AppEvent::Wheel { pos, delta, .. } => {
                self.pointer_move(pos);
                self.wheel(delta.y);
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
        }

        if let Some(node) = self.take_focus_request() {
            ctx.request_focus = Some((node, FocusCause::Programmatic));
        }
        if self.take_blur_request() {
            ctx.request_blur = true;
        }
        // The shell paints every frame it is asked for; `view` decides from
        // the interaction snapshot whether the scene has to be rebuilt.
        DirtyFlags::LAYOUT
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
        AppKey::Left => PlatformKey::Named("ArrowLeft".into()),
        AppKey::Right => PlatformKey::Named("ArrowRight".into()),
        AppKey::Up => PlatformKey::Named("ArrowUp".into()),
        AppKey::Down => PlatformKey::Named("ArrowDown".into()),
        AppKey::Home => PlatformKey::Named("Home".into()),
        AppKey::End => PlatformKey::Named("End".into()),
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
