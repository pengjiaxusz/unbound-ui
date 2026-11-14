//! 外壳的布局树。
//!
//! 外壳过去是手动摆放每个矩形。[`ShellLayout`] 携带了十四个手算矩形，
//! 页面通过累加卡片高度进行堆叠，因此窄卡片必须依靠硬编码的 `inner < 520.0` 断点
//! 才能把选项面板折叠到舞台下方。
//!
//!
//!
//! 如今两者都统一为由 [`LayoutEngine`] 求解的节点树。树*即*布局：
//! 卡片折叠是因为弹性行空间不足，而非某个断点规定如此；且卡片自身尺寸固定，
//! 使其成为重布局边界（relayout boundary）——页面可在不触及它的情况下重新求解。
//!

use unbound_abi::{Align, Dim, Edges, NodeFlags, Position, RawNodeDesc};
use unbound_layout::{LayoutEngine, LayoutStats};
use unbound_math::Rect;
use unbound_world::{NodeId, UiWorld};

use crate::pages::{ExampleDef, KnobSpec, PageId};
use crate::state::GalleryState;

/// 自定义标题栏的高度。
pub const TOP_BAR_HEIGHT: f32 = 56.0;

/// 宽内容列上的页面水平内边距。
const PAGE_PAD_WIDE: f32 = 36.0;
/// 窄内容列上的页面水平内边距。
const PAGE_PAD_COMPACT: f32 = 24.0;
/// 页面采用紧凑内边距的内容宽度阈值。
const PAGE_PAD_BREAKPOINT: f32 = 760.0;
/// 页面顶部内边距（位于页面标题上方）。
const PAGE_PAD_TOP: f32 = 26.0;
/// 页面底部内边距（位于最后一张卡片下方）。
const PAGE_PAD_BOTTOM: f32 = 26.0;
/// 页面标题与第一张卡片之间的间距。
const PAGE_HEAD_GAP: f32 = 6.0;
/// 两张卡片之间的间距。
const CARD_GAP: f32 = 18.0;
/// 卡片的水平内边距。
const CARD_PAD_X: f32 = 16.0;
/// 卡片的顶部内边距。
const CARD_PAD_TOP: f32 = 12.0;
/// 卡片的底部内边距。
const CARD_PAD_BOTTOM: f32 = 10.0;
/// 为卡片标题与描述预留的高度。
const CARD_HEAD: f32 = 64.0;
/// 舞台与选项面板之间的间距。
const STAGE_GAP: f32 = 6.0;
/// 舞台与选项面板并排放置时的间距。
///
const BODY_GAP: f32 = 18.0;
/// 舞台的弹性基准尺寸（flex basis）。
///
/// 连同选项面板的最小宽度及两者间距，共同决定了选项面板折叠到舞台下方的宽度阈值。
///
const STAGE_BASIS: f32 = 314.0;
/// 舞台的最小宽度。
const STAGE_MIN: f32 = 120.0;
/// 选项面板占卡片宽度的比例。
const KNOB_SHARE: f32 = 30.0;
/// 选项面板的最小宽度。
const KNOB_MIN: f32 = 188.0;
/// 选项面板的最大宽度。
const KNOB_MAX: f32 = 318.0;
/// 选项面板位于舞台旁时，单个旋钮行的弹性基准尺寸。
const KNOB_BASIS: f32 = 180.0;
/// 两个旋钮行之间的间距。
const KNOB_GAP: f32 = 12.0;
/// 折叠后选项面板的高度。
const KNOB_FOLDED: f32 = 96.0;
/// "OPTIONS" 小标题的高度。
const KNOB_CAPTION: f32 = 18.0;
/// 小标题与第一个旋钮行之间的垂直间距。
const KNOB_CAPTION_GAP: f32 = 16.0;
/// 选项面板的水平内边距。
const KNOB_PAD: f32 = 10.0;
/// 面板顶部到第一个旋钮行的距离。
const KNOB_ROW_TOP: f32 = 34.0;
/// 代码栏的高度。
const CODE_BAR: f32 = 30.0;
/// 卡片主体与代码栏之间的间距。
const CODE_GAP: f32 = 6.0;
/// 代码栏与代码面板之间的间距。
const CODE_PANEL_GAP: f32 = 4.0;
/// 单个窗口控制按钮的宽度。
///
/// 三个按钮紧贴右边缘：此处若留有缝隙，就会形成既无法关闭窗口也无法触发拖拽的死区。
///
const WINDOW_BUTTON_WIDTH: f32 = 46.0;

/// 单帧的响应式几何信息。
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

/// 选项面板中的一个旋钮行。
#[derive(Clone, Debug)]
pub struct KnobRow {
    pub index: usize,
    pub row: Rect,
    pub label: Option<Rect>,
    pub control: Rect,
}

/// 布局后的单个示例卡片。
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

/// 布局后的页面。
#[derive(Clone, Debug)]
pub struct PageLayout {
    pub page: PageId,
    pub header: Rect,
    pub cards: Vec<CardLayout>,
    pub content_height: f32,
}

/// 标题栏控件组。
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

/// 构成单个旋钮行的节点。
#[derive(Clone, Copy, Debug)]
struct KnobNodes {
    row: NodeId,
    label: Option<NodeId>,
    control: NodeId,
}

/// 构成单个卡片的节点。
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

/// 外壳的布局树。
///
/// 该树的生命周期与整个应用相同：外框（chrome）只构建一次，后续仅刷新几何信息，
/// 而页面子树则在每次用户导航时重建。
///
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
    /// 上次求解的统计信息。
    last_stats: LayoutStats,
}

impl Default for ShellTree {
    fn default() -> Self {
        Self::new()
    }
}

/// 插入一个节点，并将其归因于调用方的源码位置。
#[track_caller]
fn insert(world: &mut UiWorld, parent: NodeId, desc: RawNodeDesc) -> NodeId {
    world.insert(parent, desc, core::panic::Location::caller())
}

/// 沿垂直轴堆叠子节点的容器。
fn column() -> RawNodeDesc {
    RawNodeDesc::container().column()
}

/// 沿水平轴排列子节点的容器。
fn row() -> RawNodeDesc {
    RawNodeDesc::container().row()
}

/// 在父节点内以绝对偏移放置的节点。
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

/// 单个旋钮行的度量信息：行高、控件高度，以及该行是否自带标题。
///
fn knob_row_metrics(spec: &KnobSpec) -> (f32, f32, bool) {
    match spec {
        KnobSpec::Toggle { .. } | KnobSpec::CheckBox { .. } => (44.0, 30.0, false),
        KnobSpec::Slider { .. } => (58.0, 28.0, true),
        KnobSpec::Segmented { .. } => (60.0, 28.0, true),
    }
}

/// 卡片代码面板的高度。
fn code_panel_height(example: &ExampleDef) -> f32 {
    let lines = example.code.lines().count().min(12);
    12.0 + lines as f32 * 15.0 + 24.0
}

/// 标题栏的控件几何布局。
///
/// 标题栏是一条挂在右边缘的控件链，外加位于栏中央的搜索框。
///
fn chrome_rects(width: f32, compact: bool) -> [Rect; 9] {
    // 窗口按钮锚定在右边缘且始终优先：窄窗口绝不能把“关闭”挤出屏幕，
    // 且关闭按钮必须紧贴边缘，使屏幕角落成为有效点击目标。
    //
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

    // 其余控件在导航切换按钮与窗口按钮之间的空隙内从右向左排列。
    // 放不下的控件会被隐藏（宽度设为零）而不是与邻居重叠——
    // 过去在 320 px 下会导致布局出现负值。
    //
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
    // 四个缩放档位需要每个 42 px 的槽位来容纳 `"1.25×"` 及其内边距；
    // 低于该宽度时改用紧凑标签。
    let zoom = take(if width < 1000.0 { 168.0 } else { 190.0 });
    let theme = take(if compact { 34.0 } else { 118.0 });

    //
    // 窄窗口保留搜索*图标*而非完全丢弃搜索：在 150–200 % 缩放下
    // 逻辑视口太小，无法容纳输入框，此时浮层是唯一的入口。
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
    /// 创建仅含空页面的布局树。
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

        // 页面列会在每次用户导航时重建；该节点从一开始就存在，使树始终保持相同形状。
        //
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

    /// 树中的节点总数（诊断用）。
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.world.len()
    }

    /// 树中的重布局边界数量。
    ///
    /// 固定尺寸的卡片是一个边界：其内部任何变化都无法改变自身尺寸，
    /// 因此卡片内部的变更绝不会重新求解整个页面。
    #[must_use]
    pub fn boundary_count(&self) -> usize {
        self.world.relayout_boundary_count()
    }

    /// 作为重排边界的卡片数量。
    #[must_use]
    pub fn card_boundary_count(&self) -> usize {
        self.cards
            .iter()
            .filter(|nodes| self.world.node(nodes.card).is_relayout_boundary())
            .count()
    }

    /// 按当前视口和导航宽度求解布局树。
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
        // 160 px 是卡片旋钮行仍能将标签与控件分开的最小宽度。
        // 更大的下限过去曾使页面比窗格更宽，导致右边缘被裁剪而不是重排。
        //
        //
        let page_width = (content_width - pad * 2.0).max(160.0);

        {
            let root = &mut self.world.node_mut(self.root).desc;
            root.width = Dim::px(width);
            root.height = Dim::px(viewport.size.y);
        }
        {
            // 主体区域占据标题栏剩余的所有空间。固定其高度可防止导航窗格随页面一起拉长。
            //
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
        // 第一遍假设选项面板位于舞台旁。它实际是否并排由引擎决定，而非我们：
        // 若弹性行空间不足，第二遍会将面板折叠到下方。
        //
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

    /// 上次求解的统计信息。
    ///
    /// 当树完全未求解时 `nodes_visited` 为零——仅重绘的帧不触及布局。
    ///
    #[must_use]
    pub fn stats(&self) -> LayoutStats {
        self.last_stats
    }

    /// 页面内容在页面自身坐标系中的高度。
    #[must_use]
    pub fn content_height(&self) -> f32 {
        self.rect_of(self.page).size.y
    }

    /// 读取页面几何信息。
    ///
    /// `scroll` 会在内容视口内平移页面；布局本身不受其影响，因此滚动绝不会重新求解布局树。
    ///
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

    /// 节点在绝对窗口坐标中的矩形。
    ///
    /// Taffy 上报的节点位置是相对父节点的，因此树改为读取世界中的绝对矩形。
    ///
    fn rect_of(&self, id: NodeId) -> Rect {
        self.engine
            .world_rect(&self.world, id)
            .unwrap_or(Rect::ZERO)
    }

    /// 引擎是否将选项面板折叠到了舞台下方。
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

    /// 在用户导航后重建页面子树。
    fn ensure_page(&mut self, page: PageId) {
        if self.page_id == Some(page) {
            return;
        }
        self.page_id = Some(page);
        self.rebuild_page(page);
    }

    /// 丢弃页面现有卡片并为 `page` 重新构建。
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

            // 标题与描述由画布绘制；节点仅预留它们的高度。
            //
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
                        // 开关和复选框在控件内部自带标签，因此起点略微下移。
                        //
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

    /// 按当前状态刷新页面几何信息。
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
                // 折叠后的面板横跨整张卡片：用于将其保持在舞台旁的宽度限制反而会裁剪它。
                //
                desc.min_width = if folded { Dim::AUTO } else { Dim::px(KNOB_MIN) };
                desc.max_width = if folded { Dim::AUTO } else { Dim::px(KNOB_MAX) };
                desc.height = Dim::px(knob_height);

                // 折叠后的面板宽而矮：其行并排排列而非按列堆叠。
                // 基准尺寸即每行分得的面板内容宽度份额。
                //
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
