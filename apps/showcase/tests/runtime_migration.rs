//! 通过通用应用运行时驱动 showcase。
//!
//! 窗口宿主不再拥有场景 diff：`AppRuntime` 负责重建帧、将其与上一帧做 diff
//! 并报告上传计划。这些测试在无 GPU 情况下精确验证该路径。
//!

use unbound_app::AppRuntime;
use unbound_math::Vec2;
use unbound_widgets::diff::SceneDiff;

use showcase::pages::PageId;
use showcase::runtime_app::GalleryRuntime;

fn runtime() -> AppRuntime<GalleryRuntime> {
    AppRuntime::new(GalleryRuntime::new(), Vec2::new(1280.0, 800.0))
}

#[test]
fn the_first_frame_builds_the_whole_scene() {
    let mut runtime = runtime();
    let report = runtime.frame(0.0);
    assert!(report.primitives > 0, "the shell painted nothing");
    assert_eq!(
        report.diff,
        SceneDiff::Whole,
        "the first frame has no predecessor to patch"
    );
    assert!(
        !runtime.scene().focus_order.is_empty(),
        "the tab order must reach the runtime"
    );
    assert!(
        runtime.scene().semantics.pending() > 0,
        "the semantic tree must reach the runtime"
    );
}

#[test]
fn a_frame_without_changes_uploads_nothing() {
    let mut runtime = runtime();
    let _ = runtime.frame(0.0);
    let idle = runtime.frame(0.0);
    assert_eq!(idle.diff, SceneDiff::Identical);
    assert_eq!(idle.bytes, 0);
}

#[test]
fn navigating_rebuilds_and_reports_a_new_scene() {
    let mut runtime = runtime();
    let _ = runtime.frame(0.0);
    runtime
        .application_mut()
        .shell_mut()
        .navigate(PageId::Telemetry);
    // 宿主在 reducer 之外做出的变更必须显式宣告，就像窗口外壳对缩放和窗口外框所做的那样。
    //
    runtime.invalidate();
    let moved = runtime.frame(0.0);
    assert_ne!(moved.diff, SceneDiff::Identical);
    assert!(moved.primitives > 0);
}

#[test]
fn zooming_reflows_for_the_smaller_logical_viewport() {
    let mut runtime = runtime();
    let _ = runtime.frame(0.0);
    let wide = runtime.application().shell().layout.content.size.x;

    // 放大时物理尺寸除以有效缩放，因此宿主交给运行时一个更小的逻辑视口。
    // 页面必须按该视口重新布局，而不是保留旧几何并发生溢出。
    //
    runtime.resize(Vec2::new(640.0, 400.0));
    let _ = runtime.frame(0.0);
    let narrow = runtime.application().shell().layout.content.size.x;

    assert!(narrow < wide, "the page did not reflow: {narrow} >= {wide}");
    assert!(
        (runtime.application().shell().layout.viewport.size.x - 640.0).abs() < 1.0,
        "the shell laid out for a stale viewport"
    );
}

#[test]
fn an_accessibility_click_reaches_the_shell() {
    let mut runtime = runtime();
    let _ = runtime.frame(0.0);
    // 导航行注册了稳定标识；即使从未发生过指针事件，点击 Telemetry 行也必须触发导航。
    //
    let node = showcase::target::id(showcase::target::Target::NavPage(
        PageId::Telemetry.index() as u8
    ));
    runtime.a11y(node, unbound_a11y::A11yAction::Click, None);
    let _ = runtime.frame(0.0);
    assert_eq!(
        runtime.application().shell().state.active_page,
        PageId::Telemetry
    );
}
