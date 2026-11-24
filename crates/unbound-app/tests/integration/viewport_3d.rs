//! 3D 视口运行时集成测试（无头模式）。
//!
//! 覆盖 Issue #245 / #256 / #258 / #259 的运行时契约：
//! 占位渲染与外部纹理软件降级、GPU Bindless 槽位切换、多实例并发布局、
//! 零尺寸退化安全跳过、多按键拖拽与滚轮手势路由、高 Z 轴 HUD 按钮命中优先级、
//! `set_viewport_render_scale` 零重排轻量同步，以及分屏动画调整与设备恢复槽位失效。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use unbound_abi::{Dim, RawNodeDesc};
use unbound_app::{
    AppContext, AppEvent, AppKey, AppRuntime, Application, DirtyFlags, EventOutcome,
    ExternalTextureHandle, Modifiers, ViewBuilder,
};
use unbound_gpu::data::PrimType;
use unbound_gpu::ViewportId;
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_widgets::button::emit_button;
use unbound_widgets::diff::SceneDiff;
use unbound_widgets::{ButtonDesc, Viewport3DDesc, Viewport3DGesture, Viewport3DGestureEvent};

const VP_KEY_1: u64 = 1001;
const VP_KEY_2: u64 = 1002;
const HUD_BUTTON_KEY: u64 = 2001;

fn prim_type_of(prim: &unbound_gpu::data::GpuPrimitiveInstance) -> PrimType {
    PrimType::from_u8((prim.prim_type_and_flags & 0xFF) as u8)
}

/// 单 3D 视口测试应用，支持动态尺寸、降级外部纹理与视图重建计数。
struct SingleViewportApp {
    viewport_id: ViewportId,
    size: Vec2,
    render_scale: f32,
    fallback_external: Option<ExternalTextureHandle>,
    interactive: bool,
    view_calls: Arc<AtomicUsize>,
}

struct SingleViewportState {
    size: Vec2,
    fallback_external: Option<ExternalTextureHandle>,
}

impl Application for SingleViewportApp {
    type State = SingleViewportState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        SingleViewportState {
            size: self.size,
            fallback_external: self.fallback_external,
        }
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        self.view_calls.fetch_add(1, Ordering::Relaxed);
        let mut desc = Viewport3DDesc::new(self.viewport_id)
            .with_size(state.size)
            .with_render_scale(self.render_scale)
            .with_interactive(self.interactive)
            .with_alt("Primary 3D Viewport");
        if let Some(handle) = state.fallback_external {
            desc = desc.with_fallback_external_texture(Some(handle.0));
        }
        view.viewport_3d(VP_KEY_1, desc);
    }
}

/// 1. 占位渲染 -> 外部纹理软件降级 -> GPU Bindless 槽位提交与通道派发计数回环。
#[test]
fn placeholder_fallback_external_texture_and_gpu_slot_transition() {
    let vp_id = ViewportId::new(1);
    let view_calls = Arc::new(AtomicUsize::new(0));
    let app = SingleViewportApp {
        viewport_id: vp_id,
        size: Vec2::new(320.0, 200.0),
        render_scale: 1.0,
        fallback_external: None,
        interactive: true,
        view_calls,
    };
    let mut rt = AppRuntime::new(app, Vec2::new(800.0, 600.0));

    // 首帧未绑定任何纹理：应发射占位图元（无 `PrimType::Image`）。
    let r1 = rt.frame(0.0);
    assert_eq!(r1.active_viewports_3d, 1);
    assert_eq!(r1.viewports_3d.len(), 1);
    assert_eq!(r1.viewports_3d[0].viewport_id, vp_id);
    assert_eq!(r1.viewports_3d[0].physical_size, Some((320, 200)));
    assert_eq!(r1.viewport_resize_count, 1);
    assert_eq!(r1.viewport_pass_dispatches, 0);
    let image_prims_1: Vec<_> = rt
        .scene()
        .primitives
        .iter()
        .filter(|p| prim_type_of(p) == PrimType::Image)
        .collect();
    assert!(image_prims_1.is_empty());

    // 上传外部 RGBA8 纹理并绑定为软件降级目标，随后提交外部槽位 52。
    let ext_handle = rt
        .asset_hub_mut()
        .upload_external_rgba8(2, 2, true, vec![255u8; 16]);
    rt.state_mut().fallback_external = Some(ext_handle);
    rt.asset_hub_mut()
        .commit_external_texture_slot(ext_handle, Ok(52));
    rt.invalidate();

    let r2 = rt.frame(0.0);
    assert_eq!(r2.active_viewports_3d, 1);
    assert_eq!(
        r2.viewports_3d[0].fallback_external_texture,
        Some(ext_handle)
    );
    let image_prims_2: Vec<_> = rt
        .scene()
        .primitives
        .iter()
        .filter(|p| prim_type_of(p) == PrimType::Image)
        .collect();
    assert_eq!(image_prims_2.len(), 1);
    assert_eq!(image_prims_2[0].border_token_or_tex, 52);

    // GPU 离屏视口槽位就绪（槽位 60）后应优先覆盖软件降级纹理。
    rt.commit_viewport_3d_slot(vp_id, Some(60));
    rt.record_viewport_pass_dispatches(2);
    let r3 = rt.frame(0.0);
    assert_eq!(r3.viewport_pass_dispatches, 2);
    let image_prims_3: Vec<_> = rt
        .scene()
        .primitives
        .iter()
        .filter(|p| prim_type_of(p) == PrimType::Image)
        .collect();
    assert_eq!(image_prims_3.len(), 1);
    assert_eq!(image_prims_3[0].border_token_or_tex, 60);

    // 下一帧通道派发计数应清零，不重复累计。
    let r4 = rt.frame(0.0);
    assert_eq!(r4.viewport_pass_dispatches, 0);
}

/// 双 3D 视口并发布局测试应用。
struct DualViewportApp;

impl Application for DualViewportApp {
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
        view.row(9001, |row| {
            row.viewport_3d(
                VP_KEY_1,
                Viewport3DDesc::new(ViewportId::new(1))
                    .with_size(Vec2::new(400.0, 300.0))
                    .with_render_scale(1.0)
                    .with_alt("Left Viewport"),
            );
            row.viewport_3d(
                VP_KEY_2,
                Viewport3DDesc::new(ViewportId::new(2))
                    .with_size(Vec2::new(200.0, 150.0))
                    .with_render_scale(1.5)
                    .with_alt("Right Viewport"),
            );
        });
    }
}

/// 2. 多实例 `ViewportId(1)` 与 `ViewportId(2)` 并发布局、物理目标尺寸推导与 resize 统计。
#[test]
fn multi_instance_concurrent_layout_physical_extent_and_resize_count() {
    let mut rt = AppRuntime::new(DualViewportApp, Vec2::new(1000.0, 600.0));
    rt.sync_physical_viewport(Vec2::new(2000.0, 1200.0), 2.0);

    let r1 = rt.frame(0.0);
    assert_eq!(r1.active_viewports_3d, 2);
    assert_eq!(r1.viewports_3d.len(), 2);
    assert_eq!(r1.viewport_resize_count, 2);

    let e1 = r1
        .viewports_3d
        .iter()
        .find(|e| e.viewport_id == ViewportId::new(1))
        .expect("viewport 1 entry");
    let e2 = r1
        .viewports_3d
        .iter()
        .find(|e| e.viewport_id == ViewportId::new(2))
        .expect("viewport 2 entry");

    assert_eq!(e1.physical_size, Some((800, 600)));
    assert_eq!(e2.physical_size, Some((600, 450)));
    assert!((e1.effective_scale - 2.0).abs() < 1e-4);
    assert!((e2.render_scale - 1.5).abs() < 1e-4);

    // 稳态重绘帧：尺寸未变化，resize 计数应为 0。
    rt.invalidate();
    let r2 = rt.frame(0.0);
    assert_eq!(r2.active_viewports_3d, 2);
    assert_eq!(r2.viewport_resize_count, 0);
}

/// 3. 退化零宽 / 零高尺寸安全跳过（零图元、不计入活跃视口），恢复尺寸后正常恢复。
#[test]
fn degenerate_zero_size_safe_skip_and_recovery() {
    let vp_id = ViewportId::new(3);
    let view_calls = Arc::new(AtomicUsize::new(0));
    let app = SingleViewportApp {
        viewport_id: vp_id,
        size: Vec2::new(0.0, 300.0),
        render_scale: 1.0,
        fallback_external: None,
        interactive: true,
        view_calls,
    };
    let mut rt = AppRuntime::new(app, Vec2::new(800.0, 600.0));

    // 零宽退化帧。
    let r_zero_w = rt.frame(0.0);
    assert_eq!(r_zero_w.active_viewports_3d, 0);
    assert!(r_zero_w.viewports_3d.is_empty());
    assert_eq!(r_zero_w.viewport_resize_count, 0);
    assert_eq!(r_zero_w.primitives, 0);

    // 零高退化帧。
    rt.state_mut().size = Vec2::new(400.0, 0.0);
    rt.invalidate();
    let r_zero_h = rt.frame(0.0);
    assert_eq!(r_zero_h.active_viewports_3d, 0);
    assert!(r_zero_h.viewports_3d.is_empty());
    assert_eq!(r_zero_h.viewport_resize_count, 0);
    assert_eq!(r_zero_h.primitives, 0);

    // 恢复正常尺寸后应完整发射图元并记录一次尺寸变化。
    rt.state_mut().size = Vec2::new(400.0, 300.0);
    rt.invalidate();
    let r_restored = rt.frame(0.0);
    assert_eq!(r_restored.active_viewports_3d, 1);
    assert_eq!(r_restored.viewports_3d[0].physical_size, Some((400, 300)));
    assert_eq!(r_restored.viewport_resize_count, 1);
    assert!(r_restored.primitives > 0);
}

/// 4. 多按键手势（左键旋转、右键平移、Shift+左键平移、滚轮缩放）与 `FrameReport` 往返验证。
#[test]
fn multi_button_gestures_and_wheel_round_trip() {
    let vp_id = ViewportId::new(4);
    let view_calls = Arc::new(AtomicUsize::new(0));
    let app = SingleViewportApp {
        viewport_id: vp_id,
        size: Vec2::new(500.0, 400.0),
        render_scale: 1.0,
        fallback_external: None,
        interactive: true,
        view_calls,
    };
    let mut rt = AppRuntime::new(app, Vec2::new(800.0, 600.0));
    let _ = rt.frame(0.0);

    // 左键拖拽 -> OrbitDelta。
    rt.pointer_down(Vec2::new(100.0, 100.0), PointerButton::Left);
    rt.pointer_move(Vec2::new(124.0, 112.0));
    rt.pointer_up(Vec2::new(124.0, 112.0), PointerButton::Left);
    let r_orbit = rt.frame(0.0);
    assert_eq!(
        r_orbit.viewport_gesture_events,
        vec![Viewport3DGestureEvent {
            viewport_id: vp_id,
            gesture: Viewport3DGesture::OrbitDelta { dx: 24.0, dy: 12.0 },
        }]
    );

    // 右键拖拽 -> PanDelta。
    rt.pointer_down(Vec2::new(150.0, 150.0), PointerButton::Right);
    rt.pointer_move(Vec2::new(140.0, 170.0));
    rt.pointer_up(Vec2::new(140.0, 170.0), PointerButton::Right);
    let r_pan_right = rt.frame(0.0);
    assert_eq!(
        r_pan_right.viewport_gesture_events,
        vec![Viewport3DGestureEvent {
            viewport_id: vp_id,
            gesture: Viewport3DGesture::PanDelta {
                dx: -10.0,
                dy: 20.0,
            },
        }]
    );

    // Shift + 左键拖拽 -> PanDelta（通过 `key` 更新修饰键并配合 `set_modifiers` 复位）。
    rt.key(
        AppKey::Character(' '),
        true,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    rt.pointer_down(Vec2::new(200.0, 200.0), PointerButton::Left);
    rt.pointer_move(Vec2::new(215.0, 195.0));
    rt.pointer_up(Vec2::new(215.0, 195.0), PointerButton::Left);
    rt.set_modifiers(Modifiers::default());
    let r_pan_shift = rt.frame(0.0);
    assert_eq!(
        r_pan_shift.viewport_gesture_events,
        vec![Viewport3DGestureEvent {
            viewport_id: vp_id,
            gesture: Viewport3DGesture::PanDelta { dx: 15.0, dy: -5.0 },
        }]
    );

    // 滚轮 -> ZoomDelta，且下一帧自动排空。
    rt.wheel(Vec2::new(200.0, 200.0), Vec2::new(0.0, 3.0));
    let r_zoom = rt.frame(0.0);
    assert_eq!(
        r_zoom.viewport_gesture_events,
        vec![Viewport3DGestureEvent {
            viewport_id: vp_id,
            gesture: Viewport3DGesture::ZoomDelta { delta: 3.0 },
        }]
    );
    let r_empty = rt.frame(0.0);
    assert!(r_empty.viewport_gesture_events.is_empty());
}

/// 上层 HUD 按钮覆盖 3D 视口测试应用。
struct HudOverlayApp;

struct HudOverlayState {
    hud_clicks: u32,
}

impl Application for HudOverlayApp {
    type State = HudOverlayState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        HudOverlayState { hud_clicks: 0 }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        if let AppEvent::Click { node, .. } = event {
            if node == HUD_BUTTON_KEY {
                state.hud_clicks += 1;
                return EventOutcome::new(DirtyFlags::PAINT);
            }
        }
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder) {
        view.viewport_3d(
            VP_KEY_1,
            Viewport3DDesc::new(ViewportId::new(5))
                .with_size(Vec2::new(400.0, 300.0))
                .with_z(0)
                .with_alt("Scene Viewport"),
        );
        // 在视口左上角叠加更高 Z 序的 HUD 工具栏按钮。
        let mut hud_btn = ButtonDesc::new(Rect::new(20.0, 20.0, 100.0, 36.0), "Reset Camera");
        hud_btn.z = 16;
        let control = view.ctx.control_state(HUD_BUTTON_KEY, false, false);
        let out = emit_button(HUD_BUTTON_KEY, &hud_btn, control, &mut []);
        view.push(HUD_BUTTON_KEY, out);
    }
}

/// 5. 高 Z 序 HUD 按钮命中优先级：点击/拖拽 HUD 按钮触发按钮点击且产出零视口手势。
#[test]
fn upper_z_hud_button_overlay_blocks_viewport_gestures() {
    let mut rt = AppRuntime::new(HudOverlayApp, Vec2::new(600.0, 400.0));
    let _ = rt.frame(0.0);

    // 点击位于 HUD 按钮内部的坐标 `(50, 36)`。
    rt.pointer_down(Vec2::new(50.0, 36.0), PointerButton::Left);
    rt.pointer_move(Vec2::new(52.0, 37.0));
    rt.pointer_up(Vec2::new(52.0, 37.0), PointerButton::Left);
    let r_hud = rt.frame(0.0);
    assert_eq!(rt.state().hud_clicks, 1);
    assert!(
        r_hud.viewport_gesture_events.is_empty(),
        "clicking upper-Z HUD button must not emit viewport gestures"
    );

    // 在暴露的 3D 视口空白区域 `(250, 180)` 拖拽，应触发视口旋转且不增加按钮点击数。
    rt.pointer_down(Vec2::new(250.0, 180.0), PointerButton::Left);
    rt.pointer_move(Vec2::new(280.0, 190.0));
    rt.pointer_up(Vec2::new(280.0, 190.0), PointerButton::Left);
    let r_vp = rt.frame(0.0);
    assert_eq!(rt.state().hud_clicks, 1);
    assert_eq!(
        r_vp.viewport_gesture_events,
        vec![Viewport3DGestureEvent {
            viewport_id: ViewportId::new(5),
            gesture: Viewport3DGesture::OrbitDelta { dx: 30.0, dy: 10.0 },
        }]
    );
}

/// 6. `set_viewport_render_scale` 更新物理目标尺寸与 resize 计数，且不触发布局重算。
#[test]
fn set_viewport_render_scale_updates_physical_size_without_layout() {
    let vp_id = ViewportId::new(6);
    let view_calls = Arc::new(AtomicUsize::new(0));
    let app = SingleViewportApp {
        viewport_id: vp_id,
        size: Vec2::new(640.0, 360.0),
        render_scale: 1.0,
        fallback_external: None,
        interactive: true,
        view_calls: Arc::clone(&view_calls),
    };
    let mut rt = AppRuntime::new(app, Vec2::new(800.0, 600.0));

    let r1 = rt.frame(0.0);
    assert_eq!(r1.viewports_3d[0].physical_size, Some((640, 360)));
    assert_eq!(r1.viewport_resize_count, 1);
    let calls_after_init = view_calls.load(Ordering::Relaxed);

    // 稳态空帧确认快路径生效。
    let r_idle = rt.frame(0.0);
    assert_eq!(r_idle.diff, SceneDiff::Identical);
    assert_eq!(view_calls.load(Ordering::Relaxed), calls_after_init);

    // 动态调整渲染分辨率倍率至 0.5：不得触发布局重算或 `view()` 重建。
    rt.set_viewport_render_scale(vp_id, 0.5);
    let r_scaled = rt.frame(0.0);
    assert_eq!(r_scaled.diff, SceneDiff::Identical);
    assert_eq!(r_scaled.dirty, DirtyFlags::NONE);
    assert_eq!(view_calls.load(Ordering::Relaxed), calls_after_init);
    assert_eq!(r_scaled.active_viewports_3d, 1);
    assert!((r_scaled.viewports_3d[0].render_scale - 0.5).abs() < 1e-6);
    assert_eq!(r_scaled.viewports_3d[0].physical_size, Some((320, 180)));
    assert_eq!(r_scaled.viewport_resize_count, 1);

    // 紧接着的下一帧不应重复计入 resize。
    let r_next = rt.frame(0.0);
    assert_eq!(r_next.viewport_resize_count, 0);
    assert_eq!(r_next.viewports_3d[0].physical_size, Some((320, 180)));
}

/// 分屏动画宽度驱动的 3D 视口测试应用。
struct SplitPaneViewportApp;

struct SplitPaneViewportState {
    pane_width: f32,
    fallback_external: Option<ExternalTextureHandle>,
}

impl Application for SplitPaneViewportApp {
    type State = SplitPaneViewportState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        SplitPaneViewportState {
            pane_width: 200.0,
            fallback_external: None,
        }
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        let mut desc = Viewport3DDesc::new(ViewportId::new(7))
            .with_size(Vec2::new(state.pane_width, 360.0))
            .with_alt("Split Pane Viewport");
        if let Some(handle) = state.fallback_external {
            desc = desc.with_fallback_external_texture(Some(handle.0));
        }
        view.viewport_3d_with_layout(
            VP_KEY_1,
            RawNodeDesc::container()
                .width(Dim::px(state.pane_width))
                .height(Dim::px(360.0)),
            desc,
        );
    }
}

/// 7. 分屏动画逐帧调整尺寸与模拟 `recover_gpu` 槽位失效恢复（零黑帧与零陈旧槽位）。
#[test]
fn animated_split_pane_resize_and_gpu_slot_invalidation_recovery() {
    let vp_id = ViewportId::new(7);
    let mut rt = AppRuntime::new(SplitPaneViewportApp, Vec2::new(900.0, 600.0));

    // 预备软件降级外部纹理与初始 GPU Bindless 槽位。
    let fallback_h = rt
        .asset_hub_mut()
        .upload_external_rgba8(2, 2, true, vec![200u8; 16]);
    rt.asset_hub_mut()
        .commit_external_texture_slot(fallback_h, Ok(51));
    rt.state_mut().fallback_external = Some(fallback_h);
    rt.commit_viewport_3d_slot(vp_id, Some(61));

    // 模拟分屏拖拽动画经过 5 个中间宽度帧。
    for width in [200.0_f32, 260.0, 320.0, 380.0, 440.0] {
        rt.state_mut().pane_width = width;
        rt.invalidate();
        let report = rt.frame(1.0 / 60.0);
        assert_eq!(report.active_viewports_3d, 1);
        assert_eq!(report.viewport_resize_count, 1);
        assert_eq!(
            report.viewports_3d[0].physical_size,
            Some((width as u32, 360))
        );
        let img = rt
            .scene()
            .primitives
            .iter()
            .find(|p| prim_type_of(p) == PrimType::Image)
            .expect("viewport image primitive");
        assert_eq!(img.border_token_or_tex, 61);
    }

    // 模拟 `recover_gpu`：失效全部 3D 视口 GPU 槽位，下一帧无缝回退到软件降级外部纹理 51。
    rt.invalidate_all_viewport_3d_slots();
    assert_eq!(rt.viewport_3d_slot(vp_id), None);
    let r_fallback = rt.frame(1.0 / 60.0);
    assert_eq!(r_fallback.active_viewports_3d, 1);
    let img_fallback = rt
        .scene()
        .primitives
        .iter()
        .find(|p| prim_type_of(p) == PrimType::Image)
        .expect("fallback external texture primitive");
    assert_eq!(img_fallback.border_token_or_tex, 51);

    // 模拟设备恢复完成并重新提交新 GPU 槽位 62。
    rt.commit_viewport_3d_slot(vp_id, Some(62));
    let r_recovered = rt.frame(1.0 / 60.0);
    assert_eq!(r_recovered.active_viewports_3d, 1);
    let img_recovered = rt
        .scene()
        .primitives
        .iter()
        .find(|p| prim_type_of(p) == PrimType::Image)
        .expect("recovered gpu slot primitive");
    assert_eq!(img_recovered.border_token_or_tex, 62);
}
