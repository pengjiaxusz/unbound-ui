//! 视频表面无头集成测试（覆盖生命周期、零闪烁切换、Latest-Wins、显存 LRU 治理与视图能力边界）。

use unbound_app::{
    AppContext, AppEvent, AppRuntime, Application, DirtyFlags, EventOutcome, VideoSurfaceDesc,
    VideoSurfaceHandle, VideoSurfaceOp, VideoSurfaceState, VideoUploadTier, ViewBuilder,
};
use unbound_asset::{VideoColorSpace, VideoFramePayload, VideoPixelFormat};
use unbound_gpu::data::PrimType;
use unbound_input::PointerButton;
use unbound_math::{f16, Rect, Vec2};
use unbound_theme::Token;
use unbound_widgets::diff::SceneDiff;
use unbound_widgets::ObjectFit;

const VIDEO_KEY_A: u64 = 1001;
const VIDEO_KEY_B: u64 = 1002;
const VIDEO_KEY_C: u64 = 1003;
const ZERO_W_KEY: u64 = 1004;
const ZERO_H_KEY: u64 = 1005;

/// 构造合法的紧凑 NV12 测试帧载荷。
fn make_nv12_frame(width: u32, height: u32, pts_us: i64, fill: u8) -> VideoFramePayload {
    let bytes = VideoPixelFormat::Nv12.estimated_source_vram_bytes(width, height) as usize;
    VideoFramePayload::from_nv12(
        width,
        height,
        VideoColorSpace::bt709_limited(),
        pts_us,
        vec![fill; bytes],
    )
    .expect("valid nv12 frame")
}

/// 构造合法的紧凑 P010 测试帧载荷。
fn make_p010_frame(width: u32, height: u32, pts_us: i64, fill: u8) -> VideoFramePayload {
    let bytes = VideoPixelFormat::P010.estimated_source_vram_bytes(width, height) as usize;
    VideoFramePayload::from_p010(
        width,
        height,
        VideoColorSpace::bt2020_pq(),
        pts_us,
        vec![fill; bytes],
    )
    .expect("valid p010 frame")
}

/// 单视频表面测试应用，支持按需调整描述符与可见开关。
struct SingleVideoApp;

struct SingleVideoState {
    desc: Option<VideoSurfaceDesc>,
    extra_degenerates: bool,
}

impl Application for SingleVideoApp {
    type State = SingleVideoState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        SingleVideoState {
            desc: None,
            extra_degenerates: false,
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
        if let Some(desc) = &state.desc {
            view.video_surface(VIDEO_KEY_A, desc.clone());
        }
        if state.extra_degenerates {
            let handle = state
                .desc
                .as_ref()
                .map_or(VideoSurfaceHandle(9999), |d| d.handle);
            view.video_surface(
                ZERO_W_KEY,
                VideoSurfaceDesc::new(handle, Vec2::new(0.0, 120.0)),
            );
            view.video_surface(
                ZERO_H_KEY,
                VideoSurfaceDesc::new(handle, Vec2::new(240.0, 0.0)),
            );
        }
    }
}

/// 多路视频表面测试应用，用于在视口内动态挂载或隐藏不同通道。
struct MultiVideoApp;

struct MultiVideoState {
    visible: Vec<(u64, VideoSurfaceHandle)>,
}

impl Application for MultiVideoApp {
    type State = MultiVideoState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        MultiVideoState {
            visible: Vec::new(),
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
        for &(key, handle) in &state.visible {
            let desc = VideoSurfaceDesc::new(handle, Vec2::new(240.0, 135.0));
            view.video_surface(key, desc);
        }
    }
}

#[test]
fn full_lifecycle_pending_ready_resizing_evicted_released_and_placeholder() {
    let mut rt = AppRuntime::new(SingleVideoApp, Vec2::new(800.0, 600.0));
    let handle = rt.asset_hub_mut().create_video_surface();
    assert_eq!(
        rt.asset_hub().video_surface_state(handle),
        Some(&VideoSurfaceState::PendingFirstFrame)
    );
    assert_eq!(rt.asset_hub().video_surface_slot(handle), None);

    rt.state_mut().desc = Some(
        VideoSurfaceDesc::new(handle, Vec2::new(320.0, 180.0))
            .with_placeholder_color(Token::SURFACE_SUNKEN)
            .with_radius(8.0),
    );
    rt.invalidate();

    // 阶段 1：PendingFirstFrame 输出受控圆角占位图元，绝不发射 PRIM_IMAGE。
    let report1 = rt.frame(0.016);
    assert_eq!(report1.video_upload_tier, VideoUploadTier::HeadlessStub);
    assert!(report1.video_surface_ops.is_empty());
    assert_eq!(rt.scene().primitives.len(), 1);
    assert_eq!(rt.scene().primitives[0].prim_type(), PrimType::RoundedRect);
    assert_eq!(
        rt.scene().primitives[0].fill_token_or_uv,
        u32::from(Token::SURFACE_SUNKEN.raw())
    );

    // 阶段 2：提交首帧并 commit 后进入 Ready，发射 PRIM_IMAGE 采样新槽位。
    let f1 = make_nv12_frame(640, 360, 1_000, 128);
    let vram_f1 = f1.estimated_vram_bytes();
    rt.asset_hub_mut()
        .submit_video_frame(handle, f1)
        .expect("submit first frame");
    let report2 = rt.frame(0.016);
    assert_eq!(report2.video_frame_upload_count, 1);
    assert_eq!(report2.video_surface_ops.len(), 1);
    assert!(matches!(
        &report2.video_surface_ops[0],
        VideoSurfaceOp::UploadFrame {
            handle: h,
            target_slot: None,
            ..
        } if *h == handle
    ));
    let stale = rt.asset_hub_mut().commit_video_surface_slot(
        handle,
        48,
        640,
        360,
        VideoPixelFormat::Nv12,
        vram_f1,
    );
    assert_eq!(stale, None);
    assert_eq!(
        rt.asset_hub().video_surface_state(handle),
        Some(&VideoSurfaceState::Ready {
            slot: 48,
            width: 640,
            height: 360,
            format: VideoPixelFormat::Nv12,
            vram_bytes: vram_f1,
        })
    );

    let report3 = rt.frame(0.016);
    assert_eq!(report3.active_video_surfaces, 1);
    assert_eq!(report3.video_vram_bytes, vram_f1);
    assert_eq!(rt.scene().primitives[0].prim_type(), PrimType::Image);
    assert_eq!(rt.scene().primitives[0].border_token_or_tex, 48);

    // 阶段 3：切换分辨率与格式进入 Resizing，提交新槽位后原子切换并返回旧槽位。
    let f2 = make_p010_frame(1280, 720, 2_000, 64);
    let vram_f2 = f2.estimated_vram_bytes();
    rt.asset_hub_mut()
        .submit_video_frame(handle, f2)
        .expect("submit resized p010 frame");
    let report4 = rt.frame(0.016);
    assert!(matches!(
        rt.asset_hub().video_surface_state(handle),
        Some(VideoSurfaceState::Resizing {
            prev_slot: 48,
            prev_width: 640,
            prev_height: 360,
            prev_format: VideoPixelFormat::Nv12,
            pending_width: 1280,
            pending_height: 720,
            pending_format: VideoPixelFormat::P010,
            ..
        })
    ));
    assert_eq!(
        rt.asset_hub().video_surface_slot(handle),
        Some((48, 640, 360))
    );
    assert!(matches!(
        &report4.video_surface_ops[0],
        VideoSurfaceOp::UploadFrame {
            handle: h,
            target_slot: None,
            ..
        } if *h == handle
    ));
    let old_slot = rt.asset_hub_mut().commit_video_surface_slot(
        handle,
        49,
        1280,
        720,
        VideoPixelFormat::P010,
        vram_f2,
    );
    assert_eq!(old_slot, Some(48));

    // 阶段 4：隐藏表面并收紧预算触发离屏 LRU 驱逐，转为 Evicted 后重回视口绘制占位图元。
    rt.state_mut().desc = None;
    rt.asset_hub_mut().set_video_vram_budget_bytes(1024);
    rt.invalidate();
    let report5 = rt.frame(0.016);
    assert_eq!(report5.video_eviction_count, 1);
    assert_eq!(report5.active_video_surfaces, 0);
    assert_eq!(report5.video_vram_bytes, 0);
    assert_eq!(
        report5.video_surface_ops,
        vec![VideoSurfaceOp::Evict { handle, slot: 49 }]
    );
    assert_eq!(
        rt.asset_hub().video_surface_state(handle),
        Some(&VideoSurfaceState::Evicted {
            last_width: 1280,
            last_height: 720,
            last_format: VideoPixelFormat::P010,
        })
    );
    assert_eq!(rt.asset_hub().video_surface_slot(handle), None);

    // 阶段 5：主动释放后转为 Released，视图绑定该句柄仍退化为受控占位图元。
    rt.asset_hub_mut().release_video_surface(handle);
    assert_eq!(
        rt.asset_hub().video_surface_state(handle),
        Some(&VideoSurfaceState::Released)
    );
    rt.state_mut().desc = Some(VideoSurfaceDesc::new(handle, Vec2::new(320.0, 180.0)));
    rt.invalidate();
    let report6 = rt.frame(0.016);
    assert_eq!(
        report6.video_surface_ops,
        vec![VideoSurfaceOp::Release { handle, slot: None }]
    );
    assert_eq!(rt.scene().primitives.len(), 1);
    assert_eq!(rt.scene().primitives[0].prim_type(), PrimType::RoundedRect);
}

#[test]
fn format_and_resolution_switch_preserves_previous_slot_without_flicker() {
    let mut rt = AppRuntime::new(SingleVideoApp, Vec2::new(800.0, 600.0));
    let handle = rt.asset_hub_mut().create_video_surface();
    rt.state_mut().desc = Some(VideoSurfaceDesc::new(handle, Vec2::new(320.0, 180.0)));
    rt.invalidate();

    let nv12 = make_nv12_frame(320, 180, 100, 100);
    let nv12_vram = nv12.estimated_vram_bytes();
    rt.asset_hub_mut()
        .submit_video_frame(handle, nv12)
        .expect("submit nv12");
    let _ = rt.frame(0.016);
    let _ = rt.asset_hub_mut().commit_video_surface_slot(
        handle,
        48,
        320,
        180,
        VideoPixelFormat::Nv12,
        nv12_vram,
    );
    let _ = rt.frame(0.016);
    assert_eq!(rt.scene().primitives[0].prim_type(), PrimType::Image);
    assert_eq!(rt.scene().primitives[0].border_token_or_tex, 48);

    // 切至 P010 640x360：在 commit 之前，中间帧持续绘制旧槽位 48 实现零闪烁保帧。
    let p010 = make_p010_frame(640, 360, 200, 200);
    let p010_vram = p010.estimated_vram_bytes();
    rt.asset_hub_mut()
        .submit_video_frame(handle, p010)
        .expect("submit p010");
    let resize_report = rt.frame(0.016);
    assert_eq!(resize_report.video_surface_ops.len(), 1);
    assert!(matches!(
        &resize_report.video_surface_ops[0],
        VideoSurfaceOp::UploadFrame {
            target_slot: None,
            ..
        }
    ));

    // 再跑一帧未 commit 的中间帧，画面依旧稳定绑定旧槽位 48 而非回退占位框。
    rt.invalidate();
    let _ = rt.frame(0.016);
    assert_eq!(rt.scene().primitives[0].prim_type(), PrimType::Image);
    assert_eq!(rt.scene().primitives[0].border_token_or_tex, 48);

    // 新槽位 49 提交后返回旧槽位 48 供延迟销毁，下一帧原子切换至槽位 49。
    let stale = rt.asset_hub_mut().commit_video_surface_slot(
        handle,
        49,
        640,
        360,
        VideoPixelFormat::P010,
        p010_vram,
    );
    assert_eq!(stale, Some(48));
    let _ = rt.frame(0.016);
    assert_eq!(rt.scene().primitives[0].prim_type(), PrimType::Image);
    assert_eq!(rt.scene().primitives[0].border_token_or_tex, 49);
}

#[test]
fn burst_submission_uses_latest_wins_and_tracks_dropped_frames() {
    let mut rt = AppRuntime::new(SingleVideoApp, Vec2::new(800.0, 600.0));
    let handle = rt.asset_hub_mut().create_video_surface();
    rt.state_mut().desc = Some(VideoSurfaceDesc::new(handle, Vec2::new(320.0, 180.0)));
    rt.invalidate();

    // 在单帧消费前快速连续提交 5 帧，前 4 帧应被 Latest-Wins 覆盖并计入丢帧数。
    for i in 1..=5_i64 {
        let frame = make_nv12_frame(320, 180, i * 16_666, i as u8);
        rt.asset_hub_mut()
            .submit_video_frame(handle, frame)
            .expect("burst submit");
    }

    let report = rt.frame(0.016);
    assert_eq!(report.video_frame_upload_count, 1);
    assert_eq!(report.video_frame_dropped_count, 4);
    assert_eq!(report.video_surface_ops.len(), 1);
    match &report.video_surface_ops[0] {
        VideoSurfaceOp::UploadFrame { frame, .. } => {
            assert_eq!(frame.pts_us, 5 * 16_666);
        }
        other => panic!("expected UploadFrame op, got {other:?}"),
    }
}

#[test]
fn same_size_updates_avoid_layout_recompute_and_idle_frames_sleep() {
    let mut rt = AppRuntime::new(SingleVideoApp, Vec2::new(800.0, 600.0));
    let handle = rt.asset_hub_mut().create_video_surface();
    rt.state_mut().desc = Some(VideoSurfaceDesc::new(handle, Vec2::new(320.0, 180.0)));
    rt.invalidate();

    // 首帧建立布局并提交槽位 50。
    let first = make_nv12_frame(320, 180, 1_000, 10);
    let vram = first.estimated_vram_bytes();
    rt.asset_hub_mut()
        .submit_video_frame(handle, first)
        .expect("submit first");
    let _ = rt.frame(0.016);
    let _ = rt.asset_hub_mut().commit_video_surface_slot(
        handle,
        50,
        320,
        180,
        VideoPixelFormat::Nv12,
        vram,
    );
    let _ = rt.frame(0.016);

    // 连续提交 3 帧同尺寸同格式视频：走原地更新且零布局重算。
    for step in 1..=3_u64 {
        let next = make_nv12_frame(320, 180, (step as i64 + 1) * 16_666, step as u8);
        rt.asset_hub_mut()
            .submit_video_frame(handle, next)
            .expect("submit same-size frame");
        let report = rt.frame(0.016);
        assert_eq!(report.video_frame_upload_count, 1 + step);
        assert!(!report.dirty.contains(DirtyFlags::LAYOUT));
        assert_eq!(rt.last_layout_stats().nodes_visited, 0);
        assert_eq!(report.diff, SceneDiff::Identical);
        assert_eq!(report.video_surface_ops.len(), 1);
        assert!(matches!(
            &report.video_surface_ops[0],
            VideoSurfaceOp::UploadFrame {
                handle: h,
                target_slot: Some(50),
                ..
            } if *h == handle
        ));
        let stale = rt.asset_hub_mut().commit_video_surface_slot(
            handle,
            50,
            320,
            180,
            VideoPixelFormat::Nv12,
            vram,
        );
        assert_eq!(stale, None);
    }

    // 无新帧到达时：video_surface_ops 为空、无脏标志、SceneDiff 保持 Identical（允许空闲休眠）。
    let idle_report = rt.frame(0.016);
    assert!(idle_report.video_surface_ops.is_empty());
    assert_eq!(idle_report.dirty, DirtyFlags::NONE);
    assert_eq!(idle_report.diff, SceneDiff::Identical);
    assert_eq!(idle_report.video_frame_upload_count, 4);
}

#[test]
fn multi_stream_vram_budget_evicts_offscreen_lru_and_recovers_seamlessly() {
    let mut rt = AppRuntime::new(MultiVideoApp, Vec2::new(1280.0, 720.0));
    let h_a = rt.asset_hub_mut().create_video_surface();
    let h_b = rt.asset_hub_mut().create_video_surface();
    let h_c = rt.asset_hub_mut().create_video_surface();

    let sample = make_nv12_frame(640, 360, 0, 1);
    let single_vram = sample.estimated_vram_bytes();
    // 预算恰好容纳 2 路视频，第 3 路进入时必须驱逐离屏最久的表面。
    let budget = single_vram * 2;
    rt.asset_hub_mut().set_video_vram_budget_bytes(budget);
    assert_eq!(rt.asset_hub().video_vram_budget_bytes(), budget);

    // 第 1 帧：A 与 B 均在视口内可见并上传提交。
    rt.state_mut().visible = vec![(VIDEO_KEY_A, h_a), (VIDEO_KEY_B, h_b)];
    rt.asset_hub_mut()
        .submit_video_frame(h_a, make_nv12_frame(640, 360, 10, 1))
        .expect("submit A");
    rt.asset_hub_mut()
        .submit_video_frame(h_b, make_nv12_frame(640, 360, 10, 2))
        .expect("submit B");
    rt.invalidate();
    let _ = rt.frame(0.016);
    let _ = rt.asset_hub_mut().commit_video_surface_slot(
        h_a,
        48,
        640,
        360,
        VideoPixelFormat::Nv12,
        single_vram,
    );
    let _ = rt.asset_hub_mut().commit_video_surface_slot(
        h_b,
        49,
        640,
        360,
        VideoPixelFormat::Nv12,
        single_vram,
    );

    // 第 2 帧：A 离开视口，B 与 C 在视口内可见；C 提交后立即驱逐离屏的 A，且不误逐 B/C。
    rt.state_mut().visible = vec![(VIDEO_KEY_B, h_b), (VIDEO_KEY_C, h_c)];
    rt.asset_hub_mut()
        .submit_video_frame(h_c, make_nv12_frame(640, 360, 20, 3))
        .expect("submit C");
    rt.invalidate();
    let _ = rt.frame(0.016);
    let _ = rt.asset_hub_mut().commit_video_surface_slot(
        h_c,
        50,
        640,
        360,
        VideoPixelFormat::Nv12,
        single_vram,
    );

    let report_after_c = rt.frame(0.016);
    assert!(report_after_c.video_eviction_count >= 1);
    assert!(report_after_c.video_vram_bytes <= budget);
    assert_eq!(report_after_c.active_video_surfaces, 2);
    assert!(report_after_c
        .video_surface_ops
        .contains(&VideoSurfaceOp::Evict {
            handle: h_a,
            slot: 48
        }));
    assert!(matches!(
        rt.asset_hub().video_surface_state(h_a),
        Some(VideoSurfaceState::Evicted {
            last_width: 640,
            last_height: 360,
            last_format: VideoPixelFormat::Nv12,
        })
    ));
    assert!(matches!(
        rt.asset_hub().video_surface_state(h_b),
        Some(VideoSurfaceState::Ready { slot: 49, .. })
    ));
    assert!(matches!(
        rt.asset_hub().video_surface_state(h_c),
        Some(VideoSurfaceState::Ready { slot: 50, .. })
    ));

    // 第 3 帧：A 重回视口（C 离屏）并收到新帧，应无缝重新分配槽位并恢复 Ready。
    rt.state_mut().visible = vec![(VIDEO_KEY_A, h_a), (VIDEO_KEY_B, h_b)];
    rt.asset_hub_mut()
        .submit_video_frame(h_a, make_nv12_frame(640, 360, 30, 4))
        .expect("resubmit A");
    rt.invalidate();
    let recover_report = rt.frame(0.016);
    assert!(recover_report.video_surface_ops.iter().any(|op| matches!(
        op,
        VideoSurfaceOp::UploadFrame {
            handle,
            target_slot: None,
            ..
        } if *handle == h_a
    )));
    let _ = rt.asset_hub_mut().commit_video_surface_slot(
        h_a,
        51,
        640,
        360,
        VideoPixelFormat::Nv12,
        single_vram,
    );
    let settled = rt.frame(0.016);
    assert!(settled.video_vram_bytes <= budget);
    assert!(matches!(
        rt.asset_hub().video_surface_state(h_a),
        Some(VideoSurfaceState::Ready { slot: 51, .. })
    ));
    assert!(rt
        .scene()
        .primitives
        .iter()
        .any(|p| p.prim_type() == PrimType::Image && p.border_token_or_tex == 51));
}

#[test]
fn view_builder_video_surface_geometry_boundaries_focus_animation_and_inspector() {
    let mut rt = AppRuntime::new(SingleVideoApp, Vec2::new(800.0, 600.0));
    rt.enable_debug_sanitizers(true);
    let handle = rt.asset_hub_mut().create_video_surface();

    // 提交 16:9（640x360）视频并置为 Ready。
    let frame = make_nv12_frame(640, 360, 1_000, 99);
    let vram = frame.estimated_vram_bytes();
    rt.asset_hub_mut()
        .submit_video_frame(handle, frame)
        .expect("submit frame");
    rt.state_mut().desc = Some(
        VideoSurfaceDesc::new(handle, Vec2::new(200.0, 200.0))
            .with_fit(ObjectFit::Contain)
            .with_radius(12.0)
            .with_opacity(0.5)
            .with_alt("Main Video"),
    );
    rt.state_mut().extra_degenerates = true;
    rt.invalidate();
    let _ = rt.frame(0.016);
    let _ = rt.asset_hub_mut().commit_video_surface_slot(
        handle,
        52,
        640,
        360,
        VideoPixelFormat::Nv12,
        vram,
    );

    // 1. Contain 裁剪几何 + radius + opacity + 零宽/零高退化尺寸安全跳过 + Inspector 零违规。
    let _ = rt.frame(0.016);
    // 零宽与零高节点均被安全跳过，仅存在主视频表面的 1 个 PRIM_IMAGE 图元。
    let image_prims: Vec<_> = rt
        .scene()
        .primitives
        .iter()
        .filter(|p| p.prim_type() == PrimType::Image)
        .collect();
    assert_eq!(image_prims.len(), 1);
    let contain_prim = image_prims[0];
    assert_eq!(contain_prim.border_token_or_tex, 52);
    // 16:9 放入 200x200 的 Contain：宽 200，高 112.5，垂直居中。
    assert!((contain_prim.size[0] - 200.0).abs() < 0.5);
    assert!((contain_prim.size[1] - 112.5).abs() < 0.5);
    assert_eq!(contain_prim.corner_radii, f16::pack_radii([12.0; 4]));
    assert!((contain_prim.opacity() - 0.5).abs() < 0.02);
    assert!(
        rt.inspector().violations().is_empty(),
        "inspector reported violations: {:?}",
        rt.inspector().violations()
    );

    // 2. 默认 interactive = false，点击视频表面不抢占键盘焦点。
    let _ = rt.dispatch(AppEvent::PointerDown {
        pos: Vec2::new(100.0, 100.0),
        target: None,
        button: PointerButton::Left,
        click_count: 1,
    });
    let _ = rt.dispatch(AppEvent::PointerUp {
        pos: Vec2::new(100.0, 100.0),
        target: None,
        button: PointerButton::Left,
    });
    assert_eq!(rt.focus().focused(), None);

    // 3. Cover 模式 + 自定义 sample_uv 验证。
    let custom_uv = Rect::new(0.25, 0.25, 0.5, 0.5);
    rt.state_mut().desc = Some(
        VideoSurfaceDesc::new(handle, Vec2::new(200.0, 200.0))
            .with_fit(ObjectFit::Cover)
            .with_sample_uv(custom_uv),
    );
    rt.state_mut().extra_degenerates = false;
    rt.invalidate();
    let _ = rt.frame(0.016);
    let cover_prim = rt
        .scene()
        .primitives
        .iter()
        .find(|p| p.prim_type() == PrimType::Image)
        .expect("cover image primitive");
    assert!((cover_prim.size[0] - 200.0).abs() < 0.5);
    assert!((cover_prim.size[1] - 200.0).abs() < 0.5);
    let payload = &rt.scene().payloads[cover_prim.extra_payload_idx as usize];
    assert!((payload.p0[0] - 0.25).abs() < 1e-4);
    assert!((payload.p0[1] - 0.25).abs() < 1e-4);
    assert!((payload.p1[0] - 0.75).abs() < 1e-4);
    assert!((payload.p1[1] - 0.75).abs() < 1e-4);

    // 4. 容器缩放动画中间帧不触发目标纹理重建（无 UploadFrame 且槽位恒定）。
    for step_size in [
        Vec2::new(320.0, 180.0),
        Vec2::new(400.0, 225.0),
        Vec2::new(480.0, 270.0),
    ] {
        rt.state_mut().desc = Some(VideoSurfaceDesc::new(handle, step_size));
        rt.invalidate();
        let anim_report = rt.frame(0.016);
        assert!(anim_report.video_surface_ops.is_empty());
        let prim = rt
            .scene()
            .primitives
            .iter()
            .find(|p| p.prim_type() == PrimType::Image)
            .expect("animated image primitive");
        assert_eq!(prim.border_token_or_tex, 52);
        assert!((prim.size[0] - step_size.x).abs() < 0.5);
        assert!((prim.size[1] - step_size.y).abs() < 0.5);
    }
    assert!(
        rt.inspector().violations().is_empty(),
        "inspector reported violations after animation: {:?}",
        rt.inspector().violations()
    );
}
