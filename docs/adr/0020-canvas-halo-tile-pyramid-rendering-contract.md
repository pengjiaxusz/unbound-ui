# ADR-0020：无限画布 512+4px Halo 瓦片金字塔渲染契约

- **状态**：已采纳（对应 `GAP-UI-006`，依赖 ADR-0019）
- **影响 crate**：`unbound-canvas`、`unbound-widgets`、`unbound-app`

## 背景

`crates/unbound-canvas` 当前已具备 `LooseQuadtree`、吸附求解与框选模型，但缺少超大图像（十亿像素级）的分层瓦片金字塔抽象。直接整图上传会超出 GPU 最大纹理边长并耗尽显存，而普通 512×512 拼接在双线性过滤缩放时会在瓦片接缝处产生黑边/渗色伪影。

## 决策

1. **纯 CPU 瓦片坐标与 LOD 求解留在 `unbound-canvas`**：
   - 新增 `unbound_canvas::tile_pyramid` 模块，定义常量 `TILE_CONTENT_SIZE = 512`、`TILE_HALO_PX = 4`、`TILE_PHYSICAL_SIZE = 520`；
   - 定义 `TileKey { image_id: u64, lod: u8, tx: u32, ty: u32 }` 与 `TileDrawCommand { world_rect: Rect, asset_id: u64, uv_rect: Rect, is_fallback: bool }`；
   - 有效采样 UV 固定向内缩进 4px Halo：`[4.0 / 520.0, 516.0 / 520.0]`（边缘不满 512px 的残片按实际有效像素线性映射）。
2. **粗级 LOD 回退（Coarse Fallback）**：
   - 给定当前视口、缩放比与已驻留 `TileKey -> AssetId` 映射表，求解器先计算目标 LOD 的可见瓦片集合；
   - 若某目标瓦片尚未上传完成，自动向上查找最近的已驻留祖先 LOD 瓦片并裁剪其对应子 UV 区域作为即时占位，同时返回缺失的 `Vec<TileKey>` 列表供应用侧调度解码。
3. **零平台依赖与无头可测**：
   - `TilePyramid` 求解器不依赖 Vulkan/Win32，可 100% 在 `just test` 中完成 UV 数学、接缝防渗色与粗级回退单测；
   - `unbound-widgets` / `ViewBuilder` 提供 `canvas_tile_layer` 发射接口，消费 `TileDrawCommand` 列表并复用 ADR-0019 的 `AssetId` 渲染路径。

## 后果

- 下游资产应用 无限画布可直接把现有解码调度器产出的 520×520 RGBA8 瓦片喂给 `AssetHub::upload_rgba8`，再由 `TilePyramid` 驱动无接缝深缩放绘制。

## 实现修订（2026-11-21）

交接补丁集落地后，瓦片金字塔的公开 API 与决策稿不一致；以代码为准的对照如下。

| 决策稿写法 | 实际 API（`unbound-canvas::tile_pyramid`） |
| :--- | :--- |
| `TILE_CONTENT_SIZE` / `TILE_HALO_PX` / `TILE_PHYSICAL_SIZE` | `DEFAULT_TILE_SIZE` / `DEFAULT_TILE_HALO_PX`（物理边长 = 边长 + 2 × Halo） |
| `TileKey { image_id, lod, tx, ty }` | `TileKey { level, x, y }`，`TileKey::parent` 向上回溯 |
| `TilePyramidSpec::new(image_id, w, h)` + `lod_dimensions` / `grid_size` | `TilePyramidSpec::new(w, h, tile_size)`（含 `with_default_tile_size`）+ `level_dimensions` / `max_level` |
| `TileDrawCommand` + `plan_visible_tiles` | `ResolvedTileDraw` + `TilePyramidCache` / `TileSchedulePlan`（可见区优先、预取环与 LRU 淘汰队列同源输出） |
