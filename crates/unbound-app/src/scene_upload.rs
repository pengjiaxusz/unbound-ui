//! 场景上传：把逻辑像素的场景收集器转换为 GPU 上传描述。
//!
//! 窗口路径与离屏路径**必须**走同一份转换。若两边各写一份，产出的画面
//! 就会有系统性差异——离屏断言会拿窗口里永远不会出现的像素去对账。
//! 因此这段转换属于框架，而不是某个外壳的私有逻辑。

use unbound_gpu::data::PassLayer;
use unbound_gpu::frame::BackdropRequest;
use unbound_gpu::{Camera, SceneUpload};
use unbound_widgets::BackdropRequest as WidgetBackdrop;

/// 收集需要模糊背景的浮层请求，并换算为物理像素。
///
/// 控件侧的矩形是逻辑像素；GPU 侧的背板矩形是物理像素，两者相差一个
/// `effective_scale`。
#[must_use]
pub fn collect_backdrops(
    backdrops: &[WidgetBackdrop],
    effective_scale: f32,
) -> Vec<BackdropRequest> {
    backdrops
        .iter()
        .map(|b| BackdropRequest {
            rect: [
                b.rect.origin.x * effective_scale,
                b.rect.origin.y * effective_scale,
                b.rect.size.x * effective_scale,
                b.rect.size.y * effective_scale,
            ],
            radius: b.blur * effective_scale,
            corner_radii: [0.0; 4],
            layer_key_prefix: (PassLayer::Overlay as u64) << 56,
        })
        .collect()
}

/// 由场景收集器构造 GPU 上传描述。
///
/// `backdrops` 由调用方持有：[`SceneUpload`] 借用它，因此它不能是本函数
/// 内部的临时值——否则返回值会指向已经销毁的内存。
#[must_use]
pub fn scene_upload<'a>(
    scene: &'a unbound_widgets::SceneCollector,
    backdrops: &'a [BackdropRequest],
    camera: Camera,
    generation: u64,
    dirty_ranges: &'a [(u32, u32)],
    aux_changed: bool,
) -> SceneUpload<'a> {
    SceneUpload {
        primitives: &scene.primitives,
        transforms: &scene.transforms,
        clips: &scene.clips,
        payloads: &scene.payloads,
        generation,
        camera,
        key_prefix: (PassLayer::Ui as u64) << 56,
        dirty_ranges,
        aux_changed,
        backdrops,
    }
}
