//! 展厅的单一事实来源（Single Source of Truth）。
//!
//! 导航、环境开关、旋钮值与实时演示状态全部集中于此。页面是该状态的纯函数，
//! 这使得整个外壳无需窗口即可进行测试。
//!

use std::collections::{HashMap, HashSet, VecDeque};

use unbound_anim::{Spring, SpringPreset};
use unbound_app::AssetHub;
use unbound_asset::{AssetId, StagingRing};
use unbound_canvas::{TransactionKind, UndoStack};
use unbound_math::Vec2;
use unbound_widgets::curve_editor::CurveEditorState;
use unbound_widgets::progress::ProgressState;
use unbound_widgets::WidgetCatalogId;
use unbound_world::virtualize::{VirtualList, WindowUpdate};

use crate::pages::{component_scroll_offset, ExampleDef, KnobSpec, PageId};

/// 随附的两个应用主题。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    #[must_use]
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}

/// 一个可选的 UI 缩放档位。
pub const ZOOM_STEPS: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

/// 遥测图表保留的最近帧数。
pub const FRAME_HISTORY_CAP: usize = 96;

/// 单帧记录，送入遥测图表的滑动窗口。
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSample {
    /// 帧耗时（毫秒）。
    pub ms: f32,
    /// 本帧上传的字节数。
    pub upload: f32,
    /// 本帧发出的绘制调用次数。
    pub draws: f32,
}

/// 非示例定义派生的运行时状态。
#[derive(Debug)]
pub struct DemoState {
    /// 文本输入框演示的缓冲区。
    pub text: String,
    /// 文本输入框是否持有键盘焦点。
    pub typing: bool,
    /// Overlays 演示中的下拉框 / 模态框。
    pub dropdown_open: bool,
    pub modal_open: bool,
    pub toggle_a: bool,
    pub toggle_b: bool,
    pub toggle_c: bool,
    pub radio_choice: usize,
    pub pill_choice: usize,
    pub sliders: [f32; 3],
    pub drag_order: Vec<usize>,
    pub drag_active: Option<usize>,
    pub drag_position: Option<Vec2>,
    pub list_scroll: [f32; 2],
    pub list_selected: [Option<usize>; 2],
    /// 每个列表卡片对应的真实槽位复用虚拟化器。
    pub virtual_lists: [VirtualList; 2],
    /// 来自 `virtual_lists` 的最新更新遥测。
    pub virtual_updates: [WindowUpdate; 2],
    /// 资源流式加载网格的异步 PNG 解码中心（延迟启动）。
    pub asset_hub: Option<AssetHub>,
    /// 为流式图块向 `asset_hub` 请求的资源 ID。
    pub asset_ids: Vec<AssetId>,
    /// 资源流式加载演示使用的暂存环（staging ring）。
    pub asset_ring: StagingRing,
    pub canvas_pan: Vec2,
    pub canvas_zoom: f32,
    pub marquee: Option<(Vec2, Vec2)>,
    pub selected_nodes: [bool; 12],
    pub history: Vec<&'static str>,
    pub history_index: usize,
    /// 来自 `unbound-canvas` 的真实写时复制（CoW）事务栈。
    pub undo_stack: UndoStack,
    pub spring: Spring,
    /// 分段胶囊控件的动画槽位位置。
    pub pill_spring: Spring,
    pub spring_high: bool,
    pub a11y_focus: usize,
    pub frame_ms: f32,
    pub fps: f32,
    pub upload_bytes: u32,
    pub primitive_count: u32,
    pub draw_calls: u32,
    pub elapsed: f32,
    /// 按钮演示的激活次数。
    pub clicks: u32,
    /// 图标变体中上次选中的图标按钮。
    pub selected_icon: usize,
    /// 系统专页中当前选中的子系统索引（`0..SubsystemId::COUNT`）。
    pub selected_subsystem: usize,
    /// 进度条演示的缓动条显示值。
    pub progress_bar: ProgressState,
    /// 进度条演示的缓动圆环显示值。
    pub progress_ring: ProgressState,
    /// 送入遥测图表的最近帧滑动窗口。
    pub frame_history: VecDeque<FrameSample>,
    /// 缓动演示的交互式三次贝塞尔控制点。
    pub curve_editor: CurveEditorState,
}

impl Default for DemoState {
    fn default() -> Self {
        let mut spring = Spring::new(SpringPreset::Snappy, 0.0);
        spring.set_target(0.0);
        // 默认胶囊位于槽位 1（"All"），因此弹簧从该处起步，
        // 而不是在首帧从左侧滑入。
        let mut pill_spring = Spring::new(SpringPreset::Snappy, 1.0);
        pill_spring.snap_to(1.0);
        Self {
            text: "Unbound UI".to_string(),
            typing: false,
            dropdown_open: false,
            modal_open: false,
            toggle_a: true,
            toggle_b: false,
            toggle_c: true,
            radio_choice: 0,
            pill_choice: 1,
            sliders: [0.62, 0.35, 0.8],
            drag_order: vec![0, 1, 2, 3],
            drag_active: None,
            drag_position: None,
            list_scroll: [0.0, 0.0],
            list_selected: [Some(2), Some(2)],
            virtual_lists: [VirtualList::new(32.0, 0.25), VirtualList::new(32.0, 0.25)],
            virtual_updates: [WindowUpdate::default(), WindowUpdate::default()],
            asset_hub: None,
            asset_ids: Vec::new(),
            asset_ring: StagingRing::new(4096, 3),
            canvas_pan: Vec2::new(18.0, 12.0),
            canvas_zoom: 1.0,
            marquee: None,
            selected_nodes: [false; 12],
            history: vec!["Open canvas", "Add node", "Move node"],
            history_index: 3,
            undo_stack: {
                let mut stack = UndoStack::new(64);
                for (i, label) in ["Open canvas", "Add node", "Move node"].iter().enumerate() {
                    stack.begin(TransactionKind::Discrete, label, i as f64);
                    stack.record_exist(
                        i as u32,
                        false,
                        unbound_math::Rect::new(i as f32 * 40.0, 0.0, 124.0, 60.0),
                        i as f64,
                    );
                    stack.commit();
                }
                stack
            },
            spring,
            pill_spring,
            spring_high: false,
            a11y_focus: 0,
            frame_ms: 4.2,
            fps: 120.0,
            upload_bytes: 0,
            primitive_count: 0,
            draw_calls: 4,
            elapsed: 0.0,
            clicks: 0,
            selected_icon: 0,
            selected_subsystem: 0,
            progress_bar: ProgressState::new(),
            progress_ring: ProgressState::new(),
            frame_history: VecDeque::new(),
            curve_editor: CurveEditorState::new(),
        }
    }
}

impl DemoState {
    /// 使用滑动胶囊选中一个分段槽位。
    pub fn set_pill(&mut self, index: usize) {
        self.pill_choice = index;
        self.pill_spring.set_target(index as f32);
    }

    /// 延迟启动流式网格的异步资源管线并轮询已完成的解码。
    ///
    pub fn poll_assets(&mut self, text: &mut unbound_text::TextEngine) {
        let hub = self.asset_hub.get_or_insert_with(AssetHub::new);
        if self.asset_ids.is_empty() {
            let palette_rgba: [[u8; 4]; 5] = [
                [88, 130, 245, 255],
                [64, 196, 140, 255],
                [242, 169, 64, 255],
                [164, 118, 242, 255],
                [72, 184, 212, 255],
            ];
            for idx in 0..10 {
                let color = palette_rgba[idx % palette_rgba.len()];
                let png_bytes = procedural_tile_png(16, 16, color);
                if let Ok(staging_ref) = self.asset_ring.write(&png_bytes) {
                    let _ = staging_ref;
                }
                self.asset_ids.push(hub.request_png(png_bytes));
            }
            self.asset_ring.begin_frame();
        }
        let _ = hub.poll(text);
    }
}

fn procedural_tile_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        if let Ok(mut writer) = encoder.write_header() {
            let mut pixels = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                for x in 0..width {
                    let checker = ((x / 4 + y / 4) % 2) == 0;
                    let shade = if checker { 1.0 } else { 0.78 };
                    pixels.push((f32::from(rgba[0]) * shade) as u8);
                    pixels.push((f32::from(rgba[1]) * shade) as u8);
                    pixels.push((f32::from(rgba[2]) * shade) as u8);
                    pixels.push(rgba[3]);
                }
            }
            let _ = writer.write_image_data(&pixels);
        }
    }
    bytes
}

/// 完整的展厅状态。
#[derive(Debug)]
pub struct GalleryState {
    pub active_page: PageId,
    /// 当通过侧边栏直接选中某个具体组件时记录该组件 ID。
    pub active_component: Option<WidgetCatalogId>,
    pub nav_expanded: bool,
    pub search_query: String,
    pub search_focused: bool,
    /// 文本控件聚焦时输入法的组合字符串。
    ///
    /// 保存在状态中（而非渲染器中），使页面和搜索框能精确绘制 IME 正在组合的内容。
    ///
    pub ime_preedit: String,
    pub theme: ThemeMode,
    pub zoom_step: usize,
    /// 是否启用像素网格对齐。
    pub pixel_snap: bool,
    /// 帧率上限（FPS）；`0` 表示无限制。
    pub fps_limit: u32,
    /// 自定义 FPS 上限的输入缓冲区。
    pub fps_input: String,
    /// 自定义 FPS 输入框是否持有键盘焦点。
    pub fps_focused: bool,
    pub show_hud: bool,
    pub content_scroll: f32,
    pub nav_scroll: f32,
    pub knobs: HashMap<u32, f32>,
    pub expanded_code: HashSet<(u8, u8)>,
    pub demo: DemoState,
    pub sandbox: ComponentSandboxState,
    /// 每次状态变更时递增，使窗口可跳过完全相同的帧。
    pub revision: u64,
    /// 将演示动画重建节流到约 30 Hz 的累加器。
    animation_clock: f32,
}

impl Default for GalleryState {
    fn default() -> Self {
        Self {
            active_page: PageId::Home,
            active_component: None,
            nav_expanded: true,
            search_query: String::new(),
            search_focused: false,
            ime_preedit: String::new(),
            theme: ThemeMode::Dark,
            zoom_step: 0,
            pixel_snap: true,
            fps_limit: 120,
            fps_input: "120".to_string(),
            fps_focused: false,
            // HUD 是从顶栏打开的浮动弹出层；默认显示会在 960×600 下遮挡首屏卡片。
            //
            show_hud: false,
            content_scroll: 0.0,
            nav_scroll: 0.0,
            knobs: HashMap::new(),
            expanded_code: HashSet::new(),
            demo: DemoState::default(),
            sandbox: ComponentSandboxState::default(),
            revision: 1,
            animation_clock: 0.0,
        }
    }
}

impl GalleryState {
    /// 将状态标记为脏。
    pub fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// 当前 UI 缩放。
    #[must_use]
    pub fn zoom(&self) -> f32 {
        ZOOM_STEPS[self.zoom_step.min(ZOOM_STEPS.len() - 1)]
    }

    /// 导航到指定页面；重置内容滚动位置以使页头可见。
    pub fn navigate(&mut self, page: PageId) {
        if self.active_page != page || self.active_component.is_some() {
            self.active_page = page;
            self.active_component = None;
            self.content_scroll = 0.0;
            self.demo.modal_open = false;
            self.demo.dropdown_open = false;
            self.bump();
        }
    }

    /// 导航到组件展厅并自动将页面滚动到指定组件的卡片位置。
    pub fn navigate_to_component(&mut self, id: WidgetCatalogId) {
        self.active_page = PageId::Components;
        self.active_component = Some(id);
        self.content_scroll = component_scroll_offset(id);
        self.demo.modal_open = false;
        self.demo.dropdown_open = false;
        self.bump();
    }

    /// 设置帧率上限。
    ///
    /// `0` 为无限制；其他值会被钳制到引擎的 60 FPS 下限。
    /// 文本缓冲区会被规范化，使 UI 始终显示实际生效的值。
    ///
    pub fn set_fps_limit(&mut self, fps: u32) {
        self.fps_limit = if fps == 0 {
            0
        } else {
            fps.max(unbound_platform::MIN_FPS_LIMIT)
        };
        self.fps_input = self.fps_limit.to_string();
        self.bump();
    }

    /// 帧率控制（pacer）当前应使用的上限。
    ///
    /// 当用户在输入 `"144"` 的过程中先键入 `"1"` 时，该非零部分值会被提升到
    /// 60 FPS 下限，避免渲染器卡在 1 FPS。
    ///
    #[must_use]
    pub fn effective_fps_limit(&self) -> u32 {
        if !self.fps_focused {
            return self.fps_limit;
        }
        match self.fps_input.trim().parse::<u32>() {
            Ok(0) => 0,
            Ok(fps) => fps.max(unbound_platform::MIN_FPS_LIMIT),
            Err(_) => self.fps_limit,
        }
    }

    /// 应用自定义输入：规范化文本并提交上限。
    pub fn commit_fps_input(&mut self) {
        let parsed = self
            .fps_input
            .trim()
            .parse::<u32>()
            .unwrap_or(self.fps_limit);
        self.set_fps_limit(parsed);
        self.fps_focused = false;
    }

    /// 与当前上限匹配的 FPS 预设索引（如有）。
    #[must_use]
    pub fn fps_preset(&self) -> Option<u8> {
        match self.fps_limit {
            60 => Some(0),
            120 => Some(1),
            144 => Some(2),
            240 => Some(3),
            0 => Some(4),
            _ => None,
        }
    }

    /// 单个旋钮的键。
    #[must_use]
    pub fn knob_key(page: PageId, card: usize, knob: usize) -> u32 {
        ((page.index() as u32) << 16) | ((card as u32) << 8) | knob as u32
    }

    /// 旋钮的当前值，缺省时回退到其声明的默认值。
    #[must_use]
    pub fn knob(&self, page: PageId, card: usize, knob: usize, spec: &KnobSpec) -> f32 {
        self.knobs
            .get(&Self::knob_key(page, card, knob))
            .copied()
            .unwrap_or_else(|| spec.default_value())
    }

    /// 单个示例的所有旋钮值。
    #[must_use]
    pub fn knob_values(&self, page: PageId, card: usize, example: &ExampleDef) -> Vec<f32> {
        example
            .knobs
            .iter()
            .enumerate()
            .map(|(index, spec)| self.knob(page, card, index, spec))
            .collect()
    }

    /// 写入旋钮值。
    pub fn set_knob(&mut self, page: PageId, card: usize, knob: usize, value: f32) {
        self.knobs.insert(Self::knob_key(page, card, knob), value);
        self.bump();
    }

    /// 切换代码面板的展开/收起。
    pub fn toggle_code(&mut self, page: PageId, card: u8) {
        let key = (page.index() as u8, card);
        if !self.expanded_code.remove(&key) {
            self.expanded_code.insert(key);
        }
        self.bump();
    }

    /// 代码面板是否处于打开状态。
    #[must_use]
    pub fn code_open(&self, page: PageId, card: u8) -> bool {
        self.expanded_code.contains(&(page.index() as u8, card))
    }

    /// 推进时间驱动的演示。只要有元素移动则返回 `true`。
    pub fn tick(&mut self, dt: f32) -> bool {
        self.demo.elapsed = (self.demo.elapsed + dt).rem_euclid(3600.0);
        // 记录实时帧统计数据，使遥测图表绘制真实测量值而非合成曲线。
        //
        self.demo.frame_history.push_back(FrameSample {
            ms: self.demo.frame_ms,
            upload: self.demo.upload_bytes as f32,
            draws: self.demo.draw_calls as f32,
        });
        if self.demo.frame_history.len() > FRAME_HISTORY_CAP {
            self.demo.frame_history.pop_front();
        }
        // 进度条演示通过真实的 `ProgressState` 将条和圆环缓动到时钟驱动的目标值，
        // 因此数值跳变会表现为平滑移动。暂停会同时冻结目标值与不确定态扫描动画。
        //
        let progress_example = &PageId::Sliders.examples()[1];
        let progress_knobs = self.knob_values(PageId::Sliders, 1, progress_example);
        let paused = progress_knobs.get(1).copied().unwrap_or(0.0) > 0.5;
        let target = if paused {
            0.64
        } else {
            (self.demo.elapsed * 0.18).rem_euclid(1.0)
        };
        self.demo.progress_bar.tick(dt, Some(target), !paused);
        self.demo.progress_ring.tick(dt, Some(target), !paused);
        let mut moving = false;
        self.demo.spring.step(dt);
        if !self.demo.spring.is_settled() {
            moving = true;
        }
        self.demo.pill_spring.step(dt);
        if !self.demo.pill_spring.is_settled() {
            moving = true;
        }
        // 以约 30 Hz 重新塑形并重绘动画页面。静态页面永不重建，
        // 这正是展厅在空闲时保持低功耗的原因。
        let animated = matches!(
            self.active_page,
            PageId::Assets | PageId::Sliders | PageId::Telemetry | PageId::Springs
        );
        self.animation_clock += dt;
        if animated && self.animation_clock >= 1.0 / 30.0 {
            self.animation_clock = 0.0;
            moving = true;
        }
        if moving {
            self.bump();
        }
        moving
    }

    /// 向撤销时间线推入一个新操作。
    pub fn push_history(&mut self, label: &'static str) {
        self.demo.history.truncate(self.demo.history_index);
        self.demo.history.push(label);
        self.demo.history_index = self.demo.history.len();
        let now = f64::from(self.demo.elapsed);
        let idx = self.demo.history_index as u32;
        self.demo
            .undo_stack
            .begin(TransactionKind::Discrete, label, now);
        self.demo.undo_stack.record_geometry(
            0,
            unbound_math::Rect::new((idx.saturating_sub(1) as f32) * 30.0, 0.0, 124.0, 60.0),
            unbound_math::Rect::new((idx as f32) * 30.0, 0.0, 124.0, 60.0),
            now,
        );
        self.demo.undo_stack.commit();
        self.bump();
    }

    /// 撤销一步操作。
    pub fn undo(&mut self) {
        if self.demo.history_index > 0 {
            self.demo.history_index -= 1;
            let _ = self.demo.undo_stack.undo();
            self.bump();
        }
    }

    /// 重做一步操作。
    pub fn redo(&mut self) {
        if self.demo.history_index < self.demo.history.len() {
            self.demo.history_index += 1;
            let _ = self.demo.undo_stack.redo();
            self.bump();
        }
    }

    /// 按搜索查询词过滤页面目录。
    #[must_use]
    pub fn search_results(&self) -> Vec<PageId> {
        let query = self.search_query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        PageId::ALL
            .iter()
            .copied()
            .filter(|page| page.matches(&query))
            .take(4)
            .collect()
    }

    /// 同时检索页面目录与 91 个标准组件目录。
    #[must_use]
    pub fn search_hits(&self) -> Vec<SearchHit> {
        let query = self.search_query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let terms: Vec<&str> = query.split_whitespace().collect();
        let mut hits = Vec::new();
        for page in PageId::ALL {
            if page.matches(&query) {
                hits.push(SearchHit::Page(page));
                if hits.len() >= 6 {
                    return hits;
                }
            }
        }
        for &id in WidgetCatalogId::ALL {
            let label = crate::pages::components::component_nav_label(id).to_lowercase();
            let mod_name = id.module_name().to_lowercase();
            let hay = format!("{label} {mod_name} component widget");
            if terms.iter().all(|term| hay.contains(term)) {
                hits.push(SearchHit::Component(id));
                if hits.len() >= 6 {
                    break;
                }
            }
        }
        hits
    }
}

/// 搜索命中条目：页面或标准组件。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SearchHit {
    Page(PageId),
    Component(WidgetCatalogId),
}

/// 91 个组件展示沙盒的实时交互状态。
#[derive(Clone, Debug, Default)]
pub struct ComponentSandboxState {
    pub click_counts: HashMap<u8, u32>,
    pub toggles: HashMap<u8, bool>,
    pub selected_indices: HashMap<u8, usize>,
    pub text_buffers: HashMap<u8, String>,
    pub slider_values: HashMap<u8, f32>,
}

impl ComponentSandboxState {
    #[must_use]
    pub fn get_clicks(&self, id: WidgetCatalogId) -> u32 {
        self.click_counts.get(&(id as u8)).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn get_toggle(&self, id: WidgetCatalogId) -> bool {
        self.toggles.get(&(id as u8)).copied().unwrap_or(false)
    }

    #[must_use]
    pub fn get_selected(&self, id: WidgetCatalogId) -> usize {
        self.selected_indices.get(&(id as u8)).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn get_text<'a>(&'a self, id: WidgetCatalogId, default: &'a str) -> &'a str {
        self.text_buffers
            .get(&(id as u8))
            .map(String::as_str)
            .unwrap_or(default)
    }

    #[must_use]
    pub fn get_slider(&self, id: WidgetCatalogId, default: f32) -> f32 {
        self.slider_values
            .get(&(id as u8))
            .copied()
            .unwrap_or(default)
    }
}

/// 展厅中任意组件卡片用户交互的穷举式状态归约。
#[deny(unreachable_patterns)]
pub fn reduce_showcase_component(
    id: WidgetCatalogId,
    sandbox: &mut ComponentSandboxState,
    _click_pos: Option<Vec2>,
) -> bool {
    let key = id as u8;
    match id {
        WidgetCatalogId::Button
        | WidgetCatalogId::SplitButton
        | WidgetCatalogId::Descriptions
        | WidgetCatalogId::Alert
        | WidgetCatalogId::AlertDialog
        | WidgetCatalogId::Badge
        | WidgetCatalogId::Chip
        | WidgetCatalogId::CommandPalette
        | WidgetCatalogId::Icon
        | WidgetCatalogId::Popconfirm
        | WidgetCatalogId::Toast
        | WidgetCatalogId::Tour
        | WidgetCatalogId::Avatar
        | WidgetCatalogId::AvatarGroup
        | WidgetCatalogId::Dock
        | WidgetCatalogId::Containers
        | WidgetCatalogId::EmptyState
        | WidgetCatalogId::Watermark
        | WidgetCatalogId::NinePatch
        | WidgetCatalogId::Stack
        | WidgetCatalogId::Wrap
        | WidgetCatalogId::Divider
        | WidgetCatalogId::Form
        | WidgetCatalogId::StatusBar
        | WidgetCatalogId::PropertyGrid
        | WidgetCatalogId::NotificationCenter
        | WidgetCatalogId::Banner
        | WidgetCatalogId::FileDrop
        | WidgetCatalogId::RichText
        | WidgetCatalogId::Overlay
        | WidgetCatalogId::Image => {
            let count = sandbox.click_counts.entry(key).or_insert(0);
            *count = count.wrapping_add(1);
            true
        }
        WidgetCatalogId::Checkbox
        | WidgetCatalogId::Switch
        | WidgetCatalogId::ToggleButton
        | WidgetCatalogId::Callout
        | WidgetCatalogId::Collapsible
        | WidgetCatalogId::Modal
        | WidgetCatalogId::Drawer
        | WidgetCatalogId::Popover
        | WidgetCatalogId::Tooltip
        | WidgetCatalogId::HoverCard
        | WidgetCatalogId::LoadingOverlay => {
            let on = sandbox.toggles.entry(key).or_insert(false);
            *on = !*on;
            true
        }
        WidgetCatalogId::Accordion
        | WidgetCatalogId::TreeTable
        | WidgetCatalogId::VirtualMasonry
        | WidgetCatalogId::Tabs
        | WidgetCatalogId::Radio
        | WidgetCatalogId::Segmented
        | WidgetCatalogId::Stepper
        | WidgetCatalogId::Rating
        | WidgetCatalogId::Pagination
        | WidgetCatalogId::Carousel
        | WidgetCatalogId::NavigationRail
        | WidgetCatalogId::Menu
        | WidgetCatalogId::Menubar
        | WidgetCatalogId::ContextMenu
        | WidgetCatalogId::Breadcrumb
        | WidgetCatalogId::Toolbar
        | WidgetCatalogId::CanvasToolbar
        | WidgetCatalogId::Select
        | WidgetCatalogId::Cascader
        | WidgetCatalogId::Table
        | WidgetCatalogId::DataGrid
        | WidgetCatalogId::VirtualViews
        | WidgetCatalogId::Masonry
        | WidgetCatalogId::Chart
        | WidgetCatalogId::Timeline
        | WidgetCatalogId::TimelineTrack
        | WidgetCatalogId::Transfer
        | WidgetCatalogId::TreeView
        | WidgetCatalogId::DatePicker
        | WidgetCatalogId::DateRangePicker
        | WidgetCatalogId::TimePicker => {
            let sel = sandbox.selected_indices.entry(key).or_insert(0);
            *sel = (*sel + 1) % 5;
            true
        }
        WidgetCatalogId::Slider
        | WidgetCatalogId::MultiSplit
        | WidgetCatalogId::Progress
        | WidgetCatalogId::NumberInput
        | WidgetCatalogId::Spinner
        | WidgetCatalogId::AspectRatioBox
        | WidgetCatalogId::SplitPane
        | WidgetCatalogId::Ruler
        | WidgetCatalogId::Minimap
        | WidgetCatalogId::Scroll
        | WidgetCatalogId::ColorPicker
        | WidgetCatalogId::GradientEditor
        | WidgetCatalogId::CurveEditor
        | WidgetCatalogId::NodeGraph
        | WidgetCatalogId::Gizmo => {
            let val = sandbox.slider_values.entry(key).or_insert(50.0);
            *val = (*val + 10.0) % 100.0;
            true
        }
        WidgetCatalogId::TextInput
        | WidgetCatalogId::TerminalView
        | WidgetCatalogId::TextArea
        | WidgetCatalogId::AutoComplete
        | WidgetCatalogId::Combobox
        | WidgetCatalogId::PinInput
        | WidgetCatalogId::TagInput
        | WidgetCatalogId::HotkeyInput
        | WidgetCatalogId::CodeBlock
        | WidgetCatalogId::CodeEditor
        | WidgetCatalogId::Skeleton => {
            let text = sandbox.text_buffers.entry(key).or_default();
            if text.len() >= 12 {
                text.clear();
            } else {
                text.push_str("Unbound ");
            }
            true
        }
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn search_hits_matches_both_pages_and_catalog_components() {
        let mut state = GalleryState {
            search_query: "Accordion".to_string(),
            ..Default::default()
        };
        let hits = state.search_hits();
        assert!(
            hits.iter()
                .any(|h| matches!(h, SearchHit::Component(WidgetCatalogId::Accordion))),
            "搜索 Accordion 应命中对应组件卡片: {hits:?}"
        );

        state.search_query = "Button".to_string();
        let hits = state.search_hits();
        assert!(
            hits.iter()
                .any(|h| matches!(h, SearchHit::Page(PageId::Buttons))),
            "搜索 Button 应命中页面: {hits:?}"
        );
    }
}
