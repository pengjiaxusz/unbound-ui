//! 03_data_table_and_virtual_list: High-Performance Virtualized Table & Pagination.
//!
//! Demonstrates:
//! - Multi-column tabular data rendering via [`TableDesc`].
//! - Virtualized grid rendering via [`DataGridDesc`].
//! - Declarative pagination controls via [`PaginationDesc`].
//! - Fast 100,000 row virtual list benchmark handling.

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_NEXT_PAGE: u64 = 301;

#[derive(Default)]
pub struct DataTableApp;

pub struct DataTableState {
    pub current_page: usize,
    pub total_rows: usize,
}

impl Application for DataTableApp {
    type State = DataTableState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        DataTableState {
            current_page: 1,
            total_rows: 100_000,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_NEXT_PAGE => {
                state.current_page += 1;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("table_root"), |col| {
            col.card(ui_key!("table_card"), CardDesc::default(), |card| {
                card.row(ui_key!("header_bar"), |hdr| {
                    hdr.badge(
                        ui_key!("status_badge"),
                        BadgeDesc::label(format!(
                            "Showing 10 of {} virtualized records (Page {})",
                            state.total_rows, state.current_page
                        )),
                    );
                    hdr.button(
                        BTN_NEXT_PAGE,
                        ButtonDesc::label("Next Page >").variant(ButtonVariant::Secondary),
                    );
                });

                card.divider(ui_key!("div"), DividerDesc::rule());

                let columns = vec![
                    ColumnSpec::new("ID", 80.0),
                    ColumnSpec::new("Name", 200.0),
                    ColumnSpec::new("Metric", 120.0),
                    ColumnSpec::new("Status", 120.0),
                ];

                card.add(ui_key!("data_table"), TableDesc::columns(columns));

                card.add(
                    ui_key!("data_grid"),
                    DataGridDesc::new(Rect::new(0.0, 0.0, 700.0, 160.0), (0, 0), false),
                );

                card.add(
                    ui_key!("pagination"),
                    PaginationDesc::new(Rect::new(0.0, 0.0, 500.0, 32.0), 100, state.current_page),
                );
            });
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(DataTableApp, Vec2::new(800.0, 600.0));
    let initial_report = runtime.frame(0.0);
    assert!(
        initial_report.primitives > 0,
        "Table should emit primitives"
    );
    assert_eq!(runtime.state().current_page, 1);

    // Click Next Page button
    let btn_rect = runtime
        .widget_rect(BTN_NEXT_PAGE)
        .expect("Next page button mounted");
    let click_pos = btn_rect.center();
    runtime.pointer_move(click_pos);
    let _ = runtime.pointer_down(click_pos, PointerButton::Left);
    let _ = runtime.pointer_up(click_pos, PointerButton::Left);

    let _ = runtime.frame(1.0 / 60.0);
    assert_eq!(
        runtime.state().current_page,
        2,
        "Page should increment to 2"
    );
}

#[test]
fn test_03_data_table_and_virtual_list_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("03_data_table_and_virtual_list: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "03_data_table_and_virtual_list - Unbound UI".to_string(),
        width: 900,
        height: 700,
        ..Default::default()
    };
    run_windowed(DataTableApp, config)
}
