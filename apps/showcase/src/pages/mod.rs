//! 展厅目录：分类、页面与示例定义。
//! lang-allow: bilingual page metadata; Chinese is the zh-CN half
//!
//! 页面即数据。外壳只知道如何布局页面、渲染卡片和路由目标；
//! 所有标题、旋钮和代码片段均来自下表。
//!

pub mod canvas;
pub mod collections;
pub mod components;
pub mod design;
pub mod fonts;
pub mod home;
pub mod i18n;
pub mod input;
pub mod layout;
pub mod motion;
pub mod systems;

use unbound_app::Locale;
use unbound_math::Rect;
use unbound_widgets::WidgetCatalogId;

use crate::canvas::Canvas;
use crate::state::GalleryState;

pub use components::{
    component_card_index, component_nav_label, component_nav_label_localized,
    component_scroll_offset, showcase_entry_for_widget, SIDEBAR_COMPONENT_IDS,
};
pub use systems::SubsystemId;

/// 导航分类。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CategoryId {
    Home,
    Design,
    Basic,
    Layout,
    Collections,
    Canvas,
    Motion,
    Components,
    System,
}

impl CategoryId {
    /// 导航树中渲染的全部非 Home 分类。
    ///
    /// 包含 `Design`、`Basic`、`Layout`、`Collections`、`Canvas`、`Motion`、`System` 与 `Components`，
    /// 由下方编译期 `const` 断言强制保证 `PageId::ALL` 中的每一个页面都出现在侧边栏分类树中。
    pub const ALL: [CategoryId; 8] = [
        CategoryId::Design,
        CategoryId::Basic,
        CategoryId::Layout,
        CategoryId::Collections,
        CategoryId::Canvas,
        CategoryId::Motion,
        CategoryId::System,
        CategoryId::Components,
    ];

    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Design => "Design Guidance",
            Self::Basic => "Basic Input",
            Self::Layout => "Layout & Portals",
            Self::Collections => "Collections",
            Self::Canvas => "Infinite Canvas",
            Self::Motion => "Motion, A11y & Telemetry",
            Self::Components => "Components (107)",
            Self::System => "Engine Systems",
        }
    }

    /// 按当前激活语言返回分类显示名称。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn localized_label(self, locale: Locale) -> &'static str {
        if locale.is_zh() {
            match self {
                Self::Home => "首页",
                Self::Design => "设计指南",
                Self::Basic => "基础输入",
                Self::Layout => "布局与传送门",
                Self::Collections => "集合与流式资产",
                Self::Canvas => "无限画布",
                Self::Motion => "动效、无障碍与遥测",
                Self::Components => "组件展厅 (107)",
                Self::System => "引擎系统与国际化",
            }
        } else {
            self.label()
        }
    }

    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn pages(self) -> &'static [PageId] {
        match self {
            Self::Home => &[PageId::Home],
            Self::Design => &[
                PageId::Colors,
                PageId::Typography,
                PageId::Fonts,
                PageId::Elevation,
                PageId::Vector,
            ],
            Self::Basic => &[
                PageId::Buttons,
                PageId::Toggles,
                PageId::Sliders,
                PageId::TextField,
            ],
            Self::Layout => &[PageId::Flex, PageId::Overlays, PageId::DragDrop],
            Self::Collections => &[PageId::ListView, PageId::Assets],
            Self::Canvas => &[PageId::Viewport, PageId::Marquee, PageId::Undo],
            Self::Motion => &[PageId::Springs, PageId::A11y, PageId::Telemetry],
            Self::Components => &[PageId::Components],
            Self::System => &[PageId::Systems, PageId::I18n, PageId::Settings],
        }
    }
}

/// 每个可导航的页面。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PageId {
    Home,
    Colors,
    Typography,
    /// 多语言书写系统矩阵、曲线检查器与 BiDi 沙盒。
    Fonts,
    Elevation,
    Vector,
    Buttons,
    Toggles,
    Sliders,
    TextField,
    Flex,
    Overlays,
    DragDrop,
    ListView,
    Assets,
    Viewport,
    Marquee,
    Undo,
    Springs,
    A11y,
    Telemetry,
    Settings,
    /// 完整组件展厅：每个目录组件对应一张卡片。
    Components,
    /// 引擎核心子系统架构全景页：覆盖全部 21 个核心 crate 子系统。
    Systems,
    /// 原生国际化（i18n）与多语言资源包教学页。
    I18n,
}

impl PageId {
    pub const ALL: [PageId; 25] = [
        PageId::Home,
        PageId::Colors,
        PageId::Typography,
        PageId::Fonts,
        PageId::Elevation,
        PageId::Vector,
        PageId::Buttons,
        PageId::Toggles,
        PageId::Sliders,
        PageId::TextField,
        PageId::Flex,
        PageId::Overlays,
        PageId::DragDrop,
        PageId::ListView,
        PageId::Assets,
        PageId::Viewport,
        PageId::Marquee,
        PageId::Undo,
        PageId::Springs,
        PageId::A11y,
        PageId::Telemetry,
        PageId::Settings,
        PageId::Components,
        PageId::Systems,
        PageId::I18n,
    ];

    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn index(self) -> usize {
        match self {
            Self::Home => 0,
            Self::Colors => 1,
            Self::Typography => 2,
            Self::Fonts => 3,
            Self::Elevation => 4,
            Self::Vector => 5,
            Self::Buttons => 6,
            Self::Toggles => 7,
            Self::Sliders => 8,
            Self::TextField => 9,
            Self::Flex => 10,
            Self::Overlays => 11,
            Self::DragDrop => 12,
            Self::ListView => 13,
            Self::Assets => 14,
            Self::Viewport => 15,
            Self::Marquee => 16,
            Self::Undo => 17,
            Self::Springs => 18,
            Self::A11y => 19,
            Self::Telemetry => 20,
            Self::Settings => 21,
            Self::Components => 22,
            Self::Systems => 23,
            Self::I18n => 24,
        }
    }

    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(PageId::Home)
    }

    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn category(self) -> CategoryId {
        match self {
            Self::Home => CategoryId::Home,
            Self::Colors | Self::Typography | Self::Fonts | Self::Elevation | Self::Vector => {
                CategoryId::Design
            }
            Self::Buttons | Self::Toggles | Self::Sliders | Self::TextField => CategoryId::Basic,
            Self::Flex | Self::Overlays | Self::DragDrop => CategoryId::Layout,
            Self::ListView | Self::Assets => CategoryId::Collections,
            Self::Viewport | Self::Marquee | Self::Undo => CategoryId::Canvas,
            Self::Springs | Self::A11y | Self::Telemetry => CategoryId::Motion,
            Self::Components => CategoryId::Components,
            Self::Systems | Self::I18n | Self::Settings => CategoryId::System,
        }
    }

    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn info(self) -> PageInfo {
        match self {
            Self::Home => home::INFO,
            Self::Colors => design::COLORS_INFO,
            Self::Typography => design::TYPOGRAPHY_INFO,
            Self::Fonts => fonts::FONTS_INFO,
            Self::Elevation => design::ELEVATION_INFO,
            Self::Vector => design::VECTOR_INFO,
            Self::Buttons => input::BUTTONS_INFO,
            Self::Toggles => input::TOGGLES_INFO,
            Self::Sliders => input::SLIDERS_INFO,
            Self::TextField => input::TEXT_INFO,
            Self::Flex => layout::FLEX_INFO,
            Self::Overlays => layout::OVERLAYS_INFO,
            Self::DragDrop => layout::DRAG_INFO,
            Self::ListView => collections::LIST_INFO,
            Self::Assets => collections::ASSETS_INFO,
            Self::Viewport => canvas::VIEWPORT_INFO,
            Self::Marquee => canvas::MARQUEE_INFO,
            Self::Undo => canvas::UNDO_INFO,
            Self::Springs => motion::SPRINGS_INFO,
            Self::A11y => motion::A11Y_INFO,
            Self::Telemetry => motion::TELEMETRY_INFO,
            Self::Settings => motion::SETTINGS_INFO,
            Self::Components => components::COMPONENTS_INFO,
            Self::Systems => systems::SYSTEMS_INFO,
            Self::I18n => i18n::I18N_INFO,
        }
    }

    /// 按当前激活语言返回本地化页面元数据（标题、短标题与描述）。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn localized_info(self, locale: Locale) -> PageInfo {
        let base = self.info();
        if !locale.is_zh() {
            return base;
        }
        let (title, short, description) = match self {
            Self::Home => (
                "Unbound UI 组件与架构展台",
                "首页",
                "引擎实时导览：GPU 渲染器、多语言文本、Taffy 布局、输入路由、无限画布与性能遥测。本页所有卡片均由框架自举绘制。",
            ),
            Self::Colors => (
                "色彩与调色盘",
                "色彩调色盘",
                "语义颜色令牌在每个主题下一次性解析为 512 槽查找表；图元仅携带令牌索引，切换主题零几何重传。",
            ),
            Self::Typography => (
                "排版与字阶",
                "排版系统",
                "MTSDF 字形图集从 9pt 注释到 60pt 展示标题始终保持锐利边缘，支持无损缩放与中日韩字体回退。",
            ),
            Self::Fonts => (
                "多语言文字与双向排版",
                "字体与书写系统",
                "一套塑形管线承载十二种书写系统：HarfBuzz 附标偏移、BiDi 双向层级与系统字体回退链均在此实时运行。",
            ),
            Self::Elevation => (
                "海拔阴影与圆角裁剪",
                "阴影与裁剪",
                "圆角裁剪与解析阴影共用同一表面图元，圆角、模糊半径与描边均为着色器 uniform，零额外绘制调用。",
            ),
            Self::Vector => (
                "矢量图形与贝塞尔曲线",
                "矢量与曲线",
                "贝塞尔丝带在 GPU 上直接展开，同一曲线图元驱动首屏横幅、焦点环与无限画布缩略图。",
            ),
            Self::Buttons => (
                "按钮与交互状态",
                "按钮",
                "四种视觉权重，完整支持常规、悬停、按压、聚焦与禁用五态，每次按压反馈均走真实路由器命中。",
            ),
            Self::Toggles => (
                "开关与选择控件",
                "选择控件",
                "切换开关、复选框、单选按钮组与分段胶囊，共享统一的稳定节点标识与无障碍语义契约。",
            ),
            Self::Sliders => (
                "滑块与进度指示器",
                "滑块与进度",
                "支持指针捕获的连续数值滑块，以及由动画时钟驱动的确定态与不确定态进度条/进度环。",
            ),
            Self::TextField => (
                "文本输入与拼音检索",
                "文本输入框",
                "带光标闪烁、拖拽选区、IME 预编辑候选条以及全量汉字拼音/双拼实时过滤沙盒的文本表面。",
            ),
            Self::Flex => (
                "弹性布局与盒模型沙盒",
                "弹性布局",
                "舞台背后运行真实 Taffy 弹性求解器：调整方向、间距与对齐方式，容器在帧构建前实时重算。",
            ),
            Self::Overlays => (
                "传送门与浮层定位",
                "浮层与弹窗",
                "菜单、模态框与工具提示在 Portal 层绘制，Tooltip 支持窗口边界自动翻转与安全边距钳制。",
            ),
            Self::DragDrop => (
                "拖拽排序与放置区",
                "拖拽排序",
                "指针捕获、阈值防抖与放置区解析：释放时将新顺序提交至应用状态。",
            ),
            Self::ListView => (
                "10 万行虚拟化列表",
                "虚拟化列表",
                "十万行数据集仅实例化 12 个活跃节点，滚动时复用实体槽位而非每帧分配内存。",
            ),
            Self::Assets => (
                "异步流式资源网格",
                "流式资产网格",
                "后台异步 PNG 解码、环形暂存缓冲上传与渐进式纹理切换，加载期间界面保持满帧响应。",
            ),
            Self::Viewport => (
                "无限画布与四叉树剔除",
                "视口与四叉树",
                "带松散四叉树空间索引的无界画布：滚轮缩放、拖拽平移，仅向渲染器提交相交分块。",
            ),
            Self::Marquee => (
                "框选与智能对齐参考线",
                "框选与磁吸",
                "在舞台上拖拽即可框选节点，移动过程中智能参考线自动吸附节点边缘与中心。",
            ),
            Self::Undo => (
                "撤销 / 重做历史时间线",
                "撤销与历史",
                "写时复制（CoW）有界事务历史栈，实时展示当前应用游标、命令标签与字节占用。",
            ),
            Self::Springs => (
                "物理弹簧与缓动曲线",
                "弹簧与缓动",
                "可调刚度、阻尼与质量的解析物理弹簧，以及控制点可交互拖拽的三次贝塞尔曲线编辑器。",
            ),
            Self::A11y => (
                "无障碍语义树与焦点环",
                "无障碍",
                "每个控件均发布角色、名称、状态与焦点顺序，右侧面板实时镜像导出至平台适配器的语义树。",
            ),
            Self::Telemetry => (
                "GPU 与 KPI 性能遥测",
                "性能遥测",
                "实时监测帧耗时、图元数、绘制调用、上传字节数与帧率上限，与顶栏 HUD 同源驱动。",
            ),
            Self::Settings => (
                "设置、双拼方案与关于",
                "设置与关于",
                "展台本身即兼容性基准：主题、缩放、像素吸附、帧率上限与双拼方案均支持运行时即时切换。",
            ),
            Self::Components => (
                "107 款标准组件展厅",
                "全部组件",
                "收录全部 107 款标准组件，每个组件均在侧边栏拥有直达入口并由真实 unbound-widgets 发射器绘制。",
            ),
            Self::Systems => (
                "21 个引擎核心子系统全景",
                "引擎子系统",
                "工作区全部 21 个核心 crate 架构矩阵、端到端 6 阶段帧执行管线与无头几何消毒规则全景。",
            ),
            Self::I18n => (
                "原生国际化 (i18n) 与本地化指南",
                "原生 i18n",
                "零外部重型依赖的原生多语言系统：内置中英预设、回退链、占位符插值、单复数、区域数字/日期格式化与三步接入教程。",
            ),
        };
        PageInfo {
            title,
            short,
            description,
            crates: base.crates,
            keywords: base.keywords,
        }
    }

    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn examples(self) -> &'static [ExampleDef] {
        match self {
            Self::Home => home::EXAMPLES,
            Self::Colors => design::COLOR_EXAMPLES,
            Self::Typography => design::TYPOGRAPHY_EXAMPLES,
            Self::Fonts => fonts::FONTS_EXAMPLES,
            Self::Elevation => design::ELEVATION_EXAMPLES,
            Self::Vector => design::VECTOR_EXAMPLES,
            Self::Buttons => input::BUTTON_EXAMPLES,
            Self::Toggles => input::TOGGLE_EXAMPLES,
            Self::Sliders => input::SLIDER_EXAMPLES,
            Self::TextField => input::TEXT_EXAMPLES,
            Self::Flex => layout::FLEX_EXAMPLES,
            Self::Overlays => layout::OVERLAY_EXAMPLES,
            Self::DragDrop => layout::DRAG_EXAMPLES,
            Self::ListView => collections::LIST_EXAMPLES,
            Self::Assets => collections::ASSET_EXAMPLES,
            Self::Viewport => canvas::VIEWPORT_EXAMPLES,
            Self::Marquee => canvas::MARQUEE_EXAMPLES,
            Self::Undo => canvas::UNDO_EXAMPLES,
            Self::Springs => motion::SPRING_EXAMPLES,
            Self::A11y => motion::A11Y_EXAMPLES,
            Self::Telemetry => motion::TELEMETRY_EXAMPLES,
            Self::Settings => motion::SETTINGS_EXAMPLES,
            Self::Components => components::COMPONENT_EXAMPLES,
            Self::Systems => systems::SYSTEMS_EXAMPLES,
            Self::I18n => i18n::I18N_EXAMPLES,
        }
    }

    /// 支持中英文子串、全拼、双拼（默认小鹤双拼）及首字母缩写的页面搜索。
    #[must_use]
    pub fn matches(self, query: &str) -> bool {
        let info = self.info();
        let zh_info = self.localized_info(Locale::ZhCn);
        let zh_keywords = match self {
            Self::Home => "首页 概览 欢迎 主页",
            Self::Colors => "色彩 主题 调色盘 品牌色",
            Self::Typography => "排版 字阶 行高 字体",
            Self::Fonts => "多语言 字体塑形 双向文本 曲线",
            Self::Elevation => "阴影 层级 裁剪 圆角",
            Self::Vector => "矢量图形 路径 描边 图标",
            Self::Buttons => "按钮 交互状态 焦点环",
            Self::Toggles => "开关 复选框 单选 分段器",
            Self::Sliders => "滑块 进度条 磁吸 区间",
            Self::TextField => "文本输入框 选区 输入法 拼音双拼检索",
            Self::Flex => "弹性布局 盒模型 网格",
            Self::Overlays => "浮层 模态框 抽屉 提示框",
            Self::DragDrop => "拖拽 拖放 列表排序",
            Self::ListView => "虚拟列表 大数据滚动",
            Self::Assets => "资产网格 缩略图 材质球 着色器",
            Self::Viewport => "无限画布 视口 四叉树",
            Self::Marquee => "框选 对齐吸附",
            Self::Undo => "撤销 重做 历史栈",
            Self::Springs => "弹簧动画 缓动物理",
            Self::A11y => "无障碍 语义树 焦点顺序",
            Self::Telemetry => "性能遥测 帧耗时 显存监控",
            Self::Settings => "设置 关于 双拼方案 小鹤双拼 帧率上限",
            Self::Components => "组件展厅 全部标准组件 目录",
            Self::Systems => "引擎子系统 架构全景 渲染管线",
            Self::I18n => "国际化 本地化 多语言 翻译 中英文切换 i18n locale",
        };
        let haystack = format!(
            "{} {} {} {} {} {} {} {} {}",
            info.title,
            info.nav_label(),
            info.description,
            info.keywords,
            info.crates.join(" "),
            self.category().label(),
            zh_info.title,
            zh_info.short,
            zh_keywords
        );
        let matcher = unbound_text::PinyinMatcher::new();
        query
            .split_whitespace()
            .all(|term| matcher.match_text(&haystack, term).is_some())
    }
}

/// 编译期完备性门禁：验证 `PageId::ALL` 中的每个页面都精确出现在 `CategoryId::Home`
/// 或 `CategoryId::ALL` 的侧边栏树中，任何页面或分类遗漏都会直接触发编译错误。
const _: () = {
    let mut i = 0;
    while i < PageId::ALL.len() {
        let page = PageId::ALL[i];
        assert!(
            page.index() == i,
            "PageId::ALL order must match PageId::index()",
        );
        let in_sidebar = if matches!(page, PageId::Home) {
            true
        } else {
            let mut c = 0;
            let mut found = false;
            while c < CategoryId::ALL.len() {
                let pages = CategoryId::ALL[c].pages();
                let mut p = 0;
                while p < pages.len() {
                    if pages[p] as usize == page as usize {
                        found = true;
                    }
                    p += 1;
                }
                c += 1;
            }
            found
        };
        assert!(
            in_sidebar,
            "every PageId in PageId::ALL must be present in the sidebar CategoryId::ALL",
        );
        i += 1;
    }
};

/// 静态页面元数据。
#[derive(Copy, Clone, Debug)]
pub struct PageInfo {
    pub title: &'static str,
    pub short: &'static str,
    pub description: &'static str,
    pub crates: &'static [&'static str],
    pub keywords: &'static str,
}

impl PageInfo {
    #[must_use]
    pub const fn nav_label(&self) -> &'static str {
        self.short
    }
}

/// 示例的一个可调属性。
#[derive(Copy, Clone, Debug)]
pub enum KnobSpec {
    Toggle {
        label: &'static str,
        default_on: bool,
    },
    CheckBox {
        label: &'static str,
        default_on: bool,
    },
    Slider {
        label: &'static str,
        min: f32,
        max: f32,
        step: f32,
        default: f32,
        unit: &'static str,
    },
    Segmented {
        label: &'static str,
        options: &'static [&'static str],
        default_index: usize,
    },
}

impl KnobSpec {
    pub const fn toggle(label: &'static str, default_on: bool) -> Self {
        Self::Toggle { label, default_on }
    }

    pub const fn check(label: &'static str, default_on: bool) -> Self {
        Self::CheckBox { label, default_on }
    }

    pub const fn slider(
        label: &'static str,
        min: f32,
        max: f32,
        step: f32,
        default: f32,
        unit: &'static str,
    ) -> Self {
        Self::Slider {
            label,
            min,
            max,
            step,
            default,
            unit,
        }
    }

    pub const fn segmented(
        label: &'static str,
        options: &'static [&'static str],
        default_index: usize,
    ) -> Self {
        Self::Segmented {
            label,
            options,
            default_index,
        }
    }

    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Toggle { label, .. }
            | Self::CheckBox { label, .. }
            | Self::Slider { label, .. }
            | Self::Segmented { label, .. } => label,
        }
    }

    #[must_use]
    pub fn default_value(&self) -> f32 {
        match self {
            Self::Toggle { default_on, .. } | Self::CheckBox { default_on, .. } => {
                if *default_on {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Slider { default, .. } => *default,
            Self::Segmented { default_index, .. } => *default_index as f32,
        }
    }

    #[must_use]
    pub const fn range(&self) -> Option<(f32, f32, f32)> {
        match self {
            Self::Slider { min, max, step, .. } => Some((*min, *max, *step)),
            _ => None,
        }
    }

    #[must_use]
    pub const fn options(&self) -> Option<&'static [&'static str]> {
        match self {
            Self::Segmented { options, .. } => Some(options),
            _ => None,
        }
    }
}

/// 示例实时舞台渲染的内容。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DemoKind {
    Hero,
    CapabilityGrid,
    ControlIndex,
    Palette {
        compare: bool,
    },
    Typography {
        scaling: bool,
    },
    ScriptMatrix,
    CurveInspector,
    BidiSandbox,
    Elevation {
        clipping: bool,
    },
    Vector {
        shapes: bool,
    },
    Buttons {
        icons: bool,
    },
    Selection {
        segmented: bool,
    },
    Sliders {
        progress: bool,
    },
    TextField {
        selection: bool,
    },
    /// 全量汉字全拼 + 6 套双拼 + 首字母实时检索与高亮区间沙盒。
    PinyinSandbox,
    Flex {
        box_model: bool,
    },
    Overlays {
        tooltips: bool,
    },
    DragDrop,
    VirtualList {
        metrics: bool,
    },
    AssetGrid,
    Viewport {
        quadtree: bool,
    },
    Marquee {
        snapping: bool,
    },
    Undo,
    Springs {
        easing: bool,
    },
    A11y {
        focus_rings: bool,
    },
    Telemetry {
        graph: bool,
    },
    About,
    /// 拼音与多方案双拼（默认小鹤双拼）运行时切换与实时检索沙盒。
    PinyinEngine,
    /// 引擎核心子系统架构全景与流水线舞台。
    Systems {
        mode: u8,
    },
    /// 原生 i18n 与本地化交互教学舞台。
    I18n {
        mode: u8,
    },
    /// 由其真实发射器绘制的单个目录组件。
    Component {
        id: WidgetCatalogId,
    },
}

/// 单个示例卡片定义。
#[derive(Copy, Clone, Debug)]
pub struct ExampleDef {
    pub title: &'static str,
    pub description: &'static str,
    pub crates: &'static [&'static str],
    pub code: &'static str,
    pub a11y: &'static str,
    pub knobs: &'static [KnobSpec],
    pub demo: DemoKind,
    pub stage_height: f32,
}

impl ExampleDef {
    #[must_use]
    pub const fn new(
        title: &'static str,
        description: &'static str,
        crates: &'static [&'static str],
        code: &'static str,
        a11y: &'static str,
        knobs: &'static [KnobSpec],
        demo: DemoKind,
    ) -> Self {
        Self {
            title,
            description,
            crates,
            code,
            a11y,
            knobs,
            demo,
            stage_height: 220.0,
        }
    }

    #[must_use]
    pub const fn with_height(mut self, height: f32) -> Self {
        self.stage_height = height;
        self
    }

    /// 按当前语言返回示例卡片的本地化标题与简介。
    #[must_use]
    pub const fn localized_header(&self, locale: Locale) -> (&'static str, &'static str) {
        if !locale.is_zh() {
            return (self.title, self.description);
        }
        match self.demo {
            DemoKind::Hero => (
                "首屏横幅 (Hero Banner)",
                "组合表面：多层圆角矩形、解析阴影与贝塞尔丝带曲线，全部来自与外壳相同的图元流。",
            ),
            DemoKind::CapabilityGrid => (
                "核心特性入口 (Featured Capabilities)",
                "通往框架四大差异化特性的快捷入口，点击任意卡片即可像侧边栏导航一样跳转。",
            ),
            DemoKind::ControlIndex => (
                "全部页面索引 (All Controls)",
                "展厅页面网格索引：每个磁贴都是真实的导航目标，使首页本身即自举的导航视图。",
            ),
            DemoKind::Palette { compare: false } => (
                "语义色板 (Semantic Swatches)",
                "外壳消费的十二个核心颜色令牌，同时展示语义角色名与存储在图元中的槽位编号。",
            ),
            DemoKind::Palette { compare: true } => (
                "亮暗主题对等 (Light / Dark Parity)",
                "亮色与暗色主题共享完全相同的槽位布局，切换主题仅重写 4 KiB 颜色查找表。",
            ),
            DemoKind::Typography { scaling: false } => (
                "字号阶梯 (Type Scale)",
                "七级字阶体系：正文字号支持通过右侧旋钮实时调节，字形共享同一图集缓存。",
            ),
            DemoKind::Typography { scaling: true } => (
                "极限缩放 (Extreme Scaling)",
                "字形四边形携带屏幕空间偏导数，400% 放大下依然呈现平滑的 MTSDF 矢量轮廓。",
            ),
            DemoKind::ScriptMatrix => (
                "全球书写系统矩阵 (Global Script Matrix)",
                "同一字号下渲染十二种文字系统，右侧徽章实时回读塑形器实际命中的系统字体族。",
            ),
            DemoKind::CurveInspector => (
                "曲线与字干锐度检查器 (Curve & Stem Sharpness)",
                "针对边角敏感字符（O/G/C/Q/S 与汉字永/国/回）的七级字号瀑布流与五级字重测试。",
            ),
            DemoKind::BidiSandbox => (
                "多语言与双向文本沙盒 (Multilingual & BiDi Sandbox)",
                "支持阿拉伯语/希伯来语 RTL 与数字混排，切换到自定义预设即可直接使用输入法键入。",
            ),
            DemoKind::Elevation { clipping: false } => (
                "海拔阴影阶梯 (Elevation Ladder)",
                "三级解析阴影：模糊半径与圆角均为单精度 uniform，平滑阴影与扁平矩形开销一致。",
            ),
            DemoKind::Elevation { clipping: true } => (
                "圆角裁剪 (Rounded Clipping)",
                "子内容由父表面的 SDF 圆角精确裁剪，切换右侧开关可观察溢出边界。",
            ),
            DemoKind::Vector { shapes: false } => (
                "贝塞尔丝带 (Bézier Ribbons)",
                "二次曲线段由超级着色器直接描边，曲率旋钮可实时移动控制点，线宽在缩放下保持稳定。",
            ),
            DemoKind::Vector { shapes: true } => (
                "几何形状语言 (Shape Language)",
                "圆角表面、圆环与丝带共用同一套图元，构成整套图标与首屏装饰的几何词汇表。",
            ),
            DemoKind::Buttons { icons: false } => (
                "按钮变体 (Button Variants)",
                "主按钮、次按钮、轻量按钮与图标按钮共享统一命中标识，点击按钮可验证完整事件闭环。",
            ),
            DemoKind::Buttons { icons: true } => (
                "几何图标按钮 (Icon Buttons)",
                "标题栏与导航栏使用的几何图标词汇表，纯图元绘制，自动继承主题色与像素网格。",
            ),
            DemoKind::Selection { segmented: false } => (
                "开关、复选与单选 (Switch, Check & Radio)",
                "三种选择模型遵循相同的焦点契约：单选按钮成组互斥，开关与复选框相互独立。",
            ),
            DemoKind::Selection { segmented: true } => (
                "分段控制器 (Segmented Control)",
                "带平滑滑动指示胶囊的单选分段控件，由物理弹簧驱动过渡动画。",
            ),
            DemoKind::Sliders { progress: false } => (
                "连续滑块 (Continuous Sliders)",
                "在轨道任意位置按下拖拽，指针捕获确保光标移出控件行后滑块依然精准跟随。",
            ),
            DemoKind::Sliders { progress: true } => (
                "进度条与进度环 (Progress)",
                "确定态与不确定态进度指示器，进度条与圆环均由真实的 emit_progress 发射器绘制。",
            ),
            DemoKind::TextField { selection: false } => (
                "聚焦文本输入框 (Focused Text Field)",
                "点击输入框即可键入文字，支持剪贴板复制/剪切/粘贴，光标随动画时钟闪烁。",
            ),
            DemoKind::TextField { selection: true } => (
                "文本选区与输入法候选条 (Selection & IME)",
                "展示词级/行级选区高亮矩形，以及锚定在光标下方的 IME 预编辑与候选词条。",
            ),
            DemoKind::PinyinSandbox => (
                "拼音与双拼实时过滤沙盒 (Pinyin & Shuangpin Sandbox)",
                "覆盖 20,992 汉字与多音字，支持全拼、6 套双拼（默认小鹤双拼）与首字母缩写实时匹配高亮。",
            ),
            DemoKind::Flex { box_model: false } => (
                "弹性布局沙盒 (Flex Sandbox)",
                "实时弹性容器中的四个子节点，虚线框显示求解后的内容盒，支持切换方向、间距与对齐。",
            ),
            DemoKind::Flex { box_model: true } => (
                "盒模型检视器 (Box Model)",
                "外边距（Margin）、边框（Border）、内边距（Padding）与内容区四层嵌套表面可视化。",
            ),
            DemoKind::Overlays { tooltips: false } => (
                "菜单与模态对话框 (Menus & Dialogs)",
                "菜单与对话框均在 Portal 层（Layer 2）绘制并优先捕获点击，背景遮罩阻断底层输入。",
            ),
            DemoKind::Overlays { tooltips: true } => (
                "窗口内自适应工具提示 (In-Window Auto-Positioning Tooltips)",
                "悬停四角或中心按钮：TooltipDesc 自动检测窗口边界并翻转方向、钳制安全边距。",
            ),
            DemoKind::DragDrop => (
                "可拖拽排序卡片 (Sortable Cards)",
                "按住并拖拽任意卡片，放置目标针对四个槽位实时解析，释放鼠标时提交新排序。",
            ),
            DemoKind::VirtualList { metrics: false } => (
                "100,000 行虚拟列表 (100 000 Rows)",
                "滚动舞台或拖拽右侧滚动条：无论数据集多大，活跃节点数始终保持恒定。",
            ),
            DemoKind::VirtualList { metrics: true } => (
                "虚拟化遥测指标 (Virtualization Metrics)",
                "右侧面板实时展示虚拟化器遥测：可见行范围、预取区、实例化节点数与每帧复用计数。",
            ),
            DemoKind::AssetGrid => (
                "渐进式流式图像网格 (Progressive Image Grid)",
                "每个图块执行异步解码、环形暂存缓冲上传与纹理切换，加载中自动显示骨架屏微光。",
            ),
            DemoKind::Viewport { quadtree: false } => (
                "平移、缩放与视口剔除 (Pan, Zoom & Cull)",
                "在舞台上滚动滚轮缩放、拖拽平移，右上角与底栏实时报告四叉树剔除后的可见节点数。",
            ),
            DemoKind::Viewport { quadtree: true } => (
                "四叉树空间索引覆盖层 (Quadtree Overlay)",
                "直接可视化松散四叉树空间划分：每一层级将分块四等分，直至达到深度预算。",
            ),
            DemoKind::Marquee { snapping: false } => (
                "矩形框选 (Box Selection)",
                "在舞台空白处拖拽拉出框选矩形，中心点落在框内的节点将在释放时被选中。",
            ),
            DemoKind::Marquee { snapping: true } => (
                "智能对齐参考线 (Smart Guides)",
                "拖拽过程中，落入吸附阈值内的节点边缘与中心会自动发射水平与垂直对齐参考线。",
            ),
            DemoKind::Undo => (
                "撤销 / 重做历史时间线 (History Timeline)",
                "点击按钮推入新移动命令并测试撤销与重做，推入新命令时自动截断重做分支。",
            ),
            DemoKind::Springs { easing: false } => (
                "物理弹簧实验室 (Spring Lab)",
                "切换目标位置观察滑块收敛，刚度与阻尼作为物理参数实时调节，实时报告速度与过冲。",
            ),
            DemoKind::Springs { easing: true } => (
                "交互式三次贝塞尔曲线 (Cubic Easing)",
                "直接拖拽两个控制点手柄重塑缓动曲线，或通过右侧预设加载标准曲线。",
            ),
            DemoKind::A11y { focus_rings: false } => (
                "无障碍语义树镜像 (Semantic Tree)",
                "点击左侧按钮切换焦点，右侧面板实时镜像导出的语义节点角色、名称与聚焦状态。",
            ),
            DemoKind::A11y { focus_rings: true } => (
                "无障碍焦点环 (Focus Rings)",
                "焦点环粗细与对比度均由主题令牌驱动，作为外扩描边圆角矩形绘制，绝不引起布局抖动。",
            ),
            DemoKind::Telemetry { graph: false } => (
                "运行时 KPI 仪表盘 (Runtime KPIs)",
                "当前帧实时计数器：帧耗时、FPS、图元总数、间接绘制调用、上传字节数与帧预算。",
            ),
            DemoKind::Telemetry { graph: true } => (
                "滚动帧耗时曲线图 (Frame Graph)",
                "带 16.7ms 预算参考线的实时滑动窗口图表，由真实帧历史数据与 Chart 发射器绘制。",
            ),
            DemoKind::About => (
                "关于本构建版本 (About This Build)",
                "Unbound UI 引擎级 Rust 图形界面工作区概览，以及实时生效的全局帧率上限控制器。",
            ),
            DemoKind::PinyinEngine => (
                "拼音与双拼运行时配置台 (Pinyin & Shuangpin Config)",
                "实时切换小鹤、自然码、微软、搜狗、紫光、智能ABC 六套双拼方案并联动全局搜索框。",
            ),
            DemoKind::Systems { mode: 0 } => (
                "21 个引擎核心子系统矩阵 (21 Engine Subsystems Matrix)",
                "工作区全部 21 个核心 crate 交互式目录，点击任意子系统磁贴可检视其分层、职责与红线指标。",
            ),
            DemoKind::Systems { mode: 1 } => (
                "6 阶段帧执行管线与场景差分 (Frame Pipeline & Scene Diff)",
                "从 Win32 原生输入、命中路由、Taffy 增量排版到恒定形状图元差分与 Vulkan 1.3 呈现。",
            ),
            DemoKind::Systems { .. } => (
                "Host-Guest ABI 与无头几何体检 (ABI & Headless Sanitizers)",
                "零依赖客体插件通过 #[repr(C)] ABI 发射节点，配合 5 项无头几何与对比度规则毫秒级自检。",
            ),
            DemoKind::I18n { mode: 0 } => (
                "交互式 i18n 资源包与插值沙盒 (i18n Bundle & Interpolation)",
                "实时演练键值查找、{name} 占位符插值、单复数规则（.zero/.one/.other）与缺失键自动回退。",
            ),
            DemoKind::I18n { mode: 1 } => (
                "区域感知数字/日期格式化与 RTL 检视器 (Locale Formatting & RTL)",
                "对比不同区域（en-US / zh-CN / de-DE / ar-SA）下的千分位数字、日历日期与阅读方向。",
            ),
            DemoKind::I18n { .. } => (
                "三步接入原生 i18n 指南 (3-Step Integration Guide)",
                "手把手教你如何在自己的 Unbound UI 项目中注册双语词条、绑定语言切换与声明式渲染。",
            ),
            DemoKind::Component { id } => (
                components::component_nav_label_localized(id, locale),
                self.description,
            ),
        }
    }
}

/// 传递给每个演示渲染器的上下文。
pub struct DemoCtx<'a, 'b> {
    pub canvas: &'a mut Canvas<'b>,
    pub state: &'a GalleryState,
    pub page: PageId,
    pub card: usize,
    pub rect: Rect,
    pub knobs: &'a [f32],
}

impl DemoCtx<'_, '_> {
    #[must_use]
    pub fn knob(&self, index: usize) -> f32 {
        self.knobs.get(index).copied().unwrap_or(0.0)
    }

    #[must_use]
    pub fn knob_on(&self, index: usize) -> bool {
        self.knob(index) > 0.5
    }

    #[must_use]
    pub fn part(&self, part: u16) -> u64 {
        crate::target::id(crate::target::Target::Demo(
            self.page.index() as u8,
            self.card as u8,
            part,
        ))
    }

    pub fn control(
        &mut self,
        part: u16,
        rect: Rect,
        role: accesskit::Role,
        label: &str,
        selected: bool,
    ) -> u64 {
        self.canvas.demo_control(
            self.page.index() as u8,
            self.card as u8,
            part,
            rect,
            role,
            label,
            selected,
        );
        self.part(part)
    }
}

/// 渲染单个示例的实时舞台。
pub fn render_demo(ctx: &mut DemoCtx<'_, '_>, demo: DemoKind) {
    match demo {
        DemoKind::Hero => home::hero(ctx),
        DemoKind::CapabilityGrid => home::capability_grid(ctx),
        DemoKind::ControlIndex => home::control_index(ctx),
        DemoKind::Palette { compare } => design::palette(ctx, compare),
        DemoKind::Typography { scaling } => design::typography(ctx, scaling),
        DemoKind::ScriptMatrix => fonts::script_matrix(ctx),
        DemoKind::CurveInspector => fonts::curve_inspector(ctx),
        DemoKind::BidiSandbox => fonts::bidi_sandbox(ctx),
        DemoKind::Elevation { clipping } => design::elevation(ctx, clipping),
        DemoKind::Vector { shapes } => design::vector(ctx, shapes),
        DemoKind::Buttons { icons } => input::buttons(ctx, icons),
        DemoKind::Selection { segmented } => input::selection(ctx, segmented),
        DemoKind::Sliders { progress } => input::sliders(ctx, progress),
        DemoKind::TextField { selection } => input::text_field(ctx, selection),
        DemoKind::PinyinSandbox => input::pinyin_sandbox(ctx),
        DemoKind::Flex { box_model } => layout::flex(ctx, box_model),
        DemoKind::Overlays { tooltips } => layout::overlays(ctx, tooltips),
        DemoKind::DragDrop => layout::drag_drop(ctx),
        DemoKind::VirtualList { metrics } => collections::virtual_list(ctx, metrics),
        DemoKind::AssetGrid => collections::asset_grid(ctx),
        DemoKind::Viewport { quadtree } => canvas::viewport(ctx, quadtree),
        DemoKind::Marquee { snapping } => canvas::marquee(ctx, snapping),
        DemoKind::Undo => canvas::undo(ctx),
        DemoKind::Springs { easing } => motion::springs(ctx, easing),
        DemoKind::A11y { focus_rings } => motion::a11y(ctx, focus_rings),
        DemoKind::Telemetry { graph } => motion::telemetry(ctx, graph),
        DemoKind::About => motion::about(ctx),
        DemoKind::PinyinEngine => motion::pinyin_engine(ctx),
        DemoKind::Systems { mode } => systems::systems(ctx, mode),
        DemoKind::I18n { mode } => i18n::i18n_demo(ctx, mode),
        DemoKind::Component { id } => components::component(ctx, id),
    }
}
