//! The dogfooded shell as a generic [`Application`].
//!
//! `GalleryApp` already owns the whole state machine of the showcase; this
//! adapter only translates the runtime's resolved events into its method
//! calls and copies the finished scene into the runtime's collector. The
//! window host then uploads `AppRuntime::frame`'s report instead of doing its
//! own diff, so the showcase runs through exactly the same frame pipeline a
//! third-party application would.

use unbound_app::{
    AppContext, AppEvent, AppKey, Application, DirtyFlags, ImeUpdate, Modifiers as AppModifiers,
    ViewBuilder,
};
use unbound_platform::{Modifiers, PlatformKey};
use unbound_theme::ThemeKind;

use crate::shell::GalleryApp;
use crate::state::ThemeMode;
use crate::target::ROOT;

/// The observable state a rebuild depends on.
///
/// A pointer move that crosses empty space changes none of these, so the
/// adapter reports no dirty flags and the runtime skips the rebuild.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Token {
    revision: u64,
    hover: Option<u64>,
    pressed: Option<u64>,
    focus: Option<u64>,
}

impl Token {
    fn of(app: &GalleryApp) -> Self {
        Self {
            revision: app.state.revision,
            hover: app.router.hover(),
            pressed: app.router.pointer().pressed(),
            focus: app.focused,
        }
    }
}

/// The showcase shell, reduced through [`unbound_app::AppRuntime`].
pub struct GalleryRuntime {
    app: GalleryApp,
}

impl Default for GalleryRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl GalleryRuntime {
    /// Wrap a fresh shell.
    #[must_use]
    pub fn new() -> Self {
        let mut app = GalleryApp::new();
        // The runtime owns the frame diff now.
        app.set_external_diff(true);
        Self { app }
    }

    /// The shell, for the window host.
    #[must_use]
    pub fn shell(&self) -> &GalleryApp {
        &self.app
    }

    /// The shell, mutably, for the parts the runtime does not own.
    pub fn shell_mut(&mut self) -> &mut GalleryApp {
        &mut self.app
    }
}

impl Application for GalleryRuntime {
    type State = ();

    fn init(&mut self, ctx: &mut AppContext) -> Self::State {
        ctx.theme = crate::theme::kind(self.app.state.theme == ThemeMode::Dark);
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> DirtyFlags {
        let before = Token::of(&self.app);
        match event {
            AppEvent::PointerMove { pos, .. } => self.app.pointer_move(pos),
            AppEvent::PointerDown { pos, .. } => self.app.pointer_down(pos, ctx.time),
            AppEvent::PointerUp { pos, .. } => self.app.pointer_up(pos, ctx.time),
            AppEvent::Wheel { pos, delta, .. } => {
                self.app.pointer_move(pos);
                self.app.wheel(delta.y);
            }
            AppEvent::Key {
                key,
                pressed,
                modifiers,
            } => {
                if pressed {
                    let key = platform_key(key);
                    let modifiers = platform_modifiers(modifiers);
                    self.app.key(&key, modifiers);
                }
            }
            AppEvent::Text { text } => self.app.type_text(&text),
            AppEvent::Ime { update } => self.apply_ime(&update),
            AppEvent::Resize { size } => {
                ctx.viewport = size;
                return DirtyFlags::LAYOUT;
            }
            AppEvent::Theme { kind } => {
                self.app.state.theme = if kind == ThemeKind::Dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                };
                self.app.state.bump();
            }
            AppEvent::A11y {
                target,
                action,
                value,
            } => self.app.apply_a11y(target, action, value),
            AppEvent::Tick { dt } => {
                self.app.tick(dt);
            }
            // The shell's own router already resolves clicks and drags; the
            // adapter forwards the position-level events above, so the
            // runtime's derived events are informational only.
            AppEvent::Click { .. }
            | AppEvent::DoubleClick { .. }
            | AppEvent::DragStart { .. }
            | AppEvent::Drag { .. } => {}
        }
        let after = Token::of(&self.app);
        if after != before {
            DirtyFlags::PAINT
        } else {
            DirtyFlags::NONE
        }
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
        let viewport = (view.ctx.viewport.x, view.ctx.viewport.y);
        self.app.rebuild(viewport);
        let collector = view.collector();
        collector
            .primitives
            .extend_from_slice(&self.app.scene.primitives);
        collector
            .transforms
            .extend_from_slice(&self.app.scene.transforms);
        collector.clips.extend_from_slice(&self.app.scene.clips);
        collector
            .payloads
            .extend_from_slice(&self.app.scene.payloads);
        collector
            .focus_order
            .extend_from_slice(&self.app.focus_order);
        collector
            .node_spans
            .extend(self.app.node_spans().iter().map(|(k, v)| (*k, *v)));
        // The shell rebuilds the semantic tree from scratch every frame, so
        // handing it over costs nothing: the next rebuild replaces it.
        collector.semantics =
            core::mem::replace(&mut self.app.semantics, unbound_a11y::Semantics::new(ROOT));
    }
}

impl GalleryRuntime {
    fn apply_ime(&mut self, update: &ImeUpdate) {
        if let Some(commit) = &update.commit {
            self.app.type_text(commit);
        }
        if self.app.state.ime_preedit != update.preedit {
            self.app.state.ime_preedit = update.preedit.clone();
            self.app.state.bump();
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
