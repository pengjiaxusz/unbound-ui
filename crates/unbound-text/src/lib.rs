//! `unbound-text` — 工业级文本管线：塑形缓存、双格式图集与距离场。
//!
//! 各模块与里程碑交付物一一对应：
//!
//! | 模块 | 职责 |
//! | :--- | :--- |
//! | [`shaping`] | `cosmic-text` 塑形 + 一级塑形缓存 + 二级换行缓存 |
//! | [`atlas`] | `Atlas_R8`（每个字形 8 个亚像素相位）+ `Atlas_RGBA8`，分页式，按帧 LRU 淘汰 |
//! | [`glyph`] | 光栅化、图集驻留管理与四边形图元发射 |
//! | [`sdf`] | 面向可缩放画布文本的精确欧氏距离变换 |
//! | [`pinyin`] | 全量汉字（全拼 + 6 套双拼 + 全拼/双拼首字母 + 多音字）运行时可调检索引擎 |
//!
//! 此处所有模块均不依赖 GPU，使整个文本栈可在无显示器的 CI 环境中
//! 完成回归测试。

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod atlas;
#[cfg(windows)]
mod directwrite;
pub mod glyph;
pub mod pinyin;
pub mod pinyin_data;
pub mod sdf;
pub mod selection;
pub mod shaping;

pub use atlas::{Atlas, AtlasConfig, GlyphAtlas, GlyphKey, GlyphSlot, SUBPIXEL_PHASES};
pub use glyph::{AtlasTarget, AtlasUpdate, GlyphCache, GlyphQuad, RasterBackend, ResolvedGlyph};
pub use pinyin::{
    global_pinyin_config, global_shuangpin_scheme, set_global_pinyin_config,
    set_global_shuangpin_scheme, PinyinConfig, PinyinMatch, PinyinMatcher, ShuangpinScheme,
};
pub use sdf::{from_coverage, reconstruct_coverage, SdfBitmap};
pub use selection::{ime_cursor_area, merge_line_rects, outline_polygon, RoundedSlot};
pub use shaping::{
    cjk_family_for_locale, PositionedGlyph, ShapedText, TextCacheStats, TextEngine, TextLine,
    TextSpan, TextStyle, WrappedText,
};
