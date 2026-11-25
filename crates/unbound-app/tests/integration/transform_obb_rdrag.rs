//! Integration tests for 2D affine transform passthrough, ExternalImage UV/crop/transform
//! emission, rotated OBB gizmo builder, and right-button drag vs context-menu flow.

use unbound_abi::{NodeFlags, RawNodeDesc};
use unbound_app::{
    AppContext, AppEvent, AppRuntime, Application, DirtyFlags, EventOutcome, ExternalTextureDesc,
    ExternalTextureHandle, ViewBuilder,
};
use unbound_canvas::Handle;
use unbound_gpu::data::PrimType;
use unbound_input::PointerButton;
use unbound_math::{Affine2, Rect, Vec2};
use unbound_theme::Token;
use unbound_widgets::{GizmoDesc, ObjectFit};

const NODE_CONTAINER: u64 = 101;
const NODE_CHILD: u64 = 102;
const NODE_LEAF: u64 = 103;
const NODE_EXT_IMG: u64 = 201;
const NODE_GIZMO: u64 = 301;
const NODE_DRAG_TARGET: u64 = 401;

#[test]
fn non_identity_node_transform_pushes_gpu_transform_for_containers_and_leaves() {
    struct TransformApp;

    impl Application for TransformApp {
        type State = ();

        fn init(&mut self, _ctx: &mut AppContext) {}

        fn reduce(
            &mut self,
            _state: &mut Self::State,
            _event: AppEvent,
            _ctx: &mut AppContext,
        ) -> EventOutcome {
            EventOutcome::new(DirtyFlags::NONE)
        }

        fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
            let rot = Affine2::around(
                Vec2::new(50.0, 50.0),
                Vec2::new(-1.0, 1.0),
                core::f32::consts::FRAC_PI_4,
                Vec2::new(10.0, 5.0),
            );
            let raw_rot = [rot.m00, rot.m10, rot.m01, rot.m11, rot.tx, rot.ty];

            view.container(
                NODE_CONTAINER,
                RawNodeDesc::container()
                    .size(100.0, 100.0)
                    .fill(u32::from(Token::PRIMARY.raw()))
                    .transform(raw_rot),
                |v| {
                    v.container(
                        NODE_CHILD,
                        RawNodeDesc::container()
                            .size(40.0, 40.0)
                            .fill(u32::from(Token::SURFACE.raw())),
                        |_| {},
                    );
                },
            );

            // Leaf widget with its own non-identity RawNodeDesc::transform.
            let leaf_tf = Affine2::translation(Vec2::new(12.0, 18.0));
            let leaf_desc = RawNodeDesc::container().size(60.0, 30.0).transform([
                leaf_tf.m00,
                leaf_tf.m10,
                leaf_tf.m01,
                leaf_tf.m11,
                leaf_tf.tx,
                leaf_tf.ty,
            ]);
            view.image_external_with_layout(
                NODE_LEAF,
                leaf_desc,
                ExternalTextureDesc::new(
                    ExternalTextureHandle(999),
                    Vec2::new(60.0, 30.0),
                    ObjectFit::Contain,
                ),
            );
        }
    }

    let mut rt = AppRuntime::new(TransformApp, Vec2::new(640.0, 480.0));
    let _ = rt.frame(0.016);
    let scene = rt.scene();

    // Slot 0 is identity; container and leaf each push one non-identity transform.
    assert!(
        scene.transforms.len() >= 3,
        "expected identity + container + leaf transforms, got {}",
        scene.transforms.len()
    );

    let child_span = scene.span(
        NODE_CHILD
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(1),
    );
    let child_prim = child_span
        .and_then(|s| s.first())
        .expect("child box surface primitive");
    assert_ne!(
        child_prim.transform_idx, 0,
        "child inside transformed container must inherit non-zero transform_idx"
    );
    let child_tf = scene.transforms[child_prim.transform_idx as usize];
    assert!(
        !child_tf.is_identity(),
        "inherited transform must not be identity"
    );

    let leaf_prim = scene
        .span(NODE_LEAF)
        .and_then(|s| s.first())
        .expect("leaf external image primitive");
    assert_ne!(
        leaf_prim.transform_idx, 0,
        "leaf node with RawNodeDesc::transform must receive non-zero transform_idx"
    );
    let leaf_tf = scene.transforms[leaf_prim.transform_idx as usize];
    assert!((leaf_tf.translation[0] - 12.0).abs() < 1e-4);
    assert!((leaf_tf.translation[1] - 18.0).abs() < 1e-4);
}

#[test]
fn external_image_emits_crop_uv_and_local_2d_rotation_flip_transform() {
    struct ExtImgState {
        desc: Option<ExternalTextureDesc>,
    }
    struct ExtImgApp;

    impl Application for ExtImgApp {
        type State = ExtImgState;

        fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
            ExtImgState { desc: None }
        }

        fn reduce(
            &mut self,
            _state: &mut Self::State,
            _event: AppEvent,
            _ctx: &mut AppContext,
        ) -> EventOutcome {
            EventOutcome::new(DirtyFlags::NONE)
        }

        fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>) {
            if let Some(desc) = state.desc {
                view.image_external_with(NODE_EXT_IMG, desc);
            }
        }
    }

    let mut rt = AppRuntime::new(ExtImgApp, Vec2::new(400.0, 300.0));
    let handle =
        rt.asset_hub_mut()
            .upload_external_rgba8(200, 100, true, vec![255u8; 200 * 100 * 4]);
    let _ = rt.frame(0.0);
    rt.asset_hub_mut()
        .commit_external_texture_slot(handle, Ok(42));

    // 1. Pixel-space crop_rect (50, 25, 100, 50) on a 200x100 texture -> UV (0.25, 0.25)..(0.75, 0.75)
    // plus horizontal flip and 90 deg rotation.
    rt.state_mut().desc = Some(
        ExternalTextureDesc::new(handle, Vec2::new(160.0, 80.0), ObjectFit::Fill)
            .with_crop_rect(Rect::new(50.0, 25.0, 100.0, 50.0))
            .with_rotation(core::f32::consts::FRAC_PI_2)
            .with_flip_x(true),
    );
    rt.invalidate();
    let _ = rt.frame(0.016);

    let scene = rt.scene();
    let prim = scene
        .primitives
        .iter()
        .find(|p| p.prim_type() == PrimType::Image)
        .expect("emitted external image primitive");
    assert_eq!(prim.border_token_or_tex, 42);
    assert_ne!(
        prim.transform_idx, 0,
        "rotated/flipped ExternalImage must push a GPU transform"
    );
    let tf = scene.transforms[prim.transform_idx as usize];
    assert!(!tf.is_identity());
    let payload = &scene.payloads[prim.extra_payload_idx as usize];
    assert!((payload.p0[0] - 0.25).abs() < 1e-4);
    assert!((payload.p0[1] - 0.25).abs() < 1e-4);
    assert!((payload.p1[0] - 0.75).abs() < 1e-4);
    assert!((payload.p1[1] - 0.75).abs() < 1e-4);

    // 2. Explicit sample_uv overrides crop_rect.
    rt.state_mut().desc = Some(
        ExternalTextureDesc::new(handle, Vec2::new(160.0, 80.0), ObjectFit::Fill)
            .with_crop_rect(Rect::new(50.0, 25.0, 100.0, 50.0))
            .with_sample_uv(Rect::new(0.1, 0.2, 0.4, 0.5)),
    );
    rt.invalidate();
    let _ = rt.frame(0.016);
    let scene2 = rt.scene();
    let prim2 = scene2
        .primitives
        .iter()
        .find(|p| p.prim_type() == PrimType::Image)
        .expect("emitted sample_uv image primitive");
    let payload2 = &scene2.payloads[prim2.extra_payload_idx as usize];
    assert!((payload2.p0[0] - 0.1).abs() < 1e-4);
    assert!((payload2.p0[1] - 0.2).abs() < 1e-4);
    assert!((payload2.p1[0] - 0.5).abs() < 1e-4);
    assert!((payload2.p1[1] - 0.7).abs() < 1e-4);
}

#[test]
fn view_builder_gizmo_emits_rotated_obb_handles_and_hit_regions() {
    struct GizmoApp;

    impl Application for GizmoApp {
        type State = ();

        fn init(&mut self, _ctx: &mut AppContext) {}

        fn reduce(
            &mut self,
            _state: &mut Self::State,
            _event: AppEvent,
            _ctx: &mut AppContext,
        ) -> EventOutcome {
            EventOutcome::new(DirtyFlags::NONE)
        }

        fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
            let desc = GizmoDesc::new(Rect::new(100.0, 100.0, 200.0, 100.0), 1.0)
                .with_rotation(core::f32::consts::FRAC_PI_2);
            view.gizmo(NODE_GIZMO, desc);
        }
    }

    let mut rt = AppRuntime::new(GizmoApp, Vec2::new(640.0, 480.0));
    let _ = rt.frame(0.016);
    let gizmo_hits: Vec<_> = rt
        .router()
        .regions()
        .iter()
        .filter(|r| r.node == NODE_GIZMO)
        .collect();
    assert_eq!(gizmo_hits.len(), Handle::ALL.len());

    let rot_pos = Handle::Rotate.position_rotated(
        Rect::new(100.0, 100.0, 200.0, 100.0),
        core::f32::consts::FRAC_PI_2,
        unbound_widgets::ROTATE_OFFSET,
    );
    assert!(
        gizmo_hits.iter().any(|r| r.bounds.contains_point(rot_pos)),
        "rotated Rotate handle hit region must contain rotated handle point {rot_pos:?}"
    );
}

#[test]
fn right_button_drag_suppresses_context_menu_while_short_press_opens_it() {
    #[derive(Default)]
    struct DragLogState {
        drag_starts: usize,
        drags: usize,
        drag_ends: Vec<PointerButton>,
        context_menus: usize,
    }

    struct RightDragApp;

    impl Application for RightDragApp {
        type State = DragLogState;

        fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
            DragLogState::default()
        }

        fn reduce(
            &mut self,
            state: &mut Self::State,
            event: AppEvent,
            _ctx: &mut AppContext,
        ) -> EventOutcome {
            match event {
                AppEvent::DragStart {
                    node: NODE_DRAG_TARGET,
                    ..
                } => state.drag_starts += 1,
                AppEvent::Drag {
                    node: NODE_DRAG_TARGET,
                    ..
                } => state.drags += 1,
                AppEvent::DragEnd {
                    node: NODE_DRAG_TARGET,
                    button,
                    ..
                } => state.drag_ends.push(button),
                AppEvent::ContextMenu {
                    node: NODE_DRAG_TARGET,
                    ..
                } => state.context_menus += 1,
                _ => {}
            }
            EventOutcome::new(DirtyFlags::NONE)
        }

        fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
            view.container(
                NODE_DRAG_TARGET,
                RawNodeDesc::container()
                    .size(300.0, 300.0)
                    .flags(NodeFlags::INTERACTIVE),
                |_| {},
            );
        }
    }

    let mut rt = AppRuntime::new(RightDragApp, Vec2::new(640.0, 480.0));
    let _ = rt.frame(0.016);

    // 1. Short right press below drag slop triggers ContextMenu and zero drag events.
    rt.pointer_down(Vec2::new(50.0, 50.0), PointerButton::Right);
    rt.pointer_move(Vec2::new(51.0, 50.0));
    rt.pointer_up(Vec2::new(51.0, 50.0), PointerButton::Right);

    assert_eq!(rt.state().context_menus, 1);
    assert_eq!(rt.state().drag_starts, 0);
    assert_eq!(rt.state().drags, 0);
    assert!(rt.state().drag_ends.is_empty());

    // 2. Right drag past drag slop emits DragStart -> Drag -> DragEnd and suppresses ContextMenu.
    rt.pointer_down(Vec2::new(50.0, 50.0), PointerButton::Right);
    rt.pointer_move(Vec2::new(80.0, 50.0));
    rt.pointer_move(Vec2::new(110.0, 70.0));
    rt.pointer_up(Vec2::new(110.0, 70.0), PointerButton::Right);

    assert_eq!(rt.state().drag_starts, 1);
    assert_eq!(rt.state().drags, 1);
    assert_eq!(rt.state().drag_ends, vec![PointerButton::Right]);
    assert_eq!(
        rt.state().context_menus,
        1,
        "right-button drag must not trigger a context menu on release"
    );
}
