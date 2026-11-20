//! lang-allow: 中文注释与文档
//! Win11 规范设置卡片（Settings Card）与设置行排版构建器。
//!
//! 提供符合 Windows 11 设计语言的自包含设置卡片，内置：
//! 1. 恒定 57px 文本左内边距（无图标）或 16px 图标槽 + 22px 间距（带图标）；
//! 2. 连续添加多行时自动插入 1px 细线分隔线（`DividerDesc::rule()`），
//!    彻底消除调用方手工计算键值或追加分隔线的样板代码；
//! 3. 下拉选择框弹出浮层（ComboBox Popup Overlay）的确定性键派生。

use unbound_abi::{Dim, RawNodeDesc};
use unbound_widgets::divider::DividerDesc;
use unbound_widgets::{ShellIcon, ShellIconDesc};

use crate::{mix_key, KeyDiscriminator, NodeId, ViewBuilder};

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
