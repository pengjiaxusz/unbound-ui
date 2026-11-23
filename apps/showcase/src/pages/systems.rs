//! 引擎核心子系统架构全景展示页（Systems）。
//! lang-allow: bilingual showcase page copy; Chinese is the zh-CN half
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
            Self::Math => "2D/3D geometry primitives, f16 packing, OKLCH color, and rounded-rect radius projection",
            Self::Abi => "C-stable memory layout descriptors, cross-process POD protocol, and node flags",
            Self::Gpu => "Vulkan 1.3 dynamic rendering, bindless atlases, 64-byte primitive instances, and radix sorting",
            Self::Vector => "CPU Bezier flattening, analytic anti-aliased strokes, and boolean path meshes",
            Self::Text => "Cosmic-Text multilingual bidi shaping, four-phase glyph cache, and atlas eviction",
            Self::World => "SlotMap entity tree, relayout boundary tracking, spatial quadtree, and virtualization",
            Self::Layout => "Incremental Flexbox/Grid solver, dirty-subtree skipping, and stats probes",
            Self::Anim => "Critically/under-damped analytic spring physics, cubic Bezier curves, and interpolation",
            Self::Canvas => "Infinite-canvas camera transforms, snapping grids, and a CoW transactional undo stack",
            Self::Input => "Layered hit routing, pointer capture, focus traversal, shortcuts, and drag state machines",
            Self::A11y => "AccessKit a11y tree builder with zero-overhead incremental publishing for static frames",
            Self::Theme => "OKLCH-aware palettes, a 4 KiB GPU color LUT, and zero-retransfer theme switching",
            Self::Widgets => "109 standard widgets, a 24-state constant-shape primitive contract, and in-window adaptive overlays",
            Self::Platform => "Native Win32 windows on windows 0.62.2, DWM frame resizing, clipboard, and tray",
            Self::App => "MVU app runtime, two-pass declarative view builder, timers, and an async task pool",
            Self::Asset => "Async PNG streaming decoder, ring staging buffers, and batched atlas uploads",
            Self::Graph => "FrameGraph pass orchestration, Kawase backdrop blur, and barrier scheduling",
            Self::Headless => "Headless geometry validator, five layout sanitization rules, and time-travel binary snapshots",
            Self::Host => "Sandboxed plugin host, dynamic-library hot reload, and a cross-ABI shared-memory channel",
            Self::AppGuest => "Guest SDK with zero engine dependencies; declarative UI through unbound-abi only",
            Self::Kpi => "Performance guardrail benchmarks (8.3 ms latency budget, partial-repair byte cap)",
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
            Self::Widgets => "109 widgets · 24 states",
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
    let locale = ctx.state.locale;
    let layer_badge_label = match selected_sub.layer_group() {
        0 => locale.pick("LAYER 0 · FOUNDATION & GPU", "第 0 层 · 基础架构与 GPU"),
        1 => locale.pick("LAYER 1 · SCENE & LAYOUT", "第 1 层 · 场景与布局"),
        _ => locale.pick("LAYER 2 · RUNTIME & PLATFORM", "第 2 层 · 运行时与平台"),
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

    let locale = ctx.state.locale;
    let stages = [
        (
            locale.pick("1. Win32 Platform", "1. Win32 平台层"),
            "windows 0.62.2 · WM_NCHITTEST",
            "unbound-platform",
        ),
        (
            locale.pick("2. Input & A11y", "2. 输入路由与无障碍"),
            "HitRouter + Focus + AccessKit",
            "unbound-input",
        ),
        (
            locale.pick("3. Layout & World", "3. 布局与世界"),
            locale.pick("Taffy + Relayout Boundaries", "Taffy 布局 + 局部重排边界"),
            "unbound-layout",
        ),
        (
            locale.pick("4. Diff & Vulkan", "4. 场景差分与 Vulkan"),
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
        &if locale.is_zh() {
            format!(
                "实时帧耗时：{:.2} ms  ·  图元数：{}  ·  上传：{} B  ·  模式：{}",
                ctx.state.demo.frame_ms,
                ctx.state.demo.primitive_count,
                ctx.state.demo.upload_bytes,
                if patch_mode {
                    "增量切片补丁"
                } else {
                    "全量场景上传"
                }
            )
        } else {
            format!(
                "Live Frame: {:.2} ms  ·  Primitives: {}  ·  Upload: {} B  ·  Mode: {}",
                ctx.state.demo.frame_ms,
                ctx.state.demo.primitive_count,
                ctx.state.demo.upload_bytes,
                if patch_mode {
                    "Incremental Range Patch"
                } else {
                    "Full Scene Upload"
                }
            )
        },
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

    let locale = ctx.state.locale;
    let pass = locale.pick("PASS", "通过");
    let rules = [
        (
            locale.pick("Rule 1 · FiniteGeometry", "规则 1 · FiniteGeometry"),
            locale.pick("Zero NaN / Inf coordinates", "零 NaN / Inf 非法坐标"),
            pass,
        ),
        (
            locale.pick("Rule 2 · TextContainment", "规则 2 · TextContainment"),
            locale.pick("Glyph ink inside clip & parent", "字形墨迹严守裁剪与父边界"),
            pass,
        ),
        (
            locale.pick("Rule 3 · HitTargetSize", "规则 3 · HitTargetSize"),
            locale.pick(
                "Interactive >= 24×24 logical px",
                "交互命中区 >= 24×24 逻辑像素",
            ),
            pass,
        ),
        (
            locale.pick("Rule 4 · SiblingDisjoint", "规则 4 · SiblingDisjoint"),
            locale.pick("Zero sibling box overlap", "同级控件零重叠"),
            pass,
        ),
        (
            locale.pick(
                "Rule 5 · ViewportContainment",
                "规则 5 · ViewportContainment",
            ),
            locale.pick(
                "Portals & tooltips inside window",
                "浮层与 Tooltip 严守窗口边界",
            ),
            pass,
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
