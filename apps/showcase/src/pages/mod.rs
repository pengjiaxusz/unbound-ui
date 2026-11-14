//! 展厅目录：分类、页面与示例定义。
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
pub mod input;
pub mod layout;
pub mod motion;

use unbound_math::Rect;
use unbound_widgets::WidgetCatalogId;

use crate::canvas::Canvas;
use crate::state::GalleryState;

pub use components::showcase_entry_for_widget;

/// 七个导航分类。
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
    /// 导航树中渲染的分类。
    ///
    /// `Design` 也是树的一部分：此前将其漏掉导致整个设计分区
    /// （色彩、排版、层级阴影、矢量图形、字体）只能通过总是打开 Colors 的
    /// 单个“Design Guidance”行访问。
    pub const ALL: [CategoryId; 7] = [
        CategoryId::Design,
        CategoryId::Basic,
        CategoryId::Layout,
        CategoryId::Collections,
        CategoryId::Canvas,
        CategoryId::Motion,
        CategoryId::Components,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Design => "Design Guidance",
            Self::Basic => "Basic Input",
            Self::Layout => "Layout & Portals",
            Self::Collections => "Collections",
            Self::Canvas => "Infinite Canvas",
            Self::Motion => "Motion, A11y & Telemetry",
            Self::Components => "Components",
            Self::System => "System",
        }
    }

    #[must_use]
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
            Self::System => &[PageId::Settings],
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
}

impl PageId {
    pub const ALL: [PageId; 23] = [
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
    ];

    #[must_use]
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
        }
    }

    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(PageId::Home)
    }

    #[must_use]
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
            Self::Settings => CategoryId::System,
        }
    }

    #[must_use]
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
        }
    }

    #[must_use]
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
        }
    }

    /// 不区分大小写的目录搜索。
    #[must_use]
    pub fn matches(self, query: &str) -> bool {
        let info = self.info();
        let haystack = format!(
            "{} {} {} {} {} {}",
            info.title,
            info.nav_label(),
            info.description,
            info.keywords,
            info.crates.join(" "),
            self.category().label()
        )
        .to_lowercase();
        query.split_whitespace().all(|term| haystack.contains(term))
    }
}

/// 静态页面元数据。
#[derive(Copy, Clone, Debug)]
pub struct PageInfo {
    pub title: &'static str,
    pub short: &'static str,
    pub description: &'static str,
    pub crates: &'static [&'static str],
    pub docs: &'static str,
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
        DemoKind::Component { id } => components::component(ctx, id),
    }
}
