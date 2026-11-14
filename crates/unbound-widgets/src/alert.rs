//! 行内提示横幅（Alert）。
//!
//! Alert 是页面内的*常驻*消息：与 Toast 不同，它不会自动超时消失；
//! 与 Dialog 不同，它不会打断用户操作。其严重级别同时通过三个通道传达——
//! 左侧强调条、图标与无障碍标签——即使在单色主题或色盲场景下也能清晰传达语义。
//! 横幅本身不持有状态。关闭控件作为单一命中区域上报；点击后的处理由 reducer 决定。
//!

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

/// 左侧强调条宽度。
pub const ALERT_ACCENT_WIDTH: f32 = 4.0;
/// 横幅内边距。
pub const ALERT_PADDING: f32 = 14.0;
/// 严重级别图标直径。
pub const ALERT_ICON_SIZE: f32 = 18.0;
/// 关闭按钮正方形边长。
pub const ALERT_CLOSE_SIZE: f32 = 20.0;
/// 横幅圆角半径。
pub const ALERT_RADIUS: f32 = 8.0;

/// 提示横幅的语义类别。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Severity {
    /// 中性信息。
    #[default]
    Info,
    /// 操作已完成。
    Success,
    /// 需要关注但尚未出错。
    Warning,
    /// 发生错误或失败。
    Error,
}

impl Severity {
    /// 低饱和度横幅填充色。
    #[must_use]
    pub fn fill(self) -> Token {
        match self {
            Self::Info => Token::PRIMARY_SOFT,
            Self::Success => Token::SUCCESS_SOFT,
            Self::Warning => Token::WARNING_SOFT,
            Self::Error => Token::ERROR_SOFT,
        }
    }

    /// 强调条与图标使用的饱和强调色。
    #[must_use]
    pub fn accent(self) -> Token {
        match self {
            Self::Info => Token::PRIMARY,
            Self::Success => Token::SUCCESS,
            Self::Warning => Token::WARNING,
            Self::Error => Token::ERROR,
        }
    }

    /// 屏幕阅读器随标题一同播报的文本标签。
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

/// 行内提示横幅描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct AlertDesc {
    /// 横幅矩形区域。
    pub rect: Rect,
    /// 严重级别类别。
    pub severity: Severity,
    /// 主标题。
    pub title: String,
    /// 可选正文文案。
    pub message: String,
    /// 是否显示关闭控件。
    pub dismissible: bool,
    /// 圆角半径。
    pub radius: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl AlertDesc {
    /// 创建带标题的提示横幅。
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

    /// 附加正文文案。
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// 附加关闭控件。
    #[must_use]
    pub fn dismissible(mut self) -> Self {
        self.dismissible = true;
        self
    }

    /// 左侧强调条矩形。
    #[must_use]
    pub fn accent_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            ALERT_ACCENT_WIDTH.min(self.rect.size.x),
            self.rect.size.y,
        )
    }

    /// 位于强调条内侧的图标正方形区域。
    #[must_use]
    pub fn icon_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + ALERT_ACCENT_WIDTH + ALERT_PADDING,
            self.rect.origin.y + ALERT_PADDING,
            ALERT_ICON_SIZE,
            ALERT_ICON_SIZE,
        )
    }

    /// 可关闭时的关闭按钮矩形。
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

    /// 位于图标右侧、关闭按钮左侧的文本区域。
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

/// 发射提示横幅及其关闭命中区域。
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

/// 图标圆圈内部的小型严重级别标记。
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

/// 以退化二次贝塞尔曲线编码的直线描边。
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
            // 背景、强调条与图标圆圈占用索引 0..=2；
            // 始终存在的焦点环位于最后一个槽位，不属于标记图形。
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
