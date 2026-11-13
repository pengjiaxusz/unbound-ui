//! Avatars and avatar groups.
//!
//! An avatar is a circle with an optional presence dot. It deliberately does
//! not sample a texture itself: the emitter returns the circle and the caller
//! composes an [`crate::emit_image`] inside [`AvatarDesc::image_rect`]. That
//! keeps the widget free of the asset resolver while still giving the image a
//! stable, centred box.
//!
//! Initials are derived from the name rather than stored, so a rename never
//! leaves a stale two-letter label behind. CJK names keep their first
//! character (a Chinese given name is one glyph, not two initials).

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{glyphs, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The presence state shown on a dot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presence {
    /// Active.
    #[default]
    Online,
    /// Idle.
    Away,
    /// In a call or focused; do not disturb.
    Busy,
    /// Signed out.
    Offline,
}

impl Presence {
    /// The dot colour.
    #[must_use]
    pub fn token(self) -> Token {
        match self {
            Self::Online => Token::SUCCESS,
            Self::Away => Token::WARNING,
            Self::Busy => Token::ERROR,
            Self::Offline => Token::OUTLINE_STRONG,
        }
    }

    /// The accessible label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "在线",
            Self::Away => "离开",
            Self::Busy => "忙碌",
            Self::Offline => "离线",
        }
    }
}

/// A circular avatar.
#[derive(Clone, Debug, PartialEq)]
pub struct AvatarDesc {
    /// The square the circle inscribes.
    pub rect: Rect,
    /// The display name; initials are derived from it.
    pub name: String,
    /// The presence dot; `None` hides it.
    pub presence: Option<Presence>,
    /// The fill behind the initials.
    pub fill: Token,
    /// The draw order key.
    pub z: u16,
}

impl AvatarDesc {
    /// An avatar with a name.
    #[must_use]
    pub fn new(rect: Rect, name: impl Into<String>) -> Self {
        Self {
            rect,
            name: name.into(),
            presence: None,
            fill: Token::PRIMARY_SOFT,
            z: 540,
        }
    }

    /// The same avatar with a presence dot.
    #[must_use]
    pub fn presence(mut self, presence: Presence) -> Self {
        self.presence = Some(presence);
        self
    }

    /// The circle radius.
    #[must_use]
    pub fn radius(&self) -> f32 {
        (self.rect.size.x.min(self.rect.size.y) * 0.5).max(0.0)
    }

    /// The square an avatar image should fill.
    #[must_use]
    pub fn image_rect(&self) -> Rect {
        let size = self.radius() * 2.0;
        Rect::new(
            self.rect.origin.x + (self.rect.size.x - size) * 0.5,
            self.rect.origin.y + (self.rect.size.y - size) * 0.5,
            size,
            size,
        )
    }

    /// The presence dot, on the bottom-trailing edge.
    #[must_use]
    pub fn presence_rect(&self) -> Option<Rect> {
        self.presence?;
        let size = (self.radius() * 0.6).max(6.0);
        Some(Rect::new(
            self.image_rect().max().x - size,
            self.image_rect().max().y - size,
            size,
            size,
        ))
    }

    /// The initials painted inside the circle.
    #[must_use]
    pub fn initials(&self) -> String {
        initials(&self.name)
    }
}

/// Derive one or two initials from a display name.
#[must_use]
pub fn initials(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "?".to_owned();
    }
    let mut words = trimmed.split_whitespace();
    let first = words.next().unwrap_or("");
    let second = words.next();
    let mut out = String::with_capacity(4);
    if let Some(second) = second {
        if let Some(ch) = first.chars().next() {
            out.extend(ch.to_uppercase());
        }
        if let Some(ch) = second.chars().next() {
            out.extend(ch.to_uppercase());
        }
    } else {
        out.extend(first.chars().take(2).flat_map(char::to_uppercase));
    }
    out
}

/// Emit the circle, the initials and the presence dot.
#[must_use]
pub fn emit_avatar(
    node: u64,
    desc: &AvatarDesc,
    avatar_glyphs: &[GlyphQuad],
    control: ControlState,
) -> WidgetOutput {
    let radius = desc.radius();
    let mut primitives = Vec::with_capacity(3 + avatar_glyphs.len());
    let mut payloads = Vec::with_capacity(avatar_glyphs.len());
    primitives.push(surface(desc.image_rect(), radius, desc.fill, desc.z));
    glyphs(
        avatar_glyphs,
        Token::PRIMARY,
        desc.z + 1,
        &mut primitives,
        &mut payloads,
    );
    if let Some(dot) = desc.presence_rect() {
        primitives.push(surface(
            dot.inflate(2.0),
            (dot.size.x + 4.0) * 0.5,
            Token::SURFACE,
            desc.z + 2,
        ));
        primitives.push(surface(
            dot,
            dot.size.x * 0.5,
            desc.presence.map_or(Token::SUCCESS, Presence::token),
            desc.z + 3,
        ));
    }

    let mut a11y = A11yNode::new(Role::Image, desc.name.clone(), desc.rect);
    if let Some(presence) = desc.presence {
        a11y.description = presence.label().to_owned();
    }
    a11y.disabled = control.disabled;

    WidgetOutput {
        primitives,
        payloads,
        hit: Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_shape(Shape::Rounded { radii: [radius; 4] })
                .with_cursor(CursorKind::Default),
        ),
        a11y: Some(a11y),
        focusable: false,
    }
}

/// A stack of overlapping avatars plus an overflow counter.
#[derive(Clone, Debug, PartialEq)]
pub struct AvatarGroupDesc {
    /// The group rectangle; avatars are centred vertically.
    pub rect: Rect,
    /// The avatars, most important first.
    pub avatars: Vec<AvatarDesc>,
    /// How far each avatar moves over the previous one.
    pub overlap: f32,
    /// How many avatars fit before the `+N` counter.
    pub max_visible: usize,
    /// The draw order key.
    pub z: u16,
}

impl AvatarGroupDesc {
    /// A group with a 24 px overlap and five visible avatars.
    #[must_use]
    pub fn new(rect: Rect, avatars: Vec<AvatarDesc>) -> Self {
        Self {
            rect,
            avatars,
            overlap: 24.0,
            max_visible: 5,
            z: 540,
        }
    }

    /// The size of one avatar.
    #[must_use]
    pub fn avatar_size(&self) -> f32 {
        self.rect.size.y.max(1.0)
    }

    /// How many avatars are painted.
    #[must_use]
    pub fn visible_count(&self) -> usize {
        self.avatars.len().min(self.max_visible)
    }

    /// The overflow count (`+N`), zero when everything fits.
    #[must_use]
    pub fn overflow(&self) -> usize {
        self.avatars.len().saturating_sub(self.visible_count())
    }

    /// The circle of the `index`-th visible avatar.
    #[must_use]
    pub fn avatar_rect(&self, index: usize) -> Option<Rect> {
        if index >= self.visible_count() {
            return None;
        }
        let size = self.avatar_size();
        let step = self.overlap.max(1.0).min(size);
        Some(Rect::new(
            self.rect.origin.x + index as f32 * step,
            self.rect.origin.y,
            size,
            size,
        ))
    }

    /// The `+N` counter, when some avatars overflow.
    #[must_use]
    pub fn overflow_rect(&self) -> Option<Rect> {
        if self.overflow() == 0 {
            return None;
        }
        let size = self.avatar_size();
        let step = self.overlap.max(1.0).min(size);
        Some(Rect::new(
            self.rect.origin.x + self.visible_count() as f32 * step,
            self.rect.origin.y,
            size,
            size,
        ))
    }

    /// The width the painted group needs.
    #[must_use]
    pub fn painted_width(&self) -> f32 {
        let size = self.avatar_size();
        let count = self.visible_count() + usize::from(self.overflow() > 0);
        if count == 0 {
            return 0.0;
        }
        let step = self.overlap.max(1.0).min(size);
        (count - 1) as f32 * step + size
    }
}

/// Emit every visible avatar and the overflow counter.
#[must_use]
pub fn emit_avatar_group(
    group: &AvatarGroupDesc,
    member_glyphs: &[Vec<GlyphQuad>],
    overflow_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::new();
    let mut payloads = Vec::new();
    for (index, avatar) in group.avatars.iter().take(group.visible_count()).enumerate() {
        let Some(rect) = group.avatar_rect(index) else {
            continue;
        };
        let ring = rect.inflate(2.0);
        primitives.push(surface(ring, ring.size.x * 0.5, Token::SURFACE, group.z));
        primitives.push(surface(rect, rect.size.x * 0.5, avatar.fill, group.z + 1));
        if let Some(dot) = avatar.presence_rect() {
            primitives.push(surface(
                dot,
                dot.size.x * 0.5,
                avatar.presence.map_or(Token::SUCCESS, Presence::token),
                group.z + 2,
            ));
        }
        if let Some(member) = member_glyphs.get(index) {
            glyphs(
                member,
                Token::PRIMARY,
                group.z + 3,
                &mut primitives,
                &mut payloads,
            );
        }
    }
    if let Some(rect) = group.overflow_rect() {
        primitives.push(surface(
            rect,
            rect.size.x * 0.5,
            Token::SURFACE_SUNKEN,
            group.z,
        ));
        glyphs(
            overflow_glyphs,
            Token::TEXT_SECONDARY,
            group.z + 1,
            &mut primitives,
            &mut payloads,
        );
    }

    let names: Vec<&str> = group
        .avatars
        .iter()
        .take(group.visible_count())
        .map(|avatar| avatar.name.as_str())
        .collect();
    let mut a11y = A11yNode::new(Role::Group, names.join("、"), group.rect);
    a11y.description = if group.overflow() > 0 {
        format!("另有 {} 人", group.overflow())
    } else {
        String::new()
    };

    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(a11y),
        focusable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_use_two_words_or_two_characters() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("grace"), "GR");
        assert_eq!(initials("张伟"), "张伟");
        assert_eq!(initials("  "), "?");
    }

    #[test]
    fn the_circle_inscribes_the_square() {
        let avatar = AvatarDesc::new(Rect::new(0.0, 0.0, 40.0, 40.0), "Ada");
        assert_eq!(avatar.radius(), 20.0);
        assert_eq!(avatar.image_rect(), avatar.rect);
    }

    #[test]
    fn a_non_square_box_centres_the_circle() {
        let avatar = AvatarDesc::new(Rect::new(0.0, 0.0, 60.0, 40.0), "Ada");
        assert_eq!(avatar.radius(), 20.0);
        assert_eq!(avatar.image_rect().origin.x, 10.0);
    }

    #[test]
    fn the_presence_dot_sits_on_the_bottom_trailing_edge() {
        let avatar =
            AvatarDesc::new(Rect::new(0.0, 0.0, 40.0, 40.0), "Ada").presence(Presence::Busy);
        let dot = avatar.presence_rect().expect("dot");
        assert!(avatar.rect.contains_rect(&dot));
        assert!(dot.max().x >= avatar.rect.max().x - 12.0);
        assert_eq!(Presence::Busy.token(), Token::ERROR);
    }

    #[test]
    fn the_group_overlaps_and_reports_overflow() {
        let avatars: Vec<AvatarDesc> = (0..7)
            .map(|index| AvatarDesc::new(Rect::new(0.0, 0.0, 32.0, 32.0), format!("User {index}")))
            .collect();
        let group = AvatarGroupDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), avatars);
        assert_eq!(group.visible_count(), 5);
        assert_eq!(group.overflow(), 2);
        assert_eq!(group.avatar_rect(1).unwrap().origin.x, 24.0);
        assert_eq!(group.overflow_rect().unwrap().origin.x, 120.0);
        assert_eq!(group.painted_width(), 152.0);
    }

    #[test]
    fn the_avatar_emitter_labels_the_image() {
        let avatar =
            AvatarDesc::new(Rect::new(0.0, 0.0, 40.0, 40.0), "Ada").presence(Presence::Online);
        let output = emit_avatar(4, &avatar, &[], ControlState::default());
        let a11y = output.a11y.expect("image");
        assert_eq!(a11y.role, Role::Image);
        assert_eq!(a11y.label, "Ada");
        assert_eq!(a11y.description, "在线");
        assert!(output.hit.is_some());
    }

    #[test]
    fn the_group_emitter_paints_rings_and_the_counter() {
        let avatars = vec![
            AvatarDesc::new(Rect::new(0.0, 0.0, 32.0, 32.0), "Ada"),
            AvatarDesc::new(Rect::new(0.0, 0.0, 32.0, 32.0), "Grace"),
        ];
        let group = AvatarGroupDesc::new(Rect::new(0.0, 0.0, 100.0, 32.0), avatars);
        let output = emit_avatar_group(&group, &[], &[]);
        assert_eq!(output.a11y.expect("group").role, Role::Group);
        assert_eq!(output.primitives.len(), 4, "two rings and two circles");
    }
}
