//! 应用侧边栏（Sidebar）：整栏面板、分组标题、导航条目与页脚固定条目。
//!
//! 本模块把「整栏」侧边栏收敛为一个标准控件：面板底色与描边、分组小节、
//! 条目胶囊、页脚分隔线与固定项全部来自 [`SidebarDesc`] 的同一份几何。
//! 条目视觉完全复用 [`crate::shell::emit_sidebar_item`]，本模块只负责垂直排版、
//! 选中 / 悬停归属与命中切分，避免组合层与应用层各自重复实现一套条目。
//!
//! 三条契约与外壳组合层保持一致：
//! 1. **跨状态图元数量恒定**：[`ControlState`] 只修改面板描边与焦点环的字段；
//!    条目自身的悬停 / 选中由描述符（`hovered_id` / `active_id`）驱动，绝不增删图元。
//! 2. **颜色全部走语义令牌**：面板、字母、图标与指示条只引用 [`Token`]。
//! 3. **双模复用**：`emit_sidebar` 面向画布与无头渲染器，[`UiWidget`] 面向声明式视图树。
//!
//! 面板不做内部滚动：与其它标准控件一致，滚动偏移归应用运行时所有。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::ergonomics::{AlignX, AlignY, TextCtx, UiWidget};
use crate::primitive::{glyphs, outlined, surface};
use crate::shell::{emit_sidebar_item, ShellIcon, SidebarItemDesc};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 展开模式下侧边栏的默认宽度。
pub const SIDEBAR_WIDTH: f32 = 244.0;

/// 折叠（仅图标）模式下侧边栏的默认宽度。
pub const SIDEBAR_COLLAPSED_WIDTH: f32 = 64.0;

/// 导航条目的默认行高。
pub const SIDEBAR_ITEM_HEIGHT: f32 = 34.0;

/// 分组标题的默认行高。
pub const SIDEBAR_SECTION_HEIGHT: f32 = 26.0;

/// 分组之间的额外垂直间距。
pub const SIDEBAR_SECTION_GAP: f32 = 6.0;

/// 面板内容区四周的内边距。
pub const SIDEBAR_PADDING: f32 = 10.0;

/// 页脚与滚动区之间的留白（含分隔线）。
pub const SIDEBAR_FOOTER_GAP: f32 = 12.0;

/// 条目尾部徽标胶囊的固定宽度。
pub const SIDEBAR_BADGE_WIDTH: f32 = 26.0;

/// 条目尾部徽标胶囊的高度。
pub const SIDEBAR_BADGE_HEIGHT: f32 = 18.0;

/// 侧边栏中的单个导航条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarEntry {
    /// 稳定的条目标识，同时用于比较 `active_id` / `hovered_id`。
    pub id: u64,
    /// 条目文本标签。
    pub label: String,
    /// 条目左侧几何矢量图标。
    pub icon: ShellIcon,
    /// 可选尾部徽标文本（如未读数或快捷键提示）。
    pub badge: Option<String>,
}

impl SidebarEntry {
    /// 创建一个导航条目。
    #[must_use]
    pub fn new(id: u64, label: impl Into<String>, icon: ShellIcon) -> Self {
        Self {
            id,
            label: label.into(),
            icon,
            badge: None,
        }
    }

    /// 附加尾部徽标文本。
    #[must_use]
    pub fn badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }
}

/// 侧边栏中的一个分组小节（大写标题 + 若干条目）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarSection {
    /// 分组标题（折叠模式下隐藏）。
    pub title: String,
    /// 分组内的条目，按声明顺序自上而下排列。
    pub entries: Vec<SidebarEntry>,
}

impl SidebarSection {
    /// 创建空分组。
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            entries: Vec::new(),
        }
    }

    /// 追加一个条目。
    #[must_use]
    pub fn with_entry(mut self, entry: SidebarEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// 批量追加条目。
    #[must_use]
    pub fn with_entries(mut self, entries: impl IntoIterator<Item = SidebarEntry>) -> Self {
        self.entries.extend(entries);
        self
    }

    /// 该分组是否包含给定条目标识。
    #[must_use]
    pub fn contains(&self, id: u64) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }
}

/// 侧边栏一行在垂直排版中的角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarRowKind {
    /// 分组标题行。
    Section {
        /// 分组下标。
        section: usize,
    },
    /// 分组内的导航条目行。
    Item {
        /// 分组下标。
        section: usize,
        /// 分组内条目下标。
        entry: usize,
    },
    /// 页脚固定条目行。
    FooterItem {
        /// 页脚下标。
        entry: usize,
    },
}

/// 侧边栏的一行（角色 + 已排版矩形）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SidebarRow {
    /// 行的角色与内容定位。
    pub kind: SidebarRowKind,
    /// 行矩形（已扣除内边距）。
    pub rect: Rect,
}

/// 与 [`SidebarDesc::rows`] 一一对应的已塑形字形。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SidebarRowGlyphs {
    /// 标题或条目标签字形。
    pub label: Vec<GlyphQuad>,
    /// 尾部徽标字形（无徽标时为空）。
    pub badge: Vec<GlyphQuad>,
}

/// 侧边栏的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarDesc {
    /// 面板矩形。
    pub rect: Rect,
    /// 分组列表。
    pub sections: Vec<SidebarSection>,
    /// 贴着面板底部固定显示的页脚条目。
    pub footer: Vec<SidebarEntry>,
    /// 当前激活条目标识。
    pub active_id: u64,
    /// 当前指针悬停条目标识（`0` 表示无）。
    pub hovered_id: u64,
    /// 是否折叠为仅图标模式。
    pub collapsed: bool,
    /// 展开模式宽度。
    pub width: f32,
    /// 折叠模式宽度。
    pub collapsed_width: f32,
    /// 条目行高。
    pub item_height: f32,
    /// 分组标题行高。
    pub section_height: f32,
    /// 无障碍标签。
    pub label: String,
    /// 绘制深度。
    pub z: u16,
}

impl SidebarDesc {
    /// 创建侧边栏描述符。
    #[must_use]
    pub fn new(rect: Rect, sections: Vec<SidebarSection>) -> Self {
        Self {
            rect,
            sections,
            footer: Vec::new(),
            active_id: 0,
            hovered_id: 0,
            collapsed: false,
            width: SIDEBAR_WIDTH,
            collapsed_width: SIDEBAR_COLLAPSED_WIDTH,
            item_height: SIDEBAR_ITEM_HEIGHT,
            section_height: SIDEBAR_SECTION_HEIGHT,
            label: "Sidebar".to_owned(),
            z: 610,
        }
    }

    /// 设置底部固定条目。
    #[must_use]
    pub fn footer(mut self, footer: Vec<SidebarEntry>) -> Self {
        self.footer = footer;
        self
    }

    /// 设置当前激活条目标识。
    #[must_use]
    pub const fn active(mut self, id: u64) -> Self {
        self.active_id = id;
        self
    }

    /// 设置当前悬停条目标识。
    #[must_use]
    pub const fn hovered(mut self, id: u64) -> Self {
        self.hovered_id = id;
        self
    }

    /// 设置是否折叠为仅图标模式。
    #[must_use]
    pub const fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 面板当前宽度（折叠状态取折叠宽度）。
    #[must_use]
    pub fn panel_width(&self) -> f32 {
        if self.collapsed {
            self.collapsed_width.min(self.rect.width().max(1.0))
        } else {
            self.width.min(self.rect.width().max(1.0))
        }
    }

    /// 分组是否包含当前激活条目（标题随之高亮为主色）。
    #[must_use]
    pub fn section_active(&self, index: usize) -> bool {
        self.sections
            .get(index)
            .is_some_and(|section| section.contains(self.active_id))
    }

    /// 页脚固定条目占用的总高度。
    #[must_use]
    pub fn footer_height(&self) -> f32 {
        if self.footer.is_empty() {
            0.0
        } else {
            SIDEBAR_FOOTER_GAP + self.footer.len() as f32 * self.item_height
        }
    }

    /// 计算垂直排版结果（分组标题 + 条目 + 页脚），返回所有放得下的行。
    ///
    /// 放不下的行会被裁剪：面板不做内部滚动，超出高度的内容由调用方
    /// 通过调节 `rect` 或折叠模式处理。
    #[must_use]
    pub fn rows(&self) -> Vec<SidebarRow> {
        let width = (self.panel_width() - SIDEBAR_PADDING * 2.0).max(1.0);
        let x = self.rect.origin.x + SIDEBAR_PADDING;
        let mut y = self.rect.origin.y + SIDEBAR_PADDING;
        let limit = self.rect.max().y - SIDEBAR_PADDING - self.footer_height();

        let mut rows = Vec::new();
        'sections: for (section_index, section) in self.sections.iter().enumerate() {
            if !self.collapsed {
                if y + self.section_height > limit {
                    break 'sections;
                }
                rows.push(SidebarRow {
                    kind: SidebarRowKind::Section {
                        section: section_index,
                    },
                    rect: Rect::new(x, y, width, self.section_height),
                });
                y += self.section_height;
            }
            for entry_index in 0..section.entries.len() {
                if y + self.item_height > limit {
                    break 'sections;
                }
                rows.push(SidebarRow {
                    kind: SidebarRowKind::Item {
                        section: section_index,
                        entry: entry_index,
                    },
                    rect: Rect::new(x, y, width, self.item_height),
                });
                y += self.item_height;
            }
            y += SIDEBAR_SECTION_GAP;
        }

        let mut footer_y = limit + SIDEBAR_FOOTER_GAP;
        for entry_index in 0..self.footer.len() {
            rows.push(SidebarRow {
                kind: SidebarRowKind::FooterItem { entry: entry_index },
                rect: Rect::new(x, footer_y, width, self.item_height),
            });
            footer_y += self.item_height;
        }

        rows
    }

    /// 取出某一行对应的条目（分组标题行返回 `None`）。
    #[must_use]
    pub fn row_entry(&self, row: &SidebarRow) -> Option<&SidebarEntry> {
        match row.kind {
            SidebarRowKind::Section { .. } => None,
            SidebarRowKind::Item { section, entry } => {
                self.sections.get(section)?.entries.get(entry)
            }
            SidebarRowKind::FooterItem { entry } => self.footer.get(entry),
        }
    }

    /// 把某一行包装成复用的条目描述符。
    #[must_use]
    pub fn item_desc(&self, row: &SidebarRow, entry: &SidebarEntry) -> SidebarItemDesc {
        let mut item = SidebarItemDesc::new(entry.label.clone(), entry.icon)
            .with_rect(row.rect)
            .selected(entry.id == self.active_id)
            .collapsed(self.collapsed)
            .height(row.rect.height());
        item.z = self.z + 2;
        if let Some(badge) = &entry.badge {
            item = item.badge(badge.clone());
        }
        item
    }

    /// 行标签的排版矩形（与发射器内部几何完全一致，供调用方塑形字形）。
    #[must_use]
    pub fn row_label_rect(&self, row: &SidebarRow) -> Rect {
        match (row.kind, self.row_entry(row)) {
            (SidebarRowKind::Section { .. }, _) => Rect::new(
                row.rect.origin.x + 8.0,
                row.rect.origin.y,
                (row.rect.width() - 12.0).max(0.0),
                row.rect.height(),
            ),
            (_, Some(entry)) => {
                let base = self.item_desc(row, entry).label_rect(0.0);
                if self.collapsed || entry.badge.is_none() {
                    return base;
                }
                let badge = self.row_badge_rect(row);
                Rect::new(
                    base.origin.x,
                    base.origin.y,
                    (badge.origin.x - 6.0 - base.origin.x).max(0.0),
                    base.size.y,
                )
            }
            (_, None) => Rect::ZERO,
        }
    }

    /// 行尾部徽标胶囊矩形；折叠模式或无徽标时返回零矩形。
    #[must_use]
    pub fn row_badge_rect(&self, row: &SidebarRow) -> Rect {
        if self.collapsed {
            return Rect::ZERO;
        }
        Rect::new(
            row.rect.max().x - SIDEBAR_BADGE_WIDTH - 8.0,
            row.rect.origin.y + (row.rect.height() - SIDEBAR_BADGE_HEIGHT) * 0.5,
            SIDEBAR_BADGE_WIDTH,
            SIDEBAR_BADGE_HEIGHT,
        )
    }
}

/// 侧边栏行的确定性子节点 ID。
#[must_use]
pub const fn sidebar_row_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ (0x5B0 + index as u64)
}

/// 把子发射结果并入整栏输出：重定位载荷索引，并把命中区与语义节点降为子控件。
fn append_child(out: &mut WidgetOutput, mut child: WidgetOutput, key: u64) {
    let payload_base = u32::try_from(out.payloads.len()).unwrap_or(u32::MAX);
    if payload_base != 0 {
        for primitive in &mut child.primitives {
            primitive.extra_payload_idx = primitive.extra_payload_idx.saturating_add(payload_base);
        }
    }
    out.primitives.extend(child.primitives);
    out.payloads.extend(child.payloads);
    if let (Some(hit), Some(a11y)) = (child.hit.take(), child.a11y.take()) {
        out.with_sub_control(key, hit, a11y, child.focusable);
    }
}

/// 发射整栏侧边栏（面板 + 分组标题 + 条目 + 页脚）。
///
/// `row_glyphs` 必须与 [`SidebarDesc::rows`] 一一对应；缺省的行按空字形处理。
/// 返回的命中区包含整块面板与每个条目行，条目行以 [`sidebar_row_node`] 派生的子键区分。
#[must_use]
pub fn emit_sidebar(
    node: u64,
    desc: &SidebarDesc,
    control: ControlState,
    row_glyphs: &[SidebarRowGlyphs],
) -> WidgetOutput {
    let hovered = matches!(control.phase, PointerPhase::Hovered | PointerPhase::Pressed);
    let mut out = WidgetOutput::default();
    out.primitives.push(outlined(
        desc.rect,
        0.0,
        Token::NAV_BG,
        if hovered || control.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let rows = desc.rows();
    for (index, row) in rows.iter().enumerate() {
        let glyph_pair = row_glyphs.get(index);
        let label_glyphs = glyph_pair.map_or(&[][..], |pair| pair.label.as_slice());
        match row.kind {
            SidebarRowKind::Section { section } => {
                let color = if desc.section_active(section) {
                    Token::PRIMARY
                } else {
                    Token::NAV_TEXT_MUTED
                };
                glyphs(
                    label_glyphs,
                    color,
                    desc.z + 1,
                    &mut out.primitives,
                    &mut out.payloads,
                );
            }
            SidebarRowKind::Item { .. } | SidebarRowKind::FooterItem { .. } => {
                let Some(entry) = desc.row_entry(row) else {
                    continue;
                };
                let item = desc.item_desc(row, entry);
                let row_control = ControlState {
                    phase: if entry.id == desc.hovered_id {
                        PointerPhase::Hovered
                    } else {
                        PointerPhase::Idle
                    },
                    focused: false,
                    disabled: control.disabled,
                    selected: item.selected,
                };
                let badge_glyphs = glyph_pair.map_or(&[][..], |pair| pair.badge.as_slice());
                let badge_rect = if desc.collapsed || entry.badge.is_none() {
                    None
                } else {
                    Some(desc.row_badge_rect(row))
                };
                let row_node = sidebar_row_node(node, index);
                let child = emit_sidebar_item(
                    row_node,
                    &item,
                    row_control,
                    label_glyphs,
                    badge_glyphs,
                    badge_rect,
                );
                append_child(&mut out, child, row_node);
            }
        }
    }

    if !desc.footer.is_empty() {
        let divider_y = desc.rect.max().y - SIDEBAR_PADDING - desc.footer_height();
        out.primitives.push(surface(
            Rect::new(
                desc.rect.origin.x + SIDEBAR_PADDING,
                divider_y,
                (desc.panel_width() - SIDEBAR_PADDING * 2.0).max(1.0),
                1.0,
            ),
            0.0,
            Token::DIVIDER,
            desc.z + 1,
        ));
    }

    let mut a11y = A11yNode::new(Role::Navigation, desc.label.clone(), desc.rect);
    a11y.disabled = control.disabled;
    out.a11y = Some(a11y);
    out.hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    out.focusable = false;
    out
}

impl UiWidget for SidebarDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let mut height = SIDEBAR_PADDING * 2.0 + self.footer_height();
        for section in &self.sections {
            if !self.collapsed {
                height += self.section_height;
            }
            height += section.entries.len() as f32 * self.item_height + SIDEBAR_SECTION_GAP;
        }
        Vec2::new(self.panel_width(), height)
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let rows = desc.rows();
        let mut shaped = Vec::with_capacity(rows.len());
        for row in &rows {
            match row.kind {
                SidebarRowKind::Section { section } => {
                    let title = desc
                        .sections
                        .get(section)
                        .map_or("", |value| value.title.as_str());
                    shaped.push(SidebarRowGlyphs {
                        label: text_ctx.shape_line(
                            title,
                            desc.row_label_rect(row),
                            10.5,
                            700,
                            AlignX::Left,
                            AlignY::Middle,
                        ),
                        badge: Vec::new(),
                    });
                }
                SidebarRowKind::Item { .. } | SidebarRowKind::FooterItem { .. } => {
                    let Some(entry) = desc.row_entry(row) else {
                        shaped.push(SidebarRowGlyphs::default());
                        continue;
                    };
                    let item = desc.item_desc(row, entry);
                    let weight = if item.selected { 600 } else { 400 };
                    let label = text_ctx.shape_line(
                        &item.label,
                        desc.row_label_rect(row),
                        12.5,
                        weight,
                        AlignX::Left,
                        AlignY::Middle,
                    );
                    let badge = match &entry.badge {
                        Some(text) if !desc.collapsed => text_ctx.shape_line(
                            text,
                            desc.row_badge_rect(row),
                            10.5,
                            600,
                            AlignX::Center,
                            AlignY::Middle,
                        ),
                        _ => Vec::new(),
                    };
                    shaped.push(SidebarRowGlyphs { label, badge });
                }
            }
        }
        emit_sidebar(node, &desc, control, &shaped)
    }
}
