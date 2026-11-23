//! 动效、无障碍、遥测与设置。
//! lang-allow: pinyin/Shuangpin demo page; Chinese queries and IME labels are the subject

use accesskit::Role;
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;
use unbound_widgets::chart::{emit_chart, ChartDesc, ChartSeries, ChartState};
use unbound_widgets::curve_editor::{emit_curve_editor, CurveEditorDesc};
use unbound_widgets::WidgetCatalogId;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::state::{FrameSample, FRAME_HISTORY_CAP};
use crate::widgets::{self, ButtonVariant, Icon};

pub const SPRINGS_INFO: PageInfo = PageInfo {
    title: "Animation & springs",
    short: "Springs",
    description: "Physical springs with stiffness, damping and mass, plus cubic-bezier easing for one-shot transitions. Tune them live.",
    crates: &["unbound-anim"],
    keywords: "animation spring physics easing cubic bezier motion",
};

pub const A11Y_INFO: PageInfo = PageInfo {
    title: "Accessibility tree",
    short: "Accessibility",
    description: "Every control publishes a role, name, state and focus order. The panel renders the exact tree exported to the platform adapter.",
    crates: &["unbound-a11y"],
    keywords: "accessibility a11y focus ring tree roles keyboard screen reader",
};

pub const TELEMETRY_INFO: PageInfo = PageInfo {
    title: "GPU & KPI inspector",
    short: "Telemetry",
    description: "Frame timing, primitive counts, draw calls, uploads and pool watermarks. The same meters feed the HUD in the title bar.",
    crates: &["unbound-kpi", "unbound-gpu/src/timing.rs"],
    keywords: "telemetry kpi gpu frame time draw calls uploads tlsf memory hud",
};

pub const SETTINGS_INFO: PageInfo = PageInfo {
    title: "Settings & about",
    short: "Settings",
    description: "The gallery itself is the compatibility test: theme, zoom, snapping and telemetry preferences are all live.",
    crates: &["unbound-platform", "unbound-theme"],
    keywords: "settings about preferences version build theme zoom",
};

pub const SPRING_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Spring lab",
        "Toggle the target and watch the chip settle. Stiffness and damping are uniforms on the animator, not hard-coded durations.",
        &["unbound-anim"],
        r#"let mut spring = Spring::new(SpringPreset::Snappy, 0.0);
spring.stiffness = 260.0;
spring.damping = 24.0;
spring.set_target(1.0);"#,
        "A toggle and a replay button; the chip reports overshoot and velocity.",
        &[
            KnobSpec::slider("Stiffness", 80.0, 800.0, 10.0, 280.0, ""),
            KnobSpec::slider("Damping", 5.0, 60.0, 1.0, 24.0, ""),
            KnobSpec::toggle("Show velocity", true),
        ],
        DemoKind::Springs { easing: false },
    )
    .with_height(280.0),
    ExampleDef::new(
        "Cubic easing",
        "An interactive cubic-bezier editor: drag the two control points to reshape the curve. Presets load the engine's ease, ease-out and linear curves; springs are for direct manipulation.",
        &["unbound-widgets", "unbound-anim"],
        r#"let mut curve = CurveEditorState::new();
curve.begin(CurveHandle::P1);
curve.drag_to(desc.from_screen(pointer));
emit_curve_editor(node, &desc, &curve, control);"#,
        "A curve editor whose control points are draggable; its cubic-bezier value is announced.",
        &[
            KnobSpec::segmented("Curve", &["Ease", "Ease out", "Linear"], 1),
            KnobSpec::slider("Duration", 100.0, 1000.0, 25.0, 320.0, "ms"),
        ],
        DemoKind::Springs { easing: true },
    )
    .with_height(280.0),
];

pub const A11Y_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Semantic tree",
        "Tab through the controls on the left; the right panel mirrors the semantic tree with roles, names and focus state.",
        &["unbound-a11y"],
        r#"semantics.upsert(node, A11yNode::new(Role::Button, "Save").clickable());
semantics.set_focus(node);"#,
        "A focus-order demo whose exported tree is visible beside it.",
        &[
            KnobSpec::toggle("Focus visible", true),
            KnobSpec::segmented("Traversal", &["Visual", "DOM"], 0),
        ],
        DemoKind::A11y { focus_rings: false },
    )
    .with_height(300.0),
    ExampleDef::new(
        "Focus rings",
        "Ring thickness and contrast are theme tokens. The ring is painted as a stroked rounded rect and never shifts layout.",
        &["unbound-a11y", "unbound-theme"],
        r#"scene.stroke(focus_rect.inflate(2.0), 10.0, Token::FOCUS_RING, width);"#,
        "Three focusable surfaces with ring width and contrast knobs.",
        &[
            KnobSpec::slider("Ring width", 1.0, 4.0, 0.5, 2.0, "px"),
            KnobSpec::toggle("High contrast", false),
        ],
        DemoKind::A11y { focus_rings: true },
    )
    .with_height(240.0),
];

pub const TELEMETRY_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Runtime KPIs",
        "Live counters from the current frame: primitives, glyphs, draw calls, upload bytes and the frame budget.",
        &["unbound-kpi", "unbound-gpu/src/timing.rs"],
        r#"let stats = FrameStats { primitives, glyphs, draw_calls, upload_bytes };
kpi.record(FrameSample::new(stats, frame_ms));"#,
        "Five status cards announced as a live region while the gallery animates.",
        &[
            KnobSpec::toggle("GPU timing", true),
            KnobSpec::segmented("Window", &["1 s", "5 s", "10 s"], 1),
        ],
        DemoKind::Telemetry { graph: false },
    )
    .with_height(270.0),
    ExampleDef::new(
        "Frame graph",
        "A rolling frame-time chart with a 16.7 ms budget line. The series is the real frame buffer, plotted by the same chart emitter as the component gallery.",
        &["unbound-widgets", "unbound-kpi"],
        r#"let series = ChartSeries::line(history.points(), Token::PRIMARY);
let desc = ChartDesc::new(rect).with_series(vec![series]);
emit_chart(node, &desc, &ChartState::new(), &[], control);"#,
        "A chart described as an image with its current, minimum and p95 frame time.",
        &[
            KnobSpec::segmented("Metric", &["Frame", "Uploads", "Draw calls"], 0),
            KnobSpec::toggle("Budget line", true),
        ],
        DemoKind::Telemetry { graph: true },
    )
    .with_height(280.0),
];

pub const SETTINGS_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "About this build",
        "Unbound UI is a Rust workspace: GPU renderer, text stack, layout, input, world, canvas and a headless verifier. This gallery is its flagship application.",
        &["unbound-platform", "unbound-headless"],
        r#"Showcase::new()
    .theme(ThemeMode::Dark)
    .zoom(1.0)
    .pixel_snap(true)
    .run()"#,
        "A definition list of build facts plus the version heading.",
        &[
            KnobSpec::toggle("Telemetry opt-in", true),
            KnobSpec::segmented("Motion", &["Full", "Reduced"], 0),
        ],
        DemoKind::About,
    )
    .with_height(348.0),
    ExampleDef::new(
        "Pinyin & Shuangpin runtime config",
        "Full 20,992 CJK ideographs + polyphones with Full Pinyin, 6 Shuangpin schemes (Xiaohe enabled by default), and Full/Shuangpin initials. Switch schemes at runtime below.",
        &["unbound-text/src/pinyin.rs"],
        r#"set_global_shuangpin_scheme(ShuangpinScheme::Xiaohe);
let matcher = PinyinMatcher::new();
let hit = matcher.match_text("着色器管线", "voseqi");"#,
        "Interactive Shuangpin scheme selector and live CJK query matcher with character highlight ranges.",
        &[
            KnobSpec::segmented(
                "Scheme",
                &["Xiaohe", "Natural", "Microsoft", "Sogou", "Ziguang", "SmartABC"],
                0,
            ),
            KnobSpec::toggle("Shuangpin", true),
            KnobSpec::toggle("Initials", true),
        ],
        DemoKind::PinyinEngine,
    )
    .with_height(280.0),
];

/// 弹簧实验室或缓动曲线图。
pub fn springs(ctx: &mut DemoCtx<'_, '_>, easing: bool) {
    if easing {
        easing_page(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let show_velocity = ctx.knob_on(2);
    let spring = &ctx.state.demo.spring;
    let track = Rect::new(
        rect.origin.x + 24.0,
        rect.origin.y + 40.0,
        rect.size.x - 48.0,
        6.0,
    );
    ctx.canvas
        .rect(track, 3.0, palette.control_track, 0, 0.0, 0.0);
    ctx.canvas.rect(
        Rect::new(
            track.origin.x,
            track.origin.y,
            track.size.x * spring.value.clamp(0.0, 1.0),
            track.size.y,
        ),
        3.0,
        palette.primary,
        0,
        0.0,
        0.0,
    );
    let chip_travel = track.size.x - 44.0;
    let chip_x = track.origin.x + chip_travel * spring.value.clamp(-0.15, 1.2);
    let chip = Rect::new(chip_x + 2.0, track.origin.y - 19.0, 44.0, 44.0);
    if show_velocity {
        for index in 1..4 {
            let x = chip.origin.x - spring.velocity * 0.004 * index as f32;
            ctx.canvas.rect(
                Rect::new(x + 6.0, chip.origin.y + 8.0, 32.0, 28.0),
                8.0,
                palette.primary_soft,
                0,
                0.0,
                0.0,
            );
        }
    }
    ctx.canvas.rect(chip, 10.0, palette.primary, 0, 0.0, 12.0);
    ctx.canvas.text(
        if ctx.state.demo.spring_high { "B" } else { "A" },
        chip,
        14.0,
        palette.on_primary,
        AlignX::Center,
        AlignY::Center,
    );
    let toggle = Rect::new(rect.origin.x + 24.0, rect.max().y - 52.0, 150.0, 34.0);
    let replay = Rect::new(toggle.max().x + 10.0, toggle.origin.y, 108.0, 34.0);
    let locale = ctx.state.locale;
    widgets::button(
        ctx.canvas,
        ctx.part(1),
        toggle,
        if ctx.state.demo.spring_high {
            locale.pick("Target A", "切换至目标 A")
        } else {
            locale.pick("Target B", "切换至目标 B")
        },
        ButtonVariant::Secondary,
        true,
    );
    widgets::button(
        ctx.canvas,
        ctx.part(2),
        replay,
        locale.pick("Replay", "重播"),
        ButtonVariant::Subtle,
        true,
    );
    ctx.canvas.text(
        &if locale.is_zh() {
            format!(
                "位置 {:.3} · 速度 {:.2} · 过冲 {:.2}",
                spring.value,
                spring.velocity,
                spring.overshoot(0.0)
            )
        } else {
            format!(
                "value {:.3} · velocity {:.2} · overshoot {:.2}",
                spring.value,
                spring.velocity,
                spring.overshoot(0.0)
            )
        },
        Rect::new(
            replay.max().x + 12.0,
            replay.origin.y,
            rect.max().x - replay.max().x - 36.0,
            34.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
}

fn easing_page(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let plot = Rect::new(
        rect.origin.x + 24.0,
        rect.origin.y + 18.0,
        (rect.size.x - 48.0).min(rect.size.y * 1.6),
        rect.size.y - 52.0,
    );
    // 该曲线是真实的交互式编辑器：其两个控制点存储在演示状态中并由外壳拖拽，
    // 因此绘制出的贝塞尔曲线与报告的 cubic-bezier(...) 绝不会不一致。
    //
    let node = ctx.part(1);
    let control = ctx.canvas.control_state(node, false, false);
    let mut desc = CurveEditorDesc::new(plot);
    desc.label = ctx.state.locale.pick("Easing curve", "缓动曲线").to_owned();
    let output = emit_curve_editor(node, &desc, &ctx.state.demo.curve_editor, control);
    ctx.canvas
        .widget_output(WidgetCatalogId::CurveEditor, node, &output);
    ctx.canvas.text(
        &ctx.state.demo.curve_editor.css(),
        Rect::new(plot.origin.x, plot.origin.y + 8.0, plot.size.x, 18.0),
        11.0,
        palette.text,
        AlignX::Center,
        AlignY::Top,
    );
    ctx.canvas.text(
        &format!("{} ms", ctx.knob(1)),
        Rect::new(plot.origin.x, plot.max().y + 6.0, plot.size.x, 16.0),
        10.0,
        palette.text_muted,
        AlignX::Center,
        AlignY::Top,
    );
}

/// 语义树或焦点环实验室。
pub fn a11y(ctx: &mut DemoCtx<'_, '_>, focus_rings: bool) {
    if focus_rings {
        focus_ring_page(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let focus_visible = ctx.knob_on(0);
    let locale = ctx.state.locale;
    let labels = [
        locale.pick("Save", "保存"),
        locale.pick("Cancel", "取消"),
        locale.pick("Delete", "删除"),
    ];
    let left = Rect::new(
        rect.origin.x + 16.0,
        rect.origin.y + 16.0,
        rect.size.x * 0.48 - 24.0,
        rect.size.y - 32.0,
    );
    for (index, label) in labels.iter().enumerate() {
        let button = Rect::new(
            left.origin.x + 8.0,
            left.origin.y + 12.0 + index as f32 * 48.0,
            left.size.x - 16.0,
            36.0,
        );
        let selected = ctx.state.demo.a11y_focus == index;
        let hover = ctx.canvas.visual(ctx.part(10 + index as u16), false);
        ctx.canvas.rect(
            button,
            8.0,
            if selected {
                palette.primary_soft
            } else if hover.is_hovered() {
                palette.control_hover
            } else {
                palette.card_bg
            },
            palette.card_border,
            1.0,
            0.0,
        );
        ctx.canvas.text(
            label,
            button,
            12.0,
            if selected {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Center,
            AlignY::Center,
        );
        if focus_visible && selected {
            ctx.canvas
                .stroke(button.inflate(2.0), 10.0, palette.focus_ring, 2.0);
        }
        ctx.control(10 + index as u16, button, Role::Button, label, selected);
    }
    ctx.canvas.text(
        locale.pick(
            "Tab / Shift+Tab moves focus · Enter activates",
            "Tab / Shift+Tab 移动焦点 · Enter 触发激活",
        ),
        Rect::new(left.origin.x, left.max().y - 22.0, left.size.x, 16.0),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    let tree = Rect::new(
        rect.origin.x + rect.size.x * 0.52,
        rect.origin.y + 16.0,
        rect.size.x * 0.48 - 18.0,
        rect.size.y - 32.0,
    );
    ctx.canvas
        .rect(tree, 10.0, palette.stage_bg, palette.card_border, 1.0, 0.0);
    ctx.canvas.text(
        locale.pick("Semantics tree", "无障碍语义树"),
        Rect::new(
            tree.origin.x + 12.0,
            tree.origin.y + 8.0,
            tree.size.x - 24.0,
            16.0,
        ),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    let sem = ctx.canvas.semantics();
    let focus_id = sem.focus();
    let mut rows: Vec<(String, String, usize, bool)> = Vec::new();
    let root_id = sem.root();
    if let Some(root_node) = sem.get(root_id) {
        rows.push((
            format!("{:?}", root_node.role),
            root_node.label.clone(),
            0,
            focus_id == root_id,
        ));
    } else {
        rows.push((
            "Window".to_string(),
            format!("Unbound UI Gallery ({} nodes)", sem.len()),
            0,
            false,
        ));
    }
    rows.push((
        "Group".to_string(),
        format!("Demo card ({} live)", sem.len()),
        1,
        false,
    ));
    for index in 0..labels.len() {
        let id = ctx.part(10 + index as u16);
        if let Some(node) = sem.get(id) {
            let focused = focus_id == id || ctx.state.demo.a11y_focus == index;
            rows.push((format!("{:?}", node.role), node.label.clone(), 2, focused));
        }
    }
    for (index, (role, name, indent, focused)) in rows.iter().enumerate() {
        let y = tree.origin.y + 32.0 + index as f32 * 24.0;
        if *focused {
            ctx.canvas.rect(
                Rect::new(tree.origin.x + 6.0, y - 2.0, tree.size.x - 12.0, 22.0),
                5.0,
                palette.primary_soft,
                0,
                0.0,
                0.0,
            );
        }
        ctx.canvas.text(
            &format!("{}{}", "· ".repeat(*indent), role),
            Rect::new(tree.origin.x + 12.0, y, tree.size.x * 0.4, 18.0),
            9.5,
            if *focused {
                palette.primary
            } else {
                palette.text_muted
            },
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            name,
            Rect::new(
                tree.origin.x + tree.size.x * 0.4,
                y,
                tree.size.x * 0.56 - 12.0,
                18.0,
            ),
            10.0,
            if *focused {
                palette.text
            } else {
                palette.text_muted
            },
            AlignX::Start,
            AlignY::Center,
        );
    }
    widgets::badge(
        ctx.canvas,
        Rect::new(tree.origin.x + 12.0, tree.max().y - 30.0, 92.0, 20.0),
        "accesskit",
        palette.primary,
        palette.primary_soft,
    );
}

fn focus_ring_page(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let width = ctx.knob(0);
    let contrast = ctx.knob_on(1);
    let ring = if contrast {
        palette.text
    } else {
        palette.focus_ring
    };
    for index in 0..3 {
        let cell = Rect::new(
            rect.origin.x + 30.0 + index as f32 * (rect.size.x - 60.0) / 3.0,
            rect.origin.y + rect.size.y * 0.5 - 34.0,
            (rect.size.x - 96.0) / 3.0,
            68.0,
        );
        ctx.canvas.rect(
            cell,
            12.0,
            if index == 1 {
                palette.primary_soft
            } else {
                palette.card_bg
            },
            palette.card_border,
            1.0,
            0.0,
        );
        if index == 1 {
            ctx.canvas.stroke(cell.inflate(3.0), 15.0, ring, width);
        }
        widgets::icon(
            ctx.canvas,
            [Icon::Search, Icon::Grid, Icon::Chart][index],
            cell,
            palette.text,
            palette.card_bg,
        );
    }
    ctx.canvas.text(
        ctx.state.locale.pick(
            "focus ring · 2 px offset · never shifts layout",
            "焦点环 · 2 px 外扩偏移 · 绝不引起布局抖动",
        ),
        Rect::new(rect.origin.x, rect.max().y - 24.0, rect.size.x, 16.0),
        9.5,
        palette.text_muted,
        AlignX::Center,
        AlignY::Center,
    );
}

/// KPI 卡片或滚动帧图表。
pub fn telemetry(ctx: &mut DemoCtx<'_, '_>, graph: bool) {
    if graph {
        frame_graph(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let gpu = ctx.knob_on(0);
    let window = ["1 s", "5 s", "10 s"][ctx.knob(1).min(2.0) as usize];
    let stats = &ctx.state.demo;
    let locale = ctx.state.locale;
    let cards = [
        (
            locale.pick("Frame", "帧耗时"),
            format!("{:.2} ms", stats.frame_ms),
            locale.pick("16.7 ms budget", "16.7 ms 预算"),
        ),
        (
            locale.pick("FPS", "帧率 (FPS)"),
            format!("{:.0}", stats.fps),
            window,
        ),
        (
            locale.pick("Primitives", "图元数量"),
            format!("{}", stats.primitive_count),
            locale.pick("instances", "实例数"),
        ),
        (
            locale.pick("Draw calls", "绘制调用"),
            format!("{}", stats.draw_calls),
            locale.pick("indirect", "间接绘制"),
        ),
        (
            locale.pick("Upload", "上传字节"),
            format!("{} B", stats.upload_bytes),
            locale.pick("this frame", "本帧上传"),
        ),
        (
            locale.pick("Frame budget", "帧预算占比"),
            "23 %".to_string(),
            if gpu {
                locale.pick("GPU timing on", "已开启 GPU 计时")
            } else {
                locale.pick("CPU only", "仅 CPU 计时")
            },
        ),
    ];
    let columns = if rect.size.x > 520.0 { 3 } else { 2 };
    let gap = 10.0;
    let cell_w = (rect.size.x - gap * (columns - 1) as f32) / columns as f32;
    // 帧率上限栏占用舞台底部 34 px。
    let grid_height = (rect.size.y - 34.0).max(0.0);
    let cell_h = (grid_height - gap) * 0.5;
    for (index, (label, value, detail)) in cards.iter().enumerate() {
        let col = index % columns;
        let row = index / columns;
        let cell = Rect::new(
            rect.origin.x + col as f32 * (cell_w + gap),
            rect.origin.y + row as f32 * (cell_h + gap),
            cell_w,
            cell_h,
        );
        ctx.canvas
            .rect(cell, 9.0, palette.card_bg, palette.card_border, 1.0, 0.0);
        ctx.canvas.text(
            label,
            Rect::new(
                cell.origin.x + 12.0,
                cell.origin.y + 10.0,
                cell.size.x - 24.0,
                14.0,
            ),
            9.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.text(
            value,
            Rect::new(
                cell.origin.x + 12.0,
                cell.origin.y + 26.0,
                cell.size.x - 24.0,
                28.0,
            ),
            20.0,
            if index == 0 {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.text(
            detail,
            Rect::new(
                cell.origin.x + 12.0,
                cell.max().y - 22.0,
                cell.size.x - 24.0,
                14.0,
            ),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
    }
    ctx.canvas.scroll_zone(rect, 4);
    fps_limit_bar(ctx);
}

/// 帧率上限控件：五个预设加上一个自定义输入框。
///
/// `0` 表示无限制；其他值会被引擎钳制到 60 FPS 下限。
pub fn fps_limit_bar(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let state = ctx.state;
    let bar = Rect::new(rect.origin.x, rect.max().y - 30.0, rect.size.x, 26.0);
    let presets = [
        ("60", 60u32),
        ("120", 120),
        ("144", 144),
        ("240", 240),
        ("0", 0),
    ];
    let active_preset = state.fps_preset();
    let mut x = bar.origin.x;
    for (index, (label, _)) in presets.iter().enumerate() {
        let chip = Rect::new(x, bar.origin.y, 46.0, bar.size.y);
        let active = active_preset == Some(index as u8);
        let hover = matches!(
            ctx.canvas.visual(ctx.part(20 + index as u16), active),
            crate::canvas::Visual::Hover
        );
        ctx.canvas.rect(
            chip,
            7.0,
            if active {
                palette.primary
            } else if hover {
                palette.control_hover
            } else {
                palette.stage_bg
            },
            if active { 0 } else { palette.card_border },
            1.0,
            0.0,
        );
        ctx.canvas.text(
            if *label == "0" { "0 ∞" } else { label },
            chip,
            10.5,
            if active {
                palette.on_primary
            } else {
                palette.text
            },
            AlignX::Center,
            AlignY::Center,
        );
        ctx.control(
            20 + index as u16,
            chip,
            accesskit::Role::Button,
            if *label == "0" {
                state.locale.pick("Unlimited frame rate", "无限制帧率")
            } else {
                label
            },
            active,
        );
        x += 52.0;
    }
    let field = Rect::new(
        x + 4.0,
        bar.origin.y,
        (bar.max().x - x - 4.0).max(70.0),
        bar.size.y,
    );
    if field.size.x >= 70.0 {
        let focused = state.fps_input.focused;
        ctx.canvas.rect(
            field,
            7.0,
            palette.card_bg,
            if focused {
                palette.primary
            } else {
                palette.card_border
            },
            if focused { 2.0 } else { 1.0 },
            0.0,
        );
        let edit = &state.fps_input.edit;
        let buffer = edit.text();
        let text = if focused {
            buffer
        } else if state.fps_limit == 0 {
            state.locale.pick("uncapped", "无限制")
        } else {
            ""
        };
        let display = if text.is_empty() {
            state.locale.pick(
                "Custom FPS (min 60, 0 = ∞)",
                "自定义 FPS（最低 60，0 = 无限制）",
            )
        } else {
            text
        };
        let scroll = edit.scroll_x();
        let view_width = (field.size.x - 16.0).max(24.0);
        state.set_editor_view(crate::state::EditorId::Fps, view_width);
        let line = Rect::new(
            field.origin.x + 8.0,
            field.origin.y + 6.0,
            view_width,
            (field.size.y - 12.0).max(8.0),
        );
        if focused {
            if let Some((from, to)) = crate::editor::selection_span(ctx.canvas.labels, edit, 10.0) {
                ctx.canvas.rect(
                    Rect::new(
                        line.origin.x + from - scroll,
                        line.origin.y,
                        (to - from).max(1.0),
                        line.size.y,
                    ),
                    1.5,
                    palette.selection_fill,
                    0,
                    0.0,
                    0.0,
                );
            }
        }
        ctx.canvas.begin_clip(Rect::new(
            line.origin.x,
            field.origin.y,
            view_width,
            field.size.y,
        ));
        let text_width = ctx.canvas.labels.measure(display, 10.0).x.max(view_width);
        let text_rect_used = Rect::new(
            line.origin.x - scroll,
            field.origin.y,
            text_width,
            field.size.y,
        );
        let (quads, _) =
            ctx.canvas
                .layout_text(display, text_rect_used, 10.0, AlignX::Start, AlignY::Center);
        ctx.canvas.glyphs(
            &quads,
            if text.is_empty() {
                palette.text_muted
            } else {
                palette.text
            },
        );
        if let Some(underline) =
            crate::editor::preedit_underline(ctx.canvas.labels, edit, 10.0, text_rect_used)
        {
            ctx.canvas
                .rect(underline, 0.5, palette.primary, 0, 0.0, 0.0);
        }
        if focused {
            let caret = line.origin.x - scroll
                + crate::editor::caret_x(ctx.canvas.labels, buffer, edit.cursor(), 10.0);
            if state.fps_input.caret_visible() {
                ctx.canvas.rect(
                    Rect::new(caret, line.origin.y, 1.0, line.size.y),
                    0.5,
                    palette.primary,
                    0,
                    0.0,
                    0.0,
                );
            }
            state.set_ime_anchor(Vec2::new(caret, field.max().y));
        }
        ctx.canvas.end_clip();
        ctx.control(
            30,
            field,
            accesskit::Role::TextInput,
            state.locale.pick(
                "Custom FPS limit, minimum 60, 0 is unlimited",
                "自定义帧率上限，最低 60，0 表示无限制",
            ),
            focused,
        );
    }
}

fn frame_graph(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let metric = ctx.knob(0) as usize;
    let budget = ctx.knob_on(1);
    let plot = rect.inflate(-8.0);

    // 该序列是真实的滚动帧缓冲区，而非合成曲线。
    let value = |sample: &FrameSample| match metric {
        1 => sample.upload,
        2 => sample.draws,
        _ => sample.ms,
    };
    let points: Vec<Vec2> = ctx
        .state
        .demo
        .frame_history
        .iter()
        .enumerate()
        .map(|(index, sample)| Vec2::new(index as f32, value(sample)))
        .collect();
    // 帧耗时保持固定的两倍预算上限，使 16.7 ms 参考线位于图表中央；
    // 其他指标按各自观测到的峰值进行缩放。
    let y_max = match metric {
        0 => 33.4,
        _ => points.iter().map(|point| point.y).fold(1.0, f32::max) * 1.25,
    };
    let mut desc =
        ChartDesc::new(plot).with_series(vec![ChartSeries::line(points, Token::PRIMARY)]);
    desc.x_range = (0.0, (FRAME_HISTORY_CAP - 1) as f32);
    desc.y_range = (0.0, y_max);

    let node = ctx.part(1);
    let control = ctx.canvas.control_state(node, false, false);
    let output = emit_chart(node, &desc, &ChartState::new(), &[], control);
    ctx.canvas
        .widget_output(WidgetCatalogId::Chart, node, &output.output);

    // 预算参考线与指标标题是图表发射器未建模的标注；
    // 参考线使用图表自身的映射，因此绝不会与绘制数据不一致。
    //
    let inner = desc.plot_rect();
    if budget && metric == 0 {
        let y = desc.map_point(Vec2::new(0.0, 16.7)).y;
        ctx.canvas.rect(
            Rect::new(inner.origin.x, y, inner.size.x, 1.0),
            0.0,
            palette.warning,
            0,
            0.0,
            0.0,
        );
        ctx.canvas.text(
            "16.7 ms",
            Rect::new(inner.max().x - 56.0, y - 14.0, 56.0, 12.0),
            9.5,
            palette.warning,
            AlignX::End,
            AlignY::Top,
        );
    }
    let locale = ctx.state.locale;
    ctx.canvas.text(
        match metric {
            1 => locale.pick("upload bytes / frame", "每帧上传字节数"),
            2 => locale.pick("draw calls / frame", "每帧绘制调用数"),
            _ => locale.pick("frame time · rolling window", "帧耗时 · 滚动窗口"),
        },
        Rect::new(
            plot.origin.x + 12.0,
            plot.origin.y + 8.0,
            plot.size.x - 24.0,
            14.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    ctx.canvas.scroll_zone(rect, 4);
}

/// 设置 / 关于。
pub fn about(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    ctx.canvas.rect(
        Rect::new(rect.origin.x, rect.origin.y, 64.0, 64.0),
        16.0,
        palette.primary,
        0,
        0.0,
        10.0,
    );
    ctx.canvas.text(
        "U",
        Rect::new(rect.origin.x, rect.origin.y, 64.0, 64.0),
        30.0,
        palette.on_primary,
        AlignX::Center,
        AlignY::Center,
    );
    ctx.canvas.text(
        "Unbound UI",
        Rect::new(
            rect.origin.x + 80.0,
            rect.origin.y + 2.0,
            rect.size.x - 80.0,
            26.0,
        ),
        22.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    ctx.canvas.text(
        "0.1.0 · gallery build · win64",
        Rect::new(
            rect.origin.x + 80.0,
            rect.origin.y + 28.0,
            rect.size.x - 80.0,
            16.0,
        ),
        10.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    ctx.canvas
        .divider(rect.origin.x, rect.origin.y + 78.0, rect.size.x);
    let locale = ctx.state.locale;
    let facts = [
        (
            locale.pick("Renderer", "渲染器"),
            locale.pick(
                "Vulkan 1.3 · uber shader · 4 draw calls",
                "Vulkan 1.3 · 统一超级着色器 · 4 次绘制调用",
            ),
        ),
        (
            locale.pick("Text", "文本管线"),
            locale.pick(
                "MTSDF 512² atlas · CJK fallback",
                "MTSDF 512² 字形图集 · 中日韩字体回退",
            ),
        ),
        (
            locale.pick("Layout", "布局引擎"),
            locale.pick(
                "Taffy flex + grid · measure cache",
                "Taffy 弹性 + 网格布局 · 测量缓存",
            ),
        ),
        (
            locale.pick("World", "世界与虚拟化"),
            locale.pick(
                "DOD archetypes · quadtree virtualization",
                "DOD 数据原型 · 四叉树空间虚拟化",
            ),
        ),
        (
            locale.pick("Platform", "平台层"),
            locale.pick(
                "Windows 11 · custom title bar · DPI aware",
                "Windows 11 · 自定义标题栏 · DPI 感知",
            ),
        ),
        (
            locale.pick("Verifier", "无头校验器"),
            locale.pick(
                "unbound-headless · rules + golden screenshots",
                "unbound-headless · 几何规则 + 基准截图",
            ),
        ),
    ];
    for (index, (label, value)) in facts.iter().enumerate() {
        let y = rect.origin.y + 92.0 + index as f32 * 25.0;
        ctx.canvas.text(
            label,
            Rect::new(rect.origin.x, y, 110.0, 18.0),
            10.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            value,
            Rect::new(rect.origin.x + 116.0, y, rect.size.x - 116.0, 18.0),
            10.5,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
    }
    widgets::badge(
        ctx.canvas,
        Rect::new(rect.origin.x, rect.max().y - 62.0, 96.0, 20.0),
        "MIT OR Apache-2.0",
        palette.text_muted,
        palette.stage_bg,
    );
    fps_limit_bar(ctx);
}

/// 拼音与多方案双拼（默认小鹤双拼）运行时配置与实时编码检视台。
pub fn pinyin_engine(ctx: &mut DemoCtx<'_, '_>) {
    use unbound_text::{set_global_pinyin_config, PinyinConfig, PinyinMatcher, ShuangpinScheme};

    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let scheme_idx = (ctx.knob(0).round().max(0.0) as usize).min(ShuangpinScheme::ALL.len() - 1);
    let enable_sp = ctx.knob_on(1);
    let enable_init = ctx.knob_on(2);
    let scheme = ShuangpinScheme::from_index(scheme_idx);

    // 同步至全局原子拼音配置，使全局搜索框与数据表格实时响应切换
    let cfg = PinyinConfig::new()
        .with_scheme(scheme)
        .with_shuangpin(enable_sp)
        .with_full_initials(enable_init)
        .with_shuangpin_initials(enable_init);
    set_global_pinyin_config(cfg);
    let matcher = PinyinMatcher::with_config(cfg);

    let locale = ctx.state.locale;
    ctx.canvas.text(
        &if locale.is_zh() {
            format!(
                "当前生效双拼方案：{}  ·  20,992 常用汉字 + 6,592 扩展 A 区 + 多音字",
                scheme.label()
            )
        } else {
            format!(
                "Active Runtime Scheme: {}  ·  20,992 CJK + 6,592 Ext-A + Polyphones",
                scheme.label()
            )
        },
        Rect::new(
            rect.origin.x + 16.0,
            rect.origin.y + 10.0,
            rect.size.x - 32.0,
            20.0,
        ),
        12.5,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );

    let rule_summary = match scheme {
        ShuangpinScheme::Xiaohe => {
            "zh=v  ch=i  sh=u  |  zero initial: first letter + final key (ang=ah, eng=eg)  |  default scheme"
        }
        ShuangpinScheme::Natural => {
            "zh=v  ch=i  sh=u  |  zero initial: first letter + final key (ang=ah, eng=eg)  |  Ziranma"
        }
        ShuangpinScheme::Microsoft => {
            "zh=v  ch=i  sh=u  |  zero initial: o + final key (a=oa, an=oj)  |  üe=v"
        }
        ShuangpinScheme::Sogou => "zh=v  ch=i  sh=u  |  zero initial: o + final key (a=oa, an=oj)  |  üe=v",
        ShuangpinScheme::Ziguang => {
            "zh=u  ch=a  sh=i  |  zero initial: o + final key (a=oa, an=or)  |  Ziguang"
        }
        ShuangpinScheme::SmartAbc => {
            "zh=a  ch=e  sh=v  |  zero initial: o + final key (a=oa, ei=oq)  |  Intelligent ABC"
        }
    };
    ctx.canvas.text(
        rule_summary,
        Rect::new(
            rect.origin.x + 16.0,
            rect.origin.y + 32.0,
            rect.size.x - 32.0,
            18.0,
        ),
        10.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    let samples = [
        ("着色器", "zhuoseqi"),          // lang-ok: Pinyin & Shuangpin demo sample
        ("材质球", "caizhiqiu"),         // lang-ok: Pinyin & Shuangpin demo sample
        ("重命名", "chongmingming"),     // lang-ok: Pinyin & Shuangpin demo sample
        ("魑魅魍魉", "chimeiwangliang"), // lang-ok: Pinyin & Shuangpin demo sample
    ];

    for (i, (term, full_py)) in samples.iter().enumerate() {
        let y = rect.origin.y + 62.0 + i as f32 * 44.0;
        let row_r = Rect::new(rect.origin.x + 16.0, y, rect.size.x - 32.0, 38.0);
        ctx.canvas
            .rect(row_r, 6.0, palette.stage_bg, palette.border, 1.0, 0.0);

        let sp_code = scheme.encode_text(term);
        let init_char = scheme
            .initial_key(full_py)
            .unwrap_or_else(|| full_py.chars().next().unwrap_or('z'));
        let hit = matcher.match_text(term, &sp_code[..sp_code.len().min(6)]);
        let score = hit.as_ref().map_or(0, |m| m.score);

        ctx.canvas.text(
            term,
            Rect::new(row_r.origin.x + 12.0, y + 4.0, 160.0, 30.0),
            12.5,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            &if locale.is_zh() {
                format!("全拼：{full_py}   双拼：{sp_code}   双拼首字母：'{init_char}'")
            } else {
                format!("Full: {full_py}   Shuangpin: {sp_code}   Shuangpin initial: '{init_char}'")
            },
            Rect::new(row_r.origin.x + 176.0, y + 4.0, row_r.size.x - 276.0, 30.0),
            11.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        widgets::badge(
            ctx.canvas,
            Rect::new(row_r.max().x - 92.0, y + 9.0, 80.0, 20.0),
            &format!("{} {score}", locale.pick("score", "得分")),
            palette.primary,
            palette.primary_soft,
        );
    }
}
