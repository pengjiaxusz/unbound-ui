//! Native BC7 压缩纹理与动态多页图集全链路无头集成回归测试（Issue #246 / #268~#271）。
//!
//! 覆盖以下 6 大验收维度：
//! 1. 大小分流（`<= 512px` 多页图集 vs `> 512px` 独立 Bindless 外部槽）；
//! 2. 帧级上传节拍闸门、排队 `RoundedRect` 占位与离屏 `cancel_compressed_upload`；
//! 3. 1 B/px 真实显存预算、可见项 `touch` 保护、离屏 LRU 驱逐、冷页回收与重提恢复；
//! 4. `recover_gpu`（`invalidate_all_bc7_pages`）全量失效占位与无缝恢复；
//! 5. 无头 / 非 BC7 透明安全降级与原生 BC7 能力槽位回写；
//! 6. 能力边界裁决：4×4 非法载荷拒收、同页 `sample_uv` 零重叠、动画中间帧零图集重排。

use unbound_app::{
    AppContext, AppEvent, AppRuntime, Application, AtlasPageId, Bc7GpuOp, CompressedTextureBackend,
    CompressedTextureCapabilities, CompressedTextureError, CompressedTexturePayload, DirtyFlags,
    EventOutcome, UploadPacingBudget, ViewBuilder,
};
use unbound_asset::{bc7_solid_mode6_block, AssetId, BC7_BLOCK_BYTES, BC7_BLOCK_DIM};
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType};
use unbound_math::{Rect, Vec2};
use unbound_widgets::{ImageDesc, ObjectFit};

/// 构造指定尺寸与颜色的合法 BC7 Mode 6 纯色压缩纹理载荷（1 B/px）。
fn make_bc7_payload(width: u32, height: u32, color: [u8; 4]) -> CompressedTexturePayload {
    let block = bc7_solid_mode6_block(color[0], color[1], color[2], color[3]);
    let block_count = ((width / BC7_BLOCK_DIM) * (height / BC7_BLOCK_DIM)) as usize;
    let mut bytes = Vec::with_capacity(block_count * BC7_BLOCK_BYTES);
    for _ in 0..block_count {
        bytes.extend_from_slice(&block);
    }
    CompressedTexturePayload::bc7_unorm(width, height, bytes).expect("valid BC7 payload")
}

/// 视图中单个待绘制的 BC7 图像项描述。
#[derive(Clone, Debug)]
struct GalleryItem {
    key: u64,
    asset: AssetId,
    rect: Rect,
    natural_size: Vec2,
    sample_uv: Option<Rect>,
}

/// 用于无头测试的极简 BC7 图库应用状态。
#[derive(Default)]
struct GalleryState {
    items: Vec<GalleryItem>,
}

/// 用于驱动 `AppRuntime::frame` 的无头 BC7 图库应用。
#[derive(Default)]
struct GalleryApp;

impl Application for GalleryApp {
    type State = GalleryState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        GalleryState::default()
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
        for item in &state.items {
            let mut desc = ImageDesc::new(item.rect, item.natural_size, 0).with_asset(item.asset);
            desc.fit = ObjectFit::Fill;
            if let Some(uv) = item.sample_uv {
                desc = desc.with_sample_uv(uv);
            }
            view.image(item.key, desc);
        }
    }
}

fn new_gallery_runtime() -> AppRuntime<GalleryApp> {
    AppRuntime::new(GalleryApp, Vec2::new(1024.0, 768.0))
}

fn image_and_placeholder_primitives(
    rt: &AppRuntime<GalleryApp>,
) -> Vec<(GpuPrimitiveInstance, Option<GpuPayload>)> {
    let scene = rt.scene();
    scene
        .primitives
        .iter()
        .copied()
        .map(|prim| {
            let payload = if prim.prim_type() == PrimType::Image {
                scene.payloads.get(prim.extra_payload_idx as usize).copied()
            } else {
                None
            };
            (prim, payload)
        })
        .collect()
}

#[test]
fn size_routing_splits_atlas_and_standalone_slots() {
    let mut rt = new_gallery_runtime();
    rt.asset_hub_mut()
        .set_bc7_upload_pacing(UploadPacingBudget::unlimited());

    let small_id = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(512, 256, [200, 100, 50, 255]))
        .expect("submit small BC7");
    let large_id = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(516, 256, [50, 150, 220, 255]))
        .expect("submit large BC7");

    rt.state_mut().items = vec![
        GalleryItem {
            key: 100,
            asset: small_id,
            rect: Rect::new(10.0, 10.0, 128.0, 64.0),
            natural_size: Vec2::new(512.0, 256.0),
            sample_uv: None,
        },
        GalleryItem {
            key: 101,
            asset: large_id,
            rect: Rect::new(150.0, 10.0, 258.0, 128.0),
            natural_size: Vec2::new(516.0, 256.0),
            sample_uv: None,
        },
    ];
    rt.invalidate();

    let report = rt.frame(0.0);
    assert_eq!(
        report.bc7_active_pages, 1,
        "only <= 512px allocates atlas page"
    );
    assert_eq!(report.bc7_uploads_this_frame, 2);
    assert_eq!(report.bc7_pending_uploads, 0);
    assert_eq!(
        report.bc7_vram_bytes,
        (512 * 256 + 516 * 256) as u64,
        "VRAM accounting must use exact 1 B/px"
    );
    assert!(!rt.asset_hub().is_bc7_standalone(small_id));
    assert!(rt.asset_hub().is_bc7_standalone(large_id));

    assert!(
        report
            .bc7_ops
            .iter()
            .any(|op| matches!(op, Bc7GpuOp::CreateAtlasPage { .. })),
        "small texture must emit CreateAtlasPage"
    );
    assert!(
        report.bc7_ops.iter().any(
            |op| matches!(op, Bc7GpuOp::UploadStandalone { asset_id, .. } if *asset_id == large_id)
        ),
        "large texture must emit UploadStandalone"
    );

    let prims = image_and_placeholder_primitives(&rt);
    assert_eq!(prims.len(), 2);
    assert_eq!(prims[0].0.prim_type(), PrimType::Image);
    assert_eq!(prims[1].0.prim_type(), PrimType::Image);
    assert_ne!(
        prims[0].0.border_token_or_tex, prims[1].0.border_token_or_tex,
        "atlas page and standalone slot must not collide"
    );
}

#[test]
fn upload_pacing_gates_per_frame_renders_placeholders_and_supports_cancel() {
    let mut rt = new_gallery_runtime();
    // 严格限制每帧最多上传 1 张纹理。
    rt.asset_hub_mut()
        .set_bc7_upload_pacing(UploadPacingBudget::new(1, 256 * 1024));

    let id1 = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(64, 64, [255, 0, 0, 255]))
        .expect("submit id1");
    let id2 = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(64, 64, [0, 255, 0, 255]))
        .expect("submit id2");
    let id3 = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(64, 64, [0, 0, 255, 255]))
        .expect("submit id3");

    rt.state_mut().items = vec![
        GalleryItem {
            key: 201,
            asset: id1,
            rect: Rect::new(0.0, 0.0, 64.0, 64.0),
            natural_size: Vec2::new(64.0, 64.0),
            sample_uv: None,
        },
        GalleryItem {
            key: 202,
            asset: id2,
            rect: Rect::new(70.0, 0.0, 64.0, 64.0),
            natural_size: Vec2::new(64.0, 64.0),
            sample_uv: None,
        },
        GalleryItem {
            key: 203,
            asset: id3,
            rect: Rect::new(140.0, 0.0, 64.0, 64.0),
            natural_size: Vec2::new(64.0, 64.0),
            sample_uv: None,
        },
    ];
    rt.invalidate();

    // 第 1 帧：仅 id1 出队上传，id2 与 id3 保持排队占位。
    let r1 = rt.frame(0.0);
    assert_eq!(r1.bc7_uploads_this_frame, 1);
    assert_eq!(r1.bc7_pending_uploads, 2);
    let prims1 = image_and_placeholder_primitives(&rt);
    assert_eq!(prims1.len(), 3);
    assert_eq!(prims1[0].0.prim_type(), PrimType::Image);
    assert_eq!(prims1[1].0.prim_type(), PrimType::RoundedRect);
    assert_eq!(prims1[2].0.prim_type(), PrimType::RoundedRect);

    // 取消仍在队列中的 id3。
    assert!(rt.asset_hub_mut().cancel_compressed_upload(id3));
    assert!(!rt.asset_hub_mut().cancel_compressed_upload(id3));
    assert_eq!(rt.asset_hub().bc7_pending_uploads(), 1);

    // 第 2 帧：id2 顺延上传变为 Image，已取消的 id3 仍保持 RoundedRect 占位。
    let r2 = rt.frame(0.0);
    assert_eq!(r2.bc7_uploads_this_frame, 1);
    assert_eq!(r2.bc7_pending_uploads, 0);
    let prims2 = image_and_placeholder_primitives(&rt);
    assert_eq!(prims2.len(), 3);
    assert_eq!(prims2[0].0.prim_type(), PrimType::Image);
    assert_eq!(prims2[1].0.prim_type(), PrimType::Image);
    assert_eq!(prims2[2].0.prim_type(), PrimType::RoundedRect);
}

#[test]
fn lru_eviction_enforces_1b_per_px_budget_protects_visible_and_reclaims_cold_page() {
    let mut rt = new_gallery_runtime();
    rt.asset_hub_mut()
        .set_bc7_upload_pacing(UploadPacingBudget::unlimited());

    // 先提交 16 张 512×512 填满 Page 0（单页 2048×2048 恰好容纳 4×4 = 16 张）。
    let mut page0_ids = Vec::new();
    for idx in 0..16u8 {
        let id = rt
            .asset_hub_mut()
            .submit_compressed_bc7(make_bc7_payload(512, 512, [idx * 10, 80, 160, 255]))
            .expect("submit page0 item");
        page0_ids.push(id);
    }
    let r_seed = rt.frame(0.0);
    assert_eq!(r_seed.bc7_active_pages, 1);
    assert_eq!(r_seed.bc7_vram_bytes, 16 * 512 * 512);

    // 第 2 帧：提交第 17 张 512×512（开辟 Page 1），并在视图中仅渲染这张可见项，
    // 同时将显存预算收紧到精确 1 B/px 的 1 张 512×512（262,144 B，而非 RGBA8 的 1,048,576 B）。
    let visible_id = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(512, 512, [240, 200, 40, 255]))
        .expect("submit visible page1 item");
    let one_tile_bytes = (512 * 512) as u64;
    rt.asset_hub_mut().set_vram_budget_bytes(one_tile_bytes);
    rt.state_mut().items = vec![GalleryItem {
        key: 300,
        asset: visible_id,
        rect: Rect::new(0.0, 0.0, 256.0, 256.0),
        natural_size: Vec2::new(512.0, 512.0),
        sample_uv: None,
    }];
    rt.invalidate();

    let r_evict = rt.frame(0.0);
    assert_eq!(
        r_evict.bc7_eviction_count, 16,
        "all 16 offscreen page-0 items must be evicted"
    );
    assert_eq!(
        r_evict.bc7_vram_bytes, one_tile_bytes,
        "VRAM must drop to exact 1 B/px budget"
    );
    assert_eq!(
        r_evict.bc7_active_pages, 1,
        "empty cold page 0 must be reclaimed immediately"
    );
    assert!(
        r_evict.bc7_ops.iter().any(
            |op| matches!(op, Bc7GpuOp::DestroyAtlasPage { page_id } if *page_id == AtlasPageId(0))
        ),
        "reclaimed page 0 must emit DestroyAtlasPage"
    );

    // 可见项受 touch 保护，仍正常渲染为 Image。
    let prims_visible = image_and_placeholder_primitives(&rt);
    assert_eq!(prims_visible.len(), 1);
    assert_eq!(prims_visible[0].0.prim_type(), PrimType::Image);

    // 将被驱逐项重新放回视图，验证回退为 RoundedRect 占位符。
    let evicted_id = page0_ids[0];
    rt.state_mut().items.push(GalleryItem {
        key: 301,
        asset: evicted_id,
        rect: Rect::new(280.0, 0.0, 128.0, 128.0),
        natural_size: Vec2::new(512.0, 512.0),
        sample_uv: None,
    });
    rt.invalidate();
    let _ = rt.frame(0.0);
    let prims_after_evict = image_and_placeholder_primitives(&rt);
    assert_eq!(prims_after_evict.len(), 2);
    assert_eq!(prims_after_evict[0].0.prim_type(), PrimType::Image);
    assert_eq!(prims_after_evict[1].0.prim_type(), PrimType::RoundedRect);

    // 放宽预算并重新提交被驱逐项，验证无缝恢复为 Image。
    rt.asset_hub_mut().set_vram_budget_bytes(one_tile_bytes * 4);
    rt.asset_hub_mut()
        .submit_compressed_bc7_for(evicted_id, make_bc7_payload(512, 512, [120, 180, 90, 255]))
        .expect("resubmit evicted asset");
    let r_restore = rt.frame(0.0);
    assert_eq!(r_restore.bc7_uploads_this_frame, 1);
    let prims_restored = image_and_placeholder_primitives(&rt);
    assert_eq!(prims_restored.len(), 2);
    assert_eq!(prims_restored[0].0.prim_type(), PrimType::Image);
    assert_eq!(prims_restored[1].0.prim_type(), PrimType::Image);
}

#[test]
fn recover_gpu_invalidation_falls_back_to_placeholder_and_recovers_seamlessly() {
    let mut rt = new_gallery_runtime();
    let id = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(128, 128, [80, 200, 120, 255]))
        .expect("submit BC7");
    rt.state_mut().items = vec![GalleryItem {
        key: 400,
        asset: id,
        rect: Rect::new(20.0, 20.0, 128.0, 128.0),
        natural_size: Vec2::new(128.0, 128.0),
        sample_uv: None,
    }];
    rt.invalidate();

    let r1 = rt.frame(0.0);
    assert_eq!(r1.bc7_active_pages, 1);
    assert_eq!(r1.bc7_vram_bytes, 128 * 128);
    assert_eq!(
        image_and_placeholder_primitives(&rt)[0].0.prim_type(),
        PrimType::Image
    );

    // 模拟 recover_gpu 触发全量 BC7 页失效。
    rt.asset_hub_mut().invalidate_all_bc7_pages();
    rt.invalidate();
    let r2 = rt.frame(0.0);
    assert_eq!(r2.bc7_active_pages, 0);
    assert_eq!(r2.bc7_vram_bytes, 0);
    assert_eq!(
        image_and_placeholder_primitives(&rt)[0].0.prim_type(),
        PrimType::RoundedRect,
        "invalidated BC7 asset must fall back to RoundedRect placeholder"
    );

    // 重新提交同一 AssetId，验证零悬空引用无缝恢复为 Image。
    rt.asset_hub_mut()
        .submit_compressed_bc7_for(id, make_bc7_payload(128, 128, [80, 200, 120, 255]))
        .expect("resubmit after recover_gpu");
    let r3 = rt.frame(0.0);
    assert_eq!(r3.bc7_active_pages, 1);
    assert_eq!(r3.bc7_vram_bytes, 128 * 128);
    assert_eq!(
        image_and_placeholder_primitives(&rt)[0].0.prim_type(),
        PrimType::Image
    );
}

#[test]
fn fallback_and_native_bc7_capabilities_behave_deterministically() {
    let mut rt = new_gallery_runtime();
    assert!(rt.asset_hub().bc7_fallback_active());

    let fallback_id = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(64, 64, [200, 60, 60, 255]))
        .expect("submit in fallback mode");
    rt.state_mut().items = vec![GalleryItem {
        key: 500,
        asset: fallback_id,
        rect: Rect::new(0.0, 0.0, 64.0, 64.0),
        natural_size: Vec2::new(64.0, 64.0),
        sample_uv: None,
    }];
    rt.invalidate();
    let r_fallback = rt.frame(0.0);
    assert!(r_fallback.bc7_fallback_active);
    assert_eq!(
        image_and_placeholder_primitives(&rt)[0].0.prim_type(),
        PrimType::Image
    );

    // 切换至原生 BC7 能力并清空旧页，验证未回写槽位前保持占位、回写后切换为 Image。
    rt.asset_hub_mut().invalidate_all_bc7_pages();
    rt.asset_hub_mut()
        .set_bc7_capabilities(CompressedTextureCapabilities {
            supports_bc7: true,
            active_backend: CompressedTextureBackend::NativeBc7,
        });
    assert!(!rt.asset_hub().bc7_fallback_active());

    rt.asset_hub_mut()
        .submit_compressed_bc7_for(fallback_id, make_bc7_payload(64, 64, [60, 200, 60, 255]))
        .expect("submit in native BC7 mode");
    let r_native_pending = rt.frame(0.0);
    assert!(!r_native_pending.bc7_fallback_active);
    let created_page = r_native_pending
        .bc7_ops
        .iter()
        .find_map(|op| match op {
            Bc7GpuOp::CreateAtlasPage { page_id } => Some(*page_id),
            _ => None,
        })
        .expect("native mode must emit CreateAtlasPage");
    assert_eq!(
        image_and_placeholder_primitives(&rt)[0].0.prim_type(),
        PrimType::RoundedRect,
        "native mode must stay placeholder before GPU slot commit"
    );

    // 模拟窗口运行器回写 Bindless 槽位 52。
    rt.asset_hub_mut()
        .commit_bc7_page_slot(created_page, Ok(52));
    let _ = rt.frame(0.0);
    let prim = image_and_placeholder_primitives(&rt)[0].0;
    assert_eq!(prim.prim_type(), PrimType::Image);
    assert_eq!(prim.border_token_or_tex, 52);
}

#[test]
fn capability_boundaries_reject_invalid_payloads_isolate_uvs_and_avoid_animation_repack() {
    // 1. 4×4 非法尺寸与错误字节长度在入口层即被拒收。
    assert_eq!(
        CompressedTexturePayload::bc7_unorm(0, 4, Vec::new()),
        Err(CompressedTextureError::ZeroDimension {
            width: 0,
            height: 4,
        })
    );
    assert_eq!(
        CompressedTexturePayload::bc7_unorm(5, 4, vec![0; 32]),
        Err(CompressedTextureError::UnalignedDimension {
            width: 5,
            height: 4,
            block_dim: 4,
        })
    );
    assert_eq!(
        CompressedTexturePayload::bc7_unorm(8, 8, vec![0; 48]),
        Err(CompressedTextureError::ByteLengthMismatch {
            expected: 64,
            actual: 48,
            width: 8,
            height: 8,
        })
    );

    // 2. 同页相邻条目的 sample_uv 显式裁剪子矩形互不重叠（零跨条目串色）。
    let mut rt = new_gallery_runtime();
    let id_a = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(128, 64, [255, 40, 40, 255]))
        .expect("submit item A");
    let id_b = rt
        .asset_hub_mut()
        .submit_compressed_bc7(make_bc7_payload(128, 64, [40, 40, 255, 255]))
        .expect("submit item B");

    rt.state_mut().items = vec![
        GalleryItem {
            key: 601,
            asset: id_a,
            rect: Rect::new(10.0, 10.0, 128.0, 64.0),
            natural_size: Vec2::new(128.0, 64.0),
            sample_uv: Some(Rect::new(0.1, 0.1, 0.8, 0.8)),
        },
        GalleryItem {
            key: 602,
            asset: id_b,
            rect: Rect::new(160.0, 10.0, 128.0, 64.0),
            natural_size: Vec2::new(128.0, 64.0),
            sample_uv: Some(Rect::new(0.1, 0.1, 0.8, 0.8)),
        },
    ];
    rt.invalidate();
    let r_pack = rt.frame(0.0);
    assert_eq!(r_pack.bc7_active_pages, 1);

    let prims_before = image_and_placeholder_primitives(&rt);
    assert_eq!(prims_before.len(), 2);
    let (prim_a, payload_a) = (prims_before[0].0, prims_before[0].1.expect("payload A"));
    let (prim_b, payload_b) = (prims_before[1].0, prims_before[1].1.expect("payload B"));
    assert_eq!(prim_a.prim_type(), PrimType::Image);
    assert_eq!(prim_b.prim_type(), PrimType::Image);
    assert_eq!(
        prim_a.border_token_or_tex, prim_b.border_token_or_tex,
        "both items should share the same atlas page"
    );

    let disjoint_x = payload_a.p1[0] <= payload_b.p0[0] || payload_b.p1[0] <= payload_a.p0[0];
    let disjoint_y = payload_a.p1[1] <= payload_b.p0[1] || payload_b.p1[1] <= payload_a.p0[1];
    assert!(
        disjoint_x || disjoint_y,
        "UV sub-rectangles on the same atlas page must not overlap"
    );

    // 3. 缩放/滚动动画中间帧仅改变图元显示几何 rect，不触发图集重排或重传。
    rt.state_mut().items[0].rect = Rect::new(42.0, 35.0, 210.0, 105.0);
    rt.state_mut().items[1].rect = Rect::new(280.0, 35.0, 96.0, 48.0);
    rt.invalidate();
    let r_anim = rt.frame(0.016);
    assert_eq!(r_anim.bc7_uploads_this_frame, 0);
    assert!(
        r_anim.bc7_ops.is_empty(),
        "animation frame must not repack or re-upload atlas entries"
    );

    let prims_after = image_and_placeholder_primitives(&rt);
    let (prim_a_after, payload_a_after) =
        (prims_after[0].0, prims_after[0].1.expect("payload A after"));
    let (prim_b_after, payload_b_after) =
        (prims_after[1].0, prims_after[1].1.expect("payload B after"));
    assert_eq!(prim_a.size, [128.0, 64.0]);
    assert_eq!(prim_a_after.size, [210.0, 105.0]);
    assert_eq!(prim_b_after.size, [96.0, 48.0]);
    assert_eq!(prim_a_after.border_token_or_tex, prim_a.border_token_or_tex);
    assert_eq!(payload_a_after.p0, payload_a.p0);
    assert_eq!(payload_a_after.p1, payload_a.p1);
    assert_eq!(prim_b_after.border_token_or_tex, prim_b.border_token_or_tex);
    assert_eq!(payload_b_after.p0, payload_b.p0);
    assert_eq!(payload_b_after.p1, payload_b.p1);
}
