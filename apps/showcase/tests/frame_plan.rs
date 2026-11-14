//! 帧计划：帧的呈现尺寸以及它上传的内容。
//!
//! 这里包含两条不变量，两者在实践中都曾被违反过：
//!
//! * 布局跟随**已提交的 swapchain 范围（extent）**，而非窗口尺寸。
//!   驱动决定图像大小，合成器显示的是图像；按窗口尺寸构建的布局会在右侧/底部
//!   留下空白带或被裁剪掉。
//!
//! * **被丢弃**的帧（已重建但未绘制）通过**全量**上传来修复。
//!   运行时每次重建都会推进 diff 基准，因此丢帧后缓冲池落后于基准：
//!   若只修补 diff 切片会写错字节位置，且下一帧内容相同时会报告“无变化”并完全不上传。
//!
//!

use unbound_app::AppRuntime;
use unbound_math::Vec2;
use unbound_widgets::diff::SceneDiff;

use showcase::frame_plan::{frame_size, needs_rebuild, upload_plan};
use showcase::pages::PageId;
use showcase::runtime_app::GalleryRuntime;

fn runtime() -> AppRuntime<GalleryRuntime> {
    AppRuntime::new(GalleryRuntime::new(), Vec2::new(1280.0, 800.0))
}

/// 布局视口由帧实际绘制的范围推导。
///
/// 呈现链返回驱动自身的尺寸，可能与请求尺寸相差一像素。
/// 页面必须按图像实际拥有的尺寸进行布局。
///
#[test]
fn the_layout_follows_the_committed_extent() {
    let size = frame_size((1920, 1040), 1.0);
    assert_eq!(size.physical, (1920, 1040));
    assert_eq!(size.logical, (1920.0, 1040.0));

    // 缩放会除物理尺寸：布局缩小，帧尺寸不变。
    //
    let zoomed = frame_size((1920, 1040), 2.0);
    assert_eq!(zoomed.physical, (1920, 1040));
    assert_eq!(zoomed.logical, (960.0, 520.0));
}

/// 窗口请求与呈现链的实际范围绝不互相比较。
///
/// 驱动会将呈现链钳制到自身限制并上报其视角下的窗口尺寸，因此两者合法存在差异。
/// 比较两者会导致每帧都重建呈现链——且永不收敛。
///
#[test]
fn a_one_pixel_difference_does_not_rebuild_forever() {
    assert!(
        !needs_rebuild((1920, 1040), (1920, 1040), false),
        "the request did not change, so the chain is fine"
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

/// 被丢弃的帧通过全量上传修复。
///
/// 当被丢弃的帧重建时，运行时的 diff 基准已前移，因此缓冲池落后于它。
/// 下一次上传必须携带整个场景——修补切片会写错位置，
/// 再下一帧则会报告“完全相同”从而什么都不上传。
///
#[test]
fn a_dropped_frame_is_repaired_by_a_full_upload() {
    let mut runtime = runtime();
    let first = runtime.frame(0.0);
    assert_eq!(
        first.diff,
        SceneDiff::Whole,
        "the first frame has no predecessor"
    );

    // 帧已重建但未绘制：运行时的基准已前移，GPU 的缓冲池没有。
    //
    runtime
        .application_mut()
        .shell_mut()
        .navigate(PageId::Telemetry);
    runtime.invalidate();
    let dropped = runtime.frame(0.0);
    assert_ne!(dropped.diff, SceneDiff::Identical);

    // 下一帧将显示为“完全相同”——基准已经移动过了。
    let next = runtime.frame(0.0);
    assert_eq!(
        next.diff,
        SceneDiff::Identical,
        "the base moved on when the frame was rebuilt"
    );

    // 此处若只修补切片，会把丢弃帧的内容永久留在屏幕上。
    // 计划必须是全量上传。
    let plan = upload_plan(7, &next.diff, true);
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

/// 页面按帧实际绘制的尺寸重新排版。
///
/// 宿主将从已提交范围推导出的逻辑视口交给运行时；
/// 页面必须按*该*视口布局，而非按窗口视口。
///
#[test]
fn the_page_reflows_for_the_presented_size() {
    let mut runtime = runtime();
    let _ = runtime.frame(0.0);
    let wide = runtime.application().shell().layout.content.size.x;

    // 窗口已最大化：呈现链现在返回更宽的范围，逻辑视口随之跟随。
    //
    let presented = frame_size((2560, 1400), 1.0);
    runtime.resize(Vec2::new(presented.logical.0, presented.logical.1));
    let _ = runtime.frame(0.0);
    let wider = runtime.application().shell().layout.content.size.x;

    assert!(
        wider > wide,
        "the page did not reflow for the new size: {wider} <= {wide}"
    );
    assert!(
        (runtime.application().shell().layout.viewport.size.x - 2560.0).abs() < 1.0,
        "the shell laid out for a stale viewport"
    );
}
