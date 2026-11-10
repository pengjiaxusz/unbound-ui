//! The five layout sanitizers (`docs` §7.2).
//!
//! Each rule is a pure function of a [`Frame`], so a failure is reproducible
//! from a binary snapshot alone. Diagnostics carry the node id, the *source
//! line that built the node* (`#[track_caller]`), the parent chain, the actual
//! rectangle and the semantic path — the doc's "structured JSON error stack" —
//! so an agent can jump straight to the offending component.

#![forbid(unsafe_op_in_unsafe_fn)]

use core::fmt::Write as _;

use unbound_math::Rect;

use crate::frame::{Frame, NodeKind, SourceLoc};

/// Which invariant was violated.
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
}

impl Rule {
    pub const ALL: [Self; 5] = [
        Self::NoNaNOrInf,
        Self::TextNotClipped,
        Self::InteractiveTargetSize,
        Self::NoSiblingOverlap,
        Self::ViewportContainment,
    ];

    /// The doc's name for the assertion.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NoNaNOrInf => "AssertNoNaNOrInf",
            Self::TextNotClipped => "AssertTextNotClipped",
            Self::InteractiveTargetSize => "AssertInteractiveTargetSize",
            Self::NoSiblingOverlap => "AssertNoUnintendedSiblingOverlap",
            Self::ViewportContainment => "AssertViewportContainment",
        }
    }
}

/// One failed assertion.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub rule: Rule,
    pub node: u64,
    pub loc: SourceLoc,
    pub path: String,
    pub rect: Rect,
    /// Root-to-node chain, so the parent constraints are visible.
    pub chain: Vec<u64>,
    pub message: String,
}

impl Diagnostic {
    /// Structured, single-line JSON. Hand-rolled so the harness has no
    /// serialization dependency and the output is stable enough to diff.
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

/// Run every sanitizer. Diagnostics are sorted by `(rule, node)` so the output
/// is deterministic.
#[must_use]
pub fn check(frame: &Frame) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    check_finite(frame, &mut diagnostics);
    check_text(frame, &mut diagnostics);
    check_targets(frame, &mut diagnostics);
    check_siblings(frame, &mut diagnostics);
    check_portals(frame, &mut diagnostics);
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

/// 1. No `NaN`, no `inf`, no negative extents.
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

/// 2. Text ink must survive the effective clip unless it is ellipsised or inside
///    a scroll container.
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

/// 3. Interactive targets must keep at least `24 × 24` visible pixels and must
///    not have their centre swallowed by a sibling.
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

/// 4. Siblings in normal flow must not overlap; overlays may.
///
/// The pair scan is a sweep on X: children are sorted by their left edge and the
/// inner loop stops as soon as a sibling starts to the right of the current one's
/// right edge. A 5 000 cell grid therefore costs one pass plus a handful of
/// comparisons, not 12 million pair tests.
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
                // A sub-pixel touch is a rounding artefact, not a layout bug.
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

/// 5. Portals must stay inside the physical viewport.
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
        b.open(NodeKind::Text {
            ink: rect(120.0, 10.0, 100.0, 20.0),
            ellipsis: false,
        }, rect(120.0, 10.0, 100.0, 20.0), "Label");
        b.close();
        b.close();
        let diagnostics = check(&b.build());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn nan_is_caught_with_its_source_line() {
        let mut b = root();
        b.open(NodeKind::Container, rect(f32::NAN, 0.0, 10.0, 10.0), "Broken");
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
        // 100×100 clipped to 40×40 is still above 24×24.
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
