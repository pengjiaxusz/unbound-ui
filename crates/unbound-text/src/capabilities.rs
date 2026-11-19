//! 文本管线三态能力探测与查询模型。
//!
//! 提供 [`CapabilityStatus`]（`Available` / `Unavailable` / `NotCompiled`）三态判断，
//! 作为面板控件灰化与集成方配置校验的单一能力真相源。

use crate::glyph::RasterBackend;
use crate::settings::PhaseMode;

/// 单项特性的三态能力状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CapabilityStatus {
    /// 已编译且当前系统环境支持。
    Available,
    /// 已编译但当前系统环境/配置不满足（带具体原因）。
    Unavailable(&'static str),
    /// 当前构建未包含该功能或后端。
    NotCompiled,
}

impl CapabilityStatus {
    /// 特性是否完全可用。
    #[must_use]
    pub fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }

    /// 若不可用，获取其说明原因（英文文案）。
    #[must_use]
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Unavailable(reason) => Some(reason),
            Self::NotCompiled => Some("Feature not included in this build"),
            Self::Available => None,
        }
    }
}

/// 文本引擎与当前平台的能力全貌快照。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextCapabilities {
    /// Windows DirectWrite 覆盖率光栅化可用性。
    pub directwrite: CapabilityStatus,
    /// Cosmic (swash) 光栅化可用性。
    pub cosmic: CapabilityStatus,
    /// 欧氏距离场（SDF）可用性。
    pub sdf: CapabilityStatus,
    /// 多通道距离场（MTSDF/MSDF）可用性。
    pub mtsdf: CapabilityStatus,
    /// 亚像素相位（8 相位）可用性。
    pub subpixel_phase_8: CapabilityStatus,
    /// 亚像素相位（4 相位）可用性。
    pub subpixel_phase_4: CapabilityStatus,
    /// 整像素相位（1 相位）可用性。
    pub subpixel_phase_1: CapabilityStatus,
    /// 彩色字形（RGBA8 图集）可用性。
    pub color_glyphs: CapabilityStatus,
    /// 单色字形回退可用性。
    pub mono_fallback: CapabilityStatus,
    /// 单色 DirectWrite 可用性。
    pub mono_directwrite: CapabilityStatus,
    /// 彩色 DirectWrite 可用性。
    pub color_directwrite: CapabilityStatus,
    /// 网格拟合（Hinting）可用性。
    pub hinting: CapabilityStatus,
    /// 子像素抗锯齿（RGB/BGR）可用性。
    pub subpixel_antialiasing: CapabilityStatus,
}

impl TextCapabilities {
    /// 探测当前平台与当前构建支持的能力。
    #[must_use]
    pub fn probe() -> Self {
        #[cfg(windows)]
        let directwrite = {
            // Windows 下探测 DirectWrite 是否可正常初始化
            let is_ok = crate::directwrite::DirectWriteRasterizer::new().is_some();
            if is_ok {
                CapabilityStatus::Available
            } else {
                CapabilityStatus::Unavailable("DirectWrite initialization failed")
            }
        };

        #[cfg(not(windows))]
        let directwrite = CapabilityStatus::NotCompiled;

        #[cfg(windows)]
        let (mono_directwrite, color_directwrite) = if directwrite.is_available() {
            (
                CapabilityStatus::Available,
                CapabilityStatus::Unavailable("Requires layer traversal (deferred)"),
            )
        } else {
            (
                CapabilityStatus::Unavailable("DirectWrite initialization failed"),
                CapabilityStatus::Unavailable("Requires layer traversal (deferred)"),
            )
        };

        #[cfg(not(windows))]
        let (mono_directwrite, color_directwrite) =
            (CapabilityStatus::NotCompiled, CapabilityStatus::NotCompiled);

        Self {
            directwrite,
            cosmic: CapabilityStatus::Available,
            sdf: CapabilityStatus::Available,
            mtsdf: CapabilityStatus::Unavailable("Pipeline not wired up"),
            subpixel_phase_8: CapabilityStatus::Available,
            subpixel_phase_4: CapabilityStatus::Available,
            subpixel_phase_1: CapabilityStatus::Available,
            color_glyphs: CapabilityStatus::Available,
            mono_fallback: CapabilityStatus::Available,
            mono_directwrite,
            color_directwrite,
            hinting: CapabilityStatus::Unavailable("Hinting is not supported by current backends"),
            subpixel_antialiasing: CapabilityStatus::Unavailable(
                "Mutually exclusive with transparent layers and background blending",
            ),
        }
    }

    /// 查询特定光栅化后端的可用性。
    #[must_use]
    pub fn backend_status(&self, backend: RasterBackend) -> CapabilityStatus {
        match backend {
            RasterBackend::DirectWrite => self.directwrite,
            RasterBackend::Cosmic => self.cosmic,
        }
    }

    /// 查询特定相位档位的可用性。
    #[must_use]
    pub fn phase_mode_status(&self, mode: PhaseMode) -> CapabilityStatus {
        match mode {
            PhaseMode::Eight => self.subpixel_phase_8,
            PhaseMode::Four => self.subpixel_phase_4,
            PhaseMode::One => self.subpixel_phase_1,
        }
    }
}
