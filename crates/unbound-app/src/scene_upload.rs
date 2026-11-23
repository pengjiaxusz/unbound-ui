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
            corner_radii: [
                b.corner_radii[0] * effective_scale,
                b.corner_radii[1] * effective_scale,
                b.corner_radii[2] * effective_scale,
                b.corner_radii[3] * effective_scale,
            ],
            layer_key_prefix: (PassLayer::Overlay as u64) << 56,
            split_z: b.z,
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

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_math::Rect;

    #[test]
    fn collect_backdrops_scales_blur_and_corner_radii_independently() {
        let widget_req = WidgetBackdrop::new(7, Rect::new(10.0, 20.0, 200.0, 120.0), 24.0)
            .expect("backdrop request")
            .with_corner_radii([8.0, 8.0, 8.0, 8.0])
            .with_z(1100);
        let gpu_reqs = collect_backdrops(&[widget_req], 1.5);
        assert_eq!(gpu_reqs.len(), 1);
        let req = gpu_reqs[0];
        assert_eq!(req.rect, [15.0, 30.0, 300.0, 180.0]);
        assert_eq!(req.radius, 36.0);
        assert_eq!(req.corner_radii, [12.0, 12.0, 12.0, 12.0]);
        assert_eq!(req.split_z, 1100);
    }
}
