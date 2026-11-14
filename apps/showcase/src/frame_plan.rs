//! 帧计划：帧的呈现尺寸以及渲染器需要上传的内容。
//!
//!
//! 这里包含两条规则，它们都源于窗口与 swapchain 回答的是不同问题：
//!
//! * **呈现尺寸即已提交的 swapchain 范围（extent）。** 驱动决定 swapchain 图像的尺寸——
//!   `resolve_extent` 优先采用驱动的 `currentExtent` 而非请求尺寸——且合成器最终显示的是该图像。
//!   因此布局必须跟随该范围，而非窗口尺寸：若驱动返回了不同的窗口尺寸，
//!   UI 会按一种尺寸布局却投影到另一种尺寸上，导致右侧/底部出现空白带或内容被裁剪。
//! * **重建帧即全量上传帧。** 运行时每次重建都会推进 diff 基准，无论该帧是否真正到达 GPU。
//!   因此若某帧被丢弃，缓冲池就会落后于基准，下一次上传必须是全量上传——
//!   否则被丢弃的切片永远无法修复，并会永久残留在屏幕上。
//!
//!
//!
//!
//!

use unbound_widgets::diff::SceneDiff;

/// 帧的呈现尺寸。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FrameSize {
    /// 已提交的 swapchain 范围，单位为物理像素。
    pub physical: (u32, u32),
    /// 布局视口，单位为逻辑单位。
    pub logical: (f32, f32),
}

/// 从已提交的 swapchain 范围推导布局视口。
///
/// 缩放比例为有效缩放（`OS scale × app zoom`）：布局在逻辑单位下工作，
/// 相机再将其放大回物理像素。
#[must_use]
pub fn frame_size(chain_extent: (u32, u32), scale: f32) -> FrameSize {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let physical = (chain_extent.0.max(1), chain_extent.1.max(1));
    FrameSize {
        physical,
        logical: (physical.0 as f32 / scale, physical.1 as f32 / scale),
    }
}

/// 是否需要重建 swapchain。
///
/// 两个尺寸均为**请求**尺寸：窗口当前请求的尺寸与上次创建 swapchain 时请求的尺寸。
/// swapchain 的*实际*范围由驱动返回，合法情况下可能与请求不同——
/// 驱动会将其钳制到自身限制并上报其视角下的窗口尺寸。
/// 若拿实际范围与请求尺寸比较，会导致每帧都重建 swapchain 且永不收敛，
/// 还会把属于一种尺寸的布局呈现到另一种尺寸的图像中。
///
///
#[must_use]
pub fn needs_rebuild(requested: (u32, u32), committed_request: (u32, u32), dirty: bool) -> bool {
    dirty || requested != committed_request
}

/// 渲染器在单帧中需要上传的内容。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UploadPlan<'a> {
    /// 渲染器用于与自身对比的代次（generation）。
    pub generation: u64,
    /// 需要修补的切片；空列表表示“全量上传”。
    pub ranges: &'a [(u32, u32)],
}

/// 即将绘制的这一帧的上传计划。
///
/// `dropped` 表示**上一帧**已重建但未能到达 GPU（swapchain 过期、渲染失败或环形缓冲耗尽）。
/// 运行时的 diff 基准仍已前移，因此缓冲池落后于基准：
/// 若只修补 diff 切片会写错字节位置，且下一帧内容相同时会误报“无变化”从而完全不上传。
/// 推进代次可将下一次上传转为全量上传，这就是丢帧的代价。
///
///
///
#[must_use]
pub fn upload_plan<'a>(generation: u64, diff: &'a SceneDiff, dropped: bool) -> UploadPlan<'a> {
    let mut generation = generation;
    if diff.bumps_generation() {
        generation = generation.wrapping_add(1);
    }
    if dropped {
        return UploadPlan {
            generation: generation.wrapping_add(1),
            ranges: &[],
        };
    }
    UploadPlan {
        generation,
        ranges: diff.ranges(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_widgets::diff::SceneDiff;

    /// 布局视口必须由 swapchain 范围推导，绝不直接取窗口尺寸。
    #[test]
    fn the_layout_follows_the_committed_extent() {
        let size = frame_size((1919, 1039), 1.5);
        assert_eq!(size.physical, (1919, 1039));
        assert!((size.logical.0 - 1919.0 / 1.5).abs() < 1e-4);
        assert!((size.logical.1 - 1039.0 / 1.5).abs() < 1e-4);
    }

    /// 退化的缩放比例不得产生退化的视口。
    #[test]
    fn a_broken_scale_falls_back_to_one() {
        let size = frame_size((800, 600), 0.0);
        assert_eq!(size.logical, (800.0, 600.0));
        let size = frame_size((800, 600), f32::NAN);
        assert_eq!(size.logical, (800.0, 600.0));
    }

    /// 与请求相差一个像素的 swapchain 不应无限重建。
    #[test]
    fn a_one_pixel_difference_does_not_rebuild_forever() {
        assert!(
            !needs_rebuild((1920, 1040), (1920, 1040), false),
            "the request did not change"
        );
        assert!(
            needs_rebuild((1921, 1040), (1920, 1040), false),
            "the window asked for a different size"
        );
        assert!(
            needs_rebuild((1920, 1040), (1920, 1040), true),
            "the driver asked for a rebuild"
        );
    }

    /// 丢帧必须使下一次上传变为全量上传。
    #[test]
    fn a_dropped_frame_forces_a_full_upload() {
        let diff = SceneDiff::Ranges(vec![(3, 4)]);
        let plan = upload_plan(7, &diff, true);
        assert!(
            plan.ranges.is_empty(),
            "a patch would repair the wrong slices"
        );
        assert_ne!(
            plan.generation, 7,
            "the generation has to move so the renderer re-uploads everything"
        );
    }

    /// 成功到达 GPU 的帧保持低开销增量路径。
    #[test]
    fn an_uploaded_frame_keeps_its_patch() {
        let diff = SceneDiff::Ranges(vec![(3, 4)]);
        let plan = upload_plan(7, &diff, false);
        assert_eq!(plan.ranges, &[(3, 4)]);
        assert_eq!(plan.generation, 7);
    }

    /// 形状变化总是推进代次：缓冲池必须从头重写。
    ///
    #[test]
    fn a_shape_change_moves_the_generation() {
        let plan = upload_plan(7, &SceneDiff::Whole, false);
        assert_eq!(plan.generation, 8);
        assert!(plan.ranges.is_empty());
    }
}
