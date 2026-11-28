//! L2 统一交互驱动与 L3 [`OffscreenTestHarness`] 端到端集成测试。

use unbound_app::testing::{HeadlessTestHarness, InteractionDriver};
use unbound_app::{AppContext, AppEvent, Application, DirtyFlags, EventOutcome, ViewBuilder};
use unbound_math::{Rect, Vec2};
use unbound_widgets::button::emit_button;
use unbound_widgets::{ButtonDesc, ButtonVariant};

const BTN_KEY: u64 = 101;
const DRAG_KEY: u64 = 102;

#[derive(Default)]
struct CounterToggleApp;

struct CounterToggleState {
    clicks: u32,
    drags: u32,
    hook_runs: u32,
    scroll_y: f32,
}

impl Application for CounterToggleApp {
    type State = CounterToggleState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        CounterToggleState {
            clicks: 0,
            drags: 0,
            hook_runs: 0,
            scroll_y: 0.0,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        let dirty = match event {
            AppEvent::Click { node, .. } if node == BTN_KEY => {
                state.clicks += 1;
                DirtyFlags::PAINT | DirtyFlags::A11Y
            }
            AppEvent::DragEnd { node, .. } if node == DRAG_KEY => {
                state.drags += 1;
                DirtyFlags::PAINT
            }
            AppEvent::Wheel { delta, .. } => {
                state.scroll_y += delta.y;
                DirtyFlags::PAINT
            }
            _ => DirtyFlags::NONE,
        };
        EventOutcome::new(dirty)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>) {
        let mut btn = ButtonDesc::new(Rect::new(40.0, 40.0, 180.0, 48.0), "Toggle");
        btn.variant = if state.clicks % 2 == 0 {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        };
        let control = view.ctx.control_state(BTN_KEY, false, false);
        let output = emit_button(BTN_KEY, &btn, control, &mut []);
        view.push(BTN_KEY, output);

        let drag_box = ButtonDesc::new(Rect::new(40.0, 120.0, 180.0, 48.0), "Drag");
        let drag_ctrl = view.ctx.control_state(DRAG_KEY, false, false);
        let drag_out = emit_button(DRAG_KEY, &drag_box, drag_ctrl, &mut []);
        view.push(DRAG_KEY, drag_out);
    }
}

#[test]
fn l2_headless_harness_unified_interaction_primitives_work() {
    let mut harness =
        HeadlessTestHarness::new(CounterToggleApp, 640, 480).with_on_before_render(|rt| {
            rt.state_mut().hook_runs += 1;
        });

    harness.step_frames(1, 1.0 / 60.0);
    assert_eq!(harness.runtime().state().hook_runs, 1);

    harness.click(Vec2::new(130.0, 64.0));
    harness.step_frames(1, 1.0 / 60.0);
    assert_eq!(harness.runtime().state().clicks, 1);

    harness.drag(Vec2::new(100.0, 144.0), Vec2::new(220.0, 144.0), 4);
    harness.step_frames(1, 1.0 / 60.0);
    assert_eq!(harness.runtime().state().drags, 1);

    harness.key_tap(
        unbound_app::AppKey::Enter,
        unbound_app::Modifiers::default(),
    );
    harness.wheel(Vec2::new(100.0, 100.0), Vec2::new(0.0, 24.0));
    assert_eq!(harness.runtime().state().scroll_y, 24.0);

    harness.resize(Vec2::new(800.0, 600.0));
    assert_eq!(
        harness.runtime().context().viewport,
        Vec2::new(800.0, 600.0)
    );
}

#[cfg(feature = "offscreen-test")]
mod l3_offscreen {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use unbound_app::testing::{OffscreenInitError, OffscreenTestHarness};
    use unbound_app::ExternalTextureHandle;
    use unbound_gpu::{
        CustomViewportPass, ViewportId, ViewportPassContext, ViewportRenderPassDesc,
    };

    #[derive(Default)]
    struct ExternalTextureApp {
        handle: Option<ExternalTextureHandle>,
    }

    struct ExternalTextureState {
        uploaded: bool,
    }

    impl Application for ExternalTextureApp {
        type State = ExternalTextureState;

        fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
            ExternalTextureState { uploaded: false }
        }

        fn reduce(
            &mut self,
            _state: &mut Self::State,
            _event: AppEvent,
            _ctx: &mut AppContext,
        ) -> EventOutcome {
            EventOutcome::new(DirtyFlags::NONE)
        }

        fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
            if let Some(handle) = self.handle {
                view.image_external(
                    201,
                    handle,
                    Vec2::new(128.0, 128.0),
                    unbound_widgets::ObjectFit::Fill,
                );
            }
        }
    }

    #[derive(Debug)]
    struct SolidClearViewportPass {
        id: ViewportId,
        calls: Arc<AtomicU32>,
    }

    impl CustomViewportPass for SolidClearViewportPass {
        fn viewport_id(&self) -> ViewportId {
            self.id
        }

        fn render_pass_desc(&self) -> ViewportRenderPassDesc {
            let mut desc = ViewportRenderPassDesc::new(self.id);
            desc.clear_color = [0.1, 0.75, 0.25, 1.0];
            desc
        }

        fn record_pass(&mut self, _ctx: &ViewportPassContext<'_>) {
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[derive(Default)]
    struct Viewport3dApp;

    impl Application for Viewport3dApp {
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

        fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
            let desc = unbound_widgets::Viewport3DDesc::new(ViewportId::new(1))
                .with_size(Vec2::new(160.0, 120.0));
            view.viewport_3d(301, desc);
        }
    }

    #[test]
    fn l3_offscreen_renders_click_diff_and_before_render_hook() {
        let mut harness = match OffscreenTestHarness::try_new(CounterToggleApp, 400, 300) {
            Ok(h) => h.with_on_before_render(|rt| {
                rt.state_mut().hook_runs += 1;
            }),
            Err(OffscreenInitError::NoVulkanDevice(reason)) => {
                eprintln!("skipping L3 offscreen test: {reason}");
                return;
            }
            Err(err) => panic!("unexpected L3 init error: {err}"),
        };

        let first = harness.render_frame(1.0 / 60.0).expect("first frame");
        assert_eq!(first.width, 400);
        assert_eq!(first.height, 300);
        assert_eq!(harness.runtime().state().hook_runs, 1);

        let btn_rect = Rect::new(40.0, 40.0, 180.0, 48.0);
        harness.assert_region_not_uniform(btn_rect, "initial_button");

        harness.click(Vec2::new(130.0, 64.0));
        let second = harness.render_frame(1.0 / 60.0).expect("second frame");
        assert_eq!(harness.runtime().state().clicks, 1);
        assert_eq!(harness.runtime().state().hook_runs, 2);
        harness.assert_region_changed(&first, btn_rect, 0.1, "button_variant_toggled");
        assert_eq!(second.width, 400);

        harness.resize_target(512, 384).expect("resize target");
        let resized = harness.render_frame(1.0 / 60.0).expect("resized frame");
        assert_eq!(resized.width, 512);
        assert_eq!(resized.height, 384);
    }

    #[test]
    fn l3_offscreen_uploads_external_texture_via_before_render_and_renders_3d_pass() {
        let mut tex_harness =
            match OffscreenTestHarness::try_new(ExternalTextureApp::default(), 256, 256) {
                Ok(h) => h.with_on_before_render(|rt| {
                    if !rt.state().uploaded {
                        let mut rgba = Vec::with_capacity(64 * 64 * 4);
                        for y in 0..64u32 {
                            for x in 0..64u32 {
                                let r = ((x * 4) & 0xff) as u8;
                                let g = ((y * 4) & 0xff) as u8;
                                rgba.extend_from_slice(&[r, g, 200, 255]);
                            }
                        }
                        let handle = rt.asset_hub_mut().upload_external_rgba8(64, 64, true, rgba);
                        rt.application_mut().handle = Some(handle);
                        rt.state_mut().uploaded = true;
                        rt.invalidate();
                    }
                }),
                Err(OffscreenInitError::NoVulkanDevice(reason)) => {
                    eprintln!("skipping L3 external texture test: {reason}");
                    return;
                }
                Err(err) => panic!("unexpected L3 init error: {err}"),
            };

        let _ = tex_harness
            .render_frame(1.0 / 60.0)
            .expect("render external tex");
        tex_harness.assert_region_not_uniform(
            Rect::new(0.0, 0.0, 128.0, 128.0),
            "external_texture_visible",
        );

        let calls = Arc::new(AtomicU32::new(0));
        let mut vp_harness = OffscreenTestHarness::try_new(Viewport3dApp, 320, 240)
            .expect("shared GPU already initialized")
            .with_viewport_pass(Box::new(SolidClearViewportPass {
                id: ViewportId::new(1),
                calls: Arc::clone(&calls),
            }))
            .expect("register custom 3d pass");
        vp_harness.step_frames(2, 1.0 / 60.0);
        assert!(
            vp_harness
                .last_report()
                .is_some_and(|r| r.active_viewports_3d >= 1),
            "3D viewport must be active in FrameReport"
        );
        assert!(
            calls.load(Ordering::Relaxed) >= 1,
            "CustomViewportPass::record_pass must be invoked"
        );
        let frame = vp_harness.last_frame().expect("3d frame");
        let px = frame.pixel(80, 60);
        assert!(
            px[1] > px[0] && px[1] > 100,
            "expected green 3D viewport clear pixel, got {px:?}"
        );
    }

    #[test]
    fn l3_offscreen_supports_parallel_threads_without_device_conflict() {
        let handles: Vec<_> = (0..3)
            .map(|_| {
                std::thread::spawn(|| {
                    let mut harness =
                        match OffscreenTestHarness::try_new(CounterToggleApp, 240, 180) {
                            Ok(h) => h,
                            Err(OffscreenInitError::NoVulkanDevice(_)) => return,
                            Err(err) => panic!("unexpected init error: {err}"),
                        };
                    harness.step_frames(2, 1.0 / 60.0);
                    harness.click(Vec2::new(100.0, 60.0));
                    let _ = harness.render_frame(1.0 / 60.0).expect("render");
                    assert_eq!(harness.runtime().state().clicks, 1);
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread join");
        }
    }
}
