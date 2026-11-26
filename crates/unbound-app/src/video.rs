//! 视频表面生命周期、多路显存治理与声明式描述符。
//!
//! 本模块管理视频表面从首帧等待、就绪、切分辨率/切格式保帧过渡、离屏 LRU 显存驱逐到主动释放的完整状态机，
//! 并为 [`crate::AssetHub`] 与 [`crate::ViewBuilder::video_surface`] 提供无头可测的数据契约。

use rustc_hash::FxHashMap;

use unbound_asset::{LatestFrameMailbox, VideoFrameError, VideoFramePayload, VideoPixelFormat};
use unbound_gpu::{ImageChannelMask, VideoUploadTier};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;
use unbound_widgets::ObjectFit;

/// 默认视频表面 VRAM 预算上限（256 MiB）。
pub const DEFAULT_VIDEO_VRAM_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// 视频表面逻辑句柄。
///
/// 由 [`crate::AssetHub::create_video_surface`] 分配，在首帧到达前即可绑定至视图；
/// 切分辨率、切像素格式或离屏驱逐后均保持句柄稳定，不会导致调用方重绑。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoSurfaceHandle(pub u64);

/// 视频表面生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSurfaceState {
    /// 已创建句柄但尚未收到并提交首帧，渲染阶段输出受控圆角占位图元。
    PendingFirstFrame,
    /// 已绑定到有效 GPU 外部 Bindless 纹理槽并可稳定采样。
    Ready {
        /// 当前绑定的 GPU 纹理槽位索引。
        slot: u32,
        /// 当前纹理宽度（像素）。
        width: u32,
        /// 当前纹理高度（像素）。
        height: u32,
        /// 当前源视频像素格式。
        format: VideoPixelFormat,
        /// 当前占用的 GPU VRAM 字节数。
        vram_bytes: u64,
    },
    /// 分辨率或像素格式正在切换：继续采样 `prev_slot` 实现零闪烁保帧，直至新槽位提交后原子切换。
    Resizing {
        /// 仍在显示中的旧 GPU 纹理槽位索引。
        prev_slot: u32,
        /// 旧纹理宽度（像素）。
        prev_width: u32,
        /// 旧纹理高度（像素）。
        prev_height: u32,
        /// 旧源视频像素格式。
        prev_format: VideoPixelFormat,
        /// 目标新宽度（像素）。
        pending_width: u32,
        /// 目标新高度（像素）。
        pending_height: u32,
        /// 目标新像素格式。
        pending_format: VideoPixelFormat,
        /// 当前仍计入预算的 VRAM 字节数。
        vram_bytes: u64,
    },
    /// 因超出多路 VRAM 预算被离屏 LRU 驱逐，保留句柄与最后分辨率元数据，收到新帧后无缝恢复。
    Evicted {
        /// 被驱逐前的最后宽度（像素）。
        last_width: u32,
        /// 被驱逐前的最后高度（像素）。
        last_height: u32,
        /// 被驱逐前的最后像素格式。
        last_format: VideoPixelFormat,
    },
    /// 句柄已主动释放，后续提交将被忽略。
    Released,
}

/// 供窗口运行器或无头测试在渲染前消费的视频表面 GPU 操作队列项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoSurfaceOp {
    /// 上传或原地更新一帧视频载荷。
    UploadFrame {
        /// 视频表面逻辑句柄。
        handle: VideoSurfaceHandle,
        /// 经校验的最新视频帧载荷。
        frame: VideoFramePayload,
        /// 为 `Some(slot)` 时表示同尺寸同格式原地更新；为 `None` 时分配新槽位。
        target_slot: Option<u32>,
    },
    /// 驱逐离屏视频表面并销毁其 GPU 纹理槽。
    Evict {
        /// 视频表面逻辑句柄。
        handle: VideoSurfaceHandle,
        /// 待销毁的 GPU 纹理槽位索引。
        slot: u32,
    },
    /// 主动释放视频表面及其可能持有的 GPU 纹理槽。
    Release {
        /// 视频表面逻辑句柄。
        handle: VideoSurfaceHandle,
        /// 若表面释放时持有 GPU 槽位则为 `Some(slot)`，否则为 `None`。
        slot: Option<u32>,
    },
}

/// 视频表面子系统的帧级观测指标快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoSurfaceMetrics {
    /// 累计排空并派发上传的视频帧总数。
    pub frame_upload_count: u64,
    /// 累计因 Latest-Wins 覆盖而丢弃的未读帧总数。
    pub frame_dropped_count: u64,
    /// 当前持有活跃 GPU 纹理槽的视频表面数量（`Ready` 与 `Resizing`）。
    pub active_surfaces: usize,
    /// 当前所有活跃视频表面合计占用的 VRAM 字节数。
    pub vram_bytes: u64,
    /// 累计因超出 VRAM 预算触发的离屏 LRU 驱逐次数。
    pub eviction_count: u64,
    /// 当前渲染后端激活的视频上传档位。
    pub upload_tier: VideoUploadTier,
}

/// 声明式视频表面控件描述符，供 [`crate::ViewBuilder::video_surface`] 使用。
#[derive(Clone, Debug, PartialEq)]
pub struct VideoSurfaceDesc {
    /// 关联的视频表面逻辑句柄。
    pub handle: VideoSurfaceHandle,
    /// 布局期望尺寸（逻辑像素）。
    pub size: Vec2,
    /// 画面适配模式（默认 [`ObjectFit::Contain`]）。
    pub fit: ObjectFit,
    /// 四角独立圆角半径：左上、右上、右下、左下。
    pub radius: [f32; 4],
    /// 显式指定的归一化采样子矩形；`None` 时按 [`Self::fit`] 自动推导。
    pub sample_uv: Option<Rect>,
    /// 边框宽度（逻辑像素）；`0.0` 表示无边框。
    pub border_width: f32,
    /// 边框颜色令牌。
    pub border_color: Token,
    /// 画面着色令牌；[`Token::NONE`] 表示保持原色。
    pub tint: Token,
    /// 图元不透明度（范围 `0.0..=1.0`，默认 `1.0`）。
    pub opacity: f32,
    /// 未就绪或被驱逐时的占位背景色令牌。
    pub placeholder_color: Token,
    /// 可选无障碍描述文本。
    pub alt: Option<String>,
    /// 绘制深度键。
    pub z: u16,
    /// RGBA 通道隔离掩码（默认 [`ImageChannelMask::Rgba`]）。
    pub channel_mask: ImageChannelMask,
    /// 是否启用源纹理 Alpha 混合；为 `false` 时强制以不透明方式预览（默认 `true`）。
    pub enable_alpha: bool,
    /// 是否接受指针命中交互（默认 `false`，不抢占焦点）。
    pub interactive: bool,
}

impl VideoSurfaceDesc {
    /// 以默认参数构造视频表面描述符（`Contain`、`opacity = 1.0`、`interactive = false`）。
    #[must_use]
    pub fn new(handle: VideoSurfaceHandle, size: Vec2) -> Self {
        Self {
            handle,
            size,
            fit: ObjectFit::Contain,
            radius: [0.0; 4],
            sample_uv: None,
            border_width: 0.0,
            border_color: Token::NONE,
            tint: Token::NONE,
            opacity: 1.0,
            placeholder_color: Token::SURFACE_SUNKEN,
            alt: None,
            z: 690,
            channel_mask: ImageChannelMask::Rgba,
            enable_alpha: true,
            interactive: false,
        }
    }

    /// 设置画面适配模式（[`ObjectFit`]）。
    #[must_use]
    pub const fn with_fit(mut self, fit: ObjectFit) -> Self {
        self.fit = fit;
        self
    }

    /// 设置四角统一圆角半径。
    #[must_use]
    pub const fn with_radius(mut self, radius: f32) -> Self {
        self.radius = [radius; 4];
        self
    }

    /// 设置四角独立圆角半径。
    #[must_use]
    pub const fn with_radii(mut self, radii: [f32; 4]) -> Self {
        self.radius = radii;
        self
    }

    /// 设置归一化采样子矩形。
    #[must_use]
    pub const fn with_sample_uv(mut self, uv: Rect) -> Self {
        self.sample_uv = Some(uv);
        self
    }

    /// 设置边框宽度与颜色令牌。
    #[must_use]
    pub const fn with_border(mut self, width: f32, color: Token) -> Self {
        self.border_width = width;
        self.border_color = color;
        self
    }

    /// 设置画面着色令牌。
    #[must_use]
    pub const fn with_tint(mut self, tint: Token) -> Self {
        self.tint = tint;
        self
    }

    /// 设置图元不透明度（渲染时自动钳制至 `0.0..=1.0`）。
    #[must_use]
    pub const fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// 设置占位背景色令牌。
    #[must_use]
    pub const fn with_placeholder_color(mut self, color: Token) -> Self {
        self.placeholder_color = color;
        self
    }

    /// 设置无障碍替代文本。
    #[must_use]
    pub fn with_alt(mut self, alt: impl Into<String>) -> Self {
        self.alt = Some(alt.into());
        self
    }

    /// 设置绘制深度键。
    #[must_use]
    pub const fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 设置 RGBA 通道隔离掩码。
    #[must_use]
    pub const fn with_channel_mask(mut self, mask: ImageChannelMask) -> Self {
        self.channel_mask = mask;
        self
    }

    /// 设置 RGBA 通道隔离掩码（无前缀别名）。
    #[must_use]
    pub const fn channel_mask(self, mask: ImageChannelMask) -> Self {
        self.with_channel_mask(mask)
    }

    /// 设置是否启用源纹理 Alpha 混合。
    #[must_use]
    pub const fn with_enable_alpha(mut self, enable: bool) -> Self {
        self.enable_alpha = enable;
        self
    }

    /// 设置是否启用源纹理 Alpha 混合（无前缀别名）。
    #[must_use]
    pub const fn enable_alpha(self, enable: bool) -> Self {
        self.with_enable_alpha(enable)
    }

    /// 设置是否允许指针交互命中。
    #[must_use]
    pub const fn with_interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }
}

/// 单个视频表面的内部托管记录。
#[derive(Debug)]
struct VideoSurfaceRecord {
    state: VideoSurfaceState,
    mailbox: LatestFrameMailbox,
    last_visible_frame: u64,
    created_seq: u64,
}

/// 视频表面生命周期与多路 VRAM 预算管理器（由 [`crate::AssetHub`] 持有）。
#[derive(Debug)]
pub(crate) struct VideoSurfaceManager {
    surfaces: FxHashMap<VideoSurfaceHandle, VideoSurfaceRecord>,
    pending_ops: Vec<VideoSurfaceOp>,
    next_handle_id: u64,
    next_created_seq: u64,
    vram_budget_bytes: u64,
    upload_tier: VideoUploadTier,
    frame_upload_count: u64,
    frame_dropped_count: u64,
    eviction_count: u64,
    last_frame_index: u64,
    pending_dirty: bool,
}

impl Default for VideoSurfaceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoSurfaceManager {
    /// 构造默认配置的视频表面管理器（无头默认上传档位为 `HeadlessStub`）。
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            surfaces: FxHashMap::default(),
            pending_ops: Vec::new(),
            next_handle_id: 1,
            next_created_seq: 1,
            vram_budget_bytes: DEFAULT_VIDEO_VRAM_BUDGET_BYTES,
            upload_tier: VideoUploadTier::HeadlessStub,
            frame_upload_count: 0,
            frame_dropped_count: 0,
            eviction_count: 0,
            last_frame_index: 0,
            pending_dirty: false,
        }
    }

    /// 取出并重置轻量绘制脏标志（仅触发 `PAINT`，不触发 `LAYOUT`）。
    pub(crate) fn take_dirty(&mut self) -> bool {
        core::mem::take(&mut self.pending_dirty)
    }

    /// 创建新的视频表面逻辑句柄，初始状态为 [`VideoSurfaceState::PendingFirstFrame`]。
    pub(crate) fn create_surface(&mut self) -> VideoSurfaceHandle {
        let handle = VideoSurfaceHandle(self.next_handle_id);
        self.next_handle_id = self.next_handle_id.wrapping_add(1).max(1);
        let created_seq = self.next_created_seq;
        self.next_created_seq = self.next_created_seq.wrapping_add(1);
        self.surfaces.insert(
            handle,
            VideoSurfaceRecord {
                state: VideoSurfaceState::PendingFirstFrame,
                mailbox: LatestFrameMailbox::new(),
                last_visible_frame: 0,
                created_seq,
            },
        );
        handle
    }

    /// 向指定视频表面提交一帧载荷，经由 `LatestFrameMailbox` 实现 Latest-Wins 覆盖。
    pub(crate) fn submit_frame(
        &mut self,
        handle: VideoSurfaceHandle,
        frame: VideoFramePayload,
    ) -> Result<(), VideoFrameError> {
        frame.validate()?;
        let Some(record) = self.surfaces.get_mut(&handle) else {
            return Ok(());
        };
        if matches!(record.state, VideoSurfaceState::Released) {
            return Ok(());
        }
        let _ = record.mailbox.push(frame);
        self.pending_dirty = true;
        Ok(())
    }

    /// 释放指定视频表面，若持有活跃槽位则入队 [`VideoSurfaceOp::Release`]。
    pub(crate) fn release_surface(&mut self, handle: VideoSurfaceHandle) {
        let Some(record) = self.surfaces.get_mut(&handle) else {
            return;
        };
        if matches!(record.state, VideoSurfaceState::Released) {
            return;
        }
        // 同步释放前已累积的覆盖丢帧增量，并静默清空尚未消费的待上传帧。
        self.frame_dropped_count = self
            .frame_dropped_count
            .saturating_add(record.mailbox.take_dropped_delta());
        let _ = record.mailbox.take_latest();
        let active_slot = match record.state {
            VideoSurfaceState::Ready { slot, .. } => Some(slot),
            VideoSurfaceState::Resizing { prev_slot, .. } => Some(prev_slot),
            VideoSurfaceState::PendingFirstFrame
            | VideoSurfaceState::Evicted { .. }
            | VideoSurfaceState::Released => None,
        };
        record.state = VideoSurfaceState::Released;
        self.pending_ops.push(VideoSurfaceOp::Release {
            handle,
            slot: active_slot,
        });
        self.pending_dirty = true;
    }

    /// 查询视频表面当前状态。
    #[must_use]
    pub(crate) fn state(&self, handle: VideoSurfaceHandle) -> Option<&VideoSurfaceState> {
        self.surfaces.get(&handle).map(|record| &record.state)
    }

    /// 查询视频表面当前可供采样的 GPU 槽位与分辨率。
    ///
    /// `Resizing` 期间持续返回 `prev_slot`，确保切分辨率或切像素格式时不闪烁。
    #[must_use]
    pub(crate) fn slot(&self, handle: VideoSurfaceHandle) -> Option<(u32, u32, u32)> {
        match self.surfaces.get(&handle)?.state {
            VideoSurfaceState::Ready {
                slot,
                width,
                height,
                ..
            } => Some((slot, width, height)),
            VideoSurfaceState::Resizing {
                prev_slot,
                prev_width,
                prev_height,
                ..
            } => Some((prev_slot, prev_width, prev_height)),
            VideoSurfaceState::PendingFirstFrame
            | VideoSurfaceState::Evicted { .. }
            | VideoSurfaceState::Released => None,
        }
    }

    /// 标记指定视频表面在 `frame_index` 帧中可见。
    pub(crate) fn mark_visible(&mut self, handle: VideoSurfaceHandle, frame_index: u64) {
        self.last_frame_index = self.last_frame_index.max(frame_index);
        if let Some(record) = self.surfaces.get_mut(&handle) {
            if !matches!(record.state, VideoSurfaceState::Released) {
                record.last_visible_frame = record.last_visible_frame.max(frame_index);
            }
        }
    }

    /// 设置多路视频表面的 VRAM 预算上限（字节）。
    pub(crate) fn set_vram_budget_bytes(&mut self, budget: u64) {
        self.vram_budget_bytes = budget;
        if self.total_vram_bytes() > self.vram_budget_bytes {
            self.pending_dirty = true;
        }
    }

    /// 查询当前多路视频表面的 VRAM 预算上限（字节）。
    #[must_use]
    pub(crate) const fn vram_budget_bytes(&self) -> u64 {
        self.vram_budget_bytes
    }

    /// 同步底层渲染器协商出的视频上传档位。
    pub(crate) fn set_upload_tier(&mut self, tier: VideoUploadTier) {
        self.upload_tier = tier;
    }

    /// 排空各表面信箱中的最新帧并执行超预算离屏 LRU 驱逐。
    pub(crate) fn drain_ops(&mut self, frame_index: u64) -> Vec<VideoSurfaceOp> {
        self.last_frame_index = self.last_frame_index.max(frame_index);
        let mut ops = core::mem::take(&mut self.pending_ops);

        // 按句柄编号升序遍历，确保同帧多路视频操作顺序完全确定。
        let mut handles: Vec<VideoSurfaceHandle> = self.surfaces.keys().copied().collect();
        handles.sort_by_key(|handle| handle.0);

        for handle in handles {
            let Some(record) = self.surfaces.get_mut(&handle) else {
                continue;
            };
            self.frame_dropped_count = self
                .frame_dropped_count
                .saturating_add(record.mailbox.take_dropped_delta());
            if matches!(record.state, VideoSurfaceState::Released) {
                continue;
            }
            let Some(frame) = record.mailbox.take_latest() else {
                continue;
            };

            let target_slot = match record.state {
                VideoSurfaceState::PendingFirstFrame | VideoSurfaceState::Evicted { .. } => None,
                VideoSurfaceState::Ready {
                    slot,
                    width,
                    height,
                    format,
                    vram_bytes,
                } => {
                    if width == frame.width && height == frame.height && format == frame.format {
                        Some(slot)
                    } else {
                        record.state = VideoSurfaceState::Resizing {
                            prev_slot: slot,
                            prev_width: width,
                            prev_height: height,
                            prev_format: format,
                            pending_width: frame.width,
                            pending_height: frame.height,
                            pending_format: frame.format,
                            vram_bytes,
                        };
                        None
                    }
                }
                VideoSurfaceState::Resizing {
                    prev_slot,
                    prev_width,
                    prev_height,
                    prev_format,
                    vram_bytes,
                    ..
                } => {
                    record.state = VideoSurfaceState::Resizing {
                        prev_slot,
                        prev_width,
                        prev_height,
                        prev_format,
                        pending_width: frame.width,
                        pending_height: frame.height,
                        pending_format: frame.format,
                        vram_bytes,
                    };
                    None
                }
                VideoSurfaceState::Released => continue,
            };

            self.frame_upload_count = self.frame_upload_count.saturating_add(1);
            ops.push(VideoSurfaceOp::UploadFrame {
                handle,
                frame,
                target_slot,
            });
        }

        self.evict_over_budget(frame_index, &mut ops);
        ops
    }

    /// 在 GPU 完成上传后原子提交新槽位与显存占用，必要时返回待延迟销毁的旧槽位。
    pub(crate) fn commit_slot(
        &mut self,
        handle: VideoSurfaceHandle,
        slot: u32,
        width: u32,
        height: u32,
        format: VideoPixelFormat,
        vram_bytes: u64,
    ) -> Option<u32> {
        let record = self.surfaces.get_mut(&handle)?;
        if matches!(record.state, VideoSurfaceState::Released) {
            return Some(slot);
        }

        let (old_slot, visual_binding_changed) = match record.state {
            VideoSurfaceState::Ready {
                slot: prev_slot,
                width: prev_width,
                height: prev_height,
                ..
            } => {
                let stale = (prev_slot != slot).then_some(prev_slot);
                let changed = prev_slot != slot || prev_width != width || prev_height != height;
                (stale, changed)
            }
            VideoSurfaceState::Resizing { prev_slot, .. } => {
                let stale = (prev_slot != slot).then_some(prev_slot);
                (stale, true)
            }
            VideoSurfaceState::PendingFirstFrame | VideoSurfaceState::Evicted { .. } => {
                (None, true)
            }
            VideoSurfaceState::Released => return Some(slot),
        };

        record.state = VideoSurfaceState::Ready {
            slot,
            width,
            height,
            format,
            vram_bytes,
        };
        if visual_binding_changed {
            self.pending_dirty = true;
        }

        // 提交后立即复核 VRAM 预算，对非当前帧可见的离屏表面执行 LRU 驱逐并入队销毁操作。
        let frame_index = self.last_frame_index;
        let prev_ops_len = self.pending_ops.len();
        let mut evicted_ops = Vec::new();
        self.evict_over_budget(frame_index, &mut evicted_ops);
        if !evicted_ops.is_empty() {
            self.pending_ops.extend(evicted_ops);
            if self.pending_ops.len() > prev_ops_len {
                self.pending_dirty = true;
            }
        }

        old_slot
    }

    /// 返回当前视频表面子系统的观测指标快照。
    #[must_use]
    pub(crate) fn metrics(&self) -> VideoSurfaceMetrics {
        let mut active_surfaces = 0usize;
        let mut vram_bytes = 0u64;
        for record in self.surfaces.values() {
            match record.state {
                VideoSurfaceState::Ready {
                    vram_bytes: bytes, ..
                }
                | VideoSurfaceState::Resizing {
                    vram_bytes: bytes, ..
                } => {
                    active_surfaces += 1;
                    vram_bytes = vram_bytes.saturating_add(bytes);
                }
                VideoSurfaceState::PendingFirstFrame
                | VideoSurfaceState::Evicted { .. }
                | VideoSurfaceState::Released => {}
            }
        }
        VideoSurfaceMetrics {
            frame_upload_count: self.frame_upload_count,
            frame_dropped_count: self.frame_dropped_count,
            active_surfaces,
            vram_bytes,
            eviction_count: self.eviction_count,
            upload_tier: self.upload_tier,
        }
    }

    /// 在 GPU 设备丢失或重建上下文时清空所有活跃槽位绑定，将活跃表面转为 `Evicted`。
    pub(crate) fn invalidate_all(&mut self) {
        self.pending_ops.clear();
        for record in self.surfaces.values_mut() {
            let _ = record.mailbox.take_latest();
            match record.state {
                VideoSurfaceState::Ready {
                    width,
                    height,
                    format,
                    ..
                } => {
                    record.state = VideoSurfaceState::Evicted {
                        last_width: width,
                        last_height: height,
                        last_format: format,
                    };
                }
                VideoSurfaceState::Resizing {
                    prev_width,
                    prev_height,
                    prev_format,
                    ..
                } => {
                    record.state = VideoSurfaceState::Evicted {
                        last_width: prev_width,
                        last_height: prev_height,
                        last_format: prev_format,
                    };
                }
                VideoSurfaceState::PendingFirstFrame
                | VideoSurfaceState::Evicted { .. }
                | VideoSurfaceState::Released => {}
            }
        }
        self.pending_dirty = true;
    }

    /// 计算当前所有活跃视频表面合计占用的 VRAM 字节数。
    fn total_vram_bytes(&self) -> u64 {
        let mut total = 0u64;
        for record in self.surfaces.values() {
            match record.state {
                VideoSurfaceState::Ready { vram_bytes, .. }
                | VideoSurfaceState::Resizing { vram_bytes, .. } => {
                    total = total.saturating_add(vram_bytes);
                }
                VideoSurfaceState::PendingFirstFrame
                | VideoSurfaceState::Evicted { .. }
                | VideoSurfaceState::Released => {}
            }
        }
        total
    }

    /// 当总 VRAM 超出预算时，按 `last_visible_frame` 升序驱逐非本帧可见的离屏表面。
    fn evict_over_budget(&mut self, frame_index: u64, ops: &mut Vec<VideoSurfaceOp>) {
        let mut total = self.total_vram_bytes();
        if total <= self.vram_budget_bytes {
            return;
        }

        let mut candidates: Vec<(u64, u64, VideoSurfaceHandle)> = self
            .surfaces
            .iter()
            .filter_map(|(&handle, record)| {
                let is_resident = matches!(
                    record.state,
                    VideoSurfaceState::Ready { .. } | VideoSurfaceState::Resizing { .. }
                );
                if is_resident && record.last_visible_frame < frame_index {
                    Some((record.last_visible_frame, record.created_seq, handle))
                } else {
                    None
                }
            })
            .collect();
        candidates.sort_by_key(|&(last_visible, seq, handle)| (last_visible, seq, handle.0));

        for (_, _, handle) in candidates {
            if total <= self.vram_budget_bytes {
                break;
            }
            let Some(record) = self.surfaces.get_mut(&handle) else {
                continue;
            };
            let evicted = match record.state {
                VideoSurfaceState::Ready {
                    slot,
                    width,
                    height,
                    format,
                    vram_bytes,
                } => Some((slot, width, height, format, vram_bytes)),
                VideoSurfaceState::Resizing {
                    prev_slot,
                    prev_width,
                    prev_height,
                    prev_format,
                    vram_bytes,
                    ..
                } => Some((prev_slot, prev_width, prev_height, prev_format, vram_bytes)),
                VideoSurfaceState::PendingFirstFrame
                | VideoSurfaceState::Evicted { .. }
                | VideoSurfaceState::Released => None,
            };
            if let Some((slot, last_width, last_height, last_format, vram_bytes)) = evicted {
                record.state = VideoSurfaceState::Evicted {
                    last_width,
                    last_height,
                    last_format,
                };
                total = total.saturating_sub(vram_bytes);
                self.eviction_count = self.eviction_count.saturating_add(1);
                ops.push(VideoSurfaceOp::Evict { handle, slot });
            }
        }
    }
}
