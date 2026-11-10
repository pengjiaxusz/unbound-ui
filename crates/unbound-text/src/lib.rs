//! `unbound-text` — industrial text: shaping caches, dual atlases, distance
//! fields (`docs` §3.5).
//!
//! The modules map one-to-one onto the milestone's deliverables:
//!
//! | module | responsibility |
//! | :--- | :--- |
//! | [`shaping`] | `cosmic-text` shaping + level-1 shape cache + level-2 wrap cache |
//! | [`atlas`] | `Atlas_R8` (4 subpixel phases) + `Atlas_RGBA8`, paged, LRU by frame |
//! | [`glyph`] | rasterisation, atlas residency, quad emission |
//! | [`sdf`] | exact Euclidean distance transform for scalable canvas text |
//!
//! Nothing here needs a GPU, which is what lets the whole text stack be
//! regression-tested in CI on a machine without a display.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod atlas;
pub mod glyph;
pub mod sdf;
pub mod shaping;

pub use atlas::{Atlas, AtlasConfig, GlyphAtlas, GlyphKey, GlyphSlot, SUBPIXEL_PHASES};
pub use glyph::{AtlasTarget, AtlasUpdate, GlyphCache, GlyphQuad, ResolvedGlyph};
pub use sdf::{from_coverage, reconstruct_coverage, SdfBitmap};
pub use shaping::{
    PositionedGlyph, ShapedText, TextCacheStats, TextEngine, TextLine, TextStyle, WrappedText,
};
