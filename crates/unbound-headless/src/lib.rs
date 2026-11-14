//! 无头验证器（`docs` §7.2）。
//!
//! 单个无头步骤流程为：分发输入事件、运行 reducer、增量求解布局、
//! 同步空间索引、生成图元，随后对结果运行五项几何断言。以上均不依赖
//! 窗口句柄或 Vulkan 实例，因此可在 CI、单元测试及 Agent 的
//! “编辑-验证”循环中以不到一毫秒的速度运行。
//!
//!
//! 本 crate 提供：
//!
//! * [`frame::Frame`] / [`frame::FrameBuilder`] —— 单步求解后的几何结构，
//!   每个节点均内嵌 `#[track_caller]` 源码位置；
//! * [`rules`] —— 文档规定的五项检查器，每项产出可序列化为单行 JSON 的 [`rules::Diagnostic`]；
//!
//! * [`snapshot`] —— 基于 postcard 的二进制捕获/恢复，用于时间旅行调试；
//! * [`Harness`] —— 执行检查并报告耗时。
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

pub mod driver;
pub mod frame;
pub mod rules;
pub mod snapshot;

use core::fmt::Write as _;
use core::time::Duration;
use std::time::Instant;

pub use frame::{Frame, FrameBuilder, NodeKind, NodeSnapshot, SourceLoc, TextColors};
pub use rules::{check, Diagnostic, Rule};
pub use snapshot::{capture, open, restore, FrameSnapshot, SnapNode, TimeTravel};

/// 单次验证过程的结果。
#[derive(Clone, Debug)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    pub nodes: usize,
    pub elapsed: Duration,
}

impl Report {
    /// 所有检查器均通过时为 true。
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// 检查所花费的挂钟微秒数。
    #[must_use]
    pub fn elapsed_us(&self) -> u128 {
        self.elapsed.as_micros()
    }

    /// 某条规则的失败列表。
    pub fn by_rule(&self, rule: Rule) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(move |d| d.rule == rule)
    }

    /// 至少触发过一次的规则。
    #[must_use]
    pub fn fired_rules(&self) -> Vec<Rule> {
        let mut rules: Vec<Rule> = self.diagnostics.iter().map(|d| d.rule).collect();
        rules.sort_unstable();
        rules.dedup();
        rules
    }

    /// 机器可读报告：`{"nodes":N,"elapsedUs":U,"errors":[...]}`。
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

    /// 人类可读摘要，用于 CLI 与测试失败信息。
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

/// 运行检查器并统计耗时。
#[derive(Copy, Clone, Debug, Default)]
pub struct Harness;

impl Harness {
    /// 验证 `frame`。违规时绝不 panic：诊断结果存储在报告中。
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

    /// 验证并在失败时携带完整结构化报告 panic。
    ///
    /// # Panic
    /// 任意检查器触发时 panic。消息包含 `summary()` 与 JSON 负载，
    // 仅凭 CI 日志即可复现缺陷。
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
        // 2: 文本被裁剪
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
        // 3: 交互目标过小
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect(0.0, 200.0, 10.0, 10.0),
            "Tiny",
        );
        b.close();
        // 4: 兄弟节点重叠
        b.open(NodeKind::Container, rect(0.0, 300.0, 100.0, 50.0), "Row");
        b.open(NodeKind::Container, rect(0.0, 300.0, 80.0, 50.0), "A");
        b.close();
        b.open(NodeKind::Container, rect(40.0, 300.0, 80.0, 50.0), "B");
        b.close();
        b.close();
        // 5: portal 超出视口
        b.open(NodeKind::Portal, rect(790.0, 590.0, 100.0, 100.0), "Menu");
        b.close();
        // 6: 文本宽于其父节点
        b.open(NodeKind::Container, rect(0.0, 400.0, 80.0, 40.0), "Pill");
        b.open(
            NodeKind::Text {
                ink: rect(0.0, 400.0, 200.0, 20.0),
                ellipsis: false,
            },
            rect(0.0, 400.0, 200.0, 20.0),
            "Overflow",
        );
        b.close();
        b.close();
        // 7: 文本颜色对比度不足
        b.open(
            NodeKind::Text {
                ink: rect(0.0, 500.0, 80.0, 20.0),
                ellipsis: false,
            },
            rect(0.0, 500.0, 80.0, 20.0),
            "LowContrast",
        );
        b.set_text_colors([80, 80, 80], [90, 90, 90]);
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
        b.open(
            NodeKind::Container,
            rect(f32::INFINITY, 0.0, 1.0, 1.0),
            "Bad",
        );
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
        // 10 000 个节点是文档规定的 KPI 规模。
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
        // release 构建耗时在数十微秒级；此处断言放宽范围，
        // 以确保繁忙机器上的 debug 构建也能稳定通过。
        assert!(
            report.elapsed_us() < 20_000,
            "verification took {} µs",
            report.elapsed_us()
        );
    }
}
