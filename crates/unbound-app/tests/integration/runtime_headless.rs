//! 运行时必须在无窗口、无 GPU 的环境下驱动真实应用。
//!
//! 这是“单事件循环”重构的验收测试：计数器应用通过窗口外壳使用的同一个
//! [`AppRuntime`] 构建、点击与聚焦，并在场景层面断言报告结果。
//!

use unbound_app::{AppEvent, AppKey, AppRuntime, Application, DirtyFlags, EventOutcome, Modifiers};
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_widgets::button::emit_button;
use unbound_widgets::diff::SceneDiff;
use unbound_widgets::text_input::{TextEditState, TextInputDesc, TextInputVisual, TextInputWidget};
use unbound_widgets::{ButtonDesc, ButtonVariant, ControlState, WidgetDesc, WidgetOutput};

const BUTTON: u64 = 10;
const FIELD: u64 = 42;
const BACKGROUND: u64 = 9;
const TEXT_INPUT_SLOTS: usize = 64;

#[derive(Default)]
struct Counter;

struct CounterState {
    clicks: u32,
}

impl Application for Counter {
    type State = CounterState;

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {
        CounterState { clicks: 0 }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        let dirty = match event {
            AppEvent::Click { node, .. } if node == BUTTON => {
                state.clicks += 1;
                DirtyFlags::PAINT | DirtyFlags::A11Y
            }
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn view(&mut self, state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        let mut desc = ButtonDesc::new(Rect::new(40.0, 40.0, 200.0, 44.0), "Count");
        desc.variant = if state.clicks % 2 == 0 {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        };
        let control = view.ctx.control_state(BUTTON, false, false);
        let output: WidgetOutput = emit_button(BUTTON, &desc, control, &mut []);
        view.push(BUTTON, output);
        if state.clicks == 0 {
            if let Some(request) = unbound_widgets::BackdropRequest::new(
                BUTTON,
                Rect::new(20.0, 20.0, 240.0, 80.0),
                12.0,
            ) {
                view.request_backdrop(request);
            }
        }
    }
}

fn runtime() -> AppRuntime<Counter> {
    AppRuntime::new(Counter, Vec2::new(800.0, 600.0))
}

/// 包含 5,000 个静态背景图元与单个预留 span 的文本字段，
/// 使得打字不会移动背景图元。
#[derive(Default)]
struct TypingScene;

struct TypingState {
    text: String,
}

impl Application for TypingScene {
    type State = TypingState;

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {
        TypingState {
            text: String::new(),
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        let dirty = match event {
            AppEvent::Text { text } => {
                state.text.push_str(&text);
                DirtyFlags::PAINT | DirtyFlags::A11Y
            }
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn view(&mut self, state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        let background = WidgetOutput {
            primitives: (0..5_000)
                .map(|index| GpuPrimitiveInstance {
                    local_pos: [(index % 800) as f32, (index / 800) as f32],
                    size: [1.0, 1.0],
                    ..GpuPrimitiveInstance::default()
                })
                .collect(),
            ..WidgetOutput::default()
        };
        view.push(BACKGROUND, background);

        let mut glyphs: Vec<GlyphQuad> = state
            .text
            .chars()
            .enumerate()
            .map(|(index, _)| GlyphQuad {
                rect: Rect::new(20.0 + index as f32 * 8.0, 20.0, 7.0, 16.0),
                uv: [0.0; 4],
                layer: 0,
                color: false,
                msdf: false,
                phase: 0,
            })
            .collect();
        let widget = TextInputWidget {
            node: FIELD,
            desc: TextInputDesc::new(Rect::new(10.0, 10.0, 400.0, 32.0)),
            edit: TextEditState::with_text(state.text.clone()),
            visual: TextInputVisual::default(),
        };
        let output = widget.emit(ControlState::default(), &mut glyphs);
        view.push_with_capacity(FIELD, output, TEXT_INPUT_SLOTS);
    }
}

/// 唯一交互节点刻意小于 24px 下限的视图。
#[derive(Default)]
struct TinyTarget;

impl Application for TinyTarget {
    type State = ();

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {}

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, _state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        view.container(
            7,
            unbound_abi::RawNodeDesc::container()
                .size(6.0, 6.0)
                .flags(unbound_abi::NodeFlags::INTERACTIVE),
            |_| {},
        );
    }
}

#[test]
fn inspector_overlay_renders_on_debug_layer_and_reports_hovered_node_box_model() {
    let mut runtime = AppRuntime::new(TinyTarget, Vec2::new(800.0, 600.0));
    let plain = runtime.frame(0.0).primitives;

    assert!(runtime.key(AppKey::F(12), true, Modifiers::default()) != DirtyFlags::NONE);
    assert!(runtime.inspector().enabled());
    runtime.enable_debug_sanitizers(true);

    let report = runtime.frame(1.0 / 60.0);
    assert!(
        report.primitives > plain,
        "the inspector must append overlay primitives"
    );
    let violations = runtime.inspector().violations().len();
    assert!(
        violations > 0,
        "a 6×6 interactive target must fail the touch-target rule"
    );

    // 检查器按帧运行：下一帧会再次报告相同的违规项。
    runtime.frame(1.0 / 60.0);
    assert_eq!(runtime.inspector().violations().len(), violations);

    assert!(runtime.key(AppKey::F(12), true, Modifiers::default()) != DirtyFlags::NONE);
    assert!(!runtime.inspector().enabled());
}

#[test]
fn the_first_frame_is_a_whole_scene() {
    let mut runtime = runtime();
    let report = runtime.frame(0.0);
    assert_eq!(report.diff, SceneDiff::Whole);
    assert!(report.primitives > 0);
    assert!(report.bytes > 0);
    assert!(!runtime.scene().semantics.is_empty());
}

#[test]
fn hovering_repaints_only_the_button_span() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    let (start, end) = runtime.scene().node_spans[&BUTTON];

    runtime.pointer_move(Vec2::new(100.0, 60.0));
    let report = runtime.frame(1.0 / 60.0);
    let SceneDiff::Ranges(ranges) = &report.diff else {
        panic!("hover should produce ranges, got {:?}", report.diff);
    };
    assert!(!ranges.is_empty());
    for (from, to) in ranges {
        assert!(
            *from >= start && *to <= end,
            "{from}..{to} outside {start}..{end}"
        );
    }
    assert!(report.bytes < u32::MAX);
}

#[test]
fn a_click_reaches_reduce_and_rebuilds_the_button() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    runtime.pointer_move(Vec2::new(100.0, 60.0));
    runtime.pointer_down(Vec2::new(100.0, 60.0), PointerButton::Left);
    runtime.pointer_up(Vec2::new(100.0, 60.0), PointerButton::Left);
    assert_eq!(runtime.state().clicks, 1);

    let report = runtime.frame(1.0 / 60.0);
    assert!(matches!(
        report.diff,
        SceneDiff::Ranges(_) | SceneDiff::Whole
    ));
}

#[test]
fn clicking_outside_does_nothing() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    runtime.pointer_move(Vec2::new(700.0, 500.0));
    runtime.pointer_down(Vec2::new(700.0, 500.0), PointerButton::Left);
    runtime.pointer_up(Vec2::new(700.0, 500.0), PointerButton::Left);
    assert_eq!(runtime.state().clicks, 0);
}

#[test]
fn a_frame_with_no_dirty_flags_does_no_work() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    let report = runtime.frame(1.0 / 60.0);
    assert_eq!(report.diff, SceneDiff::Identical);
    assert_eq!(report.bytes, 0);
}

#[test]
fn tab_focuses_the_button_and_the_next_frame_repaints_the_ring() {
    let mut runtime = runtime();
    runtime.frame(0.0);

    let flags = runtime.key(AppKey::Tab, true, Modifiers::default());
    assert!(flags.merge(DirtyFlags::NONE) != DirtyFlags::NONE);
    assert_eq!(runtime.focus().focused(), Some(BUTTON));
    assert!(runtime.focus().focus_visible());

    let report = runtime.frame(1.0 / 60.0);
    assert!(matches!(
        report.diff,
        SceneDiff::Ranges(_) | SceneDiff::Whole
    ));
}

#[test]
fn backdrop_requests_reach_the_frame_report() {
    let mut runtime = runtime();
    let report = runtime.frame(0.0);
    assert_eq!(report.backdrops.len(), 1);
    assert_eq!(report.backdrops[0].blur, 12.0);

    runtime.pointer_move(Vec2::new(100.0, 60.0));
    runtime.pointer_down(Vec2::new(100.0, 60.0), PointerButton::Left);
    runtime.pointer_up(Vec2::new(100.0, 60.0), PointerButton::Left);
    let report = runtime.frame(1.0 / 60.0);
    assert!(
        report.backdrops.is_empty(),
        "the dialog closed, so the blur pass must not run"
    );
}

#[test]
fn resizing_forces_a_whole_scene() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    runtime.resize(Vec2::new(1024.0, 768.0));
    assert_eq!(runtime.context().viewport, Vec2::new(1024.0, 768.0));
    let report = runtime.frame(1.0 / 60.0);
    assert_eq!(report.diff, SceneDiff::Whole);
}

#[test]
fn typing_in_text_input_uploads_only_local_range() {
    let mut runtime = AppRuntime::new(TypingScene, Vec2::new(800.0, 600.0));
    let first = runtime.frame(0.0);
    assert_eq!(first.diff, SceneDiff::Whole);

    for index in 0..10 {
        runtime.text(if index % 2 == 0 { "a" } else { "b" });
        let report = runtime.frame(1.0 / 60.0);
        assert!(
            matches!(report.diff, SceneDiff::Ranges(_)),
            "frame {index} re-uploaded the whole scene: {:?}",
            report.diff
        );
        assert!(
            report.bytes <= 512,
            "frame {index} uploaded {} B, expected <= 512 B",
            report.bytes
        );
        assert!(
            report.aux_changed,
            "the glyph payload table must ride the auxiliary upload"
        );
    }
}

#[test]
fn inspector_emits_debug_glyph_primitives_when_enabled() {
    let mut runtime = runtime();
    runtime.frame(0.0);
    let base_prims = runtime.scene().primitives.len();
    let base_payloads = runtime.scene().payloads.len();

    runtime.toggle_inspector();
    runtime.frame(1.0 / 60.0);
    assert!(
        runtime.scene().primitives.len() > base_prims,
        "enabled inspector must emit panel and glyph primitives into scene"
    );
    assert!(
        runtime.scene().payloads.len() > base_payloads,
        "enabled inspector must shape debug text lines into glyph payloads"
    );
}

#[derive(Default)]
struct PlatformBridgeApp;

struct PlatformBridgeState {
    dropped: Vec<std::path::PathBuf>,
    hovered: Vec<std::path::PathBuf>,
    hover_cancelled: bool,
    pasted: Option<String>,
}

impl Application for PlatformBridgeApp {
    type State = PlatformBridgeState;

    fn init(&mut self, ctx: &mut unbound_app::AppContext) -> Self::State {
        ctx.clipboard_set("hello clipboard");
        PlatformBridgeState {
            dropped: Vec::new(),
            hovered: Vec::new(),
            hover_cancelled: false,
            pasted: ctx.clipboard_get(),
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::FileHover(paths) => {
                state.hovered = paths;
            }
            AppEvent::FileDrop(paths) => {
                state.dropped = paths;
            }
            AppEvent::FileHoverCancelled
            | AppEvent::FloatingWindowCreated { .. }
            | AppEvent::FloatingWindowClosed { .. } => {
                state.hover_cancelled = true;
            }
            _ => {}
        }
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, _state: &Self::State, _view: &mut unbound_app::ViewBuilder) {}
}

#[test]
fn clipboard_and_file_drop_events_flow_through_runtime() {
    let mut runtime = AppRuntime::new(PlatformBridgeApp, Vec2::new(800.0, 600.0));
    runtime.set_clipboard(Box::new(unbound_app::MemoryClipboard::default()));
    runtime.context().clipboard_set("from runtime");
    assert_eq!(
        runtime.context().clipboard_get().as_deref(),
        Some("from runtime")
    );
    assert_eq!(runtime.state().pasted.as_deref(), Some("hello clipboard"));

    let p = std::path::PathBuf::from("sample.txt");
    runtime.file_hover(vec![p.clone()]);
    assert_eq!(runtime.state().hovered, vec![p.clone()]);
    runtime.file_drop(vec![p.clone()]);
    assert_eq!(runtime.state().dropped, vec![p]);
    runtime.file_hover_cancelled();
    assert!(runtime.state().hover_cancelled);
}

/// 用于验证 `TreeView` 展开箭头悬停保活与滚动条按需显隐的无头测试应用。
#[derive(Default)]
struct TreeInteractionApp;

struct TreeInteractionState {
    tree: std::cell::RefCell<unbound_widgets::TreeViewState>,
}

const TREE_KEY: u64 = 9001;

impl Application for TreeInteractionApp {
    type State = TreeInteractionState;

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {
        let mut tree = unbound_widgets::TreeViewState::new_with_density(
            unbound_widgets::TreeViewDensity::Compact,
        );
        let roots: Vec<unbound_world::virtualize::TreeRow> = (1..=10)
            .map(|id| unbound_world::virtualize::TreeRow::new(id, 0, id == 1))
            .collect();
        tree.rebuild(&roots, |_| Vec::new());
        TreeInteractionState {
            tree: std::cell::RefCell::new(tree),
        }
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        let desc = unbound_widgets::TreeViewDesc::new(Rect::new(20.0, 20.0, 240.0, 120.0))
            .with_density(unbound_widgets::TreeViewDensity::Compact)
            .with_navigation_pane(true);
        let mut tree = state.tree.borrow_mut();
        view.tree_view(TREE_KEY, desc, &mut tree);
    }
}

#[test]
fn tree_view_keeps_row_highlighted_on_toggle_hover_and_hides_scrollbar_when_idle() {
    let mut rt = AppRuntime::new(TreeInteractionApp, Vec2::new(800.0, 600.0));
    rt.frame(0.0);

    // 1. 内容溢出但指针未悬停且未滚动时，按 ADR-0031 渲染克制的 2px 半透明细指示条（轨道胶囊透明）
    let sb_key = TREE_KEY ^ 0x5452_4545_0003;
    let (sb_start, sb_end) = rt.scene().node_spans[&sb_key];
    let sb_prims = &rt.scene().primitives[sb_start as usize..sb_end as usize];
    assert_eq!(
        sb_prims.len(),
        2,
        "overflowing tree must emit stable two-slot scrollbar primitives"
    );
    assert_eq!(
        sb_prims[0].opacity(),
        0.0,
        "idle track capsule must remain transparent"
    );
    assert_eq!(
        sb_prims[1].size[0],
        unbound_widgets::SCROLLBAR_THUMB_IDLE_WIDTH,
        "idle overflowing tree scrollbar must render 2px thin indicator"
    );
    assert!(
        (sb_prims[1].opacity() - unbound_widgets::SCROLLBAR_IDLE_OPACITY).abs() < 0.02,
        "idle overflowing tree scrollbar must use restrained idle opacity"
    );

    // 2. 将指针移至第 1 行的展开箭头热区 `tree_toggle_node(1)`，验证整行仍保持 NAV_HOVER 高亮且滚动条显露
    let toggle_key = unbound_widgets::tree_toggle_node(1);
    let toggle_region = rt
        .router()
        .regions()
        .iter()
        .find(|h| h.node == toggle_key)
        .expect("row 1 toggle hit region")
        .bounds;
    rt.pointer_move(toggle_region.center());
    rt.frame(1.0 / 60.0);

    let (row_start, row_end) = rt.scene().node_spans[&1];
    assert_eq!(
        rt.scene().primitives[row_start as usize].fill_token_or_uv,
        u32::from(unbound_theme::Token::NAV_HOVER.raw()),
        "hovering tree_toggle_node must keep the parent tree row highlighted"
    );
    assert!(row_end > row_start);

    let (sb_start, sb_end) = rt.scene().node_spans[&sb_key];
    assert!(
        rt.scene().primitives[sb_start as usize..sb_end as usize]
            .iter()
            .any(|p| p.opacity() > 0.5),
        "hovering the tree viewport must reveal the scrollbar"
    );
}

/// 用于验证 `AddressBar` 在无头运行时自动追踪子热区悬停/按压态的测试应用。
#[derive(Default)]
struct AddressBarAutoTrackApp;

struct AddressBarAutoTrackState {
    bar: unbound_widgets::AddressBarState,
}

const ADDR_BAR_KEY: u64 = 9002;

impl Application for AddressBarAutoTrackApp {
    type State = AddressBarAutoTrackState;

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {
        AddressBarAutoTrackState {
            bar: unbound_widgets::AddressBarState::new(),
        }
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        let desc = unbound_widgets::AddressBarDesc::with_path_segments(
            Rect::new(20.0, 20.0, 720.0, unbound_widgets::ADDRESS_BAR_HEIGHT),
            "C:/Workspace/Assets",
            vec![
                unbound_widgets::PathSegment::new("C:", "C:/"),
                unbound_widgets::PathSegment::new("Workspace", "C:/Workspace"),
                unbound_widgets::PathSegment::new("Assets", "C:/Workspace/Assets")
                    .with_subfolders(false),
            ],
        )
        .with_nav_state(true, false, true)
        .with_chrome_style(unbound_widgets::AddressBarChromeStyle::Outlined)
        .with_search("Search");
        view.address_bar(ADDR_BAR_KEY, desc, &state.bar);
    }
}

#[test]
fn address_bar_automatically_tracks_sub_hit_hover_press_and_outer_border() {
    let mut rt = AppRuntime::new(AddressBarAutoTrackApp, Vec2::new(800.0, 600.0));
    rt.frame(0.0);

    let root_chevron_key = unbound_widgets::address_bar_sub_node(
        ADDR_BAR_KEY,
        unbound_widgets::AddressBarHitKind::RootChevron,
    );
    let rc_bounds = rt
        .router()
        .regions()
        .iter()
        .find(|h| h.node == root_chevron_key)
        .expect("RootChevron sub hit region")
        .bounds;

    // 悬停 RootChevron：业务层未手动写 hovered_hit，但运行时应自动高亮 RootChevron 且地址框外边框保持 OUTLINE_STRONG
    rt.pointer_move(rc_bounds.center());
    rt.frame(1.0 / 60.0);

    let (start, end) = rt.scene().node_spans[&ADDR_BAR_KEY];
    let slice = &rt.scene().primitives[start as usize..end as usize];
    let rc_hover = slice
        .iter()
        .find(|p| {
            (p.local_pos[0] - rc_bounds.origin.x).abs() < 0.5
                && (p.local_pos[1] - rc_bounds.origin.y).abs() < 0.5
                && (p.size[0] - rc_bounds.size.x).abs() < 0.5
        })
        .expect("RootChevron surface primitive");
    assert_eq!(
        rc_hover.fill_token_or_uv,
        u32::from(unbound_theme::Token::HOVER.raw())
    );
    assert!(
        slice
            .iter()
            .any(|p| p.border_token_or_tex == u32::from(unbound_theme::Token::OUTLINE_STRONG.raw())),
        "address box outer border must remain OUTLINE_STRONG while hovering RootChevron"
    );

    // 按下 RootChevron：运行时自动派生 pressed_hit 并切换为 Token::PRESSED
    rt.pointer_down(rc_bounds.center(), PointerButton::Left);
    rt.frame(1.0 / 60.0);

    let (start, end) = rt.scene().node_spans[&ADDR_BAR_KEY];
    let rc_pressed = rt.scene().primitives[start as usize..end as usize]
        .iter()
        .find(|p| {
            (p.local_pos[0] - rc_bounds.origin.x).abs() < 0.5
                && (p.local_pos[1] - rc_bounds.origin.y).abs() < 0.5
                && (p.size[0] - rc_bounds.size.x).abs() < 0.5
        })
        .expect("RootChevron pressed surface primitive");
    assert_eq!(
        rc_pressed.fill_token_or_uv,
        u32::from(unbound_theme::Token::PRESSED.raw())
    );
}
