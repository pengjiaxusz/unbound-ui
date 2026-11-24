//! 首页落地页（Landing）：无示例外框、无旋钮与代码折叠条，专注品牌首屏、引擎核心能力、
//! 最近访问/收藏快捷区与 10 大功能分类导览网格（ADR 0035）。
//! lang-allow: bilingual home landing copy; Chinese is the zh-CN half

use accesskit::Role;
use unbound_app::Locale;
use unbound_input::CursorKind;
use unbound_math::Rect;
use unbound_widgets::{ControlCategory, ControlClass, WidgetCatalogId};

use super::{
    category_localized_label, category_localized_summary, component_nav_label_localized,
    control_class_badge, DemoCtx, DemoKind, ExampleDef, PageId, PageInfo,
};
use crate::canvas::{AlignX, AlignY, Canvas};
use crate::state::GalleryState;
use crate::target::{self, Target};
use crate::widgets::{self, ButtonVariant, Icon};

pub const INFO: PageInfo = PageInfo {
    title: "Unbound UI Gallery",
    short: "Home",
    description: "A live tour of the GPU-driven UI engine: 133 categorized controls, infinite canvas, MTSDF typography, and WinUI 3 pixel parity.",
    crates: &["unbound-gpu", "unbound-widgets", "unbound-canvas", "unbound-text"],
    keywords: "overview hero gallery start home controls categories",
};

/// 首屏 CTA 跳转的目标页面。
pub const HERO_TARGETS: [PageId; 2] = [PageId::Components, PageId::Viewport];

/// 首次打开且尚无最近访问记录时展示的精选推荐控件。
pub const FEATURED_CONTROLS: [WidgetCatalogId; 4] = [
    WidgetCatalogId::Button,
    WidgetCatalogId::NavigationView,
    WidgetCatalogId::DataGrid,
    WidgetCatalogId::NodeGraph,
];

/// 四项核心引擎特性展示。
pub const CAPABILITIES: [(&str, &str, PageId, Icon); 4] = [
    (
        "Typography",
        "MTSDF glyphs at every zoom",
        PageId::Typography,
        Icon::Layers,
    ),
    (
        "Virtualized List",
        "100 000 rows, 12 active nodes",
        PageId::ListView,
        Icon::Grid,
    ),
    (
        "Infinite Canvas",
        "Quadtree culling and snapping",
        PageId::Viewport,
        Icon::Snap,
    ),
    (
        "Pixel Parity",
        "WinUI 3 ROI 4-gate inspector",
        PageId::Parity,
        Icon::Chart,
    ),
];

/// 按当前语言返回四项核心特性的本地化标题与副标题。
const fn localized_capability(index: usize, locale: Locale) -> (&'static str, &'static str) {
    match locale {
        Locale::EnUs => (CAPABILITIES[index].0, CAPABILITIES[index].1),
        Locale::ZhCn => match index {
            0 => ("字体与排版", "任意缩放比下的高精 MTSDF 字形"),
            1 => ("虚拟化列表", "10 万行数据集，仅 12 个活跃节点"),
            2 => ("无限画布", "四叉树空间剔除与磁吸对齐"),
            _ => ("像素对照检视", "WinUI 3 ROI 四重门禁与卷帘对比"),
        },
    }
}

/// 保留兼容导出：指向核心引擎专题页。
pub const INDEX_PAGES: [PageId; 12] = [
    PageId::Colors,
    PageId::Typography,
    PageId::Buttons,
    PageId::Toggles,
    PageId::Sliders,
    PageId::TextField,
    PageId::Flex,
    PageId::Overlays,
    PageId::ListView,
    PageId::Viewport,
    PageId::I18n,
    PageId::A11y,
];

/// 保留非空元数据以兼容现有静态检查；实际首页走 [`draw_landing`] 落地页模板，
/// 不再套用 `ExampleCard` 外框、旋钮与代码折叠条。
pub const EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "Unbound UI Gallery",
    "Landing page rendered directly without ExampleCard chrome.",
    &["unbound-gpu", "unbound-widgets"],
    "",
    "Landing page with hero banner, engine highlights, quick access, and 10 control categories.",
    &[],
    DemoKind::Hero,
)
.with_height(220.0)];

const HERO_HEIGHT: f32 = 196.0;
const SECTION_GAP: f32 = 24.0;
const SECTION_HEAD_H: f32 = 30.0;
const CAP_CARD_H: f32 = 76.0;
const QUICK_CARD_H: f32 = 74.0;
const CAT_CARD_H: f32 = 82.0;
const GRID_GAP: f32 = 12.0;

fn category_columns(width: f32) -> usize {
    if width >= 1040.0 {
        4
    } else if width >= 720.0 {
        3
    } else if width >= 440.0 {
        2
    } else {
        1
    }
}

fn quick_columns(width: f32) -> usize {
    if width >= 920.0 {
        4
    } else if width >= 540.0 {
        2
    } else {
        1
    }
}

/// 计算首页落地页在指定宽度与状态下的总高度（逻辑像素）。
#[must_use]
pub fn landing_height(width: f32, state: &GalleryState) -> f32 {
    let cap_cols = if width >= 760.0 {
        4
    } else if width >= 440.0 {
        2
    } else {
        1
    };
    let cap_rows = CAPABILITIES.len().div_ceil(cap_cols);
    let cap_h = SECTION_HEAD_H + cap_rows as f32 * CAP_CARD_H + (cap_rows.saturating_sub(1)) as f32 * GRID_GAP;

    let quick_count = quick_access_items(state).len().max(1);
    let q_cols = quick_columns(width);
    let q_rows = if quick_access_items(state).is_empty() {
        1
    } else {
        quick_count.div_ceil(q_cols)
    };
    let quick_h = SECTION_HEAD_H + q_rows as f32 * QUICK_CARD_H + (q_rows.saturating_sub(1)) as f32 * GRID_GAP;

    let c_cols = category_columns(width);
    let c_rows = ControlCategory::ALL.len().div_ceil(c_cols);
    let cat_h = SECTION_HEAD_H + c_rows as f32 * CAT_CARD_H + (c_rows.saturating_sub(1)) as f32 * GRID_GAP;

    HERO_HEIGHT + SECTION_GAP + cap_h + SECTION_GAP + quick_h + SECTION_GAP + cat_h + 16.0
}

/// 返回首页快捷访问区当前展示的控件列表。
#[must_use]
pub fn quick_access_items(state: &GalleryState) -> Vec<WidgetCatalogId> {
    if state.home_quick_tab == 1 {
        state.favorite_controls.iter().copied().take(8).collect()
    } else if !state.recent_controls.is_empty() {
        state.recent_controls.iter().copied().take(8).collect()
    } else {
        FEATURED_CONTROLS.to_vec()
    }
}

/// 绘制首页落地页（无 `ExampleCard` 外框、无旋钮、无代码折叠条）。
pub fn draw_landing(canvas: &mut Canvas<'_>, state: &GalleryState, body: Rect) {
    let mut y = body.origin.y;
    let width = body.size.x;
    let zh = state.locale.is_zh();

    // 1. Compact Unbound Hero 首屏
    let hero_rect = Rect::new(body.origin.x, y, width, HERO_HEIGHT);
    draw_landing_hero(canvas, state, hero_rect, zh);
    y += HERO_HEIGHT + SECTION_GAP;

    // 2. Engine Highlights（4 张引擎能力横卡）
    draw_section_heading(
        canvas,
        Rect::new(body.origin.x, y, width, SECTION_HEAD_H),
        if zh { "引擎核心能力" } else { "Engine Highlights" },
        if zh {
            "直接进入 Unbound UI 的底层引擎实验室"
        } else {
            "Jump straight into Unbound UI's GPU-driven engine labs"
        },
    );
    y += SECTION_HEAD_H;
    let cap_cols = if width >= 760.0 {
        4
    } else if width >= 440.0 {
        2
    } else {
        1
    };
    let cap_w = (width - GRID_GAP * (cap_cols - 1) as f32) / cap_cols as f32;
    for (index, (_, _, page, icon)) in CAPABILITIES.iter().enumerate() {
        let col = index % cap_cols;
        let row = index / cap_cols;
        let cell = Rect::new(
            body.origin.x + col as f32 * (cap_w + GRID_GAP),
            y + row as f32 * (CAP_CARD_H + GRID_GAP),
            cap_w,
            CAP_CARD_H,
        );
        let (title, subtitle) = localized_capability(index, state.locale);
        let node = target::id(Target::NavPage(page.index() as u8));
        widgets::feature_card(canvas, node, cell, *icon, title, subtitle, title, index);
    }
    let cap_rows = CAPABILITIES.len().div_ceil(cap_cols);
    y += cap_rows as f32 * CAP_CARD_H + (cap_rows.saturating_sub(1)) as f32 * GRID_GAP + SECTION_GAP;

    // 3. Quick Access（Recent | Favorites 快捷访问区）
    draw_quick_access_header(canvas, state, Rect::new(body.origin.x, y, width, SECTION_HEAD_H), zh);
    y += SECTION_HEAD_H;
    let items = quick_access_items(state);
    let q_cols = quick_columns(width);
    if items.is_empty() {
        let palette = *canvas.palette;
        let empty_rect = Rect::new(body.origin.x, y, width, QUICK_CARD_H);
        canvas.rect(empty_rect, 10.0, palette.card_bg, palette.card_border, 1.0, 0.0);
        canvas.text(
            if zh {
                "暂无收藏控件 — 在任意分类索引或控件详情页点击星标即可收藏到这里。"
            } else {
                "No favorites yet — click the star on any control card or detail page to pin it here."
            },
            Rect::new(empty_rect.origin.x + 16.0, empty_rect.origin.y, empty_rect.size.x - 32.0, empty_rect.size.y),
            12.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        y += QUICK_CARD_H + SECTION_GAP;
    } else {
        let q_w = (width - GRID_GAP * (q_cols - 1) as f32) / q_cols as f32;
        for (idx, comp_id) in items.iter().copied().enumerate() {
            let col = idx % q_cols;
            let row = idx / q_cols;
            let cell = Rect::new(
                body.origin.x + col as f32 * (q_w + GRID_GAP),
                y + row as f32 * (QUICK_CARD_H + GRID_GAP),
                q_w,
                QUICK_CARD_H,
            );
            super::components::draw_control_tile(canvas, state, cell, comp_id);
        }
        let q_rows = items.len().div_ceil(q_cols);
        y += q_rows as f32 * QUICK_CARD_H + (q_rows.saturating_sub(1)) as f32 * GRID_GAP + SECTION_GAP;
    }

    // 4. Explore by Category（10 大功能分类导览网格）
    draw_section_heading(
        canvas,
        Rect::new(body.origin.x, y, width, SECTION_HEAD_H),
        if zh {
            "按功能分类浏览全部 133 个控件"
        } else {
            "Explore All 133 Controls by Category"
        },
        if zh {
            "由 WidgetCatalogId 穷举元数据自动派生"
        } else {
            "Derived directly from WidgetCatalogId exhaustive metadata"
        },
    );
    y += SECTION_HEAD_H;

    let c_cols = category_columns(width);
    let c_w = (width - GRID_GAP * (c_cols - 1) as f32) / c_cols as f32;
    let palette = *canvas.palette;
    for (idx, cat) in ControlCategory::ALL.iter().copied().enumerate() {
        let col = idx % c_cols;
        let row = idx / c_cols;
        let cell = Rect::new(
            body.origin.x + col as f32 * (c_w + GRID_GAP),
            y + row as f32 * (CAT_CARD_H + GRID_GAP),
            c_w,
            CAT_CARD_H,
        );
        let node = target::id(Target::NavCategory(cat.index() as u8));
        let visual = canvas.visual(node, false);
        let bg = if visual.is_pressed() || visual.is_hovered() {
            palette.primary_soft
        } else {
            palette.card_bg
        };
        let border = if visual.is_hovered() {
            palette.primary
        } else {
            palette.card_border
        };
        canvas.rect(cell, 10.0, bg, border, 1.0, 0.0);

        let (accent_fg, accent_bg) = category_accent(palette, idx);
        let icon_box = Rect::new(cell.origin.x + 12.0, cell.origin.y + 14.0, 36.0, 36.0);
        canvas.rect(icon_box, 8.0, accent_bg, 0, 0.0, 0.0);
        widgets::icon(canvas, category_widget_icon(cat), icon_box, accent_fg, accent_bg);

        let title = category_localized_label(cat, state.locale);
        let count_text = format!("{}", cat.count());
        let count_w = canvas.labels.measure(&count_text, 10.0).x + 14.0;
        let badge_rect = Rect::new(
            cell.max().x - count_w - 10.0,
            cell.origin.y + 12.0,
            count_w,
            18.0,
        );
        widgets::badge(canvas, badge_rect, &count_text, accent_fg, accent_bg);

        canvas.text(
            title,
            Rect::new(
                icon_box.max().x + 10.0,
                cell.origin.y + 12.0,
                (cell.size.x - 74.0 - count_w).max(40.0),
                18.0,
            ),
            12.5,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        let summary = category_localized_summary(cat, state.locale);
        canvas.text_wrapped(
            summary,
            Rect::new(
                icon_box.max().x + 10.0,
                cell.origin.y + 34.0,
                (cell.size.x - 70.0).max(60.0),
                40.0,
            ),
            10.5,
            palette.text_muted,
        );
        canvas.control(node, cell, Role::Link, title, CursorKind::Pointer, false, false);
    }
}

fn draw_section_heading(canvas: &mut Canvas<'_>, rect: Rect, title: &str, subtitle: &str) {
    let palette = *canvas.palette;
    let title_w = canvas.labels.measure(title, 15.0).x + 12.0;
    canvas.text(
        title,
        Rect::new(rect.origin.x, rect.origin.y, title_w.min(rect.size.x), 22.0),
        15.0,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
    if rect.size.x > title_w + 160.0 {
        canvas.text(
            subtitle,
            Rect::new(
                rect.origin.x + title_w + 4.0,
                rect.origin.y + 2.0,
                rect.size.x - title_w - 4.0,
                20.0,
            ),
            11.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
    }
}

fn draw_quick_access_header(canvas: &mut Canvas<'_>, state: &GalleryState, rect: Rect, zh: bool) {
    let palette = *canvas.palette;
    let heading = if state.home_quick_tab == 1 {
        if zh { "已收藏控件" } else { "Favorite Controls" }
    } else if state.recent_controls.is_empty() {
        if zh { "精选控件入口" } else { "Featured Controls" }
    } else if zh {
        "最近访问"
    } else {
        "Recently Visited"
    };
    canvas.text(
        heading,
        Rect::new(rect.origin.x, rect.origin.y, 240.0, 22.0),
        15.0,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );

    // Recent | Favorites 轻量切换胶囊
    let tabs = [
        (0u8, if zh { "最近" } else { "Recent" }),
        (1u8, if zh { "收藏" } else { "Favorites" }),
    ];
    let mut x = rect.max().x;
    for &(tab_idx, label) in tabs.iter().rev() {
        let w = canvas.labels.measure(label, 11.0).x + 22.0;
        x -= w;
        let pill = Rect::new(x, rect.origin.y, w, 24.0);
        let active = state.home_quick_tab == tab_idx;
        let node = target::id(Target::HomeQuickTab(tab_idx));
        let visual = canvas.visual(node, active);
        let fill = if active {
            palette.primary
        } else if visual.is_hovered() {
            palette.primary_soft
        } else {
            palette.card_bg
        };
        let fg = if active { palette.on_primary } else { palette.text };
        canvas.rect(pill, 12.0, fill, palette.card_border, if active { 0.0 } else { 1.0 }, 0.0);
        canvas.text(label, pill, 11.0, fg, AlignX::Center, AlignY::Center);
        canvas.control(node, pill, Role::Tab, label, CursorKind::Pointer, active, false);
        x -= 6.0;
    }
}

fn draw_landing_hero(canvas: &mut Canvas<'_>, state: &GalleryState, rect: Rect, zh: bool) {
    let palette = *canvas.palette;
    let compact = rect.size.x < 720.0;

    canvas.rect(rect, 16.0, palette.hero_start, 0, 0.0, 20.0);
    canvas.begin_clip_rounded(rect, 16.0);
    canvas.rect(
        Rect::new(
            rect.max().x - rect.size.x * 0.52,
            rect.origin.y - rect.size.y * 0.45,
            rect.size.x * 0.78,
            rect.size.y * 1.5,
        ),
        rect.size.y * 0.75,
        palette.hero_end,
        0,
        0.0,
        0.0,
    );
    canvas.rect(
        Rect::new(
            rect.max().x - rect.size.x * 0.3,
            rect.max().y - rect.size.y * 0.42,
            rect.size.x * 0.34,
            rect.size.y * 0.42,
        ),
        90.0,
        palette.secondary,
        0,
        0.0,
        0.0,
    );
    for index in 0..4 {
        let f = index as f32;
        canvas.curve(
            [rect.origin.x + rect.size.x * (0.56 + f * 0.03), rect.max().y + 8.0],
            [rect.origin.x + rect.size.x * 0.84, rect.origin.y + rect.size.y * 0.52 + f * 14.0],
            [rect.max().x + 14.0, rect.origin.y + rect.size.y * 0.85 - f * 12.0],
            2.0,
            palette.on_hero,
        );
    }
    canvas.end_clip();

    let pad_x = if compact { 20.0 } else { 28.0 };
    let pad_y = if compact { 16.0 } else { 22.0 };
    let left_w = if compact {
        rect.size.x - pad_x * 2.0
    } else {
        rect.size.x * 0.60
    };
    let left = Rect::new(rect.origin.x + pad_x, rect.origin.y + pad_y, left_w, rect.size.y - pad_y * 2.0);

    let badge_text = "UNBOUND UI · VULKAN 1.3";
    let badge_w = canvas.labels.measure(badge_text, 10.0).x + 18.0;
    widgets::badge(
        canvas,
        Rect::new(left.origin.x, left.origin.y, badge_w, 22.0),
        badge_text,
        palette.on_hero,
        palette.hero_end,
    );

    canvas.text(
        state.t("home.hero.title"),
        Rect::new(left.origin.x, left.origin.y + 28.0, left.size.x, 44.0),
        if compact { 30.0 } else { 36.0 },
        palette.on_hero,
        AlignX::Start,
        AlignY::Top,
    );
    canvas.text_wrapped(
        state.t("home.hero.subtitle"),
        Rect::new(left.origin.x, left.origin.y + 74.0, left.size.x, 42.0),
        12.5,
        palette.on_hero,
    );

    let cta_y = left.max().y - 34.0;
    let primary_label = if zh { "浏览全部 133 控件" } else { "Browse 133 Controls" };
    let primary_w = (canvas.labels.measure(primary_label, 12.0).x + 28.0).max(150.0);
    let primary = Rect::new(left.origin.x, cta_y, primary_w, 34.0);
    widgets::button(
        canvas,
        target::id(Target::NavPage(PageId::Components.index() as u8)),
        primary,
        primary_label,
        ButtonVariant::OnHero,
        true,
    );
    let secondary_label = if zh { "打开无限画布" } else { "Open Canvas Studio" };
    let secondary_w = (canvas.labels.measure(secondary_label, 12.0).x + 28.0).max(142.0);
    let secondary = Rect::new(primary.max().x + 10.0, cta_y, secondary_w, 34.0);
    widgets::button(
        canvas,
        target::id(Target::NavPage(PageId::Viewport.index() as u8)),
        secondary,
        secondary_label,
        ButtonVariant::OnHeroSecondary,
        true,
    );

    // 右侧实时引擎状态胶囊（仅宽屏展示，凸显 Unbound 独有工程气质）
    if !compact {
        let stat_w = (rect.size.x * 0.30).clamp(220.0, 290.0);
        let stat = Rect::new(
            rect.max().x - pad_x - stat_w,
            rect.origin.y + pad_y,
            stat_w,
            rect.size.y - pad_y * 2.0,
        );
        canvas.rect(stat, 12.0, palette.hero_scrim, palette.on_hero, 1.0, 0.0);
        canvas.text(
            if zh { "引擎与控件概览" } else { "Engine & Catalog Pulse" },
            Rect::new(stat.origin.x + 14.0, stat.origin.y + 12.0, stat.size.x - 28.0, 18.0),
            12.0,
            palette.on_hero,
            AlignX::Start,
            AlignY::Top,
        );
        let line1 = if zh {
            format!(
                "{} 控件 · {} 大分类",
                WidgetCatalogId::COUNT,
                ControlCategory::COUNT
            )
        } else {
            format!(
                "{} controls · {} categories",
                WidgetCatalogId::COUNT,
                ControlCategory::COUNT
            )
        };
        let line2 = format!(
            "WinUI 3 {} · Toolkit {} · Unbound {}",
            unbound_widgets::BASELINE_COUNT,
            unbound_widgets::TOOLKIT_COUNT,
            unbound_widgets::EXCLUSIVE_COUNT
        );
        canvas.text(
            &line1,
            Rect::new(stat.origin.x + 14.0, stat.origin.y + 38.0, stat.size.x - 28.0, 18.0),
            12.0,
            palette.on_hero,
            AlignX::Start,
            AlignY::Top,
        );
        canvas.text(
            &line2,
            Rect::new(stat.origin.x + 14.0, stat.origin.y + 60.0, stat.size.x - 28.0, 18.0),
            10.5,
            palette.on_hero,
            AlignX::Start,
            AlignY::Top,
        );
        let parity_btn = Rect::new(stat.origin.x + 14.0, stat.max().y - 42.0, stat.size.x - 28.0, 30.0);
        widgets::button(
            canvas,
            target::id(Target::NavPage(PageId::Parity.index() as u8)),
            parity_btn,
            if zh { "打开像素对照检视台 →" } else { "Open Parity Inspector →" },
            ButtonVariant::OnHeroSecondary,
            true,
        );
    }

    canvas.semantic(
        Role::Heading,
        state.t("app.title"),
        Rect::new(left.origin.x, left.origin.y + 28.0, left.size.x, 44.0),
    );
}

/// 按分类索引分配稳定的语义色板组合。
#[must_use]
pub const fn category_accent(palette: crate::gallery::Palette, index: usize) -> (u16, u16) {
    match index % 4 {
        0 => (palette.primary, palette.primary_soft),
        1 => (palette.success, palette.success_soft),
        2 => (palette.warning, palette.warning_soft),
        _ => (palette.secondary, palette.secondary_soft),
    }
}

/// 按来源层级返回徽章配色。
#[must_use]
pub const fn origin_badge_colors(palette: crate::gallery::Palette, class: ControlClass) -> (u16, u16) {
    match class {
        ControlClass::Baseline => (palette.primary, palette.primary_soft),
        ControlClass::Toolkit => (palette.success, palette.success_soft),
        ControlClass::Exclusive => (palette.secondary, palette.secondary_soft),
    }
}

/// 为 10 个功能分类返回直观的展台矢量图标。
#[must_use]
pub const fn category_widget_icon(cat: ControlCategory) -> Icon {
    match cat {
        ControlCategory::BasicInput => Icon::Sliders,
        ControlCategory::TextAndForm => Icon::Menu,
        ControlCategory::DateAndTime => Icon::Layers,
        ControlCategory::CollectionsAndData => Icon::Grid,
        ControlCategory::LayoutAndContainers => Icon::Grid,
        ControlCategory::Navigation => Icon::Layers,
        ControlCategory::MenusAndToolbars => Icon::Menu,
        ControlCategory::DialogsAndOverlays => Icon::Snap,
        ControlCategory::StatusMediaAndScrolling => Icon::Chart,
        ControlCategory::CanvasGraphAndDevTools => Icon::Snap,
    }
}

/// 兼容旧分派入口（当以 `DemoKind::Hero` 单独调用时）。
pub fn hero(ctx: &mut DemoCtx<'_, '_>) {
    draw_landing_hero(ctx.canvas, ctx.state, ctx.rect, ctx.state.locale.is_zh());
}

/// 兼容旧分派入口。
pub fn capability_grid(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let columns = if rect.size.x >= 620.0 { 2 } else { 1 };
    let gap = 12.0;
    let cell_w = (rect.size.x - gap * (columns - 1) as f32) / columns as f32;
    let cell_h = (rect.size.y - gap) * 0.5;
    for (index, (_, _, page, icon)) in CAPABILITIES.iter().enumerate() {
        let (title, subtitle) = localized_capability(index, ctx.state.locale);
        let col = index % columns;
        let row = index / columns;
        let cell = Rect::new(
            rect.origin.x + col as f32 * (cell_w + gap),
            rect.origin.y + row as f32 * (cell_h + gap),
            cell_w,
            cell_h,
        );
        let page_info = page.localized_info(ctx.state.locale);
        let label = format!("{title} — {}", page_info.title);
        widgets::feature_card(
            ctx.canvas,
            ctx.part(index as u16 + 1),
            cell,
            *icon,
            title,
            subtitle,
            &label,
            index,
        );
    }
}

/// 兼容旧分派入口。
pub fn control_index(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let cols = category_columns(rect.size.x);
    let cell_w = (rect.size.x - GRID_GAP * (cols - 1) as f32) / cols as f32;
    let palette = *ctx.canvas.palette;
    for (idx, cat) in ControlCategory::ALL.iter().copied().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let cell = Rect::new(
            rect.origin.x + col as f32 * (cell_w + GRID_GAP),
            rect.origin.y + row as f32 * (64.0 + GRID_GAP),
            cell_w,
            64.0,
        );
        if cell.max().y > rect.max().y {
            continue;
        }
        let title = category_localized_label(cat, ctx.state.locale);
        ctx.canvas.rect(cell, 8.0, palette.card_bg, palette.card_border, 1.0, 0.0);
        ctx.canvas.text(title, cell, 12.0, palette.text, AlignX::Center, AlignY::Center);
        ctx.control(idx as u16 + 1, cell, Role::Link, title, false);
    }
}
