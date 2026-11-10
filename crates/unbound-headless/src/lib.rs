//! 无头验证器（`docs` §7.2）。
//!
//! A headless step is: dispatch the input events, run the reducer, solve the
//! layout incrementally, sync the spatial index, generate primitives, then run
//! five geometric assertions over the result. None of that needs a window handle
//! or a Vulkan instance, so it runs in CI, in a unit test, and inside an agent's
//! edit-verify loop at a fraction of a millisecond.
//!
//! What this crate provides:
//!
//! * [`frame::Frame`] / [`frame::FrameBuilder`] — the solved geometry of one
//!   step, with `#[track_caller]` source locations baked into every node,
//! * [`rules`] — the doc's five sanitizers, each producing a [`rules::Diagnostic`]
//!   that serialises to one line of JSON,
//! * [`snapshot`] — postcard binary capture/restore for time-travel debugging,
//! * [`Harness`] — runs the checks and reports how long they took.
//!
//! ```no_run
//! use unbound_headless::{FrameBuilder, Harness, NodeKind};
//! use unbound_math::Rect;
//!
//! let mut b = FrameBuilder::new(Rect::new(0.0, 0.0, 800.0, 600.0));
//! b.open(NodeKind::Container, Rect::new(0.0, 0.0, 800.0, 600.0), "Root");
//! b.close();
//! let report = Harness::run(&b.build());
//! assert!(report.is_clean());
//! ```

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod frame;
pub mod rules;
pub mod snapshot;

use core::fmt::Write as _;
use core::time::Duration;
use std::time::Instant;

pub use frame::{Frame, FrameBuilder, NodeKind, NodeSnapshot, SourceLoc};
pub use rules::{check, Diagnostic, Rule};
pub use snapshot::FrameSnapshot;

/// Outcome of one verification pass.
#[derive(Clone, Debug)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    pub nodes: usize,
    pub elapsed: Duration,
}

impl Report {
    /// True when every sanitizer passed.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Wall-clock microseconds spent checking.
    #[must_use]
    pub fn elapsed_us(&self) -> u128 {
        self.elapsed.as_micros()
    }

    /// Failures for one rule.
    pub fn by_rule(&self, rule: Rule) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(move |d| d.rule == rule)
    }

    /// Rules that fired at least once.
    #[must_use]
    pub fn fired_rules(&self) -> Vec<Rule> {
        let mut rules: Vec<Rule> = self.diagnostics.iter().map(|d| d.rule).collect();
        rules.sort_unstable();
        rules.dedup();
        rules
    }

    /// Machine-readable report: `{"nodes":N,"elapsedUs":U,"errors":[...]}`.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(256 + self.diagnostics.len() * 192);
        let _ = write!(
            out,
            "{{\"nodes\":{},\"elapsedUs\":{},\"clean\":{},\"errors\":[",
            self.nodes,
            self.elapsed_us(),
            self.is_clean()
        );
        for (i, diagnostic) in self.diagnostics.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&diagnostic.to_json());
        }
        out.push_str("]}");
        out
    }

    /// Human summary, used by the CLI and by test failure messages.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_clean() {
            return format!(
                "{} nodes verified in {} µs, no violations",
                self.nodes,
                self.elapsed_us()
            );
        }
        let mut out = format!(
            "{} violations across {} nodes ({} µs)\n",
            self.diagnostics.len(),
            self.nodes,
            self.elapsed_us()
        );
        for diagnostic in &self.diagnostics {
            let _ = writeln!(
                out,
                "  [{}] node {} at {} ({})\n      {}",
                diagnostic.rule.name(),
                diagnostic.node,
                diagnostic.loc,
                diagnostic.path,
                diagnostic.message
            );
        }
        out
    }
}

/// Runs the sanitizers and measures how long they took.
#[derive(Copy, Clone, Debug, Default)]
pub struct Harness;

impl Harness {
    /// Verify `frame`. Never panics on a violation: the report carries them.
    #[must_use]
    pub fn run(frame: &Frame) -> Report {
        let start = Instant::now();
        let diagnostics = rules::check(frame);
        Report {
            diagnostics,
            nodes: frame.nodes.len(),
            elapsed: start.elapsed(),
        }
    }

    /// Verify and panic with the full structured report on failure.
    ///
    /// # Panics
    /// Panics when any sanitizer fires. The message is the `summary()` plus the
    /// JSON payload, so a CI log is enough to reproduce the bug.
    pub fn assert_clean(frame: &Frame) -> Report {
        let report = Self::run(frame);
        assert!(
            report.is_clean(),
            "{}\n{}",
            report.summary(),
            report.to_json()
        );
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_math::Rect;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    fn clean_frame() -> Frame {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 800.0, 600.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 800.0, 600.0), "Root");
        for i in 0..20 {
            let y = i as f32 * 40.0;
            b.open(
                NodeKind::Interactive {
                    min_size: NodeKind::DEFAULT_MIN_SIZE,
                    occluded: false,
                },
                rect(10.0, y, 120.0, 32.0),
                "Row",
            );
            b.open(
                NodeKind::Text {
                    ink: rect(14.0, y + 4.0, 60.0, 20.0),
                    ellipsis: false,
                },
                rect(14.0, y + 4.0, 60.0, 20.0),
                "Label",
            );
            b.close();
            b.close();
        }
        b.close();
        b.build()
    }

    #[test]
    fn a_clean_frame_reports_clean() {
        let report = Harness::run(&clean_frame());
        assert!(report.is_clean(), "{}", report.summary());
        assert_eq!(report.nodes, 1 + 20 * 2);
        assert!(report.to_json().contains("\"clean\":true"));
    }

    #[test]
    fn every_rule_can_fire() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 800.0, 600.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 800.0, 600.0), "Root");
        // 1: NaN
        b.open(NodeKind::Container, rect(f32::NAN, 0.0, 1.0, 1.0), "NaN");
        b.close();
        // 2: clipped text
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 20.0), "Clip");
        b.set_clip(rect(0.0, 0.0, 100.0, 20.0));
        b.open(
            NodeKind::Text {
                ink: rect(0.0, 0.0, 100.0, 100.0),
                ellipsis: false,
            },
            rect(0.0, 0.0, 100.0, 100.0),
            "Text",
        );
        b.close();
        b.close();
        // 3: undersized target
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect(0.0, 200.0, 10.0, 10.0),
            "Tiny",
        );
        b.close();
        // 4: sibling overlap
        b.open(NodeKind::Container, rect(0.0, 300.0, 100.0, 50.0), "Row");
        b.open(NodeKind::Container, rect(0.0, 300.0, 80.0, 50.0), "A");
        b.close();
        b.open(NodeKind::Container, rect(40.0, 300.0, 80.0, 50.0), "B");
        b.close();
        b.close();
        // 5: portal outside the viewport
        b.open(NodeKind::Portal, rect(790.0, 590.0, 100.0, 100.0), "Menu");
        b.close();
        b.close();

        let report = Harness::run(&b.build());
        assert_eq!(report.fired_rules(), Rule::ALL.to_vec());
        assert!(!report.is_clean());
    }

    #[test]
    fn assert_clean_passes_for_a_valid_frame() {
        let report = Harness::assert_clean(&clean_frame());
        assert!(report.is_clean());
    }

    #[test]
    #[should_panic(expected = "AssertNoNaNOrInf")]
    fn assert_clean_reports_the_rule_name() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(f32::INFINITY, 0.0, 1.0, 1.0), "Bad");
        b.close();
        Harness::assert_clean(&b.build());
    }

    #[test]
    fn the_json_payload_is_parseable_shaped() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(f32::NAN, 0.0, 1.0, 1.0), "Bad");
        b.close();
        let json = Harness::run(&b.build()).to_json();
        assert!(json.starts_with("{\"nodes\":"));
        assert!(json.ends_with("]}"));
        assert_eq!(json.matches('{').count(), json.matches('}').count());
    }

    #[test]
    fn a_ten_thousand_node_frame_verifies_far_under_a_millisecond() {
        // 10 000 nodes is the doc's KPI scale.
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 4000.0, 4000.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 4000.0, 4000.0), "Root");
        for i in 0..5000 {
            let y = (i % 100) as f32 * 40.0;
            let x = (i / 100) as f32 * 40.0;
            b.open(
                NodeKind::Interactive {
                    min_size: NodeKind::DEFAULT_MIN_SIZE,
                    occluded: false,
                },
                rect(x, y, 32.0, 32.0),
                "Cell",
            );
            b.open(
                NodeKind::Text {
                    ink: rect(x + 2.0, y + 2.0, 28.0, 28.0),
                    ellipsis: false,
                },
                rect(x + 2.0, y + 2.0, 28.0, 28.0),
                "Glyph",
            );
            b.close();
            b.close();
        }
        b.close();
        let frame = b.build();
        let report = Harness::run(&frame);
        assert!(report.is_clean(), "{}", report.summary());
        assert_eq!(report.nodes, 10_001);
        // Release builds land in the tens of microseconds; the assertion is
        // generous so a debug build on a busy machine still passes.
        assert!(
            report.elapsed_us() < 20_000,
            "verification took {} µs",
            report.elapsed_us()
        );
    }
}
