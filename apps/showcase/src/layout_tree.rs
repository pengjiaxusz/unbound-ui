//! The shell's layout tree.
//!
//! The shell used to place every rectangle by hand. [`ShellLayout`] carried
//! fourteen hand-computed rectangles and the page was stacked by adding up
//! card heights, so a narrow card had to be recognised by a hard-coded
//! `inner < 520.0` breakpoint before it could fold its options panel below
//! the stage.
//!
//! Both are now a node tree that [`LayoutEngine`] solves. The tree *is* the
//! layout: a card folds because the flex line runs out of room, not because a
//! breakpoint said so, and the card's own size is fixed, which makes it a
//! relayout boundary — the page can be re-solved without touching it.

use unbound_abi::{Align, Dim, Edges, NodeFlags, Position, RawNodeDesc};
use unbound_layout::{LayoutEngine, LayoutStats};
use unbound_math::Rect;
use unbound_world::{NodeId, UiWorld};

use crate::pages::{ExampleDef, KnobSpec, PageId};
use crate::state::GalleryState;

/// The height of the custom title bar.
pub const TOP_BAR_HEIGHT: f32 = 56.0;

/// The page's horizontal padding on a wide content column.
const PAGE_PAD_WIDE: f32 = 36.0;
/// The page's horizontal padding on a narrow content column.
const PAGE_PAD_COMPACT: f32 = 24.0;
/// The content width below which the page uses the compact padding.
const PAGE_PAD_BREAKPOINT: f32 = 760.0;
/// The page's top padding, above the page heading.
const PAGE_PAD_TOP: f32 = 26.0;
/// The page's bottom padding, below the last card.
const PAGE_PAD_BOTTOM: f32 = 26.0;
/// The gap between the page heading and the first card.
const PAGE_HEAD_GAP: f32 = 6.0;
/// The gap between two cards.
const CARD_GAP: f32 = 18.0;
/// The card's horizontal padding.
const CARD_PAD_X: f32 = 16.0;
/// The card's top padding.
const CARD_PAD_TOP: f32 = 12.0;
/// The card's bottom padding.
const CARD_PAD_BOTTOM: f32 = 10.0;
/// The height reserved for the card's title and description.
const CARD_HEAD: f32 = 64.0;
/// The gap between the stage and the options panel.
const STAGE_GAP: f32 = 6.0;
/// The gap between the stage and the options panel when they stand side by
/// side.
const BODY_GAP: f32 = 18.0;
/// The stage's flex basis.
///
/// Together with the options panel's minimum width and the gap between them
/// this is the width at which the options panel folds below the stage.
const STAGE_BASIS: f32 = 314.0;
/// The stage's minimum width.
const STAGE_MIN: f32 = 120.0;
/// The options panel's share of the card's width.
const KNOB_SHARE: f32 = 30.0;
/// The options panel's minimum width.
const KNOB_MIN: f32 = 188.0;
/// The options panel's maximum width.
const KNOB_MAX: f32 = 318.0;
/// A knob row's flex basis while the options panel stands beside the stage.
const KNOB_BASIS: f32 = 180.0;
/// The gap between two knob rows.
const KNOB_GAP: f32 = 12.0;
/// The height of a folded options panel.
const KNOB_FOLDED: f32 = 96.0;
/// The height of the "OPTIONS" caption.
const KNOB_CAPTION: f32 = 18.0;
/// The vertical gap between the caption and the first knob row.
const KNOB_CAPTION_GAP: f32 = 16.0;
/// The options panel's horizontal padding.
const KNOB_PAD: f32 = 10.0;
/// The distance from the panel's top to the first knob row.
const KNOB_ROW_TOP: f32 = 34.0;
/// The code bar's height.
const CODE_BAR: f32 = 30.0;
/// The gap between the card body and the code bar.
const CODE_GAP: f32 = 6.0;
/// The gap between the code bar and the code panel.
const CODE_PANEL_GAP: f32 = 4.0;
/// The width of one window control button.
///
/// The three buttons are flush with the right edge: a gap there would be a
/// dead strip where a click neither closes the window nor starts a drag.
const WINDOW_BUTTON_WIDTH: f32 = 46.0;

/// Responsive geometry for one frame.
#[derive(Copy, Clone, Debug)]
pub struct ShellLayout {
    pub viewport: Rect,
    pub top_bar: Rect,
    pub nav: Rect,
    pub content: Rect,
    pub nav_width: f32,
    pub nav_toggle: Rect,
    pub search: Rect,
    pub theme: Rect,
    pub zoom: Rect,
    pub snap: Rect,
    pub hud: Rect,
    pub window_minimize: Rect,
    pub window_maximize: Rect,
    pub window_close: Rect,
    pub compact: bool,
    pub wide: bool,
}

/// One knob row inside an options panel.
#[derive(Clone, Debug)]
pub struct KnobRow {
    pub index: usize,
    pub row: Rect,
    pub label: Option<Rect>,
    pub control: Rect,
}

/// One example card after layout.
#[derive(Clone, Debug)]
pub struct CardLayout {
    pub index: usize,
    pub rect: Rect,
    pub stage: Rect,
    pub knobs: Rect,
    pub knob_rows: Vec<KnobRow>,
    pub code_bar: Rect,
    pub code_panel: Option<Rect>,
}

/// A page after layout.
#[derive(Clone, Debug)]
pub struct PageLayout {
    pub page: PageId,
    pub header: Rect,
    pub cards: Vec<CardLayout>,
    pub content_height: f32,
}

/// The title bar's controls.
#[derive(Clone, Copy, Debug)]
struct ChromeNodes {
    nav_toggle: NodeId,
    search: NodeId,
    theme: NodeId,
    zoom: NodeId,
    snap: NodeId,
    hud: NodeId,
    minimize: NodeId,
    maximize: NodeId,
    close: NodeId,
}

/// The nodes that make up one knob row.
#[derive(Clone, Copy, Debug)]
struct KnobNodes {
    row: NodeId,
    label: Option<NodeId>,
    control: NodeId,
}

/// The nodes that make up one card.
#[derive(Clone, Debug)]
struct CardNodes {
    card: NodeId,
    body: NodeId,
    stage: NodeId,
    knobs: Option<NodeId>,
    knob_list: Option<NodeId>,
    rows: Vec<KnobNodes>,
    code_bar: NodeId,
    code_panel: Option<NodeId>,
}

/// The shell's layout tree.
///
/// The tree lives for as long as the application: the chrome is built once
/// and only its geometry is refreshed, while the page subtree is rebuilt
/// whenever the user navigates.
pub struct ShellTree {
    world: UiWorld,
    engine: LayoutEngine,
    root: NodeId,
    top_bar: NodeId,
    body: NodeId,
    nav: NodeId,
    content: NodeId,
    page: NodeId,
    header: NodeId,
    cards_list: NodeId,
    chrome: ChromeNodes,
    cards: Vec<CardNodes>,
    page_id: Option<PageId>,
    /// The statistics of the last solve.
    last_stats: LayoutStats,
}

impl Default for ShellTree {
    fn default() -> Self {
        Self::new()
    }
}

/// Insert a node, attributing it to the caller's source location.
#[track_caller]
fn insert(world: &mut UiWorld, parent: NodeId, desc: RawNodeDesc) -> NodeId {
    world.insert(parent, desc, core::panic::Location::caller())
}

/// A container that stacks its children along the vertical axis.
fn column() -> RawNodeDesc {
    RawNodeDesc::container().column()
}

/// A container that lays its children out along the horizontal axis.
fn row() -> RawNodeDesc {
    RawNodeDesc::container().row()
}

/// A node that is placed at an absolute offset inside its parent.
fn absolute(x: f32, y: f32, width: f32, height: f32) -> RawNodeDesc {
    let mut desc = RawNodeDesc::container();
    desc.position = Position::Absolute as u8;
    desc.width = Dim::px(width);
    desc.height = Dim::px(height);
    desc.inset = Edges {
        left: x,
        top: y,
        right: 0.0,
        bottom: 0.0,
    };
    desc
}

/// The metrics of one knob row: its height, its control's height and whether
/// the row carries a caption of its own.
fn knob_row_metrics(spec: &KnobSpec) -> (f32, f32, bool) {
    match spec {
        KnobSpec::Toggle { .. } | KnobSpec::CheckBox { .. } => (44.0, 30.0, false),
        KnobSpec::Slider { .. } => (58.0, 28.0, true),
        KnobSpec::Segmented { .. } => (60.0, 28.0, true),
    }
}

/// The height of a card's code panel.
fn code_panel_height(example: &ExampleDef) -> f32 {
    let lines = example.code.lines().count().min(12);
    12.0 + lines as f32 * 15.0 + 24.0
}

/// The title bar's control geometry.
///
/// The bar is a chain of controls that hangs off the right edge, plus a
/// search field that sits in the middle of the bar.
fn chrome_rects(width: f32, compact: bool) -> [Rect; 9] {
    // The window buttons are anchored to the right edge and always win: a
    // narrow window must never push "close" off-screen, and the close button
    // must reach the edge so the corner is a valid target.
    let close = Rect::new(
        (width - WINDOW_BUTTON_WIDTH).max(0.0),
        0.0,
        WINDOW_BUTTON_WIDTH,
        TOP_BAR_HEIGHT,
    );
    let maximize = Rect::new(
        close.origin.x - WINDOW_BUTTON_WIDTH,
        0.0,
        WINDOW_BUTTON_WIDTH,
        TOP_BAR_HEIGHT,
    );
    let minimize = Rect::new(
        maximize.origin.x - WINDOW_BUTTON_WIDTH,
        0.0,
        WINDOW_BUTTON_WIDTH,
        TOP_BAR_HEIGHT,
    );
    let nav_toggle = Rect::new(12.0, 12.0, 32.0, 32.0);

    // Everything else is laid out right-to-left inside the gap between the
    // navigation toggle and the window buttons. A control that no longer fits
    // is hidden (zero width) instead of overlapping its neighbour, which used
    // to make the layout negative at 320 px.
    let left_limit = nav_toggle.origin.x + nav_toggle.size.x + 8.0;
    let right_limit = (minimize.origin.x - 10.0).max(left_limit);
    let mut cursor = right_limit;
    let mut take = |w: f32| -> Rect {
        if cursor - w >= left_limit {
            let placed = Rect::new(cursor - w, 12.0, w, 32.0);
            cursor = placed.origin.x - 8.0;
            placed
        } else {
            let hidden = Rect::new(cursor.max(left_limit), 12.0, 0.0, 32.0);
            cursor = left_limit;
            hidden
        }
    };

    let hud = take(32.0);
    let snap = take(32.0);
    // Four zoom steps need a 42 px slot each for `"1.25×"` plus its padding;
    // below that the compact labels are used instead.
    let zoom = take(if width < 1000.0 { 168.0 } else { 190.0 });
    let theme = take(if compact { 34.0 } else { 118.0 });

    // A narrow window keeps a search *icon* instead of dropping search
    // entirely: the overlay is the only entry point at 150–200 % zoom, where
    // the logical viewport is too small for the field.
    let search_width = if width < 900.0 {
        32.0
    } else if width < 1000.0 {
        170.0
    } else {
        (width * 0.24).clamp(220.0, 380.0)
    };
    let search_right = if theme.size.x > 0.0 {
        theme.origin.x - 20.0
    } else {
        cursor - 12.0
    };
    let search_x = (width * 0.5 - search_width * 0.5)
        .max(left_limit)
        .min((search_right - search_width).max(left_limit));
    let search = if search_width > 0.0 && search_right - search_width >= left_limit {
        Rect::new(search_x, 12.0, search_width, 32.0)
    } else {
        Rect::new(0.0, 12.0, 0.0, 32.0)
    };
    [
        nav_toggle, search, theme, zoom, snap, hud, minimize, maximize, close,
    ]
}

impl ShellTree {
    /// Create the tree with an empty page.
    #[must_use]
    pub fn new() -> Self {
        let mut world = UiWorld::new();
        let root = world.root();

        let mut top_bar_desc = row();
        top_bar_desc.height = Dim::px(TOP_BAR_HEIGHT);
        let top_bar = insert(&mut world, root, top_bar_desc);

        let mut body_desc = row();
        body_desc.height = Dim::px(0.0);
        let body = insert(&mut world, root, body_desc);

        let mut nav_desc = column();
        nav_desc.flex_shrink = 0.0;
        let nav = insert(&mut world, body, nav_desc);

        let mut content_desc = column();
        content_desc.flex_shrink = 0.0;
        content_desc = content_desc.flags(NodeFlags::CLIP | NodeFlags::SCROLL_Y);
        let content = insert(&mut world, body, content_desc);

        // The page column is rebuilt whenever the user navigates; the node
        // exists from the start so the tree always has the same shape.
        let mut page_desc = column();
        page_desc.padding = Edges {
            left: 0.0,
            top: PAGE_PAD_TOP,
            right: 0.0,
            bottom: PAGE_PAD_BOTTOM,
        };
        let page = insert(&mut world, content, page_desc);

        let mut header_desc = column();
        header_desc.width = Dim::px(0.0);
        header_desc.height = Dim::px(140.0);
        let header = insert(&mut world, page, header_desc);

        let mut list_desc = column();
        list_desc.width = Dim::percent(100.0);
        list_desc.gap_row = CARD_GAP;
        list_desc.margin = Edges {
            left: 0.0,
            top: PAGE_HEAD_GAP,
            right: 0.0,
            bottom: 0.0,
        };
        let cards_list = insert(&mut world, page, list_desc);

        let chrome = ChromeNodes {
            nav_toggle: insert(&mut world, top_bar, absolute(0.0, 0.0, 0.0, 0.0)),
            search: insert(&mut world, top_bar, absolute(0.0, 0.0, 0.0, 0.0)),
            theme: insert(&mut world, top_bar, absolute(0.0, 0.0, 0.0, 0.0)),
            zoom: insert(&mut world, top_bar, absolute(0.0, 0.0, 0.0, 0.0)),
            snap: insert(&mut world, top_bar, absolute(0.0, 0.0, 0.0, 0.0)),
            hud: insert(&mut world, top_bar, absolute(0.0, 0.0, 0.0, 0.0)),
            minimize: insert(&mut world, top_bar, absolute(0.0, 0.0, 0.0, 0.0)),
            maximize: insert(&mut world, top_bar, absolute(0.0, 0.0, 0.0, 0.0)),
            close: insert(&mut world, top_bar, absolute(0.0, 0.0, 0.0, 0.0)),
        };

        Self {
            world,
            engine: LayoutEngine::new(),
            root,
            top_bar,
            body,
            nav,
            content,
            page,
            header,
            cards_list,
            chrome,
            cards: Vec::new(),
            page_id: None,
            last_stats: LayoutStats::default(),
        }
    }

    /// The number of nodes in the tree (diagnostics).
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.world.len()
    }

    /// The number of relayout boundaries in the tree.
    ///
    /// A fixed-size card is a boundary: nothing inside it can change its
    /// size, so a change inside a card never re-solves the page.
    #[must_use]
    pub fn boundary_count(&self) -> usize {
        self.world.relayout_boundary_count()
    }

    /// The number of cards that are relayout boundaries.
    #[must_use]
    pub fn card_boundary_count(&self) -> usize {
        self.cards
            .iter()
            .filter(|nodes| self.world.node(nodes.card).is_relayout_boundary())
            .count()
    }

    /// Solve the tree for the current viewport and navigation width.
    pub fn solve(&mut self, state: &GalleryState, viewport: Rect, nav_width: f32) -> ShellLayout {
        let width = viewport.size.x;
        let compact = width < 1080.0;
        let wide = width >= 1240.0;

        let content_width = (width - nav_width).max(0.0);
        let pad = if content_width < PAGE_PAD_BREAKPOINT {
            PAGE_PAD_COMPACT
        } else {
            PAGE_PAD_WIDE
        };
        // 160 px is the narrowest width at which a card's knob row still
        // separates its label from its control. A larger floor used to make
        // the page wider than the pane, which clipped the right edge instead
        // of reflowing it.
        let page_width = (content_width - pad * 2.0).max(160.0);

        {
            let root = &mut self.world.node_mut(self.root).desc;
            root.width = Dim::px(width);
            root.height = Dim::px(viewport.size.y);
        }
        {
            // The body takes whatever the title bar left over. Fixing its
            // height keeps the navigation pane from growing with the page.
            let body = &mut self.world.node_mut(self.body).desc;
            body.height = Dim::px((viewport.size.y - TOP_BAR_HEIGHT).max(0.0));
        }
        self.world.node_mut(self.nav).desc.width = Dim::px(nav_width);
        self.world.node_mut(self.content).desc.width = Dim::px(content_width);
        {
            let page = &mut self.world.node_mut(self.page).desc;
            page.margin.left = pad;
            page.width = Dim::px(page_width);
        }
        self.world.node_mut(self.header).desc.width = Dim::px(page_width);

        let rects = chrome_rects(width, compact);
        let ids = [
            self.chrome.nav_toggle,
            self.chrome.search,
            self.chrome.theme,
            self.chrome.zoom,
            self.chrome.snap,
            self.chrome.hud,
            self.chrome.minimize,
            self.chrome.maximize,
            self.chrome.close,
        ];
        for (id, rect) in ids.into_iter().zip(rects) {
            let desc = &mut self.world.node_mut(id).desc;
            desc.position = Position::Absolute as u8;
            desc.width = Dim::px(rect.size.x);
            desc.height = Dim::px(rect.size.y);
            desc.inset = Edges {
                left: rect.origin.x,
                top: rect.origin.y,
                right: 0.0,
                bottom: 0.0,
            };
        }

        self.ensure_page(state.active_page);
        // The first pass assumes the options panel stands beside the stage.
        // Whether it actually does is the engine's answer, not ours: the
        // second pass folds the panel if the flex line ran out of room.
        self.update_page(state, page_width, false);
        self.last_stats = self.engine.solve(&mut self.world, self.root, viewport.size);
        if self.options_folded() {
            self.update_page(state, page_width, true);
            self.last_stats = self.engine.solve(&mut self.world, self.root, viewport.size);
        }

        let rect = |id: NodeId| self.rect_of(id);
        ShellLayout {
            viewport,
            top_bar: rect(self.top_bar),
            nav: rect(self.nav),
            content: rect(self.content),
            nav_width,
            nav_toggle: rect(self.chrome.nav_toggle),
            search: rect(self.chrome.search),
            theme: rect(self.chrome.theme),
            zoom: rect(self.chrome.zoom),
            snap: rect(self.chrome.snap),
            hud: rect(self.chrome.hud),
            window_minimize: rect(self.chrome.minimize),
            window_maximize: rect(self.chrome.maximize),
            window_close: rect(self.chrome.close),
            compact,
            wide,
        }
    }

    /// The statistics of the last solve.
    ///
    /// `nodes_visited` is zero when the tree was not solved at all — a
    /// paint-only frame does not touch the layout.
    #[must_use]
    pub fn stats(&self) -> LayoutStats {
        self.last_stats
    }

    /// The height of the page's content, in the page's own coordinate space.
    #[must_use]
    pub fn content_height(&self) -> f32 {
        self.rect_of(self.page).size.y
    }

    /// Read the page's geometry.
    ///
    /// `scroll` shifts the page inside the content viewport; the layout
    /// itself is untouched by it, so a scroll never re-solves the tree.
    #[must_use]
    pub fn page_layout(&self, state: &GalleryState, scroll: f32) -> PageLayout {
        let shift = |rect: Rect| {
            Rect::new(
                rect.origin.x,
                rect.origin.y - scroll,
                rect.size.x,
                rect.size.y,
            )
        };
        let rect = |id: NodeId| shift(self.rect_of(id));
        let mut cards = Vec::with_capacity(self.cards.len());
        for (index, nodes) in self.cards.iter().enumerate() {
            let mut knob_rows = Vec::with_capacity(nodes.rows.len());
            for (row_index, row) in nodes.rows.iter().enumerate() {
                knob_rows.push(KnobRow {
                    index: row_index,
                    row: rect(row.row),
                    label: row.label.map(&rect),
                    control: rect(row.control),
                });
            }
            cards.push(CardLayout {
                index,
                rect: rect(nodes.card),
                stage: rect(nodes.stage),
                knobs: nodes.knobs.map_or_else(Rect::default, &rect),
                knob_rows,
                code_bar: rect(nodes.code_bar),
                code_panel: nodes.code_panel.map(&rect),
            });
        }
        PageLayout {
            page: state.active_page,
            header: rect(self.header),
            cards,
            content_height: self.content_height(),
        }
    }

    /// The node's rectangle in absolute window coordinates.
    ///
    /// Taffy reports a node's position relative to its parent, so the tree
    /// reads the world's absolute rectangle instead.
    fn rect_of(&self, id: NodeId) -> Rect {
        self.engine
            .world_rect(&self.world, id)
            .unwrap_or(Rect::ZERO)
    }

    /// Whether the engine folded an options panel below its stage.
    fn options_folded(&self) -> bool {
        self.cards.iter().any(|nodes| {
            let Some(knobs) = nodes.knobs else {
                return false;
            };
            let stage = self.rect_of(nodes.stage);
            let panel = self.rect_of(knobs);
            panel.origin.y > stage.origin.y + 0.5
        })
    }

    /// Rebuild the page subtree when the user navigated.
    fn ensure_page(&mut self, page: PageId) {
        if self.page_id == Some(page) {
            return;
        }
        self.page_id = Some(page);
        self.rebuild_page(page);
    }

    /// Drop the page's cards and build them again for `page`.
    fn rebuild_page(&mut self, page: PageId) {
        for nodes in &self.cards {
            self.world.remove(nodes.card);
        }
        self.cards.clear();

        for example in page.examples() {
            let mut card_desc = column();
            card_desc.width = Dim::px(0.0);
            card_desc.height = Dim::px(0.0);
            card_desc.padding = Edges {
                left: CARD_PAD_X,
                top: CARD_PAD_TOP,
                right: CARD_PAD_X,
                bottom: CARD_PAD_BOTTOM,
            };
            card_desc = card_desc.flags(NodeFlags::CLIP);
            let card = insert(&mut self.world, self.cards_list, card_desc);

            // The title and the description are painted by the canvas; the
            // node only reserves their height.
            let mut head_desc = column();
            head_desc.width = Dim::percent(100.0);
            head_desc.height = Dim::px(CARD_HEAD);
            insert(&mut self.world, card, head_desc);

            let mut body_desc = row();
            body_desc.width = Dim::percent(100.0);
            body_desc.height = Dim::px(example.stage_height);
            body_desc.gap_column = BODY_GAP;
            body_desc.gap_row = STAGE_GAP;
            body_desc = body_desc.align(Align::FlexStart);
            body_desc = body_desc.flags(NodeFlags::WRAP);
            let body = insert(&mut self.world, card, body_desc);

            let mut stage_desc = column();
            stage_desc.height = Dim::px(example.stage_height);
            stage_desc.flex_grow = 1.0;
            stage_desc.flex_basis = Dim::px(STAGE_BASIS);
            stage_desc.min_width = Dim::px(STAGE_MIN);
            let stage = insert(&mut self.world, body, stage_desc);

            let mut knobs = None;
            let mut knob_list = None;
            let mut rows = Vec::new();
            if !example.knobs.is_empty() {
                let mut panel_desc = column();
                panel_desc.width = Dim::percent(KNOB_SHARE);
                panel_desc.min_width = Dim::px(KNOB_MIN);
                panel_desc.max_width = Dim::px(KNOB_MAX);
                panel_desc.height = Dim::px(example.stage_height);
                let panel = insert(&mut self.world, body, panel_desc);
                knobs = Some(panel);

                let mut caption_desc = column();
                caption_desc.width = Dim::percent(100.0);
                caption_desc.height = Dim::px(KNOB_CAPTION);
                insert(&mut self.world, panel, caption_desc);

                let mut list_desc = row();
                list_desc.width = Dim::percent(100.0);
                list_desc.height = Dim::px((example.stage_height - KNOB_ROW_TOP).max(0.0));
                list_desc.gap_column = KNOB_GAP;
                list_desc.gap_row = KNOB_GAP;
                list_desc = list_desc.align(Align::FlexStart);
                list_desc.margin = Edges {
                    left: KNOB_PAD,
                    top: KNOB_CAPTION_GAP,
                    right: KNOB_PAD,
                    bottom: 0.0,
                };
                list_desc = list_desc.flags(NodeFlags::WRAP);
                let list = insert(&mut self.world, panel, list_desc);
                knob_list = Some(list);

                for spec in example.knobs {
                    let (row_height, control_height, captioned) = knob_row_metrics(spec);
                    let mut row_desc = column();
                    row_desc.height = Dim::px(row_height);
                    row_desc.flex_grow = 1.0;
                    row_desc.flex_basis = Dim::px(KNOB_BASIS);
                    let row = insert(&mut self.world, list, row_desc);

                    let label = if captioned {
                        let mut label_desc = column();
                        label_desc.width = Dim::percent(100.0);
                        label_desc.height = Dim::px(16.0);
                        label_desc.margin = Edges {
                            left: 0.0,
                            top: 0.0,
                            right: 0.0,
                            bottom: 4.0,
                        };
                        Some(insert(&mut self.world, row, label_desc))
                    } else {
                        None
                    };

                    let mut control_desc = column();
                    control_desc.width = Dim::percent(100.0);
                    control_desc.height = Dim::px(control_height);
                    if !captioned {
                        // A toggle and a checkbox carry their own label
                        // inside the control, so they start a little lower.
                        control_desc.margin = Edges {
                            left: 0.0,
                            top: 6.0,
                            right: 0.0,
                            bottom: 0.0,
                        };
                    }
                    let control = insert(&mut self.world, row, control_desc);
                    rows.push(KnobNodes {
                        row,
                        label,
                        control,
                    });
                }
            }

            let mut code_desc = column();
            code_desc.width = Dim::percent(100.0);
            code_desc.height = Dim::px(CODE_BAR);
            code_desc.margin = Edges {
                left: 0.0,
                top: CODE_GAP,
                right: 0.0,
                bottom: 0.0,
            };
            let code_bar = insert(&mut self.world, card, code_desc);

            self.cards.push(CardNodes {
                card,
                body,
                stage,
                knobs,
                knob_list,
                rows,
                code_bar,
                code_panel: None,
            });
        }
    }

    /// Refresh the page's geometry for the current state.
    fn update_page(&mut self, state: &GalleryState, page_width: f32, folded: bool) {
        let page = state.active_page;
        let inner = (page_width - CARD_PAD_X * 2.0).max(0.0);
        let mut open_panels: Vec<(usize, NodeId)> = Vec::new();
        let mut closed_panels: Vec<NodeId> = Vec::new();

        for (index, nodes) in self.cards.iter().enumerate() {
            let Some(example) = page.examples().get(index) else {
                continue;
            };
            let open = state.code_open(page, index as u8);
            let panel_height = if open {
                code_panel_height(example)
            } else {
                0.0
            };
            let folded = folded && nodes.knobs.is_some();
            let body_height =
                example.stage_height + if folded { STAGE_GAP + KNOB_FOLDED } else { 0.0 };
            let card_height = CARD_PAD_TOP
                + CARD_HEAD
                + body_height
                + CODE_GAP
                + CODE_BAR
                + if open {
                    CODE_PANEL_GAP + panel_height
                } else {
                    0.0
                }
                + CARD_PAD_BOTTOM;

            {
                let desc = &mut self.world.node_mut(nodes.card).desc;
                desc.width = Dim::px(page_width);
                desc.height = Dim::px(card_height);
            }
            {
                let desc = &mut self.world.node_mut(nodes.body).desc;
                desc.height = Dim::px(body_height);
            }
            if let Some(panel) = nodes.knobs {
                let knob_height = if folded {
                    KNOB_FOLDED
                } else {
                    example.stage_height
                };
                let desc = &mut self.world.node_mut(panel).desc;
                desc.width = if folded {
                    Dim::percent(100.0)
                } else {
                    Dim::percent(KNOB_SHARE)
                };
                // A folded panel spans the whole card: the width limits that
                // keep the panel next to the stage would clip it instead.
                desc.min_width = if folded { Dim::AUTO } else { Dim::px(KNOB_MIN) };
                desc.max_width = if folded { Dim::AUTO } else { Dim::px(KNOB_MAX) };
                desc.height = Dim::px(knob_height);

                // A folded panel is wide and short: its rows sit side by side
                // instead of in a column. The basis is the share each row
                // gets of the panel's content width.
                let count = nodes.rows.len().max(1) as f32;
                let basis = if folded {
                    ((inner - KNOB_PAD * 2.0 - KNOB_GAP * (count - 1.0)) / count).max(80.0)
                } else {
                    KNOB_BASIS
                };
                if let Some(list) = nodes.knob_list {
                    let desc = &mut self.world.node_mut(list).desc;
                    desc.height = Dim::px((knob_height - KNOB_ROW_TOP).max(0.0));
                    for row in &nodes.rows {
                        let desc = &mut self.world.node_mut(row.row).desc;
                        desc.flex_basis = Dim::px(basis);
                    }
                }
            }

            match (open, nodes.code_panel) {
                (true, None) => {
                    let mut desc = column();
                    desc.width = Dim::percent(100.0);
                    desc.height = Dim::px(panel_height);
                    desc.margin = Edges {
                        left: 0.0,
                        top: CODE_PANEL_GAP,
                        right: 0.0,
                        bottom: 0.0,
                    };
                    let panel = insert(&mut self.world, nodes.card, desc);
                    open_panels.push((index, panel));
                }
                (false, Some(panel)) => closed_panels.push(panel),
                (true, Some(panel)) => {
                    self.world.node_mut(panel).desc.height = Dim::px(panel_height);
                }
                (false, None) => {}
            }
        }

        for panel in closed_panels {
            self.world.remove(panel);
        }
        for (index, panel) in open_panels {
            self.cards[index].code_panel = Some(panel);
        }
    }
}
