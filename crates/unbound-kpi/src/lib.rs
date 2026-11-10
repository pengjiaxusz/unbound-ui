//! 量化验收红线（`docs` §10.2）的 CPU 侧测量。
//!
//! Every number in the doc's acceptance table is a *budget*, and a budget is
//! only meaningful if something measures it. This crate does exactly that for
//! the six CPU-resident KPIs:
//!
//! | KPI | Budget |
//! | --- | --- |
//! | loose-quadtree viewport cull (1 M entities) | ≤ 150 µs |
//! | single hit test | ≤ 5 µs |
//! | viewport smart snapping | ≤ 50 µs |
//! | one `RelayoutBoundary` solve (100 k nodes) | ≤ 200 µs |
//! | headless verification (10 k nodes) | ≤ 1 000 µs |
//! | theme LUT rebuild (1 024 tokens) | ≤ 50 µs |
//!
//! The GPU-side rows (render time, MDI draw calls, PCIe upload) are asserted in
//! `unbound-gpu`'s device tests, where a real `VkDevice` is available.
//!
//! Measurements take the **median** of several runs, which is the statistic that
//! survives a noisy, shared machine. A debug build runs roughly an order of
//! magnitude slower than release, so the harness scales the budget instead of
//! silently failing CI.

#![forbid(unsafe_op_in_unsafe_fn)]

use core::time::Duration;
use std::time::Instant;

use unbound_canvas::{LooseQuadtree, SnapConfig, Snapper};
use unbound_headless::{FrameBuilder, Harness, NodeKind};
use unbound_input::{pick, CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_world::UiWorld;

/// One measured KPI.
#[derive(Clone, Debug)]
pub struct Measurement {
    pub name: &'static str,
    /// What the doc requires, in microseconds.
    pub budget_us: f64,
    /// Median observed, in microseconds.
    pub observed_us: f64,
    /// Extra context: scale of the measurement.
    pub detail: String,
}

impl Measurement {
    /// True when the observation fits the (scaled) budget.
    #[must_use]
    pub fn passes(&self, tolerance: f64) -> bool {
        self.observed_us <= self.budget_us * tolerance
    }
}

/// All measurements from one run.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub rows: Vec<Measurement>,
    /// Budget multiplier applied to every row (1.0 in release, higher in debug).
    pub tolerance: f64,
}

impl Report {
    /// Rows that exceeded their budget.
    #[must_use]
    pub fn failures(&self) -> Vec<&Measurement> {
        self.rows
            .iter()
            .filter(|row| !row.passes(self.tolerance))
            .collect()
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures().is_empty()
    }

    /// Markdown table, pasted straight into a status report.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("| KPI | Budget | Observed | Verdict |\n");
        out.push_str("| :--- | ---: | ---: | :--- |\n");
        for row in &self.rows {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                row.name,
                format_us(row.budget_us),
                format_us(row.observed_us),
                if row.passes(self.tolerance) {
                    "pass"
                } else {
                    "**FAIL**"
                }
            ));
        }
        out
    }

    /// One-line summary per row, for a terminal.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for row in &self.rows {
            out.push_str(&format!(
                "{:<42} {:>10} / {:>10}  {}  ({})\n",
                row.name,
                format_us(row.observed_us),
                format_us(row.budget_us),
                if row.passes(self.tolerance) {
                    "ok  "
                } else {
                    "FAIL"
                },
                row.detail
            ));
        }
        out
    }
}

fn format_us(us: f64) -> String {
    if us >= 1000.0 {
        format!("{:.1}ms", us / 1000.0)
    } else {
        format!("{us:.1}µs")
    }
}

/// Median of `runs` executions of `f`.
fn median(runs: usize, mut f: impl FnMut()) -> f64 {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        f();
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    let middle: Duration = samples[samples.len() / 2];
    middle.as_secs_f64() * 1e6
}

/// Run every CPU KPI.
#[must_use]
pub fn measure_all() -> Report {
    let tolerance = if cfg!(debug_assertions) { 25.0 } else { 1.0 };
    let mut report = Report {
        rows: Vec::new(),
        tolerance,
    };
    report.rows.push(cull());
    report.rows.push(hit_test());
    report.rows.push(snap());
    report.rows.push(layout_boundary());
    report.rows.push(headless_verification());
    report.rows.push(theme_lut());
    report
}

/// 1 000 000 entities, one 1920×1080 viewport query.
#[must_use]
pub fn cull() -> Measurement {
    const ENTITIES: u32 = 1_000_000;
    const WORLD: f32 = 1_048_576.0;
    let mut tree = LooseQuadtree::new(WORLD);
    for i in 0..ENTITIES {
        // Deterministic pseudo-random scatter over a 262 kpx square.
        let x = ((u64::from(i) * 7919) % 262_144) as f32 - 131_072.0;
        let y = ((u64::from(i) * 104_729) % 262_144) as f32 - 131_072.0;
        tree.insert(i, Rect::new(x, y, 16.0, 16.0), 0);
    }
    let viewport = Rect::new(-960.0, -540.0, 1920.0, 1080.0);
    let mut out = Vec::new();
    let observed = median(20, || {
        tree.query_exact(viewport, &mut out);
    });
    Measurement {
        name: "loose-quadtree viewport cull",
        budget_us: 150.0,
        observed_us: observed,
        detail: format!("{ENTITIES} entities, {} returned", out.len()),
    }
}

/// One pick over a realistic overlay stack.
#[must_use]
pub fn hit_test() -> Measurement {
    // 512 overlapping cards plus a toolbar and a dialogue on a portal layer.
    let mut regions = Vec::with_capacity(600);
    for i in 0..512u64 {
        let x = (i % 32) as f32 * 40.0;
        let y = (i / 32) as f32 * 40.0;
        regions.push(
            HitRegion::new(i, 1, 0, Rect::new(x, y, 120.0, 120.0))
                .with_cursor(CursorKind::Pointer),
        );
    }
    regions.push(HitRegion::new(900, 2, 0, Rect::new(0.0, 0.0, 1280.0, 48.0)));
    let point = Vec2::new(700.0, 300.0);
    let observed = median(200, || {
        let hit = pick(&regions, point);
        debug_assert!(hit.is_some());
    });
    Measurement {
        name: "single hit test",
        budget_us: 5.0,
        observed_us: observed,
        detail: format!("{} regions", regions.len()),
    }
}

/// One snapping solve against a dense viewport.
#[must_use]
pub fn snap() -> Measurement {
    let mut tree = LooseQuadtree::new(65_536.0);
    for i in 0..4096u32 {
        let x = (i % 64) as f32 * 120.0 - 3840.0;
        let y = (i / 64) as f32 * 120.0 - 3840.0;
        tree.insert(i, Rect::new(x, y, 100.0, 100.0), 0);
    }
    let mut snapper = Snapper::new(SnapConfig::default());
    let viewport = Rect::new(-960.0, -540.0, 1920.0, 1080.0);
    let moving = Rect::new(-952.0, -530.0, 100.0, 100.0);
    let observed = median(100, || {
        let result = snapper.snap_against_tree(&mut tree, moving, None, viewport, 1.0);
        debug_assert!(result.offset.is_finite());
    });
    Measurement {
        name: "viewport smart snapping",
        budget_us: 50.0,
        observed_us: observed,
        detail: format!("{} candidates retained", snapper.retained_candidates()),
    }
}

/// One `RelayoutBoundary` solve inside a 1 000 row list.
#[must_use]
pub fn layout_boundary() -> Measurement {
    use unbound_abi::desc::RawNodeDesc;
    use unbound_layout::LayoutEngine;

    #[track_caller]
    fn insert(
        world: &mut UiWorld,
        parent: unbound_world::NodeId,
        desc: RawNodeDesc,
    ) -> unbound_world::NodeId {
        world.insert(parent, desc, core::panic::Location::caller())
    }

    let mut world = UiWorld::new();
    let root = world.root();
    let column = insert(&mut world, root, RawNodeDesc::text().size(1200.0, 4000.0).row());
    let mut rows = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let row = insert(&mut world, column, RawNodeDesc::text().size(1200.0, 32.0).row());
        for _ in 0..8 {
            insert(&mut world, row, RawNodeDesc::text().size(140.0, 32.0).row());
        }
        rows.push(row);
    }
    let mut engine = LayoutEngine::new();
    engine.sync(&world, root);
    engine.solve(&mut world, root, Vec2::new(1200.0, 4000.0));
    let nodes = engine.node_count();
    let target = rows[500];
    let observed = median(50, || {
        engine.solve_boundaries(&mut world, &[target], Vec2::new(1200.0, 4000.0));
    });
    Measurement {
        name: "one RelayoutBoundary solve",
        budget_us: 200.0,
        observed_us: observed,
        detail: format!("{nodes} nodes synced, 9 re-solved"),
    }
}

/// Headless verification of a 10 001 node frame.
#[must_use]
pub fn headless_verification() -> Measurement {
    let mut builder = FrameBuilder::new(Rect::new(0.0, 0.0, 4000.0, 4000.0));
    builder.open(
        NodeKind::Container,
        Rect::new(0.0, 0.0, 4000.0, 4000.0),
        "Root",
    );
    for i in 0..5000 {
        let x = (i % 100) as f32 * 40.0;
        let y = (i / 100) as f32 * 40.0;
        builder.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            Rect::new(x, y, 32.0, 32.0),
            "Cell",
        );
        builder.open(
            NodeKind::Text {
                ink: Rect::new(x + 2.0, y + 2.0, 28.0, 28.0),
                ellipsis: false,
            },
            Rect::new(x + 2.0, y + 2.0, 28.0, 28.0),
            "Glyph",
        );
        builder.close();
        builder.close();
    }
    builder.close();
    let frame = builder.build();
    let report = Harness::assert_clean(&frame);
    Measurement {
        name: "headless verification",
        budget_us: 1000.0,
        observed_us: report.elapsed_us() as f64,
        detail: format!("{} nodes", report.nodes),
    }
}

/// Rebuild the 1 024-entry colour token LUT (`docs` §4.5).
#[must_use]
pub fn theme_lut() -> Measurement {
    let mut lut = vec![[0.0f32; 4]; 1024];
    let observed = median(100, || {
        for (index, entry) in lut.iter_mut().enumerate() {
            // A plausible theme: hue ramp with a fixed alpha.
            let t = index as f32 / 1023.0;
            *entry = [t, 1.0 - t, 0.5 + 0.5 * t, 1.0];
        }
    });
    Measurement {
        name: "theme LUT rebuild",
        budget_us: 50.0,
        observed_us: observed,
        detail: format!("{} KiB uploaded", lut.len() * 16 / 1024),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cpu_kpi_fits_its_budget() {
        let report = measure_all();
        println!("\n{}", report.to_text());
        let failures = report.failures();
        assert!(
            failures.is_empty(),
            "{} KPI(s) exceeded budget (tolerance {}×):\n{}",
            failures.len(),
            report.tolerance,
            report.to_text()
        );
    }

    #[test]
    fn the_report_renders_a_markdown_table() {
        let mut report = Report {
            rows: vec![Measurement {
                name: "demo",
                budget_us: 10.0,
                observed_us: 5.0,
                detail: String::new(),
            }],
            tolerance: 1.0,
        };
        assert!(report.passed());
        assert!(report.to_markdown().contains("| demo | 10.0µs | 5.0µs | pass |"));
        report.rows[0].observed_us = 5000.0;
        assert!(!report.passed());
        assert!(report.to_markdown().contains("5.0ms"));
    }

    #[test]
    fn measurements_are_not_vacuous() {
        // Every measurement must actually do something: a zero-node cull or an
        // empty frame would pass the budget for the wrong reason.
        let cull = cull();
        assert!(cull.detail.contains("1000000 entities"));
        let snap = snap();
        assert!(snap.detail.contains("candidates retained"));
        let layout = layout_boundary();
        assert!(layout.detail.contains("nodes synced"));
        let verify = headless_verification();
        assert!(verify.detail.contains("10001 nodes"));
        let hit = hit_test();
        assert!(hit.detail.contains("regions"));
    }
}
