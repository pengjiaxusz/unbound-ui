//! 五项布局检查器（`docs` §7.2）。
//!
//! 每条规则都是 [`Frame`] 的纯函数，因此仅凭二进制快照即可复现任何失败。
//! 诊断信息携带节点 ID、*构建节点的源码行*（`#[track_caller]`）、
//! 父节点链、实际矩形与语义路径——即文档中的“结构化 JSON 错误栈”——
//! 使 Agent 能直接定位到问题组件。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use core::fmt::Write as _;

use unbound_math::Rect;

use crate::frame::{Frame, NodeKind, SourceLoc};

/// 违反了哪项不变量。
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    /// `AssertNoNaNOrInf`
    NoNaNOrInf,
    /// `AssertTextNotClipped`
    TextNotClipped,
    /// `AssertInteractiveTargetSize`
    InteractiveTargetSize,
    /// `AssertNoUnintendedSiblingOverlap`
    NoSiblingOverlap,
    /// `AssertViewportContainment`
    ViewportContainment,
    /// `AssertTextNotOverflowParent`
    TextOverflow,
    /// `AssertTextContrast`
    TextContrast,
}

impl Rule {
    pub const ALL: [Self; 7] = [
        Self::NoNaNOrInf,
        Self::TextNotClipped,
        Self::InteractiveTargetSize,
        Self::NoSiblingOverlap,
        Self::ViewportContainment,
        Self::TextOverflow,
        Self::TextContrast,
    ];

    /// 文档中该断言的名称。
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NoNaNOrInf => "AssertNoNaNOrInf",
            Self::TextNotClipped => "AssertTextNotClipped",
            Self::InteractiveTargetSize => "AssertInteractiveTargetSize",
            Self::NoSiblingOverlap => "AssertNoUnintendedSiblingOverlap",
            Self::ViewportContainment => "AssertViewportContainment",
            Self::TextOverflow => "AssertTextNotOverflowParent",
            Self::TextContrast => "AssertTextContrast",
        }
    }
}

/// 单条失败的断言。
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub rule: Rule,
    pub node: u64,
    pub loc: SourceLoc,
    pub path: String,
    pub rect: Rect,
    /// 从根节点到当前节点的链条，便于查看父级约束。
    pub chain: Vec<u64>,
    pub message: String,
}

impl Diagnostic {
    /// 结构化单行 JSON。手写序列化使测试框架无需额外序列化依赖，
    /// 且输出稳定便于 diff 比对。
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push('{');
        let _ = write!(out, "\"rule\":\"{}\"", self.rule.name());
        let _ = write!(out, ",\"node\":{}", self.node);
        let _ = write!(out, ",\"source\":\"{}\"", escape(&self.loc.to_string()));
        let _ = write!(out, ",\"path\":\"{}\"", escape(&self.path));
        let _ = write!(
            out,
            ",\"rect\":[{},{},{},{}]",
            self.rect.origin.x, self.rect.origin.y, self.rect.size.x, self.rect.size.y
        );
        let chain: Vec<String> = self.chain.iter().map(u64::to_string).collect();
        let _ = write!(out, ",\"chain\":[{}]", chain.join(","));
        let _ = write!(out, ",\"message\":\"{}\"", escape(&self.message));
        out.push('}');
        out
    }
}

fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// 运行全部检查器。诊断结果按 `(rule, node)` 排序以确保输出确定性。
///
#[must_use]
pub fn check(frame: &Frame) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    check_finite(frame, &mut diagnostics);
    check_text(frame, &mut diagnostics);
    check_targets(frame, &mut diagnostics);
    check_siblings(frame, &mut diagnostics);
    check_portals(frame, &mut diagnostics);
    check_text_overflow(frame, &mut diagnostics);
    check_text_contrast(frame, &mut diagnostics);
    diagnostics.sort_by_key(|d| (d.rule, d.node));
    diagnostics
}

fn push(frame: &Frame, diagnostics: &mut Vec<Diagnostic>, rule: Rule, node: u64, message: String) {
    let Some(snapshot) = frame.get(node) else {
        return;
    };
    diagnostics.push(Diagnostic {
        rule,
        node,
        loc: snapshot.loc,
        path: snapshot.path.clone(),
        rect: snapshot.rect,
        chain: frame.chain(node),
        message,
    });
}

/// 1. 禁止 `NaN`、`inf` 与负尺寸。
fn check_finite(frame: &Frame, diagnostics: &mut Vec<Diagnostic>) {
    for node in &frame.nodes {
        if !node.rect.is_finite() || !node.rect.origin.is_finite() {
            push(
                frame,
                diagnostics,
                Rule::NoNaNOrInf,
                node.id,
                format!("non-finite geometry {:?}", node.rect),
            );
        } else if node.rect.size.x < 0.0 || node.rect.size.y < 0.0 {
            push(
                frame,
                diagnostics,
                Rule::NoNaNOrInf,
                node.id,
                format!("negative extent {:?}", node.rect.size),
            );
        } else if let Some(clip) = node.clip {
            if !clip.is_finite() {
                push(
                    frame,
                    diagnostics,
                    Rule::NoNaNOrInf,
                    node.id,
                    format!("non-finite clip {clip:?}"),
                );
            }
        }
    }
}

/// 2. 除非启用省略号或位于滚动容器内，否则文本墨迹不得被有效裁剪区截断。
///
fn check_text(frame: &Frame, diagnostics: &mut Vec<Diagnostic>) {
    for node in &frame.nodes {
        let NodeKind::Text { ink, ellipsis } = &node.kind else {
            continue;
        };
        if *ellipsis || node.scrollable {
            continue;
        }
        let Some(clip) = node.clip else { continue };
        if !clip.contains_rect(ink) {
            push(
                frame,
                diagnostics,
                Rule::TextNotClipped,
                node.id,
                format!("ink {ink:?} escapes clip {clip:?}"),
            );
        }
    }
}

/// 3. 交互目标必须保留至少 `24 × 24` 可见像素，且中心点不得被兄弟节点遮挡。
///
fn check_targets(frame: &Frame, diagnostics: &mut Vec<Diagnostic>) {
    for node in &frame.nodes {
        let NodeKind::Interactive { min_size, occluded } = &node.kind else {
            continue;
        };
        let visible = node.visible_rect();
        if visible.size.x + 0.01 < min_size.x || visible.size.y + 0.01 < min_size.y {
            push(
                frame,
                diagnostics,
                Rule::InteractiveTargetSize,
                node.id,
                format!(
                    "visible target {}×{} is under the {}×{} minimum",
                    visible.size.x, visible.size.y, min_size.x, min_size.y
                ),
            );
        }
        if *occluded {
            push(
                frame,
                diagnostics,
                Rule::InteractiveTargetSize,
                node.id,
                "centre is fully covered by a sibling".to_owned(),
            );
        }
    }
}

/// 4. 常规流中的兄弟节点不得重叠；覆盖层允许重叠。
///
/// 成对扫描采用 X 轴扫描线：子节点按左边缘排序，一旦后续兄弟节点的左边缘
/// 超过当前节点的右边缘，内层循环即提前终止。因此 5 000 格的网格仅需
/// 一次排序外加少量比较，而非 1 200 万次两两检测。
///
fn check_siblings(frame: &Frame, diagnostics: &mut Vec<Diagnostic>) {
    let mut flow: Vec<&crate::frame::NodeSnapshot> = Vec::new();
    for parent in &frame.nodes {
        if parent.children.len() < 2 {
            continue;
        }
        flow.clear();
        flow.extend(
            parent
                .children
                .iter()
                .filter_map(|id| frame.get(*id))
                .filter(|node| !node.overlays_siblings),
        );
        flow.sort_by(|a, b| a.rect.min().x.total_cmp(&b.rect.min().x));
        for (i, a) in flow.iter().enumerate() {
            for b in &flow[i + 1..] {
                if b.rect.min().x >= a.rect.max().x {
                    break;
                }
                let overlap = a.rect.intersect(&b.rect);
                // 亚像素级接触属于舍入误差，并非布局缺陷。
                if overlap.size.x > 0.5 && overlap.size.y > 0.5 {
                    push(
                        frame,
                        diagnostics,
                        Rule::NoSiblingOverlap,
                        b.id,
                        format!(
                            "overlaps sibling {} by {}×{}",
                            a.id, overlap.size.x, overlap.size.y
                        ),
                    );
                }
            }
        }
    }
}

/// 5. Portal 必须完全保持在物理视口内。
fn check_portals(frame: &Frame, diagnostics: &mut Vec<Diagnostic>) {
    for node in &frame.nodes {
        if !node.kind.is_portal() {
            continue;
        }
        if !frame.viewport.contains_rect(&node.rect) {
            push(
                frame,
                diagnostics,
                Rule::ViewportContainment,
                node.id,
                format!(
                    "portal {:?} escapes viewport {:?}",
                    node.rect, frame.viewport
                ),
            );
        }
    }
}

/// 6. 文本墨迹在水平方向上必须保持在父节点内容盒之内。
///
/// 标签宽于胶囊容器的 pill，或文字跑到自身控件下方的旋钮标签，即便未被
/// 滚动视口裁剪，也属于真实的视觉缺陷。垂直溢出交由滚动容器契约处理
/// （`AssertTextNotClipped`）；本规则仅检查排版后的文本行是否宽于其
/// 所属盒子。启用省略号的文本豁免。
///
fn check_text_overflow(frame: &Frame, diagnostics: &mut Vec<Diagnostic>) {
    for node in &frame.nodes {
        let NodeKind::Text { ink, ellipsis } = &node.kind else {
            continue;
        };
        if *ellipsis {
            continue;
        }
        let Some(parent) = node.parent.and_then(|id| frame.get(id)) else {
            continue;
        };
        let bounds = parent.rect;
        if ink.min().x < bounds.min().x - 0.5 || ink.max().x > bounds.max().x + 0.5 {
            push(
                frame,
                diagnostics,
                Rule::TextOverflow,
                node.id,
                format!(
                    "ink {ink:?} is wider than parent {} content box {bounds:?}",
                    parent.id
                ),
            );
        }
    }
}

/// 7. 声明的文本颜色必须具备可读对比度。
///
/// 帧仅记录通过 `set_text_colors` 显式声明颜色的组件，因此规则无法臆测
/// 图元的填充色。这正是捕获亮色主题下误用暗色 HUD 配色问题的机制。
///
fn check_text_contrast(frame: &Frame, diagnostics: &mut Vec<Diagnostic>) {
    for node in &frame.nodes {
        let Some(colors) = node.text_colors else {
            continue;
        };
        let foreground = relative_luminance(colors.foreground);
        let background = relative_luminance(colors.background);
        let ratio = (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05);
        if ratio < 4.5 {
            push(
                frame,
                diagnostics,
                Rule::TextContrast,
                node.id,
                format!(
                    "contrast {ratio:.2}:1 is below 4.5:1 ({:?} on {:?})",
                    colors.foreground, colors.background
                ),
            );
        }
    }
}

/// 不透明 sRGB 颜色的 WCAG 2.1 相对亮度。
fn relative_luminance(color: [u8; 3]) -> f32 {
    let channel = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color[0]) + 0.7152 * channel(color[1]) + 0.0722 * channel(color[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameBuilder;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    fn viewport() -> Rect {
        rect(0.0, 0.0, 800.0, 600.0)
    }

    fn root() -> FrameBuilder {
        let mut b = FrameBuilder::new(viewport());
        b.open(NodeKind::Container, viewport(), "Root");
        b
    }

    #[test]
    fn a_clean_frame_produces_no_diagnostics() {
        let mut b = root();
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect(10.0, 10.0, 80.0, 30.0),
            "Button",
        );
        b.close();
        b.open(
            NodeKind::Text {
                ink: rect(120.0, 10.0, 100.0, 20.0),
                ellipsis: false,
            },
            rect(120.0, 10.0, 100.0, 20.0),
            "Label",
        );
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn nan_is_caught_with_its_source_line() {
        let mut b = root();
        b.open(
            NodeKind::Container,
            rect(f32::NAN, 0.0, 10.0, 10.0),
            "Broken",
        );
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::NoNaNOrInf);
        assert!(diagnostics[0].loc.file.ends_with("rules.rs"));
        assert!(diagnostics[0].loc.line > 0);
    }

    #[test]
    fn negative_extent_is_caught() {
        let mut b = root();
        b.open(NodeKind::Container, rect(0.0, 0.0, -5.0, 10.0), "Broken");
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert_eq!(diagnostics[0].rule, Rule::NoNaNOrInf);
        assert!(diagnostics[0].message.contains("negative"));
    }

    #[test]
    fn clipped_text_is_caught_unless_ellipsised() {
        let mut b = root();
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Box");
        b.set_clip(rect(0.0, 0.0, 100.0, 20.0));
        b.open(
            NodeKind::Text {
                ink: rect(0.0, 0.0, 100.0, 60.0),
                ellipsis: false,
            },
            rect(0.0, 0.0, 100.0, 60.0),
            "Long",
        );
        b.close();
        b.close();
        b.close();
        let frame = b.build();
        let diagnostics = check(&frame);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::TextNotClipped);
    }

    #[test]
    fn ellipsis_makes_the_clip_intentional() {
        let mut b = root();
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Box");
        b.set_clip(rect(0.0, 0.0, 100.0, 20.0));
        b.open(
            NodeKind::Text {
                ink: rect(0.0, 0.0, 100.0, 60.0),
                ellipsis: true,
            },
            rect(0.0, 0.0, 100.0, 60.0),
            "Long",
        );
        b.close();
        b.close();
        b.close();
        assert!(check(&b.build()).is_empty());
    }

    #[test]
    fn a_scroll_container_may_clip_its_text() {
        let mut b = root();
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Scroll");
        b.set_scrollable(true);
        b.set_clip(rect(0.0, 0.0, 100.0, 20.0));
        b.open(
            NodeKind::Text {
                ink: rect(0.0, 0.0, 100.0, 500.0),
                ellipsis: false,
            },
            rect(0.0, 0.0, 100.0, 500.0),
            "Row",
        );
        b.close();
        b.close();
        b.close();
        assert!(check(&b.build()).is_empty());
    }

    #[test]
    fn undersized_targets_are_caught() {
        let mut b = root();
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect(0.0, 0.0, 20.0, 20.0),
            "Tiny",
        );
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::InteractiveTargetSize);
        assert!(diagnostics[0].message.contains("under"));
    }

    #[test]
    fn a_clipped_target_is_measured_after_the_clip() {
        let mut b = root();
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Box");
        b.set_clip(rect(0.0, 0.0, 40.0, 40.0));
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect(0.0, 0.0, 100.0, 100.0),
            "Giant",
        );
        b.close();
        b.close();
        b.close();
        let frame = b.build();
        // 100×100 被裁剪至 40×40 后仍大于 24×24。
        assert!(check(&frame).is_empty());
    }

    #[test]
    fn an_occluded_target_is_caught() {
        let mut b = root();
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: true,
            },
            rect(0.0, 0.0, 100.0, 30.0),
            "Covered",
        );
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert_eq!(diagnostics[0].rule, Rule::InteractiveTargetSize);
        assert!(diagnostics[0].message.contains("covered"));
    }

    #[test]
    fn overlapping_siblings_are_caught() {
        let mut b = root();
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Row");
        b.open(NodeKind::Container, rect(0.0, 0.0, 60.0, 30.0), "A");
        b.close();
        b.open(NodeKind::Container, rect(40.0, 0.0, 60.0, 30.0), "B");
        b.close();
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::NoSiblingOverlap);
        assert!(diagnostics[0].message.contains("overlaps sibling"));
    }

    #[test]
    fn overlays_may_overlap() {
        let mut b = root();
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Stack");
        b.open(NodeKind::Container, rect(0.0, 0.0, 60.0, 30.0), "A");
        b.close();
        b.open(NodeKind::Container, rect(40.0, 0.0, 60.0, 30.0), "B");
        b.set_overlays_siblings(true);
        b.close();
        b.close();
        b.close();
        assert!(check(&b.build()).is_empty());
    }

    #[test]
    fn a_portal_outside_the_viewport_is_caught() {
        let mut b = root();
        b.open(NodeKind::Portal, rect(700.0, 500.0, 300.0, 200.0), "Menu");
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ViewportContainment);
    }

    #[test]
    fn an_inside_portal_is_fine() {
        let mut b = root();
        b.open(NodeKind::Portal, rect(10.0, 10.0, 200.0, 100.0), "Menu");
        b.close();
        b.close();
        assert!(check(&b.build()).is_empty());
    }

    #[test]
    fn text_that_escapes_its_parent_is_caught() {
        let mut b = root();
        b.open(NodeKind::Container, rect(10.0, 10.0, 60.0, 20.0), "Pill");
        b.open(
            NodeKind::Text {
                ink: rect(10.0, 10.0, 120.0, 20.0),
                ellipsis: false,
            },
            rect(10.0, 10.0, 120.0, 20.0),
            "Label",
        );
        b.close();
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::TextOverflow);
        assert!(diagnostics[0].message.contains("content box"));
    }

    #[test]
    fn ellipsised_text_may_paint_past_its_parent() {
        let mut b = root();
        b.open(NodeKind::Container, rect(10.0, 10.0, 60.0, 20.0), "Pill");
        b.open(
            NodeKind::Text {
                ink: rect(10.0, 10.0, 120.0, 20.0),
                ellipsis: true,
            },
            rect(10.0, 10.0, 120.0, 20.0),
            "Label",
        );
        b.close();
        b.close();
        b.close();
        assert!(check(&b.build()).is_empty());
    }

    #[test]
    fn low_contrast_text_is_caught() {
        let mut b = root();
        b.open(
            NodeKind::Text {
                ink: rect(10.0, 10.0, 100.0, 20.0),
                ellipsis: false,
            },
            rect(10.0, 10.0, 100.0, 20.0),
            "Hud",
        );
        b.set_text_colors([220, 227, 242], [233, 237, 247]);
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::TextContrast);
        assert!(diagnostics[0].message.contains("below 4.5:1"));
    }

    #[test]
    fn a_dark_hud_has_readable_contrast() {
        let mut b = root();
        b.open(
            NodeKind::Text {
                ink: rect(10.0, 10.0, 100.0, 20.0),
                ellipsis: false,
            },
            rect(10.0, 10.0, 100.0, 20.0),
            "Hud",
        );
        b.set_text_colors([28, 30, 38], [255, 255, 255]);
        b.close();
        b.close();
        assert!(check(&b.build()).is_empty());
    }

    #[test]
    fn diagnostics_serialise_to_structured_json() {
        let mut b = root();
        b.open(NodeKind::Portal, rect(900.0, 0.0, 10.0, 10.0), "Menu");
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        let json = diagnostics[0].to_json();
        assert!(json.starts_with('{') && json.ends_with('}'));
        assert!(json.contains("\"rule\":\"AssertViewportContainment\""));
        assert!(json.contains("\"node\":2"));
        assert!(json.contains("\"source\":\"crates"));
        assert!(json.contains("\"path\":\"Root/Menu\""));
        assert!(json.contains("\"chain\":[1,2]"));
        assert!(!json.contains('\n'));
    }

    #[test]
    fn json_escaping_handles_quotes() {
        assert_eq!(escape("a\"b"), "a\\\"b");
        assert_eq!(escape("a\\b"), "a\\\\b");
        assert_eq!(escape("a\nb"), "a\\nb");
    }

    #[test]
    fn every_rule_has_a_doc_name() {
        for rule in Rule::ALL {
            assert!(rule.name().starts_with("Assert"));
        }
    }
}
