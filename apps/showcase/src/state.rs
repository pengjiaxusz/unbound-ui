//! 展厅的单一事实来源（Single Source of Truth）。
//! lang-allow: bilingual showcase i18n bundle; Chinese is the zh-CN half
//!
//! 导航、环境开关、旋钮值与实时演示状态全部集中于此。页面是该状态的纯函数，
//! 这使得整个外壳无需窗口即可进行测试。
//!

use std::collections::{HashMap, HashSet, VecDeque};

use unbound_anim::{Spring, SpringPreset};
use unbound_app::{set_global_locale, AssetHub, I18nBundle, Locale};
use unbound_asset::{AssetId, StagingRing};
use unbound_backdrop::{
    is_illegal_combination, native_available, resolve_combination, selectable,
    BackdropCapabilities, BackdropCombination, BackdropPlan, BackdropSelection, BackdropSource,
    MaterialKind, DEFAULT_ACRYLIC_TINT_ARGB,
};
use unbound_canvas::{TransactionKind, UndoStack};
use unbound_math::Vec2;
use unbound_widgets::curve_editor::CurveEditorState;
use unbound_widgets::progress::ProgressState;
use unbound_widgets::text_input::TextEditState;
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

/// 展厅内所有单行文本编辑器共享的编辑状态。
///
/// 搜索框、演示输入框与 FPS 输入框都只保存这一份真实缓冲区，光标、选区、
/// 滚动、撤销和 IME 组合全部由 `TextEditState` 承担，页面不再维护字符串镜像。
#[derive(Clone, Debug, Default)]
pub struct TextEditor {
    /// 真实编辑缓冲区。
    pub edit: TextEditState,
    /// 是否持有键盘焦点。
    pub focused: bool,
    /// 自上次光标相位归零以来经过的秒数。
    pub blink: f32,
}

impl TextEditor {
    /// 创建带初始文本的编辑器。
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            edit: TextEditState::with_text(text),
            focused: false,
            blink: 0.0,
        }
    }

    /// 当前缓冲区文本（包含正在组合的 IME 内容）。
    #[must_use]
    pub fn text(&self) -> &str {
        self.edit.text()
    }

    /// 获取焦点并重置光标相位。
    pub fn focus(&mut self) {
        self.focused = true;
        self.blink = 0.0;
    }

    /// 释放焦点、折叠选区并丢弃未提交的 IME 组合。
    pub fn blur(&mut self) {
        self.focused = false;
        self.blink = 0.0;
        self.edit.cancel_composition();
        // 锚点与光标相等即无选区：失焦后悬停不得再复现旧的选区。
        self.edit.set_cursor(self.edit.cursor(), false);
    }

    /// 用户交互后重置光标闪烁相位。
    pub fn touch(&mut self) {
        self.blink = 0.0;
    }

    /// 推进光标闪烁时钟。
    pub fn tick(&mut self, dt: f32) {
        if self.focused {
            self.blink = (self.blink + dt).rem_euclid(1.06);
        }
    }

    /// 插入符当前是否处于可见相位（约 530ms 亮 / 530ms 灭）。
    #[must_use]
    pub fn caret_visible(&self) -> bool {
        self.focused && self.blink < 0.53
    }
}

/// 展厅内可编辑文本的标识。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EditorId {
    /// 顶栏全局搜索框。
    Search,
    /// 文本输入框演示（TextField 页与 Fonts 沙盒共享）。
    Demo,
    /// 自定义 FPS 上限输入框。
    Fps,
}

impl EditorId {
    /// 在 `editor_views` 中的下标。
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Search => 0,
            Self::Demo => 1,
            Self::Fps => 2,
        }
    }
}

/// 非示例定义派生的运行时状态。
#[derive(Debug)]
pub struct DemoState {
    /// 文本输入框演示的共享编辑状态。
    pub text: TextEditor,
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
    /// WinUI 3 像素对照报告（进入报告页时延迟载入一次）。
    pub parity_report: Option<crate::pages::parity_report::ParityReport>,
    /// 报告页刷新按钮按下后置位：下一帧重新读取差分产物，并复用未变化的纹理。
    pub parity_reload: bool,
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
    /// i18n 教学页演示的任务计数增量。
    pub i18n_counter: u32,
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
            text: TextEditor::new("Unbound UI"),
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
            parity_report: None,
            parity_reload: false,
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
            i18n_counter: 0,
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

    /// 延迟载入像素对照报告，并为参考 / 本框架 / 热力图三组截图提交异步解码。
    pub fn poll_parity(&mut self, text: &mut unbound_text::TextEngine) {
        let hub = self.asset_hub.get_or_insert_with(AssetHub::new);
        if self.parity_reload || self.parity_report.is_none() {
            let previous = self.parity_report.take();
            self.parity_report = Some(crate::pages::parity_report::ParityReport::load(
                hub,
                previous.as_ref(),
            ));
            self.parity_reload = false;
        }
        let _ = hub.poll(text);
    }

    /// 取出演示资产中心累积的图集上传任务。
    ///
    /// 该中心独立于运行时内置的资产管线，因此宿主必须在每帧重建后主动排空，
    /// 否则解码完成的截图永远不会写入 GPU 图集。
    pub fn take_asset_uploads(&mut self) -> Vec<unbound_gpu::atlas::AtlasUpload> {
        self.asset_hub
            .as_mut()
            .map(|hub| hub.take_uploads())
            .unwrap_or_default()
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
    /// 用户的后景选择：材质种类与来源两个正交维度。
    ///
    /// 默认是「标准云母 + 自动来源」。
    pub backdrop_selection: BackdropSelection,
    /// 宿主按平台能力解析后的实际生效组合。
    ///
    /// 它由宿主在下一帧回写：包含实际生效的材质与来源，以及回退原因与
    /// 机器能力不足的原因。
    pub backdrop_combination: BackdropCombination,
    /// 由路径、能力、屏幕几何与主题共同推导出的完整后景计划。
    ///
    /// 启动、运行期切换与自绘生成只消费这一份结果，不再各自推导清屏 alpha、
    /// 根背景透明性或纹理取样区域。
    pub backdrop_plan: BackdropPlan,
    /// 图形表面是否支持预乘 alpha 合成。
    ///
    /// 与构建号一起构成逐档材质门槛所需的全部平台能力。
    pub premultiplied_alpha_supported: bool,
    /// 运行时能否解析出未公开的 accent policy 导出。
    ///
    /// 这是亚克力自定义色调子档的专属能力，只影响该档的可用性。
    pub accent_policy_export: bool,
    /// 亚克力自定义色调（ABGR）。
    pub backdrop_accent_tint: u32,
    /// Windows 构建号；`None` 表示非 Windows 环境或探测失败。
    pub windows_build: Option<u32>,
    /// 客户区在屏幕坐标系中的物理矩形（后景锚点）。
    ///
    /// 由平台层在移动、尺寸与 DPI 变化时写入；布局不使用它，只有依赖屏幕位置的
    /// 自绘后景取样会读取。
    pub window_screen_rect: Option<unbound_platform::ScreenRect>,
    /// 当通过侧边栏直接选中某个具体组件时记录该组件 ID。
    pub active_component: Option<WidgetCatalogId>,
    /// 顶栏全局搜索框的共享编辑状态。
    pub search: TextEditor,
    /// 搜索命中列表中的键盘选中项索引；随查询变化归零。
    pub search_selection: usize,
    /// 各编辑器最近一帧的文本视口宽度，用于保持插入符可见。
    pub editor_views: [core::cell::Cell<f32>; 3],
    /// 最近一帧记录的插入符位置（IME 候选框锚点）。
    pub ime_anchor: core::cell::Cell<Vec2>,
    /// 当前激活的界面语言区域（默认 `Locale::EnUs`，支持 `Locale::ZhCn`）。
    pub locale: Locale,
    /// 外壳内建的原生国际化资源包。
    pub i18n: I18nBundle,
    /// 标题栏语言下拉菜单是否处于展开状态。
    pub lang_dropdown_open: bool,
    /// 标题栏窗口缩放下拉菜单是否处于展开状态。
    pub zoom_dropdown_open: bool,
    pub theme: ThemeMode,
    pub zoom_step: usize,
    /// 是否启用像素网格对齐。
    pub pixel_snap: bool,
    /// 帧率上限（FPS）；`0` 表示无限制。
    pub fps_limit: u32,
    /// 自定义 FPS 上限的共享编辑状态。
    pub fps_input: TextEditor,
    pub show_hud: bool,
    pub content_scroll: f32,
    pub knobs: HashMap<u32, f32>,
    pub expanded_code: HashSet<(u8, u8)>,
    pub demo: DemoState,
    pub sandbox: ComponentSandboxState,
    /// 文本渲染控制台持久页面状态。
    pub text_render: crate::pages::text_render::TextRenderPageState,
    /// 每次状态变更时递增，使窗口可跳过完全相同的帧。
    pub revision: u64,
    /// 将演示动画重建节流到约 30 Hz 的累加器。
    animation_clock: f32,
}

impl Default for GalleryState {
    fn default() -> Self {
        let backdrop_selection = BackdropSelection::new(MaterialKind::Mica, BackdropSource::Auto);
        Self {
            active_page: PageId::Home,
            backdrop_combination: resolve_combination(
                backdrop_selection,
                &BackdropCapabilities {
                    windows_build: None,
                    premultiplied_alpha_supported: false,
                    accent_policy_export: false,
                },
            ),
            backdrop_selection,
            backdrop_plan: BackdropPlan::default(),
            premultiplied_alpha_supported: false,
            accent_policy_export: false,
            backdrop_accent_tint: DEFAULT_ACRYLIC_TINT_ARGB,
            windows_build: None,
            window_screen_rect: None,
            active_component: None,
            search: TextEditor::default(),
            search_selection: 0,
            editor_views: core::array::from_fn(|_| core::cell::Cell::new(200.0)),
            ime_anchor: core::cell::Cell::new(Vec2::new(24.0, 60.0)),
            locale: Locale::EnUs,
            i18n: build_showcase_i18n(Locale::EnUs),
            lang_dropdown_open: false,
            zoom_dropdown_open: false,
            theme: ThemeMode::Dark,
            zoom_step: 0,
            pixel_snap: true,
            fps_limit: 120,
            fps_input: TextEditor::new("120"),
            // HUD 是从顶栏打开的浮动弹出层；默认显示会在 960×600 下遮挡首屏卡片。
            //
            show_hud: false,
            content_scroll: 0.0,
            knobs: HashMap::new(),
            expanded_code: HashSet::new(),
            demo: DemoState::default(),
            sandbox: ComponentSandboxState::default(),
            text_render: crate::pages::text_render::TextRenderPageState::default(),
            revision: 1,
            animation_clock: 0.0,
        }
    }
}

/// 构建 Showcase 外壳的原生中英双语资源包（默认 `en-US` 并以其作为回退语言）。
#[must_use]
pub fn build_showcase_i18n(locale: Locale) -> I18nBundle {
    let mut bundle = I18nBundle::with_locale(locale);
    bundle.register_bilingual(&[
        ("app.title", "Unbound UI Gallery", "Unbound UI 组件展厅"),
        ("a11y.toggle_nav", "Toggle navigation", "折叠或展开导航栏"),
        (
            "a11y.search",
            "Search controls, crates and features",
            "搜索组件、模块与特性",
        ),
        ("a11y.lang_selector", "Language selector", "界面语言选择器"),
        ("a11y.toggle_theme", "Toggle theme", "切换主题"),
        (
            "a11y.zoom_selector",
            "Window scale selector",
            "窗口缩放选择器",
        ),
        (
            "a11y.pixel_snap",
            "Pixel Snap (align layout to physical pixels)",
            "像素对齐（Pixel Snap，贴合物理像素网格）",
        ),
        (
            "a11y.hud",
            "Toggle live FPS HUD overlay",
            "切换实时帧率 HUD 浮窗",
        ),
        (
            "tooltip.pixel_snap.on",
            "Pixel Snap: On — Align geometry to physical pixels",
            "像素对齐：已开启 — 几何边界贴合物理像素网格",
        ),
        (
            "tooltip.pixel_snap.off",
            "Pixel Snap: Off — Allow subpixel layout coordinates",
            "像素对齐：已关闭 — 允许亚像素坐标排版",
        ),
        (
            "tooltip.hud.on",
            "FPS HUD Overlay: On — Hide floating frame-rate & frame-time HUD",
            "实时帧率 HUD 浮窗：已开启 — 隐藏悬浮帧率与耗时面板",
        ),
        (
            "tooltip.hud.off",
            "FPS HUD Overlay: Off — Show floating frame-rate & frame-time HUD",
            "实时帧率 HUD 浮窗：已关闭 — 显示悬浮帧率与耗时面板",
        ),
        (
            "tooltip.nav_toggle",
            "Toggle navigation pane",
            "展开或折叠侧边导航栏",
        ),
        (
            "tooltip.theme.to_light",
            "Switch to Light theme",
            "切换为浅色主题",
        ),
        (
            "tooltip.theme.to_dark",
            "Switch to Dark theme",
            "切换为深色主题",
        ),
        (
            "tooltip.search",
            "Open Command Palette (Ctrl+K)",
            "打开命令面板（Ctrl+K）",
        ),
        (
            "tooltip.footer.telemetry",
            "Open Engine Telemetry page — Full GPU, layout & memory diagnostics",
            "打开引擎性能遥测页 — 查看完整 GPU、布局与内存诊断图表",
        ),
        (
            "tooltip.footer.settings",
            "Open Settings & About page",
            "打开设置与关于页面",
        ),
        (
            "a11y.code_and_a11y",
            "Code and accessibility",
            "源码与无障碍信息",
        ),
        ("top.search.short", "Search…", "搜索…"),
        (
            "top.search.placeholder",
            "Search controls, crates, features…",
            "搜索组件、模块或特性…",
        ),
        ("top.theme.light", "Light", "浅色"),
        ("top.theme.dark", "Dark", "深色"),
        ("nav.home", "Home", "首页"),
        ("nav.footer.telemetry", "Engine Telemetry", "引擎性能遥测"),
        ("nav.footer.settings", "Settings & About", "设置与关于"),
        ("page.options", "OPTIONS", "可调参数"),
        (
            "page.code_bar",
            "Rust · ABI · accessibility",
            "Rust 源码 · ABI · 无障碍契约",
        ),
        ("page.source", "SOURCE", "源码实现"),
        ("hud.title", "LIVE FRAME", "实时帧遥测"),
        ("hud.frame", "FRAME", "帧耗时"),
        ("hud.fps", "FPS", "帧率"),
        ("hud.prims", "PRIMS", "图元"),
        ("hud.glyphs", "GLYPHS", "字形"),
        ("hud.cards", "CARDS", "卡片"),
        ("hud.limit", "LIMIT", "帧率上限"),
        ("hud.uncapped", "uncapped", "无限制"),
        (
            "search.palette_title",
            "COMMAND PALETTE · CTRL K",
            "命令面板 · CTRL K",
        ),
        ("search.matches", "MATCHES", "匹配结果"),
        (
            "search.hint",
            "Try “Button”, “MTSDF”, “Quadtree”, “DnD”…",
            "试试输入 “Button”、“MTSDF”、“四叉树”、“拖放”…",
        ),
        (
            "search.empty",
            "No controls match that query.",
            "未找到匹配的页面或组件。",
        ),
        ("home.hero.title", "Unbound UI", "Unbound UI"),
        (
            "home.hero.subtitle",
            "An engine-level UI framework: GPU primitives, MTSDF text, Taffy layout, an infinite canvas and a headless verifier — dogfooded by the application you are looking at.",
            "引擎级 UI 框架：GPU 图元、MTSDF 矢量文本、Taffy 布局、无限画布与无头几何校验器 —— 全部由你眼前这个应用自举驱动。",
        ),
        ("home.hero.cta_primary", "Explore components", "浏览标准组件"),
        (
            "home.hero.cta_secondary",
            "Open canvas studio",
            "打开画布工作台",
        ),
        ("window.minimize", "Minimise window", "最小化窗口"),
        ("window.maximize", "Maximise window", "最大化窗口"),
        ("window.close", "Close window", "关闭窗口"),
    ]);
    bundle
}

impl GalleryState {
    /// 写入后景的材质种类选择；返回是否发生变化。
    ///
    /// 机器承载不了的材质会被拒绝：禁用按钮即便因为命中区域仍然收到点击，
    /// 也不会改变选择。实际生效组合由宿主在下一帧解析回写。
    pub fn set_backdrop_material(&mut self, material: MaterialKind) -> bool {
        if self.backdrop_selection.material == material {
            return false;
        }
        if !self.backdrop_material_selectable(material) {
            return false;
        }
        self.backdrop_selection.material = material;
        self.bump();
        true
    }

    /// 写入后景的来源选择；返回是否发生变化。
    pub fn set_backdrop_source(&mut self, source: BackdropSource) -> bool {
        if self.backdrop_selection.source == source {
            return false;
        }
        self.backdrop_selection.source = source;
        self.bump();
        true
    }

    /// 当前平台能力快照：后景逐档门槛所需的全部事实。
    #[must_use]
    pub fn backdrop_capabilities(&self) -> BackdropCapabilities {
        BackdropCapabilities {
            windows_build: self.windows_build,
            premultiplied_alpha_supported: self.premultiplied_alpha_supported,
            accent_policy_export: self.accent_policy_export,
        }
    }

    /// 写入 accent policy 通道的可用性；返回是否发生变化。
    ///
    /// 它只影响亚克力自定义色调子档，其它材质的门槛完全不受影响。
    pub fn set_backdrop_accent_capability(&mut self, available: bool) -> bool {
        if self.accent_policy_export == available {
            return false;
        }
        self.accent_policy_export = available;
        self.bump();
        true
    }

    /// 写入亚克力自定义色调（ABGR）；返回是否发生变化。
    pub fn set_backdrop_accent_tint(&mut self, argb: u32) -> bool {
        if self.backdrop_accent_tint == argb {
            return false;
        }
        self.backdrop_accent_tint = argb;
        self.bump();
        true
    }

    /// 某个材质在当前来源与平台能力下是否能被选中。
    ///
    /// 展台按钮的禁用态直接取自它；来源为「引擎自绘」时永不因缺少自绘变体而禁用。
    #[must_use]
    pub fn backdrop_material_selectable(&self, material: MaterialKind) -> bool {
        selectable(
            material,
            self.backdrop_selection.source,
            &self.backdrop_capabilities(),
        )
    }

    /// 当前用户选择是否是「组合非法但允许选中」。
    ///
    /// 只有显式自绘来源撞上没有自绘变体的材质才算非法；机器能力不足不是非法，
    /// 它表现为禁用。展台据此在选中的材质按钮上标出非法组合。
    #[must_use]
    pub fn backdrop_combination_illegal(&self) -> bool {
        is_illegal_combination(self.backdrop_selection)
    }

    /// 用户选择的材质种类在这台机器上能否走原生路径。
    #[must_use]
    pub fn backdrop_native_available(&self) -> bool {
        native_available(
            self.backdrop_selection.material,
            &self.backdrop_capabilities(),
        )
    }

    /// 切换 Showcase 激活语言并同步更新内置 `I18nBundle` 与全局语言状态。
    pub fn set_locale(&mut self, locale: Locale) {
        set_global_locale(locale);
        self.locale = locale;
        self.i18n.set_locale_enum(locale);
        self.lang_dropdown_open = false;
        self.bump();
    }

    /// 按键查询当前语言下的外壳文案。
    #[must_use]
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.i18n.t(key)
    }

    /// 将状态标记为脏。
    pub fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// 当前 UI 缩放。
    #[must_use]
    pub fn zoom(&self) -> f32 {
        ZOOM_STEPS[self.zoom_step.min(ZOOM_STEPS.len() - 1)]
    }

    /// 记录编辑器最近一帧的文本视口宽度。
    pub fn set_editor_view(&self, id: EditorId, width: f32) {
        self.editor_views[id.index()].set(width.max(24.0));
    }

    /// 编辑器最近一帧的文本视口宽度。
    #[must_use]
    pub fn editor_view(&self, id: EditorId) -> f32 {
        self.editor_views[id.index()].get().max(24.0)
    }

    /// 记录最近一帧的插入符位置（IME 候选框锚点）。
    pub fn set_ime_anchor(&self, position: Vec2) {
        self.ime_anchor.set(position);
    }

    /// 最近一帧的插入符位置（IME 候选框锚点）。
    #[must_use]
    pub fn ime_anchor(&self) -> Vec2 {
        self.ime_anchor.get()
    }

    /// 释放所有文本编辑器的焦点并取消未提交的 IME 组合。
    pub fn blur_editors(&mut self) {
        self.search.blur();
        self.demo.text.blur();
        self.fps_input.blur();
    }

    /// 导航到指定页面；重置内容滚动位置以使页头可见。
    pub fn navigate(&mut self, page: PageId) {
        if self.active_page != page || self.active_component.is_some() {
            self.active_page = page;
            self.active_component = None;
            self.content_scroll = 0.0;
            self.demo.modal_open = false;
            self.demo.dropdown_open = false;
            self.lang_dropdown_open = false;
            self.zoom_dropdown_open = false;
            self.blur_editors();
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
        self.lang_dropdown_open = false;
        self.zoom_dropdown_open = false;
        self.blur_editors();
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
        self.fps_input.edit.set_text(self.fps_limit.to_string());
        self.bump();
    }

    /// 帧率控制（pacer）当前应使用的上限。
    ///
    /// 当用户在输入 `"144"` 的过程中先键入 `"1"` 时，该非零部分值会被提升到
    /// 60 FPS 下限，避免渲染器卡在 1 FPS。
    ///
    #[must_use]
    pub fn effective_fps_limit(&self) -> u32 {
        if !self.fps_input.focused {
            return self.fps_limit;
        }
        match self.fps_input.text().trim().parse::<u32>() {
            Ok(0) => 0,
            Ok(fps) => fps.max(unbound_platform::MIN_FPS_LIMIT),
            Err(_) => self.fps_limit,
        }
    }

    /// 应用自定义输入：规范化文本并提交上限。
    pub fn commit_fps_input(&mut self) {
        let parsed = self
            .fps_input
            .text()
            .trim()
            .parse::<u32>()
            .unwrap_or(self.fps_limit);
        self.set_fps_limit(parsed);
        self.fps_input.blur();
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
        if self.search.focused || self.demo.text.focused || self.fps_input.focused {
            self.search.tick(dt);
            self.demo.text.tick(dt);
            self.fps_input.tick(dt);
            moving = true;
        }
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
        let animated = match self.active_page {
            PageId::Assets | PageId::Sliders | PageId::Telemetry | PageId::Springs => true,
            // 对照报告只在截图仍在解码时保持重建，纹理到齐后即恢复空闲。
            PageId::Parity => self
                .demo
                .asset_hub
                .as_ref()
                .is_some_and(|hub| hub.inflight() > 0),
            _ => false,
        };
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
        let query = self.search.text().trim().to_lowercase();
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

    /// 同时检索页面目录与全部标准组件目录（支持中英文子串、全拼、多方案双拼与首字母缩写）。
    ///
    /// 页面与组件统一按匹配分数降序混排：查询词为每个命中贡献分数，全部词都命中才算命中。
    /// 因此完全匹配与前缀匹配会排在离散子序列之前。同分沿用目录声明顺序（稳定排序）。
    #[must_use]
    pub fn search_hits(&self) -> Vec<SearchHit> {
        let query = self.search.text().trim();
        if query.is_empty() {
            return Vec::new();
        }
        let terms: Vec<&str> = query.split_whitespace().collect();
        let matcher = unbound_text::PinyinMatcher::new();
        let mut scored: Vec<(u32, SearchHit)> = Vec::new();
        for page in PageId::ALL {
            if let Some(score) = page.match_score(query) {
                scored.push((score, SearchHit::Page(page)));
            }
        }
        for &id in WidgetCatalogId::ALL {
            let label = crate::pages::components::component_nav_label(id);
            let mod_name = id.module_name();
            let keywords = crate::pages::components::component_search_keywords(id);
            let hay = format!("{label} {mod_name} {keywords} component widget");
            let mut total = 0;
            let mut all_terms_match = true;
            for term in &terms {
                match matcher.match_text(&hay, term) {
                    Some(matched) => total += matched.score,
                    None => {
                        all_terms_match = false;
                        break;
                    }
                }
            }
            if all_terms_match {
                scored.push((total, SearchHit::Component(id)));
            }
        }
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        scored.into_iter().take(6).map(|(_, hit)| hit).collect()
    }

    /// 把搜索选中项钳制到当前命中数内；命中数变化后调用，避免索引越界。
    pub fn clamp_search_selection(&mut self, count: usize) {
        if count == 0 {
            self.search_selection = 0;
        } else if self.search_selection >= count {
            self.search_selection = count - 1;
        }
    }

    /// 按增量移动搜索选中项并在两端环绕。
    pub fn move_search_selection(&mut self, delta: isize, count: usize) {
        if count == 0 {
            self.search_selection = 0;
            return;
        }
        let current = self.search_selection.min(count - 1) as isize;
        self.search_selection = (current + delta).rem_euclid(count as isize) as usize;
        self.bump();
    }
}

/// 搜索命中条目：页面或标准组件。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SearchHit {
    Page(PageId),
    Component(WidgetCatalogId),
}

/// 99 个组件展示沙盒的实时交互状态。
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
        | WidgetCatalogId::HyperlinkButton
        | WidgetCatalogId::RepeatButton
        | WidgetCatalogId::DropDownButton
        | WidgetCatalogId::Descriptions
        | WidgetCatalogId::InfoBar
        | WidgetCatalogId::AlertDialog
        | WidgetCatalogId::InfoBadge
        | WidgetCatalogId::Chip
        | WidgetCatalogId::CommandPalette
        | WidgetCatalogId::FontIcon
        | WidgetCatalogId::Popconfirm
        | WidgetCatalogId::Toast
        | WidgetCatalogId::Tour
        | WidgetCatalogId::PersonPicture
        | WidgetCatalogId::AvatarGroup
        | WidgetCatalogId::Dock
        | WidgetCatalogId::Containers
        | WidgetCatalogId::EmptyState
        | WidgetCatalogId::Watermark
        | WidgetCatalogId::NinePatch
        | WidgetCatalogId::StackPanel
        | WidgetCatalogId::Wrap
        | WidgetCatalogId::Divider
        | WidgetCatalogId::Form
        | WidgetCatalogId::StatusBar
        | WidgetCatalogId::PropertyGrid
        | WidgetCatalogId::NotificationCenter
        | WidgetCatalogId::Banner
        | WidgetCatalogId::FileDrop
        | WidgetCatalogId::RichTextBlock
        | WidgetCatalogId::Overlay
        | WidgetCatalogId::Image
        | WidgetCatalogId::ShortcutBar
        | WidgetCatalogId::CommandBarFlyout
        | WidgetCatalogId::AppBarButton
        | WidgetCatalogId::AppBarSeparator
        | WidgetCatalogId::ColorSwatch => {
            let count = sandbox.click_counts.entry(key).or_insert(0);
            *count = count.wrapping_add(1);
            true
        }
        WidgetCatalogId::CheckBox
        | WidgetCatalogId::ToggleSwitch
        | WidgetCatalogId::ToggleButton
        | WidgetCatalogId::ToggleSplitButton
        | WidgetCatalogId::CalendarDatePicker
        | WidgetCatalogId::Callout
        | WidgetCatalogId::Expander
        | WidgetCatalogId::ContentDialog
        | WidgetCatalogId::SplitView
        | WidgetCatalogId::Flyout
        | WidgetCatalogId::ToolTip
        | WidgetCatalogId::HoverCard
        | WidgetCatalogId::LoadingOverlay
        | WidgetCatalogId::AddressBar
        | WidgetCatalogId::QueryBuilder
        | WidgetCatalogId::SchemaForm
        | WidgetCatalogId::AppBarToggleButton
        | WidgetCatalogId::TeachingTip
        | WidgetCatalogId::HeaderedContentControl => {
            let on = sandbox.toggles.entry(key).or_insert(false);
            *on = !*on;
            true
        }
        WidgetCatalogId::ExpanderGroup
        | WidgetCatalogId::TreeTable
        | WidgetCatalogId::VirtualMasonry
        | WidgetCatalogId::TabView
        | WidgetCatalogId::Pivot
        | WidgetCatalogId::RadioButton
        | WidgetCatalogId::SelectorBar
        | WidgetCatalogId::Stepper
        | WidgetCatalogId::RatingControl
        | WidgetCatalogId::PipsPager
        | WidgetCatalogId::FlipView
        | WidgetCatalogId::NavigationView
        | WidgetCatalogId::MenuFlyout
        | WidgetCatalogId::MenuBar
        | WidgetCatalogId::ContextMenu
        | WidgetCatalogId::BreadcrumbBar
        | WidgetCatalogId::CommandBar
        | WidgetCatalogId::CanvasToolbar
        | WidgetCatalogId::ComboBox
        | WidgetCatalogId::Cascader
        | WidgetCatalogId::Table
        | WidgetCatalogId::DataGrid
        | WidgetCatalogId::ListView
        | WidgetCatalogId::Masonry
        | WidgetCatalogId::Chart
        | WidgetCatalogId::Timeline
        | WidgetCatalogId::TimelineTrack
        | WidgetCatalogId::Transfer
        | WidgetCatalogId::TreeView
        | WidgetCatalogId::CalendarView
        | WidgetCatalogId::DatePicker
        | WidgetCatalogId::DateRangePicker
        | WidgetCatalogId::TimePicker
        | WidgetCatalogId::GenericDataTable
        | WidgetCatalogId::PipelineView
        | WidgetCatalogId::GridView
        | WidgetCatalogId::ListBox
        | WidgetCatalogId::SwipeControl
        | WidgetCatalogId::ItemsView
        | WidgetCatalogId::OrbitView
        | WidgetCatalogId::BladeView => {
            let sel = sandbox.selected_indices.entry(key).or_insert(0);
            *sel = (*sel + 1) % 5;
            true
        }
        WidgetCatalogId::Slider
        | WidgetCatalogId::MultiSplit
        | WidgetCatalogId::ProgressBar
        | WidgetCatalogId::NumberBox
        | WidgetCatalogId::ProgressRing
        | WidgetCatalogId::AspectRatioBox
        | WidgetCatalogId::SplitPane
        | WidgetCatalogId::Ruler
        | WidgetCatalogId::Minimap
        | WidgetCatalogId::ScrollView
        | WidgetCatalogId::ColorPicker
        | WidgetCatalogId::GradientEditor
        | WidgetCatalogId::CurveEditor
        | WidgetCatalogId::NodeGraph
        | WidgetCatalogId::Gizmo
        | WidgetCatalogId::TaskHud
        | WidgetCatalogId::RefreshContainer
        | WidgetCatalogId::AnnotatedScrollBar
        | WidgetCatalogId::RangeSelector
        | WidgetCatalogId::RadialGauge
        | WidgetCatalogId::ImageCropper => {
            let val = sandbox.slider_values.entry(key).or_insert(50.0);
            *val = (*val + 10.0) % 100.0;
            true
        }
        WidgetCatalogId::TextBox
        | WidgetCatalogId::TerminalView
        | WidgetCatalogId::TextBoxMultiline
        | WidgetCatalogId::PasswordBox
        | WidgetCatalogId::AutoSuggestBox
        | WidgetCatalogId::ComboBoxEditable
        | WidgetCatalogId::PinInput
        | WidgetCatalogId::TagInput
        | WidgetCatalogId::HotkeyInput
        | WidgetCatalogId::CodeBlock
        | WidgetCatalogId::CodeEditor
        | WidgetCatalogId::RichEditBox
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
            search: TextEditor::new("Accordion"),
            ..Default::default()
        };
        let hits = state.search_hits();
        assert!(
            hits.iter()
                .any(|h| matches!(h, SearchHit::Component(WidgetCatalogId::ExpanderGroup))),
            "搜索 Accordion 应命中对应组件卡片: {hits:?}"
        );

        // 支持中文、全拼、小鹤双拼（如 "dzl" / "ulki"）检索组件
        state.search.edit.set_text("地址栏");
        assert!(
            state
                .search_hits()
                .iter()
                .any(|h| matches!(h, SearchHit::Component(WidgetCatalogId::AddressBar))),
            "中文搜索 '地址栏' 应命中 AddressBar"
        );
        state.search.edit.set_text("voseqi"); // 小鹤双拼 "着色器"
        assert!(
            !state.search_hits().is_empty(),
            "小鹤双拼 'voseqi'（着色器）应命中相关页面或组件"
        );

        state.search.edit.set_text("Button");
        let hits = state.search_hits();
        assert!(
            hits.iter()
                .any(|h| matches!(h, SearchHit::Page(PageId::Buttons))),
            "搜索 Button 应命中页面: {hits:?}"
        );
    }

    #[test]
    fn search_hits_ranks_prefix_match_first() {
        // 完全匹配的组件名必须排在仅离散子序列命中的项之前。
        let state = GalleryState {
            search: TextEditor::new("toggle_button"),
            ..Default::default()
        };
        let hits = state.search_hits();
        assert_eq!(
            hits.first(),
            Some(&SearchHit::Component(WidgetCatalogId::ToggleButton)),
            "完全匹配应排第一: {hits:?}"
        );
    }

    #[test]
    fn search_selection_wraps_and_clamps() {
        let mut state = GalleryState::default();
        state.move_search_selection(-1, 3);
        assert_eq!(state.search_selection, 2, "向上越过首项应环绕到末项");
        state.move_search_selection(1, 3);
        assert_eq!(state.search_selection, 0, "向下越过末项应环绕到首项");
        state.clamp_search_selection(1);
        assert_eq!(state.search_selection, 0, "命中数减少后索引应被钳制");
        state.clamp_search_selection(0);
        assert_eq!(state.search_selection, 0, "无命中时索引归零");
    }
}
