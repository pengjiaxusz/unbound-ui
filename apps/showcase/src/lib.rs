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
/// The shell's layout tree: the node tree `LayoutEngine` solves.
pub mod layout_tree;
/// The page catalogue and the live example renderers.
pub mod pages;
/// The shell adapted to the generic application runtime.
pub mod runtime_app;
/// The dogfooded application shell and its state machine.
pub mod shell;
/// Navigation, knob and demo state.
pub mod state;
/// Stable widget identities.
pub mod target;
/// Headless test harness for the button.
pub mod testing;
/// The brand palettes of the showcase, packed into the engine's colour LUT.
pub mod theme;
/// Reusable gallery controls (buttons, switches, sliders, pills).
pub mod widgets;
