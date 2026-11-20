# ADR-0019：`AssetHub` RGBA8 原始像素动态上传与显式/LRU 回收

- **状态**：已采纳（对应 `GAP-UI-001`）
- **影响 crate**：`unbound-asset`、`unbound-gpu`、`unbound-app`

## 背景

`crates/unbound-app/src/assets.rs` 目前仅暴露 `request_png(Vec<u8>)` 与 `request_font(Vec<u8>)`，依赖内置 PNG 解码线程。而 下游资产应用 等媒体工作台已在应用侧完成并发解码（JPEG/WebP/AVIF/缩略图块缓存/画布 520px 瓦片），直接持有内存中的 RGBA8 像素缓冲区。若强行重新编码为 PNG 再交给 `AssetHub`，会带来不可接受的 CPU/内存浪费。

另一方面，`crates/unbound-gpu/src/atlas.rs` 已有 `AtlasUpload` 与 `PAGE_SIZE = 2048` 的 RGBA8 图集页，且 `crates/unbound-gpu/src/pipeline.rs` 已有 `ExternalTextureSlots` 与 `bind_external_texture` / `unbind_external_texture`。

## 决策

1. **新增零重新编码上传入口**：
   - 在 `AssetHub` 新增：
     ```rust
     pub fn upload_rgba8(
         &mut self,
         width: u32,
         height: u32,
         stride_bytes: u32,
         pixels: std::sync::Arc<[u8]>,
     ) -> Result<AssetId, AssetUploadError>;
     ```
   - 严格校验 `width > 0 && height > 0`、`stride_bytes >= width * 4` 及 `pixels.len() >= stride_bytes as usize * height as usize`，不满足则返回强类型 `AssetUploadError`。
2. **双路存储策略（统一 `AssetId` 寻址）**：
   - 尺寸 `<= 512 × 512` 且图集页有空位：打包进 RGBA8 `AtlasKind::Rgba8` 图集页，产出 `AtlasUpload` 走现有 `take_uploads()`；
   - 尺寸 `> 512 × 512` 或单张超页宽：分配 `ExternalTextureSlots` 槽位，产出独立外部纹理上传指令，避免大图打爆共享图集页。
3. **显式释放 + 预算制 LRU 回收**：
   - 新增 `pub fn release_asset(&mut self, id: AssetId) -> bool`：立即回收句柄与对应图集槽/外部纹理槽，并将旧图集页在空页或碎片率超阈值时整体重置复用；
   - 新增 `pub fn touch_asset(&mut self, id: AssetId, frame_index: u64)` 与 `pub fn evict_lru_to_budget(&mut self, max_bytes: usize) -> Vec<AssetId>`，保证大目录滚动或深缩放画布下显存占用有严格上界。

## 后果

- 下游资产应用 的文件浏览缩略图与单图查看器可直接零拷贝对接 `AssetHub`；
- 为 `ADR-0020`（瓦片金字塔渲染）提供统一的 `AssetId` 纹理驻留与回收底座。
