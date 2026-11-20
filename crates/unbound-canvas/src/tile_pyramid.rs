//! 512+4px Halo 大图瓦片金字塔纯 CPU 求解器（ADR-0020 / `GAP-UI-006`）。
//!
//! 负责将十亿像素级图像按 512×512 有效内容 + 四周 4px Halo（物理 520×520）拆分为多级 LOD 瓦片，
//! 在给定视口与缩放比下输出无接缝 UV 裁切的绘制指令；当目标 LOD 瓦片尚未上传时，自动向上查找
//! 已驻留的粗级祖先瓦片裁剪子 UV 占位，并返回缺失瓦片列表供调度器异步解码。

use unbound_math::Rect;

/// 单张瓦片的有效内容边长（像素）。
pub const TILE_CONTENT_SIZE: u32 = 512;
/// 单张瓦片四周防双线性采样渗色的光晕边距（像素）。
pub const TILE_HALO_PX: u32 = 4;
/// 单张瓦片实际上传至 GPU 的物理边长（`512 + 4 * 2 = 520`）。
pub const TILE_PHYSICAL_SIZE: u32 = TILE_CONTENT_SIZE + TILE_HALO_PX * 2;

/// 瓦片金字塔唯一寻址键。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    /// 所属大图标识。
    pub image_id: u64,
    /// 金字塔细节层级（`0` 为原始 1:1 最高清，`lod` 越大越粗，缩放比例为 `2^lod`）。
    pub lod: u8,
    /// 当前层级下的水平瓦片下标。
    pub tx: u32,
    /// 当前层级下的垂直瓦片下标。
    pub ty: u32,
}

/// 大图金字塔规格描述。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TilePyramidSpec {
    /// 所属大图标识。
    pub image_id: u64,
    /// 原始图像全尺寸宽度（像素）。
    pub full_width: u32,
    /// 原始图像全尺寸高度（像素）。
    pub full_height: u32,
    /// 最大粗糙层级（含）。
    pub max_lod: u8,
}

impl TilePyramidSpec {
    /// 按图像全尺寸构造规格，并自动推导覆盖至单瓦片所需的 `max_lod`。
    #[must_use]
    pub fn new(image_id: u64, full_width: u32, full_height: u32) -> Self {
        let longest = full_width.max(full_height).max(1);
        let mut lod = 0u8;
        let mut span = TILE_CONTENT_SIZE;
        while span < longest && lod < 16 {
            span = span.saturating_mul(2);
            lod += 1;
        }
        Self {
            image_id,
            full_width,
            full_height,
            max_lod: lod,
        }
    }

    /// 计算指定 `lod` 层级下的图像有效像素尺寸。
    #[must_use]
    pub fn lod_dimensions(&self, lod: u8) -> (u32, u32) {
        let shift = u32::from(lod.min(self.max_lod));
        let scale = 1u32.checked_shl(shift).unwrap_or(u32::MAX);
        let w = self.full_width.div_ceil(scale).max(1);
        let h = self.full_height.div_ceil(scale).max(1);
        (w, h)
    }

    /// 计算指定 `lod` 层级下的瓦片网格行列数 `(cols, rows)`。
    #[must_use]
    pub fn grid_size(&self, lod: u8) -> (u32, u32) {
        let (w, h) = self.lod_dimensions(lod);
        (
            w.div_ceil(TILE_CONTENT_SIZE).max(1),
            h.div_ceil(TILE_CONTENT_SIZE).max(1),
        )
    }
}

/// 单条可供渲染层消费的瓦片绘制指令。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TileDrawCommand {
    /// 瓦片在图像世界坐标系（原图像素空间）中的覆盖矩形。
    pub world_rect: Rect,
    /// 已驻留资产的 `AssetId` 裸值。
    pub asset_id: u64,
    /// 裁去 4px Halo 后的归一化采样 UV 矩形。
    pub uv_rect: Rect,
    /// 是否由更粗的祖先 LOD 瓦片临时回退占位。
    pub is_fallback: bool,
}

/// 一帧可见瓦片求解结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TilePlan {
    /// 已可直接提交渲染的瓦片绘制列表（含粗级回退项）。
    pub draw_commands: Vec<TileDrawCommand>,
    /// 目标 LOD 缺失、需要上游解码调度器异步加载的瓦片键列表。
    pub missing_tiles: Vec<TileKey>,
}

/// 计算物理 520×520（含 4px Halo）瓦片中有效像素 `(valid_w, valid_h)` 对应的归一化 UV 矩形。
#[must_use]
pub fn tile_uv_rect(valid_width: u32, valid_height: u32) -> Rect {
    let w = valid_width.clamp(1, TILE_CONTENT_SIZE) as f32;
    let h = valid_height.clamp(1, TILE_CONTENT_SIZE) as f32;
    let phys = TILE_PHYSICAL_SIZE as f32;
    let halo = TILE_HALO_PX as f32;
    Rect::new(halo / phys, halo / phys, w / phys, h / phys)
}

/// 根据当前缩放倍率（`1.0` 表示 100%，`0.25` 表示缩小至 1/4）选择目标 LOD 层级。
#[must_use]
pub fn select_lod(zoom: f32, max_lod: u8) -> u8 {
    if !zoom.is_finite() || zoom >= 1.0 {
        return 0;
    }
    let inv = (1.0 / zoom.max(1e-4)).log2().floor();
    inv.clamp(0.0, f32::from(max_lod)) as u8
}

/// 求解给定世界坐标视口内的可见瓦片绘制计划，并为缺失瓦片自动计算粗级祖先回退子 UV。
#[must_use]
pub fn plan_visible_tiles(
    spec: &TilePyramidSpec,
    viewport_world: Rect,
    zoom: f32,
    resident: &dyn Fn(TileKey) -> Option<u64>,
) -> TilePlan {
    if spec.full_width == 0
        || spec.full_height == 0
        || viewport_world.width() <= 0.0
        || viewport_world.height() <= 0.0
    {
        return TilePlan::default();
    }

    let target_lod = select_lod(zoom, spec.max_lod);
    let scale = 1u32.checked_shl(u32::from(target_lod)).unwrap_or(u32::MAX) as f32;
    let tile_world_span = TILE_CONTENT_SIZE as f32 * scale;
    let (cols, rows) = spec.grid_size(target_lod);
    let (lod_w, lod_h) = spec.lod_dimensions(target_lod);

    let min_tx = (viewport_world.origin.x / tile_world_span).floor().max(0.0) as u32;
    let min_ty = (viewport_world.origin.y / tile_world_span).floor().max(0.0) as u32;
    let max_tx = ((viewport_world.max().x / tile_world_span).ceil().max(0.0) as u32).min(cols);
    let max_ty = ((viewport_world.max().y / tile_world_span).ceil().max(0.0) as u32).min(rows);

    let mut plan = TilePlan::default();

    for ty in min_ty..max_ty {
        for tx in min_tx..max_tx {
            let key = TileKey {
                image_id: spec.image_id,
                lod: target_lod,
                tx,
                ty,
            };
            let valid_w = lod_w
                .saturating_sub(tx.saturating_mul(TILE_CONTENT_SIZE))
                .min(TILE_CONTENT_SIZE);
            let valid_h = lod_h
                .saturating_sub(ty.saturating_mul(TILE_CONTENT_SIZE))
                .min(TILE_CONTENT_SIZE);
            let world_x = tx as f32 * tile_world_span;
            let world_y = ty as f32 * tile_world_span;
            let world_w = (valid_w as f32 * scale).min(spec.full_width as f32 - world_x);
            let world_h = (valid_h as f32 * scale).min(spec.full_height as f32 - world_y);
            let world_rect = Rect::new(world_x, world_y, world_w.max(1.0), world_h.max(1.0));

            if let Some(asset_id) = resident(key) {
                plan.draw_commands.push(TileDrawCommand {
                    world_rect,
                    asset_id,
                    uv_rect: tile_uv_rect(valid_w, valid_h),
                    is_fallback: false,
                });
                continue;
            }

            plan.missing_tiles.push(key);

            // 目标瓦片未就绪：向上逐级查找最近已驻留祖先瓦片并裁切对应子 UV 区域。
            if let Some(fallback) =
                find_coarse_fallback(spec, target_lod, tx, ty, world_rect, resident)
            {
                plan.draw_commands.push(fallback);
            }
        }
    }

    plan
}

fn find_coarse_fallback(
    spec: &TilePyramidSpec,
    target_lod: u8,
    tx: u32,
    ty: u32,
    world_rect: Rect,
    resident: &dyn Fn(TileKey) -> Option<u64>,
) -> Option<TileDrawCommand> {
    for ancestor_lod in (target_lod + 1)..=spec.max_lod {
        let shift = u32::from(ancestor_lod - target_lod);
        let atx = tx >> shift;
        let aty = ty >> shift;
        let ancestor_key = TileKey {
            image_id: spec.image_id,
            lod: ancestor_lod,
            tx: atx,
            ty: aty,
        };
        let Some(asset_id) = resident(ancestor_key) else {
            continue;
        };

        let ancestor_scale = 1u32
            .checked_shl(u32::from(ancestor_lod))
            .unwrap_or(u32::MAX) as f32;
        let ancestor_span = TILE_CONTENT_SIZE as f32 * ancestor_scale;
        let ancestor_origin_x = atx as f32 * ancestor_span;
        let ancestor_origin_y = aty as f32 * ancestor_span;

        let rel_x = ((world_rect.origin.x - ancestor_origin_x) / ancestor_span).clamp(0.0, 1.0);
        let rel_y = ((world_rect.origin.y - ancestor_origin_y) / ancestor_span).clamp(0.0, 1.0);
        let rel_w = (world_rect.width() / ancestor_span).clamp(0.0, 1.0 - rel_x);
        let rel_h = (world_rect.height() / ancestor_span).clamp(0.0, 1.0 - rel_y);

        let full_uv = tile_uv_rect(TILE_CONTENT_SIZE, TILE_CONTENT_SIZE);
        let uv_rect = Rect::new(
            full_uv.origin.x + full_uv.width() * rel_x,
            full_uv.origin.y + full_uv.height() * rel_y,
            full_uv.width() * rel_w,
            full_uv.height() * rel_h,
        );
        return Some(TileDrawCommand {
            world_rect,
            asset_id,
            uv_rect,
            is_fallback: true,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_tile_uv_crops_four_pixel_halo_on_all_sides() {
        let uv = tile_uv_rect(TILE_CONTENT_SIZE, TILE_CONTENT_SIZE);
        let expected_start = 4.0 / 520.0;
        let expected_span = 512.0 / 520.0;
        assert!((uv.origin.x - expected_start).abs() < 1e-6);
        assert!((uv.origin.y - expected_start).abs() < 1e-6);
        assert!((uv.width() - expected_span).abs() < 1e-6);
        assert!((uv.height() - expected_span).abs() < 1e-6);
    }

    #[test]
    fn select_lod_tracks_power_of_two_zoom_levels() {
        assert_eq!(select_lod(2.0, 4), 0);
        assert_eq!(select_lod(1.0, 4), 0);
        assert_eq!(select_lod(0.5, 4), 1);
        assert_eq!(select_lod(0.25, 4), 2);
        assert_eq!(select_lod(0.01, 4), 4);
    }

    #[test]
    fn missing_target_tile_falls_back_to_resident_coarse_ancestor() {
        let spec = TilePyramidSpec::new(7, 1024, 1024);
        assert_eq!(spec.max_lod, 1);

        // 仅驻留最粗级 (lod=1, tx=0, ty=0) 的整图概览瓦片，asset_id=99
        let plan = plan_visible_tiles(&spec, Rect::new(0.0, 0.0, 1024.0, 1024.0), 1.0, &|k| {
            if k.lod == 1 && k.tx == 0 && k.ty == 0 {
                Some(99)
            } else {
                None
            }
        });

        assert_eq!(plan.missing_tiles.len(), 4);
        assert_eq!(plan.draw_commands.len(), 4);
        assert!(plan
            .draw_commands
            .iter()
            .all(|c| c.is_fallback && c.asset_id == 99));
        // 每个 1/4 子瓦片的回退 UV 宽应等于完整内容 UV 的一半
        let half_span = (512.0 / 520.0) * 0.5;
        assert!((plan.draw_commands[0].uv_rect.width() - half_span).abs() < 1e-5);
    }
}
