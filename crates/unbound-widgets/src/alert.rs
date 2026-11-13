//! Inline alert banners.
//!
//! An alert is a *persistent* message inside the page: unlike a toast it does
//! not time out, and unlike a dialog it does not interrupt. Its severity is
//! carried by three channels at once — a left accent bar, an icon and the
//! accessible label — so it survives a monochrome theme or a colour-blind
//! reader without losing meaning.
//!
//! The banner never owns state. The dismiss affordance is reported as the
//! single hit region; clicking it is the reducer's decision.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{instance, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The width of the left accent bar.
pub const ALERT_ACCENT_WIDTH: f32 = 4.0;
/// The banner's inner padding.
pub const ALERT_PADDING: f32 = 14.0;
/// The severity icon diameter.
pub const ALERT_ICON_SIZE: f32 = 18.0;
/// The dismiss button square.
pub const ALERT_CLOSE_SIZE: f32 = 20.0;
/// The banner corner radius.
pub const ALERT_RADIUS: f32 = 8.0;

/// The semantic family of an alert.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Severity {
    /// Neutral information.
    #[default]
    Info,
    /// A completed operation.
    Success,
    /// Something needs attention but is not broken.
    Warning,
    /// Something failed.
    Error,
}

impl Severity {
    /// The low-saturation banner fill.
    #[must_use]
    pub fn fill(self) -> Token {
        match self {
            Self::Info => Token::PRIMARY_SOFT,
            Self::Success => Token::SUCCESS_SOFT,
            Self::Warning => Token::WARNING_SOFT,
            Self::Error => Token::ERROR_SOFT,
        }
    }

    /// The saturated accent used by the bar and the icon.
    #[must_use]
    pub fn accent(self) -> Token {
        match self {
            Self::Info => Token::PRIMARY,
            Self::Success => Token::SUCCESS,
            Self::Warning => Token::WARNING,
            Self::Error => Token::ERROR,
        }
    }

    /// The text label a screen reader announces with the title.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "提示",
            Self::Success => "成功",
            Self::Warning => "警告",
            Self::Error => "错误",
        }
    }
}

/// An inline alert banner.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertDesc {
    /// The banner rectangle.
    pub rect: Rect,
    /// The severity family.
    pub severity: Severity,
    /// The headline.
    pub title: String,
    /// The optional body copy.
    pub message: String,
    /// Show a close affordance.
    pub dismissible: bool,
    /// The corner radius.
    pub radius: f32,
    /// The draw order key.
    pub z: u16,
}

impl AlertDesc {
    /// A banner with a title.
    #[must_use]
    pub fn new(rect: Rect, severity: Severity, title: impl Into<String>) -> Self {
        Self {
            rect,
            severity,
            title: title.into(),
            message: String::new(),
            dismissible: false,
            radius: ALERT_RADIUS,
            z: 540,
        }
    }

    /// The same banner with body copy.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// The same banner with a close affordance.
    #[must_use]
    pub fn dismissible(mut self) -> Self {
        self.dismissible = true;
        self
    }

    /// The left accent bar.
    #[must_use]
    pub fn accent_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            ALERT_ACCENT_WIDTH.min(self.rect.size.x),
            self.rect.size.y,
        )
    }

    /// The icon square, just inside the accent bar.
    #[must_use]
    pub fn icon_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + ALERT_ACCENT_WIDTH + ALERT_PADDING,
            self.rect.origin.y + ALERT_PADDING,
            ALERT_ICON_SIZE,
            ALERT_ICON_SIZE,
        )
    }

    /// The dismiss button, when the banner is dismissible.
    #[must_use]
    pub fn close_rect(&self) -> Option<Rect> {
        self.dismissible.then(|| {
            Rect::new(
                self.rect.max().x - ALERT_PADDING * 0.5 - ALERT_CLOSE_SIZE,
                self.rect.origin.y + ALERT_PADDING * 0.5,
                ALERT_CLOSE_SIZE,
                ALERT_CLOSE_SIZE,
            )
        })
    }

    /// The text block, right of the icon and left of the close button.
    #[must_use]
    pub fn content_rect(&self) -> Rect {
        let icon = self.icon_rect();
        let left = icon.max().x + 10.0;
        let right = match self.close_rect() {
            Some(close) => close.origin.x - 6.0,
            None => self.rect.max().x - ALERT_PADDING,
        };
        Rect::new(
            left,
            self.rect.origin.y + ALERT_PADDING,
            (right - left).max(0.0),
            (self.rect.size.y - ALERT_PADDING * 2.0).max(0.0),
        )
    }
}

/// Emit a banner and its dismiss hit region.
#[must_use]
pub fn emit_alert(
    node: u64,
    desc: &AlertDesc,
    alert_glyphs: &[GlyphQuad],
    control: ControlState,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(8 + alert_glyphs.len());
    let mut payloads = Vec::with_capacity(alert_glyphs.len());
    let accent = desc.severity.accent();

    primitives.push(surface(
        desc.rect,
        desc.radius,
        desc.severity.fill(),
        desc.z,
    ));
    primitives.push(surface(desc.accent_rect(), 2.0, accent, desc.z + 1));

    let icon = desc.icon_rect();
    primitives.push(surface(icon, icon.size.x * 0.5, accent, desc.z + 1));
    severity_mark(
        desc.severity,
        icon,
        Token::ON_PRIMARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    crate::primitive::glyphs(
        alert_glyphs,
        Token::TEXT_PRIMARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    let mut hit = None;
    if let Some(close) = desc.close_rect() {
        let inset = 6.0;
        stroke(
            (close.origin.x + inset, close.origin.y + inset),
            (close.max().x - inset, close.max().y - inset),
            1.5,
            Token::TEXT_SECONDARY,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
        stroke(
            (close.max().x - inset, close.origin.y + inset),
            (close.origin.x + inset, close.max().y - inset),
            1.5,
            Token::TEXT_SECONDARY,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
        hit = Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), close)
                .with_cursor(CursorKind::Pointer),
        );
    }

    primitives.push(crate::primitive::focus_ring(
        desc.rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        desc.z + 4,
        control.focused,
    ));

    let mut a11y = A11yNode::new(
        Role::Alert,
        format!("{}：{}", desc.severity.label(), desc.title),
        desc.rect,
    );
    a11y.description = desc.message.clone();

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: desc.dismissible && !control.disabled,
        ..Default::default()
    }
}

/// A small severity mark inside the icon circle.
fn severity_mark(
    severity: Severity,
    icon: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let x = icon.origin.x;
    let y = icon.origin.y;
    let w = icon.size.x;
    let h = icon.size.y;
    match severity {
        Severity::Success => {
            stroke(
                (x + w * 0.26, y + h * 0.53),
                (x + w * 0.44, y + h * 0.71),
                2.0,
                color,
                z,
                primitives,
                payloads,
            );
            stroke(
                (x + w * 0.44, y + h * 0.71),
                (x + w * 0.75, y + h * 0.31),
                2.0,
                color,
                z,
                primitives,
                payloads,
            );
        }
        Severity::Error => {
            stroke(
                (x + w * 0.32, y + h * 0.32),
                (x + w * 0.68, y + h * 0.68),
                2.0,
                color,
                z,
                primitives,
                payloads,
            );
            stroke(
                (x + w * 0.68, y + h * 0.32),
                (x + w * 0.32, y + h * 0.68),
                2.0,
                color,
                z,
                primitives,
                payloads,
            );
        }
        Severity::Info | Severity::Warning => {
            primitives.push(surface(
                Rect::new(x + w * 0.5 - 1.0, y + h * 0.26, 2.0, h * 0.28),
                1.0,
                color,
                z,
            ));
            primitives.push(surface(
                Rect::new(x + w * 0.5 - 1.0, y + h * 0.62, 2.0, 2.0),
                1.0,
                color,
                z,
            ));
        }
    }
}

/// A straight stroke encoded as a degenerate quadratic curve.
#[allow(clippy::too_many_arguments)]
fn stroke(
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let origin = Rect::new(
        from.0.min(to.0),
        from.1.min(to.1),
        (from.0 - to.0).abs().max(1.0),
        (from.1 - to.1).abs().max(1.0),
    );
    let payload_idx = payloads.len() as u32;
    payloads.push(GpuPayload::curve(
        [from.0 - origin.origin.x, from.1 - origin.origin.y],
        [
            (from.0 + to.0) * 0.5 - origin.origin.x,
            (from.1 + to.1) * 0.5 - origin.origin.y,
        ],
        [to.0 - origin.origin.x, to.1 - origin.origin.y],
        width,
    ));
    let mut primitive = instance(
        origin,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    primitive.extra_payload_idx = payload_idx;
    primitives.push(primitive);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> AlertDesc {
        AlertDesc::new(
            Rect::new(0.0, 0.0, 400.0, 64.0),
            Severity::Warning,
            "Low disk space",
        )
    }

    #[test]
    fn every_severity_has_a_distinct_fill_and_accent() {
        let severities = [
            Severity::Info,
            Severity::Success,
            Severity::Warning,
            Severity::Error,
        ];
        let fills: Vec<Token> = severities.iter().map(|severity| severity.fill()).collect();
        let accents: Vec<Token> = severities
            .iter()
            .map(|severity| severity.accent())
            .collect();
        for (index, fill) in fills.iter().enumerate() {
            assert!(!fills[..index].contains(fill));
        }
        for (index, accent) in accents.iter().enumerate() {
            assert!(!accents[..index].contains(accent));
        }
    }

    #[test]
    fn the_accent_bar_hugs_the_left_edge() {
        let alert = desc();
        let accent = alert.accent_rect();
        assert_eq!(accent.origin.x, alert.rect.origin.x);
        assert_eq!(accent.size.x, ALERT_ACCENT_WIDTH);
        assert_eq!(accent.size.y, alert.rect.size.y);
    }

    #[test]
    fn the_close_button_only_exists_when_dismissible() {
        let alert = desc();
        assert!(alert.close_rect().is_none());
        let alert = alert.dismissible();
        let close = alert.close_rect().expect("close");
        assert!(alert.rect.contains_rect(&close));
        assert_eq!(close.size.x, ALERT_CLOSE_SIZE);
    }

    #[test]
    fn the_content_stops_before_the_close_button() {
        let alert = desc().dismissible();
        let content = alert.content_rect();
        let close = alert.close_rect().expect("close");
        assert!(content.max().x <= close.origin.x);
        assert!(content.origin.x > alert.icon_rect().max().x);
    }

    #[test]
    fn the_banner_announces_its_severity_and_title() {
        let alert = desc().with_message("Delete the cache to continue.");
        let output = emit_alert(9, &alert, &[], ControlState::default());
        let a11y = output.a11y.expect("alert");
        assert_eq!(a11y.role, Role::Alert);
        assert!(a11y.label.contains("警告"));
        assert!(a11y.label.contains("Low disk space"));
        assert_eq!(a11y.description, "Delete the cache to continue.");
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }

    #[test]
    fn a_dismissible_banner_routes_the_click_to_the_close_button() {
        let alert = desc().dismissible();
        let output = emit_alert(9, &alert, &[], ControlState::default());
        let hit = output.hit.expect("close hit");
        assert_eq!(hit.node, 9);
        assert_eq!(hit.cursor, Some(CursorKind::Pointer));
        assert_eq!(hit.bounds, alert.close_rect().unwrap());
        assert!(output.focusable);
    }

    #[test]
    fn the_icon_mark_stays_inside_the_icon_circle() {
        for severity in [
            Severity::Info,
            Severity::Success,
            Severity::Warning,
            Severity::Error,
        ] {
            let alert = AlertDesc::new(Rect::new(0.0, 0.0, 400.0, 64.0), severity, "x");
            let output = emit_alert(1, &alert, &[], ControlState::default());
            let icon = alert.icon_rect();
            // Background, accent bar and icon circle are indices 0..=2; the
            // always-present focus ring is the last slot and is not a mark.
            let marks = &output.primitives[3..output.primitives.len() - 1];
            for primitive in marks {
                let bounds = Rect::new(
                    primitive.local_pos[0],
                    primitive.local_pos[1],
                    primitive.size[0],
                    primitive.size[1],
                );
                assert!(
                    icon.inflate(0.5).contains_rect(&bounds),
                    "mark {bounds:?} escaped icon {icon:?}"
                );
            }
        }
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc().dismissible();
        let mut counts = Vec::new();
        for phase in [
            crate::PointerPhase::Idle,
            crate::PointerPhase::Hovered,
            crate::PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let control = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        let output = emit_alert(9, &desc, &[], control);
                        counts.push(output.primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}
