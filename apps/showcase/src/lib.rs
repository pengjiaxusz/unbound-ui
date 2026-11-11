//! The windowed component gallery.
//!
//! The library half exposes the gallery scene so it can be rendered
//! offscreen in tests; the binary half opens a window and shows it.

/// The first interactive widget: a flat, pure-function button.
pub mod button;
/// The component gallery scene.
pub mod gallery;
/// Keyboard actions shared by the window and the headless tests.
pub mod keys;
/// Headless test harness for the button.
pub mod testing;
