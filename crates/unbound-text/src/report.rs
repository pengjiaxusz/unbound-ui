//! 文本渲染只读运行期生效报告模型。
//!
//! 承载实际生效的光栅化后端、相位数、Emoji 路径、回退计数与最近一次回退原因。
//! 由引擎底层在光栅化与塑形阶段产生，展台与集成方只读该报告。

use serde::{Deserialize, Serialize};

use crate::glyph::RasterBackend;

/// 文本渲染实际生效报告只读快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRenderReport {
    /// 正文实际生效光栅化后端。
    pub backend: RasterBackend,
    /// 实际生效亚像素相位数（8 / 4 / 1）。
    pub phases: u8,
    /// Emoji 实际生效渲染路径描述，取值来自 [`TextRenderReport::emoji_path`] 一节
    /// 的常量，不在调用方另写字面量。
    pub emoji_path: String,
    /// 当前生效路径与目标语义的已知差异；无差异时为 `None`。
    ///
    /// 只记录**已经确定不打算在本轮修掉**的偏差（例如 swash 侧的 `Mono` 仍
    /// 取彩色位图的亮度），不重复 [`Self::last_fallback_reason`] 已经表达的
    /// 运行期回退。面板必须把这条显示出来，不能假装两个后端一致。
    #[serde(default)]
    pub known_deviation: Option<String>,
    /// 字体或光栅化回退累计发生次数。
    pub fallback_count: u64,
    /// 最近一次回退的原因说明（若未发生回退则为 None）。
    pub last_fallback_reason: Option<String>,
    /// RGBA8 图集是否已开辟。
    pub rgba_atlas_allocated: bool,
    /// 图集显存占用字节数。
    pub atlas_memory_bytes: u64,
}

impl TextRenderReport {
    /// DirectWrite 彩色层合成：相位锁 0，一簇一个图集条目。
    pub const EMOJI_PATH_DIRECTWRITE_COLOR: &'static str = "DirectWrite (Color layers, phase 0)";
    /// swash 彩色：RGBA8 图集，逐相位重采样。
    pub const EMOJI_PATH_SWASH_COLOR: &'static str = "Swash (Color RGBA8)";
    /// DirectWrite 单色：基字形覆盖率，R8 图集按文字色染色。
    pub const EMOJI_PATH_DIRECTWRITE_MONO: &'static str = "DirectWrite (Mono R8)";
    /// swash 单色：彩色位图取亮度，R8 图集按文字色染色。
    pub const EMOJI_PATH_SWASH_MONO: &'static str = "Swash (Mono R8)";
    /// 没有任何可用的 Emoji 光栅器（此构建不含对应后端）。
    pub const EMOJI_PATH_NONE: &'static str = "Unavailable (no emoji rasterizer in this build)";

    /// 已知差异：swash 的 `Mono` 由彩色位图取亮度得到，与 DirectWrite 的
    /// 基字形剪影不是同一张图（见 ADR 0017 第 5、7 条）。
    pub const KNOWN_DEVIATION_SWASH_MONO: &'static str =
        "swash Mono is luminance-derived while DirectWrite Mono is the base silhouette";

    /// 已知差异：此构建不含 swash，显式选择 `Swash` 回退时按 `Mono` 执行。
    pub const KNOWN_DEVIATION_NO_SWASH_FALLBACK: &'static str =
        "this build has no swash; EmojiFallback::Swash executes as Mono";
}

impl Default for TextRenderReport {
    fn default() -> Self {
        Self {
            backend: RasterBackend::default(),
            phases: 8,
            emoji_path: Self::EMOJI_PATH_SWASH_COLOR.to_string(),
            known_deviation: None,
            fallback_count: 0,
            last_fallback_reason: None,
            rgba_atlas_allocated: false,
            atlas_memory_bytes: 0,
        }
    }
}
