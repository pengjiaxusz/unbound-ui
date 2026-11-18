//! 后景计划到宿主系统材质请求的翻译。
//!
//! 后景计划只说「要哪一档材质」，宿主才说「向系统要哪个取值」：这段翻译必须只有
//! 一处。启动路径与运行期切换各写一份规则，规则迟早漂移——启动路径当年只会请求
//! 标准云母，于是亚克力在首帧之前拿不到自己的系统取值。
//!
//! 失焦锁定跟随实际生效的材质档：原生材质在场时，失焦不得让窗口退化为纯色；
//! 自绘路径与「无后景」都不锁定。

use unbound_backdrop::{BackdropPlan, BackdropSource, DwmMaterial, MaterialKind};
use unbound_platform::WindowBackdropMaterial;

/// 把后景计划翻译成窗口的系统材质请求与失焦锁定状态。
///
/// 材质档与系统取值一一对应：跟随系统、标准云母、云母变体、亚克力各有各的取值。
/// 沉浸式后景的系统取值没有公开文档依据（issue #36 调研后再补），因此它不请求
/// 任何系统材质——计划给出的不透明清屏负责让窗口退回不透明根背景。
#[must_use]
pub fn window_backdrop_request(plan: &BackdropPlan) -> (WindowBackdropMaterial, bool) {
    let material = match plan.material {
        DwmMaterial::None => WindowBackdropMaterial::None,
        DwmMaterial::System => WindowBackdropMaterial::System,
        DwmMaterial::Mica => WindowBackdropMaterial::Mica,
        // 云母变体走标签页材质：系统取值 4 即 Mica Alt。
        DwmMaterial::MicaAlt => WindowBackdropMaterial::Tabbed,
        DwmMaterial::Acrylic => WindowBackdropMaterial::Acrylic,
        // 沉浸式后景当前没有系统取值（issue #36）：不请求任何材质，窗口保持不透明。
        DwmMaterial::Immersive => WindowBackdropMaterial::None,
    };
    let lock_mica =
        plan.source == BackdropSource::Native && plan.material_kind != MaterialKind::None;
    (material, lock_mica)
}
