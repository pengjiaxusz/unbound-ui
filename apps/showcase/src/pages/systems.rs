//! 引擎核心子系统架构全景展示页（Systems）。
//!
//! 将工作区内全部 21 个核心引擎库子系统（`unbound-math` 至 `unbound-kpi`）纳入 Showcase 专页，
//! 并通过 [`SubsystemId`] 穷举枚举与编译期 `const` 断言确保每一个引擎子系统均被完整收录与展示。

use accesskit::Role;
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets;

pub const SYSTEMS_INFO: PageInfo = PageInfo {
    title: "Engine Subsystems & Architecture",
    short: "Subsystems",
    description: "All 21 engine crates and runtime subsystems — from SIMD geometry and Vulkan 1.3 bindless rendering to Win32 (windows 0.62.2) windowing, Taffy layout, CoW canvas undo, and headless verification.",
    crates: &[
        "unbound-platform",
        "unbound-gpu",
        "unbound-layout",
        "unbound-widgets",
        "unbound-app",
    ],
    docs: "docs/architecture/README.md",
    keywords: "subsystems architecture engine crates platform win32 windows gpu layout text world anim canvas input a11y theme widgets app asset graph headless host guest kpi",
};

/// 引擎 21 个核心子系统枚举。
///
/// 未标记 `#[non_exhaustive]`，配合下方 `#[deny(unreachable_patterns)]` 与编译期 `const` 断言，
/// 确保所有子系统均提供完整元数据并在 Showcase 中渲染。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SubsystemId {
    Math,
    Abi,
    Gpu,
    Vector,
    Text,
    World,
    Layout,
    Anim,
    Canvas,
    Input,
    A11y,
    Theme,
    Widgets,
    Platform,
    App,
    Asset,
    Graph,
    Headless,
    Host,
    AppGuest,
    Kpi,
}

impl SubsystemId {
    /// 工作区全部 21 个核心库子系统。
    pub const ALL: [SubsystemId; 21] = [
        Self::Math,
        Self::Abi,
        Self::Gpu,
        Self::Vector,
        Self::Text,
        Self::World,
        Self::Layout,
        Self::Anim,
        Self::Canvas,
        Self::Input,
        Self::A11y,
        Self::Theme,
        Self::Widgets,
        Self::Platform,
        Self::App,
        Self::Asset,
        Self::Graph,
        Self::Headless,
        Self::Host,
        Self::AppGuest,
        Self::Kpi,
    ];

    /// 子系统总数。
    pub const COUNT: usize = Self::ALL.len();

    /// 子系统在 [`Self::ALL`] 中的稳定索引。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn index(self) -> usize {
        match self {
            Self::Math => 0,
            Self::Abi => 1,
            Self::Gpu => 2,
            Self::Vector => 3,
            Self::Text => 4,
            Self::World => 5,
            Self::Layout => 6,
            Self::Anim => 7,
            Self::Canvas => 8,
            Self::Input => 9,
            Self::A11y => 10,
            Self::Theme => 11,
            Self::Widgets => 12,
            Self::Platform => 13,
            Self::App => 14,
            Self::Asset => 15,
            Self::Graph => 16,
            Self::Headless => 17,
            Self::Host => 18,
            Self::AppGuest => 19,
            Self::Kpi => 20,
        }
    }

    /// 对应的 Cargo crate 名称。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn crate_name(self) -> &'static str {
        match self {
            Self::Math => "unbound-math",
            Self::Abi => "unbound-abi",
            Self::Gpu => "unbound-gpu",
            Self::Vector => "unbound-vector",
            Self::Text => "unbound-text",
            Self::World => "unbound-world",
            Self::Layout => "unbound-layout",
            Self::Anim => "unbound-anim",
            Self::Canvas => "unbound-canvas",
            Self::Input => "unbound-input",
            Self::A11y => "unbound-a11y",
            Self::Theme => "unbound-theme",
            Self::Widgets => "unbound-widgets",
            Self::Platform => "unbound-platform",
            Self::App => "unbound-app",
            Self::Asset => "unbound-asset",
            Self::Graph => "unbound-graph",
            Self::Headless => "unbound-headless",
            Self::Host => "unbound-host",
            Self::AppGuest => "app-guest",
            Self::Kpi => "unbound-kpi",
        }
    }

    /// 架构分层编号（`0` = 基础与渲染层，`1` = 场景与交互层，`2` = 运行时与平台层）。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn layer_group(self) -> u8 {
        match self {
            Self::Math
            | Self::Abi
            | Self::Gpu
            | Self::Vector
            | Self::Text
            | Self::Graph
            | Self::Asset => 0,
            Self::World
            | Self::Layout
            | Self::Anim
            | Self::Canvas
            | Self::Input
            | Self::A11y
            | Self::Theme => 1,
            Self::Widgets
            | Self::Platform
            | Self::App
            | Self::Headless
            | Self::Host
            | Self::AppGuest
            | Self::Kpi => 2,
        }
    }

    /// 子系统职责简述。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Math => "2D/3D 几何原语、f16 打包、OKLCH 色彩与圆角半径投影",
            Self::Abi => "C 稳定内存布局描述符、跨进程 POD 协议与节点标志",
            Self::Gpu => "Vulkan 1.3 动态渲染、Bindless 图集、64B 图元实例与基数排序",
            Self::Vector => "CPU 贝塞尔扁平化、解析抗锯齿描边与布尔路径网格",
            Self::Text => "Cosmic-Text 多语言双向整形、四相位字形缓存与图集驱逐",
            Self::World => "SlotMap 实体节点树、重布局边界追踪、空间四叉树与虚拟化",
            Self::Layout => "增量 Flexbox / Grid 求解器、脏子树跳过与统计探针",
            Self::Anim => "临界/欠阻尼解析弹簧物理引擎、三次贝塞尔曲线与插值",
            Self::Canvas => "无限画布相机变换、磁吸对齐网格与 CoW 事务撤销栈",
            Self::Input => "分层命中路由、指针捕获、焦点环游、快捷键与拖拽状态机",
            Self::A11y => "AccessKit 无障碍语义树构建器与零开销静态帧增量发布",
            Self::Theme => "OKLCH 感知调色板、4 KiB GPU 颜色查找表与零重传主题切换",
            Self::Widgets => "99 个标准组件、24 态恒定图元形状契约与窗口内自适应浮层",
            Self::Platform => "基于 windows 0.62.2 的原生 Win32 窗口、DWM 边框缩放、剪贴板与托盘",
            Self::App => "MVU 应用运行时、双遍声明式视图构建器、定时器与异步任务池",
            Self::Asset => "异步 PNG 流式解码中心、环形暂存缓冲与图集批量上传",
            Self::Graph => "FrameGraph 渲染通道编排、Kawase 背景毛玻璃模糊与屏障调度",
            Self::Headless => "无头几何验证器、五项布局消毒规则与时间旅行二进制快照",
            Self::Host => "沙盒插件宿主、动态库热重载与跨 ABI 共享内存通道",
            Self::AppGuest => "零引擎依赖客体 SDK，仅通过 unbound-abi 构建声明式界面",
            Self::Kpi => "性能红线基准门禁（8.3ms 延迟预算、局部修补字节上限）",
        }
    }

    /// 核心量化指标或架构契约。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn key_metric(self) -> &'static str {
        match self {
            Self::Math => "0 alloc · f16/f32 SIMD",
            Self::Abi => "#[repr(C)] POD ABI",
            Self::Gpu => "64 B/prim · 4 draws",
            Self::Vector => "Analytic AA · curves",
            Self::Text => "4 subpixel phases",
            Self::World => "O(1) SlotMap + Quadtree",
            Self::Layout => "Relayout boundaries",
            Self::Anim => "Exact ODE spring",
            Self::Canvas => "CoW UndoStack",
            Self::Input => "Layered HitRouter",
            Self::A11y => "AccessKit delta tree",
            Self::Theme => "4 KiB LUT · 0 upload",
            Self::Widgets => "99 widgets · 24 states",
            Self::Platform => "windows 0.62.2 Win32",
            Self::App => "MVU + SceneDiff",
            Self::Asset => "Async StagingRing",
            Self::Graph => "Pass DAG + Backdrop",
            Self::Headless => "5 layout sanitizers",
            Self::Host => "Hot-reload IPC host",
            Self::AppGuest => "Zero-engine guest",
            Self::Kpi => "< 8.3 ms redline",
        }
    }
}

/// 编译期完备性门禁：验证全部 21 个核心子系统的索引、名称、摘要与指标均有效且互不冲突。
const _: () = {
    assert!(
        SubsystemId::COUNT == 21,
        "workspace must have all 21 core subsystems registered"
    );
    let mut i = 0;
    while i < SubsystemId::COUNT {
        let sub = SubsystemId::ALL[i];
        assert!(
            sub.index() == i,
            "SubsystemId::index() must match position in SubsystemId::ALL"
        );
        assert!(
            !sub.crate_name().is_empty(),
            "subsystem crate_name must not be empty"
        );
        assert!(
            !sub.summary().is_empty(),
            "subsystem summary must not be empty"
        );
        assert!(
            !sub.key_metric().is_empty(),
            "subsystem key_metric must not be empty"
        );
        assert!(
            sub.layer_group() <= 2,
            "subsystem layer_group must be 0, 1, or 2"
        );
        i += 1;
    }
};

pub const SYSTEMS_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "21 Engine Subsystems Matrix",
        "Interactive directory of all 21 workspace crates. Click any subsystem tile to inspect its architecture layer, responsibility, and redline contract.",
        &[
            "unbound-math",
            "unbound-abi",
            "unbound-gpu",
            "unbound-platform",
            "unbound-widgets",
            "unbound-app",
        ],
        r#"for sub in SubsystemId::ALL {
    let crate_name = sub.crate_name();
    let contract = sub.key_metric();
    let summary = sub.summary();
}"#,
        "21 interactive subsystem tiles grouped into Foundation & GPU, Scene & Layout, and Runtime & Platform layers.",
        &[
            KnobSpec::segmented("Layer Filter", &["All (21)", "Foundation", "Scene", "Runtime"], 0),
            KnobSpec::toggle("Show Metrics", true),
        ],
        DemoKind::Systems { mode: 0 },
    )
    .with_height(340.0),
    ExampleDef::new(
        "Frame Execution Pipeline & Scene Diff",
        "End-to-end 6-stage frame pipeline from Win32 (windows 0.62.2) input and hit routing to Taffy layout, constant-shape widget diffing, and Vulkan 1.3 presentation.",
        &[
            "unbound-platform",
            "unbound-input",
            "unbound-layout",
            "unbound-widgets",
            "unbound-graph",
            "unbound-gpu",
        ],
        r#"// 1. Win32 Input -> 2. HitRouter -> 3. MVU Reduce
// 4. Relayout Boundaries -> 5. SceneDiff -> 6. Vulkan Present
let report = runtime.frame(dt);
let plan = upload_plan(generation, &report.diff, dropped);"#,
        "Six pipeline stage cards showing live frame timing, primitive count, and diff upload bytes.",
        &[
            KnobSpec::segmented("Highlight Stage", &["Input", "Layout", "Diff", "Present"], 2),
            KnobSpec::toggle("Simulate Patch", true),
        ],
        DemoKind::Systems { mode: 1 },
    )
    .with_height(260.0),
    ExampleDef::new(
        "Host-Guest ABI & Headless Sanitizer Loop",
        "Zero-dependency guest plugins emit #[repr(C)] RawNodeDesc streams across unbound-abi while unbound-headless verifies 5 geometric and contrast rules in under 1 ms.",
        &[
            "unbound-abi",
            "unbound-host",
            "app-guest",
            "unbound-headless",
            "unbound-kpi",
        ],
        r#"let report = Harness::run(&app.frame());
assert!(report.is_clean(), "{}", report.summary());"#,
        "Live status of the 5 headless sanitizers (NoNaN, TextClip, HitTargetSize, SiblingOverlap, ViewportContainment) and Host-Guest ABI channel.",
        &[
            KnobSpec::toggle("Show Rule Thresholds", true),
        ],
        DemoKind::Systems { mode: 2 },
    )
    .with_height(250.0),
];

/// 渲染系统架构专页的三个交互舞台。
pub fn systems(ctx: &mut DemoCtx<'_, '_>, mode: u8) {
    match mode {
        0 => draw_subsystems_matrix(ctx),
        1 => draw_pipeline_stage(ctx),
        _ => draw_abi_and_headless(ctx),
    }
}

fn draw_subsystems_matrix(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let filter = ctx.knob(0) as u8;
    let show_metrics = ctx.knob_on(1);

    let selected_idx = ctx
        .state
        .demo
        .selected_subsystem
        .min(SubsystemId::COUNT - 1);
    let selected_sub = SubsystemId::ALL[selected_idx];

    // 顶部选中子系统详情横幅
    let detail_rect = Rect::new(rect.origin.x, rect.origin.y, rect.size.x, 62.0);
    ctx.canvas.rect(
        detail_rect,
        10.0,
        palette.card_bg,
        palette.primary,
        1.5,
        0.0,
    );
    let layer_badge_label = match selected_sub.layer_group() {
        0 => "LAYER 0 · FOUNDATION & GPU",
        1 => "LAYER 1 · SCENE & LAYOUT",
        _ => "LAYER 2 · RUNTIME & PLATFORM",
    };
    let badge_w = (ctx.canvas.labels.measure(layer_badge_label, 9.5).x + 16.0).min(210.0);
    widgets::badge(
        ctx.canvas,
        Rect::new(
            detail_rect.origin.x + 12.0,
            detail_rect.origin.y + 10.0,
            badge_w,
            18.0,
        ),
        layer_badge_label,
        palette.primary,
        palette.primary_soft,
    );
    ctx.canvas.text(
        &format!(
            "{}  —  {}",
            selected_sub.crate_name(),
            selected_sub.key_metric()
        ),
        Rect::new(
            detail_rect.origin.x + badge_w + 22.0,
            detail_rect.origin.y + 8.0,
            (detail_rect.size.x - badge_w - 34.0).max(40.0),
            20.0,
        ),
        12.5,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
    ctx.canvas.text(
        selected_sub.summary(),
        Rect::new(
            detail_rect.origin.x + 12.0,
            detail_rect.origin.y + 34.0,
            (detail_rect.size.x - 24.0).max(40.0),
            20.0,
        ),
        10.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    // 过滤后的子系统列表
    let filtered: Vec<SubsystemId> = SubsystemId::ALL
        .into_iter()
        .filter(|sub| filter == 0 || sub.layer_group() + 1 == filter)
        .collect();

    let grid_top = detail_rect.max().y + 10.0;
    let grid_h = (rect.max().y - grid_top).max(60.0);
    let cols = if filtered.len() <= 7 {
        4usize
    } else if rect.size.x >= 680.0 {
        7usize
    } else {
        4usize
    };
    let rows = filtered.len().div_ceil(cols).max(1);
    let gap = 6.0;
    let cell_w = ((rect.size.x - gap * (cols.saturating_sub(1) as f32)) / cols as f32).max(56.0);
    let cell_h = ((grid_h - gap * (rows.saturating_sub(1) as f32)) / rows as f32).clamp(28.0, 56.0);

    for (slot, sub) in filtered.iter().copied().enumerate() {
        let col = slot % cols;
        let row = slot / cols;
        let cell = Rect::new(
            rect.origin.x + col as f32 * (cell_w + gap),
            grid_top + row as f32 * (cell_h + gap),
            cell_w,
            cell_h,
        );
        if cell.max().y > rect.max().y + 1.0 {
            continue;
        }
        let part = 100 + sub.index() as u16;
        let is_selected = sub.index() == selected_idx;
        let hover = ctx.canvas.visual(ctx.part(part), is_selected);
        let bg = if is_selected {
            palette.primary_soft
        } else if hover.is_hovered() {
            palette.control_hover
        } else {
            palette.card_bg
        };
        let border = if is_selected {
            palette.primary
        } else {
            palette.card_border
        };
        ctx.canvas.rect(cell, 7.0, bg, border, 1.0, 0.0);

        let short_name = sub
            .crate_name()
            .strip_prefix("unbound-")
            .unwrap_or(sub.crate_name());
        ctx.canvas.text(
            short_name,
            Rect::new(
                cell.origin.x + 6.0,
                cell.origin.y + 4.0,
                (cell.size.x - 12.0).max(20.0),
                16.0,
            ),
            10.5,
            if is_selected {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
        if show_metrics && cell_h >= 38.0 {
            ctx.canvas.text(
                sub.key_metric(),
                Rect::new(
                    cell.origin.x + 6.0,
                    cell.origin.y + 21.0,
                    (cell.size.x - 12.0).max(20.0),
                    14.0,
                ),
                8.5,
                palette.text_muted,
                AlignX::Start,
                AlignY::Center,
            );
        }
        ctx.control(
            part,
            cell,
            Role::Button,
            &format!("{} — {}", sub.crate_name(), sub.summary()),
            is_selected,
        );
    }
}

fn draw_pipeline_stage(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let active_stage = (ctx.knob(0) as usize).min(3);
    let patch_mode = ctx.knob_on(1);

    let stages = [
        (
            "1. Win32 Platform",
            "windows 0.62.2 · WM_NCHITTEST",
            "unbound-platform",
        ),
        (
            "2. Input & A11y",
            "HitRouter + Focus + AccessKit",
            "unbound-input",
        ),
        (
            "3. Layout & World",
            "Taffy + Relayout Boundaries",
            "unbound-layout",
        ),
        (
            "4. Diff & Vulkan",
            if patch_mode {
                "SceneDiff::Ranges (<1KB)"
            } else {
                "SceneDiff::Whole"
            },
            "unbound-gpu",
        ),
    ];

    let gap = 10.0;
    let card_w = ((rect.size.x - gap * 3.0) / 4.0).max(90.0);
    let card_h = (rect.size.y - 52.0).clamp(80.0, 150.0);

    for (idx, (title, detail, krate)) in stages.iter().enumerate() {
        let card = Rect::new(
            rect.origin.x + idx as f32 * (card_w + gap),
            rect.origin.y + 8.0,
            card_w,
            card_h,
        );
        let highlighted = idx == active_stage;
        ctx.canvas.rect(
            card,
            10.0,
            if highlighted {
                palette.primary_soft
            } else {
                palette.card_bg
            },
            if highlighted {
                palette.primary
            } else {
                palette.card_border
            },
            if highlighted { 1.5 } else { 1.0 },
            0.0,
        );
        ctx.canvas.text(
            title,
            Rect::new(
                card.origin.x + 10.0,
                card.origin.y + 10.0,
                card.size.x - 20.0,
                18.0,
            ),
            11.0,
            if highlighted {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text_wrapped(
            detail,
            Rect::new(
                card.origin.x + 10.0,
                card.origin.y + 34.0,
                card.size.x - 20.0,
                36.0,
            ),
            9.5,
            palette.text_muted,
        );
        widgets::badge(
            ctx.canvas,
            Rect::new(
                card.origin.x + 10.0,
                card.max().y - 28.0,
                (card.size.x - 20.0).min(126.0),
                18.0,
            ),
            krate,
            palette.primary,
            palette.stage_bg,
        );
    }

    let footer = Rect::new(rect.origin.x, rect.max().y - 32.0, rect.size.x, 26.0);
    ctx.canvas
        .rect(footer, 6.0, palette.card_bg, palette.card_border, 1.0, 0.0);
    ctx.canvas.text(
        &format!(
            "Live Frame: {:.2} ms  ·  Primitives: {}  ·  Upload: {} B  ·  Mode: {}",
            ctx.state.demo.frame_ms,
            ctx.state.demo.primitive_count,
            ctx.state.demo.upload_bytes,
            if patch_mode {
                "Incremental Range Patch"
            } else {
                "Full Scene Upload"
            }
        ),
        Rect::new(
            footer.origin.x + 12.0,
            footer.origin.y,
            footer.size.x - 24.0,
            footer.size.y,
        ),
        10.0,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
}

fn draw_abi_and_headless(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let show_thresholds = ctx.knob_on(0);

    let rules = [
        (
            "Rule 1 · FiniteGeometry",
            "Zero NaN / Inf coordinates",
            "PASS",
        ),
        (
            "Rule 2 · TextContainment",
            "Glyph ink inside clip & parent",
            "PASS",
        ),
        (
            "Rule 3 · HitTargetSize",
            "Interactive >= 24×24 logical px",
            "PASS",
        ),
        (
            "Rule 4 · SiblingDisjoint",
            "Zero sibling box overlap",
            "PASS",
        ),
        (
            "Rule 5 · ViewportContainment",
            "Portals & tooltips inside window",
            "PASS",
        ),
    ];

    let row_h = ((rect.size.y - 16.0) / rules.len() as f32).clamp(28.0, 38.0);
    for (i, (name, threshold, status)) in rules.iter().enumerate() {
        let row = Rect::new(
            rect.origin.x + 8.0,
            rect.origin.y + 8.0 + i as f32 * row_h,
            rect.size.x - 16.0,
            row_h - 4.0,
        );
        ctx.canvas
            .rect(row, 6.0, palette.card_bg, palette.card_border, 1.0, 0.0);
        ctx.canvas.text(
            name,
            Rect::new(row.origin.x + 12.0, row.origin.y, 210.0, row.size.y),
            10.5,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        if show_thresholds && row.size.x > 360.0 {
            ctx.canvas.text(
                threshold,
                Rect::new(
                    row.origin.x + 226.0,
                    row.origin.y,
                    row.size.x - 300.0,
                    row.size.y,
                ),
                9.5,
                palette.text_muted,
                AlignX::Start,
                AlignY::Center,
            );
        }
        widgets::badge(
            ctx.canvas,
            Rect::new(row.max().x - 62.0, row.center().y - 9.0, 50.0, 18.0),
            status,
            palette.success,
            palette.success_soft,
        );
    }
}
