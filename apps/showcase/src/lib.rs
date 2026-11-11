//! The windowed Unbound UI Gallery.
//!
//! The library half exposes the application shell, its catalogue and the
//! scene painter so the gallery can be driven headlessly in tests; the binary
//! half opens a window and shows it.

/// The first interactive widget: a flat, pure-function button.
pub mod button;
/// Canvas: primitives, clipping, hit regions and semantics.
pub mod canvas;
/// The low-level scene painter and the palette.
pub mod gallery;
/// Keyboard actions shared by the window and the headless tests.
pub mod keys;
/// The page catalogue and the live example renderers.
pub mod pages;
/// The dogfooded application shell and its state machine.
pub mod shell;
/// Navigation, knob and demo state.
pub mod state;
/// Stable widget identities.
pub mod target;
/// Headless test harness for the button.
pub mod testing;
/// Reusable gallery controls (buttons, switches, sliders, pills).
pub mod widgets;
