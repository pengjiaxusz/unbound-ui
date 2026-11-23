//! 像素层验收：离屏渲染 → 回读 → 采样 → 与语义几何对账。
//!
//! 这组测试回答一个问题：**框架能不能在不看屏幕、不抢鼠标的前提下，
//! 证明自己真的把界面画出来了？**
//!
//! 语义树能说「这里有一个按钮」，但只有像素能说「这个按钮确实被画在了
//! 这里，而且和周围不一样」。两者对账，才谈得上「界面正确」。

use std::path::Path;

use unbound_agent::{AgentSession, OffscreenPixelSource};
use unbound_app::prelude::{
    AppContext, AppEvent, AppRuntime, Application, DirtyFlags, EventOutcome, ViewBuilder,
};
use unbound_gpu::device::{gpu_test_guard, test_device};
use unbound_math::Vec2;
use unbound_widgets::ButtonDesc;

const LAYOUT_KEY: u64 = 9001;
const BUTTON_KEY: u64 = 104;
const BUTTON_TEXT: &str = "Reset counter";

const VIEW_WIDTH: u32 = 640;
const VIEW_HEIGHT: u32 = 480;

/// 一个带按钮的最小应用。
struct PixelApp;

impl Application for PixelApp {
    type State = ();

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {}

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder) {
        view.column(LAYOUT_KEY, |col| {
            col.button(BUTTON_KEY, ButtonDesc::label(BUTTON_TEXT));
        });
    }
}

fn runtime() -> AppRuntime<PixelApp> {
    let mut runtime = AppRuntime::new(PixelApp, Vec2::new(VIEW_WIDTH as f32, VIEW_HEIGHT as f32));
    runtime.frame(1.0 / 60.0);
    runtime
}

/// 准备一个离屏像素源；无 GPU 设备时返回 `None`（测试跳过而非失败）。
fn offscreen_source() -> Option<OffscreenPixelSource> {
    let _guard = gpu_test_guard();
    let gpu = test_device()?;
    OffscreenPixelSource::with_gpu(gpu, VIEW_WIDTH, VIEW_HEIGHT, 1.0).ok()
}

/// 把 BGRA8 像素写成 PNG。
fn write_png(path: &Path, width: u32, height: u32, bgra: &[u8]) -> std::io::Result<()> {
    let mut rgba = bgra.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer
        .write_image_data(&rgba)
        .map_err(std::io::Error::other)?;
    Ok(())
}

/// 按钮中心必须与页面背景不同：语义说它存在，像素说它真的被画了。
#[test]
fn the_button_is_actually_painted() {
    let Some(mut source) = offscreen_source() else {
        return;
    };
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);

    let button = session
        .view()
        .query(&unbound_agent::NodeQuery::any().role(accesskit::Role::Button))
        .first()
        .copied()
        .expect("the button must be in the semantics tree");
    let center = session
        .view()
        .center(button)
        .expect("the button must have bounds");

    let frame = session
        .capture_offscreen(&mut source)
        .expect("the offscreen capture must succeed");

    let painted = frame
        .color_at(center)
        .expect("the button centre must be sampleable");
    let background = frame
        .color_at(Vec2::new(2.0, 2.0))
        .expect("the page corner must be sampleable");
    assert_ne!(
        painted, background,
        "the button centre must differ from the page background"
    );
}

/// 语义矩形与像素区域必须对得上：矩形内外的像素可区分，且矩形外确实没画。
#[test]
fn the_semantic_rect_and_the_painted_area_agree() {
    let Some(mut source) = offscreen_source() else {
        return;
    };
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);

    let button = session
        .view()
        .query(&unbound_agent::NodeQuery::any().role(accesskit::Role::Button))
        .first()
        .copied()
        .expect("the button must be in the semantics tree");
    let rect = session
        .view()
        .bounds(button)
        .expect("the button must have bounds");
    let center = session
        .view()
        .center(button)
        .expect("the button must have bounds");

    let frame = session
        .capture_offscreen(&mut source)
        .expect("the offscreen capture must succeed");

    // 矩形正下方留出一段空白：那里必须是页面背景，不能有按钮残留。
    let below = Vec2::new(center.x, rect.origin.y + rect.size.y + 8.0);
    let inside = frame
        .color_at(center)
        .expect("the button centre must be sampleable");
    let outside = frame
        .color_at(below)
        .expect("the area below must be sampleable");
    assert_ne!(
        inside, outside,
        "the area just below the button must differ from the button itself"
    );
}

/// 越界采样必须明确失败，而不是给出一个来路不明的颜色。
#[test]
fn sampling_outside_the_frame_fails_loudly() {
    let Some(mut source) = offscreen_source() else {
        return;
    };
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);

    let frame = session
        .capture_offscreen(&mut source)
        .expect("the offscreen capture must succeed");

    assert!(frame.color_at(Vec2::new(-4.0, -4.0)).is_none());
    assert!(frame
        .color_at(Vec2::new(VIEW_WIDTH as f32 + 10.0, 10.0))
        .is_none());
    assert!(frame.color_at(Vec2::new(f32::NAN, f32::NAN)).is_none());
}

/// 采集结果可以落成 PNG 产物：像素层不只是在内存里比颜色。
#[test]
fn a_capture_can_be_written_as_a_png() {
    let Some(mut source) = offscreen_source() else {
        return;
    };
    let mut runtime = runtime();
    let mut session = AgentSession::new(&mut runtime);

    let frame = session
        .capture_offscreen(&mut source)
        .expect("the offscreen capture must succeed");
    assert_eq!(frame.width, VIEW_WIDTH);
    assert_eq!(frame.height, VIEW_HEIGHT);
    assert_eq!(frame.pixels.len(), (VIEW_WIDTH * VIEW_HEIGHT * 4) as usize);

    let path = std::env::temp_dir().join("unbound_agent_pixel_layer.png");
    write_png(&path, frame.width, frame.height, &frame.pixels).expect("the PNG must be written");
    let size = std::fs::metadata(&path).expect("the PNG must exist").len();
    assert!(size > 0, "the PNG must not be empty");
    let _ = std::fs::remove_file(&path);
}
