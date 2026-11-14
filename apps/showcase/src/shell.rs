//! 自举（dogfooded）的应用外壳。
//!
//! `GalleryApp` 拥有导航、环境开关与演示状态。它绘制自定义标题栏、
//! 可折叠的 NavigationView、页面视口与 `ExampleCard` 组合，使 showcase
//! 演练真实应用会使用的同一套图元。
//!

use accesskit::Role;
use unbound_anim::{CubicBezier, Spring, SpringPreset};
use unbound_headless::{Frame, FrameBuilder, NodeKind};
use unbound_input::CursorKind;
use unbound_layout::LayoutStats;
use unbound_math::{Rect, Vec2};
use unbound_platform::{Modifiers, PlatformKey};
use unbound_widgets::curve_editor::{CurveEditorDesc, CurveEditorState, CurveHandle};
use unbound_widgets::diff::{diff_primitives, SceneDiff};

use crate::canvas::{AlignX, AlignY, Canvas, RangeZone};
use crate::gallery::{self, Labels, Palette};
use crate::layout_tree::{CardLayout, KnobRow, PageLayout, ShellLayout, ShellTree};
use crate::pages::{self, CategoryId, DemoCtx, KnobSpec, PageId};
use crate::state::{GalleryState, ThemeMode, ZOOM_STEPS};
use crate::target::{self, Target, ROOT};
use crate::widgets::{self, Icon};

const NAV_EXPANDED: f32 = 248.0;
const NAV_COLLAPSED: f32 = 68.0;

/// 自定义标题栏请求的窗口动作。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WindowCommand {
    /// 最小化窗口。
    Minimize,
    /// 切换最大化状态。
    ToggleMaximize,
    /// 关闭窗口。
    Close,
}

/// 页面垂直滚动条的几何信息。
///
/// 滑块是真实控件：可拖拽，且点击轨道可跳转。仅作为绘制矩形存在的滚动条只是装饰，
/// 而非控件。
///
#[derive(Copy, Clone, Debug)]
struct Scrollbar {
    track: Rect,
    thumb_h: f32,
    max_scroll: f32,
}

impl Scrollbar {
    /// 指定滚动位置对应的滑块顶部边缘。
    fn thumb_top(&self, scroll: f32) -> f32 {
        let t = (scroll / self.max_scroll).clamp(0.0, 1.0);
        self.track.origin.y + (self.track.size.y - self.thumb_h) * t
    }

    /// 滑块顶部边缘对应的滚动位置。
    fn scroll_for(&self, thumb_top: f32) -> f32 {
        let span = (self.track.size.y - self.thumb_h).max(1.0);
        let t = ((thumb_top - self.track.origin.y) / span).clamp(0.0, 1.0);
        t * self.max_scroll
    }
}

/// 页面滚动条；当页面无需滚动时为 `None`。
fn content_scrollbar(layout: &ShellLayout, page: &PageLayout) -> Option<Scrollbar> {
    let max_scroll = (page.content_height - layout.content.size.y + 48.0).max(0.0);
    if max_scroll <= 1.0 {
        return None;
    }
    let track = Rect::new(
        layout.content.max().x - 10.0,
        layout.content.origin.y + 8.0,
        6.0,
        (layout.content.size.y - 16.0).max(1.0),
    );
    let thumb_h = (track.size.y * layout.content.size.y / page.content_height.max(1.0))
        .clamp(28.0, track.size.y);
    Some(Scrollbar {
        track,
        thumb_h,
        max_scroll,
    })
}

/// 导航窗格滚动条；当树无需滚动时为 `None`。
fn nav_scrollbar(layout: &ShellLayout) -> Option<Scrollbar> {
    let tree = nav_tree_rect(layout);
    let max_scroll = (nav_height(layout) - tree.size.y).max(0.0);
    if max_scroll <= 1.0 || tree.size.y <= 1.0 {
        return None;
    }
    let track = Rect::new(layout.nav.max().x - 10.0, tree.origin.y, 6.0, tree.size.y);
    let thumb_h =
        (track.size.y * tree.size.y / (tree.size.y + max_scroll)).clamp(28.0, track.size.y);
    Some(Scrollbar {
        track,
        thumb_h,
        max_scroll,
    })
}

/// 应用程序。
pub struct GalleryApp {
    pub state: GalleryState,
    pub scene: gallery::Gallery,
    pub labels: Labels,
    pub layout: ShellLayout,
    /// 上次重建产生的焦点顺序。
    ///
    /// 运行时在 `view` 之后立即将其纳入 `FocusManager`，因此它是输出，
    /// 绝非第二套事实来源。
    pub focus_order: Vec<u64>,
    /// 上次重建产生的命中区域。
    ///
    /// 运行时在 `view` 之后立即将其纳入 `Router`；外壳不再保留自己的路由器。
    ///
    pub regions: Vec<unbound_input::HitRegion>,
    pub ranges: Vec<RangeZone>,
    pub scroll_zones: Vec<(Rect, u8)>,
    pub semantics: unbound_a11y::Semantics,
    /// 外壳几何信息所源自的布局树。
    ///
    /// 该树是外壳矩形的单一事实来源：标题栏、导航窗格与页面均由 `LayoutEngine` 求解，
    /// 而非在本文件中手算。
    ///
    tree: ShellTree,
    page_layout: Option<PageLayout>,
    nav_spring: Spring,
    /// 当前场景的上传代次。
    ///
    /// 每次重建且*仅*在重建时递增代次：当世代未推进时 GPU 跳过上传，
    /// 因此忘记递增的宿主会显示过期帧。将计数器绑定到重建本身使该错误不可能发生。
    ///
    ///
    generation: u64,
    /// 字形图集执行驱逐所依据的时钟。
    ///
    /// 图集丢弃最近最少*使用*的字形，“使用”按帧数度量。因此每次重建都必须推进时钟——
    /// 否则所有槽位看起来一样旧，图集将永远驱逐相同的字形。它刻意与上传代次分离：
    /// 代次仅在场景*形状*变化时推进，且在外部 diff 模式下由宿主持有。
    ///
    ///
    ///
    atlas_clock: u64,
    last_revision: u64,
    last_hover: Option<u64>,
    last_pressed: Option<u64>,
    last_focus: Option<u64>,
    last_viewport: (f32, f32),
    nav_max_scroll: f32,
    pointer: Vec2,
    panning: Option<Vec2>,
    dragging_range: Option<u64>,
    effective_scale: f32,
    pending_window: Option<WindowCommand>,
    /// 为标题栏图标镜像原生窗口的最大化状态。
    pub window_maximized: bool,
    /// 当页面高于视口时显示的页面滚动条。
    scrollbar: Option<Scrollbar>,
    /// 拖拽过程中滑块内部的抓取偏移量。
    scroll_drag: Option<f32>,
    /// 当导航树高于窗格时显示的导航窗格滚动条。
    nav_scrollbar: Option<Scrollbar>,
    /// 拖拽过程中导航滚动条滑块内部的抓取偏移量。
    nav_scroll_drag: Option<f32>,
    /// 虚拟列表滚动条拖拽状态：`(card, grab offset)`。
    list_scroll_drag: Option<(usize, f32)>,
    /// 上次重建中发生变化的图元范围。
    ///
    /// 空列表表示“整个场景已变化”，渲染器需全量上传；非空列表表示“仅这些切片变化”，
    /// 这正是将悬停交互转为寥寥数个字节上传的关键。
    ///
    dirty_ranges: Vec<(u32, u32)>,
    /// 布局所依赖的输入。
    ///
    /// 悬停、按压或焦点变化不会移动任何元素，因此不会为其重新求解布局：
    /// 该帧的布局开销保持为零。
    layout_key: LayoutKey,
    /// 宿主运行时是否拥有帧 diff。
    ///
    /// showcase 由 `unbound_app::AppRuntime` 驱动，后者每帧对图元列表做一次 diff。
    /// 此处再做一次 diff 会为同一结果扫描整个场景两次。
    ///
    external_diff: bool,
    /// 当前帧的布局统计信息。
    frame_layout_stats: LayoutStats,
    /// 每个控件拥有的图元范围。
    ///
    /// 这些范围将脏切片归因回拥有它的控件：悬停修补必须停留在单个控件的跨度内。
    ///
    node_spans: rustc_hash::FxHashMap<u64, (u32, u32)>,
    /// 本帧实际运行的真实控件发射器，用于覆盖率门禁。
    emitted_widgets: rustc_hash::FxHashSet<unbound_widgets::WidgetCatalogId>,
    /// 输入法候选框应放置的位置。
    ///
    /// 平台层只需近似的光标位置；点击位置已足够精确，且避免了在每个页面间传递插入符矩形。
    ///
    ///
    ime_caret: Vec2,
    /// 由点击、导航或无障碍动作引发的焦点请求。
    ///
    /// 运行时拥有焦点管理器，因此外壳仅记录请求并在归约后交出。
    ///
    pending_focus: Option<u64>,
    /// 由无障碍动作引发的失焦请求。
    pending_blur: bool,
}

/// 运行时为本帧解析出的交互状态。
///
/// 外壳不再自行路由指针：运行时的 `Router` 挑选悬停 / 按压节点，
/// 其 `FocusManager` 拥有聚焦节点。因此绘制一帧只需只读快照，而非第二个路由器。
///
///
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Interaction {
    /// 指针下方的节点。
    pub hovered: Option<u64>,
    /// 指针正在按压的节点。
    pub pressed: Option<u64>,
    /// 当前聚焦的节点。
    pub focused: Option<u64>,
}

/// 外壳布局所依赖的输入。
///
/// 键相同的两帧具有相同的几何信息，因此第二帧无需再次求解。
///
#[derive(Copy, Clone, PartialEq, Eq)]
struct LayoutKey {
    width: u32,
    height: u32,
    nav_width: u32,
    page: u8,
    /// 处于打开状态的代码面板。
    code_mask: u64,
}

impl Default for GalleryApp {
    fn default() -> Self {
        Self::new()
    }
}

impl GalleryApp {
    /// 在首页创建展厅。
    #[must_use]
    pub fn new() -> Self {
        let mut tree = ShellTree::new();
        let layout = tree.solve(
            &GalleryState::default(),
            Rect::new(0.0, 0.0, 1280.0, 800.0),
            NAV_EXPANDED,
        );
        let mut nav_spring = Spring::new(SpringPreset::Snappy, NAV_EXPANDED);
        nav_spring.set_target(NAV_EXPANDED);
        Self {
            state: GalleryState::default(),
            scene: gallery::Gallery::new(),
            labels: Labels::new(),
            layout,
            focus_order: Vec::new(),
            regions: Vec::new(),
            ranges: Vec::new(),
            scroll_zones: Vec::new(),
            semantics: unbound_a11y::Semantics::new(ROOT),
            tree,
            page_layout: None,
            nav_spring,
            generation: 0,
            atlas_clock: 0,
            last_revision: 0,
            last_hover: None,
            last_pressed: None,
            last_focus: None,
            last_viewport: (0.0, 0.0),
            nav_max_scroll: 0.0,
            pointer: Vec2::ZERO,
            panning: None,
            dragging_range: None,
            effective_scale: 1.0,
            pending_window: None,
            window_maximized: false,
            scrollbar: None,
            scroll_drag: None,
            nav_scrollbar: None,
            nav_scroll_drag: None,
            list_scroll_drag: None,
            dirty_ranges: Vec::new(),
            layout_key: LayoutKey {
                width: 0,
                height: 0,
                nav_width: 0,
                page: 0,
                code_mask: 0,
            },
            external_diff: false,
            frame_layout_stats: LayoutStats::default(),
            node_spans: rustc_hash::FxHashMap::default(),
            emitted_widgets: rustc_hash::FxHashSet::default(),
            ime_caret: Vec2::new(24.0, 60.0),
            pending_focus: None,
            pending_blur: false,
        }
    }

    /// 为标题栏图标镜像原生窗口的最大化状态。
    pub fn set_window_maximized(&mut self, maximized: bool) {
        self.window_maximized = maximized;
    }

    /// 设置每逻辑单位对应的物理像素数（`OS scale × app zoom`）。
    ///
    /// 此后绘制的所有内容按逻辑单位布局，但按此缩放比例光栅化字形并对齐矩形。
    ///
    pub fn set_effective_scale(&mut self, scale: f32) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        self.effective_scale = scale.clamp(0.5, 4.0);
    }

    /// 当前使用的每逻辑单位物理像素数。
    #[must_use]
    pub fn effective_scale(&self) -> f32 {
        self.effective_scale
    }

    /// 取出上次按压请求的窗口动作（如有）。
    pub fn take_window_command(&mut self) -> Option<WindowCommand> {
        self.pending_window.take()
    }

    /// 由宿主运行时接管帧 diff。
    ///
    /// [`crate::runtime_app::GalleryRuntime`] 安装此设置，使场景仅由运行时精确 diff 一次，
    /// 而非此处一次、彼处一次。
    ///
    pub fn set_external_diff(&mut self, external: bool) {
        self.external_diff = external;
    }

    /// `point` 是否为可拖拽的标题栏背景。
    ///
    /// 运行时负责解析悬停，因此调用方将其传入：窗口按钮上的点属于点击，而非拖拽。
    ///
    #[must_use]
    pub fn is_titlebar_drag(&self, point: Vec2, hovered: Option<u64>) -> bool {
        self.layout.top_bar.contains_point(point) && hovered.is_none()
    }

    /// 外壳当前使用的调色板。
    #[must_use]
    pub fn palette(&self) -> Palette {
        gallery::palette()
    }

    /// 导航并关闭瞬态表面。
    pub fn navigate(&mut self, page: PageId) {
        self.state.navigate(page);
        self.state.search_focused = false;
        self.state.search_query.clear();
        self.state.demo.modal_open = false;
        self.state.demo.dropdown_open = false;
        self.pending_focus = Some(target::id(Target::NavPage(page.index() as u8)));
    }

    /// 像等价的指针或键盘手势一样应用无障碍动作。
    /// OS 适配器以引擎词汇表传回动作，因此 reducer 路径与点击相同——
    /// 辅助技术用户无法触及鼠标用户无法触及的代码路径。
    ///
    ///
    pub fn apply_a11y(
        &mut self,
        node: u64,
        action: unbound_a11y::A11yAction,
        value: Option<String>,
    ) {
        use unbound_a11y::A11yAction;
        match action {
            A11yAction::Click => {
                if let Some(target) = target::decode(node) {
                    let position = self.pointer;
                    self.click_target(target, position);
                }
            }
            A11yAction::Focus => {
                if target::decode(node).is_some() {
                    self.pending_focus = Some(node);
                    self.state.bump();
                }
            }
            A11yAction::Blur => {
                self.pending_blur = true;
                self.state.bump();
            }
            A11yAction::SetValue => {
                if let Some(value) = value {
                    if self.state.search_focused {
                        self.state.search_query = value;
                        self.state.bump();
                    } else if self.state.demo.typing {
                        self.state.demo.text = value;
                        self.state.bump();
                    }
                }
            }
            // 递增 / 递减需要控件的范围，该范围位于页面描述中；演示目前尚无此类描述符。
            //
            _ => {}
        }
    }

    /// 是否需要针对此交互快照重建场景。
    ///
    /// 悬停、按压或焦点变化会重绘控件但不会移动任何元素，
    /// 因此快照未变的帧直接复用上一帧场景而非重建。`force` 来自运行时：
    /// 显式的 `AppRuntime::invalidate` 即使在无元素移动时也会触发重建。
    ///
    #[must_use]
    pub fn needs_rebuild(&self, viewport: (f32, f32), inter: &Interaction, force: bool) -> bool {
        force
            || self.last_revision != self.state.revision
            || self.last_hover != inter.hovered
            || self.last_pressed != inter.pressed
            || self.last_focus != inter.focused
            || self.last_viewport != viewport
            || !self.nav_spring.is_settled()
    }

    /// 上次重建中发生变化的图元范围。
    ///
    /// 空列表表示“整个场景已变化”。非空列表指明渲染器需修补进缓冲池的切片。
    ///
    #[must_use]
    pub fn dirty_ranges(&self) -> &[(u32, u32)] {
        &self.dirty_ranges
    }

    /// 宿主已上传待处理切片：丢弃它们。
    ///
    /// `dirty_ranges` 在两次上传之间的每次重建中累积，因此唯有宿主知道数据何时真正到达 GPU。
    ///
    pub fn mark_uploaded(&mut self) {
        self.dirty_ranges.clear();
    }

    /// 当前帧的布局统计信息。
    ///
    /// 当本帧无需重新布局时 `nodes_visited` 为零。
    #[must_use]
    pub fn layout_stats(&self) -> LayoutStats {
        self.frame_layout_stats
    }

    /// 单个控件拥有的图元范围 `[start, end)`。
    ///
    /// 手绘外壳控件与发射器控件都会注册该范围，因此脏切片总能归因到所属控件。
    ///
    #[must_use]
    pub fn span_of(&self, node: u64) -> Option<(u32, u32)> {
        self.node_spans.get(&node).copied()
    }

    /// 按节点标识索引的每个控件的图元范围。
    #[must_use]
    pub fn node_spans(&self) -> &rustc_hash::FxHashMap<u64, (u32, u32)> {
        &self.node_spans
    }

    /// 本帧实际运行的真实控件发射器。
    ///
    /// Showcase 覆盖率门禁读取此集合，以证明每个目录组件都由真实演示承载，而非手绘仿制品。
    ///
    #[must_use]
    pub fn emitted_widgets(&self) -> &rustc_hash::FxHashSet<unbound_widgets::WidgetCatalogId> {
        &self.emitted_widgets
    }

    /// 布局树求解出的页面几何信息。
    #[must_use]
    pub fn page_layout(&self) -> Option<&PageLayout> {
        self.page_layout.as_ref()
    }

    /// 作为重布局边界的卡片数量。
    ///
    /// 卡片尺寸固定，其内部任何内容都无法改变它：页面可在不触及卡片的情况下重新求解。
    ///
    #[must_use]
    pub fn relayout_boundary_count(&self) -> usize {
        self.tree.card_boundary_count()
    }

    /// 当前场景的上传代次。
    ///
    /// 代次未推进时 GPU 跳过上传，因此宿主必须在场景重建时精确推进它。
    /// 将其绑定到重建本身正是点击能够显示在屏幕上的原因：宿主不可能“忘记”递增。
    ///
    ///
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 布局所依赖的输入。
    fn layout_key(&self, viewport: (f32, f32), nav_width: f32) -> LayoutKey {
        let page = self.state.active_page;
        let mut code_mask = 0u64;
        for (index, _) in page.examples().iter().enumerate() {
            if self.state.code_open(page, index as u8) {
                code_mask |= 1u64 << index;
            }
        }
        LayoutKey {
            width: viewport.0.to_bits(),
            height: viewport.1.to_bits(),
            nav_width: nav_width.to_bits(),
            page: page.index() as u8,
            code_mask,
        }
    }

    /// 无条件重建。
    pub fn rebuild(&mut self, viewport: (f32, f32), inter: Interaction) {
        let (width, height) = viewport;
        let viewport_rect = Rect::new(0.0, 0.0, width.max(320.0), height.max(240.0));
        let nav_width = self.nav_spring.value.clamp(NAV_COLLAPSED, NAV_EXPANDED);
        let palette = gallery::palette();
        let hover = inter.hovered;
        let pressed = inter.pressed;
        // 布局树拥有几何信息：标题栏、导航窗格与页面均由树求解，而非手算测量。
        //
        //
        // 仅当发生*移动*的要素变化时才求解布局。悬停、按压或焦点变化会重绘控件
        // 但不移动任何元素，因此不得付出布局开销。
        //
        let key = self.layout_key(viewport, nav_width);
        if key != self.layout_key {
            self.layout_key = key;
            self.layout = self.tree.solve(&self.state, viewport_rect, nav_width);
            self.frame_layout_stats = self.tree.stats();
        } else {
            self.frame_layout_stats = LayoutStats::default();
        }
        let layout = self.layout;
        let nav_max_scroll = nav_height(&layout) - nav_tree_rect(&layout).size.y;
        self.state.nav_scroll = self.state.nav_scroll.clamp(0.0, nav_max_scroll.max(0.0));
        let content_height = self.tree.content_height();
        self.state.content_scroll = self.state.content_scroll.clamp(
            0.0,
            (content_height - layout.content.size.y + 48.0).max(0.0),
        );
        let page_layout = self
            .tree
            .page_layout(&self.state, self.state.content_scroll);
        if self.state.active_page == PageId::ListView {
            for card in 0..2 {
                let row_h = self.list_row_height(card);
                let total = self.list_total(card);
                let overscan_knob = if card == 0 { 2 } else { 1 };
                let overscan_spec = PageId::ListView.examples()[card].knobs[overscan_knob];
                let overscan =
                    self.state
                        .knob(PageId::ListView, card, overscan_knob, &overscan_spec)
                        >= 0.5;
                let overscan_screens = if overscan { 0.25 } else { 0.0 };
                let vlist = &mut self.state.demo.virtual_lists[card];
                if (vlist.row_height() - row_h).abs() > 0.01 {
                    *vlist = unbound_world::virtualize::VirtualList::new(row_h, overscan_screens);
                }
                let viewport_h = page_layout
                    .cards
                    .get(card)
                    .map(|c| (c.stage.size.y - 38.0).max(1.0))
                    .unwrap_or(262.0);
                let scroll = self.state.demo.list_scroll[card].max(0.0);
                self.state.demo.virtual_updates[card] = vlist.update(scroll, viewport_h, total);
            }
        } else if self.state.active_page == PageId::Assets {
            self.state.demo.poll_assets(&mut self.labels.engine);
        }

        // 上一帧场景保留一帧：与新场景做 diff 可告知渲染器哪些图元实际发生了变化。
        //
        let previous = std::mem::replace(&mut self.scene, gallery::Gallery::new());
        let focused = inter.focused;
        let scale = self.effective_scale;
        let pixel_snap = self.state.pixel_snap;
        // 图集时钟每次重建都会推进，独立于上传代次：代次仅在场景*形状*变化时推进，
        // 而驱逐顺序必须在每一帧前进。
        //
        self.atlas_clock = self.atlas_clock.wrapping_add(1);
        let atlas_clock = self.atlas_clock;
        let GalleryApp {
            scene,
            labels,
            state,
            ..
        } = self;
        labels.set_raster_scale(scale);
        // 字形槽位按最近最少使用帧进行驱逐。若无此时钟推进，所有槽位看起来一样旧，
        // 导致满载的图集永远驱逐左上角相同的那几个字形。
        //
        labels.cache.begin_frame(atlas_clock);
        // 外壳保留四种亚像素相位：字形的光栅按其*绝对*位置隐含的相位生成，
        // 四边形则围绕它放置在像素网格上。强制相位 0 会使每个字形偏离其应处位置
        // 最多达一像素，这正是字距抖动的原因。
        //
        //
        labels.set_snap_phases(false);
        let mut canvas = Canvas::new(
            scene,
            labels,
            &palette,
            (layout.viewport.size.x, layout.viewport.size.y),
            focused,
            hover,
            pressed,
            scale,
            pixel_snap,
        );
        draw_background(&mut canvas, &layout);
        draw_top_bar(&mut canvas, state, &layout, self.window_maximized);
        draw_nav(&mut canvas, state, &layout);
        draw_page(&mut canvas, state, &layout, &page_layout);
        draw_hud(&mut canvas, state, &layout, page_layout.cards.len());
        draw_search_overlay(&mut canvas, state, &layout);
        let output = canvas.finish();
        let scrollbar = content_scrollbar(&layout, &page_layout);
        let scene_len = scene.primitives.len() as u32;
        let draw_calls = 4 + u32::from(!scene.clips.is_empty());
        let _ = output.glyph_count;

        // 场景需要传输多少数据决定了帧的上传方式：仅重绘的变更在*同一*代次下修补切片，
        // 渲染器保持缓冲池其余部分不变。
        //
        //
        // 单帧可能重建两次（先事件处理，后 `tick`），第二次 diff 仅知晓自*第一次*重建以来的变更。
        // 因此待处理切片会累积，直到宿主实际将其上传（`mark_uploaded`）；
        // 此处直接覆写会丢失帧动画 tick 之前做出的所有绘制变更。
        //
        //
        let diff = diff_scenes(&previous, scene);
        if self.external_diff {
            // 宿主运行时自行对帧做 diff；丢弃外壳原本为自身上传器累积的切片。
            //
            self.dirty_ranges.clear();
        } else {
            let bumps_generation = diff.bumps_generation();
            match diff {
                // 无任何变化：本帧早期重建仍待处理的切片必须保留。
                //
                SceneDiff::Identical => {}
                SceneDiff::Ranges(ranges) => merge_ranges(&mut self.dirty_ranges, ranges),
                // 形状已变化，因此代次必须推进，整个场景重新全量上传。
                //
                SceneDiff::Whole => {
                    self.dirty_ranges.clear();
                    self.generation = self.generation.wrapping_add(1);
                }
            }
            let stride = core::mem::size_of::<unbound_gpu::data::GpuPrimitiveInstance>() as u32;
            self.state.demo.upload_bytes = if bumps_generation {
                (scene.primitives.len() as u32).saturating_mul(stride)
            } else {
                self.dirty_ranges
                    .iter()
                    .map(|(start, end)| (end - start) * stride)
                    .sum()
            };
        }
        self.state.demo.primitive_count = scene_len;
        self.state.demo.draw_calls = draw_calls;
        self.regions = output.regions;
        self.focus_order = output.focus_order;
        self.ranges = output.ranges;
        self.scroll_zones = output.scroll_zones;
        self.semantics = output.semantics;
        self.node_spans = output.node_spans;
        self.emitted_widgets = output.emitted_widgets;
        self.layout = layout;
        self.page_layout = Some(page_layout);
        self.scrollbar = scrollbar;
        self.nav_scrollbar = nav_scrollbar(&layout);
        self.nav_max_scroll = nav_max_scroll.max(0.0);
        self.last_revision = self.state.revision;
        self.last_hover = hover;
        self.last_pressed = pressed;
        self.last_focus = focused;
        self.last_viewport = viewport;
    }

    /// 推进时间驱动的演示。
    pub fn tick(&mut self, dt: f32) -> bool {
        let target = if self.state.nav_expanded {
            NAV_EXPANDED
        } else {
            NAV_COLLAPSED
        };
        self.nav_spring.set_target(target);
        let nav_moving = self.nav_spring.step(dt);
        let demo_moving = self.state.tick(dt);
        if nav_moving || demo_moving {
            self.state.bump();
        }
        nav_moving || demo_moving
    }

    /// 移动指针（逻辑像素）。
    ///
    /// 仅外壳正在运行的拖拽会跟随指针；悬停本身由运行时负责，此处无需路由。
    ///
    pub fn pointer_move(&mut self, position: Vec2) {
        self.pointer = position;
        if let Some(grab) = self.scroll_drag {
            if let Some(bar) = self.scrollbar {
                let scroll = bar.scroll_for(position.y - grab);
                if (scroll - self.state.content_scroll).abs() > 0.01 {
                    self.state.content_scroll = scroll;
                    self.state.bump();
                }
            }
        }
        if let Some(grab) = self.nav_scroll_drag {
            if let Some(bar) = self.nav_scrollbar {
                let scroll = bar.scroll_for(position.y - grab);
                if (scroll - self.state.nav_scroll).abs() > 0.01 {
                    self.state.nav_scroll = scroll;
                    self.state.bump();
                }
            }
        }
        if let Some((card, grab)) = self.list_scroll_drag {
            if let Some(bar) = self.list_scrollbar(card) {
                let scroll = bar.scroll_for(position.y - grab);
                if (scroll - self.state.demo.list_scroll[card]).abs() > 0.01 {
                    self.state.demo.list_scroll[card] = scroll;
                    self.state.bump();
                }
            }
        }
        // 拖拽仅在外壳于按压时启动后才存在，因此无需查询快照：
        // `Option` *本身*就代表捕获状态。
        //
        if let Some(node) = self.dragging_range {
            self.update_range(node, position);
        }
        if self.panning.is_some() {
            let zoom = self.state.demo.canvas_zoom.max(0.01);
            let delta = position - self.panning.unwrap_or(position);
            self.state.demo.canvas_pan -= delta / zoom;
            self.panning = Some(position);
            self.state.bump();
        }
        if let Some((start, _)) = self.state.demo.marquee {
            self.state.demo.marquee = Some((start, position));
            self.state.bump();
        }
        if self.state.demo.drag_active.is_some() {
            self.state.demo.drag_position = Some(position);
            self.state.bump();
        }
        if self.state.demo.curve_editor.active().is_some() {
            if let Some(rect) = demo_rect(self, PageId::Springs, 1, 1) {
                let point = CurveEditorDesc::new(rect).from_screen(position);
                if self.state.demo.curve_editor.drag_to(point) {
                    self.state.bump();
                }
            }
        }
    }

    /// 按下指针。
    ///
    /// 仅*捕获*指针的控件在此响应：滚动条、滑块与可拖拽演示舞台。
    /// 所有离散控件均等待释放，因此按下后拖离控件绝不会激活它。
    ///
    ///
    pub fn pointer_down(&mut self, position: Vec2, target: Option<u64>) {
        self.pointer = position;
        // 点击 FPS 输入框之外的任意位置会提交待定值，就像原生微调框在失焦时一样。
        //
        if self.state.fps_focused && target != Some(target::id(Target::FpsInput)) {
            self.state.commit_fps_input();
        }
        if let Some(node) = target {
            self.pending_focus = Some(node);
            if let Some(target) = target::decode(node) {
                self.begin_press_target(target, position);
            }
        }
    }

    /// 释放指针。
    ///
    /// 释放本身从不激活控件：仅当按压落在同一控件上且从未转为拖拽时，
    /// 运行时才单独上报 [`AppEvent::Click`](unbound_app::AppEvent::Click)。
    ///
    pub fn pointer_up(&mut self, position: Vec2) {
        self.pointer = position;
        self.finish_drag(position);
        self.dragging_range = None;
        self.panning = None;
        if self.state.demo.curve_editor.active().is_some() {
            self.state.demo.curve_editor.end();
            self.state.bump();
        }
    }

    /// 中止所有进行中的交互。
    ///
    /// 窗口失去焦点：必须清除指针捕获、滚动条抓取与演示拖拽，
    /// 否则外壳会一直保留幽灵拖拽，直到用户恰好点击别处。
    ///
    pub fn cancel_interactions(&mut self) {
        self.scroll_drag = None;
        self.nav_scroll_drag = None;
        self.list_scroll_drag = None;
        self.dragging_range = None;
        self.panning = None;
        // 两个拖拽都必须被清除，而非仅第一个：`||` 会短路并使第二个拖拽保持存活。
        //
        let marquee = self.state.demo.marquee.take();
        let drag = self.state.demo.drag_active.take();
        if marquee.is_some() || drag.is_some() {
            self.state.demo.drag_position = None;
            self.state.bump();
        }
        if self.state.demo.curve_editor.active().is_some() {
            self.state.demo.curve_editor.end();
            self.state.bump();
        }
    }

    /// 外壳上的滚轮事件。
    pub fn wheel(&mut self, delta_y: f32) {
        let point = self.pointer;
        if self.layout.nav.contains_point(point) {
            self.state.nav_scroll =
                (self.state.nav_scroll - delta_y * 44.0).clamp(0.0, self.nav_max_scroll);
            self.state.bump();
            return;
        }
        if !self.layout.content.contains_point(point) {
            return;
        }
        if let Some((zone_rect, tag)) = self
            .scroll_zones
            .iter()
            .find(|(rect, _)| rect.contains_point(point))
        {
            let zone_rect = *zone_rect;
            let tag = *tag;
            match tag {
                // 标签 1 为第一张列表卡片，标签 5 为第二张；各自拥有独立的偏移与行度量。
                // 区域即视口，因此其高度就是精确的钳制边界（不含表头）。
                //
                1 | 5 => {
                    let card = if tag == 1 { 0 } else { 1 };
                    let row = self.list_row_height(card);
                    let total = self.list_total(card);
                    let stage_h = zone_rect.size.y;
                    let scroll = self.state.demo.list_scroll[card];
                    self.state.demo.list_scroll[card] = (scroll - delta_y * row * 2.0)
                        .clamp(0.0, (total as f32 * row - stage_h).max(0.0));
                    self.state.bump();
                }
                2 => {
                    let factor = (1.0 + delta_y * 0.08).clamp(0.85, 1.2);
                    self.state.demo.canvas_zoom =
                        (self.state.demo.canvas_zoom * factor).clamp(0.4, 2.5);
                    self.state.bump();
                }
                3 | 4 => {}
                _ => {}
            }
            return;
        }
        let max = self.page_layout.as_ref().map_or(0.0, |page| {
            (page.content_height - self.layout.content.size.y + 48.0).max(0.0)
        });
        self.state.content_scroll = (self.state.content_scroll - delta_y * 52.0).clamp(0.0, max);
        self.state.bump();
    }

    /// 文本控件是否持有键盘焦点。
    ///
    /// 窗口仅在此为 true 时启用 IME；若对整个应用常驻启用，会在每个页面上弹出候选条。
    ///
    #[must_use]
    pub fn ime_active(&self) -> bool {
        self.state.search_focused || self.state.demo.typing
    }

    /// 候选框应锚定的位置（逻辑像素）。
    #[must_use]
    pub fn ime_caret(&self) -> Vec2 {
        self.ime_caret
    }

    /// 将已提交的 IME 文本插入当前聚焦控件。
    ///
    /// 控制字符会被丢弃：文本框为单行，提交中混入的换行符会破坏缓冲区。
    ///
    pub fn type_text(&mut self, text: &str) {
        let filtered: String = text.chars().filter(|c| !c.is_control()).collect();
        if filtered.is_empty() {
            return;
        }
        if self.state.search_focused {
            self.state.search_query.push_str(&filtered);
        } else if self.state.demo.typing {
            self.state.demo.text.push_str(&filtered);
        } else {
            return;
        }
        self.state.bump();
    }

    /// 处理按住不放时 OS 重复发送的按键。
    ///
    /// 文本被排除在外：长按 `a` 不是一连串 `a`，且 `winit` 也会再次上报已组合字符。
    /// 仅编辑键触发重复。
    pub fn key_repeat(&mut self, key: &PlatformKey) {
        let PlatformKey::Named(name) = key else {
            return;
        };
        if name != "Backspace" {
            return;
        }
        if self.state.search_focused {
            self.state.search_query.pop();
        } else if self.state.demo.typing {
            self.state.demo.text.pop();
        } else {
            return;
        }
        self.state.bump();
    }

    /// 处理按键按下。
    /// `focused` 是运行时的聚焦节点：外壳本身不保存聚焦节点。
    /// `Tab` 绝不会到达此方法——运行时的 `FocusManager` 在派发前已步进焦点顺序。
    pub fn key(&mut self, key: &PlatformKey, modifiers: Modifiers, focused: Option<u64>) {
        if self.state.fps_focused {
            match key {
                PlatformKey::Named(name) if name == "Escape" => {
                    self.state.fps_focused = false;
                    self.state.fps_input = self.state.fps_limit.to_string();
                    self.state.bump();
                }
                PlatformKey::Named(name) if name == "Enter" => {
                    self.state.commit_fps_input();
                }
                PlatformKey::Named(name) if name == "Backspace" => {
                    self.state.fps_input.pop();
                    self.state.bump();
                }
                PlatformKey::Character(text) if !modifiers.control => {
                    // 仅限数字：上限为 FPS 数值。
                    for c in text.chars().filter(char::is_ascii_digit) {
                        self.state.fps_input.push(c);
                    }
                    self.state.bump();
                }
                _ => {}
            }
            return;
        }
        if self.state.search_focused {
            match key {
                PlatformKey::Named(name) if name == "Escape" => {
                    self.state.search_focused = false;
                    self.state.search_query.clear();
                    self.state.bump();
                }
                PlatformKey::Named(name) if name == "Backspace" => {
                    self.state.search_query.pop();
                    self.state.bump();
                }
                PlatformKey::Named(name) if name == "Enter" => {
                    if let Some(page) = self.state.search_results().first().copied() {
                        self.navigate(page);
                    }
                }
                PlatformKey::Character(text) if !modifiers.control => {
                    self.state.search_query.push_str(text);
                    self.state.bump();
                }
                _ => {}
            }
            return;
        }
        if self.state.demo.typing {
            match key {
                PlatformKey::Named(name) if name == "Escape" || name == "Enter" => {
                    self.state.demo.typing = false;
                    self.state.bump();
                }
                PlatformKey::Named(name) if name == "Backspace" => {
                    self.state.demo.text.pop();
                    self.state.bump();
                }
                PlatformKey::Character(text) if !modifiers.control => {
                    self.state.demo.text.push_str(text);
                    self.state.bump();
                }
                _ => {}
            }
            return;
        }
        if modifiers.control {
            if let PlatformKey::Character(text) = key {
                if text.eq_ignore_ascii_case("k") {
                    self.state.search_focused = true;
                    self.state.bump();
                }
            }
            return;
        }
        match key {
            PlatformKey::Named(name) if name == "Enter" => {
                if let Some(node) = focused {
                    if let Some(target) = target::decode(node) {
                        self.activate_target(target, self.pointer);
                    }
                }
            }
            PlatformKey::Character(text) if text == " " => {
                if let Some(node) = focused {
                    if let Some(target) = target::decode(node) {
                        self.activate_target(target, self.pointer);
                    }
                }
            }
            PlatformKey::Named(name) if name == "Escape" => {
                self.state.demo.modal_open = false;
                self.state.demo.dropdown_open = false;
                self.state.search_focused = false;
                self.state.search_query.clear();
                self.state.bump();
            }
            PlatformKey::Named(name) if name == "F12" => {
                self.state.show_hud = !self.state.show_hud;
                self.state.bump();
            }
            PlatformKey::Named(name)
                if matches!(
                    name.as_str(),
                    "ArrowDown" | "ArrowUp" | "PageDown" | "PageUp" | "Home" | "End"
                ) =>
            {
                self.scroll_page(name);
            }
            PlatformKey::Character(text) if text.eq_ignore_ascii_case("t") => {
                self.state.theme = self.state.theme.toggled();
                self.state.bump();
            }
            _ => {}
        }
    }

    /// 使用键盘滚动页面。
    ///
    /// 仅有滚轮是不够的：页面必须能在无指针时访问，`Home` / `End` 是跳转到顶部与底部的惯例方式。
    ///
    ///
    fn scroll_page(&mut self, key: &str) {
        let viewport = self.layout.content.size.y;
        let step = match key {
            "ArrowDown" => 40.0,
            "ArrowUp" => -40.0,
            "PageDown" => viewport * 0.8,
            "PageUp" => -viewport * 0.8,
            "Home" => f32::NEG_INFINITY,
            "End" => f32::INFINITY,
            _ => return,
        };
        let max = self.page_layout.as_ref().map_or(0.0, |page| {
            (page.content_height - self.layout.content.size.y + 48.0).max(0.0)
        });
        let next = if step.is_infinite() {
            if step > 0.0 {
                max
            } else {
                0.0
            }
        } else {
            (self.state.content_scroll + step).clamp(0.0, max)
        };
        if (next - self.state.content_scroll).abs() > 0.01 {
            self.state.content_scroll = next;
            self.state.bump();
        }
    }

    /// 激活点击落点处的控件。
    ///
    /// 仅当释放在被按压的控件上且按压从未转为拖拽时，运行时才上报点击，
    /// 因此这是唯一的激活入口。
    ///
    pub fn click(&mut self, node: u64, position: Vec2) {
        if let Some(target) = target::decode(node) {
            self.click_target(target, position);
        }
    }

    /// 上次归约引发的焦点请求（如有）。
    pub fn take_focus_request(&mut self) -> Option<u64> {
        self.pending_focus.take()
    }

    /// 上次归约是否请求失焦。
    pub fn take_blur_request(&mut self) -> bool {
        core::mem::take(&mut self.pending_blur)
    }

    /// 悬停控件是否捕获指针，使窗口调整大小边框必须让步而非抢走按压或悬停。
    ///
    #[must_use]
    pub fn hover_captures_pointer(&self, node: Option<u64>) -> bool {
        let Some(node) = node else {
            return false;
        };
        matches!(
            target::decode(node),
            Some(
                Target::ContentScrollbar
                    | Target::NavScrollbar
                    | Target::Knob(..)
                    | Target::Demo(..)
            )
        )
    }
    /// 开始拖拽页面滚动条滑块。
    ///
    /// 按在滑块上会从当前位置抓取它；按在轨道上则会把滑块跳转到指针位置。
    ///
    fn begin_scrollbar_drag(&mut self, position: Vec2) {
        let Some(bar) = self.scrollbar else {
            return;
        };
        let thumb_top = bar.thumb_top(self.state.content_scroll);
        let grab = position.y - thumb_top;
        self.scroll_drag = Some(grab);
        if !(0.0..=bar.thumb_h).contains(&grab) {
            // 按在轨道上——无论在滑块上方还是下方——都会将滑块中心对齐到指针并从该处继续拖拽。
            // 过去只检查 `grab > thumb_h` 导致滑块上方点击存入负的抓取偏移，
            // 并在下次移动时使页面发生跳变。
            //
            let centred = position.y - bar.thumb_h * 0.5;
            let scroll = bar.scroll_for(centred);
            if (scroll - self.state.content_scroll).abs() > 0.01 {
                self.state.content_scroll = scroll;
                self.state.bump();
            }
            self.scroll_drag = Some(bar.thumb_h * 0.5);
        }
    }

    /// 开始拖拽导航窗格滚动条滑块。
    fn begin_nav_scrollbar_drag(&mut self, position: Vec2) {
        let Some(bar) = self.nav_scrollbar else {
            return;
        };
        let thumb_top = bar.thumb_top(self.state.nav_scroll);
        let grab = position.y - thumb_top;
        self.nav_scroll_drag = Some(grab);
        if !(0.0..=bar.thumb_h).contains(&grab) {
            let centred = position.y - bar.thumb_h * 0.5;
            let scroll = bar.scroll_for(centred);
            if (scroll - self.state.nav_scroll).abs() > 0.01 {
                self.state.nav_scroll = scroll;
                self.state.bump();
            }
            self.nav_scroll_drag = Some(bar.thumb_h * 0.5);
        }
    }

    /// 开始拖拽虚拟列表卡片的滚动条滑块。
    fn begin_list_scrollbar_drag(&mut self, card: usize, position: Vec2) {
        let Some(bar) = self.list_scrollbar(card) else {
            return;
        };
        let thumb_top = bar.thumb_top(self.state.demo.list_scroll[card]);
        let grab = position.y - thumb_top;
        self.list_scroll_drag = Some((card, grab));
        if !(0.0..=bar.thumb_h).contains(&grab) {
            let centred = position.y - bar.thumb_h * 0.5;
            let scroll = bar.scroll_for(centred);
            if (scroll - self.state.demo.list_scroll[card]).abs() > 0.01 {
                self.state.demo.list_scroll[card] = scroll;
                self.state.bump();
            }
            self.list_scroll_drag = Some((card, bar.thumb_h * 0.5));
        }
    }

    /// 开始按压捕获式交互。
    ///
    /// 仅需要*捕获*指针的控件在按压时响应：内容滚动条、滑块与可拖拽演示舞台。
    /// 所有离散控件均等待释放，因此按下按钮后拖离绝不会激活它。
    ///
    ///
    fn begin_press_target(&mut self, target: Target, position: Vec2) {
        match target {
            Target::ContentScrollbar => {
                self.begin_scrollbar_drag(position);
            }
            Target::NavScrollbar => {
                self.begin_nav_scrollbar_drag(position);
            }
            Target::Knob(page, card, knob) => {
                // 仅滑块捕获指针。开关或复选框属于离散控件，等待释放。
                //
                if knob_spec(page, card, knob)
                    .and_then(|spec| spec.range())
                    .is_some()
                {
                    self.dragging_range = Some(target::id(target));
                    self.update_range(target::id(target), position);
                }
            }
            Target::Demo(page, card, part) => {
                self.press_demo(page, card, part, position);
            }
            _ => {}
        }
    }

    /// 激活控件。
    ///
    /// 外壳的所有离散动作均在此发生：导航、主题与缩放切换、开关、分段选项、
    /// 代码面板与窗口按钮。路由器仅当释放在被按压的控件上且按压从未转为拖拽时才上报点击，
    /// 因此拖离的按压不会产生任何效果。
    ///
    ///
    fn click_target(&mut self, target: Target, position: Vec2) {
        match target {
            Target::NavPage(page) => self.navigate(PageId::from_index(page as usize)),
            Target::NavToggle => {
                self.state.nav_expanded = !self.state.nav_expanded;
                self.state.bump();
            }
            Target::Theme(bit) => {
                self.state.theme = if bit == 0 {
                    ThemeMode::Light
                } else {
                    ThemeMode::Dark
                };
                self.state.bump();
            }
            Target::Zoom(step) => {
                self.state.zoom_step = (step as usize).min(ZOOM_STEPS.len() - 1);
                self.state.bump();
            }
            Target::Snap => {
                self.state.pixel_snap = !self.state.pixel_snap;
                self.state.bump();
            }
            Target::Hud => {
                self.state.show_hud = !self.state.show_hud;
                self.state.bump();
            }
            Target::Search => {
                self.state.search_focused = true;
                self.state.demo.typing = false;
                self.ime_caret = self.pointer;
                self.state.bump();
            }
            Target::FpsPreset(index) => {
                let fps = [60, 120, 144, 240, 0][usize::from(index).min(4)];
                self.state.set_fps_limit(fps);
                self.state.fps_focused = false;
            }
            Target::FpsInput => {
                self.state.fps_focused = true;
                self.state.fps_input = self.state.fps_limit.to_string();
                self.ime_caret = self.pointer;
                self.state.bump();
            }
            Target::Footer(index) => self.navigate(if index == 0 {
                PageId::Telemetry
            } else {
                PageId::Settings
            }),
            Target::SearchResult(page) => self.navigate(PageId::from_index(page as usize)),
            Target::Knob(page, card, knob) => {
                // 滑块已由按压和拖拽移动过；开关或复选框在此处翻转。
                //
                if knob_spec(page, card, knob)
                    .and_then(|spec| spec.range())
                    .is_none()
                {
                    let page_id = PageId::from_index(page as usize);
                    if let Some(spec) = knob_spec(page, card, knob) {
                        let current = self
                            .state
                            .knob(page_id, card as usize, knob as usize, &spec);
                        self.state.set_knob(
                            page_id,
                            card as usize,
                            knob as usize,
                            if current > 0.5 { 0.0 } else { 1.0 },
                        );
                    }
                }
            }
            Target::KnobOption(page, card, knob, option) => {
                self.state.set_knob(
                    PageId::from_index(page as usize),
                    card as usize,
                    knob as usize,
                    option as f32,
                );
                // 缓动演示的预设分段控件将引擎的标准曲线加载到交互式编辑器中。
                //
                if PageId::from_index(page as usize) == PageId::Springs && card == 1 && knob == 0 {
                    let bezier = match option {
                        0 => CubicBezier::EASE,
                        2 => CubicBezier::LINEAR,
                        _ => CubicBezier::EASE_OUT,
                    };
                    self.state.demo.curve_editor = CurveEditorState::with_points(
                        Vec2::new(bezier.x1, bezier.y1),
                        Vec2::new(bezier.x2, bezier.y2),
                    );
                }
            }
            Target::Code(page, card) => {
                self.state
                    .toggle_code(PageId::from_index(page as usize), card);
            }
            Target::Demo(page, card, part) => {
                self.click_demo(page, card, part, position);
            }
            Target::WindowMinimize => {
                self.pending_window = Some(WindowCommand::Minimize);
            }
            Target::WindowMaximize => {
                self.pending_window = Some(WindowCommand::ToggleMaximize);
            }
            Target::WindowClose => {
                self.pending_window = Some(WindowCommand::Close);
            }
            Target::ContentScrollbar => {
                // 滚动条在按压时生效，而非释放时。
            }
            Target::NavScrollbar => {
                // 导航滚动条在按压时生效，而非释放时。
            }
        }
    }

    /// 通过键盘激活控件。
    ///
    /// 键盘无法拖拽，因此滑块跳转到指针位置，而非跟随拖拽。
    ///
    fn activate_target(&mut self, target: Target, position: Vec2) {
        match target {
            Target::Knob(page, card, knob)
                if knob_spec(page, card, knob)
                    .and_then(|spec| spec.range())
                    .is_some() =>
            {
                self.update_range(target::id(target), position);
            }
            _ => self.click_target(target, position),
        }
    }

    fn press_demo(&mut self, page: u8, card: u8, part: u16, position: Vec2) {
        let page = PageId::from_index(page as usize);
        if page == PageId::Viewport && part == 1 {
            self.panning = Some(position);
        }
        if page == PageId::Marquee && part == 1 {
            self.state.demo.marquee = Some((position, position));
            self.state.bump();
        }
        if page == PageId::DragDrop && card == 0 && (1..=4).contains(&part) {
            self.state.demo.drag_active = Some(part as usize - 1);
            self.state.demo.drag_position = Some(position);
            self.state.bump();
        }
        if page == PageId::ListView && part == 200 {
            self.begin_list_scrollbar_drag(card as usize, position);
        }
        if page == PageId::Sliders {
            if let Some(zone) = self
                .ranges
                .iter()
                .find(|zone| zone.node == target::id(Target::Demo(page.index() as u8, card, part)))
            {
                self.dragging_range = Some(zone.node);
                self.update_range(zone.node, position);
            }
        }
        if page == PageId::Springs && card == 1 && part == 1 {
            if let Some(rect) = demo_rect(self, page, card, part) {
                let desc = CurveEditorDesc::new(rect);
                let curve = &mut self.state.demo.curve_editor;
                let handle = if (position - desc.to_screen(curve.p1())).length_squared()
                    <= (position - desc.to_screen(curve.p2())).length_squared()
                {
                    CurveHandle::P1
                } else {
                    CurveHandle::P2
                };
                curve.begin(handle);
                curve.drag_to(desc.from_screen(position));
                self.state.bump();
            }
        }
    }

    fn click_demo(&mut self, page: u8, card: u8, part: u16, position: Vec2) {
        let page = PageId::from_index(page as usize);
        // 帧率栏由 Telemetry 与 Settings 共用，并在两个页面上使用相同的部件 ID。
        //
        if matches!(page, PageId::Telemetry | PageId::Settings) {
            match part {
                20..=24 => {
                    let fps = [60, 120, 144, 240, 0][(part - 20) as usize];
                    self.state.set_fps_limit(fps);
                    self.state.fps_focused = false;
                    return;
                }
                30 => {
                    self.state.fps_focused = true;
                    self.state.fps_input = self.state.fps_limit.to_string();
                    self.ime_caret = position;
                    self.state.bump();
                    return;
                }
                _ => {}
            }
        }
        match page {
            PageId::Home => match card {
                0 => {
                    if part == 1 {
                        self.navigate(PageId::Buttons);
                    } else if part == 2 {
                        self.navigate(PageId::Viewport);
                    }
                }
                1 => {
                    if let Some((_, _, target, _)) =
                        pages::home::CAPABILITIES.get(part as usize - 1)
                    {
                        self.navigate(*target);
                    }
                }
                2 => {
                    if let Some(target) = pages::home::INDEX_PAGES.get(part as usize - 1) {
                        self.navigate(*target);
                    }
                }
                _ => {}
            },
            PageId::Buttons => {
                if card == 0 {
                    self.state.demo.clicks = self.state.demo.clicks.wrapping_add(1);
                    self.state.bump();
                } else if (10..18).contains(&part) {
                    self.state.demo.selected_icon = part as usize - 10;
                    self.state.bump();
                }
            }
            PageId::Toggles => match part {
                1 => {
                    self.state.demo.toggle_a = !self.state.demo.toggle_a;
                    self.state.bump();
                }
                2 => {
                    self.state.demo.toggle_b = !self.state.demo.toggle_b;
                    self.state.bump();
                }
                10..=12 => {
                    self.state.demo.radio_choice = (part - 10) as usize;
                    self.state.bump();
                }
                20..=22 => {
                    self.state.demo.set_pill((part - 20) as usize);
                    self.state.bump();
                }
                30..=34 => {
                    self.state.demo.set_pill((part - 30) as usize);
                    self.state.bump();
                }
                _ => {}
            },
            PageId::TextField => {
                if part == 1 {
                    self.state.demo.typing = true;
                    self.state.search_focused = false;
                    self.ime_caret = self.pointer;
                    self.state.bump();
                }
            }
            PageId::Fonts => {
                // BiDi 沙盒的自定义预设是真实的文本输入框：IME 必须像文本域页面那样
                // 组合进 `demo.text`，否则中日韩 / 阿拉伯语输入法永远只能输入 ASCII。
                //
                //
                if card == 2 && part == 1 {
                    self.state.demo.typing = true;
                    self.state.search_focused = false;
                    self.ime_caret = self.pointer;
                    self.state.bump();
                }
            }
            PageId::Overlays => match part {
                1 => {
                    self.state.demo.dropdown_open = !self.state.demo.dropdown_open;
                    self.state.demo.modal_open = false;
                    self.state.bump();
                }
                2 => {
                    self.state.demo.modal_open = !self.state.demo.modal_open;
                    self.state.demo.dropdown_open = false;
                    self.state.bump();
                }
                10..=12 => {
                    self.state.demo.dropdown_open = false;
                    self.state.bump();
                }
                20 | 21 => {
                    self.state.demo.modal_open = false;
                    self.state.bump();
                }
                _ => {}
            },
            PageId::ListView => {
                if (100..112).contains(&part) {
                    let card = card as usize;
                    let row = self.list_row_height(card);
                    let first = (self.state.demo.list_scroll[card] / row).floor() as usize;
                    self.state.demo.list_selected[card] = Some(first + (part - 100) as usize);
                    self.state.bump();
                }
            }
            PageId::Marquee => {
                if (10..22).contains(&part) {
                    let index = (part - 10) as usize;
                    self.state.demo.selected_nodes[index] = !self.state.demo.selected_nodes[index];
                    self.state.bump();
                }
            }
            PageId::Undo => match part {
                1 => self.state.undo(),
                2 => self.state.redo(),
                3 => self.state.push_history("Move node"),
                _ => {}
            },
            PageId::Springs if card == 0 => match part {
                1 => {
                    self.state.demo.spring_high = !self.state.demo.spring_high;
                    self.state
                        .demo
                        .spring
                        .set_target(if self.state.demo.spring_high {
                            1.0
                        } else {
                            0.0
                        });
                    self.state.bump();
                }
                2 => {
                    self.state.demo.spring.snap_to(0.0);
                    self.state.demo.spring_high = true;
                    self.state.demo.spring.set_target(1.0);
                    self.state.bump();
                }
                _ => {}
            },
            PageId::A11y => {
                if (10..=12).contains(&part) {
                    self.state.demo.a11y_focus = (part - 10) as usize;
                    self.state.bump();
                }
            }
            PageId::Components => {
                if let Some(example) = PageId::Components.examples().get(card as usize) {
                    if let pages::DemoKind::Component { id } = example.demo {
                        if crate::state::reduce_showcase_component(
                            id,
                            &mut self.state.sandbox,
                            Some(position),
                        ) {
                            self.state.bump();
                        }
                    }
                }
            }
            _ => {
                let _ = position;
            }
        }
    }

    fn finish_drag(&mut self, position: Vec2) {
        self.scroll_drag = None;
        self.nav_scroll_drag = None;
        self.list_scroll_drag = None;
        if let Some(start) = self.state.demo.marquee.map(|(start, _)| start) {
            let marquee = Rect::from_points(start, position);
            for index in 0..12u16 {
                if let Some(candidate) = demo_rect(self, PageId::Marquee, 0, 10 + index) {
                    if marquee.contains_point(candidate.center()) {
                        self.state.demo.selected_nodes[index as usize] = true;
                    }
                }
            }
            self.state.demo.marquee = None;
            self.state.bump();
        }
        if let Some(index) = self.state.demo.drag_active {
            let target_slot = (1..=4u16)
                .filter_map(|part| {
                    demo_rect(self, PageId::DragDrop, 0, part).map(|rect| (part, rect))
                })
                .min_by(|(_, a), (_, b)| {
                    let da = (a.center() - position).length_squared();
                    let db = (b.center() - position).length_squared();
                    da.total_cmp(&db)
                })
                .map_or(index, |(part, _)| part as usize - 1);
            let mut order = self.state.demo.drag_order.clone();
            if let Some(from) = order.iter().position(|value| *value == index) {
                let card = order.remove(from);
                let slot = target_slot.min(order.len());
                order.insert(slot, card);
                self.state.demo.drag_order = order;
            }
            self.state.demo.drag_active = None;
            self.state.demo.drag_position = None;
            self.state.bump();
        }
    }

    fn update_range(&mut self, node: u64, position: Vec2) {
        let Some(zone) = self.ranges.iter().find(|zone| zone.node == node).copied() else {
            return;
        };
        // 从指针位置到数值的映射属于控件本身，而非外壳：同一条规则负责绘制并移动滑块。
        //
        //
        let desc = unbound_widgets::slider::SliderDesc::new(
            zone.rect, "Slider", zone.min, zone.min, zone.max,
        )
        .with_step(zone.step);
        let value = unbound_widgets::slider::value_from_pointer(position, &desc);
        match target::decode(node) {
            Some(Target::Knob(page, card, knob)) => {
                let page_id = PageId::from_index(page as usize);
                self.state
                    .set_knob(page_id, card as usize, knob as usize, value);
                if page_id == PageId::Springs && card == 0 && knob <= 1 {
                    if knob == 0 {
                        self.state.demo.spring.stiffness = value.max(1.0);
                    } else {
                        self.state.demo.spring.damping = value.max(0.1);
                    }
                }
            }
            Some(Target::Demo(page, _, part))
                if page as usize == PageId::Sliders.index() && (1..=3).contains(&part) =>
            {
                self.state.demo.sliders[part as usize - 1] = value;
                self.state.bump();
            }
            _ => {}
        }
    }

    fn list_total(&self, card: usize) -> usize {
        let spec = PageId::ListView.examples()[card].knobs[0];
        match self.state.knob(PageId::ListView, card, 0, &spec) as usize {
            0 => 1_000,
            1 => 10_000,
            _ => 100_000,
        }
    }

    fn list_row_height(&self, card: usize) -> f32 {
        let spec = PageId::ListView.examples()[card].knobs[1];
        self.state
            .knob(PageId::ListView, card, 1, &spec)
            .clamp(24.0, 48.0)
    }

    /// 卡片的虚拟列表滚动条几何信息，根据已注册的轨道热区重建，
    /// 使拖拽与绘制内容保持同步。
    fn list_scrollbar(&self, card: usize) -> Option<Scrollbar> {
        let track = demo_rect(self, PageId::ListView, card as u8, 200)?;
        let viewport_h = track.size.y + 8.0;
        let row = self.list_row_height(card);
        let content_h = self.list_total(card) as f32 * row;
        let max_scroll = (content_h - viewport_h).max(0.0);
        if max_scroll <= 1.0 {
            return None;
        }
        let thumb_h = (viewport_h * viewport_h / content_h)
            .max(18.0)
            .min(track.size.y);
        Some(Scrollbar {
            track,
            thumb_h,
            max_scroll,
        })
    }

    /// 供无头校验器使用的仅布局帧。
    #[must_use]
    pub fn frame(&self) -> Frame {
        let mut builder = FrameBuilder::new(self.layout.viewport);
        builder.open(
            NodeKind::Container,
            self.layout.viewport,
            "Unbound UI Gallery",
        );
        // 标题栏。
        builder.open(NodeKind::Container, self.layout.top_bar, "TitleBar");
        builder.open(
            interactive_rect(self.layout.nav_toggle),
            self.layout.nav_toggle,
            "Button \"Toggle navigation\"",
        );
        builder.close();
        text_node(
            &mut builder,
            Rect::new(52.0, 14.0, 190.0, 28.0),
            "Unbound UI Gallery",
        );
        if self.layout.search.size.x > 100.0 {
            builder.open(
                interactive_rect(self.layout.search),
                self.layout.search,
                "SearchBox \"Search controls\"",
            );
            builder.close();
        }
        bundle_controls(&mut builder, &self.layout);
        builder.close();

        // 导航窗格。
        builder.open(NodeKind::Container, self.layout.nav, "NavigationView");
        builder.set_clip(self.layout.nav);
        builder.set_scrollable(true);
        let nav_rect = self.layout.nav;
        let nav_rows = nav_rows(&self.state, &self.layout);
        for (rect, label) in nav_rows {
            // 滚出窗格的行不绘制任何内容；校验其被裁剪为零的目标只会把滚动位置误报为布局缺陷。
            //
            //
            let visible = rect.intersect(&nav_rect);
            if visible.size.y >= 24.0 && visible.size.x >= 24.0 {
                builder.open(interactive_rect(rect), rect, &label);
                builder.close();
            }
        }
        builder.close();

        // 页面内容。
        builder.open(NodeKind::Container, self.layout.content, "Page");
        builder.set_clip(self.layout.content);
        builder.set_scrollable(true);
        if let Some(page) = &self.page_layout {
            let info = page.page.info();
            text_node(
                &mut builder,
                Rect::new(
                    page.header.origin.x,
                    page.header.origin.y,
                    page.header.size.x.min(600.0),
                    34.0,
                ),
                info.title,
            );
            text_node(
                &mut builder,
                Rect::new(
                    page.header.origin.x,
                    page.header.origin.y + 38.0,
                    page.header.size.x.min(720.0),
                    40.0,
                ),
                info.description,
            );
            let content_rect = self.layout.content;
            let visible = |rect: Rect| {
                let clipped = rect.intersect(&content_rect);
                clipped.size.x >= 24.0 && clipped.size.y >= 24.0
            };
            for card in &page.cards {
                if !card.rect.intersects(&content_rect) {
                    continue;
                }
                builder.open(NodeKind::Container, card.rect, "ExampleCard");
                builder.set_clip(card.rect);
                if visible(card.stage) {
                    builder.open(NodeKind::Container, card.stage, "LiveStage");
                    builder.set_clip(card.stage);
                    builder.open(interactive_rect(card.stage), card.stage, "Live preview");
                    builder.close();
                    builder.close();
                }
                if !card.knob_rows.is_empty() {
                    builder.open(NodeKind::Container, card.knobs, "Options");
                    for row in &card.knob_rows {
                        if let Some(label) = row.label {
                            if visible(label) {
                                text_node(&mut builder, label, "Property label");
                            }
                        }
                        if visible(row.control) {
                            builder.open(
                                interactive_rect(row.control),
                                row.control,
                                "Property control",
                            );
                            builder.close();
                        }
                    }
                    builder.close();
                }
                if visible(card.code_bar) {
                    builder.open(
                        interactive_rect(card.code_bar),
                        card.code_bar,
                        "Button \"Code and accessibility\"",
                    );
                    builder.close();
                }
                if let Some(panel) = card.code_panel {
                    if visible(panel) {
                        text_node(&mut builder, panel, "Source code");
                    }
                }
                builder.close();
            }
        }
        builder.close();

        // 传送门（Portals）。
        if self.state.search_focused {
            let overlay = search_overlay_rect(&self.layout);
            builder.open(NodeKind::Portal, overlay, "Command palette");
            builder.set_overlays_siblings(true);
            let results = self.state.search_results();
            let count = results.len().max(1);
            for index in 0..count {
                let row = Rect::new(
                    overlay.origin.x + 6.0,
                    overlay.origin.y + 34.0 + index as f32 * 36.0,
                    overlay.size.x - 12.0,
                    34.0,
                );
                builder.open(interactive_rect(row), row, "Search result");
                builder.close();
            }
            builder.close();
        }
        if self.state.demo.dropdown_open || self.state.demo.modal_open {
            let overlay = self
                .page_layout
                .as_ref()
                .and_then(|page| page.cards.first())
                .map_or(self.layout.content, |card| card.stage);
            builder.open(NodeKind::Portal, overlay, "Overlay");
            builder.set_overlays_siblings(true);
            builder.open(
                interactive_rect(overlay.inflate(-24.0)),
                overlay.inflate(-24.0),
                "Overlay action",
            );
            builder.close();
            builder.close();
        }
        if self.state.show_hud {
            if let Some(rect) = hud_rect(&self.layout) {
                builder.open(NodeKind::Portal, rect, "Stats HUD");
                builder.set_overlays_siblings(true);
                let (foreground, background) = hud_colors(self.state.theme.is_dark());
                let hud_text = |builder: &mut FrameBuilder, rect: Rect, text: &str| {
                    text_node(builder, rect, text);
                    builder.set_text_colors(foreground, background);
                };
                hud_text(
                    &mut builder,
                    Rect::new(
                        rect.origin.x + 12.0,
                        rect.origin.y + 8.0,
                        rect.size.x - 24.0,
                        14.0,
                    ),
                    "LIVE FRAME",
                );
                for index in 0..5 {
                    hud_text(
                        &mut builder,
                        Rect::new(
                            rect.origin.x + 12.0,
                            rect.origin.y + 26.0 + index as f32 * 15.0,
                            rect.size.x - 24.0,
                            14.0,
                        ),
                        "HUD metric",
                    );
                }
                builder.close();
            }
        }
        builder.close();
        builder.build()
    }
}

fn knob_spec(page: u8, card: u8, knob: u8) -> Option<KnobSpec> {
    PageId::from_index(page as usize)
        .examples()
        .get(card as usize)
        .and_then(|example| example.knobs.get(knob as usize))
        .copied()
}

fn interactive_rect(_rect: Rect) -> NodeKind {
    NodeKind::Interactive {
        min_size: NodeKind::DEFAULT_MIN_SIZE,
        occluded: false,
    }
}

fn text_node(builder: &mut FrameBuilder, rect: Rect, text: &str) {
    builder.open(
        NodeKind::Text {
            ink: rect,
            ellipsis: true,
        },
        rect,
        &format!("Text \"{text}\""),
    );
    builder.close();
}

fn bundle_controls(builder: &mut FrameBuilder, layout: &ShellLayout) {
    for (rect, label) in [
        (layout.theme, "Theme selector"),
        (layout.zoom, "Zoom selector"),
        (layout.snap, "Button \"Pixel snapping\""),
        (layout.hud, "Button \"Performance HUD\""),
        (layout.window_minimize, "Button \"Minimise window\""),
        (layout.window_maximize, "Button \"Maximise window\""),
        (layout.window_close, "Button \"Close window\""),
    ] {
        if rect.size.x >= 24.0 {
            builder.open(interactive_rect(rect), rect, label);
            builder.close();
        }
    }
}

fn nav_rows(state: &GalleryState, layout: &ShellLayout) -> Vec<(Rect, String)> {
    let mut rows = Vec::new();
    if layout.nav_width < 120.0 {
        let mut y = layout.nav.origin.y + 10.0;
        for page in [
            PageId::Home,
            PageId::Colors,
            PageId::Buttons,
            PageId::Flex,
            PageId::ListView,
            PageId::Viewport,
            PageId::Springs,
            PageId::Telemetry,
            PageId::Settings,
        ] {
            rows.push((
                Rect::new(layout.nav.origin.x + 12.0, y, layout.nav_width - 24.0, 36.0),
                format!("Navigation \"{}\"", page.info().title),
            ));
            y += 42.0;
        }
        let _ = state;
        return rows;
    }
    let mut y = layout.nav.origin.y + 10.0;
    rows.push((
        Rect::new(layout.nav.origin.x + 10.0, y, layout.nav_width - 20.0, 36.0),
        "Navigation \"Home\"".to_string(),
    ));
    // 树从 Home 行及其分隔符下方开始——与 `nav_tree_rect` 使用相同的偏移。
    //
    y = layout.nav.origin.y + 56.0;
    for category in CategoryId::ALL {
        y += 24.0;
        for page in category.pages() {
            rows.push((
                Rect::new(layout.nav.origin.x + 10.0, y, layout.nav_width - 20.0, 30.0),
                format!("Navigation \"{}\"", page.info().short),
            ));
            y += 30.0;
        }
        y += 8.0;
    }
    let _ = state;
    rows
}

fn demo_rect(app: &GalleryApp, page: PageId, card: u8, part: u16) -> Option<Rect> {
    let node = target::id(Target::Demo(page.index() as u8, card, part));
    node_rect_from_regions(app, node)
}

fn node_rect_from_regions(app: &GalleryApp, node: u64) -> Option<Rect> {
    app.regions
        .iter()
        .find(|region| region.node == node)
        .map(|region| region.bounds)
}

// ---------------------------------------------------------------------------
// 布局
// ---------------------------------------------------------------------------

fn nav_tree_rect(layout: &ShellLayout) -> Rect {
    let top = layout.nav.origin.y + 56.0;
    let bottom = layout.viewport.max().y - 10.0 - 82.0;
    Rect::new(
        layout.nav.origin.x,
        top,
        layout.nav.width(),
        (bottom - top).max(0.0),
    )
}

fn nav_height(layout: &ShellLayout) -> f32 {
    if layout.nav_width < 140.0 {
        return 9.0 * 42.0 + 10.0;
    }
    let mut height = 10.0 + 38.0 + 8.0;
    for category in CategoryId::ALL {
        height += 24.0 + category.pages().len() as f32 * 30.0 + 8.0;
    }
    height + 90.0
}

fn search_overlay_rect(layout: &ShellLayout) -> Rect {
    let width = layout.search.size.x.max(380.0);
    let x = layout
        .search
        .origin
        .x
        .min(layout.viewport.max().x - width - 14.0)
        .max(14.0);
    let results = 4.0;
    Rect::new(
        x,
        layout.search.max().y + 6.0,
        width,
        34.0 + results * 36.0 + 8.0,
    )
}

// ---------------------------------------------------------------------------
// 绘制
// ---------------------------------------------------------------------------

fn draw_background(canvas: &mut Canvas<'_>, layout: &ShellLayout) {
    let palette = *canvas.palette;
    canvas.rect(layout.viewport, 0.0, palette.background, 0, 0.0, 0.0);
}

fn draw_top_bar(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    layout: &ShellLayout,
    window_maximized: bool,
) {
    let palette = *canvas.palette;
    canvas.rect(layout.top_bar, 0.0, palette.header_bg, 0, 0.0, 0.0);
    canvas.divider(0.0, layout.top_bar.max().y - 1.0, layout.top_bar.size.x);

    widgets::icon_button(
        canvas,
        target::id(Target::NavToggle),
        layout.nav_toggle,
        Icon::Menu,
        "Toggle navigation",
        !state.nav_expanded,
    );

    let logo = Rect::new(52.0, 14.0, 28.0, 28.0);
    canvas.rect(logo, 8.0, palette.primary, 0, 0.0, 4.0);
    canvas.text(
        "U",
        logo,
        15.0,
        palette.on_primary,
        AlignX::Center,
        AlignY::Center,
    );
    // 标题在可能与主题控件碰撞前让位：在 200% 缩放下逻辑视口仅约 512 px 宽，
    // 标题撞进分段控件里比没有标题更糟糕。
    //
    if layout.theme.origin.x > 292.0 {
        canvas.text(
            "Unbound UI Gallery",
            Rect::new(88.0, 12.0, 190.0, 32.0),
            13.5,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
    }

    if layout.wide {
        let info = state.active_page.info();
        let category = state.active_page.category().label();
        let crumb = if category == info.short {
            info.short.to_string()
        } else {
            format!("{category}  ›  {}", info.short)
        };
        canvas.text(
            &crumb,
            Rect::new(220.0, 12.0, layout.theme.origin.x - 240.0, 32.0),
            10.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
    }

    let search = layout.search;
    if search.size.x >= 100.0 {
        canvas.rect(
            search,
            8.0,
            palette.card_bg,
            if state.search_focused {
                palette.primary
            } else {
                palette.card_border
            },
            if state.search_focused { 2.0 } else { 1.0 },
            0.0,
        );
        widgets::icon(
            canvas,
            Icon::Search,
            Rect::new(search.origin.x + 4.0, search.origin.y, 28.0, search.size.y),
            palette.text_muted,
            palette.card_bg,
        );
        let preedit = if state.search_focused {
            state.ime_preedit.as_str()
        } else {
            ""
        };
        let composed;
        let placeholder = if state.search_query.is_empty() {
            if search.size.x < 280.0 {
                "Search…"
            } else {
                "Search controls, crates, features…"
            }
        } else if preedit.is_empty() {
            state.search_query.as_str()
        } else {
            composed = format!("{}{}", state.search_query, preedit);
            composed.as_str()
        };
        let ink = canvas.text(
            placeholder,
            Rect::new(
                search.origin.x + 32.0,
                search.origin.y,
                search.size.x - 84.0,
                search.size.y,
            ),
            11.0,
            if state.search_query.is_empty() {
                palette.text_muted
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
        // IME 组合文本带下划线，与所有原生文本框的展示方式一致。
        //
        if !preedit.is_empty() {
            let committed = canvas.labels.measure(&state.search_query, 11.0).x;
            let width = canvas.labels.measure(preedit, 11.0).x;
            canvas.rect(
                Rect::new(ink.origin.x + committed, ink.max().y - 1.5, width, 1.0),
                0.5,
                palette.primary,
                0,
                0.0,
                0.0,
            );
        }
        if layout.search.size.x > 280.0 {
            widgets::badge(
                canvas,
                Rect::new(search.max().x - 52.0, search.origin.y + 7.0, 44.0, 18.0),
                "Ctrl K",
                palette.text_muted,
                palette.stage_bg,
            );
        }
        canvas.control(
            target::id(Target::Search),
            search,
            Role::Search,
            "Search controls, crates and features",
            CursorKind::Text,
            false,
            state.search_focused,
        );
    } else if search.size.x > 0.0 {
        // 窄窗口保留放大镜作为打开同一命令面板的按钮。
        // 若完全丢弃搜索，150–200% 缩放下将完全没有指针入口。
        //
        widgets::icon_button(
            canvas,
            target::id(Target::Search),
            search,
            Icon::Search,
            "Search controls, crates and features",
            state.search_focused,
        );
    }

    // 主题。
    if layout.compact {
        widgets::icon_button(
            canvas,
            target::id(Target::Theme(if state.theme.is_dark() { 0 } else { 1 })),
            layout.theme,
            if state.theme.is_dark() {
                Icon::Sun
            } else {
                Icon::Moon
            },
            "Toggle theme",
            false,
        );
    } else {
        let ids = [target::id(Target::Theme(0)), target::id(Target::Theme(1))];
        widgets::segmented(
            canvas,
            &ids,
            layout.theme,
            &["Light", "Dark"],
            if state.theme.is_dark() { 1 } else { 0 },
            false,
            None,
        );
    }

    // 缩放。
    let labels: [&str; 4] = if layout.zoom.size.x >= 170.0 {
        ["100%", "125%", "150%", "200%"]
    } else {
        ["1×", "1.25×", "1.5×", "2×"]
    };
    let zoom_ids = [
        target::id(Target::Zoom(0)),
        target::id(Target::Zoom(1)),
        target::id(Target::Zoom(2)),
        target::id(Target::Zoom(3)),
    ];
    widgets::segmented(
        canvas,
        &zoom_ids,
        layout.zoom,
        &labels,
        state.zoom_step,
        false,
        None,
    );

    widgets::icon_button(
        canvas,
        target::id(Target::Snap),
        layout.snap,
        Icon::Snap,
        "Pixel snapping",
        state.pixel_snap,
    );
    widgets::icon_button(
        canvas,
        target::id(Target::Hud),
        layout.hud,
        Icon::Chart,
        "Performance HUD",
        state.show_hud,
    );

    // 客户端窗口按钮。
    draw_window_controls(canvas, layout, window_maximized);
}

fn draw_window_controls(canvas: &mut Canvas<'_>, layout: &ShellLayout, canvas_maximized: bool) {
    let palette = *canvas.palette;
    let buttons = [
        (
            Target::WindowMinimize,
            layout.window_minimize,
            "Minimise window",
            false,
        ),
        (
            Target::WindowMaximize,
            layout.window_maximize,
            "Maximise window",
            false,
        ),
        (
            Target::WindowClose,
            layout.window_close,
            "Close window",
            true,
        ),
    ];
    for (target, rect, label, destructive) in buttons {
        let node = target::id(target);
        let start = canvas.span_start();
        let hovered = canvas.hovered(node);
        let pressed = canvas.pressed(node);
        // 背景始终存在，仅在悬停时淡入，因此指向窗口按钮仅重绘一个图元，而非插入新图元。
        //
        //
        let fill = if destructive {
            palette.danger
        } else {
            palette.control_hover
        };
        canvas.rect_faded(rect, 0.0, fill, if hovered || pressed { 1.0 } else { 0.0 });
        let color = if destructive && hovered {
            palette.on_hero
        } else {
            palette.text
        };
        let middle = rect.center();
        match target {
            Target::WindowMinimize => {
                // 垂直居中的 10 × 1 逻辑像素横条。描边四舍五入为整物理像素并对齐像素网格，
                // 因此在 100%、125%、150% 与 200% 下均保持一像素粗细
                // （旧的 `middle.y + 4.0` 落在图标框底边上，在小数缩放下粗细会翻倍）。
                //
                //
                let scale = canvas.scale().max(0.5);
                let stroke = scale.round().max(1.0) / scale;
                let top = (middle.y * scale - stroke * scale * 0.5).round() / scale;
                canvas.rect(
                    Rect::new(middle.x - 5.0, top, 10.0, stroke),
                    0.0,
                    color,
                    0,
                    0.0,
                    0.0,
                );
            }
            Target::WindowMaximize => {
                let middle = rect.center();
                // 两种状态都精确绘制**两个**图元，因此切换只需修补两个切片，
                // 而不会改变场景形状。形状变化会推进上传代次并全量重传场景——
                // 更糟的是，在缓冲池落后于 diff 基准时可能会让丢帧漏洞穿透。
                //
                //
                //
                if canvas_maximized {
                    // 还原图标：两个错位方框。前方方框用图标背后的颜色填充，
                    // 否则后方方框的边线会穿过其内部，使一对图形看起来像井字棋网格。
                    //
                    //
                    let occluder = if hovered || pressed {
                        palette.control_hover
                    } else {
                        palette.header_bg
                    };
                    canvas.rect(
                        Rect::new(middle.x - 3.0, middle.y - 5.0, 8.0, 8.0),
                        0.0,
                        0,
                        color,
                        1.0,
                        0.0,
                    );
                    canvas.rect(
                        Rect::new(middle.x - 5.0, middle.y - 3.0, 8.0, 8.0),
                        0.0,
                        occluder,
                        color,
                        1.0,
                        0.0,
                    );
                } else {
                    // 最大化图标：一个描边方框，外加一个充当还原图标第二个方框的透明填充。
                    // 该填充无颜色且不透明度为零，因此不绘制任何像素，但将图元数保持为两个。
                    //
                    //
                    let square = Rect::new(middle.x - 5.0, middle.y - 5.0, 10.0, 10.0);
                    canvas.rect_faded(square, 0.0, 0, 0.0);
                    canvas.rect(square, 0.0, 0, color, 1.0, 0.0);
                }
            }
            Target::WindowClose => {
                // 两条解析描边，而非 "×" 字形（U+00D7）。该字形位于字体的数学轴上且墨水仅占
                // em 框的一小部分，在 200% 下小了 40% 且偏低 4 px。
                // 解析描边精确相交于按钮中心，并由 SDF 进行抗锯齿。
                //
                //
                let half = 4.5;
                let mid = [middle.x, middle.y];
                canvas.curve(
                    [middle.x - half, middle.y - half],
                    mid,
                    [middle.x + half, middle.y + half],
                    1.25,
                    color,
                );
                canvas.curve(
                    [middle.x + half, middle.y - half],
                    mid,
                    [middle.x - half, middle.y + half],
                    1.25,
                    color,
                );
            }
            _ => {}
        }
        canvas.control(
            node,
            rect,
            Role::Button,
            label,
            CursorKind::Default,
            false,
            false,
        );
        canvas.record_span(node, start);
    }
}

fn draw_nav(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout) {
    let palette = *canvas.palette;
    canvas.rect(layout.nav, 0.0, palette.nav_bg, 0, 0.0, 0.0);
    canvas.divider(layout.nav.max().x - 1.0, layout.nav.origin.y, 1.0);

    // 弹簧仍在运动时即切换内容：展开的标签会被收缩中的窗格裁剪，
    // 因此提前回退到图标栏。
    if layout.nav_width < 140.0 {
        draw_collapsed_nav(canvas, state, layout);
        return;
    }

    let mut y = layout.nav.origin.y + 10.0;
    nav_item(
        canvas,
        layout,
        Rect::new(layout.nav.origin.x + 10.0, y, layout.nav_width - 20.0, 36.0),
        "Home",
        Icon::Grid,
        state.active_page == PageId::Home,
        Target::NavPage(PageId::Home.index() as u8),
    );
    y += 38.0;
    canvas.divider(layout.nav.origin.x + 14.0, y + 6.0, layout.nav_width - 28.0);

    let tree = nav_tree_rect(layout);
    canvas.begin_clip(tree);
    let mut tree_y = tree.origin.y - state.nav_scroll;
    for category in CategoryId::ALL {
        let active = state.active_page.category() == category;
        canvas.text(
            &category.label().to_uppercase(),
            Rect::new(
                layout.nav.origin.x + 18.0,
                tree_y,
                layout.nav_width - 36.0,
                22.0,
            ),
            9.5,
            if active {
                palette.primary
            } else {
                palette.nav_text_muted
            },
            AlignX::Start,
            AlignY::Center,
        );
        tree_y += 24.0;
        for page in category.pages() {
            nav_item(
                canvas,
                layout,
                Rect::new(
                    layout.nav.origin.x + 10.0,
                    tree_y,
                    layout.nav_width - 20.0,
                    30.0,
                ),
                page.info().short,
                Icon::ChevronRight,
                state.active_page == *page,
                Target::NavPage(page.index() as u8),
            );
            tree_y += 30.0;
        }
        tree_y += 8.0;
    }
    canvas.end_clip();

    let footer_y = layout.viewport.max().y - 90.0;
    canvas.divider(
        layout.nav.origin.x + 14.0,
        footer_y,
        layout.nav_width - 28.0,
    );
    nav_item(
        canvas,
        layout,
        Rect::new(
            layout.nav.origin.x + 10.0,
            footer_y + 8.0,
            layout.nav_width - 20.0,
            34.0,
        ),
        "Engine Telemetry",
        Icon::Chart,
        state.active_page == PageId::Telemetry,
        // 页脚快捷方式需要独立标识：同一页面的树行已拥有 `NavPage(Telemetry)`，
        // 若共享该标识会导致悬停任一行时两行同时高亮。
        //
        Target::Footer(0),
    );
    nav_item(
        canvas,
        layout,
        Rect::new(
            layout.nav.origin.x + 10.0,
            footer_y + 44.0,
            layout.nav_width - 20.0,
            34.0,
        ),
        "Settings & About",
        Icon::Layers,
        state.active_page == PageId::Settings,
        Target::Footer(1),
    );

    // 当窗口较矮或应用缩放较高时导航树会滚动，过去它是静默滚动的：
    // 仅内容窗格带有指示条，导致被裁剪的 “Layout & portals” 分区看起来像页面丢失。
    //
    draw_nav_scrollbar(canvas, state, layout);
}

/// 绘制导航树的滚动指示条。
fn draw_nav_scrollbar(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout) {
    let Some(bar) = nav_scrollbar(layout) else {
        return;
    };
    let palette = *canvas.palette;
    canvas.rect(bar.track, 3.0, palette.control_track, 0, 0.0, 0.0);
    let thumb = Rect::new(
        bar.track.origin.x,
        bar.thumb_top(state.nav_scroll),
        6.0,
        bar.thumb_h,
    );
    canvas.rect(thumb, 3.0, palette.primary, 0, 0.0, 0.0);
    // 导航滚动条是真实控件而非装饰：14px 热区允许拖拽滑块与点击轨道，
    // 与页面滚动条完全一致。
    //
    canvas.control(
        target::id(Target::NavScrollbar),
        Rect::new(
            bar.track.origin.x - 4.0,
            bar.track.origin.y,
            14.0,
            bar.track.size.y,
        ),
        Role::ScrollBar,
        "Scroll navigation",
        CursorKind::Pointer,
        false,
        false,
    );
}

fn nav_item(
    canvas: &mut Canvas<'_>,
    layout: &ShellLayout,
    rect: Rect,
    label: &str,
    icon: Icon,
    selected: bool,
    target: Target,
) {
    let palette = *canvas.palette;
    let node = target::id(target);
    let start = canvas.span_start();
    let hover = canvas.visual(node, false);
    // 无论处于何种状态，该行始终绘制两个图元——胶囊与选中指示条。
    // 空闲时胶囊采用窗格自身颜色，未选中时指示条淡出，
    // 因此悬停行只需重绘两个实例，而无需重新编号缓冲池。
    //
    let background = if selected {
        palette.nav_selected
    } else if hover.is_hovered() {
        palette.nav_hover
    } else {
        palette.nav_bg
    };
    canvas.rect(rect, 8.0, background, 0, 0.0, 0.0);
    // 选中指示条位于胶囊*内部*。紧贴窗口边缘时它看起来像渲染伪影，
    // 而非该行的一部分。
    //
    canvas.rect_faded(
        Rect::new(
            rect.origin.x + 4.0,
            rect.origin.y + (rect.size.y - 16.0) * 0.5,
            3.0,
            16.0,
        ),
        1.5,
        palette.primary,
        if selected { 1.0 } else { 0.0 },
    );
    widgets::icon(
        canvas,
        icon,
        Rect::new(rect.origin.x + 14.0, rect.origin.y, 20.0, rect.size.y),
        if selected {
            palette.primary
        } else {
            palette.nav_text_muted
        },
        palette.nav_bg,
    );
    canvas.text(
        label,
        Rect::new(
            rect.origin.x + 40.0,
            rect.origin.y,
            rect.size.x - 46.0,
            rect.size.y,
        ),
        12.0,
        if selected {
            palette.nav_text
        } else {
            palette.nav_text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
    let _ = layout;
    canvas.control(
        node,
        rect,
        Role::Link,
        label,
        CursorKind::Pointer,
        false,
        selected,
    );
    canvas.record_span(node, start);
}

fn draw_collapsed_nav(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout) {
    let _ = state;
    let items = [
        (PageId::Home, Icon::Grid),
        (PageId::Colors, Icon::Layers),
        (PageId::Buttons, Icon::Snap),
        (PageId::Flex, Icon::Grid),
        (PageId::ListView, Icon::Chart),
        (PageId::Viewport, Icon::Snap),
        (PageId::Springs, Icon::Layers),
        (PageId::Telemetry, Icon::Chart),
        (PageId::Settings, Icon::Menu),
    ];
    let mut y = layout.nav.origin.y + 10.0;
    for (page, icon) in items {
        let rect = Rect::new(layout.nav.origin.x + 14.0, y, layout.nav_width - 28.0, 36.0);
        let selected = state.active_page == page
            || (page == PageId::Colors && state.active_page.category() == CategoryId::Design);
        widgets::icon_button(
            canvas,
            target::id(Target::NavPage(page.index() as u8)),
            rect,
            icon,
            page.info().short,
            selected,
        );
        y += 42.0;
    }
    // 折叠切换按钮上方的细线使图标栏与页面内容在视觉上保持分隔。
    //
    canvas.divider(
        layout.nav.origin.x + 10.0,
        layout.nav.max().y - 12.0,
        layout.nav_width - 20.0,
    );
}

fn draw_page(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    layout: &ShellLayout,
    page: &PageLayout,
) {
    let palette = *canvas.palette;
    let info = page.page.info();
    canvas.begin_clip(layout.content);
    canvas.semantic(
        Role::Heading,
        info.title,
        Rect::new(
            page.header.origin.x,
            page.header.origin.y,
            page.header.size.x,
            34.0,
        ),
    );
    canvas.text(
        info.title,
        Rect::new(
            page.header.origin.x,
            page.header.origin.y,
            page.header.size.x,
            38.0,
        ),
        30.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    canvas.text_wrapped(
        info.description,
        Rect::new(
            page.header.origin.x,
            page.header.origin.y + 42.0,
            (page.header.size.x * 0.74).min(820.0),
            46.0,
        ),
        12.5,
        palette.text_muted,
    );
    let mut badge_x = page.header.origin.x;
    for krate in info.crates {
        let width = canvas.labels.measure(krate, 11.0).x + 16.0;
        widgets::badge(
            canvas,
            Rect::new(badge_x, page.header.origin.y + 92.0, width, 20.0),
            krate,
            palette.primary,
            palette.primary_soft,
        );
        badge_x += width + 6.0;
    }
    canvas.text(
        &format!("docs › {}", info.docs),
        Rect::new(
            page.header.origin.x,
            page.header.origin.y + 124.0,
            page.header.size.x,
            14.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );

    for card in &page.cards {
        if card.rect.max().y < layout.content.origin.y
            || card.rect.origin.y > layout.content.max().y
        {
            continue;
        }
        draw_example(canvas, state, page, card);
    }

    if let Some(bar) = content_scrollbar(layout, page) {
        canvas.rect(bar.track, 3.0, palette.control_track, 0, 0.0, 0.0);
        let thumb_h = bar.thumb_h;
        let t = (state.content_scroll / bar.max_scroll).clamp(0.0, 1.0);
        let thumb = Rect::new(
            bar.track.origin.x,
            bar.track.origin.y + (bar.track.size.y - thumb_h) * t,
            6.0,
            thumb_h,
        );
        canvas.rect(thumb, 3.0, palette.primary, 0, 0.0, 0.0);
        // 整条轨道都是控件：滑块可拖拽，点击轨道可使滑块跳转到该处。
        // 14px 热区比 6px 滚动条更宽以便抓取，且停留在窗口内部，
        // 避免宿主的调整大小边框抢走按压事件。
        //
        canvas.control(
            target::id(Target::ContentScrollbar),
            Rect::new(
                bar.track.origin.x - 4.0,
                bar.track.origin.y,
                14.0,
                bar.track.size.y,
            ),
            Role::ScrollBar,
            "Scroll page",
            CursorKind::Pointer,
            false,
            false,
        );
    }
    canvas.end_clip();
}

fn draw_example(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    page: &PageLayout,
    card: &CardLayout,
) {
    let palette = *canvas.palette;
    let example = &page.page.examples()[card.index];
    canvas.rect(
        card.rect,
        14.0,
        palette.card_bg,
        palette.card_border,
        1.0,
        4.0,
    );
    canvas.begin_clip(card.rect);
    canvas.text(
        example.title,
        Rect::new(
            card.rect.origin.x + 18.0,
            card.rect.origin.y + 12.0,
            card.rect.size.x - 36.0,
            24.0,
        ),
        15.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    canvas.text_wrapped(
        example.description,
        Rect::new(
            card.rect.origin.x + 18.0,
            card.rect.origin.y + 38.0,
            card.rect.size.x - 36.0,
            30.0,
        ),
        10.5,
        palette.text_muted,
    );

    let mut badge_x = card.rect.max().x - 18.0;
    let badge_limit = card.rect.origin.x + 170.0;
    for krate in example.crates.iter().rev() {
        let width = canvas.labels.measure(krate, 9.5).x + 16.0;
        if badge_x - width < badge_limit {
            break;
        }
        badge_x -= width;
        let badge = widgets::badge_sized(
            canvas,
            Rect::new(badge_x, card.rect.origin.y + 14.0, width, 18.0),
            krate,
            palette.text_muted,
            palette.stage_bg,
            9.5,
        );
        badge_x = badge.origin.x - 6.0;
    }

    // 实时舞台。
    canvas.rect(
        card.stage,
        10.0,
        palette.stage_bg,
        palette.stage_grid,
        1.0,
        0.0,
    );
    canvas.begin_clip_rounded(card.stage, 10.0);
    let demo_rect = card.stage.inflate(-10.0);
    let knobs = state.knob_values(page.page, card.index, example);
    {
        let mut ctx = DemoCtx {
            canvas: &mut *canvas,
            state,
            page: page.page,
            card: card.index,
            rect: demo_rect,
            knobs: &knobs,
        };
        pages::render_demo(&mut ctx, example.demo);
    }
    canvas.end_clip();

    if !example.knobs.is_empty() {
        canvas.divider(card.knobs.origin.x - 9.0, card.knobs.origin.y, 1.0);
        canvas.text(
            "OPTIONS",
            Rect::new(
                card.knobs.origin.x,
                card.knobs.origin.y,
                card.knobs.size.x,
                18.0,
            ),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
        for row in &card.knob_rows {
            draw_knob_row(canvas, state, page.page, card.index, example, row, palette);
        }
    }

    draw_code_bar(canvas, state, page, card, example);
    canvas.end_clip();
}

fn draw_knob_row(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    page: PageId,
    card: usize,
    example: &crate::pages::ExampleDef,
    row: &KnobRow,
    palette: Palette,
) {
    let spec = example.knobs[row.index];
    let value = state.knob(page, card, row.index, &spec);
    let node = target::id(Target::Knob(
        page.index() as u8,
        card as u8,
        row.index as u8,
    ));
    match spec {
        KnobSpec::Toggle { label, .. } => {
            widgets::toggle_switch(canvas, node, row.control, label, value > 0.5, false);
        }
        KnobSpec::CheckBox { label, .. } => {
            widgets::checkbox(canvas, node, row.control, label, value > 0.5, false);
        }
        KnobSpec::Slider {
            label,
            min,
            max,
            step,
            unit,
            ..
        } => {
            canvas.text(
                label,
                row.label.unwrap_or(row.row),
                9.5,
                palette.text_muted,
                AlignX::Start,
                AlignY::Top,
            );
            let shown = if step < 1.0 {
                format!("{value:.1}{unit}")
            } else {
                format!("{value:.0}{unit}")
            };
            canvas.text(
                &shown,
                Rect::new(row.row.origin.x, row.row.origin.y, row.row.size.x, 16.0),
                9.5,
                palette.primary,
                AlignX::End,
                AlignY::Top,
            );
            widgets::slider(canvas, node, row.control, value, min, max);
            canvas.range(node, row.control, min, max, step);
        }
        KnobSpec::Segmented {
            label,
            options,
            default_index,
        } => {
            canvas.text(
                label,
                row.label.unwrap_or(row.row),
                9.5,
                palette.text_muted,
                AlignX::Start,
                AlignY::Top,
            );
            let ids: Vec<u64> = (0..options.len())
                .map(|option| {
                    target::id(Target::KnobOption(
                        page.index() as u8,
                        card as u8,
                        row.index as u8,
                        option as u8,
                    ))
                })
                .collect();
            let selected = (value as usize).min(options.len().saturating_sub(1));
            let _ = default_index;
            widgets::segmented(canvas, &ids, row.control, options, selected, false, None);
        }
    }
}

fn draw_code_bar(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    page: &PageLayout,
    card: &CardLayout,
    example: &crate::pages::ExampleDef,
) {
    let palette = *canvas.palette;
    let node = target::id(Target::Code(page.page.index() as u8, card.index as u8));
    let start = canvas.span_start();
    let hover = canvas.visual(node, false);
    let open = state.code_open(page.page, card.index as u8);
    // 悬停底色始终存在且仅淡入显示，因此指向代码栏只需重绘一个图元，而非插入新图元。
    //
    canvas.rect_faded(
        card.code_bar,
        6.0,
        palette.control_hover,
        if hover.is_hovered() { 1.0 } else { 0.0 },
    );
    canvas.divider(
        card.code_bar.origin.x,
        card.code_bar.origin.y,
        card.code_bar.size.x,
    );
    canvas.text(
        "</>",
        Rect::new(
            card.code_bar.origin.x + 6.0,
            card.code_bar.origin.y,
            28.0,
            card.code_bar.size.y,
        ),
        10.0,
        palette.primary,
        AlignX::Start,
        AlignY::Center,
    );
    canvas.text(
        "Rust · ABI · accessibility",
        Rect::new(
            card.code_bar.origin.x + 40.0,
            card.code_bar.origin.y,
            card.code_bar.size.x - 80.0,
            card.code_bar.size.y,
        ),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
    widgets::icon(
        canvas,
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        Rect::new(
            card.code_bar.max().x - 28.0,
            card.code_bar.origin.y,
            20.0,
            card.code_bar.size.y,
        ),
        palette.text_muted,
        palette.card_bg,
    );
    canvas.control(
        node,
        card.code_bar,
        Role::Button,
        "Code and accessibility",
        CursorKind::Pointer,
        false,
        open,
    );
    // 跨度在代码栏处结束：其下方的新面板属于卡片，且无论如何仅在卡片形状变化时出现。
    //
    canvas.record_span(node, start);

    if let Some(panel) = card.code_panel {
        canvas.rect(panel, 10.0, palette.code_bg, 0, 0.0, 0.0);
        canvas.begin_clip(panel);
        canvas.text(
            "SOURCE",
            Rect::new(
                panel.origin.x + 12.0,
                panel.origin.y + 5.0,
                panel.size.x - 24.0,
                14.0,
            ),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
        let mut y = panel.origin.y + 20.0;
        for (line_index, line) in example.code.lines().take(12).enumerate() {
            canvas.text(
                &format!("{:>2}", line_index + 1),
                Rect::new(panel.origin.x + 12.0, y, 20.0, 14.0),
                9.0,
                palette.text_muted,
                AlignX::End,
                AlignY::Top,
            );
            canvas.text(
                line,
                Rect::new(panel.origin.x + 38.0, y, panel.size.x - 50.0, 14.0),
                9.5,
                palette.code_text,
                AlignX::Start,
                AlignY::Top,
            );
            y += 15.0;
        }
        canvas.text(
            &format!("a11y · {}", example.a11y),
            Rect::new(
                panel.origin.x + 12.0,
                panel.max().y - 20.0,
                panel.size.x - 24.0,
                14.0,
            ),
            9.0,
            palette.primary,
            AlignX::Start,
            AlignY::Top,
        );
        canvas.end_clip();
    }
}

fn hud_rect(layout: &ShellLayout) -> Option<Rect> {
    if layout.content.size.x < 560.0 {
        return None;
    }
    Some(Rect::new(
        layout.content.max().x - 232.0,
        layout.content.max().y - 168.0,
        218.0,
        148.0,
    ))
}

fn hud_colors(dark: bool) -> ([u8; 3], [u8; 3]) {
    if dark {
        ([220, 227, 242], [8, 10, 14])
    } else {
        ([28, 30, 38], [255, 255, 255])
    }
}

fn draw_hud(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout, cards: usize) {
    if !state.show_hud {
        return;
    }
    let Some(rect) = hud_rect(layout) else {
        return;
    };
    let palette = *canvas.palette;
    // 将遥测浮窗停靠在右下角：右上角区域归页面标题及其 crate 胶囊所有，
    // 放在那里的 HUD 在任何窗口尺寸下都会遮挡它们。
    //
    canvas.rect(rect, 10.0, palette.hud_bg, palette.card_border, 1.0, 18.0);
    canvas.text(
        "LIVE FRAME",
        Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y + 8.0,
            rect.size.x - 24.0,
            14.0,
        ),
        9.5,
        palette.hud_text,
        AlignX::Start,
        AlignY::Top,
    );
    let stats = &state.demo;
    let prims = canvas.scene.primitives.len();
    let glyphs = canvas.glyph_count();
    let rows = [
        ("FRAME", format!("{:.2} ms", stats.frame_ms)),
        ("FPS", format!("{:.0}", stats.fps)),
        ("PRIMS", format!("{prims}")),
        ("GLYPHS", format!("{glyphs}")),
        ("CARDS", format!("{cards}")),
        (
            "LIMIT",
            if state.fps_limit == 0 {
                "uncapped".to_string()
            } else {
                format!("{} fps", state.fps_limit)
            },
        ),
    ];
    for (index, (label, value)) in rows.iter().enumerate() {
        let y = rect.origin.y + 26.0 + index as f32 * 15.0;
        canvas.text(
            label,
            Rect::new(rect.origin.x + 12.0, y, 70.0, 14.0),
            9.5,
            palette.hud_text,
            AlignX::Start,
            AlignY::Center,
        );
        canvas.text(
            value,
            Rect::new(rect.max().x - 100.0, y, 88.0, 14.0),
            9.5,
            palette.hud_text,
            AlignX::End,
            AlignY::Center,
        );
    }
    // 迷你折线图（Sparkline）。
    let chart = Rect::new(
        rect.origin.x + 12.0,
        rect.max().y - 26.0,
        rect.size.x - 24.0,
        16.0,
    );
    for index in 0..28 {
        let phase = index as f32 / 28.0;
        let value = 0.35 + (phase * 15.0 + stats.elapsed * 3.0).sin() * 0.22;
        let height = chart.size.y * value.clamp(0.08, 0.95);
        canvas.rect(
            Rect::new(
                chart.origin.x + phase * chart.size.x,
                chart.max().y - height,
                3.0,
                height,
            ),
            1.0,
            if index > 24 {
                palette.warning
            } else {
                palette.primary
            },
            0,
            0.0,
            0.0,
        );
    }
}

fn draw_search_overlay(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout) {
    if !state.search_focused {
        return;
    }
    let palette = *canvas.palette;
    let results = state.search_results();
    let panel = search_overlay_rect(layout);
    canvas.rect(
        panel,
        12.0,
        palette.overlay_bg,
        palette.card_border,
        1.0,
        26.0,
    );
    canvas.text(
        if state.search_query.is_empty() {
            "COMMAND PALETTE · CTRL K"
        } else {
            "MATCHES"
        },
        Rect::new(
            panel.origin.x + 14.0,
            panel.origin.y + 8.0,
            panel.size.x - 28.0,
            14.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    if state.search_query.is_empty() {
        canvas.text(
            "Try “Button”, “MTSDF”, “Quadtree”, “DnD”…",
            Rect::new(
                panel.origin.x + 14.0,
                panel.origin.y + 30.0,
                panel.size.x - 28.0,
                32.0,
            ),
            11.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        return;
    }
    if results.is_empty() {
        canvas.text(
            "No controls match that query.",
            Rect::new(
                panel.origin.x + 14.0,
                panel.origin.y + 30.0,
                panel.size.x - 28.0,
                32.0,
            ),
            11.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        return;
    }
    for (index, page) in results.iter().enumerate() {
        let row = Rect::new(
            panel.origin.x + 6.0,
            panel.origin.y + 30.0 + index as f32 * 36.0,
            panel.size.x - 12.0,
            34.0,
        );
        let node = target::id(Target::SearchResult(page.index() as u8));
        let hover = canvas.visual(node, false);
        if hover.is_hovered() || *page == state.active_page {
            canvas.rect(
                row,
                7.0,
                if *page == state.active_page {
                    palette.primary_soft
                } else {
                    palette.control_hover
                },
                0,
                0.0,
                0.0,
            );
        }
        let info = page.info();
        canvas.text(
            info.short,
            Rect::new(
                row.origin.x + 12.0,
                row.origin.y,
                row.size.x * 0.52,
                row.size.y,
            ),
            11.0,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        canvas.text(
            &format!(
                "{} · {}",
                page.category().label(),
                info.crates.first().copied().unwrap_or("unbound")
            ),
            Rect::new(
                row.origin.x + row.size.x * 0.54,
                row.origin.y,
                row.size.x * 0.44 - 12.0,
                row.size.y,
            ),
            9.5,
            palette.text_muted,
            AlignX::End,
            AlignY::Center,
        );
        canvas.control_on_layer(
            node,
            row,
            Role::Link,
            info.title,
            CursorKind::Pointer,
            false,
            *page == state.active_page,
            2,
        );
    }
}

// ---------------------------------------------------------------------------
// 上传
// ---------------------------------------------------------------------------

/// 两个场景之间的 diff。
///
/// 控件层拥有该算法；外壳仅知晓其场景的*形状*，即图元所索引的各个表。
///
fn diff_scenes(previous: &gallery::Gallery, current: &gallery::Gallery) -> SceneDiff {
    let structure_changed = previous.transforms != current.transforms
        || previous.clips != current.clips
        || previous.payloads != current.payloads;
    diff_primitives(&previous.primitives, &current.primitives, structure_changed)
}

/// 将 `incoming` 合并到 `pending` 中，合并重叠或相邻的区间。
///
/// 上传路径为每个范围暂存一段连续切片，若仅做每次重建 diff 的并集，
/// 可能会把同一个图元暂存两次。
fn merge_ranges(pending: &mut Vec<(u32, u32)>, incoming: Vec<(u32, u32)>) {
    pending.extend(incoming);
    pending.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(pending.len());
    for (start, end) in pending.drain(..) {
        if start >= end {
            continue;
        }
        match merged.last_mut() {
            Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
            _ => merged.push((start, end)),
        }
    }
    *pending = merged;
}
