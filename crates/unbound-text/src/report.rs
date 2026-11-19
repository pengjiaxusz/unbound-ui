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
    /// Emoji 实际生效渲染路径描述。
    pub emoji_path: String,
    /// 字体或光栅化回退累计发生次数。
    pub fallback_count: u64,
    /// 最近一次回退的原因说明（若未发生回退则为 None）。
    pub last_fallback_reason: Option<String>,
}

impl Default for TextRenderReport {
    fn default() -> Self {
        Self {
            backend: RasterBackend::default(),
            phases: 8,
            emoji_path: "Swash (Color RGBA8)".to_string(),
            fallback_count: 0,
            last_fallback_reason: None,
        }
    }
}
