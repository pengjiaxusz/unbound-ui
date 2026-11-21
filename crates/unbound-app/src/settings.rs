//! lang-allow: 中文注释与文档
//! Win11 规范设置卡片（Settings Card）与设置行排版构建器。
//!
//! 提供符合 Windows 11 设计语言的自包含设置卡片，内置：
//! 1. 恒定 57px 文本左内边距（无图标）或 16px 图标槽 + 22px 间距（带图标）；
//! 2. 连续添加多行时自动插入 1px 细线分隔线（`DividerDesc::rule()`），
//!    彻底消除调用方手工计算键值或追加分隔线的样板代码；
//! 3. 下拉选择框弹出浮层（ComboBox Popup Overlay）与行尾溢出菜单（Overflow Menu）的确定性键派生及组合基元。

use accesskit::Role;
use unbound_a11y::SemanticIntent;
use unbound_abi::{Dim, Edges, Justify, NodeFlags, RawNodeDesc};
use unbound_math::{Rect, Vec2};
use unbound_widgets::divider::DividerDesc;
use unbound_widgets::menu::{MenuItem, MENU_ITEM_HEIGHT, MENU_RADIUS, MENU_SHADOW};
use unbound_widgets::overlay::{place_anchored_popup, PlacedPopup, Placement};
use unbound_widgets::{
    ButtonDesc, ButtonSize, ButtonTokens, ButtonVariant, ShellIcon, ShellIconDesc,
};

use crate::{mix_key, KeyDiscriminator, NodeId, ViewBuilder};

/// 行尾 `···` 溢出菜单触发按钮的标准尺寸（28×28px，与 `ButtonSize::Sm` 对齐）。
const OVERFLOW_TRIGGER_SIZE: f32 = 28.0;
/// 溢出菜单弹出浮层的最小宽度（逻辑像素）。
const OVERFLOW_POPUP_MIN_WIDTH: f32 = 160.0;
/// 溢出菜单弹出浮层的最大宽度（逻辑像素）。
const OVERFLOW_POPUP_MAX_WIDTH: f32 = 320.0;
/// 溢出菜单弹出浮层的上下内边距（逻辑像素）。
const OVERFLOW_POPUP_PAD_Y: f32 = 4.0;
/// 溢出菜单弹出浮层与视口边界的安全边距（逻辑像素）。
const OVERFLOW_POPUP_VIEWPORT_MARGIN: f32 = 8.0;
/// 溢出菜单分隔符行高（4px 上间距 + 1px 细线 + 4px 下间距）。
const OVERFLOW_SEPARATOR_HEIGHT: f32 = 9.0;
/// 溢出菜单单行高亮胶囊的水平内边距（对齐 WinUI `MenuFlyoutItemMargin`）。
const OVERFLOW_ITEM_INSET_X: f32 = 4.0;
/// 溢出菜单单行高亮胶囊的垂直内边距（对齐 WinUI `MenuFlyoutItemMargin`）。
const OVERFLOW_ITEM_INSET_Y: f32 = 2.0;
/// 溢出菜单弹出浮层的 Portal Z 基准（高于 `PORTAL_Z_BASE` 与下拉框浮层）。
const OVERFLOW_POPUP_Z: i32 = 1260;

/// 派生指定下拉选择框的选项键。
#[must_use]
pub fn select_option_key(select_key: u64, index: usize) -> u64 {
    mix_key(select_key, 0x5e1000 | (index as u64))
}

/// 派生指定下拉选择框的弹出浮层容器键。
#[must_use]
pub fn select_popup_key(select_key: u64) -> u64 {
    mix_key(select_key, 0x005e_1000_f100)
}

/// 派生指定行尾溢出菜单触发器的第 `index` 个菜单项逻辑键。
#[must_use]
pub fn overflow_menu_item_key(trigger_key: u64, index: usize) -> u64 {
    mix_key(trigger_key, 0x0f10_0000_0000u64 ^ (index as u64))
}

/// 派生指定行尾溢出菜单触发器的弹出浮层容器键。
#[must_use]
pub fn overflow_menu_popup_key(trigger_key: u64) -> u64 {
    mix_key(trigger_key, 0x0f10_ffff_f100u64)
}

/// 包含自动分割线与行排版管理的 Win11 规范设置卡片构建器。
pub struct SettingsCard<'a, 'b> {
    pub view: &'b mut ViewBuilder<'a>,
    pub card_key: u64,
    pub row_count: usize,
}

impl<'a, 'b> SettingsCard<'a, 'b> {
    /// 创建新的设置卡片包装器。
    #[must_use]
    pub fn new(view: &'b mut ViewBuilder<'a>, card_key: u64) -> Self {
        Self {
            view,
            card_key,
            row_count: 0,
        }
    }

    /// 若此前已添加过行，则自动插入一条 1px 分隔线并递增计数。
    pub fn auto_divider(&mut self, next_key: u64) {
        if self.row_count > 0 {
            let div_key = mix_key(next_key, 0xd171de);
            self.view.divider(div_key, DividerDesc::rule());
        }
        self.row_count += 1;
    }

    /// 手动追加一条 1px 分隔线。
    pub fn divider(&mut self) -> NodeId {
        let div_key = mix_key(self.card_key, 0xd17100 | (self.row_count as u64));
        self.row_count += 1;
        self.view.divider(div_key, DividerDesc::rule())
    }

    /// 声明设置卡片内的一行（无图标：标题在左上、说明在左下，右侧放置行内控件）。
    ///
    /// 文字左缩进恒为 57px（`SETTINGS_ROW_TEXT_INSET`）——与 Win11 原生规范一致。
    /// 在同一卡片内连续添加多行时，自动插入 1px 细线分隔线。
    pub fn settings_row(
        &mut self,
        key: impl KeyDiscriminator,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        control: impl FnOnce(&mut ViewBuilder<'_>, u64),
    ) -> NodeId {
        let raw_key = key.into_u64();
        self.auto_divider(raw_key);
        let title = title.into();
        let subtitle = subtitle.into();

        let mut row_desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::ltrb(
                unbound_theme::typography::geometry::SETTINGS_ROW_TEXT_INSET,
                12.0,
                16.0,
                12.0,
            ))
            .gap(2.0);
        row_desc.flex_shrink = 0.0;

        self.view.container(raw_key, row_desc, |row| {
            row.row_between(mix_key(raw_key, 0x1), |r| {
                r.col_grow(mix_key(raw_key, 0x2), 2.0, |left| {
                    left.text_primary(
                        mix_key(raw_key, 0x3),
                        title,
                        unbound_theme::typography::font_size::BODY,
                        unbound_theme::typography::weight::SEMIBOLD,
                    );
                    if !subtitle.is_empty() {
                        left.text_muted(
                            mix_key(raw_key, 0x4),
                            subtitle,
                            unbound_theme::typography::font_size::CAPTION,
                        );
                    }
                });
                r.row_center(mix_key(raw_key, 0x5), 8.0, |right| control(right, raw_key));
            });
        })
    }

    /// 声明带前置图标槽的设置行（16px 图标槽 + 22px 间距，文字基线与无图标行精准对齐）。
    pub fn settings_row_with_icon(
        &mut self,
        key: impl KeyDiscriminator,
        icon: ShellIcon,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        control: impl FnOnce(&mut ViewBuilder<'_>, u64),
    ) -> NodeId {
        let raw_key = key.into_u64();
        self.auto_divider(raw_key);
        let title = title.into();
        let subtitle = subtitle.into();

        let mut row_desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::ltrb(
                unbound_theme::typography::geometry::SETTINGS_ROW_ICON_INSET,
                12.0,
                16.0,
                12.0,
            ))
            .gap(2.0);
        row_desc.flex_shrink = 0.0;

        self.view.container(raw_key, row_desc, |row| {
            row.row_start(
                mix_key(raw_key, 0x1),
                unbound_theme::typography::geometry::SETTINGS_ROW_ICON_GAP,
                |r| {
                    r.shell_icon(
                        mix_key(raw_key, 0x2),
                        ShellIconDesc::new(icon)
                            .size(unbound_theme::typography::geometry::SETTINGS_ROW_ICON_SIZE),
                    );
                    r.col_grow(mix_key(raw_key, 0x3), 2.0, |left| {
                        left.text_primary(
                            mix_key(raw_key, 0x4),
                            title,
                            unbound_theme::typography::font_size::BODY,
                            unbound_theme::typography::weight::SEMIBOLD,
                        );
                        if !subtitle.is_empty() {
                            left.text_muted(
                                mix_key(raw_key, 0x5),
                                subtitle,
                                unbound_theme::typography::font_size::CAPTION,
                            );
                        }
                    });
                    r.row_center(mix_key(raw_key, 0x6), 8.0, |right| control(right, raw_key));
                },
            );
        })
    }

    /// [`Self::settings_row`] 的简短别名。
    pub fn row(
        &mut self,
        key: impl KeyDiscriminator,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        control: impl FnOnce(&mut ViewBuilder<'_>, u64),
    ) -> NodeId {
        self.settings_row(key, title, subtitle, control)
    }

    /// [`Self::settings_row_with_icon`] 的简短别名。
    pub fn row_with_icon(
        &mut self,
        key: impl KeyDiscriminator,
        icon: ShellIcon,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        control: impl FnOnce(&mut ViewBuilder<'_>, u64),
    ) -> NodeId {
        self.settings_row_with_icon(key, icon, title, subtitle, control)
    }

    /// 声明带有自动分割线管理的自定义卡片行容器。
    pub fn custom_row(
        &mut self,
        key: impl KeyDiscriminator,
        build: impl FnOnce(&mut ViewBuilder<'_>),
    ) -> NodeId {
        let raw_key = key.into_u64();
        self.auto_divider(raw_key);
        let mut row_desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::ltrb(16.0, 12.0, 16.0, 12.0));
        row_desc.flex_shrink = 0.0;
        self.view.container(raw_key, row_desc, build)
    }
}

impl<'a, 'b> core::ops::Deref for SettingsCard<'a, 'b> {
    type Target = ViewBuilder<'a>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.view
    }
}

impl<'a, 'b> core::ops::DerefMut for SettingsCard<'a, 'b> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.view
    }
}

impl<'a> ViewBuilder<'a> {
    /// 声明 Win11 可读限宽内容列（最大宽度钳制在 [`unbound_theme::typography::geometry::READABLE_FORM_MAX_WIDTH`] = 1000px）。
    ///
    /// 在宽视口（如 1440p / 4K / 超宽屏）下将表单与设置卡片列宽钳制在 `1000px`，
    /// 避免标题与右侧行内控件被无限拉扯；在窄视口下 `width: 100%` 自动收缩至可用宽度。
    pub fn readable_column(
        &mut self,
        key: u64,
        gap: f32,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        self.readable_column_with_max_width(
            key,
            unbound_theme::typography::geometry::READABLE_FORM_MAX_WIDTH,
            gap,
            build,
        )
    }

    /// 声明自定义最大宽度的可读限宽内容列。
    ///
    /// 宽视口下宽度被钳制在 `max_width`（非负有限值，非有限值时回退至 `READABLE_FORM_MAX_WIDTH`），
    /// 窄视口下 `width: 100%` 自动收缩至父容器可用宽度。
    pub fn readable_column_with_max_width(
        &mut self,
        key: u64,
        max_width: f32,
        gap: f32,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let clamped_max = if max_width.is_finite() {
            max_width.max(0.0)
        } else {
            unbound_theme::typography::geometry::READABLE_FORM_MAX_WIDTH
        };
        let mut desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .gap(gap);
        desc.max_width = Dim::px(clamped_max);
        desc.flex_shrink = 0.0;
        self.container(key, desc, build)
    }

    /// 声明与 [`SettingsCard`] 同视觉语汇的可折叠设置分组卡片（Win11 Expander）。
    ///
    /// - 头部行直接绑定 `key` 并标记为可交互命中区：包含左侧可选图标、标题/副标题与右侧
    ///   [`ShellIcon::ChevronDown`]（展开）/[`ShellIcon::ChevronRight`]（折叠）矢量箭头；
    ///   应用在 `reduce` 中监听 `AppEvent::Click { node, .. } if node == key` 即可切换 `expanded` 状态。
    /// - 当 `expanded == true` 时在 1px 分隔线下渲染 [`SettingsCard`] 主体内容；
    ///   当 `expanded == false` 时仅渲染头部行并隐藏主体内容。
    pub fn settings_expander(
        &mut self,
        key: u64,
        icon: Option<ShellIcon>,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        expanded: bool,
        build_body: impl FnOnce(&mut SettingsCard<'_, '_>),
    ) -> NodeId {
        let title = title.into();
        let subtitle = subtitle.into();
        let card_key = mix_key(key, 0xe49a_ca2d);
        let card_radius = unbound_theme::typography::geometry::CARD_RADIUS;

        let mut card_desc = RawNodeDesc::container()
            .column()
            .width(Dim::percent(100.0))
            .padding(unbound_abi::Edges::all(0.0))
            .gap(0.0)
            .radius(card_radius)
            .fill_token(u32::from(unbound_theme::Token::CARD_BG.raw()))
            .border(u32::from(unbound_theme::Token::CARD_BORDER.raw()), 1.0);
        card_desc.flex_shrink = 0.0;

        let effective_key = if !self.key_scope.is_root() && key != 0 {
            self.key_scope.key(key)
        } else {
            key
        };
        let is_pressed = self.ctx.pressed == Some(effective_key);
        let is_hovered = self.ctx.hovered == Some(effective_key);
        let chevron = if expanded {
            ShellIcon::ChevronDown
        } else {
            ShellIcon::ChevronRight
        };

        let mut header_intent =
            SemanticIntent::new(Role::Button, title.clone()).with_expanded(expanded);
        if !subtitle.is_empty() {
            header_intent = header_intent.with_description(subtitle.clone());
        }
        self.declare_semantics(key, header_intent);

        self.container(card_key, card_desc, |ui| {
            let left_inset = if icon.is_some() {
                unbound_theme::typography::geometry::SETTINGS_ROW_ICON_INSET
            } else {
                unbound_theme::typography::geometry::SETTINGS_ROW_TEXT_INSET
            };
            let mut header_desc = RawNodeDesc::container()
                .column()
                .width(Dim::percent(100.0))
                .justify(unbound_abi::Justify::Center)
                .padding(unbound_abi::Edges::ltrb(left_inset, 12.0, 16.0, 12.0))
                .gap(2.0)
                .flags(NodeFlags::INTERACTIVE);
            header_desc.min_height = Dim::px(48.0);
            header_desc.flex_shrink = 0.0;
            header_desc.radius = if expanded {
                [card_radius, card_radius, 0.0, 0.0]
            } else {
                [card_radius, card_radius, card_radius, card_radius]
            };
            if is_pressed {
                header_desc =
                    header_desc.fill_token(u32::from(unbound_theme::Token::PRESSED.raw()));
            } else if is_hovered {
                header_desc = header_desc.fill_token(u32::from(unbound_theme::Token::HOVER.raw()));
            }

            ui.container(key, header_desc, |header| {
                if let Some(ic) = icon {
                    header.row_start(
                        mix_key(key, 0x1),
                        unbound_theme::typography::geometry::SETTINGS_ROW_ICON_GAP,
                        |r| {
                            r.shell_icon(
                                mix_key(key, 0x2),
                                ShellIconDesc::new(ic).size(
                                    unbound_theme::typography::geometry::SETTINGS_ROW_ICON_SIZE,
                                ),
                            );
                            r.col_grow(mix_key(key, 0x3), 2.0, |left| {
                                left.text_primary(
                                    mix_key(key, 0x4),
                                    title,
                                    unbound_theme::typography::font_size::BODY,
                                    unbound_theme::typography::weight::SEMIBOLD,
                                );
                                if !subtitle.is_empty() {
                                    left.text_muted(
                                        mix_key(key, 0x5),
                                        subtitle,
                                        unbound_theme::typography::font_size::CAPTION,
                                    );
                                }
                            });
                            r.row_center(mix_key(key, 0x6), 8.0, |right| {
                                right.shell_icon(
                                    mix_key(key, 0x7),
                                    ShellIconDesc::new(chevron).size(12.0),
                                );
                            });
                        },
                    );
                } else {
                    header.row_between(mix_key(key, 0x1), |r| {
                        r.col_grow(mix_key(key, 0x2), 2.0, |left| {
                            left.text_primary(
                                mix_key(key, 0x3),
                                title,
                                unbound_theme::typography::font_size::BODY,
                                unbound_theme::typography::weight::SEMIBOLD,
                            );
                            if !subtitle.is_empty() {
                                left.text_muted(
                                    mix_key(key, 0x4),
                                    subtitle,
                                    unbound_theme::typography::font_size::CAPTION,
                                );
                            }
                        });
                        r.row_center(mix_key(key, 0x5), 8.0, |right| {
                            right.shell_icon(
                                mix_key(key, 0x6),
                                ShellIconDesc::new(chevron).size(12.0),
                            );
                        });
                    });
                }
            });

            if expanded {
                ui.divider(mix_key(key, 0xe49a_d171), DividerDesc::rule());
                let mut card = SettingsCard::new(ui, card_key);
                build_body(&mut card);
            }
        })
    }

    /// 渲染标准的 28×28px 行尾 `···` 矢量图标按钮（`ButtonVariant::Subtle` + [`ShellIcon::MoreHorizontal`]）。
    ///
    /// - 点击该按钮将通过路由器精确命中 `trigger_key`（`AppEvent::Click { node, .. } if node == trigger_key`）；
    /// - 当 `tooltip_or_label` 非空时自动声明按钮的无障碍名称语义（`Role::Button`）。
    pub fn overflow_menu_trigger(&mut self, trigger_key: u64, tooltip_or_label: &str) -> NodeId {
        let btn = ButtonDesc::label("")
            .variant(ButtonVariant::Subtle)
            .size(ButtonSize::Sm)
            .shell_icon(ShellIcon::MoreHorizontal);

        let mut layout = RawNodeDesc::container();
        layout.width = Dim::px(OVERFLOW_TRIGGER_SIZE);
        layout.height = Dim::px(OVERFLOW_TRIGGER_SIZE);
        layout.flex_shrink = 0.0;

        if !tooltip_or_label.is_empty() {
            self.declare_semantics(
                trigger_key,
                SemanticIntent::new(Role::Button, tooltip_or_label),
            );
        }

        self.add_with_layout(trigger_key, layout, btn)
    }

    /// 在根浮层容器绘制锚定于 `anchor_rect` 的标准 Win11 溢出菜单弹出层。
    ///
    /// - `items` 为空时直接返回 `None`；
    /// - 复用 [`unbound_widgets::overlay::place_anchored_popup`]（首选 [`Placement::BottomEnd`]）
    ///   进行视口边界自动翻转与钳制，保证靠近右侧或底边时不溢出视口；
    /// - 弹出层挂载至根节点的 Portal 浮层（`z_index = 1260` + `portal_root()`），
    ///   每个可交互菜单项按索引确定性绑定 [`overflow_menu_item_key`]，点击即可精确命中。
    pub fn overflow_menu_popup(
        &mut self,
        trigger_key: u64,
        anchor_rect: Rect,
        viewport: Vec2,
        items: &[MenuItem],
    ) -> Option<NodeId> {
        if items.is_empty() {
            return None;
        }

        let mut est_width = OVERFLOW_POPUP_MIN_WIDTH;
        for item in items {
            if item.separator {
                continue;
            }
            let label_w: f32 = item
                .label
                .chars()
                .map(|c| if c.is_ascii() { 8.0 } else { 14.5 })
                .sum();
            let shortcut_w: f32 = if item.shortcut.is_empty() {
                0.0
            } else {
                16.0 + item
                    .shortcut
                    .chars()
                    .map(|c| if c.is_ascii() { 7.5 } else { 13.0 })
                    .sum::<f32>()
            };
            let row_w = label_w + shortcut_w + 36.0;
            if row_w > est_width {
                est_width = row_w;
            }
        }
        let popup_width = est_width.clamp(OVERFLOW_POPUP_MIN_WIDTH, OVERFLOW_POPUP_MAX_WIDTH);

        let items_height: f32 = items
            .iter()
            .map(|item| {
                if item.separator {
                    OVERFLOW_SEPARATOR_HEIGHT
                } else {
                    MENU_ITEM_HEIGHT
                }
            })
            .sum();
        let popup_height = items_height + OVERFLOW_POPUP_PAD_Y * 2.0;

        let vp = if viewport.x > 0.0 && viewport.y > 0.0 {
            viewport
        } else {
            self.ctx.viewport
        };
        let full_viewport = Rect::new(0.0, 0.0, vp.x.max(0.0), vp.y.max(0.0));
        let safe_viewport = if full_viewport.width() > OVERFLOW_POPUP_VIEWPORT_MARGIN * 2.0
            && full_viewport.height() > OVERFLOW_POPUP_VIEWPORT_MARGIN * 2.0
        {
            full_viewport.inflate(-OVERFLOW_POPUP_VIEWPORT_MARGIN)
        } else {
            full_viewport
        };

        let placed: PlacedPopup = place_anchored_popup(
            anchor_rect,
            Vec2::new(popup_width, popup_height),
            safe_viewport,
            Placement::BottomEnd,
        );
        let rect = placed.rect;

        let popup_key = overflow_menu_popup_key(trigger_key);
        let mut popup_desc = RawNodeDesc::container()
            .column()
            .absolute()
            .bounds(rect.origin.x, rect.origin.y, rect.width(), rect.height())
            .z_index(OVERFLOW_POPUP_Z)
            .portal_root()
            .padding(Edges::ltrb(
                0.0,
                OVERFLOW_POPUP_PAD_Y,
                0.0,
                OVERFLOW_POPUP_PAD_Y,
            ))
            .gap(0.0)
            .radius(MENU_RADIUS)
            .fill_token(u32::from(unbound_theme::Token::SURFACE_RAISED.raw()))
            .border(u32::from(unbound_theme::Token::OUTLINE.raw()), 1.0);
        popup_desc.shadow_blur = MENU_SHADOW;

        let parent_saved = core::mem::replace(&mut self.parent, self.world.root());
        let popup_node = self.container(popup_key, popup_desc, |popup| {
            for (idx, item) in items.iter().enumerate() {
                if item.separator {
                    let sep_row_key = mix_key(popup_key, 0x5e9a_0000u64 ^ (idx as u64));
                    let mut sep_row = RawNodeDesc::container()
                        .column()
                        .justify(Justify::Center)
                        .width(Dim::percent(100.0))
                        .height(Dim::px(OVERFLOW_SEPARATOR_HEIGHT))
                        .padding(Edges::ltrb(8.0, 4.0, 8.0, 4.0));
                    sep_row.flex_shrink = 0.0;
                    popup.container(sep_row_key, sep_row, |row| {
                        row.divider(mix_key(sep_row_key, 0xd171de), DividerDesc::rule());
                    });
                    continue;
                }

                let item_key = overflow_menu_item_key(trigger_key, idx);

                if item.disabled {
                    popup.declare_semantics(
                        item_key,
                        SemanticIntent::new(Role::MenuItem, item.label.clone()).with_disabled(true),
                    );
                    let mut row_desc = RawNodeDesc::container()
                        .column()
                        .justify(Justify::Center)
                        .width(Dim::percent(100.0))
                        .height(Dim::px(MENU_ITEM_HEIGHT))
                        .padding(Edges::ltrb(
                            12.0 + OVERFLOW_ITEM_INSET_X,
                            0.0,
                            12.0 + OVERFLOW_ITEM_INSET_X,
                            0.0,
                        ));
                    row_desc.flex_shrink = 0.0;
                    popup.container(item_key, row_desc, |row| {
                        row.text_muted(
                            mix_key(item_key, 0x1),
                            item.label.clone(),
                            unbound_theme::typography::font_size::BODY,
                        );
                    });
                    continue;
                }

                let is_selected = item.selected || item.checked == Some(true);
                let display_label = if item.shortcut.is_empty() {
                    item.label.clone()
                } else {
                    format!("{}  {}", item.label, item.shortcut)
                };
                let tokens = if is_selected {
                    ButtonTokens {
                        fill: unbound_theme::Token::HOVER,
                        fill_hover: unbound_theme::Token::HOVER,
                        fill_pressed: unbound_theme::Token::PRESSED,
                        fill_disabled: unbound_theme::Token::NONE,
                        border: unbound_theme::Token::NONE,
                        label: unbound_theme::Token::TEXT_PRIMARY,
                        label_disabled: unbound_theme::Token::ON_DISABLED,
                        focus_ring: unbound_theme::Token::NONE,
                    }
                } else {
                    ButtonTokens {
                        fill: unbound_theme::Token::NONE,
                        fill_hover: unbound_theme::Token::CONTROL_HOVER,
                        fill_pressed: unbound_theme::Token::SURFACE_SUNKEN,
                        fill_disabled: unbound_theme::Token::NONE,
                        border: unbound_theme::Token::NONE,
                        label: unbound_theme::Token::TEXT_PRIMARY,
                        label_disabled: unbound_theme::Token::ON_DISABLED,
                        focus_ring: unbound_theme::Token::NONE,
                    }
                };

                let mut btn = ButtonDesc::label(display_label)
                    .variant(ButtonVariant::Subtle)
                    .left_align()
                    .selected(is_selected)
                    .with_font_size(unbound_theme::typography::font_size::BODY)
                    .with_label_inset(12.0)
                    .with_tokens(tokens);
                btn.radius = 4.0;
                btn.shadow = 0.0;

                popup.declare_semantics(
                    item_key,
                    SemanticIntent::new(Role::MenuItem, item.label.clone())
                        .with_selected(is_selected),
                );

                let mut row_desc = RawNodeDesc::container().column();
                row_desc.width = Dim::percent(100.0);
                row_desc.height = Dim::px(MENU_ITEM_HEIGHT);
                row_desc.padding = Edges::ltrb(
                    OVERFLOW_ITEM_INSET_X,
                    OVERFLOW_ITEM_INSET_Y,
                    OVERFLOW_ITEM_INSET_X,
                    OVERFLOW_ITEM_INSET_Y,
                );
                row_desc.flex_shrink = 0.0;
                let item_box_key = mix_key(popup_key, (idx as u64) + 1);
                popup.container(item_box_key, row_desc, |row| {
                    let mut item_layout = RawNodeDesc::container();
                    item_layout.width = Dim::percent(100.0);
                    item_layout.height = Dim::px(MENU_ITEM_HEIGHT - 2.0 * OVERFLOW_ITEM_INSET_Y);
                    row.add_with_layout(item_key, item_layout, btn);
                });
            }
        });
        self.parent = parent_saved;

        Some(popup_node)
    }
}
