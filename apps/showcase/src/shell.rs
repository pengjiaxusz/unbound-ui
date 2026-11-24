//! 自举（dogfooded）的应用外壳。
//!
//! `GalleryApp` 拥有导航、环境开关与演示状态。它绘制自定义标题栏、
//! 可折叠的 NavigationView、页面视口与 `ExampleCard` 组合，使 showcase
//! 演练真实应用会使用的同一套图元。
//!

use accesskit::Role;
use unbound_anim::CubicBezier;
use unbound_backdrop::{BackdropCombination, BackdropPlan, BackdropSource, MaterialKind};
use unbound_headless::{Frame, FrameBuilder, NodeKind};
use unbound_input::{CursorKind, PointerButton};
use unbound_layout::LayoutStats;
use unbound_math::{Rect, Vec2};
use unbound_platform::{Modifiers, PlatformKey};
use unbound_widgets::curve_editor::{CurveEditorDesc, CurveEditorState, CurveHandle};
use unbound_widgets::diff::{diff_primitives, SceneDiff};
use unbound_widgets::navigation_view::{
    emit_navigation_view, navigation_view_row_node, NavigationViewBackButtonVisible,
    NavigationViewDesc, NavigationViewEvent, NavigationViewItem, NavigationViewLayout,
    NavigationViewMenuItem, NavigationViewPaneDisplayMode, NavigationViewRowKind,
    NavigationViewState, NavigationViewTitleBarSync, NAVIGATION_VIEW_COMPACT_PANE_LENGTH,
};
use unbound_widgets::scroll::{scrollbar_thumb_node, scrollbar_track_node, ScrollbarAxis};
use unbound_widgets::{
    emit_window_caption_buttons, ControlState, ShellIcon, WidgetCatalogId, WidgetOutput,
    WindowCaptionButtonControlStates, WindowCaptionButtons, WindowCaptionButtonsLayout,
    WindowCaptionFrameHitContract,
};

use crate::canvas::{AlignX, AlignY, Canvas, RangeZone};
use crate::gallery::{self, Labels, Palette};
use crate::layout_tree::{CardLayout, KnobRow, PageLayout, ShellLayout, ShellTree};
use unbound_widgets::{ControlCategory, ControlClass};

use crate::pages::{self, CategoryId, DemoCtx, KnobSpec, PageId, PageTemplate};
use crate::state::{EditorId, GalleryState, ThemeMode, ZOOM_STEPS};
use crate::target::{self, Target, ROOT};
use crate::widgets::{self, Icon};

/// `NavigationView` 展开窗格宽度（与展厅壳层既有设计一致）。
const NAV_OPEN_PANE_LENGTH: f32 = 248.0;
/// `NavigationView` 控件的稳定节点标识（高位标签避开 `target.rs` 的 1..=26）。
const NAV_VIEW_NODE: u64 = 94u64 << 56;
/// `NavigationView` 垂直滚动条滑块子命中节点标识。
pub const NAV_SCROLLBAR_THUMB_NODE: u64 =
    scrollbar_thumb_node(NAV_VIEW_NODE, ScrollbarAxis::Vertical);
/// `NavigationView` 垂直滚动条轨道子命中节点标识。
pub const NAV_SCROLLBAR_TRACK_NODE: u64 =
    scrollbar_track_node(NAV_VIEW_NODE, ScrollbarAxis::Vertical);
/// 顶栏与页脚悬停工具提示的稳定节点标识（高位标签避开 `target.rs` 与 `NAV_VIEW_NODE`）。
const TOP_BAR_TOOLTIP_NODE: u64 = 95u64 << 56;
/// 编辑器上下文菜单容器的临时节点标识（子热区会重映射回 `Target::EditorMenuItem`）。
const EDITOR_MENU_NODE: u64 = 96u64 << 56;

/// 过滤出数字字符，最多三位。
fn numeric_only(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_ascii_digit())
        .take(3)
        .collect()
}

/// 标题栏语言下拉菜单的面板最小宽度（绘制与无障碍语义节点共用）。
const LANG_MENU_WIDTH: f32 = 148.0;

/// 标题栏窗口缩放下拉菜单的面板最小宽度（绘制与无障碍语义节点共用）。
const ZOOM_MENU_WIDTH: f32 = 132.0;

/// 编译期完备性门禁：验证侧边栏 100% 覆盖全部 `PageId::ALL`（27 页）、全部 `WidgetCatalogId::ALL`（109 组件）以及 `PageId::Systems` 覆盖全部 21 个引擎子系统。
const _: () = {
    let mut page_idx = 0;
    while page_idx < PageId::ALL.len() {
        let candidate = PageId::ALL[page_idx];
        let mut count = if matches!(candidate, PageId::Home) {
            1
        } else {
            0
        };
        let mut cat_idx = 0;
        while cat_idx < CategoryId::ALL.len() {
            let pages = CategoryId::ALL[cat_idx].pages();
            let mut p = 0;
            while p < pages.len() {
                if pages[p] as u8 == candidate as u8 {
                    count += 1;
                }
                p += 1;
            }
            cat_idx += 1;
        }
        assert!(
            count == 1,
            "PageId missing or duplicated in showcase sidebar"
        );
        page_idx += 1;
    }
    assert!(
        pages::SIDEBAR_COMPONENT_IDS.len() == unbound_widgets::WidgetCatalogId::COUNT,
        "SIDEBAR_COMPONENT_IDS count must equal WidgetCatalogId::COUNT"
    );
    assert!(
        pages::SubsystemId::ALL.len() == pages::SubsystemId::COUNT,
        "SubsystemId::ALL count mismatch"
    );
};

/// 返回导航视图挂载的全部导航目标（含首页、全部控件、10 大功能分类、133 个目录控件、引擎实验室页与页脚快捷项）。
#[must_use]
pub fn sidebar_all_targets() -> Vec<Target> {
    let mut targets = Vec::with_capacity(
        2 + ControlCategory::COUNT + PageId::ALL.len() + pages::SIDEBAR_COMPONENT_IDS.len() + 2,
    );
    targets.push(Target::NavPage(PageId::Home.index() as u8));
    targets.push(Target::NavPage(PageId::Components.index() as u8));
    for cat in ControlCategory::ALL {
        targets.push(Target::NavCategory(cat.index() as u8));
        for comp_id in cat.controls() {
            targets.push(Target::NavComponent(comp_id as u8));
        }
    }
    for category in CategoryId::ALL {
        if category == CategoryId::Components {
            continue;
        }
        for page in category.pages() {
            targets.push(Target::NavPage(page.index() as u8));
        }
    }
    targets.push(Target::Footer(0));
    targets.push(Target::Footer(1));
    targets
}

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
        layout.content.max().x - 12.0,
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

/// 应用程序。
pub struct GalleryApp {
    pub state: GalleryState,
    /// 整窗云母背板（壁纸派生；默认启用，Win11 的层次依赖它）。
    pub mica: unbound_app::MicaBackdrop,
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
    /// 规范导航视图状态机（ADR-0026）：托管 Auto 三档响应、弹簧动效与独立滚动。
    pub nav_view: NavigationViewState,
    /// 最近一帧 `NavigationView` 求解出的几何布局。
    pub nav_layout: NavigationViewLayout,
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
    /// 由点击、导航或无障碍动作引发的焦点请求。
    ///
    /// 运行时拥有焦点管理器，因此外壳仅记录请求并在归约后交出。
    ///
    pending_focus: Option<u64>,
    /// 由无障碍动作引发的失焦请求。
    pending_blur: bool,
    /// 编辑器上下文菜单。
    menu: unbound_widgets::context_menu::ContextMenuState,
    /// 打开菜单的编辑器。
    menu_owner: Option<EditorId>,
    /// 等待宿主执行的剪贴板动作。
    pending_clipboard: Option<ClipboardRequest>,
    /// 正在拖拽选择文本的编辑器及其命中节点。
    drag_editor: Option<(EditorId, u64)>,
    /// 打开菜单时剪贴板是否含有可粘贴文本。
    clipboard_has_text: bool,
    /// 顶栏工具按钮与页脚入口的悬停 Tooltip 延迟状态机。
    tooltip: unbound_widgets::tooltip::TooltipState,
    /// 已向运行时 `TimerQueue` 注册的 Tooltip 一次性延迟定时器令牌（ADR-0035）。
    tooltip_timer: Option<unbound_app::TimerToken>,
    /// 当前已调度定时器对应的悬停目标节点键。
    tooltip_timer_anchor: Option<u64>,
}

/// 等待宿主执行的剪贴板动作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardRequest {
    /// 将文本写入剪贴板。
    Write(String),
    /// 读取剪贴板并粘贴。
    Read,
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
    locale: u8,
    /// 当前激活的单控件详情 ID；`u16::MAX` 表示非单控件详情页。
    active_component: u16,
    /// 当前激活的分类索引 ID；`u8::MAX` 表示 All Controls 或非分类页。
    active_category: u8,
    /// 当前来源筛选标识。
    origin_filter: u8,
    /// 首页快捷区标签索引。
    home_quick_tab: u8,
    /// 处于打开状态的代码面板（支持最多 192 张卡片）。
    code_mask: [u64; 3],
    /// 已载入的像素对照用例数；`u32::MAX` 表示报告尚未载入。
    parity_cases: u32,
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
        let labels = Labels::new();
        let mut state = GalleryState::default();
        state.text_render.report = labels.cache.report();
        let init_viewport = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let init_nav_desc = build_navigation_view_desc(&state, init_viewport, None);
        let nav_view = NavigationViewState::from_desc(&init_nav_desc);
        // 10 个控件分类默认折叠，首屏导航树保持简洁直观（ADR 0036）。
        let nav_layout = init_nav_desc.compute_layout(&nav_view);
        let mut tree = ShellTree::new();
        let layout = tree.solve(&state, init_viewport, nav_layout.pane_rect.width());
        Self {
            state,
            mica: unbound_app::MicaBackdrop::new(),
            scene: gallery::Gallery::new(),
            labels,
            layout,
            focus_order: Vec::new(),
            regions: Vec::new(),
            ranges: Vec::new(),
            scroll_zones: Vec::new(),
            semantics: unbound_a11y::Semantics::new(ROOT),
            tree,
            page_layout: None,
            nav_view,
            nav_layout,
            generation: 0,
            atlas_clock: 0,
            last_revision: 0,
            last_hover: None,
            last_pressed: None,
            last_focus: None,
            last_viewport: (0.0, 0.0),
            pointer: Vec2::ZERO,
            panning: None,
            dragging_range: None,
            effective_scale: 1.0,
            pending_window: None,
            window_maximized: false,
            scrollbar: None,
            scroll_drag: None,
            list_scroll_drag: None,
            dirty_ranges: Vec::new(),
            layout_key: LayoutKey {
                width: 0,
                height: 0,
                nav_width: 0,
                page: u8::MAX,
                locale: u8::MAX,
                active_component: u16::MAX,
                active_category: u8::MAX,
                origin_filter: 0,
                home_quick_tab: 0,
                code_mask: [0; 3],
                parity_cases: u32::MAX,
            },
            external_diff: false,
            frame_layout_stats: LayoutStats::default(),
            node_spans: rustc_hash::FxHashMap::default(),
            emitted_widgets: rustc_hash::FxHashSet::default(),
            pending_focus: None,
            pending_blur: false,
            menu: unbound_widgets::context_menu::ContextMenuState::default(),
            menu_owner: None,
            pending_clipboard: None,
            drag_editor: None,
            clipboard_has_text: false,
            tooltip: unbound_widgets::tooltip::TooltipState::new()
                .with_warmup_ms(unbound_widgets::tooltip::TOOLTIP_WARMUP_MS),
            tooltip_timer: None,
            tooltip_timer_anchor: None,
        }
    }

    /// 取得指定编辑器的只读状态。
    fn editor(&self, id: EditorId) -> &crate::state::TextEditor {
        match id {
            EditorId::Search => &self.state.search,
            EditorId::Demo => &self.state.demo.text,
            EditorId::Fps => &self.state.fps_input,
        }
    }

    /// 取得指定编辑器的可变状态。
    fn editor_mut(&mut self, id: EditorId) -> &mut crate::state::TextEditor {
        match id {
            EditorId::Search => &mut self.state.search,
            EditorId::Demo => &mut self.state.demo.text,
            EditorId::Fps => &mut self.state.fps_input,
        }
    }

    /// 编辑器使用的基础字号。
    const fn editor_font_size(id: EditorId) -> f32 {
        match id {
            EditorId::Search => 11.0,
            EditorId::Demo => 14.0,
            EditorId::Fps => 10.0,
        }
    }

    /// 让指定编辑器独占键盘焦点。
    fn focus_editor(&mut self, id: EditorId, node: u64) {
        for other in [EditorId::Search, EditorId::Demo, EditorId::Fps] {
            if other != id {
                self.editor_mut(other).blur();
            }
        }
        self.editor_mut(id).focus();
        if id == EditorId::Search {
            self.state.search_selection = 0;
        }
        self.pending_focus = Some(node);
        self.state.bump();
    }

    /// 当前持有焦点的编辑器。
    #[must_use]
    pub fn focused_editor(&self) -> Option<EditorId> {
        [EditorId::Search, EditorId::Demo, EditorId::Fps]
            .into_iter()
            .find(|id| self.editor(*id).focused)
    }

    /// 向指定编辑器归约一次编辑事件。
    pub fn editor_event(
        &mut self,
        id: EditorId,
        event: unbound_widgets::text_input::TextInputEvent,
    ) -> unbound_widgets::text_input::TextInputOutcome {
        use unbound_widgets::text_input::{reduce_text_input, TextInputDesc, TextInputEvent};

        // FPS 输入只接受数字，最多三位；粘贴与输入法提交同样过滤，
        // 避免非法字符经非按键通道进入缓冲区。
        let event = if id == EditorId::Fps {
            match event {
                TextInputEvent::Insert(text) => TextInputEvent::Insert(numeric_only(&text)),
                TextInputEvent::Paste(text) => TextInputEvent::Paste(numeric_only(&text)),
                TextInputEvent::ImeCommit { text } => TextInputEvent::ImeCommit {
                    text: numeric_only(&text),
                },
                TextInputEvent::ImePreedit { text, cursor } => {
                    let text = numeric_only(&text);
                    let cursor = cursor.map(|value| value.min(text.len()));
                    TextInputEvent::ImePreedit { text, cursor }
                }
                other => other,
            }
        } else {
            event
        };
        let mut desc = TextInputDesc::new(Rect::new(0.0, 0.0, 1.0, 1.0));
        desc.font_size = Self::editor_font_size(id);
        desc.max_bytes = if id == EditorId::Fps { 3 } else { 256 };
        let outcome = {
            let editor = self.editor_mut(id);
            reduce_text_input(&desc, &mut editor.edit, &event)
        };
        if outcome.changed {
            self.editor_mut(id).touch();
            self.sync_editor_scroll(id);
            // 查询变化后旧选中项已失去意义，回到第一项。
            if id == EditorId::Search {
                self.state.search_selection = 0;
            }
            self.state.bump();
        }
        if let Some(text) = outcome.copied.clone() {
            self.pending_clipboard = Some(ClipboardRequest::Write(text));
        }
        if let Some(text) = outcome.cut.clone() {
            self.pending_clipboard = Some(ClipboardRequest::Write(text));
        }
        if outcome.paste_requested {
            self.pending_clipboard = Some(ClipboardRequest::Read);
        }
        outcome
    }

    /// 保持插入符在编辑器可视范围内。
    fn sync_editor_scroll(&mut self, id: EditorId) {
        let font = Self::editor_font_size(id);
        let (text, cursor) = {
            let editor = self.editor(id);
            (editor.edit.text().to_owned(), editor.edit.cursor())
        };
        let caret = crate::editor::caret_x(&mut self.labels, &text, cursor, font);
        let width = self.state.editor_view(id);
        self.editor_mut(id).edit.ensure_caret_visible(caret, width);
    }

    /// 取出待宿主执行的剪贴板动作。
    pub fn take_clipboard_request(&mut self) -> Option<ClipboardRequest> {
        self.pending_clipboard.take()
    }

    /// 将剪贴板文本粘贴到当前聚焦的编辑器。
    pub fn paste_text(&mut self, text: &str) {
        if let Some(id) = self.focused_editor() {
            self.editor_event(
                id,
                unbound_widgets::text_input::TextInputEvent::Paste(text.to_string()),
            );
        }
    }

    /// 目标节点对应的编辑器。
    #[must_use]
    pub fn editor_for_target(target: Target) -> Option<EditorId> {
        match target {
            Target::Search => Some(EditorId::Search),
            Target::FpsInput => Some(EditorId::Fps),
            Target::Demo(page, _, 1)
                if page == PageId::TextField.index() as u8
                    || page == PageId::Fonts.index() as u8 =>
            {
                Some(EditorId::Demo)
            }
            _ => None,
        }
    }

    /// 命中区域中指定节点的边界。
    #[must_use]
    fn region_bounds(&self, node: u64) -> Option<Rect> {
        self.regions
            .iter()
            .find(|region| region.node == node)
            .map(|region| region.bounds)
    }

    /// 聚焦编辑器并把插入符放到指针位置。
    ///
    /// 本函数只负责定位插入符，绝不进入拖拽态：点击报告会在释放之后再次抵达这里，
    /// 若在此捕获拖拽，普通悬停就会被当作拖拽更新，形成幽灵选区。
    pub fn focus_editor_at(&mut self, id: EditorId, node: u64, position: Vec2, extend: bool) {
        self.focus_editor(id, node);
        let font = Self::editor_font_size(id);
        let padding = Self::editor_padding(id);
        let bounds = self.region_bounds(node).or(match id {
            EditorId::Search => Some(self.layout.search),
            _ => None,
        });
        let Some(bounds) = bounds else {
            return;
        };
        let local_x = position.x - bounds.origin.x - padding;
        let offset = {
            let edit = self.editor(id).edit.clone();
            crate::editor::pointer_offset(&mut self.labels, &edit, font, local_x)
        };
        let _ = self.editor_event(
            id,
            if extend {
                unbound_widgets::text_input::TextInputEvent::PointerDrag { offset }
            } else {
                unbound_widgets::text_input::TextInputEvent::PointerDown { offset }
            },
        );
        self.editor_mut(id).touch();
    }

    /// 编辑器文本区域与控件边缘的内边距。
    const fn editor_padding(id: EditorId) -> f32 {
        match id {
            EditorId::Search => 36.0,
            EditorId::Demo => 18.0,
            EditorId::Fps => 6.0,
        }
    }

    /// 在编辑器上双击：选中指针下的单词。
    pub fn double_click_editor(&mut self, target: Target, position: Vec2) {
        let Some(id) = Self::editor_for_target(target) else {
            return;
        };
        let node = target::id(target);
        self.focus_editor(id, node);
        let bounds = self.region_bounds(node);
        let Some(bounds) = bounds else {
            return;
        };
        let local_x = position.x - bounds.origin.x - Self::editor_padding(id);
        let offset = {
            let edit = self.editor(id).edit.clone();
            crate::editor::pointer_offset(
                &mut self.labels,
                &edit,
                Self::editor_font_size(id),
                local_x,
            )
        };
        let _ = self.editor_event(
            id,
            unbound_widgets::text_input::TextInputEvent::DoubleClick { offset },
        );
    }

    /// 在编辑器上三击：全选。
    pub fn triple_click_editor(&mut self, target: Target) {
        let Some(id) = Self::editor_for_target(target) else {
            return;
        };
        let node = target::id(target);
        self.focus_editor(id, node);
        let _ = self.editor_event(id, unbound_widgets::text_input::TextInputEvent::TripleClick);
    }

    /// 在编辑器上右键：按 WinUI 规则定位或保留选区，并在指针处打开上下文菜单。
    ///
    /// 绝不在菜单打开时主动做词选：点击位于选区之外时折叠选区并把插入符移到点击处；
    /// 位于选区内时保留选区；没有选区时只移动插入符。
    ///
    pub fn open_editor_menu(&mut self, target: Target, position: Vec2, clipboard_has_text: bool) {
        let Some(id) = Self::editor_for_target(target) else {
            return;
        };
        self.clipboard_has_text = clipboard_has_text;
        let node = target::id(target);
        self.focus_editor(id, node);
        let bounds = self.region_bounds(node);
        if let Some(bounds) = bounds {
            let local_x = position.x - bounds.origin.x - Self::editor_padding(id);
            let offset = {
                let edit = self.editor(id).edit.clone();
                crate::editor::pointer_offset(
                    &mut self.labels,
                    &edit,
                    Self::editor_font_size(id),
                    local_x,
                )
            };
            // 边界也视作选区内：点击恰好落在选区边缘时不应意外丢掉选区。
            let inside_selection = self
                .editor(id)
                .edit
                .selection()
                .is_some_and(|(start, end)| (start..=end).contains(&offset));
            if !inside_selection {
                let _ = self.editor_event(
                    id,
                    unbound_widgets::text_input::TextInputEvent::PointerDown { offset },
                );
            }
        }
        self.menu_owner = Some(id);
        self.menu.open_at(position, node, Some(node));
        self.state.bump();
    }

    /// 当前编辑器上下文菜单是否可见。
    #[must_use]
    pub fn editor_menu_open(&self) -> bool {
        self.menu.is_open()
    }

    /// 编辑器上下文菜单的描述符（已按视口定位）。
    #[must_use]
    fn editor_menu_desc(&self) -> Option<unbound_widgets::menu::MenuDesc> {
        use unbound_widgets::menu::{MenuDesc, MenuItem};

        let id = self.menu_owner?;
        let editor = self.editor(id);
        let has_selection = editor.edit.selection().is_some();
        let has_text = !editor.text().is_empty();
        let mut cut = MenuItem::action(self.state.t("menu.cut")).with_shortcut("Ctrl+X");
        let mut copy = MenuItem::action(self.state.t("menu.copy")).with_shortcut("Ctrl+C");
        let mut paste = MenuItem::action(self.state.t("menu.paste")).with_shortcut("Ctrl+V");
        let mut select_all =
            MenuItem::action(self.state.t("menu.select_all")).with_shortcut("Ctrl+A");
        if !has_selection {
            cut = cut.disabled();
            copy = copy.disabled();
        }
        if !self.clipboard_has_text {
            paste = paste.disabled();
        }
        if !has_text {
            select_all = select_all.disabled();
        }
        let desc = MenuDesc::new(
            Rect::new(0.0, 0.0, 184.0, 0.0),
            vec![cut, copy, paste, MenuItem::separator(), select_all],
        );
        Some(self.menu.desc_at(desc, self.layout.viewport))
    }

    /// 激活上下文菜单项。
    fn activate_editor_menu_item(&mut self, index: usize) {
        use unbound_widgets::text_input::TextInputEvent;
        let Some(id) = self.menu_owner else {
            return;
        };
        // 菜单项的可用性在这里再判一次：命中区域可能仍会收到点击，
        // 不能只依赖绘制阶段的置灰状态。
        let has_selection = self.editor(id).edit.selection().is_some();
        let has_text = !self.editor(id).edit.text().is_empty();
        let clipboard_has_text = self.clipboard_has_text;
        // 行索引包含分隔符：Cut=0、Copy=1、Paste=2、分隔符=3、Select all=4。
        match index {
            0 if has_selection => {
                let _ = self.editor_event(id, TextInputEvent::Cut);
            }
            1 if has_selection => {
                let _ = self.editor_event(id, TextInputEvent::Copy);
            }
            2 if clipboard_has_text => {
                self.pending_clipboard = Some(ClipboardRequest::Read);
            }
            4 if has_text => {
                let _ = self.editor_event(id, TextInputEvent::SelectAll);
            }
            _ => {}
        }
        self.close_editor_menu(unbound_widgets::context_menu::ContextClose::Selection);
    }

    /// 关闭编辑器上下文菜单。
    fn close_editor_menu(&mut self, reason: unbound_widgets::context_menu::ContextClose) {
        self.menu.close(reason);
        self.menu_owner = None;
        self.state.bump();
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

    /// 当前帧与自定义标题栏双向同步的 `NavigationView` 快照（ADR-0026）。
    #[must_use]
    pub fn nav_title_bar_sync(&self) -> NavigationViewTitleBarSync {
        let desc = build_navigation_view_desc(&self.state, self.layout.viewport, None);
        self.nav_view.title_bar_sync(&desc)
    }

    /// 构建与当前顶栏尺寸及最大化状态对齐的 [`WindowCaptionButtons`] 描述符（ADR-0029）。
    #[must_use]
    pub fn window_caption_buttons_desc(&self) -> WindowCaptionButtons {
        window_caption_buttons_desc(self.layout.top_bar, self.window_maximized)
    }

    /// 计算当前顶栏窗口控制按钮组的几何布局。
    #[must_use]
    pub fn window_caption_buttons_layout(&self) -> WindowCaptionButtonsLayout {
        self.window_caption_buttons_desc()
            .layout(self.layout.top_bar)
    }

    /// 导出当前帧对齐 `FrameHitMetrics` 的统一窗口控制按钮热区契约（ADR-0029）。
    #[must_use]
    pub fn window_caption_hit_contract(&self) -> WindowCaptionFrameHitContract {
        self.window_caption_buttons_layout().frame_hit_contract()
    }

    /// 消费并映射 `NavigationViewState` 产生的强类型事件队列。
    fn drain_navigation_view_events(&mut self) {
        let events = self.nav_view.drain_events();
        for event in events {
            match event {
                NavigationViewEvent::ItemInvoked { item_id, .. } => match target::decode(item_id) {
                    Some(Target::NavPage(page)) => {
                        let target_page = PageId::from_index(page as usize);
                        self.navigate_internal(target_page, item_id);
                    }
                    Some(Target::NavCategory(cat_idx)) => {
                        if let Some(cat) = ControlCategory::from_index(cat_idx as usize) {
                            self.navigate_to_category(Some(cat));
                        }
                    }
                    Some(Target::NavComponent(index)) => {
                        if let Some(&id) = WidgetCatalogId::ALL.get(index as usize) {
                            self.navigate_to_component(id);
                        }
                    }
                    Some(Target::Footer(index)) => {
                        let target_page = if index == 0 {
                            PageId::Telemetry
                        } else {
                            PageId::Settings
                        };
                        self.navigate_internal(target_page, item_id);
                    }
                    _ => {}
                },
                NavigationViewEvent::PaneOpening
                | NavigationViewEvent::PaneClosing
                | NavigationViewEvent::DisplayModeChanged { .. }
                | NavigationViewEvent::Expanding { .. }
                | NavigationViewEvent::Collapsed { .. }
                | NavigationViewEvent::SelectionChanged { .. }
                | NavigationViewEvent::BackRequested
                | NavigationViewEvent::SearchRequested { .. } => {
                    self.state.bump();
                }
            }
        }
    }

    fn navigate_internal(&mut self, page: PageId, nav_item_id: u64) {
        self.state.navigate(page);
        self.nav_view.selected_item_id = Some(nav_item_id);
        self.state.blur_editors();
        self.state.search.edit.set_text(String::new());
        self.state.demo.modal_open = false;
        self.state.demo.dropdown_open = false;
        self.state.lang_dropdown_open = false;
        self.state.zoom_dropdown_open = false;
        self.state.text_render.family_dropdown_open = false;
        self.state.text_render.weight_dropdown_open = false;
        self.pending_focus = Some(nav_item_id);
    }

    /// 导航并关闭瞬态表面。
    pub fn navigate(&mut self, page: PageId) {
        let item_id = target::id(Target::NavPage(page.index() as u8));
        self.navigate_internal(page, item_id);
    }

    /// 导航至指定功能分类索引页（`None` 表示全部控件总览页）。
    pub fn navigate_to_category(&mut self, category: Option<ControlCategory>) {
        self.state.navigate_to_category(category);
        let item_id = match category {
            Some(cat) => target::id(Target::NavCategory(cat.index() as u8)),
            None => target::id(Target::NavPage(PageId::Components.index() as u8)),
        };
        self.nav_view.selected_item_id = Some(item_id);
        self.state.blur_editors();
        self.state.search.edit.set_text(String::new());
        self.state.demo.modal_open = false;
        self.state.demo.dropdown_open = false;
        self.state.lang_dropdown_open = false;
        self.state.zoom_dropdown_open = false;
        self.state.text_render.family_dropdown_open = false;
        self.state.text_render.weight_dropdown_open = false;
        self.pending_focus = Some(item_id);
    }

    /// 导航至指定目录控件的独立详情页，并自动展开其所属分类节点。
    pub fn navigate_to_component(&mut self, id: unbound_widgets::WidgetCatalogId) {
        self.state.navigate_to_component(id);
        let cat_node = target::id(Target::NavCategory(id.category().index() as u8));
        if !self.nav_view.expanded_item_ids.contains(&cat_node) {
            self.nav_view.expanded_item_ids.push(cat_node);
        }
        let item_id = target::id(Target::NavComponent(id as u8));
        self.nav_view.selected_item_id = Some(item_id);
        self.state.blur_editors();
        self.state.search.edit.set_text(String::new());
        self.state.demo.modal_open = false;
        self.state.demo.dropdown_open = false;
        self.state.lang_dropdown_open = false;
        self.state.zoom_dropdown_open = false;
        self.state.text_render.family_dropdown_open = false;
        self.state.text_render.weight_dropdown_open = false;
        self.pending_focus = Some(item_id);
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
                    if self.state.search.focused {
                        self.state.search.edit.set_text(value);
                        self.state.bump();
                    } else if self.state.demo.text.focused {
                        self.state.demo.text.edit.set_text(value);
                        self.state.bump();
                    }
                }
            }
            // 递增 / 递减需要控件的范围，该范围位于页面描述中；演示目前尚无此类描述符。
            //
            _ => {}
        }
    }

    /// 写入平台能力探测结果并立即重新解析后景计划。
    ///
    /// 宿主在创建首条交换链之前调用，使首帧即落到正确的后景路径；
    /// 返回后景计划是否发生变化。
    pub fn set_backdrop_capabilities(
        &mut self,
        windows_build: Option<u32>,
        premultiplied_alpha_supported: bool,
    ) -> bool {
        self.state.windows_build = windows_build;
        self.state.premultiplied_alpha_supported = premultiplied_alpha_supported;
        self.sync_backdrop_plan()
    }

    /// 写入 accent policy 通道的可用性并立即重新解析后景计划。
    ///
    /// 它与 [`Self::set_backdrop_capabilities`] 分开：该能力只影响亚克力自定义
    /// 色调子档，不能混进其它材质的门槛。返回后景计划是否发生变化。
    pub fn set_backdrop_accent_capability(&mut self, available: bool) -> bool {
        if !self.state.set_backdrop_accent_capability(available) {
            return false;
        }
        self.sync_backdrop_plan()
    }

    /// 按当前能力重新解析后景计划；变化时写入实际生效组合并请求重建。
    ///
    /// 实际生效的材质与来源由能力探测和用户选择共同决定，页面只读取解析结果，
    /// 不自行推导。返回计划是否发生变化：宿主据此重新下发清屏 alpha 与系统材质请求。
    pub fn sync_backdrop_plan(&mut self) -> bool {
        let window = self
            .state
            .window_screen_rect
            .map(|rect| unbound_backdrop::PixelRect::new(rect.x, rect.y, rect.width, rect.height));
        let plan = BackdropPlan::derive(unbound_backdrop::BackdropInputs {
            selection: self.state.backdrop_selection,
            windows_build: self.state.windows_build,
            premultiplied_alpha_supported: self.state.premultiplied_alpha_supported,
            accent_policy_export: self.state.accent_policy_export,
            accent_tint_argb: self.state.backdrop_accent_tint,
            window_screen_rect: window,
            primary_screen_rect: unbound_backdrop::primary_screen_rect(),
            primary_screen_scale: unbound_backdrop::primary_screen_scale(),
            dark: self.state.theme.is_dark(),
            self_drawn_ready: self.mica.visible(),
        });
        let plan_changed = plan != self.state.backdrop_plan;
        self.state.backdrop_combination = BackdropCombination {
            material: plan.material_kind,
            source: plan.source,
            fallback: plan.fallback,
            unavailable: plan.unavailable,
        };
        self.state.backdrop_plan = plan;
        if plan_changed {
            self.state.bump();
        }
        plan_changed
    }

    /// 根背景是否必须透明。
    ///
    /// 后景计划已经根据实际路径与自绘纹理就绪状态给出结论；宿主不再自行
    /// 组合「原生路径 || 纹理可见」，避免两处规则漂移。
    #[must_use]
    pub fn root_background_transparent(&self) -> bool {
        self.state.backdrop_plan.root_background_transparent
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
            || !self.nav_view.pane_width_spring.is_settled()
            || !self.nav_view.overlay_progress_spring.is_settled()
            || !self.nav_view.indicator_pos_spring.is_settled()
            || !self.nav_view.indicator_len_spring.is_settled()
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
        let mut code_mask = [0u64; 3];
        for (index, _) in page.examples().iter().enumerate() {
            if self.state.code_open(page, index as u8) && index < 192 {
                code_mask[index / 64] |= 1u64 << (index % 64);
            }
        }
        let origin_filter = match self.state.control_origin_filter {
            None => 0,
            Some(ControlClass::Baseline) => 1,
            Some(ControlClass::Toolkit) => 2,
            Some(ControlClass::Exclusive) => 3,
        };
        LayoutKey {
            width: viewport.0.to_bits(),
            height: viewport.1.to_bits(),
            nav_width: nav_width.to_bits(),
            page: page.index() as u8,
            locale: self.state.locale.index() as u8,
            active_component: self.state.active_component.map_or(u16::MAX, |id| id as u16),
            active_category: self
                .state
                .active_category
                .map_or(u8::MAX, |cat| cat.index() as u8),
            origin_filter,
            home_quick_tab: self.state.home_quick_tab,
            code_mask,
            parity_cases: self
                .state
                .demo
                .parity_report
                .as_ref()
                .map_or(u32::MAX, |report| report.cases.len() as u32),
        }
    }

    /// 无条件重建。
    pub fn rebuild(&mut self, viewport: (f32, f32), inter: Interaction) {
        let (width, height) = viewport;
        let viewport_rect = Rect::new(0.0, 0.0, width.max(320.0), height.max(240.0));
        let hover = inter.hovered;
        let pressed = inter.pressed;
        let nav_hovered = hover.filter(|&id| is_nav_item_node(id));
        let nav_desc = build_navigation_view_desc(&self.state, viewport_rect, nav_hovered);
        self.nav_view.sync_viewport(&nav_desc, viewport_rect.size.x);
        self.drain_navigation_view_events();
        self.nav_view.hovered_item_id = nav_hovered;
        let pre_nav_layout = nav_desc.compute_layout(&self.nav_view);
        self.nav_view.scroll_max = (pre_nav_layout.menu_content_height
            - pre_nav_layout.menu_viewport_rect.height())
        .max(0.0);
        self.nav_view.scroll_offset = self
            .nav_view
            .scroll_offset
            .clamp(0.0, self.nav_view.scroll_max);
        let active_nav_pane = pre_nav_layout
            .overlay_rect
            .unwrap_or(pre_nav_layout.pane_rect);
        let hover_on_nav_bar = matches!(
            hover,
            Some(id) if id == NAV_SCROLLBAR_THUMB_NODE || id == NAV_SCROLLBAR_TRACK_NODE
        );
        let pane_hovered = active_nav_pane.contains_point(self.pointer)
            || nav_hovered.is_some()
            || hover_on_nav_bar;
        self.nav_view.set_pane_hovered(pane_hovered);
        let bar_hovered = hover_on_nav_bar
            || nav_desc
                .menu_scrollbar_desc(&pre_nav_layout, &self.nav_view, false)
                .is_some_and(|bar| {
                    bar.is_visible() && bar.track_rect().contains_point(self.pointer)
                });
        self.nav_view.set_scrollbar_hovered(bar_hovered);
        let nav_sync = self.nav_view.title_bar_sync(&nav_desc);

        let raw_nav_width = pre_nav_layout.pane_rect.width();
        let scale = self.effective_scale.max(0.5);
        // 在布局求解前将侧边栏宽度对齐到物理像素网格，使 `layout.nav` 与 `layout.content` 的分界线在动画过程中不会出现亚像素往返跳变。
        let nav_width = if self.state.pixel_snap {
            (raw_nav_width * scale).round() / scale
        } else {
            raw_nav_width
        };
        let palette = gallery::palette();
        // 云母已覆盖整窗（或原生材质从客户区透出）时根背景必须透明，
        // 否则不透明底色会把后景盖住。
        let mica_visible = self.root_background_transparent();
        // 对照报告在布局求解前载入：舞台高度取决于用例数，先拿到用例数再求解，
        // 才能在进入报告页的首帧就得到完整滚动范围，而不是等到某次悬停才补一次重排。
        if self.state.active_page == PageId::Parity {
            self.state.demo.poll_parity(&mut self.labels.engine);
            if let Some(report) = self.state.demo.parity_report.as_ref() {
                if report.error.is_none() {
                    self.state.parity_inspector.auto_focus_for_report(report);
                }
            }
        }
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
        // 上下文菜单属于浮层状态而非页面状态，必须在把 `self` 拆成
        // 绘制所需的字段之前取出描述符，否则绘制阶段拿不到菜单几何。
        let editor_menu = if self.editor_menu_open() {
            self.editor_menu_desc()
                .map(|desc| (desc, self.menu.menu.highlighted))
        } else {
            None
        };
        let GalleryApp {
            scene,
            labels,
            state,
            nav_view,
            tooltip,
            ..
        } = self;
        labels.set_raster_scale(scale);
        // 字形槽位按最近最少使用帧进行驱逐。若无此时钟推进，所有槽位看起来一样旧，
        // 导致满载的图集永远驱逐左上角相同的那几个字形。
        //
        labels.cache.begin_frame(atlas_clock);
        // 应用运行期文本渲染设置，若产生位图级或排版级变更则按需清空缓存，并同步只读生效报告。
        labels
            .cache
            .apply_settings(&state.text_render.settings, Some(&mut labels.engine));
        state.text_render.report = labels.cache.report();
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
        let nav_output = {
            let mut text_ctx = canvas.text_ctx(atlas_clock);
            emit_navigation_view(
                NAV_VIEW_NODE,
                &nav_desc,
                ControlState::default(),
                nav_view,
                &mut text_ctx,
            )
        };
        let resolved_nav_layout = nav_output.layout.clone();
        draw_background(&mut canvas, &layout, mica_visible);
        draw_top_bar(
            &mut canvas,
            state,
            &layout,
            self.window_maximized,
            nav_sync,
            atlas_clock,
        );
        // NavigationView 前 2 个图元为窗格透明底与圆角内容区容器，需位于页面内容下方；
        // 其余窗格行、Overlay 浮层与子菜单 Flyout 在页面内容之上发射，确保紧凑/极简浮层覆盖页面。
        draw_navigation_view_shell_backdrop(&mut canvas, &nav_output.output);
        draw_page(&mut canvas, state, &layout, &page_layout);
        draw_navigation_view_pane(&mut canvas, &nav_output.output, &resolved_nav_layout);
        draw_hud(&mut canvas, state, &layout, page_layout.cards.len());
        draw_search_overlay(&mut canvas, state, &layout);
        draw_top_bar_dropdowns(&mut canvas, state, &layout);
        draw_editor_menu(&mut canvas, editor_menu.as_ref());
        let tooltip_hover = if !state.search.focused
            && !state.lang_dropdown_open
            && !state.zoom_dropdown_open
            && editor_menu.is_none()
        {
            tooltip_target_of(
                state,
                &layout,
                &resolved_nav_layout,
                Some(&page_layout),
                self.window_maximized,
                canvas.labels,
                hover,
            )
            .and(hover)
        } else {
            None
        };
        let tooltip_timer_desc =
            unbound_widgets::tooltip::TooltipDesc::new(Rect::ZERO, layout.viewport, Vec2::ZERO);
        let _ = tooltip.update(&tooltip_timer_desc, tooltip_hover, 0.0);
        if tooltip.visible() {
            draw_top_bar_tooltip(
                &mut canvas,
                state,
                &layout,
                &resolved_nav_layout,
                Some(&page_layout),
                self.window_maximized,
                tooltip_hover,
            );
        }
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
        self.nav_layout = resolved_nav_layout;
        self.last_revision = self.state.revision;
        self.last_hover = hover;
        self.last_pressed = pressed;
        self.last_focus = focused;
        self.last_viewport = viewport;
    }

    /// 推进时间驱动的演示、`NavigationView` 弹簧动效与顶栏 Tooltip 延迟计时器。
    pub fn tick(&mut self, dt: f32) -> bool {
        let nav_desc = build_navigation_view_desc(&self.state, self.layout.viewport, None);
        let nav_moving = self.nav_view.tick(dt, &nav_desc);
        let demo_moving = self.state.tick(dt);
        let tooltip_hover = if !self.state.search.focused
            && !self.state.lang_dropdown_open
            && !self.state.zoom_dropdown_open
            && !self.editor_menu_open()
        {
            tooltip_target_of(
                &self.state,
                &self.layout,
                &self.nav_layout,
                self.page_layout.as_ref(),
                self.window_maximized,
                &mut self.labels,
                self.last_hover,
            )
            .and(self.last_hover)
        } else {
            None
        };
        let tooltip_desc = unbound_widgets::tooltip::TooltipDesc::new(
            Rect::ZERO,
            self.layout.viewport,
            Vec2::ZERO,
        );
        let tooltip_changed =
            self.tooltip
                .update(&tooltip_desc, tooltip_hover, (dt * 1000.0).max(0.0));
        if tooltip_changed {
            self.state.bump();
        }
        let tooltip_pending = self.tooltip.delay().is_some() && !self.tooltip.visible();
        if nav_moving || demo_moving {
            self.state.bump();
        }
        nav_moving || demo_moving || tooltip_changed || tooltip_pending
    }

    /// 根据最新悬停节点同步壳层 Tooltip 延迟状态机与 `TimerQueue` 单次唤醒定时器（ADR-0035）。
    pub fn sync_tooltip_hover(&mut self, hovered: Option<u64>, ctx: &mut unbound_app::AppContext) {
        let tooltip_hover = if !self.state.search.focused
            && !self.state.lang_dropdown_open
            && !self.state.zoom_dropdown_open
            && !self.editor_menu_open()
        {
            tooltip_target_of(
                &self.state,
                &self.layout,
                &self.nav_layout,
                self.page_layout.as_ref(),
                self.window_maximized,
                &mut self.labels,
                hovered,
            )
            .and(hovered)
        } else {
            None
        };
        let tooltip_desc = unbound_widgets::tooltip::TooltipDesc::new(
            Rect::ZERO,
            self.layout.viewport,
            Vec2::ZERO,
        );
        if self.tooltip.update(&tooltip_desc, tooltip_hover, 0.0) {
            self.state.bump();
        }
        self.sync_tooltip_timer(ctx);
    }

    /// 同步当前 Tooltip 延迟状态与 `AppContext` 一次性定时器。
    pub fn sync_tooltip_timer(&mut self, ctx: &mut unbound_app::AppContext) {
        let pending = self
            .tooltip
            .remaining_delay_ms()
            .filter(|ms| *ms > 0.0)
            .zip(self.tooltip.hovered_anchor());
        match pending {
            Some((remaining_ms, anchor)) => {
                if self.tooltip_timer_anchor != Some(anchor) || self.tooltip_timer.is_none() {
                    if let Some(old) = self.tooltip_timer.take() {
                        ctx.cancel_timer(old);
                    }
                    let delay =
                        std::time::Duration::from_millis(remaining_ms.ceil().max(1.0) as u64);
                    let token =
                        ctx.set_timeout(delay, |token| unbound_app::AppEvent::Timer { token });
                    self.tooltip_timer = Some(token);
                    self.tooltip_timer_anchor = Some(anchor);
                }
            }
            None => {
                if let Some(old) = self.tooltip_timer.take() {
                    ctx.cancel_timer(old);
                }
                self.tooltip_timer_anchor = None;
            }
        }
    }

    /// 处理到期的一次性 Tooltip 延迟定时器并立即点亮气泡。
    pub fn handle_tooltip_timer(&mut self, token: unbound_app::TimerToken) -> bool {
        if self.tooltip_timer != Some(token) {
            return false;
        }
        self.tooltip_timer = None;
        self.tooltip_timer_anchor = None;
        let remaining_ms = self.tooltip.remaining_delay_ms().unwrap_or(0.0);
        let anchor = self.tooltip.hovered_anchor();
        let tooltip_desc = unbound_widgets::tooltip::TooltipDesc::new(
            Rect::ZERO,
            self.layout.viewport,
            Vec2::ZERO,
        );
        if self
            .tooltip
            .update(&tooltip_desc, anchor, remaining_ms + 1.0)
        {
            self.state.bump();
        }
        true
    }

    /// 移动指针（逻辑像素）。
    ///
    /// 仅外壳正在运行的拖拽会跟随指针；悬停本身由运行时负责，此处无需路由。
    ///
    pub fn pointer_move(&mut self, position: Vec2) {
        self.pointer = position;
        if self.state.active_page == PageId::Parity
            && self.state.parity_inspector.scrub_curtain(position)
        {
            self.state.bump();
        }
        if self.state.active_page == PageId::Components {
            match self.state.active_component {
                Some(WidgetCatalogId::CalendarView) => {
                    if let Some(rect) = self.state.sandbox.calendar_stage_rect.get() {
                        let desc = unbound_widgets::DatePickerDesc::new(rect);
                        if self
                            .state
                            .sandbox
                            .calendar_view
                            .handle_hover(&desc, position)
                        {
                            self.state.bump();
                        }
                    }
                }
                Some(WidgetCatalogId::DateRangePicker) => {
                    if let Some(rect) = self.state.sandbox.date_range_stage_rect.get() {
                        let desc = unbound_widgets::DatePickerDesc::new(rect);
                        if self
                            .state
                            .sandbox
                            .date_range_picker
                            .handle_hover(&desc, position)
                        {
                            self.state.bump();
                        }
                    }
                }
                _ => {}
            }
        }
        let nav_desc = build_navigation_view_desc(&self.state, self.layout.viewport, None);
        if self.nav_view.scrollbar_drag.is_some() {
            if self
                .nav_view
                .handle_pointer_drag(&nav_desc, &self.nav_layout, position)
            {
                self.state.bump();
            }
        } else {
            let active_pane_rect = self
                .nav_layout
                .overlay_rect
                .unwrap_or(self.nav_layout.pane_rect);
            let pane_changed = self
                .nav_view
                .set_pane_hovered(active_pane_rect.contains_point(position));
            let bar_hovered = nav_desc
                .menu_scrollbar_desc(&self.nav_layout, &self.nav_view, false)
                .is_some_and(|bar| bar.is_visible() && bar.track_rect().contains_point(position));
            let bar_changed = self.nav_view.set_scrollbar_hovered(bar_hovered);
            if pane_changed || bar_changed {
                self.state.bump();
            }
        }
        if let Some(grab) = self.scroll_drag {
            if let Some(bar) = self.scrollbar {
                let scroll = bar.scroll_for(position.y - grab);
                if (scroll - self.state.content_scroll).abs() > 0.01 {
                    self.state.content_scroll = scroll;
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
        if let Some((id, node)) = self.drag_editor {
            if let Some(bounds) = self.region_bounds(node) {
                let local_x = position.x - bounds.origin.x - Self::editor_padding(id);
                let offset = {
                    let edit = self.editor(id).edit.clone();
                    crate::editor::pointer_offset(
                        &mut self.labels,
                        &edit,
                        Self::editor_font_size(id),
                        local_x,
                    )
                };
                let _ = self.editor_event(
                    id,
                    unbound_widgets::text_input::TextInputEvent::PointerDrag { offset },
                );
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
    /// 只有左键承担聚焦与拖拽捕获；右键与中键仅执行关闭浮层、失焦等公共逻辑，
    /// 编辑器聚焦与插入符定位交给随后的上下文菜单事件。
    ///
    pub fn pointer_down(&mut self, position: Vec2, target: Option<u64>, button: PointerButton) {
        self.pointer = position;
        self.tooltip.hide();
        // 点击 FPS 输入框之外的任意位置会提交待定值，就像原生微调框在失焦时一样。
        //
        if self.state.fps_input.focused && target != Some(target::id(Target::FpsInput)) {
            self.state.commit_fps_input();
        }
        // 点击编辑器上下文菜单之外的区域会关闭菜单，并吞掉这次点击。
        //
        if self.menu.is_open()
            && !matches!(
                target.and_then(target::decode),
                Some(Target::EditorMenuItem(_))
            )
        {
            self.close_editor_menu(unbound_widgets::context_menu::ContextClose::ClickOutside);
            return;
        }
        // 当 NavigationView 处于 Overlay 浮层或子菜单 Flyout 打开态时，点击外部区域自动轻闭。
        let nav_desc = build_navigation_view_desc(&self.state, self.layout.viewport, None);
        if self.nav_view.has_open_transient_surface(&nav_desc) {
            let inside_overlay = self
                .nav_layout
                .overlay_rect
                .is_some_and(|r| r.contains_point(position));
            let inside_flyout = self
                .nav_layout
                .flyout_rect
                .is_some_and(|r| r.contains_point(position));
            let on_toggle = target == Some(target::id(Target::NavToggle));
            if !inside_overlay && !inside_flyout && !on_toggle {
                if self.nav_view.open_flyout_parent_id.is_some() {
                    self.nav_view.open_flyout_parent_id = None;
                    self.state.bump();
                } else if self.nav_view.is_overlay_open(&nav_desc) {
                    self.nav_view.set_pane_open(false, &nav_desc);
                    self.drain_navigation_view_events();
                    self.state.bump();
                }
            }
        }
        // 点击标题栏语言或缩放下拉菜单之外的区域时自动收起浮层。
        if (self.state.lang_dropdown_open || self.state.zoom_dropdown_open)
            && !matches!(
                target.and_then(target::decode),
                Some(Target::LangMenu | Target::Lang(_) | Target::ZoomMenu | Target::Zoom(_))
            )
        {
            self.state.lang_dropdown_open = false;
            self.state.zoom_dropdown_open = false;
            self.state.bump();
        }
        // 点击文本渲染页字体族或字重下拉菜单之外的区域时自动收起浮层。
        if self.state.text_render.family_dropdown_open
            || self.state.text_render.weight_dropdown_open
        {
            let keep_open = matches!(
                target.and_then(target::decode),
                Some(Target::Demo(page, 0, part))
                    if page == PageId::TextRender.index() as u8
                        && (part == pages::text_render::FONT_FAMILY_TRIGGER_PART
                            || part == pages::text_render::FONT_WEIGHT_TRIGGER_PART
                            || (pages::text_render::FONT_FAMILY_PART_BASE
                                ..pages::text_render::FONT_FAMILY_PART_BASE
                                    + pages::text_render::FONT_FAMILY_OPTIONS.len() as u16)
                                .contains(&part)
                            || (pages::text_render::FONT_WEIGHT_PART_BASE
                                ..pages::text_render::FONT_WEIGHT_PART_BASE
                                    + pages::text_render::FONT_WEIGHT_OPTIONS.len() as u16)
                                .contains(&part))
            );
            if !keep_open {
                self.state.text_render.family_dropdown_open = false;
                self.state.text_render.weight_dropdown_open = false;
                self.state.bump();
            }
        }
        // 点击编辑器之外的区域时释放编辑器焦点：选区随焦点一起折叠，
        // 否则旧选区会在后续悬停时继续显示。上下文菜单项属于编辑器本身。
        let editor_related = target.and_then(target::decode).is_some_and(|item| {
            Self::editor_for_target(item).is_some()
                || matches!(
                    item,
                    Target::EditorMenuItem(_)
                        | Target::SearchResult(_)
                        | Target::SearchComponent(_)
                )
        });
        if self.focused_editor().is_some() && !editor_related {
            self.state.blur_editors();
            self.state.bump();
        }
        // 右键与中键不聚焦、不进入拖拽：右键的编辑器定位由上下文菜单事件完成，
        // 若在此处走 `begin_press_target`，选区会在菜单打开前就被折叠。
        if button == PointerButton::Left {
            if let Some(node) = target {
                if node == NAV_SCROLLBAR_THUMB_NODE || node == NAV_SCROLLBAR_TRACK_NODE {
                    if self
                        .nav_view
                        .handle_pointer_down(&nav_desc, &self.nav_layout, position)
                    {
                        self.state.bump();
                    }
                    return;
                }
                self.pending_focus = Some(node);
                if let Some(target) = target::decode(node) {
                    self.begin_press_target(target, position);
                }
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
        if self.nav_view.handle_pointer_up(position) {
            self.state.bump();
        }
        if let Some((id, _)) = self.drag_editor.take() {
            let _ = self.editor_event(id, unbound_widgets::text_input::TextInputEvent::PointerUp);
        }
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
        self.tooltip.hide();
        if self.nav_view.handle_pointer_up(self.pointer) {
            self.state.bump();
        }
        self.scroll_drag = None;
        self.list_scroll_drag = None;
        self.dragging_range = None;
        self.panning = None;
        // 编辑器拖拽也必须释放：否则失焦后仍会跟随指针拖动选区。
        self.drag_editor = None;
        // 失焦时折叠编辑器选区并丢弃未提交的输入法组合。
        self.state.blur_editors();
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
        self.tooltip.hide();
        // 菜单打开时滚轮只负责关闭菜单：不滚动其下方的页面，也不进入任何拖拽。
        if self.menu.is_open() {
            self.close_editor_menu(unbound_widgets::context_menu::ContextClose::Scroll);
            return;
        }
        let point = self.pointer;
        let active_nav_rect = self
            .nav_layout
            .overlay_rect
            .unwrap_or(self.nav_layout.pane_rect);
        if active_nav_rect.contains_point(point) {
            if self.nav_view.scroll_by(-delta_y * 44.0) {
                self.state.bump();
            }
            return;
        }
        if !self.layout.content.contains_point(point) {
            return;
        }
        if self.state.active_page == PageId::Components {
            match self.state.active_component {
                Some(WidgetCatalogId::CalendarView) => {
                    if let Some(rect) = self.state.sandbox.calendar_stage_rect.get() {
                        if rect.contains_point(point) {
                            if self.state.sandbox.calendar_view.handle_wheel(delta_y) {
                                self.state.bump();
                            }
                            return;
                        }
                    }
                }
                Some(WidgetCatalogId::DateRangePicker) => {
                    if let Some(rect) = self.state.sandbox.date_range_stage_rect.get() {
                        if rect.contains_point(point) {
                            if self.state.sandbox.date_range_picker.handle_wheel(delta_y) {
                                self.state.bump();
                            }
                            return;
                        }
                    }
                }
                _ => {}
            }
        }
        if self.state.active_page == PageId::Parity {
            if let Some(pane) = self.state.parity_inspector.list_pane_rect.get() {
                if pane.contains_point(point) {
                    let count = self.state.demo.parity_report.as_ref().map_or(0, |r| {
                        pages::parity_report::filtered_case_indices(
                            r,
                            &self.state.parity_inspector,
                            self.state.search.text(),
                        )
                        .len()
                    });
                    let total_h = count as f32 * 58.0;
                    let max_scroll = (total_h - pane.size.y + 16.0).max(0.0);
                    self.state.parity_inspector.list_scroll =
                        (self.state.parity_inspector.list_scroll - delta_y * 44.0)
                            .clamp(0.0, max_scroll);
                    self.state.bump();
                    return;
                }
            }
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
        self.focused_editor().is_some()
    }

    /// 候选框应锚定的位置（逻辑像素），由绘制阶段按插入符位置更新。
    #[must_use]
    pub fn ime_caret(&self) -> Vec2 {
        self.state.ime_anchor()
    }

    /// 将已提交的文本插入当前聚焦控件。
    ///
    /// 控制字符会被丢弃：文本框为单行，提交中混入的换行符会破坏缓冲区。
    ///
    pub fn type_text(&mut self, text: &str) {
        let filtered: String = text.chars().filter(|c| !c.is_control()).collect();
        if filtered.is_empty() {
            return;
        }
        if let Some(id) = self.focused_editor() {
            self.editor_event(
                id,
                unbound_widgets::text_input::TextInputEvent::Insert(filtered),
            );
        }
    }

    /// 处理按住不放时 OS 重复发送的按键。
    ///
    /// 字符键与编辑键（含 Shift/Ctrl 修饰）随重复事件持续生效；
    /// 全选、复制、剪切、粘贴、撤销与重做是语义命令，只响应首次按下，
    /// 重复事件必须丢弃，否则长按会连续粘贴或连续撤销。
    pub fn key_repeat(&mut self, key: &PlatformKey, modifiers: Modifiers) {
        use unbound_widgets::text_input::TextInputEvent;

        let Some(id) = self.focused_editor() else {
            return;
        };
        let Some(event) = Self::key_to_edit_event(key, modifiers) else {
            return;
        };
        if matches!(
            event,
            TextInputEvent::SelectAll
                | TextInputEvent::Copy
                | TextInputEvent::Cut
                | TextInputEvent::Paste(_)
                | TextInputEvent::Undo
                | TextInputEvent::Redo
        ) {
            return;
        }
        let _ = self.editor_event(id, event);
    }

    /// 应用一次 IME 更新到当前聚焦控件。
    ///
    /// 组合文本直接写入编辑缓冲区；提交与取消都只作用于同一份真实文本，
    /// 因此同一次提交不会在页面上插入两次。
    pub fn apply_ime(&mut self, update: &unbound_app::ImeUpdate) {
        use unbound_widgets::text_input::TextInputEvent;

        let Some(id) = self.focused_editor() else {
            return;
        };
        if update.cancelled {
            self.editor_event(id, TextInputEvent::ImeCancel);
            return;
        }
        if let Some(commit) = &update.commit {
            self.editor_event(
                id,
                TextInputEvent::ImeCommit {
                    text: commit.clone(),
                },
            );
            return;
        }
        if update.enabled || !update.preedit.is_empty() {
            self.editor_event(
                id,
                TextInputEvent::ImePreedit {
                    text: update.preedit.clone(),
                    cursor: update.cursor_range.map(|(start, _)| start),
                },
            );
        }
    }

    /// 把平台按键转换为编辑事件。
    fn key_to_edit_event(
        key: &PlatformKey,
        modifiers: Modifiers,
    ) -> Option<unbound_widgets::text_input::TextInputEvent> {
        use unbound_widgets::text_input::TextInputEvent;
        let extend = modifiers.shift;
        let event = match key {
            PlatformKey::Named(name) => match name.as_str() {
                "Backspace" if modifiers.control => TextInputEvent::DeleteWordBack,
                "Backspace" => TextInputEvent::Backspace,
                "Delete" if modifiers.control => TextInputEvent::DeleteWordForward,
                "Delete" => TextInputEvent::Delete,
                "ArrowLeft" if modifiers.control => TextInputEvent::MoveWordLeft { extend },
                "ArrowLeft" => TextInputEvent::MoveLeft { extend },
                "ArrowRight" if modifiers.control => TextInputEvent::MoveWordRight { extend },
                "ArrowRight" => TextInputEvent::MoveRight { extend },
                "ArrowUp" | "Home" => TextInputEvent::Home { extend },
                "ArrowDown" | "End" => TextInputEvent::End { extend },
                _ => return None,
            },
            PlatformKey::Character(text) if modifiers.control => {
                match text.to_ascii_lowercase().as_str() {
                    "a" => TextInputEvent::SelectAll,
                    "c" => TextInputEvent::Copy,
                    "x" => TextInputEvent::Cut,
                    "z" if modifiers.shift => TextInputEvent::Redo,
                    "z" => TextInputEvent::Undo,
                    "y" => TextInputEvent::Redo,
                    _ => return None,
                }
            }
            PlatformKey::Character(text) => {
                if text.is_empty() || text.chars().any(char::is_control) {
                    return None;
                }
                TextInputEvent::Insert(text.clone())
            }
            PlatformKey::Other(_) => return None,
        };
        Some(event)
    }

    /// 处理按键按下。
    /// `focused` 是运行时的聚焦节点：外壳本身不保存聚焦节点。
    /// `Tab` 绝不会到达此方法——运行时的 `FocusManager` 在派发前已步进焦点顺序。
    pub fn key(&mut self, key: &PlatformKey, modifiers: Modifiers, focused: Option<u64>) {
        use unbound_widgets::context_menu::ContextKey;
        self.tooltip.hide();

        if self.menu.is_open() {
            let context_key = match key {
                PlatformKey::Named(name) => match name.as_str() {
                    "ArrowUp" => Some(ContextKey::Up),
                    "ArrowDown" => Some(ContextKey::Down),
                    "ArrowLeft" => Some(ContextKey::Left),
                    "ArrowRight" => Some(ContextKey::Right),
                    "Enter" => Some(ContextKey::Enter),
                    "Escape" => Some(ContextKey::Escape),
                    _ => None,
                },
                _ => None,
            };
            if let Some(context_key) = context_key {
                if let Some(desc) = self.editor_menu_desc() {
                    if let Some(index) = self.menu.key(&desc, context_key) {
                        self.activate_editor_menu_item(index);
                    }
                }
                self.state.bump();
                return;
            }
            self.close_editor_menu(unbound_widgets::context_menu::ContextClose::ClickOutside);
        }

        if let Some(id) = self.focused_editor() {
            // 搜索框的上下键用于遍历命中项：在文本编辑归约之前拦截，
            // 否则会被 key_to_edit_event 映射为 Home/End，只移动插入符。
            if id == EditorId::Search {
                let hit_count = self.state.search_hits().len();
                self.state.clamp_search_selection(hit_count);
                if let PlatformKey::Named(name) = key {
                    let delta = match name.as_str() {
                        "ArrowDown" => Some(1),
                        "ArrowUp" => Some(-1),
                        _ => None,
                    };
                    if let Some(delta) = delta {
                        let count = self.state.search_hits().len();
                        self.state.move_search_selection(delta, count);
                        return;
                    }
                }
            }
            match key {
                PlatformKey::Named(name) if name == "Escape" => {
                    // 组合进行中优先只取消组合：保留已提交的文本与焦点，
                    // 符合输入法“Esc 撤销本次组合”的惯例。
                    if self.editor_mut(id).edit.cancel_composition() {
                        self.editor_mut(id).touch();
                        self.state.bump();
                        return;
                    }
                    match id {
                        EditorId::Search => {
                            self.editor_mut(id).edit.set_text(String::new());
                            self.state.search_selection = 0;
                        }
                        EditorId::Fps => {
                            let limit = self.state.fps_limit;
                            self.editor_mut(id).edit.set_text(limit.to_string());
                        }
                        EditorId::Demo => {}
                    }
                    self.editor_mut(id).blur();
                    self.state.bump();
                    return;
                }
                PlatformKey::Named(name) if name == "Enter" => {
                    match id {
                        EditorId::Search => {
                            let hits = self.state.search_hits();
                            if let Some(hit) = hits.get(self.state.search_selection).copied() {
                                match hit {
                                    crate::state::SearchHit::Page(page) => self.navigate(page),
                                    crate::state::SearchHit::Component(component) => {
                                        self.navigate_to_component(component);
                                    }
                                }
                            }
                        }
                        EditorId::Fps => self.state.commit_fps_input(),
                        EditorId::Demo => {
                            self.editor_mut(id).blur();
                            self.state.bump();
                        }
                    }
                    return;
                }
                // Ctrl+V 需要宿主先读剪贴板，因此单独排队。
                PlatformKey::Character(text)
                    if modifiers.control && text.eq_ignore_ascii_case("v") =>
                {
                    self.pending_clipboard = Some(ClipboardRequest::Read);
                    return;
                }
                _ => {}
            }
            if let Some(event) = Self::key_to_edit_event(key, modifiers) {
                let _ = self.editor_event(id, event);
            }
            return;
        }
        if self.state.active_page == PageId::Components && !modifiers.control {
            let chord = match key {
                PlatformKey::Named(name) => match name.as_str() {
                    "ArrowLeft" => Some(unbound_input::KeyChord::new(
                        unbound_input::KeyName::Named("ArrowLeft"),
                    )),
                    "ArrowRight" => Some(unbound_input::KeyChord::new(
                        unbound_input::KeyName::Named("ArrowRight"),
                    )),
                    "ArrowUp" => Some(unbound_input::KeyChord::new(unbound_input::KeyName::Named(
                        "ArrowUp",
                    ))),
                    "ArrowDown" => Some(unbound_input::KeyChord::new(
                        unbound_input::KeyName::Named("ArrowDown"),
                    )),
                    "PageUp" => Some(unbound_input::KeyChord::new(unbound_input::KeyName::Named(
                        "PageUp",
                    ))),
                    "PageDown" => Some(unbound_input::KeyChord::new(
                        unbound_input::KeyName::Named("PageDown"),
                    )),
                    "Home" => Some(unbound_input::KeyChord::new(unbound_input::KeyName::Named(
                        "Home",
                    ))),
                    "End" => Some(unbound_input::KeyChord::new(unbound_input::KeyName::Named(
                        "End",
                    ))),
                    "Enter" => Some(unbound_input::KeyChord::new(unbound_input::KeyName::Named(
                        "Enter",
                    ))),
                    "Escape" => Some(unbound_input::KeyChord::new(unbound_input::KeyName::Named(
                        "Escape",
                    ))),
                    _ => None,
                },
                PlatformKey::Character(text) if text == " " => Some(unbound_input::KeyChord::new(
                    unbound_input::KeyName::Named("Space"),
                )),
                _ => None,
            };
            if let Some(chord) = chord {
                let handled = match self.state.active_component {
                    Some(WidgetCatalogId::CalendarView) => {
                        self.state.sandbox.calendar_view.handle_key(&chord)
                    }
                    Some(WidgetCatalogId::DateRangePicker) => {
                        self.state.sandbox.date_range_picker.handle_key(&chord)
                    }
                    _ => false,
                };
                if handled {
                    if self.state.active_component == Some(WidgetCatalogId::CalendarView) {
                        let dm = match self.state.sandbox.calendar_view.display_mode {
                            unbound_widgets::CalendarDisplayMode::Month => 0.0,
                            unbound_widgets::CalendarDisplayMode::Year => 1.0,
                            unbound_widgets::CalendarDisplayMode::Decade => 2.0,
                        };
                        let card =
                            pages::components::component_card_index(WidgetCatalogId::CalendarView);
                        self.state.set_knob(PageId::Components, card, 1, dm);
                    }
                    self.state.bump();
                    return;
                }
            }
        }
        if self.state.active_page == PageId::Parity && !modifiers.control {
            match key {
                PlatformKey::Named(name) if name == "ArrowDown" => {
                    if let Some(report) = self.state.demo.parity_report.as_ref() {
                        if self.state.parity_inspector.step_selection(
                            report,
                            self.state.search.text(),
                            1,
                        ) {
                            self.state.bump();
                        }
                    }
                    return;
                }
                PlatformKey::Named(name) if name == "ArrowUp" => {
                    if let Some(report) = self.state.demo.parity_report.as_ref() {
                        if self.state.parity_inspector.step_selection(
                            report,
                            self.state.search.text(),
                            -1,
                        ) {
                            self.state.bump();
                        }
                    }
                    return;
                }
                PlatformKey::Character(ch) if ch == "1" => {
                    self.state.parity_inspector.view_mode =
                        pages::parity_report::ParityViewMode::SideBySide;
                    self.state.bump();
                    return;
                }
                PlatformKey::Character(ch) if ch == "2" => {
                    self.state.parity_inspector.view_mode =
                        pages::parity_report::ParityViewMode::Curtain;
                    self.state.bump();
                    return;
                }
                PlatformKey::Character(ch) if ch == "3" => {
                    self.state.parity_inspector.view_mode =
                        pages::parity_report::ParityViewMode::Overlay;
                    self.state.bump();
                    return;
                }
                PlatformKey::Character(ch)
                    if (ch == " " || ch.eq_ignore_ascii_case("l"))
                        && self.state.parity_inspector.view_mode
                            == pages::parity_report::ParityViewMode::Curtain =>
                {
                    self.state.parity_inspector.curtain_locked =
                        !self.state.parity_inspector.curtain_locked;
                    self.state.bump();
                    return;
                }
                _ => {}
            }
        }
        if modifiers.control {
            if let PlatformKey::Character(text) = key {
                if text.eq_ignore_ascii_case("k") {
                    self.focus_editor(EditorId::Search, target::id(Target::Search));
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
                let nav_desc = build_navigation_view_desc(&self.state, self.layout.viewport, None);
                if self.nav_view.has_open_transient_surface(&nav_desc) {
                    self.nav_view.request_back(&nav_desc);
                    self.drain_navigation_view_events();
                }
                self.state.demo.modal_open = false;
                self.state.demo.dropdown_open = false;
                self.state.lang_dropdown_open = false;
                self.state.zoom_dropdown_open = false;
                self.state.text_render.family_dropdown_open = false;
                self.state.text_render.weight_dropdown_open = false;
                self.editor_mut(EditorId::Search)
                    .edit
                    .set_text(String::new());
                self.state.blur_editors();
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
        if node == NAV_SCROLLBAR_THUMB_NODE || node == NAV_SCROLLBAR_TRACK_NODE {
            return true;
        }
        matches!(
            target::decode(node),
            Some(Target::ContentScrollbar | Target::Knob(..) | Target::Demo(..))
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
        // 文本编辑器在按下时就定位插入符并开始拖拽选择；
        // 只有真实按压才捕获拖拽，点击报告绝不进入该状态。
        if let Some(id) = Self::editor_for_target(target) {
            self.focus_editor_at(id, target::id(target), position, false);
            self.drag_editor = Some((id, target::id(target)));
            return;
        }
        match target {
            Target::ContentScrollbar => {
                self.begin_scrollbar_drag(position);
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
        // 菜单打开时，点击菜单项执行动作，点击其他位置只负责关闭菜单。
        if self.menu.is_open() {
            if let Target::EditorMenuItem(index) = target {
                self.activate_editor_menu_item(usize::from(index));
            } else {
                self.close_editor_menu(unbound_widgets::context_menu::ContextClose::ClickOutside);
            }
            return;
        }
        if let Some(id) = Self::editor_for_target(target) {
            self.focus_editor_at(id, target::id(target), position, false);
            return;
        }
        match target {
            Target::NavPage(_)
            | Target::NavCategory(_)
            | Target::NavComponent(_)
            | Target::ContentNavPage(_)
            | Target::ContentNavCategory(_)
            | Target::ContentNavComponent(_) => {
                // 将内容区隔离目标归一化到对应的侧边栏导航项 ID，驱动 NavigationView 同步选中与展开状态。
                let sidebar_target = match target {
                    Target::ContentNavPage(page_idx) => Target::NavPage(page_idx),
                    Target::ContentNavCategory(cat_idx) => Target::NavCategory(cat_idx),
                    Target::ContentNavComponent(comp_idx) => Target::NavComponent(comp_idx),
                    other => other,
                };
                let nav_desc = build_navigation_view_desc(&self.state, self.layout.viewport, None);
                self.nav_view
                    .invoke_item(&nav_desc, target::id(sidebar_target));
                self.drain_navigation_view_events();
                // 首页、面包屑或分类索引页内直接点击的卡片可能不在当前折叠展开的树视图中，兜底确保直达。
                match sidebar_target {
                    Target::NavCategory(cat_idx) => {
                        if let Some(cat) = ControlCategory::from_index(cat_idx as usize) {
                            if self.state.active_page != PageId::Components
                                || self.state.active_category != Some(cat)
                                || self.state.active_component.is_some()
                            {
                                self.navigate_to_category(Some(cat));
                            }
                        }
                    }
                    Target::NavComponent(comp_idx) => {
                        if let Some(&id) = WidgetCatalogId::ALL.get(comp_idx as usize) {
                            if self.state.active_component != Some(id) {
                                self.navigate_to_component(id);
                            }
                        }
                    }
                    Target::NavPage(page_idx) => {
                        let page = PageId::from_index(page_idx as usize);
                        if self.state.active_page != page
                            || (page == PageId::Components
                                && (self.state.active_component.is_some()
                                    || self.state.active_category.is_some()))
                        {
                            self.navigate(page);
                        }
                    }
                    _ => {}
                }
            }
            Target::ControlFilter(filter) => {
                self.state.control_origin_filter = match filter {
                    1 => Some(ControlClass::Baseline),
                    2 => Some(ControlClass::Toolkit),
                    3 => Some(ControlClass::Exclusive),
                    _ => None,
                };
                self.state.content_scroll = 0.0;
                self.state.bump();
            }
            Target::FavoriteToggle(comp_idx) => {
                if let Some(&id) = WidgetCatalogId::ALL.get(comp_idx as usize) {
                    self.state.toggle_favorite(id);
                }
            }
            Target::HomeQuickTab(tab) => {
                self.state.home_quick_tab = tab.min(1);
                self.state.bump();
            }
            Target::NavToggle => {
                let nav_desc = build_navigation_view_desc(&self.state, self.layout.viewport, None);
                self.nav_view.toggle_pane(&nav_desc);
                self.drain_navigation_view_events();
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
            Target::ZoomMenu => {
                self.state.zoom_dropdown_open = !self.state.zoom_dropdown_open;
                self.state.lang_dropdown_open = false;
                self.state.bump();
            }
            Target::Zoom(step) => {
                self.state.zoom_step = (step as usize).min(ZOOM_STEPS.len() - 1);
                self.state.zoom_dropdown_open = false;
                self.state.bump();
            }
            Target::LangMenu => {
                self.state.lang_dropdown_open = !self.state.lang_dropdown_open;
                self.state.zoom_dropdown_open = false;
                self.state.bump();
            }
            Target::Lang(idx) => {
                self.state
                    .set_locale(unbound_app::Locale::from_index(idx as usize));
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
                self.focus_editor_at(
                    EditorId::Search,
                    target::id(Target::Search),
                    position,
                    false,
                );
            }
            Target::FpsPreset(index) => {
                let fps = [60, 120, 144, 240, 0][usize::from(index).min(4)];
                self.state.set_fps_limit(fps);
                self.state.fps_input.blur();
            }
            Target::FpsInput => {
                self.focus_editor_at(EditorId::Fps, target::id(Target::FpsInput), position, false);
            }
            Target::EditorMenuItem(_) => {}
            Target::Footer(_) => {
                let nav_desc = build_navigation_view_desc(&self.state, self.layout.viewport, None);
                self.nav_view.invoke_item(&nav_desc, target::id(target));
                self.drain_navigation_view_events();
            }
            Target::SearchResult(page) => self.navigate(PageId::from_index(page as usize)),
            Target::SearchComponent(index) => {
                if let Some(&id) = unbound_widgets::WidgetCatalogId::ALL.get(index as usize) {
                    self.navigate_to_component(id);
                }
            }
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
                        let next_val = if current > 0.5 { 0.0 } else { 1.0 };
                        self.state
                            .set_knob(page_id, card as usize, knob as usize, next_val);
                        if page_id == PageId::Components
                            && card as usize
                                == pages::components::component_card_index(
                                    WidgetCatalogId::CalendarView,
                                )
                        {
                            match knob {
                                3 => {
                                    self.state.sandbox.calendar_view.is_today_highlighted =
                                        next_val > 0.5;
                                }
                                4 => {
                                    self.state
                                        .sandbox
                                        .set_calendar_blackouts_enabled(next_val > 0.5);
                                }
                                _ => {}
                            }
                        }
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
                if PageId::from_index(page as usize) == PageId::Components
                    && card as usize
                        == pages::components::component_card_index(WidgetCatalogId::CalendarView)
                {
                    match knob {
                        0 => {
                            let mode = match option {
                                0 => unbound_widgets::CalendarSelectionMode::None,
                                2 => unbound_widgets::CalendarSelectionMode::Multiple,
                                3 => unbound_widgets::CalendarSelectionMode::Range,
                                _ => unbound_widgets::CalendarSelectionMode::Single,
                            };
                            self.state.sandbox.calendar_view.set_mode(mode);
                        }
                        1 => {
                            self.state.sandbox.calendar_view.display_mode = match option {
                                1 => unbound_widgets::CalendarDisplayMode::Year,
                                2 => unbound_widgets::CalendarDisplayMode::Decade,
                                _ => unbound_widgets::CalendarDisplayMode::Month,
                            };
                        }
                        2 => {
                            let first_day = if option == 1 { 1 } else { 0 };
                            self.state
                                .sandbox
                                .calendar_view
                                .set_first_day_of_week(first_day);
                        }
                        _ => {}
                    }
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
        if matches!(page, PageId::Sliders | PageId::Components) {
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
                    self.state.fps_input.blur();
                    return;
                }
                30 => {
                    let limit = self.state.fps_limit.to_string();
                    self.focus_editor(EditorId::Fps, target::id(Target::FpsInput));
                    self.state.fps_input.edit.set_text(limit);
                    self.state.fps_input.edit.select_all();
                    self.state.bump();
                    return;
                }
                _ => {}
            }
        }
        match page {
            PageId::Parity => {
                use pages::parity_report::{
                    ParityStatusFilter, ParityViewMode, BACKDROP_FILTER_PART, CASE_ROW_BASE,
                    CURTAIN_LOCK_PART, CURTAIN_STAGE_PART, OVERLAY_SOURCE_BASE, OVERLAY_SPLIT_BASE,
                    REFRESH_PART, RESET_FILTER_PART, SORT_MODE_PART, STATUS_FILTER_BASE,
                    THEME_FILTER_PART, VIEW_MODE_BASE, ZOOM_BASE,
                };
                if part == REFRESH_PART {
                    self.state.demo.parity_reload = true;
                    self.state.parity_inspector.initialized_for_report = false;
                    self.state.bump();
                } else if (STATUS_FILTER_BASE..STATUS_FILTER_BASE + 6).contains(&part) {
                    if let Some(f) = ParityStatusFilter::from_index(part - STATUS_FILTER_BASE) {
                        self.state.parity_inspector.status_filter = f;
                        self.state.parity_inspector.list_scroll = 0.0;
                        self.state.bump();
                    }
                } else if part == THEME_FILTER_PART {
                    self.state.parity_inspector.theme_filter =
                        self.state.parity_inspector.theme_filter.next();
                    self.state.parity_inspector.list_scroll = 0.0;
                    self.state.bump();
                } else if part == BACKDROP_FILTER_PART {
                    self.state.parity_inspector.backdrop_filter =
                        self.state.parity_inspector.backdrop_filter.next();
                    self.state.parity_inspector.list_scroll = 0.0;
                    self.state.bump();
                } else if part == SORT_MODE_PART {
                    self.state.parity_inspector.sort_mode =
                        self.state.parity_inspector.sort_mode.toggled();
                    self.state.bump();
                } else if part == RESET_FILTER_PART {
                    self.state.parity_inspector.reset_filters();
                    self.state.bump();
                } else if (VIEW_MODE_BASE..VIEW_MODE_BASE + 3).contains(&part) {
                    if let Some(m) = ParityViewMode::from_index(part - VIEW_MODE_BASE) {
                        self.state.parity_inspector.view_mode = m;
                        self.state.bump();
                    }
                } else if (ZOOM_BASE..ZOOM_BASE + 3).contains(&part) {
                    let zoom = [1u8, 2, 4][(part - ZOOM_BASE) as usize];
                    self.state.parity_inspector.zoom = zoom;
                    self.state.bump();
                } else if part == CURTAIN_STAGE_PART || part == CURTAIN_LOCK_PART {
                    self.state.parity_inspector.curtain_locked =
                        !self.state.parity_inspector.curtain_locked;
                    self.state.bump();
                } else if (OVERLAY_SOURCE_BASE..OVERLAY_SOURCE_BASE + 2).contains(&part) {
                    self.state.parity_inspector.overlay_show_heatmap =
                        (part - OVERLAY_SOURCE_BASE) == 1;
                    self.state.bump();
                } else if (OVERLAY_SPLIT_BASE..OVERLAY_SPLIT_BASE + 4).contains(&part) {
                    let split = [0.25f32, 0.50, 0.75, 1.0][(part - OVERLAY_SPLIT_BASE) as usize];
                    self.state.parity_inspector.overlay_split = split;
                    self.state.bump();
                } else if (CASE_ROW_BASE..REFRESH_PART).contains(&part) {
                    let row_idx = (part - CASE_ROW_BASE) as usize;
                    if let Some(report) = self.state.demo.parity_report.as_ref() {
                        let filtered = pages::parity_report::filtered_case_indices(
                            report,
                            &self.state.parity_inspector,
                            self.state.search.text(),
                        );
                        if let Some(&case_idx) = filtered.get(row_idx) {
                            self.state.parity_inspector.selected_case =
                                Some(report.cases[case_idx].name.clone());
                            self.state.bump();
                        }
                    }
                }
            }
            PageId::Home => match card {
                0 => match part {
                    1 => self.navigate_to_category(None),
                    2 => self.navigate(PageId::Viewport),
                    3 => self.navigate(PageId::Parity),
                    _ => {}
                },
                1 => {
                    if let Some((_, _, target, _)) =
                        pages::home::CAPABILITIES.get(part.saturating_sub(1) as usize)
                    {
                        self.navigate(*target);
                    }
                }
                2 => {
                    self.state.home_quick_tab = (part as u8).min(1);
                    self.state.bump();
                }
                3 => {
                    if let Some(cat) = ControlCategory::from_index(part as usize) {
                        self.navigate_to_category(Some(cat));
                    }
                }
                4 => {
                    if let Some(&id) = WidgetCatalogId::ALL.get(part as usize) {
                        self.navigate_to_component(id);
                    }
                }
                5 => {
                    if let Some(&id) = WidgetCatalogId::ALL.get(part as usize) {
                        self.state.toggle_favorite(id);
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
                    let node = target::id(Target::Demo(page.index() as u8, card, part));
                    self.focus_editor(EditorId::Demo, node);
                    self.state.bump();
                }
            }
            PageId::Fonts => {
                // BiDi 沙盒的自定义预设是真实的文本输入框：IME 必须像文本域页面那样
                // 组合进 `demo.text`，否则中日韩 / 阿拉伯语输入法永远只能输入 ASCII。
                //
                //
                if card == 2 && part == 1 {
                    let node = target::id(Target::Demo(page.index() as u8, card, part));
                    self.focus_editor(EditorId::Demo, node);
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
                        let stage_rect = self.state.sandbox.stage_rect(id);
                        if crate::state::reduce_showcase_component(
                            id,
                            &mut self.state.sandbox,
                            Some(position),
                            stage_rect,
                            part,
                        ) {
                            if id == WidgetCatalogId::CalendarView {
                                let dm = match self.state.sandbox.calendar_view.display_mode {
                                    unbound_widgets::CalendarDisplayMode::Month => 0.0,
                                    unbound_widgets::CalendarDisplayMode::Year => 1.0,
                                    unbound_widgets::CalendarDisplayMode::Decade => 2.0,
                                };
                                self.state
                                    .set_knob(PageId::Components, card as usize, 1, dm);
                            }
                            self.state.bump();
                        }
                    }
                }
            }
            PageId::Systems => {
                if card == 0 && (100..100 + pages::SubsystemId::COUNT as u16).contains(&part) {
                    self.state.demo.selected_subsystem = (part - 100) as usize;
                    self.state.bump();
                }
            }
            PageId::Backdrop => {
                if card == 0 {
                    // 部件编号由后景页定义：材质行 `1..=6`，来源行 `7..=9`。
                    //
                    // 两段互不重叠，材质行按材质种类顺序、来源行按来源顺序排列。
                    if (pages::backdrop::MATERIAL_PART_BASE
                        ..pages::backdrop::MATERIAL_PART_BASE + MaterialKind::ALL.len() as u16)
                        .contains(&part)
                    {
                        let index = (part - pages::backdrop::MATERIAL_PART_BASE) as usize;
                        self.state
                            .set_backdrop_material(MaterialKind::from_index(index));
                    } else if (pages::backdrop::SOURCE_PART_BASE
                        ..pages::backdrop::SOURCE_PART_BASE + BackdropSource::ALL.len() as u16)
                        .contains(&part)
                    {
                        let index = (part - pages::backdrop::SOURCE_PART_BASE) as usize;
                        self.state
                            .set_backdrop_source(BackdropSource::from_index(index));
                    }
                }
            }
            PageId::I18n => {
                if card == 0 {
                    match part {
                        1 => self.state.set_locale(unbound_app::Locale::EnUs),
                        2 => self.state.set_locale(unbound_app::Locale::ZhCn),
                        3 => {
                            self.state.demo.i18n_counter =
                                self.state.demo.i18n_counter.saturating_add(1);
                            self.state.bump();
                        }
                        4 => {
                            self.state.demo.i18n_counter = 0;
                            self.state.bump();
                        }
                        _ => {}
                    }
                }
            }
            PageId::TextRender => {
                if card == 0 {
                    // 折叠标题按钮点击：GROUP_PART_BASE .. GROUP_PART_BASE + 6
                    if (pages::text_render::GROUP_PART_BASE
                        ..pages::text_render::GROUP_PART_BASE
                            + pages::text_render::TextConsoleGroup::ALL.len() as u16)
                        .contains(&part)
                    {
                        let group_index = (part - pages::text_render::GROUP_PART_BASE) as usize;
                        self.state.text_render.toggle_group(group_index);
                        self.state.bump();
                    } else if part == pages::text_render::STATUS_BAR_PART
                        || part == pages::text_render::STATUS_FALLBACK_CELL_PART
                    {
                        self.state.text_render.toggle_fallback_details();
                        self.state.bump();
                    } else if part == pages::text_render::PART_PRESET_WINUI {
                        let caps = unbound_text::TextCapabilities::probe();
                        self.state
                            .text_render
                            .apply_preset(pages::text_render::TextRenderPreset::WinuiNative, &caps);
                        self.labels.cache.apply_settings(
                            &self.state.text_render.settings,
                            Some(&mut self.labels.engine),
                        );
                        self.state.text_render.report = self.labels.cache.report();
                        self.state.bump();
                    } else if part == pages::text_render::PART_PRESET_CROSS_PLATFORM {
                        let caps = unbound_text::TextCapabilities::probe();
                        self.state.text_render.apply_preset(
                            pages::text_render::TextRenderPreset::CrossPlatform,
                            &caps,
                        );
                        self.labels.cache.apply_settings(
                            &self.state.text_render.settings,
                            Some(&mut self.labels.engine),
                        );
                        self.state.text_render.report = self.labels.cache.report();
                        self.state.bump();
                    } else if part == pages::text_render::PART_PRESET_PIXEL {
                        let caps = unbound_text::TextCapabilities::probe();
                        self.state
                            .text_render
                            .apply_preset(pages::text_render::TextRenderPreset::Pixel, &caps);
                        self.labels.cache.apply_settings(
                            &self.state.text_render.settings,
                            Some(&mut self.labels.engine),
                        );
                        self.state.text_render.report = self.labels.cache.report();
                        self.state.bump();
                    } else if part == pages::text_render::PART_PRESET_CUSTOM {
                        self.state.text_render.mark_custom();
                        self.state.text_render.persist();
                        self.state.bump();
                    } else if part == pages::text_render::PART_BACKEND_DW {
                        let caps = unbound_text::TextCapabilities::probe();
                        let res = pages::text_render::resolve_feature(
                            pages::text_render::TextFeatureKey::BackendDirectWrite,
                            &caps,
                            &self.state.text_render.settings,
                        );
                        if !res.disabled {
                            self.state.text_render.settings.raster_backend =
                                unbound_text::RasterBackend::DirectWrite;
                            self.labels.cache.apply_settings(
                                &self.state.text_render.settings,
                                Some(&mut self.labels.engine),
                            );
                            self.state.text_render.report = self.labels.cache.report();
                            self.state.bump();
                        }
                    } else if part == pages::text_render::PART_BACKEND_COSMIC {
                        let caps = unbound_text::TextCapabilities::probe();
                        let res = pages::text_render::resolve_feature(
                            pages::text_render::TextFeatureKey::BackendCosmic,
                            &caps,
                            &self.state.text_render.settings,
                        );
                        if !res.disabled {
                            self.state.text_render.settings.raster_backend =
                                unbound_text::RasterBackend::Cosmic;
                            self.labels.cache.apply_settings(
                                &self.state.text_render.settings,
                                Some(&mut self.labels.engine),
                            );
                            self.state.text_render.report = self.labels.cache.report();
                            self.state.bump();
                        }
                    } else if part == pages::text_render::FONT_FAMILY_TRIGGER_PART {
                        self.state.text_render.family_dropdown_open =
                            !self.state.text_render.family_dropdown_open;
                        self.state.text_render.weight_dropdown_open = false;
                        self.state.bump();
                    } else if part == pages::text_render::FONT_WEIGHT_TRIGGER_PART {
                        self.state.text_render.weight_dropdown_open =
                            !self.state.text_render.weight_dropdown_open;
                        self.state.text_render.family_dropdown_open = false;
                        self.state.bump();
                    } else if (pages::text_render::FONT_FAMILY_PART_BASE
                        ..pages::text_render::FONT_FAMILY_PART_BASE
                            + pages::text_render::FONT_FAMILY_OPTIONS.len() as u16)
                        .contains(&part)
                    {
                        let index = (part - pages::text_render::FONT_FAMILY_PART_BASE) as usize;
                        let (_, fam) = pages::text_render::FONT_FAMILY_OPTIONS[index];
                        self.state.text_render.set_family(fam.map(str::to_string));
                        self.state.text_render.family_dropdown_open = false;
                        self.state.bump();
                    } else if (pages::text_render::FONT_WEIGHT_PART_BASE
                        ..pages::text_render::FONT_WEIGHT_PART_BASE
                            + pages::text_render::FONT_WEIGHT_OPTIONS.len() as u16)
                        .contains(&part)
                    {
                        let index = (part - pages::text_render::FONT_WEIGHT_PART_BASE) as usize;
                        let (_, weight) = pages::text_render::FONT_WEIGHT_OPTIONS[index];
                        self.state.text_render.set_weight(weight);
                        self.state.text_render.weight_dropdown_open = false;
                        self.state.bump();
                    } else if (pages::text_render::FONT_SIZE_PART_BASE
                        ..pages::text_render::FONT_SIZE_PART_BASE
                            + pages::text_render::FONT_SIZE_OPTIONS.len() as u16)
                        .contains(&part)
                    {
                        let index = (part - pages::text_render::FONT_SIZE_PART_BASE) as usize;
                        let (_, size) = pages::text_render::FONT_SIZE_OPTIONS[index];
                        self.state.text_render.set_size(size);
                        self.state.bump();
                    } else if part == pages::text_render::FONT_ITALIC_PART {
                        self.state.text_render.toggle_italic();
                        self.state.bump();
                    } else if (pages::text_render::PHASE_PART_BASE
                        ..pages::text_render::PHASE_PART_BASE + 3)
                        .contains(&part)
                    {
                        let opt_idx = (part - pages::text_render::PHASE_PART_BASE) as usize;
                        if let Some(opt) = pages::text_render::PHASE_OPTIONS.get(opt_idx) {
                            let caps = unbound_text::TextCapabilities::probe();
                            let res = pages::text_render::resolve_feature(
                                opt.key,
                                &caps,
                                &self.state.text_render.settings,
                            );
                            if !res.disabled {
                                self.state.text_render.settings.phase_mode = opt.mode;
                                self.state.bump();
                            }
                        }
                    } else {
                        match part {
                            pages::text_render::PART_DF_OFF => {
                                self.state.text_render.settings.distance_field =
                                    unbound_text::DistanceFieldMode::Off;
                                self.state.text_render.settings.use_sdf = false;
                                self.state.bump();
                            }
                            pages::text_render::PART_DF_SDF => {
                                self.state.text_render.settings.distance_field =
                                    unbound_text::DistanceFieldMode::Sdf;
                                self.state.text_render.settings.use_sdf = true;
                                self.state.text_render.settings.phase_mode =
                                    unbound_text::PhaseMode::One;
                                self.state.bump();
                            }
                            pages::text_render::PART_DF_MTSDF => {}
                            pages::text_render::PART_COMP_SRGB => {
                                self.state.text_render.settings.blend_space =
                                    unbound_text::BlendSpace::Srgb;
                                self.state.bump();
                            }
                            pages::text_render::PART_COMP_LINEAR => {
                                self.state.text_render.settings.blend_space =
                                    unbound_text::BlendSpace::Linear;
                                self.state.bump();
                            }
                            pages::text_render::PART_COMP_LAYER_DIRECT => {
                                self.state.text_render.settings.layer_mode =
                                    unbound_text::LayerMode::Direct;
                                self.state.bump();
                            }
                            pages::text_render::PART_COMP_LAYER_ISOLATED => {
                                self.state.text_render.settings.layer_mode =
                                    unbound_text::LayerMode::Isolated;
                                self.state.bump();
                            }
                            pages::text_render::PART_SHAPER_COSMIC => {
                                self.state.text_render.settings.shaper =
                                    unbound_text::TextShaper::Cosmic;
                                self.state.bump();
                            }
                            pages::text_render::PART_SHAPER_NATIVE => {
                                // 禁用项，不生效
                            }
                            pages::text_render::PART_LINEBREAK_WORD => {
                                self.state.text_render.settings.line_break =
                                    unbound_text::LineBreakMode::Word;
                                self.state.bump();
                            }
                            pages::text_render::PART_LINEBREAK_ANYWHERE => {
                                self.state.text_render.settings.line_break =
                                    unbound_text::LineBreakMode::Anywhere;
                                self.state.bump();
                            }
                            pages::text_render::PART_LINEBREAK_NONE => {
                                self.state.text_render.settings.line_break =
                                    unbound_text::LineBreakMode::None;
                                self.state.bump();
                            }
                            pages::text_render::PART_BIDI_ON => {
                                self.state.text_render.settings.bidi = true;
                                self.state.bump();
                            }
                            pages::text_render::PART_BIDI_OFF => {
                                self.state.text_render.settings.bidi = false;
                                self.state.bump();
                            }
                            pages::text_render::PART_LIGATURES_ON => {
                                self.state.text_render.settings.ligatures = true;
                                self.state.bump();
                            }
                            pages::text_render::PART_LIGATURES_OFF => {
                                self.state.text_render.settings.ligatures = false;
                                self.state.bump();
                            }
                            pages::text_render::PART_LETTER_SPACING_0 => {
                                self.state.text_render.settings.letter_spacing = 0.0;
                                self.state.bump();
                            }
                            pages::text_render::PART_LETTER_SPACING_1 => {
                                self.state.text_render.settings.letter_spacing = 1.0;
                                self.state.bump();
                            }
                            pages::text_render::PART_LETTER_SPACING_2 => {
                                self.state.text_render.settings.letter_spacing = 2.0;
                                self.state.bump();
                            }
                            pages::text_render::PART_LINE_HEIGHT_AUTO => {
                                self.state.text_render.settings.line_height = 0.0;
                                self.state.bump();
                            }
                            pages::text_render::PART_LINE_HEIGHT_20 => {
                                self.state.text_render.settings.line_height = 20.0;
                                self.state.bump();
                            }
                            pages::text_render::PART_LINE_HEIGHT_28 => {
                                self.state.text_render.settings.line_height = 28.0;
                                self.state.bump();
                            }
                            pages::text_render::PART_EMOJI_PRES_COLOR => {
                                let caps = unbound_text::TextCapabilities::probe();
                                let res = pages::text_render::resolve_feature(
                                    pages::text_render::TextFeatureKey::EmojiPresentationColor,
                                    &caps,
                                    &self.state.text_render.settings,
                                );
                                if !res.disabled {
                                    self.state.text_render.settings.emoji_presentation =
                                        unbound_text::EmojiPresentation::Color;
                                    self.labels.cache.apply_settings(
                                        &self.state.text_render.settings,
                                        Some(&mut self.labels.engine),
                                    );
                                    self.state.text_render.report = self.labels.cache.report();
                                    self.state.bump();
                                }
                            }
                            pages::text_render::PART_EMOJI_PRES_MONO => {
                                self.state.text_render.settings.emoji_presentation =
                                    unbound_text::EmojiPresentation::Mono;
                                self.labels.cache.apply_settings(
                                    &self.state.text_render.settings,
                                    Some(&mut self.labels.engine),
                                );
                                self.state.text_render.report = self.labels.cache.report();
                                self.state.bump();
                            }
                            pages::text_render::PART_EMOJI_BACKEND_FOLLOW => {
                                self.state.text_render.settings.emoji_backend =
                                    unbound_text::EmojiBackend::Follow;
                                self.labels.cache.apply_settings(
                                    &self.state.text_render.settings,
                                    Some(&mut self.labels.engine),
                                );
                                self.state.text_render.report = self.labels.cache.report();
                                self.state.bump();
                            }
                            pages::text_render::PART_EMOJI_BACKEND_DW => {
                                let caps = unbound_text::TextCapabilities::probe();
                                let res = pages::text_render::resolve_feature(
                                    pages::text_render::TextFeatureKey::EmojiBackendDirectWrite,
                                    &caps,
                                    &self.state.text_render.settings,
                                );
                                if !res.disabled {
                                    self.state.text_render.settings.emoji_backend =
                                        unbound_text::EmojiBackend::DirectWrite;
                                    self.labels.cache.apply_settings(
                                        &self.state.text_render.settings,
                                        Some(&mut self.labels.engine),
                                    );
                                    self.state.text_render.report = self.labels.cache.report();
                                    self.state.bump();
                                }
                            }
                            pages::text_render::PART_EMOJI_BACKEND_COSMIC => {
                                self.state.text_render.settings.emoji_backend =
                                    unbound_text::EmojiBackend::Cosmic;
                                self.labels.cache.apply_settings(
                                    &self.state.text_render.settings,
                                    Some(&mut self.labels.engine),
                                );
                                self.state.text_render.report = self.labels.cache.report();
                                self.state.bump();
                            }
                            pages::text_render::PART_EMOJI_SOURCE_SYSTEM => {
                                self.state.text_render.settings.emoji_source =
                                    unbound_text::EmojiSource::System;
                                self.state.bump();
                            }
                            pages::text_render::PART_EMOJI_SOURCE_BUILTIN
                            | pages::text_render::PART_EMOJI_SOURCE_GRID => {
                                // 禁用项，不生效
                            }
                            _ => {}
                        }
                    }

                    // 若改动的不是预设按钮和折叠标题/状态栏，确保标记为自定义并持久化
                    let is_preset_or_meta = matches!(
                        part,
                        pages::text_render::STATUS_BAR_PART
                            | pages::text_render::PART_PRESET_WINUI
                            | pages::text_render::PART_PRESET_CROSS_PLATFORM
                            | pages::text_render::PART_PRESET_PIXEL
                            | pages::text_render::PART_PRESET_CUSTOM
                    ) || (pages::text_render::GROUP_PART_BASE
                        ..pages::text_render::GROUP_PART_BASE
                            + pages::text_render::TextConsoleGroup::ALL.len() as u16)
                        .contains(&part);
                    if !is_preset_or_meta {
                        self.state.text_render.mark_custom();
                        self.state.text_render.persist();
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
            let target_card = (1..=4u16)
                .filter_map(|part| {
                    demo_rect(self, PageId::DragDrop, 0, part).map(|rect| (part as usize - 1, rect))
                })
                .min_by(|(_, a), (_, b)| {
                    let da = (a.center() - position).length_squared();
                    let db = (b.center() - position).length_squared();
                    da.total_cmp(&db)
                })
                .map_or(index, |(card_id, _)| card_id);
            let mut order = self.state.demo.drag_order.clone();
            let target_slot = order
                .iter()
                .position(|&value| value == target_card)
                .unwrap_or(index);
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
            Some(Target::Demo(page, card, _)) if page as usize == PageId::Components.index() => {
                if let Some(example) = PageId::Components.examples().get(card as usize) {
                    if let pages::DemoKind::Component { id } = example.demo {
                        self.state.sandbox.slider_values.insert(id as u8, value);
                        self.state.bump();
                    }
                }
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
        if card != 0 {
            return 32.0;
        }
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
        let right_bound = if self.layout.lang.size.x > 0.0 {
            self.layout.lang.origin.x
        } else if self.layout.theme.size.x > 0.0 {
            self.layout.theme.origin.x
        } else {
            self.layout.zoom.origin.x
        };
        if right_bound > 292.0 {
            text_node(
                &mut builder,
                Rect::new(52.0, 14.0, 190.0, 28.0),
                "Unbound UI Gallery",
            );
        }
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

        // 导航窗格（由 `NavigationView` 布局结果驱动）。
        let nav_rect = self
            .nav_layout
            .overlay_rect
            .unwrap_or(self.nav_layout.pane_rect);
        if nav_rect.size.x > 0.0 && nav_rect.size.y > 0.0 {
            builder.open(NodeKind::Container, nav_rect, "NavigationView");
            builder.set_clip(nav_rect);
            builder.set_scrollable(true);
            for row in &self.nav_layout.rows {
                let (label, clip_bounds) = match &row.kind {
                    NavigationViewRowKind::Item { item, .. } => (
                        format!("Navigation \"{}\"", item.content),
                        self.nav_layout.menu_viewport_rect.intersect(&nav_rect),
                    ),
                    NavigationViewRowKind::FooterItem { item } => (
                        format!("Navigation \"{}\"", item.content),
                        self.nav_layout.footer_rect.intersect(&nav_rect),
                    ),
                    NavigationViewRowKind::Settings => (
                        "Navigation \"Settings\"".to_string(),
                        self.nav_layout.footer_rect.intersect(&nav_rect),
                    ),
                    NavigationViewRowKind::Header { .. }
                    | NavigationViewRowKind::Separator { .. } => continue,
                };
                let visible = row.rect.intersect(&clip_bounds);
                if visible.size.y >= 24.0 && visible.size.x >= 24.0 {
                    builder.open(interactive_rect(visible), visible, &label);
                    builder.close();
                }
            }
            builder.close();
        }

        // 页面内容。
        builder.open(NodeKind::Container, self.layout.content, "Page");
        builder.set_clip(self.layout.content);
        builder.set_scrollable(true);
        if let Some(page) = &self.page_layout {
            let info = page.page.localized_info(self.state.locale);
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
        if self.state.search.focused {
            let overlay = search_overlay_rect(&self.layout, self.state.search_hits().len());
            builder.open(NodeKind::Portal, overlay, "Command palette");
            builder.set_overlays_siblings(true);
            let hits = self.state.search_hits();
            let count = hits.len().max(1);
            for index in 0..count {
                let row = Rect::new(
                    overlay.origin.x + 6.0,
                    overlay.origin.y + 30.0 + index as f32 * 36.0,
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
        if self.state.lang_dropdown_open && self.layout.lang.size.x >= 24.0 {
            let items = unbound_app::Locale::ALL.len();
            let menu = dropdown_panel_rect(
                self.layout.lang,
                LANG_MENU_WIDTH,
                items,
                self.layout.viewport,
            );
            builder.open(NodeKind::Portal, menu, "Language menu");
            builder.set_overlays_siblings(true);
            for idx in 0..items {
                let row = Rect::new(
                    menu.origin.x + 4.0,
                    menu.origin.y + 4.0 + idx as f32 * widgets::MENU_ROW_HEIGHT,
                    menu.size.x - 8.0,
                    widgets::MENU_ROW_HEIGHT,
                );
                builder.open(interactive_rect(row), row, "Language option");
                builder.close();
            }
            builder.close();
        }
        if self.state.zoom_dropdown_open && self.layout.zoom.size.x >= 24.0 {
            let items = ZOOM_STEPS.len();
            let menu = dropdown_panel_rect(
                self.layout.zoom,
                ZOOM_MENU_WIDTH,
                items,
                self.layout.viewport,
            );
            builder.open(NodeKind::Portal, menu, "Scale menu");
            builder.set_overlays_siblings(true);
            for idx in 0..items {
                let row = Rect::new(
                    menu.origin.x + 4.0,
                    menu.origin.y + 4.0 + idx as f32 * widgets::MENU_ROW_HEIGHT,
                    menu.size.x - 8.0,
                    widgets::MENU_ROW_HEIGHT,
                );
                builder.open(interactive_rect(row), row, "Scale option");
                builder.close();
            }
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
                for index in 0..6 {
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
        (layout.lang, "Language selector"),
        (layout.theme, "Theme selector"),
        (layout.zoom, "Zoom selector"),
        (layout.snap, "Button \"Pixel Snap\""),
        (layout.hud, "Button \"Toggle live FPS HUD overlay\""),
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

fn search_overlay_rect(layout: &ShellLayout, count: usize) -> Rect {
    let width = layout.search.size.x.max(380.0);
    let x = layout
        .search
        .origin
        .x
        .min(layout.viewport.max().x - width - 14.0)
        .max(14.0);
    let results = (count.max(1) as f32).min(6.0);
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

fn draw_background(canvas: &mut Canvas<'_>, layout: &ShellLayout, mica_visible: bool) {
    let palette = *canvas.palette;
    // `Token::NONE` 是引擎保留的透明槽位：云母可见时让根背景完全透明，
    // 壁纸纹理才能从最底层透出。
    let fill = if mica_visible {
        unbound_theme::Token::NONE.raw()
    } else {
        palette.background
    };
    canvas.rect(layout.viewport, 0.0, fill, 0, 0.0, 0.0);
}

fn dropdown_panel_rect(anchor: Rect, width: f32, count: usize, viewport: Rect) -> Rect {
    let row_h = widgets::MENU_ROW_HEIGHT;
    let menu_w = width.max(anchor.size.x);
    let menu_h = count as f32 * row_h + 8.0;
    let x = (anchor.max().x - menu_w)
        .max(8.0)
        .min(viewport.max().x - menu_w - 8.0);
    let y = anchor.max().y + 6.0;
    Rect::new(x, y, menu_w, menu_h)
}

fn draw_top_bar(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    layout: &ShellLayout,
    window_maximized: bool,
    nav_sync: NavigationViewTitleBarSync,
    atlas_clock: u64,
) {
    let palette = *canvas.palette;
    canvas.rect(layout.top_bar, 0.0, palette.header_bg, 0, 0.0, 0.0);
    canvas.divider(0.0, layout.top_bar.max().y - 1.0, layout.top_bar.size.x);

    if nav_sync.is_pane_toggle_button_visible {
        widgets::icon_button(
            canvas,
            target::id(Target::NavToggle),
            layout.nav_toggle,
            Icon::Menu,
            state.t("a11y.toggle_nav"),
            !nav_sync.is_pane_open,
        );
    }

    let logo = Rect::new(52.0, 14.0, 28.0, 28.0);
    widgets::brand_badge(canvas, logo, "U");
    // 右侧工具区最左边界：按 语言 -> 主题 -> 缩放 顺序回退，防止窄视口下标题或面包屑撞进控件。
    let right_bound = if layout.lang.size.x > 0.0 {
        layout.lang.origin.x
    } else if layout.theme.size.x > 0.0 {
        layout.theme.origin.x
    } else {
        layout.zoom.origin.x
    };
    if right_bound > 292.0 {
        canvas.text(
            state.t("app.title"),
            Rect::new(88.0, 12.0, 190.0, 32.0),
            13.5,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
    }

    if layout.wide {
        let info = state.active_page.localized_info(state.locale);
        let category = state.active_page.category().localized_label(state.locale);
        let crumb_rect = Rect::new(220.0, 12.0, (right_bound - 240.0).max(0.0), 32.0);
        if category == info.short {
            widgets::breadcrumb(canvas, crumb_rect, &[info.short]);
        } else {
            widgets::breadcrumb(canvas, crumb_rect, &[category, info.short]);
        }
    }

    let search = layout.search;
    if search.size.x >= 100.0 {
        let focused = state.search.focused;
        let start = canvas.span_start();
        canvas.rect(
            search,
            8.0,
            palette.card_bg,
            if focused {
                palette.primary
            } else {
                palette.card_border
            },
            if focused { 2.0 } else { 1.0 },
            0.0,
        );
        widgets::icon(
            canvas,
            Icon::Search,
            Rect::new(search.origin.x + 4.0, search.origin.y, 28.0, search.size.y),
            palette.text_muted,
            palette.card_bg,
        );
        let text_x = search.origin.x + 36.0;
        let view_width = (search.size.x - 84.0).max(24.0);
        state.set_editor_view(EditorId::Search, view_width);
        let edit = &state.search.edit;
        let text = edit.text();
        let scroll = edit.scroll_x();
        let line = Rect::new(
            text_x,
            search.origin.y + 6.0,
            view_width,
            (search.size.y - 12.0).max(8.0),
        );
        if focused {
            if let Some((from, to)) = crate::editor::selection_span(canvas.labels, edit, 11.0) {
                canvas.rect(
                    Rect::new(
                        line.origin.x + from - scroll,
                        line.origin.y,
                        (to - from).max(1.0),
                        line.size.y,
                    ),
                    2.0,
                    palette.selection_fill,
                    0,
                    0.0,
                    0.0,
                );
            }
        }
        let placeholder = if text.is_empty() {
            if search.size.x < 280.0 {
                state.t("top.search.short")
            } else {
                state.t("top.search.placeholder")
            }
        } else {
            text
        };
        canvas.begin_clip(Rect::new(
            text_x,
            search.origin.y,
            view_width,
            search.size.y,
        ));
        let text_width = canvas.labels.measure(placeholder, 11.0).x.max(view_width);
        let text_rect = Rect::new(text_x - scroll, search.origin.y, text_width, search.size.y);
        canvas.text(
            placeholder,
            text_rect,
            11.0,
            if text.is_empty() {
                palette.text_muted
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
        let caret = if focused {
            Some(text_x - scroll + crate::editor::caret_x(canvas.labels, text, edit.cursor(), 11.0))
        } else {
            None
        };
        // IME 组合文本带下划线，与原生文本框的展示方式一致。
        //
        if let Some(underline) =
            crate::editor::preedit_underline(canvas.labels, edit, 11.0, text_rect)
        {
            canvas.rect(underline, 0.5, palette.primary, 0, 0.0, 0.0);
        }
        if let Some(caret) = caret {
            if state.search.caret_visible() {
                canvas.rect(
                    Rect::new(caret, line.origin.y, 1.0, line.size.y),
                    0.5,
                    palette.text,
                    0,
                    0.0,
                    0.0,
                );
            }
            state.set_ime_anchor(Vec2::new(caret, search.max().y));
        }
        canvas.end_clip();
        if layout.search.size.x > 280.0 {
            widgets::badge_right_aligned(
                canvas,
                search.max().x - 8.0,
                search.origin.y + 7.0,
                18.0,
                "Ctrl K",
                palette.text_muted,
                palette.stage_bg,
                10.0,
            );
        }
        canvas.control(
            target::id(Target::Search),
            search,
            Role::Search,
            state.t("a11y.search"),
            CursorKind::Text,
            false,
            focused,
        );
        canvas.record_span(target::id(Target::Search), start);
    } else if search.size.x > 0.0 {
        // 窄窗口保留放大镜作为打开同一命令面板的按钮。
        // 若完全丢弃搜索，150–200% 缩放下将完全没有指针入口。
        //
        widgets::icon_button(
            canvas,
            target::id(Target::Search),
            search,
            Icon::Search,
            state.t("a11y.search"),
            state.search.focused,
        );
    }

    // 语言下拉选择控件（默认 English，支持切换简体中文）。
    if layout.lang.size.x >= 24.0 {
        let lang_label = if layout.compact {
            state.locale.short_label()
        } else {
            state.locale.native_name()
        };
        widgets::select_trigger(
            canvas,
            target::id(Target::LangMenu),
            layout.lang,
            lang_label,
            state.t("a11y.lang_selector"),
            state.lang_dropdown_open,
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
            state.t("a11y.toggle_theme"),
            false,
        );
    } else {
        let ids = [target::id(Target::Theme(0)), target::id(Target::Theme(1))];
        let light_label = state.t("top.theme.light");
        let dark_label = state.t("top.theme.dark");
        widgets::segmented(
            canvas,
            &ids,
            layout.theme,
            &[light_label, dark_label],
            if state.theme.is_dark() { 1 } else { 0 },
            false,
            None,
        );
    }

    // 窗口/界面大小缩放下拉选择控件（替代原先占地方的 4 段式胶囊）。
    if layout.zoom.size.x >= 24.0 {
        let zoom_labels = ["100%", "125%", "150%", "200%"];
        let zoom_label = zoom_labels.get(state.zoom_step).copied().unwrap_or("100%");
        widgets::select_trigger(
            canvas,
            target::id(Target::ZoomMenu),
            layout.zoom,
            zoom_label,
            state.t("a11y.zoom_selector"),
            state.zoom_dropdown_open,
        );
    }

    widgets::icon_button(
        canvas,
        target::id(Target::Snap),
        layout.snap,
        Icon::Snap,
        state.t("a11y.pixel_snap"),
        state.pixel_snap,
    );
    widgets::icon_button(
        canvas,
        target::id(Target::Hud),
        layout.hud,
        Icon::Chart,
        state.t("a11y.hud"),
        state.show_hud,
    );

    // 客户端窗口按钮（ADR-0029 独立 WindowCaptionButtons 发射器）。
    draw_window_controls(canvas, state, layout, window_maximized, atlas_clock);
}

fn draw_top_bar_dropdowns(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout) {
    if state.lang_dropdown_open && layout.lang.size.x >= 24.0 {
        let options = [
            ("English (en-US)", Target::Lang(0)),
            ("简体中文 (zh-CN)", Target::Lang(1)),
        ];
        let panel =
            dropdown_panel_rect(layout.lang, LANG_MENU_WIDTH, options.len(), layout.viewport);
        widgets::dropdown_menu(canvas, panel, &options, state.locale.index());
    }
    if state.zoom_dropdown_open && layout.zoom.size.x >= 24.0 {
        let options = [
            ("100% (1.0×)", Target::Zoom(0)),
            ("125% (1.25×)", Target::Zoom(1)),
            ("150% (1.5×)", Target::Zoom(2)),
            ("200% (2.0×)", Target::Zoom(3)),
        ];
        let panel =
            dropdown_panel_rect(layout.zoom, ZOOM_MENU_WIDTH, options.len(), layout.viewport);
        widgets::dropdown_menu(canvas, panel, &options, state.zoom_step);
    }
}

/// 编辑器上下文菜单的绘制载荷：描述符与当前键盘高亮项。
type EditorMenuPaint = (unbound_widgets::menu::MenuDesc, Option<usize>);

/// 在 Portal 层级绘制编辑器上下文菜单。
///
/// 每个可操作行都会注册命中区域与 `MenuItem` 语义节点，使指针点击与
/// `Up`/`Down`/`Enter` 键盘导航复用同一份几何。
fn draw_editor_menu(canvas: &mut Canvas<'_>, paint: Option<&EditorMenuPaint>) {
    use unbound_widgets::menu::{
        emit_menu_with_rows, menu_item_node, MenuItemRenderRow, MenuState, MENU_LAYER,
    };

    let Some((desc, highlighted)) = paint else {
        return;
    };
    let highlighted = *highlighted;
    let mut shaped_labels: Vec<Vec<unbound_text::GlyphQuad>> = Vec::with_capacity(desc.items.len());
    let mut shaped_shortcuts: Vec<Vec<unbound_text::GlyphQuad>> =
        Vec::with_capacity(desc.items.len());
    let mut phases: Vec<unbound_widgets::PointerPhase> = Vec::with_capacity(desc.items.len());
    for (index, item) in desc.items.iter().enumerate() {
        let row = desc.item_rect(index);
        if item.separator {
            shaped_labels.push(Vec::new());
            shaped_shortcuts.push(Vec::new());
            phases.push(unbound_widgets::PointerPhase::Idle);
            continue;
        }
        let (label_quads, _) = canvas.layout_text(
            &item.label,
            Rect::new(
                row.origin.x + 16.0,
                row.origin.y,
                (row.size.x - 96.0).max(0.0),
                row.size.y,
            ),
            11.5,
            AlignX::Start,
            AlignY::Center,
        );
        let shortcut_quads = if item.shortcut.is_empty() {
            Vec::new()
        } else {
            canvas
                .layout_text(
                    &item.shortcut,
                    Rect::new(
                        row.origin.x + row.size.x - 84.0,
                        row.origin.y,
                        72.0,
                        row.size.y,
                    ),
                    10.5,
                    AlignX::End,
                    AlignY::Center,
                )
                .0
        };
        let phase = u8::try_from(index)
            .ok()
            .map(|idx| {
                canvas
                    .control_state(
                        target::id(Target::EditorMenuItem(idx)),
                        item.disabled,
                        false,
                    )
                    .phase
            })
            .unwrap_or(unbound_widgets::PointerPhase::Idle);
        shaped_labels.push(label_quads);
        shaped_shortcuts.push(shortcut_quads);
        phases.push(phase);
    }

    let render_rows: Vec<MenuItemRenderRow<'_>> = (0..desc.items.len())
        .map(|idx| MenuItemRenderRow {
            label_glyphs: shaped_labels[idx].as_slice(),
            shortcut_glyphs: shaped_shortcuts[idx].as_slice(),
            phase: phases[idx],
        })
        .collect();
    let menu_state = MenuState {
        highlighted,
        ..MenuState::default()
    };

    let (mut output, _) = emit_menu_with_rows(
        EDITOR_MENU_NODE,
        0,
        desc,
        &menu_state,
        ControlState::default(),
        &render_rows,
    );

    let mut key_map: rustc_hash::FxHashMap<u64, (u64, bool)> = rustc_hash::FxHashMap::default();
    for (index, item) in desc.items.iter().enumerate() {
        if item.separator {
            continue;
        }
        if let Ok(idx) = u8::try_from(index) {
            let target_node = target::id(Target::EditorMenuItem(idx));
            let active = highlighted == Some(index) || canvas.hovered(target_node);
            key_map.insert(
                menu_item_node(EDITOR_MENU_NODE, index),
                (target_node, active),
            );
        }
    }
    for hit in &mut output.sub_hits {
        if let Some(&(target_node, _)) = key_map.get(&hit.node) {
            hit.node = target_node;
            hit.layer = MENU_LAYER;
        }
    }
    for (sub_key, a11y) in &mut output.sub_a11y {
        if let Some(&(target_node, active)) = key_map.get(sub_key) {
            *sub_key = target_node;
            a11y.selected = active;
        }
    }
    for focus_id in &mut output.sub_focus_order {
        if let Some(&(target_node, _)) = key_map.get(focus_id) {
            *focus_id = target_node;
        }
    }
    // 禁用菜单项同样保留被动热区与禁用无障碍节点，使测试与无障碍朗读均可感知其状态。
    for (index, item) in desc.items.iter().enumerate() {
        if item.separator || !item.disabled {
            continue;
        }
        if let Ok(idx) = u8::try_from(index) {
            let target_node = target::id(Target::EditorMenuItem(idx));
            let row = desc.item_rect(index);
            let mut a11y = unbound_a11y::A11yNode::new(Role::MenuItem, item.label.clone(), row);
            a11y.disabled = true;
            output.sub_hits.push(
                unbound_input::HitRegion::new(
                    target_node,
                    MENU_LAYER,
                    u32::from(desc.z).saturating_add(1),
                    row,
                )
                .with_cursor(CursorKind::Default),
            );
            output.sub_a11y.push((target_node, a11y));
        }
    }
    output.hit = None;
    output.a11y = None;
    canvas.widget_output(WidgetCatalogId::ContextMenu, 0, &output);
}

/// 壳层悬停工具提示的解析结果（支持纯文本与带 `Kbd` 快捷键徽章的富内容气泡）。
struct ShellTooltipTarget {
    anchor: Rect,
    text: String,
    shortcut: &'static [&'static str],
    placement: unbound_widgets::overlay::Placement,
}

/// 解析当前悬停节点对应的壳层 Tooltip 锚点、双语文案、快捷键徽章与放置方向。
fn tooltip_target_of(
    state: &GalleryState,
    layout: &ShellLayout,
    nav_layout: &NavigationViewLayout,
    page_layout: Option<&PageLayout>,
    window_maximized: bool,
    labels: &mut Labels,
    hovered: Option<u64>,
) -> Option<ShellTooltipTarget> {
    use unbound_widgets::overlay::Placement;
    use unbound_widgets::segmented::SegmentedDesc;

    let hovered_node = hovered?;
    let target = target::decode(hovered_node)?;
    let resolved = match target {
        Target::Snap => ShellTooltipTarget {
            anchor: layout.snap,
            text: state
                .t(if state.pixel_snap {
                    "tooltip.pixel_snap.on"
                } else {
                    "tooltip.pixel_snap.off"
                })
                .to_owned(),
            shortcut: &[],
            placement: Placement::Bottom,
        },
        Target::Hud => ShellTooltipTarget {
            anchor: layout.hud,
            text: state
                .t(if state.show_hud {
                    "tooltip.hud.on"
                } else {
                    "tooltip.hud.off"
                })
                .to_owned(),
            shortcut: &[],
            placement: Placement::Bottom,
        },
        Target::NavToggle => ShellTooltipTarget {
            anchor: layout.nav_toggle,
            text: state.t("tooltip.nav_toggle").to_owned(),
            shortcut: &[],
            placement: Placement::Bottom,
        },
        Target::LangMenu => ShellTooltipTarget {
            anchor: layout.lang,
            text: state.t("tooltip.lang_menu").to_owned(),
            shortcut: &[],
            placement: Placement::Bottom,
        },
        Target::ZoomMenu => ShellTooltipTarget {
            anchor: layout.zoom,
            text: state.t("tooltip.zoom_menu").to_owned(),
            shortcut: &[],
            placement: Placement::Bottom,
        },
        Target::Theme(bit) => {
            let anchor = if layout.compact {
                layout.theme
            } else {
                SegmentedDesc::new(layout.theme, &["", ""]).segment_rect(usize::from(bit).min(1))
            };
            ShellTooltipTarget {
                anchor,
                text: state
                    .t(if bit == 0 {
                        "tooltip.theme.to_light"
                    } else {
                        "tooltip.theme.to_dark"
                    })
                    .to_owned(),
                shortcut: &[],
                placement: Placement::Bottom,
            }
        }
        Target::Search if layout.search.size.x > 0.0 => ShellTooltipTarget {
            anchor: layout.search,
            text: state.t("tooltip.search.label").to_owned(),
            shortcut: &["Ctrl", "K"],
            placement: Placement::Bottom,
        },
        Target::WindowMinimize => ShellTooltipTarget {
            anchor: layout.window_minimize,
            text: state.t("tooltip.window.minimize").to_owned(),
            shortcut: &[],
            placement: Placement::Bottom,
        },
        Target::WindowMaximize => ShellTooltipTarget {
            anchor: layout.window_maximize,
            text: state
                .t(if window_maximized {
                    "tooltip.window.restore"
                } else {
                    "tooltip.window.maximize"
                })
                .to_owned(),
            shortcut: &[],
            placement: Placement::Bottom,
        },
        Target::WindowClose => ShellTooltipTarget {
            anchor: layout.window_close,
            text: state.t("tooltip.window.close").to_owned(),
            shortcut: &["Alt", "F4"],
            placement: Placement::Bottom,
        },
        Target::Footer(idx) => {
            let row = nav_layout
                .rows
                .iter()
                .find(|row| row.item_id() == Some(hovered_node))?;
            let key = if idx == 0 {
                "tooltip.footer.telemetry"
            } else {
                "tooltip.footer.settings"
            };
            ShellTooltipTarget {
                anchor: row.rect,
                text: state.t(key).to_owned(),
                shortcut: &[],
                placement: Placement::Right,
            }
        }
        Target::NavPage(_) | Target::NavCategory(_) | Target::NavComponent(_) => {
            let row = nav_layout
                .rows
                .iter()
                .chain(nav_layout.flyout_rows.iter())
                .chain(nav_layout.overflow_rows.iter())
                .find(|row| row.item_id() == Some(hovered_node))?;
            let NavigationViewRowKind::Item { item, depth, .. } = &row.kind else {
                return None;
            };
            let compact_rail = !nav_layout.is_top
                && nav_layout.overlay_rect.is_none()
                && nav_layout.pane_rect.width() <= NAVIGATION_VIEW_COMPACT_PANE_LENGTH + 4.0;
            let chevron_w = if item.children.is_empty() { 0.0 } else { 20.0 };
            let avail_text_w =
                (row.rect.width() - 44.0 - (*depth as f32) * 24.0 - chevron_w).max(0.0);
            let label_w = labels.measure(&item.content, 12.0).x;
            if !compact_rail && row.rect.width() > 56.0 && label_w <= avail_text_w {
                return None;
            }
            ShellTooltipTarget {
                anchor: row.rect,
                text: item.content.clone(),
                shortcut: &[],
                placement: Placement::Right,
            }
        }
        Target::Code(page_idx, card_idx) => {
            let page = page_layout?;
            if page.page.index() as u8 != page_idx {
                return None;
            }
            let card = page
                .cards
                .iter()
                .find(|card| card.index as u8 == card_idx)?;
            if card.code_bar.max().y <= layout.content.origin.y
                || card.code_bar.origin.y >= layout.content.max().y
            {
                return None;
            }
            let open = state.code_open(page.page, card_idx);
            ShellTooltipTarget {
                anchor: card.code_bar,
                text: state
                    .t(if open {
                        "tooltip.code.collapse"
                    } else {
                        "tooltip.code.expand"
                    })
                    .to_owned(),
                shortcut: &[],
                placement: Placement::Top,
            }
        }
        _ => return None,
    };
    if resolved.anchor.size.x <= 0.0 || resolved.anchor.size.y <= 0.0 {
        return None;
    }
    Some(resolved)
}

/// 壳层全部交互入口的悬停 Tooltip（统一走 Win11 默认纯文本或 `Kbd` 富内容桥接）。
fn draw_top_bar_tooltip(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    layout: &ShellLayout,
    nav_layout: &NavigationViewLayout,
    page_layout: Option<&PageLayout>,
    window_maximized: bool,
    hovered: Option<u64>,
) {
    let Some(hovered_node) = hovered else {
        return;
    };
    let Some(target) = tooltip_target_of(
        state,
        layout,
        nav_layout,
        page_layout,
        window_maximized,
        canvas.labels,
        hovered,
    ) else {
        return;
    };

    if target.shortcut.is_empty() {
        canvas.emit_tooltip_plain(
            TOP_BAR_TOOLTIP_NODE,
            hovered_node,
            target.anchor,
            layout.viewport,
            &target.text,
            target.placement,
        );
    } else {
        canvas.emit_tooltip_with_shortcut(
            TOP_BAR_TOOLTIP_NODE,
            hovered_node,
            target.anchor,
            layout.viewport,
            &target.text,
            target.shortcut,
            target.placement,
        );
    }
}

/// 构造与顶栏对齐的独立窗口控制按钮组描述符（ADR-0029）。
fn window_caption_buttons_desc(top_bar: Rect, window_maximized: bool) -> WindowCaptionButtons {
    WindowCaptionButtons::new()
        .height(top_bar.size.y)
        .button_width(46.0)
        .maximized(window_maximized)
        .focused(true)
        .minimize_id(target::id(Target::WindowMinimize))
        .maximize_id(target::id(Target::WindowMaximize))
        .close_id(target::id(Target::WindowClose))
}

fn draw_window_controls(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    layout: &ShellLayout,
    window_maximized: bool,
    atlas_clock: u64,
) {
    let desc = window_caption_buttons_desc(layout.top_bar, window_maximized);
    let states = WindowCaptionButtonControlStates {
        minimize: canvas.control_state(desc.minimize_id, false, false),
        maximize: canvas.control_state(desc.maximize_id, false, false),
        close: canvas.control_state(desc.close_id, false, false),
    };
    let caption_out = {
        let mut text_ctx = canvas.text_ctx(atlas_clock);
        emit_window_caption_buttons(
            &desc,
            layout.top_bar,
            window_maximized,
            states,
            &mut text_ctx,
        )
    };
    for mut entry in caption_out.entries {
        if let Some(a11y) = entry.output.a11y.as_mut() {
            a11y.label = match entry.kind {
                unbound_widgets::WindowCaptionKind::Minimize => {
                    state.t("window.minimize").to_owned()
                }
                unbound_widgets::WindowCaptionKind::Maximize { maximized } => state
                    .t(if maximized {
                        "tooltip.window.restore"
                    } else {
                        "window.maximize"
                    })
                    .to_owned(),
                unbound_widgets::WindowCaptionKind::Close => state.t("window.close").to_owned(),
            };
        }
        canvas.control_output(entry.node, &entry.output);
    }
}

/// 判断节点 ID 是否属于 Showcase 导航项（页面、目录组件或页脚快捷入口）。
const fn is_nav_item_node(node: u64) -> bool {
    matches!(
        target::decode(node),
        Some(
            Target::NavPage(_)
                | Target::NavCategory(_)
                | Target::NavComponent(_)
                | Target::Footer(_)
        )
    )
}

/// 为每个页面分配稳定的 `ShellIcon`，用于 `Auto` 紧凑图标栏与展开导航树。
const fn nav_page_icon(page: PageId) -> ShellIcon {
    match page {
        PageId::Home => ShellIcon::Grid,
        PageId::Components => ShellIcon::ChevronDown,
        PageId::Telemetry => ShellIcon::Chart,
        PageId::Settings => ShellIcon::Layers,
        _ => match page.category() {
            CategoryId::Design => ShellIcon::Layers,
            CategoryId::Components => ShellIcon::Grid,
            CategoryId::Layout => ShellIcon::Grid,
            CategoryId::Collections => ShellIcon::Chart,
            CategoryId::Motion => ShellIcon::Layers,
            CategoryId::System => ShellIcon::Chart,
            _ => ShellIcon::Snap,
        },
    }
}

/// 构造当前展厅状态对应的规范 [`NavigationViewDesc`]（ADR-0026）。
fn build_navigation_view_desc(
    state: &GalleryState,
    viewport: Rect,
    hovered_item: Option<u64>,
) -> NavigationViewDesc {
    let body_rect = Rect::new(
        viewport.origin.x,
        viewport.origin.y + crate::layout_tree::TOP_BAR_HEIGHT,
        viewport.size.x,
        (viewport.size.y - crate::layout_tree::TOP_BAR_HEIGHT).max(0.0),
    );

    let mut menu_items = Vec::with_capacity(64);
    menu_items.push(NavigationViewMenuItem::item(
        NavigationViewItem::new(
            target::id(Target::NavPage(PageId::Home.index() as u8)),
            state.t("nav.home"),
        )
        .with_icon(ShellIcon::Grid),
    ));
    menu_items.push(NavigationViewMenuItem::separator());

    // 域一：组件目录（All Controls + 10 个默认折叠的功能分类，由 WidgetCatalogId 穷举元数据派生）
    menu_items.push(NavigationViewMenuItem::header(
        CategoryId::Components.localized_label(state.locale),
    ));
    menu_items.push(NavigationViewMenuItem::item(
        NavigationViewItem::new(
            target::id(Target::NavPage(PageId::Components.index() as u8)),
            PageId::Components.localized_info(state.locale).short,
        )
        .with_icon(ShellIcon::Grid),
    ));
    for cat in ControlCategory::ALL {
        let children: Vec<NavigationViewMenuItem> = cat
            .controls()
            .map(|comp_id| {
                NavigationViewMenuItem::item(
                    NavigationViewItem::new(
                        target::id(Target::NavComponent(comp_id as u8)),
                        pages::component_nav_label_localized(comp_id, state.locale),
                    )
                    .with_icon(ShellIcon::Dot),
                )
            })
            .collect();
        menu_items.push(NavigationViewMenuItem::item(
            NavigationViewItem::new(
                target::id(Target::NavCategory(cat.index() as u8)),
                pages::category_localized_label(cat, state.locale),
            )
            .with_icon(pages::category_shell_icon(cat))
            .with_children(children),
        ));
    }

    // 域二：引擎与设计实验室
    menu_items.push(NavigationViewMenuItem::separator());
    for category in CategoryId::ALL {
        if category == CategoryId::Components {
            continue;
        }
        menu_items.push(NavigationViewMenuItem::header(
            category.localized_label(state.locale),
        ));
        for page in category.pages() {
            menu_items.push(NavigationViewMenuItem::item(
                NavigationViewItem::new(
                    target::id(Target::NavPage(page.index() as u8)),
                    page.localized_info(state.locale).short,
                )
                .with_icon(nav_page_icon(*page)),
            ));
        }
    }

    let footer_menu_items = vec![
        NavigationViewMenuItem::item(
            NavigationViewItem::new(
                target::id(Target::Footer(0)),
                state.t("nav.footer.telemetry"),
            )
            .with_icon(ShellIcon::Chart),
        ),
        NavigationViewMenuItem::item(
            NavigationViewItem::new(
                target::id(Target::Footer(1)),
                state.t("nav.footer.settings"),
            )
            .with_icon(ShellIcon::Layers),
        ),
    ];

    let selected = if state.active_page == PageId::Components {
        if let Some(comp_id) = state.active_component {
            Some(target::id(Target::NavComponent(comp_id as u8)))
        } else if let Some(cat) = state.active_category {
            Some(target::id(Target::NavCategory(cat.index() as u8)))
        } else {
            Some(target::id(
                Target::NavPage(PageId::Components.index() as u8),
            ))
        }
    } else {
        Some(target::id(Target::NavPage(state.active_page.index() as u8)))
    };

    let mut desc = NavigationViewDesc::new(body_rect, menu_items)
        .pane_display_mode(NavigationViewPaneDisplayMode::Auto)
        .open_pane_length(NAV_OPEN_PANE_LENGTH)
        .compact_pane_length(NAVIGATION_VIEW_COMPACT_PANE_LENGTH)
        .is_pane_toggle_button_visible(false)
        .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
        .is_settings_visible(false)
        .footer_menu_items(footer_menu_items)
        .selected_item(selected)
        .hovered_item(hovered_item)
        .label("NavigationView");
    desc.always_show_header = false;
    desc
}

/// 发射 `NavigationView` 底层背景（窗格透明底与内容区圆角卡片），需置于页面内容之前。
fn draw_navigation_view_shell_backdrop(canvas: &mut Canvas<'_>, nav_output: &WidgetOutput) {
    let backdrop_count = nav_output.primitives.len().min(2);
    if backdrop_count == 0 {
        return;
    }
    let slice = WidgetOutput {
        primitives: nav_output.primitives[..backdrop_count].to_vec(),
        ..Default::default()
    };
    canvas.control_output(0, &slice);
}

/// 发射 `NavigationView` 窗格条目、Overlay 浮层与子菜单 Flyout，并将子热区标识还原为规范 `target::id`。
fn draw_navigation_view_pane(
    canvas: &mut Canvas<'_>,
    nav_output: &WidgetOutput,
    nav_layout: &NavigationViewLayout,
) {
    let backdrop_count = nav_output.primitives.len().min(2);
    // 建立 `navigation_view_row_node(NAV_VIEW_NODE, item_id) -> (item_id, is_main_menu_row)` 映射表。
    let mut row_map: rustc_hash::FxHashMap<u64, (u64, bool)> = rustc_hash::FxHashMap::default();
    for row in &nav_layout.rows {
        if let Some(item_id) = row.item_id() {
            let is_main = matches!(row.kind, NavigationViewRowKind::Item { .. });
            row_map.insert(
                navigation_view_row_node(NAV_VIEW_NODE, item_id),
                (item_id, is_main),
            );
        }
    }
    for row in &nav_layout.flyout_rows {
        if let Some(item_id) = row.item_id() {
            row_map.insert(
                navigation_view_row_node(NAV_VIEW_NODE, item_id),
                (item_id, false),
            );
        }
    }

    let menu_clip = unbound_input::ClipRect::new(nav_layout.menu_viewport_rect);
    let mut remapped_hits = Vec::with_capacity(nav_output.sub_hits.len());
    for mut hit in nav_output.sub_hits.iter().cloned() {
        if hit.node == NAV_SCROLLBAR_THUMB_NODE || hit.node == NAV_SCROLLBAR_TRACK_NODE {
            remapped_hits.push(hit);
        } else if let Some(&(item_id, is_main)) = row_map.get(&hit.node) {
            hit.node = item_id;
            if is_main && nav_layout.menu_viewport_rect.height() > 0.0 {
                hit = hit.with_clip(menu_clip);
            }
            remapped_hits.push(hit);
        }
    }

    let mut remapped_a11y = Vec::with_capacity(nav_output.sub_a11y.len());
    for (sub_key, a11y) in &nav_output.sub_a11y {
        if let Some(&(item_id, _)) = row_map.get(sub_key) {
            remapped_a11y.push((item_id, a11y.clone()));
        }
    }

    let mut remapped_focus = Vec::with_capacity(nav_output.sub_focus_order.len());
    for sub_key in &nav_output.sub_focus_order {
        if let Some(&(item_id, _)) = row_map.get(sub_key) {
            remapped_focus.push(item_id);
        }
    }

    let pane_out = WidgetOutput {
        primitives: nav_output.primitives[backdrop_count..].to_vec(),
        payloads: nav_output.payloads.clone(),
        clips: nav_output.clips.clone(),
        hit: None,
        a11y: None,
        focusable: false,
        sub_hits: remapped_hits.into(),
        sub_a11y: remapped_a11y.into(),
        sub_focus_order: remapped_focus.into(),
    };

    let clip_rect = nav_layout.overlay_rect.unwrap_or(nav_layout.pane_rect);
    let has_flyout = nav_layout.flyout_rect.is_some();
    let base_prim = canvas.span_start();
    if !has_flyout && clip_rect.width() > 0.0 && clip_rect.height() > 0.0 {
        canvas.begin_clip(clip_rect);
        canvas.widget_output(WidgetCatalogId::NavigationView, 0, &pane_out);
        canvas.end_clip();
    } else {
        canvas.widget_output(WidgetCatalogId::NavigationView, 0, &pane_out);
    }
    // 按行记录精确的图元跨度 `[start, end)`，确保悬停单行导航项时的脏切片归因于该行自身。
    let mut cursor = 0usize;
    let prims = &pane_out.primitives;
    for row in &nav_layout.rows {
        let Some(item_id) = row.item_id() else {
            continue;
        };
        let row_pos = [row.rect.origin.x, row.rect.origin.y];
        let row_size = [row.rect.size.x, row.rect.size.y];
        let focus_rect = row.rect.inflate(3.0);
        let focus_pos = [focus_rect.origin.x, focus_rect.origin.y];
        let focus_size = [focus_rect.size.x, focus_rect.size.y];
        let approx =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 0.1 && (a[1] - b[1]).abs() < 0.1;
        if let Some(rel_start) = prims[cursor..]
            .iter()
            .position(|p| approx(p.local_pos, row_pos) && approx(p.size, row_size))
        {
            let start_idx = cursor + rel_start;
            if let Some(rel_end) = prims[start_idx..]
                .iter()
                .position(|p| approx(p.local_pos, focus_pos) && approx(p.size, focus_size))
            {
                let end_idx = start_idx + rel_end + 1;
                canvas.record_exact_span(item_id, base_prim + start_idx, base_prim + end_idx);
                cursor = end_idx;
            }
        }
    }
}

fn draw_page(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    layout: &ShellLayout,
    page: &PageLayout,
) {
    let palette = *canvas.palette;
    canvas.begin_clip(layout.content);

    match page.template {
        PageTemplate::Landing => {
            pages::home::draw_landing(canvas, state, page.template_body);
        }
        PageTemplate::Inspector => {
            pages::parity_report::draw_inspector(canvas, state, page.template_body);
        }
        PageTemplate::CategoryHub => {
            let title_owned: String;
            let desc_owned: String;
            if let Some(cat) = state.active_category {
                title_owned = format!(
                    "{} ({})",
                    pages::category_localized_label(cat, state.locale),
                    cat.count()
                );
                desc_owned = pages::category_localized_summary(cat, state.locale).to_string();
            } else {
                let info = page.page.localized_info(state.locale);
                title_owned = format!("{} ({})", info.short, WidgetCatalogId::COUNT);
                desc_owned = info.description.to_string();
            }
            canvas.semantic(
                Role::Heading,
                &title_owned,
                Rect::new(
                    page.header.origin.x,
                    page.header.origin.y,
                    page.header.size.x,
                    34.0,
                ),
            );
            canvas.text(
                &title_owned,
                Rect::new(
                    page.header.origin.x,
                    page.header.origin.y,
                    page.header.size.x,
                    38.0,
                ),
                28.0,
                palette.text,
                AlignX::Start,
                AlignY::Top,
            );
            canvas.text_wrapped(
                &desc_owned,
                Rect::new(
                    page.header.origin.x,
                    page.header.origin.y + 40.0,
                    (page.header.size.x * 0.78).min(820.0),
                    40.0,
                ),
                12.5,
                palette.text_muted,
            );
            pages::components::draw_category_hub(canvas, state, page.template_body);
        }
        PageTemplate::Workshop => {
            if page.page == PageId::Components {
                if let Some(comp_id) = state.active_component {
                    let label = pages::component_nav_label_localized(comp_id, state.locale);
                    let cat = comp_id.category();
                    let cat_label = pages::category_localized_label(cat, state.locale);
                    let all_label = PageId::Components.localized_info(state.locale).short;
                    // 面包屑导航条：All Controls / <Category> / <Control>
                    let all_w = canvas.labels.measure(all_label, 11.0).x + 8.0;
                    let all_rect =
                        Rect::new(page.header.origin.x, page.header.origin.y, all_w, 18.0);
                    canvas.text(
                        all_label,
                        all_rect,
                        11.0,
                        palette.primary,
                        AlignX::Start,
                        AlignY::Center,
                    );
                    canvas.control(
                        target::id(Target::ContentNavPage(PageId::Components.index() as u8)),
                        all_rect,
                        Role::Link,
                        all_label,
                        CursorKind::Pointer,
                        false,
                        false,
                    );
                    let sep_x = all_rect.max().x + 4.0;
                    canvas.text(
                        "/",
                        Rect::new(sep_x, page.header.origin.y, 10.0, 18.0),
                        11.0,
                        palette.text_muted,
                        AlignX::Center,
                        AlignY::Center,
                    );
                    let cat_w = canvas.labels.measure(cat_label, 11.0).x + 8.0;
                    let cat_rect = Rect::new(sep_x + 12.0, page.header.origin.y, cat_w, 18.0);
                    canvas.text(
                        cat_label,
                        cat_rect,
                        11.0,
                        palette.primary,
                        AlignX::Start,
                        AlignY::Center,
                    );
                    canvas.control(
                        target::id(Target::ContentNavCategory(cat.index() as u8)),
                        cat_rect,
                        Role::Link,
                        cat_label,
                        CursorKind::Pointer,
                        false,
                        false,
                    );

                    canvas.semantic(
                        Role::Heading,
                        label,
                        Rect::new(
                            page.header.origin.x,
                            page.header.origin.y + 22.0,
                            page.header.size.x,
                            36.0,
                        ),
                    );
                    canvas.text(
                        label,
                        Rect::new(
                            page.header.origin.x,
                            page.header.origin.y + 22.0,
                            page.header.size.x - 140.0,
                            38.0,
                        ),
                        28.0,
                        palette.text,
                        AlignX::Start,
                        AlignY::Top,
                    );
                    let desc = pages::category_localized_summary(cat, state.locale);
                    canvas.text(
                        desc,
                        Rect::new(
                            page.header.origin.x,
                            page.header.origin.y + 62.0,
                            page.header.size.x - 140.0,
                            20.0,
                        ),
                        12.0,
                        palette.text_muted,
                        AlignX::Start,
                        AlignY::Top,
                    );

                    // 来源徽章 + 模块标签 + 收藏切换按钮
                    let badge_text = pages::control_class_badge(comp_id.class());
                    let (b_fg, b_bg) = pages::home::origin_badge_colors(palette, comp_id.class());
                    let origin_badge = widgets::badge_sized(
                        canvas,
                        Rect::new(page.header.origin.x, page.header.origin.y + 90.0, 0.0, 20.0),
                        badge_text,
                        b_fg,
                        b_bg,
                        10.5,
                    );
                    widgets::badge_sized(
                        canvas,
                        Rect::new(
                            origin_badge.max().x + 6.0,
                            page.header.origin.y + 90.0,
                            0.0,
                            20.0,
                        ),
                        comp_id.module_name(),
                        palette.text_muted,
                        palette.surface,
                        10.5,
                    );
                    let starred = state.is_favorite(comp_id);
                    let fav_label = if starred {
                        "★ Favorited"
                    } else {
                        "☆ Favorite"
                    };
                    let fav_w = canvas.labels.measure(fav_label, 11.0).x + 22.0;
                    widgets::button(
                        canvas,
                        target::id(Target::FavoriteToggle(comp_id as u8)),
                        Rect::new(
                            page.header.max().x - fav_w,
                            page.header.origin.y + 24.0,
                            fav_w,
                            30.0,
                        ),
                        fav_label,
                        if starred {
                            widgets::ButtonVariant::Primary
                        } else {
                            widgets::ButtonVariant::Secondary
                        },
                        true,
                    );
                }
            } else {
                let info = page.page.localized_info(state.locale);
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
            }

            for card in &page.cards {
                if card.rect.max().y < layout.content.origin.y
                    || card.rect.origin.y > layout.content.max().y
                {
                    continue;
                }
                draw_example(canvas, state, page, card);
            }
        }
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
                12.0,
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
    let (localized_title, localized_desc) = example.localized_header(state.locale);
    canvas.rect(
        card.rect,
        14.0,
        palette.card_bg,
        palette.card_border,
        1.0,
        4.0,
    );
    canvas.begin_clip(card.rect);
    // 先自右向左排布 crate 徽章并记录最左缘，再以该左缘约束标题宽度并做省略截断。
    let mut badge_right = card.rect.max().x - 18.0;
    let badge_limit = card.rect.origin.x + 170.0;
    let mut badges_left_x = card.rect.max().x - 18.0;
    for krate in example.crates.iter().rev() {
        let width = widgets::measure_badge(canvas, krate, 9.5);
        if badge_right - width < badge_limit {
            break;
        }
        let badge = widgets::badge_right_aligned(
            canvas,
            badge_right,
            card.rect.origin.y + 14.0,
            18.0,
            krate,
            palette.text_muted,
            palette.stage_bg,
            9.5,
        );
        badges_left_x = badge.origin.x;
        badge_right = badge.origin.x - 6.0;
    }
    let title_x = card.rect.origin.x + 18.0;
    let title_w = (badges_left_x - title_x - 8.0).max(40.0);
    canvas.text_elided(
        localized_title,
        Rect::new(title_x, card.rect.origin.y + 12.0, title_w, 24.0),
        15.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    canvas.text_wrapped(
        localized_desc,
        Rect::new(
            card.rect.origin.x + 18.0,
            card.rect.origin.y + 38.0,
            card.rect.size.x - 36.0,
            30.0,
        ),
        10.5,
        palette.text_muted,
    );

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
            state.t("page.options"),
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
    let localized_label = spec.localized_label(state.locale);
    match spec {
        KnobSpec::Toggle { .. } => {
            widgets::toggle_switch(
                canvas,
                node,
                row.control,
                localized_label,
                value > 0.5,
                false,
            );
        }
        KnobSpec::CheckBox { .. } => {
            widgets::checkbox(
                canvas,
                node,
                row.control,
                localized_label,
                value > 0.5,
                false,
            );
        }
        KnobSpec::Slider {
            min,
            max,
            step,
            unit,
            ..
        } => {
            canvas.text(
                localized_label,
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
            options,
            default_index,
            ..
        } => {
            canvas.text(
                localized_label,
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
            let localized_options: Vec<&str> = options
                .iter()
                .map(|opt| KnobSpec::localized_option(opt, state.locale))
                .collect();
            let selected = (value as usize).min(options.len().saturating_sub(1));
            let _ = default_index;
            widgets::segmented(
                canvas,
                &ids,
                row.control,
                &localized_options,
                selected,
                false,
                None,
            );
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
    use unbound_widgets::code_block::{emit_code_block, CodeBlockDesc, CodeBlockState};
    use unbound_widgets::collapsible::{emit_collapsible, CollapsibleDesc};

    let palette = *canvas.palette;
    let node = target::id(Target::Code(page.page.index() as u8, card.index as u8));
    let start = canvas.span_start();
    let open = state.code_open(page.page, card.index as u8);
    let bar_label = format!("</>  {}", state.t("page.code_bar"));
    let collapsible_desc = CollapsibleDesc::new(card.code_bar, &bar_label, open);
    let ctrl_state = canvas.control_state(node, false, open);
    let (label_quads, _) = canvas.layout_text(
        &bar_label,
        Rect::new(
            card.code_bar.origin.x + 10.0,
            card.code_bar.origin.y,
            (card.code_bar.size.x - 46.0).max(0.0),
            card.code_bar.size.y,
        ),
        10.0,
        AlignX::Start,
        AlignY::Center,
    );
    let mut collapsible_out = emit_collapsible(node, &collapsible_desc, ctrl_state, &label_quads);
    if let Some(a11y) = collapsible_out.a11y.as_mut() {
        a11y.label = state.t("a11y.code_and_a11y").to_owned();
    }
    canvas.widget_output(WidgetCatalogId::Expander, node, &collapsible_out);
    // 跨度在代码栏处结束：其下方的新面板属于卡片，且无论如何仅在卡片形状变化时出现。
    canvas.record_span(node, start);

    if let Some(panel) = card.code_panel {
        let code_height = (panel.size.y - 22.0).max(24.0);
        let code_rect = Rect::new(panel.origin.x, panel.origin.y, panel.size.x, code_height);
        let snippet: String = example.code.lines().take(12).collect::<Vec<_>>().join(
            "
",
        );
        let mut code_desc = CodeBlockDesc::new(code_rect, snippet, "rust");
        code_desc.copyable = false;

        let mut code_quads: Vec<unbound_text::GlyphQuad> = Vec::new();
        let gutter = code_desc.gutter_rect();
        let code_area = code_desc.code_rect();
        for (line_index, line) in example.code.lines().take(12).enumerate() {
            let line_y = code_rect.origin.y + 6.0 + line_index as f32 * 15.0;
            let (num_quads, _) = canvas.layout_text(
                &format!("{:>2}", line_index + 1),
                Rect::new(
                    gutter.origin.x + 4.0,
                    line_y,
                    (gutter.size.x - 10.0).max(16.0),
                    14.0,
                ),
                9.0,
                AlignX::End,
                AlignY::Top,
            );
            code_quads.extend(num_quads);
            let (src_quads, _) = canvas.layout_text(
                line,
                Rect::new(
                    code_area.origin.x + 10.0,
                    line_y,
                    (code_area.size.x - 16.0).max(0.0),
                    14.0,
                ),
                9.5,
                AlignX::Start,
                AlignY::Top,
            );
            code_quads.extend(src_quads);
        }
        let mut code_out = emit_code_block(
            0,
            &code_desc,
            &CodeBlockState::new(),
            &code_quads,
            ControlState::default(),
        );
        code_out.a11y = None;
        canvas.begin_clip(panel);
        canvas.widget_output(WidgetCatalogId::CodeBlock, 0, &code_out);
        canvas.text(
            &format!("a11y · {}", example.a11y),
            Rect::new(
                panel.origin.x + 12.0,
                panel.max().y - 18.0,
                (panel.size.x - 24.0).max(0.0),
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
        state.t("hud.title"),
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
        (state.t("hud.frame"), format!("{:.2} ms", stats.frame_ms)),
        (state.t("hud.fps"), format!("{:.0}", stats.fps)),
        (state.t("hud.prims"), format!("{prims}")),
        (state.t("hud.glyphs"), format!("{glyphs}")),
        (state.t("hud.cards"), format!("{cards}")),
        (
            state.t("hud.limit"),
            if state.fps_limit == 0 {
                state.t("hud.uncapped").to_string()
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
    if !state.search.focused {
        return;
    }
    let palette = *canvas.palette;
    let hits = state.search_hits();
    let panel = search_overlay_rect(layout, hits.len());
    canvas.rect(
        panel,
        12.0,
        palette.overlay_bg,
        palette.card_border,
        1.0,
        26.0,
    );
    canvas.text(
        if state.search.text().is_empty() {
            state.t("search.palette_title")
        } else {
            state.t("search.matches")
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
    if state.search.text().is_empty() {
        canvas.text(
            state.t("search.hint"),
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
    if hits.is_empty() {
        canvas.text(
            state.t("search.empty"),
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
    for (index, hit) in hits.iter().enumerate() {
        let row = Rect::new(
            panel.origin.x + 6.0,
            panel.origin.y + 30.0 + index as f32 * 36.0,
            panel.size.x - 12.0,
            34.0,
        );
        let (node, title, meta, a11y_label) = match *hit {
            crate::state::SearchHit::Page(page) => {
                let info = page.localized_info(state.locale);
                (
                    target::id(Target::SearchResult(page.index() as u8)),
                    info.short.to_string(),
                    format!(
                        "{} · {}",
                        page.category().localized_label(state.locale),
                        info.crates.first().copied().unwrap_or("unbound")
                    ),
                    info.title.to_string(),
                )
            }
            crate::state::SearchHit::Component(id) => {
                let prefix = state.t("search.component_prefix");
                (
                    target::id(Target::SearchComponent(id as u8)),
                    crate::pages::components::component_nav_label_localized(id, state.locale)
                        .to_string(),
                    format!("{prefix} · {}", id.module_name()),
                    format!("{prefix} {}", id.module_name()),
                )
            }
        };
        // 键盘选中项优先高亮，鼠标悬停次之；两者都不再依赖当前页面。
        let selected = index == state.search_selection;
        let hovered = canvas.visual(node, false).is_hovered();
        if selected || hovered {
            canvas.rect(
                row,
                7.0,
                if selected {
                    palette.primary_soft
                } else {
                    palette.control_hover
                },
                0,
                0.0,
                0.0,
            );
        }
        canvas.text(
            &title,
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
            &meta,
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
            &a11y_label,
            CursorKind::Pointer,
            false,
            selected,
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
