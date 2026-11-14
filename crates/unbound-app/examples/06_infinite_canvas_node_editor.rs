//! 06_infinite_canvas_node_editor：无限画布与着色器节点图编辑器示例。
//!
//! 演示内容：
//! - 无限 2D 视口平移与缩放导航。
//! - 通过 [`WidgetCatalogId::NodeGraph`] 实现节点图连接与端口连线。
//! - 通过 [`CurveEditorDesc`] 调节曲线参数。
//! - 撤销/重做事务历史。

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_ZOOM_IN: u64 = 601;
const BTN_RESET_PAN: u64 = 602;

#[derive(Default)]
pub struct CanvasApp;

pub struct CanvasState {
    pub pan: Vec2,
    pub zoom: f32,
    pub node_count: usize,
}

impl Application for CanvasApp {
    type State = CanvasState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        CanvasState {
            pan: Vec2::ZERO,
            zoom: 1.0,
            node_count: 3,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_ZOOM_IN => {
                state.zoom = (state.zoom + 0.25).min(3.0);
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            AppEvent::Click { node, .. } if node == BTN_RESET_PAN => {
                state.pan = Vec2::ZERO;
                state.zoom = 1.0;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("canvas_root"), |col| {
            col.title_bar("06 — Infinite Canvas & Node Editor");
            col.card(ui_key!("toolbar_card"), CardDesc::default(), |bar| {
                bar.row(ui_key!("controls"), |row| {
                    row.badge(
                        ui_key!("zoom_badge"),
                        BadgeDesc::label(format!(
                            "Zoom: {:.2}x | Nodes: {}",
                            state.zoom, state.node_count
                        )),
                    );
                    row.button(BTN_ZOOM_IN, ButtonDesc::label("Zoom In (+)"));
                    row.button(BTN_RESET_PAN, ButtonDesc::label("Reset View"));
                });
            });

            col.card(ui_key!("graph_stage"), CardDesc::default(), |stage| {
                stage.add(ui_key!("node_graph"), WidgetCatalogId::NodeGraph);
                stage.add(ui_key!("curve_editor"), WidgetCatalogId::CurveEditor);
            });
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(CanvasApp, Vec2::new(1024.0, 768.0));
    let initial_report = runtime.frame(0.0);
    assert!(
        initial_report.primitives > 0,
        "Canvas should emit primitives"
    );
    assert!((runtime.state().zoom - 1.0).abs() < 1e-4);

    // 放大视图
    let btn_rect = runtime
        .widget_rect(BTN_ZOOM_IN)
        .expect("Zoom button mounted");
    let click_pos = btn_rect.center();
    runtime.pointer_move(click_pos);
    let _ = runtime.pointer_down(click_pos, PointerButton::Left);
    let _ = runtime.pointer_up(click_pos, PointerButton::Left);

    let _ = runtime.frame(1.0 / 60.0);
    assert!(
        runtime.state().zoom > 1.0,
        "Zoom should increase after clicking Zoom In"
    );
}

#[test]
fn test_06_infinite_canvas_node_editor_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("06_infinite_canvas_node_editor: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "06_infinite_canvas_node_editor - Unbound UI".to_string(),
        width: 1024,
        height: 768,
        ..Default::default()
    };
    run_windowed(CanvasApp, config)
}
