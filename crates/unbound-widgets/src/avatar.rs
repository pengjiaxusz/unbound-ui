//! 头像与头像组（Avatar / AvatarGroup）。
//!
//! 头像是一个带可选在线状态圆点的圆形组件。它刻意不直接采样纹理：
//! 发射器仅返回圆形图元，调用方可在 [`AvatarDesc::image_rect`] 内组合 [`crate::emit_image`]。
//! 这样使组件与资源解析器解耦，同时仍为图像提供稳定居中的绘制框。
//!
//! 首字母缩写直接从名称派生而不单独存储，因此改名后不会残留过期的双字母标签。
//! 中日韩姓名保留首字（中文名字通常取单字，而非两个字母缩写）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{glyphs, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 状态圆点显示的在线状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presence {
    /// 在线活跃。
    #[default]
    Online,
    /// 离开/空闲。
    Away,
    /// 通话中或专注模式；请勿打扰。
    Busy,
    /// 已离线。
    Offline,
}

impl Presence {
    /// 状态圆点颜色。
    #[must_use]
    pub fn token(self) -> Token {
        match self {
            Self::Online => Token::SUCCESS,
            Self::Away => Token::WARNING,
            Self::Busy => Token::ERROR,
            Self::Offline => Token::OUTLINE_STRONG,
        }
    }

    /// 无障碍标签。
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

/// 圆形头像描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct AvatarDesc {
    /// 圆形所内切的正方形区域。
    pub rect: Rect,
    /// 显示名称；首字母缩写由此派生。
    pub name: String,
    /// 在线状态圆点；`None` 表示隐藏。
    pub presence: Option<Presence>,
    /// 首字母背景填充色。
    pub fill: Token,
    /// 绘制顺序键。
    pub z: u16,
}

impl AvatarDesc {
    /// 创建带名称的头像。
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

    /// 附加在线状态圆点。
    #[must_use]
    pub fn presence(mut self, presence: Presence) -> Self {
        self.presence = Some(presence);
        self
    }

    /// 圆形半径。
    #[must_use]
    pub fn radius(&self) -> f32 {
        (self.rect.size.x.min(self.rect.size.y) * 0.5).max(0.0)
    }

    /// 头像图片应填充的正方形区域。
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

    /// 位于右下边缘的在线状态圆点。
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

    /// 圆形内部绘制的首字母缩写。
    #[must_use]
    pub fn initials(&self) -> String {
        initials(&self.name)
    }
}

/// 从显示名称派生一到两个首字母。
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

/// 发射圆形背景、首字母缩写与在线状态圆点。
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
        ..Default::default()
    }
}

/// 堆叠重叠头像组及溢出计数器。
#[derive(Clone, Debug, PartialEq)]
pub struct AvatarGroupDesc {
    /// 头像组矩形；头像垂直居中排列。
    pub rect: Rect,
    /// 头像列表，优先级高者靠前。
    pub avatars: Vec<AvatarDesc>,
    /// 每个头像相对前一个头像的偏移距离。
    pub overlap: f32,
    /// 在显示 `+N` 计数器前最多显示的头像数量。
    pub max_visible: usize,
    /// 绘制顺序键。
    pub z: u16,
}

impl AvatarGroupDesc {
    /// 创建 24 px 重叠间距、最多显示 5 个头像的头像组。
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

    /// 单个头像的尺寸。
    #[must_use]
    pub fn avatar_size(&self) -> f32 {
        self.rect.size.y.max(1.0)
    }

    /// 实际绘制的头像数量。
    #[must_use]
    pub fn visible_count(&self) -> usize {
        self.avatars.len().min(self.max_visible)
    }

    /// 溢出数量（`+N`），全部容纳时为零。
    #[must_use]
    pub fn overflow(&self) -> usize {
        self.avatars.len().saturating_sub(self.visible_count())
    }

    /// 第 `index` 个可见头像的圆形区域。
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

    /// 当存在溢出头像时的 `+N` 计数器。
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

    /// 头像组绘制所需的总宽度。
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

/// 发射所有可见头像及溢出计数器。
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
        ..Default::default()
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
