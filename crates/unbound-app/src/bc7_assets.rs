//! BC7 压缩纹理资产池、帧级上传节拍、真实 1 B/px 显存预算与离屏 LRU 驱逐状态机。
//!
//! 本模块独立于现有 RGBA8 `ShelfAtlas`，专为 Issue #246（#268 / #269 / #270 / #271）提供：
//!
//! * 按阈值分流：`<= 512px` 进入 [`TieredShelfMultiPageAllocator`] 多页动态图集，
//!   `> 512px` 路由至独立 Bindless 外部纹理槽；
//! * 帧级上传节拍闸门：基于 [`CompressedUploadQueue`] 与 [`UploadPacingBudget`] 逐帧排空，
//!   支持按 `AssetId` 去重覆写与离屏即时取消；
//! * 1 B/px 真实显存记账与离屏 LRU 驱逐：通过内部可变性记录当前可见集（`touch`），
//!   防止在屏项被误逐；整页清空后自动移除冷页并发射 [`Bc7GpuOp::DestroyAtlasPage`]；
//! * 设备丢失（`recover_gpu`）全量失效清空与无头 / 非 BC7 硬件透明降级语义。

use std::cell::RefCell;

use rustc_hash::{FxHashMap, FxHashSet};
use unbound_asset::{
    route_compressed_texture, AssetError, AssetId, AtlasAllocation, AtlasPageId,
    CompressedTexturePayload, CompressedTextureRoute, CompressedUploadQueue,
    TieredShelfMultiPageAllocator, UploadPacingBudget, DEFAULT_COMPRESSED_ATLAS_THRESHOLD,
};
use unbound_gpu::device::{CompressedTextureBackend, CompressedTextureCapabilities};
use unbound_gpu::pipeline::{EXTERNAL_TEXTURE_BASE, EXTERNAL_TEXTURE_SLOTS};
use unbound_math::{Rect, Vec2};
use unbound_widgets::ResolvedImage;

use crate::assets::AssetResidency;
use crate::DirtyFlags;

/// 默认 BC7 显存预算上限（128 MiB，按真实 1 B/px 记账）。
pub const DEFAULT_BC7_VRAM_BUDGET_BYTES: u64 = 128 * 1024 * 1024;

/// 无头 / 降级模拟模式下，图集页与独立大纹理在外部 Bindless 槽空间中的分界偏移。
const FALLBACK_PAGE_SLOT_COUNT: u32 = EXTERNAL_TEXTURE_SLOTS / 2;

/// 供窗口运行器在每帧渲染前消费的 BC7 压缩纹理 GPU 操作队列项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bc7GpuOp {
    /// 按需创建一张新的 `2048×2048` 动态 BC7 图集页并绑定 Bindless 槽位。
    CreateAtlasPage {
        /// 逻辑图集页标识。
        page_id: AtlasPageId,
    },
    /// 向指定动态 BC7 图集页的子矩形上传 4×4 块对齐的 BC7 压缩数据。
    UploadAtlasRegion {
        /// 目标逻辑图集页标识。
        page_id: AtlasPageId,
        /// 区域左上角 X 坐标（像素，4px 对齐）。
        x: u32,
        /// 区域左上角 Y 坐标（像素，4px 对齐）。
        y: u32,
        /// 区域宽度（像素，4px 对齐）。
        width: u32,
        /// 区域高度（像素，4px 对齐）。
        height: u32,
        /// 紧凑 BC7 压缩块字节（长度精确等于 `width * height`）。
        bytes: Vec<u8>,
    },
    /// 销毁已变空的冷 BC7 图集页并延迟归还其 Bindless 槽位。
    DestroyAtlasPage {
        /// 待销毁的逻辑图集页标识。
        page_id: AtlasPageId,
    },
    /// 为 `> 512px` 大图创建并异步上传独立 BC7 Bindless 外部纹理。
    UploadStandalone {
        /// 资产唯一标识。
        asset_id: AssetId,
        /// 纹理宽度（像素，4px 对齐）。
        width: u32,
        /// 纹理高度（像素，4px 对齐）。
        height: u32,
        /// 紧凑 BC7 压缩块字节（长度精确等于 `width * height`）。
        bytes: Vec<u8>,
    },
    /// 销毁独立 BC7 外部纹理并延迟归还其 Bindless 槽位。
    DestroyStandalone {
        /// 资产唯一标识。
        asset_id: AssetId,
        /// 绑定的外部 Bindless 槽位索引。
        slot: u32,
    },
}

/// 单个逻辑 BC7 图集页的 Bindless 槽位绑定状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Bc7PageSlotState {
    /// 等待窗口运行器在 GPU 侧创建并回写 Bindless 槽位。
    Pending,
    /// 已绑定到有效的外部 Bindless 槽位（`48..304`）。
    Ready(u32),
    /// GPU 创建或绑定失败。
    Failed,
}

/// 单条 BC7 资产当前的驻留形态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Bc7EntryStorage {
    /// 已进入节拍上传队列，等待后续帧出队上传。
    Queued,
    /// 已分配到多页动态图集的指定子区域。
    Atlas(AtlasAllocation),
    /// 已路由至独立 Bindless 外部槽。
    Standalone {
        /// 绑定的外部 Bindless 槽位（原生模式下由运行器异步回写）。
        slot: Option<u32>,
    },
    /// 已被 LRU 驱逐、主动取消或因 `recover_gpu` 失效（渲染回退为占位符）。
    Evicted,
    /// 分配或 GPU 上传失败。
    Failed,
}

/// 单条 BC7 资产的元数据与 LRU 时间戳。
#[derive(Clone, Debug)]
struct Bc7Entry {
    width: u32,
    height: u32,
    /// 当前实际占用的显存字节数（精确 1 B/px；未驻留时为 0）。
    vram_bytes: u64,
    storage: Bc7EntryStorage,
    last_used_frame: u64,
}

/// 嵌入 [`crate::AssetHub`] 内部的 BC7 压缩资产池状态机。
#[derive(Debug)]
pub(crate) struct Bc7AssetPool {
    capabilities: CompressedTextureCapabilities,
    vram_budget_bytes: u64,
    upload_pacing: UploadPacingBudget,
    atlas_threshold_px: u32,
    allocator: TieredShelfMultiPageAllocator,
    upload_queue: CompressedUploadQueue,
    entries: FxHashMap<AssetId, Bc7Entry>,
    page_slots: FxHashMap<AtlasPageId, Bc7PageSlotState>,
    pending_ops: Vec<Bc7GpuOp>,
    current_frame: u64,
    vram_bytes: u64,
    uploads_this_frame: usize,
    eviction_count: u64,
    dirty: bool,
    /// 本帧视图构建过程中通过 `resolve` 实际绘制的资产集合（早退帧沿用上一帧集合）。
    view_touched: RefCell<FxHashSet<AssetId>>,
    /// 外部调用 `AssetHub::touch` 显式声明保护的资产集合（每帧末 LRU 完成后清空）。
    explicit_touched: RefCell<FxHashSet<AssetId>>,
    /// 本帧刚刚从节拍队列出队上传的资产集合（防止同帧刚上传即被驱逐）。
    uploaded_this_frame: FxHashSet<AssetId>,
    /// 无头 / 降级模式下独立大纹理模拟槽位递增游标。
    next_fallback_standalone_offset: u32,
}

impl Default for Bc7AssetPool {
    fn default() -> Self {
        Self::new()
    }
}

impl Bc7AssetPool {
    /// 创建默认无头 / 降级配置的 BC7 压缩资产池。
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            capabilities: CompressedTextureCapabilities {
                supports_bc7: false,
                active_backend: CompressedTextureBackend::Rgba8Fallback,
            },
            vram_budget_bytes: DEFAULT_BC7_VRAM_BUDGET_BYTES,
            upload_pacing: UploadPacingBudget::default(),
            atlas_threshold_px: DEFAULT_COMPRESSED_ATLAS_THRESHOLD,
            allocator: TieredShelfMultiPageAllocator::new(),
            upload_queue: CompressedUploadQueue::default(),
            entries: FxHashMap::default(),
            page_slots: FxHashMap::default(),
            pending_ops: Vec::new(),
            current_frame: 0,
            vram_bytes: 0,
            uploads_this_frame: 0,
            eviction_count: 0,
            dirty: false,
            view_touched: RefCell::new(FxHashSet::default()),
            explicit_touched: RefCell::new(FxHashSet::default()),
            uploaded_this_frame: FxHashSet::default(),
            next_fallback_standalone_offset: 0,
        }
    }

    /// 同步当前设备的 BC7 压缩纹理硬件能力。
    pub(crate) fn set_capabilities(&mut self, caps: CompressedTextureCapabilities) {
        self.capabilities = caps;
    }

    /// 查询当前设备的 BC7 压缩纹理能力快照。
    #[must_use]
    pub(crate) const fn capabilities(&self) -> CompressedTextureCapabilities {
        self.capabilities
    }

    /// 设置 BC7 显存预算上限（字节，按真实 1 B/px 计）。
    pub(crate) fn set_vram_budget_bytes(&mut self, budget_bytes: u64) {
        self.vram_budget_bytes = budget_bytes;
        if self.vram_bytes > budget_bytes {
            self.dirty = true;
        }
    }

    /// 查询当前 BC7 显存预算上限（字节）。
    #[must_use]
    pub(crate) const fn vram_budget_bytes(&self) -> u64 {
        self.vram_budget_bytes
    }

    /// 设置帧级上传节拍预算。
    pub(crate) fn set_upload_pacing(&mut self, budget: UploadPacingBudget) {
        self.upload_pacing = budget;
    }

    /// 查询当前帧级上传节拍预算。
    #[must_use]
    pub(crate) const fn upload_pacing(&self) -> UploadPacingBudget {
        self.upload_pacing
    }

    /// 设置图集与独立外部槽的尺寸分流阈值（像素）。
    pub(crate) fn set_atlas_threshold(&mut self, threshold_px: u32) {
        self.atlas_threshold_px = threshold_px;
    }

    /// 为指定 `AssetId` 提交 BC7 压缩载荷并排入节拍上传队列。
    ///
    /// 若该 `AssetId` 已在队列中则就地去重覆写；若曾被驱逐或失效则重新进入排队状态。
    pub(crate) fn submit_for(
        &mut self,
        id: AssetId,
        payload: CompressedTexturePayload,
    ) -> Result<AssetId, AssetError> {
        let width = payload.width();
        let height = payload.height();
        self.upload_queue.enqueue(id, payload)?;

        match self.entries.get_mut(&id) {
            Some(entry) => {
                entry.width = width;
                entry.height = height;
                if !matches!(
                    entry.storage,
                    Bc7EntryStorage::Atlas(_) | Bc7EntryStorage::Standalone { .. }
                ) {
                    entry.storage = Bc7EntryStorage::Queued;
                }
            }
            None => {
                self.entries.insert(
                    id,
                    Bc7Entry {
                        width,
                        height,
                        vram_bytes: 0,
                        storage: Bc7EntryStorage::Queued,
                        last_used_frame: self.current_frame,
                    },
                );
            }
        }
        self.dirty = true;
        Ok(id)
    }

    /// 立即取消队列中尚未上传的压缩纹理任务。
    pub(crate) fn cancel_upload(&mut self, id: AssetId) -> bool {
        if !self.upload_queue.cancel(id) {
            return false;
        }
        if let Some(entry) = self.entries.get_mut(&id) {
            if entry.storage == Bc7EntryStorage::Queued {
                entry.storage = Bc7EntryStorage::Evicted;
            }
        }
        self.dirty = true;
        true
    }

    /// 显式标记资产在本帧活跃，防止被帧末 LRU 驱逐。
    pub(crate) fn touch_explicit(&self, id: AssetId) {
        self.explicit_touched.borrow_mut().insert(id);
    }

    /// 在视图构建开始前清空上一帧由 `resolve` 采集的可见集合。
    pub(crate) fn begin_view_pass(&mut self) {
        self.view_touched.borrow_mut().clear();
    }

    /// 帧初按上传节拍排空批次、执行图集/独立槽分流分配并生成 [`Bc7GpuOp`]。
    pub(crate) fn poll_frame(&mut self) -> DirtyFlags {
        self.current_frame = self.current_frame.wrapping_add(1).max(1);
        self.uploads_this_frame = 0;
        self.uploaded_this_frame.clear();

        let batch = self.upload_queue.drain_batch(self.upload_pacing);
        for item in batch.items {
            let id = item.id;
            let payload = item.payload;
            let width = payload.width();
            let height = payload.height();
            let entry_vram = payload.vram_bytes();

            // 若同 AssetId 之前已有驻留记录（覆写更新），先释放旧分配并扣减旧显存。
            self.release_resident_storage(id);

            match route_compressed_texture(width, height, self.atlas_threshold_px) {
                CompressedTextureRoute::AtlasPage => match self.allocator.allocate(width, height) {
                    Ok((alloc, created_page)) => {
                        if created_page {
                            self.pending_ops.push(Bc7GpuOp::CreateAtlasPage {
                                page_id: alloc.page,
                            });
                            let initial_state = if self.capabilities.supports_bc7 {
                                Bc7PageSlotState::Pending
                            } else {
                                Bc7PageSlotState::Ready(
                                    self.allocate_fallback_page_slot(alloc.page),
                                )
                            };
                            self.page_slots.insert(alloc.page, initial_state);
                        }
                        self.pending_ops.push(Bc7GpuOp::UploadAtlasRegion {
                            page_id: alloc.page,
                            x: alloc.x,
                            y: alloc.y,
                            width,
                            height,
                            bytes: payload.into_bytes(),
                        });
                        self.entries.insert(
                            id,
                            Bc7Entry {
                                width,
                                height,
                                vram_bytes: entry_vram,
                                storage: Bc7EntryStorage::Atlas(alloc),
                                last_used_frame: self.current_frame,
                            },
                        );
                        self.vram_bytes = self.vram_bytes.saturating_add(entry_vram);
                        self.uploaded_this_frame.insert(id);
                        self.uploads_this_frame += 1;
                        self.dirty = true;
                    }
                    Err(_) => {
                        self.entries.insert(
                            id,
                            Bc7Entry {
                                width,
                                height,
                                vram_bytes: 0,
                                storage: Bc7EntryStorage::Failed,
                                last_used_frame: self.current_frame,
                            },
                        );
                        self.dirty = true;
                    }
                },
                CompressedTextureRoute::ExternalStandalone => {
                    let initial_slot = if self.capabilities.supports_bc7 {
                        None
                    } else {
                        Some(self.allocate_fallback_standalone_slot())
                    };
                    self.pending_ops.push(Bc7GpuOp::UploadStandalone {
                        asset_id: id,
                        width,
                        height,
                        bytes: payload.into_bytes(),
                    });
                    self.entries.insert(
                        id,
                        Bc7Entry {
                            width,
                            height,
                            vram_bytes: entry_vram,
                            storage: Bc7EntryStorage::Standalone { slot: initial_slot },
                            last_used_frame: self.current_frame,
                        },
                    );
                    self.vram_bytes = self.vram_bytes.saturating_add(entry_vram);
                    self.uploaded_this_frame.insert(id);
                    self.uploads_this_frame += 1;
                    self.dirty = true;
                }
            }
        }

        if core::mem::take(&mut self.dirty) {
            DirtyFlags::PAINT
        } else {
            DirtyFlags::NONE
        }
    }

    /// 帧末执行显存预算检查与离屏 LRU 驱逐，并在整页变空时立即回收冷页。
    ///
    /// 返回本帧是否发生了驱逐（若发生驱逐则下一帧应重绘以更新占位表现）。
    pub(crate) fn finish_frame_lru(&mut self) -> bool {
        let mut protected = FxHashSet::default();
        protected.extend(self.view_touched.borrow().iter().copied());
        protected.extend(self.explicit_touched.borrow().iter().copied());
        protected.extend(self.uploaded_this_frame.iter().copied());
        self.explicit_touched.borrow_mut().clear();

        // 刷新本帧所有受保护驻留条目的最近使用帧号。
        for &id in &protected {
            if let Some(entry) = self.entries.get_mut(&id) {
                if matches!(
                    entry.storage,
                    Bc7EntryStorage::Atlas(_) | Bc7EntryStorage::Standalone { .. }
                ) {
                    entry.last_used_frame = self.current_frame;
                }
            }
        }

        if self.vram_bytes <= self.vram_budget_bytes {
            return false;
        }

        // 仅驱逐不在受保护集合中的离屏驻留条目，按 last_used_frame 从旧到新排序。
        let mut candidates: Vec<(u64, u32, AssetId)> = self
            .entries
            .iter()
            .filter_map(|(&id, entry)| {
                if protected.contains(&id) {
                    return None;
                }
                match entry.storage {
                    Bc7EntryStorage::Atlas(_) | Bc7EntryStorage::Standalone { .. } => {
                        Some((entry.last_used_frame, id.raw(), id))
                    }
                    Bc7EntryStorage::Queued
                    | Bc7EntryStorage::Evicted
                    | Bc7EntryStorage::Failed => None,
                }
            })
            .collect();
        candidates.sort_unstable();

        let mut evicted_any = false;
        for (_, _, id) in candidates {
            if self.vram_bytes <= self.vram_budget_bytes {
                break;
            }
            if self.release_resident_storage(id) {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.storage = Bc7EntryStorage::Evicted;
                    entry.vram_bytes = 0;
                }
                self.eviction_count = self.eviction_count.saturating_add(1);
                self.dirty = true;
                evicted_any = true;
            }
        }

        evicted_any
    }

    /// 释放单条已驻留 BC7 资产的图集区域或独立外部槽，并自动回收变空的冷页。
    fn release_resident_storage(&mut self, id: AssetId) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        let storage = entry.storage;
        let old_vram = entry.vram_bytes;
        match storage {
            Bc7EntryStorage::Atlas(alloc) => {
                entry.vram_bytes = 0;
                self.vram_bytes = self.vram_bytes.saturating_sub(old_vram);
                let _ = self.allocator.free(alloc);
                if self.allocator.is_page_empty(alloc.page) {
                    let _ = self.allocator.remove_empty_page(alloc.page);
                    self.page_slots.remove(&alloc.page);
                    self.pending_ops.push(Bc7GpuOp::DestroyAtlasPage {
                        page_id: alloc.page,
                    });
                }
                true
            }
            Bc7EntryStorage::Standalone { slot } => {
                entry.vram_bytes = 0;
                self.vram_bytes = self.vram_bytes.saturating_sub(old_vram);
                if let Some(slot) = slot {
                    self.pending_ops
                        .push(Bc7GpuOp::DestroyStandalone { asset_id: id, slot });
                }
                true
            }
            Bc7EntryStorage::Queued | Bc7EntryStorage::Evicted | Bc7EntryStorage::Failed => false,
        }
    }

    /// 取出并清空本帧累积的 BC7 GPU 操作列表。
    pub(crate) fn take_ops(&mut self) -> Vec<Bc7GpuOp> {
        core::mem::take(&mut self.pending_ops)
    }

    /// 由窗口运行器回写动态 BC7 图集页的 Bindless 槽位分配结果。
    pub(crate) fn commit_page_slot(&mut self, page_id: AtlasPageId, slot: Result<u32, String>) {
        let Some(state) = self.page_slots.get_mut(&page_id) else {
            return;
        };
        match slot {
            Ok(bound_slot) => {
                *state = Bc7PageSlotState::Ready(bound_slot);
            }
            Err(_) => {
                *state = Bc7PageSlotState::Failed;
            }
        }
        self.dirty = true;
    }

    /// 由窗口运行器回写独立 BC7 外部纹理的 Bindless 槽位分配结果。
    pub(crate) fn commit_standalone_slot(&mut self, asset_id: AssetId, slot: Result<u32, String>) {
        let Some(entry) = self.entries.get_mut(&asset_id) else {
            return;
        };
        if !matches!(entry.storage, Bc7EntryStorage::Standalone { .. }) {
            return;
        }
        match slot {
            Ok(bound_slot) => {
                entry.storage = Bc7EntryStorage::Standalone {
                    slot: Some(bound_slot),
                };
            }
            Err(_) => {
                self.vram_bytes = self.vram_bytes.saturating_sub(entry.vram_bytes);
                entry.vram_bytes = 0;
                entry.storage = Bc7EntryStorage::Failed;
            }
        }
        self.dirty = true;
    }

    /// 在 `recover_gpu` 或销毁 Vulkan 上下文时清空所有 BC7 图集页、独立槽与队列。
    ///
    /// 所有既有 BC7 条目回退为未就绪占位状态，以便后续帧重新提交时无缝恢复。
    pub(crate) fn invalidate_all_pages(&mut self) {
        self.upload_queue.clear();
        self.pending_ops.clear();
        self.allocator.clear();
        self.page_slots.clear();
        self.uploaded_this_frame.clear();
        self.view_touched.borrow_mut().clear();
        self.explicit_touched.borrow_mut().clear();
        self.vram_bytes = 0;
        self.uploads_this_frame = 0;
        for entry in self.entries.values_mut() {
            entry.storage = Bc7EntryStorage::Evicted;
            entry.vram_bytes = 0;
        }
        self.dirty = true;
    }

    /// 解析单个 BC7 资产以供渲染器采样；命中就绪条目时自动记入本帧可见保护集。
    #[must_use]
    pub(crate) fn resolve(&self, id: AssetId) -> Option<ResolvedImage> {
        let entry = self.entries.get(&id)?;
        match entry.storage {
            Bc7EntryStorage::Atlas(alloc) => {
                let Bc7PageSlotState::Ready(layer) = self.page_slots.get(&alloc.page).copied()?
                else {
                    return None;
                };
                self.view_touched.borrow_mut().insert(id);
                let [u0, v0, u1, v1] = alloc.uv_bounds();
                Some(ResolvedImage {
                    layer,
                    uv: Rect::new(u0, v0, u1 - u0, v1 - v0),
                    size: Vec2::new(alloc.width as f32, alloc.height as f32),
                })
            }
            Bc7EntryStorage::Standalone { slot: Some(layer) } => {
                self.view_touched.borrow_mut().insert(id);
                Some(ResolvedImage {
                    layer,
                    uv: Rect::new(0.0, 0.0, 1.0, 1.0),
                    size: Vec2::new(entry.width as f32, entry.height as f32),
                })
            }
            Bc7EntryStorage::Queued
            | Bc7EntryStorage::Standalone { slot: None }
            | Bc7EntryStorage::Evicted
            | Bc7EntryStorage::Failed => None,
        }
    }

    /// 查询单个 BC7 资产的生命周期状态（供 `AssetHub::residency` 统一查询）。
    #[must_use]
    pub(crate) fn residency(&self, id: AssetId) -> Option<AssetResidency> {
        let entry = self.entries.get(&id)?;
        match entry.storage {
            Bc7EntryStorage::Queued | Bc7EntryStorage::Evicted => Some(AssetResidency::Loading),
            Bc7EntryStorage::Atlas(alloc) => match self.page_slots.get(&alloc.page).copied() {
                Some(Bc7PageSlotState::Ready(layer)) => {
                    let [u0, v0, u1, v1] = alloc.uv_bounds();
                    Some(AssetResidency::Ready {
                        layer,
                        uv: Rect::new(u0, v0, u1 - u0, v1 - v0),
                        size: Vec2::new(alloc.width as f32, alloc.height as f32),
                    })
                }
                Some(Bc7PageSlotState::Pending) | None => Some(AssetResidency::Loading),
                Some(Bc7PageSlotState::Failed) => Some(AssetResidency::Failed),
            },
            Bc7EntryStorage::Standalone { slot: Some(layer) } => Some(AssetResidency::Ready {
                layer,
                uv: Rect::new(0.0, 0.0, 1.0, 1.0),
                size: Vec2::new(entry.width as f32, entry.height as f32),
            }),
            Bc7EntryStorage::Standalone { slot: None } => Some(AssetResidency::Loading),
            Bc7EntryStorage::Failed => Some(AssetResidency::Failed),
        }
    }

    /// 当前活跃的 BC7 动态图集页数量。
    #[must_use]
    pub(crate) fn active_pages(&self) -> usize {
        self.allocator.active_page_count()
    }

    /// 当前已驻留 BC7 资产占用的真实显存字节数（精确 1 B/px）。
    #[must_use]
    pub(crate) const fn vram_bytes(&self) -> u64 {
        self.vram_bytes
    }

    /// 本帧从节拍队列出队上传的 BC7 纹理数量。
    #[must_use]
    pub(crate) const fn uploads_this_frame(&self) -> usize {
        self.uploads_this_frame
    }

    /// 当前仍在上传队列中等待后续帧排空的 BC7 纹理数量。
    #[must_use]
    pub(crate) fn pending_uploads(&self) -> usize {
        self.upload_queue.len()
    }

    /// 累计因显存预算超限触发的离屏 LRU 驱逐次数。
    #[must_use]
    pub(crate) const fn eviction_count(&self) -> u64 {
        self.eviction_count
    }

    /// 当前是否处于非原生 BC7 透明降级模式。
    #[must_use]
    pub(crate) const fn fallback_active(&self) -> bool {
        !self.capabilities.supports_bc7
            || matches!(
                self.capabilities.active_backend,
                CompressedTextureBackend::Rgba8Fallback
            )
    }

    /// 判断指定 `AssetId` 当前是否驻留在独立 Bindless 外部槽（而非多页图集）。
    #[must_use]
    pub(crate) fn is_standalone(&self, id: AssetId) -> bool {
        matches!(
            self.entries.get(&id).map(|e| e.storage),
            Some(Bc7EntryStorage::Standalone { .. })
        )
    }

    /// 为无头 / 降级模式的图集页分配确定性模拟 Bindless 槽位（`48..176`）。
    fn allocate_fallback_page_slot(&self, page_id: AtlasPageId) -> u32 {
        EXTERNAL_TEXTURE_BASE + (page_id.0 % FALLBACK_PAGE_SLOT_COUNT)
    }

    /// 为无头 / 降级模式的独立大纹理分配确定性模拟 Bindless 槽位（`176..304`），避免与页槽位冲突。
    fn allocate_fallback_standalone_slot(&mut self) -> u32 {
        let base = EXTERNAL_TEXTURE_BASE + FALLBACK_PAGE_SLOT_COUNT;
        let span = EXTERNAL_TEXTURE_SLOTS - FALLBACK_PAGE_SLOT_COUNT;
        let slot = base + (self.next_fallback_standalone_offset % span);
        self.next_fallback_standalone_offset = self.next_fallback_standalone_offset.wrapping_add(1);
        slot
    }
}
