//! L3 离屏渲染测试宿主（`offscreen-test` feature）：无窗口、无交换链驱动完整渲染管线。

use std::fmt;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rustc_hash::{FxHashMap, FxHashSet};
use unbound_asset::{
    decode_bc7_to_rgba8_fallback, AssetId, AtlasPageId, CompressedTexturePayload,
    BC7_ATLAS_PAGE_SIZE,
};
use unbound_gpu::atlas::AtlasSet;
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::{Camera, CustomViewportPass, ManagedExternalTextureDesc, Renderer};
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_theme::ThemeSet;

use crate::theme_driver::{make_theme_snapshot_key, ThemeTransitionDriver};
use crate::{
    AppKey, AppRuntime, Application, Bc7GpuOp, DirtyFlags, ExternalTextureHandle,
    ExternalTextureOp, FrameReport, Modifiers, VideoSurfaceOp, Viewport3DFrameEntry,
};

use super::{BeforeRenderHookFn, InteractionDriver};

/// 进程级共享无头 Vulkan 设备（优先硬件 GPU，回退 Lavapipe / SwiftShader 等软光栅）。
static SHARED_OFFSCREEN_GPU: OnceLock<Result<Arc<Gpu>, String>> = OnceLock::new();

/// 串行化多线程测试中的 GPU 提交与回读，避免并发驱动/队列竞争。
static GPU_SUBMIT_MUTEX: Mutex<()> = Mutex::new(());

/// L3 离屏测试宿主初始化错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OffscreenInitError {
    /// 环境中既无可用硬件 Vulkan GPU，也无可用软光栅实现。
    NoVulkanDevice(String),
    /// 离屏渲染器或图集分配失败。
    RendererInit(String),
}

impl fmt::Display for OffscreenInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoVulkanDevice(msg) => write!(f, "no headless Vulkan device available: {msg}"),
            Self::RendererInit(msg) => write!(f, "failed to initialize offscreen renderer: {msg}"),
        }
    }
}

impl std::error::Error for OffscreenInitError {}

/// 获取进程内共享的单实例无头 Vulkan 设备。
pub fn shared_offscreen_gpu() -> Result<Arc<Gpu>, OffscreenInitError> {
    let result = SHARED_OFFSCREEN_GPU.get_or_init(|| {
        let validation = std::env::var("UNBOUND_VALIDATION").is_ok();
        Gpu::new(GpuConfig {
            validation,
            pool_capacity: 8 << 20,
            ring_capacity: 1 << 20,
            ..Default::default()
        })
        .map(Arc::new)
        .map_err(|err| err.to_string())
    });
    result.clone().map_err(OffscreenInitError::NoVulkanDevice)
}

/// 离屏渲染回读得到的直通 RGBA8 图像。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    /// 图像像素宽度。
    pub width: u32,
    /// 图像像素高度。
    pub height: u32,
    /// 行优先 RGBA8 字节数组（长度 = `width * height * 4`）。
    pub rgba: Vec<u8>,
}

impl RgbaImage {
    /// 从渲染器回读的 BGRA8 字节数组构造 RGBA8 图像（原地交换 R/B 通道）。
    #[must_use]
    pub fn from_bgra8(width: u32, height: u32, mut bgra: Vec<u8>) -> Self {
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Self {
            width,
            height,
            rgba: bgra,
        }
    }

    /// 读取坐标 `(x, y)` 的 RGBA8 像素；越界时返回 `[0, 0, 0, 0]`。
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0, 0, 0, 0];
        }
        let idx = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[idx],
            self.rgba[idx + 1],
            self.rgba[idx + 2],
            self.rgba[idx + 3],
        ]
    }

    /// 将逻辑矩形裁剪到当前图像整数像素边界 `(x0, y0, x1, y1)`。
    fn clamp_rect(&self, rect: Rect) -> Option<(u32, u32, u32, u32)> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let x0 = (rect.origin.x.floor().max(0.0) as u32).min(self.width);
        let y0 = (rect.origin.y.floor().max(0.0) as u32).min(self.height);
        let x1 = ((rect.origin.x + rect.size.x).ceil().max(0.0) as u32).min(self.width);
        let y1 = ((rect.origin.y + rect.size.y).ceil().max(0.0) as u32).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some((x0, y0, x1, y1))
        }
    }

    /// 判断指定矩形区域内所有像素是否完全一致（纯色/空白检测）。
    #[must_use]
    pub fn region_is_uniform(&self, rect: Rect) -> bool {
        let Some((x0, y0, x1, y1)) = self.clamp_rect(rect) else {
            return true;
        };
        let first = self.pixel(x0, y0);
        for y in y0..y1 {
            for x in x0..x1 {
                if self.pixel(x, y) != first {
                    return false;
                }
            }
        }
        true
    }

    /// 计算指定矩形区域内 RGB 通道的总方差（用于检测非纯色渲染内容）。
    #[must_use]
    pub fn region_variance(&self, rect: Rect) -> f64 {
        let Some((x0, y0, x1, y1)) = self.clamp_rect(rect) else {
            return 0.0;
        };
        let count = f64::from((x1 - x0) * (y1 - y0));
        if count <= 1.0 {
            return 0.0;
        }
        let mut sum = [0.0f64; 3];
        for y in y0..y1 {
            for x in x0..x1 {
                let p = self.pixel(x, y);
                sum[0] += f64::from(p[0]);
                sum[1] += f64::from(p[1]);
                sum[2] += f64::from(p[2]);
            }
        }
        let mean = [sum[0] / count, sum[1] / count, sum[2] / count];
        let mut sq = 0.0f64;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = self.pixel(x, y);
                for c in 0..3 {
                    let d = f64::from(p[c]) - mean[c];
                    sq += d * d;
                }
            }
        }
        sq / (count * 3.0)
    }

    /// 计算两帧在指定矩形区域内发生变化的像素占比（单通道差值 > `channel_tolerance` 计为变化）。
    #[must_use]
    pub fn diff_region_ratio(&self, other: &Self, rect: Rect, channel_tolerance: u8) -> f64 {
        if self.width != other.width || self.height != other.height {
            return 1.0;
        }
        let Some((x0, y0, x1, y1)) = self.clamp_rect(rect) else {
            return 0.0;
        };
        let total = (x1 - x0) * (y1 - y0);
        if total == 0 {
            return 0.0;
        }
        let mut changed = 0u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let a = self.pixel(x, y);
                let b = other.pixel(x, y);
                let max_d = a[0]
                    .abs_diff(b[0])
                    .max(a[1].abs_diff(b[1]))
                    .max(a[2].abs_diff(b[2]))
                    .max(a[3].abs_diff(b[3]));
                if max_d > channel_tolerance {
                    changed += 1;
                }
            }
        }
        f64::from(changed) / f64::from(total)
    }

    /// 计算两帧在指定矩形区域内的最大单通道像素差。
    #[must_use]
    pub fn diff_region_max_delta(&self, other: &Self, rect: Rect) -> u8 {
        if self.width != other.width || self.height != other.height {
            return u8::MAX;
        }
        let Some((x0, y0, x1, y1)) = self.clamp_rect(rect) else {
            return 0;
        };
        let mut max_delta = 0u8;
        for y in y0..y1 {
            for x in x0..x1 {
                let a = self.pixel(x, y);
                let b = other.pixel(x, y);
                let d = a[0]
                    .abs_diff(b[0])
                    .max(a[1].abs_diff(b[1]))
                    .max(a[2].abs_diff(b[2]))
                    .max(a[3].abs_diff(b[3]));
                max_delta = max_delta.max(d);
            }
        }
        max_delta
    }

    /// 将当前 RGBA8 图像写入 PNG 文件（自动创建父目录）。
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let file = File::create(path).map_err(|e| e.to_string())?;
        let writer = BufWriter::new(file);
        let mut encoder = png::Encoder::new(writer, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut png_writer = encoder.write_header().map_err(|e| e.to_string())?;
        png_writer
            .write_image_data(&self.rgba)
            .map_err(|e| e.to_string())
    }
}

/// L3 离屏渲染测试宿主：在无 OS 窗口与交换链的条件下驱动完整应用渲染并回读 RGBA8 像素。
pub struct OffscreenTestHarness<A: Application> {
    runtime: AppRuntime<A>,
    renderer: Option<Renderer>,
    atlases: Arc<AtlasSet>,
    theme_driver: ThemeTransitionDriver,
    on_before_render: Option<BeforeRenderHookFn<A>>,
    external_gpu_slots: FxHashMap<ExternalTextureHandle, (u32, u32, u32, bool)>,
    bc7_fallback_pages: FxHashMap<AtlasPageId, (u32, Vec<u8>)>,
    bc7_fallback_standalone: FxHashMap<AssetId, u32>,
    scene_generation: u64,
    last_frame: Option<RgbaImage>,
    last_report: Option<FrameReport>,
    failure_dump_dir: Option<PathBuf>,
}

impl<A: Application> OffscreenTestHarness<A> {
    /// 尝试构造 L3 离屏测试宿主；当环境中无可用 Vulkan/软光栅设备时返回 `Err` 供测试显式跳过。
    pub fn try_new(app: A, width: u32, height: u32) -> Result<Self, OffscreenInitError> {
        let w = width.max(1);
        let h = height.max(1);
        let gpu = shared_offscreen_gpu()?;
        let _guard = GPU_SUBMIT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let mut renderer = Renderer::new_shared(Arc::clone(&gpu), w, h)
            .map_err(|e| OffscreenInitError::RendererInit(e.to_string()))?;
        let atlases = match AtlasSet::new(&renderer.gpu, 64) {
            Ok(a) => Arc::new(a),
            Err(e) => {
                let msg = e.to_string();
                renderer.destroy();
                return Err(OffscreenInitError::RendererInit(msg));
            }
        };
        if let Err(err) = renderer.install_atlases(Arc::clone(&atlases)) {
            renderer.destroy();
            return Err(OffscreenInitError::RendererInit(err.to_string()));
        }

        let mut runtime = AppRuntime::new(app, Vec2::new(w as f32, h as f32));
        let video_tier = renderer.video_upload_capabilities().active_tier;
        runtime.asset_hub_mut().set_video_upload_tier(video_tier);
        let bc7_caps = renderer.compressed_texture_capabilities();
        runtime.asset_hub_mut().set_bc7_capabilities(bc7_caps);
        runtime.asset_hub_mut().set_bc7_gpu_backed(true);

        let ctx = runtime.context();
        let theme_driver = ThemeTransitionDriver::new(
            ThemeSet::for_product_with_system_accent(
                ctx.preset,
                ctx.accent,
                ctx.resolved_system_accent(),
            ),
            ctx.theme,
        );

        Ok(Self {
            runtime,
            renderer: Some(renderer),
            atlases,
            theme_driver,
            on_before_render: None,
            external_gpu_slots: FxHashMap::default(),
            bc7_fallback_pages: FxHashMap::default(),
            bc7_fallback_standalone: FxHashMap::default(),
            scene_generation: 0,
            last_frame: None,
            last_report: None,
            failure_dump_dir: None,
        })
    }

    /// 构造 L3 离屏测试宿主，初始化失败时直接 panic。
    #[must_use]
    pub fn new(app: A, width: u32, height: u32) -> Self {
        Self::try_new(app, width, height)
            .unwrap_or_else(|err| panic!("OffscreenTestHarness::new failed: {err}"))
    }

    /// 返回当前共享无头 Vulkan 设备名称（用于诊断输出）。
    #[must_use]
    pub fn device_name(&self) -> String {
        self.renderer
            .as_ref()
            .map(|r| r.gpu.device_name())
            .unwrap_or_default()
    }

    /// 配置像素断言失败时自动导出 PNG 截图的目录（链式构造）。
    #[must_use]
    pub fn with_failure_dump_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.failure_dump_dir = Some(dir.into());
        self
    }

    /// 注册每帧构建前执行的 `on_before_render` 钩子（链式构造）。
    #[must_use]
    pub fn with_on_before_render(
        mut self,
        hook: impl FnMut(&mut AppRuntime<A>) + Send + 'static,
    ) -> Self {
        self.on_before_render = Some(Box::new(hook));
        self
    }

    /// 设置或替换 `on_before_render` 钩子。
    pub fn set_on_before_render(&mut self, hook: impl FnMut(&mut AppRuntime<A>) + Send + 'static) {
        self.on_before_render = Some(Box::new(hook));
        self.runtime.invalidate();
    }

    /// 注册自定义 3D 视口离屏渲染通道（链式构造）。
    pub fn with_viewport_pass(mut self, pass: Box<dyn CustomViewportPass>) -> Result<Self, String> {
        self.register_viewport_pass(pass)?;
        Ok(self)
    }

    /// 向离屏渲染器注册自定义 3D 视口渲染通道。
    pub fn register_viewport_pass(
        &mut self,
        pass: Box<dyn CustomViewportPass>,
    ) -> Result<(), String> {
        let _guard = GPU_SUBMIT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| "offscreen renderer already destroyed".to_string())?;
        renderer
            .register_viewport_pass(pass)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// 只读借用内部 [`AppRuntime`]。
    #[must_use]
    pub const fn runtime(&self) -> &AppRuntime<A> {
        &self.runtime
    }

    /// 可变借用内部 [`AppRuntime`]。
    pub fn runtime_mut(&mut self) -> &mut AppRuntime<A> {
        &mut self.runtime
    }

    /// 获取最近一次渲染回读的 RGBA8 图像。
    #[must_use]
    pub const fn last_frame(&self) -> Option<&RgbaImage> {
        self.last_frame.as_ref()
    }

    /// 获取最近一次 [`FrameReport`]。
    #[must_use]
    pub const fn last_report(&self) -> Option<&FrameReport> {
        self.last_report.as_ref()
    }

    /// 调整离屏渲染目标与运行时逻辑视口尺寸（像素）。
    pub fn resize_target(&mut self, width: u32, height: u32) -> Result<(), String> {
        let w = width.max(1);
        let h = height.max(1);
        self.runtime.resize(Vec2::new(w as f32, h as f32));
        let _guard = GPU_SUBMIT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| "offscreen renderer already destroyed".to_string())?;
        renderer.resize(w, h).map_err(|e| e.to_string())
    }

    /// 推进并离屏渲染单帧，同步全部纹理/3D 视口资源并回读 RGBA8 像素。
    pub fn render_frame(&mut self, dt: f32) -> Result<RgbaImage, String> {
        if let Some(hook) = self.on_before_render.as_mut() {
            hook(&mut self.runtime);
        }

        let _ = self.runtime.tick_with_clock(dt, dt);
        let mut report = self.runtime.frame(0.0);

        let _guard = GPU_SUBMIT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut renderer) = self.renderer.take() else {
            return Err("offscreen renderer already destroyed".to_string());
        };

        let result = (|| -> Result<(RgbaImage, FrameReport), String> {
            self.sync_theme(&mut renderer);

            let mut needs_rebuild = false;
            needs_rebuild |= self.process_external_texture_ops(
                &mut renderer,
                std::mem::take(&mut report.external_texture_ops),
            );
            needs_rebuild |= self.process_video_surface_ops(
                &mut renderer,
                std::mem::take(&mut report.video_surface_ops),
            );
            needs_rebuild |= self.sync_viewports_3d(&mut renderer, &report.viewports_3d);
            needs_rebuild |=
                self.process_bc7_ops(&mut renderer, std::mem::take(&mut report.bc7_ops));

            if !report.atlas_uploads.is_empty() {
                self.atlases
                    .upload_batch(&renderer.gpu, &report.atlas_uploads)
                    .map_err(|e| e.to_string())?;
            }

            // 若本帧首次为外部纹理/BC7/3D 视口分配了 Bindless 槽位，立即重建一次场景以带上新槽位。
            if needs_rebuild {
                self.runtime.invalidate();
                let follow = self.runtime.frame(0.0);
                if !follow.atlas_uploads.is_empty() {
                    self.atlases
                        .upload_batch(&renderer.gpu, &follow.atlas_uploads)
                        .map_err(|e| e.to_string())?;
                }
                let _ = self.process_external_texture_ops(
                    &mut renderer,
                    follow.external_texture_ops.clone(),
                );
                let _ =
                    self.process_video_surface_ops(&mut renderer, follow.video_surface_ops.clone());
                let _ = self.sync_viewports_3d(&mut renderer, &follow.viewports_3d);
                let _ = self.process_bc7_ops(&mut renderer, follow.bc7_ops.clone());
                report = follow;
            }

            if self.scene_generation == 0 || report.diff.bumps_generation() {
                self.scene_generation = self.scene_generation.wrapping_add(1).max(1);
            }
            let (actual_w, actual_h) = renderer.size();
            let effective_scale = self.runtime.context().effective_scale();
            let camera = Camera::ui((actual_w as f32, actual_h as f32), effective_scale);
            let backdrops = crate::collect_backdrops(&report.backdrops, effective_scale);
            let scene_data = self.runtime.scene();
            let scene = crate::scene_upload(
                scene_data,
                &backdrops,
                camera,
                self.scene_generation,
                report.diff.ranges(),
                report.aux_changed,
            );

            renderer.set_clear_alpha(self.runtime.clear_alpha());
            renderer.render(&scene).map_err(|e| e.to_string())?;
            let bgra = renderer.read_pixels().map_err(|e| e.to_string())?;
            let image = RgbaImage::from_bgra8(actual_w, actual_h, bgra);
            Ok((image, report))
        })();

        self.renderer = Some(renderer);
        let (image, final_report) = result?;
        self.last_frame = Some(image.clone());
        self.last_report = Some(final_report);
        Ok(image)
    }

    fn sync_theme(&mut self, renderer: &mut Renderer) {
        let (key, sys_accent, transition_cfg) = {
            let ctx = self.runtime.context();
            let sys_accent = ctx.resolved_system_accent();
            (
                make_theme_snapshot_key(ctx.theme, ctx.preset, ctx.accent, sys_accent),
                sys_accent,
                ctx.theme_transition,
            )
        };
        let outcome =
            self.theme_driver
                .sync_preset(key, sys_accent, transition_cfg, Some(renderer));
        self.runtime.context_mut().theme_transition_remaining_secs = outcome.remaining_secs;
    }

    fn process_external_texture_ops(
        &mut self,
        renderer: &mut Renderer,
        ops: Vec<ExternalTextureOp>,
    ) -> bool {
        let mut needs_rebuild = false;
        for op in ops {
            match op {
                ExternalTextureOp::Upload {
                    handle,
                    width,
                    height,
                    srgb,
                    rgba,
                } => {
                    let desc = ManagedExternalTextureDesc::new(width, height, srgb);
                    match renderer.upload_external_rgba8(desc, &rgba) {
                        Ok(slot) => {
                            self.external_gpu_slots
                                .insert(handle, (slot, width, height, srgb));
                            self.runtime
                                .asset_hub_mut()
                                .commit_external_texture_slot(handle, Ok(slot));
                            needs_rebuild = true;
                        }
                        Err(error) => {
                            self.runtime
                                .asset_hub_mut()
                                .commit_external_texture_slot(handle, Err(format!("{error:?}")));
                            needs_rebuild = true;
                        }
                    }
                }
                ExternalTextureOp::Update {
                    handle,
                    width,
                    height,
                    rgba,
                } => {
                    let Some((slot, cur_w, cur_h, srgb)) =
                        self.external_gpu_slots.get(&handle).copied()
                    else {
                        continue;
                    };
                    if cur_w == width && cur_h == height {
                        if let Err(error) = renderer.update_external_rgba8(slot, &rgba) {
                            self.runtime
                                .asset_hub_mut()
                                .commit_external_texture_slot(handle, Err(format!("{error:?}")));
                        }
                        needs_rebuild = true;
                    } else {
                        let _ = renderer.destroy_external_texture(slot);
                        self.external_gpu_slots.remove(&handle);
                        let desc = ManagedExternalTextureDesc::new(width, height, srgb);
                        match renderer.upload_external_rgba8(desc, &rgba) {
                            Ok(new_slot) => {
                                self.external_gpu_slots
                                    .insert(handle, (new_slot, width, height, srgb));
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_external_texture_slot(handle, Ok(new_slot));
                            }
                            Err(error) => {
                                self.runtime.asset_hub_mut().commit_external_texture_slot(
                                    handle,
                                    Err(format!("{error:?}")),
                                );
                            }
                        }
                        needs_rebuild = true;
                    }
                }
                ExternalTextureOp::Release { handle } => {
                    if let Some((slot, _, _, _)) = self.external_gpu_slots.remove(&handle) {
                        let _ = renderer.destroy_external_texture(slot);
                    }
                }
            }
        }
        needs_rebuild
    }

    fn process_video_surface_ops(
        &mut self,
        renderer: &mut Renderer,
        ops: Vec<VideoSurfaceOp>,
    ) -> bool {
        let mut needs_rebuild = false;
        for op in ops {
            match op {
                VideoSurfaceOp::UploadFrame {
                    handle,
                    frame,
                    target_slot,
                } => {
                    if let Ok(result) = renderer.upload_video_frame(target_slot, &frame) {
                        let old_slot = self.runtime.asset_hub_mut().commit_video_surface_slot(
                            handle,
                            result.slot,
                            frame.width,
                            frame.height,
                            frame.format,
                            result.vram_bytes,
                        );
                        if let Some(stale_slot) = old_slot {
                            let _ = renderer.destroy_video_surface(stale_slot);
                        }
                        if target_slot != Some(result.slot) {
                            needs_rebuild = true;
                        }
                    }
                }
                VideoSurfaceOp::Evict { slot, .. } => {
                    let _ = renderer.destroy_video_surface(slot);
                    needs_rebuild = true;
                }
                VideoSurfaceOp::Release { slot, .. } => {
                    if let Some(slot) = slot {
                        let _ = renderer.destroy_video_surface(slot);
                    }
                }
            }
        }
        needs_rebuild
    }

    fn sync_viewports_3d(
        &mut self,
        renderer: &mut Renderer,
        entries: &[Viewport3DFrameEntry],
    ) -> bool {
        let mut active_ids = FxHashSet::default();
        let mut slot_changed = false;
        for entry in entries {
            active_ids.insert(entry.viewport_id);
            let _ = renderer.sync_viewport_layout(
                entry.viewport_id,
                entry.logical_rect,
                entry.render_scale,
                entry.physical_size,
            );
            let committed = renderer.viewport_bindless_slot(entry.viewport_id);
            if self.runtime.viewport_3d_slot(entry.viewport_id) != committed {
                slot_changed = true;
            }
            self.runtime
                .commit_viewport_3d_slot(entry.viewport_id, committed);
        }
        for id in renderer.viewport_ids() {
            if !active_ids.contains(&id) {
                let _ = renderer.sync_viewport_layout(id, Rect::ZERO, 1.0, None);
            }
        }
        let dispatches = renderer.take_viewport_pass_dispatches();
        if dispatches > 0 {
            self.runtime.record_viewport_pass_dispatches(dispatches);
        }
        slot_changed
    }

    fn process_bc7_ops(&mut self, renderer: &mut Renderer, ops: Vec<Bc7GpuOp>) -> bool {
        if ops.is_empty() {
            return false;
        }
        let supports_bc7 = renderer.compressed_texture_capabilities().supports_bc7;
        let mut needs_rebuild = false;
        let mut dirty_fallback_pages: FxHashSet<AtlasPageId> = FxHashSet::default();

        for op in ops {
            match op {
                Bc7GpuOp::CreateAtlasPage { page_id } => {
                    if supports_bc7 {
                        match renderer.create_bc7_atlas_page(page_id.0) {
                            Ok(slot) => {
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_page_slot(page_id, Ok(slot));
                                needs_rebuild = true;
                            }
                            Err(error) => {
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_page_slot(page_id, Err(format!("{error:?}")));
                                needs_rebuild = true;
                            }
                        }
                    } else {
                        let page_bytes =
                            (BC7_ATLAS_PAGE_SIZE as usize) * (BC7_ATLAS_PAGE_SIZE as usize) * 4;
                        let canvas = vec![0u8; page_bytes];
                        let desc = ManagedExternalTextureDesc::new(
                            BC7_ATLAS_PAGE_SIZE,
                            BC7_ATLAS_PAGE_SIZE,
                            false,
                        );
                        match renderer.upload_external_rgba8(desc, &canvas) {
                            Ok(slot) => {
                                self.bc7_fallback_pages.insert(page_id, (slot, canvas));
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_page_slot(page_id, Ok(slot));
                                needs_rebuild = true;
                            }
                            Err(error) => {
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_page_slot(page_id, Err(format!("{error:?}")));
                                needs_rebuild = true;
                            }
                        }
                    }
                }
                Bc7GpuOp::UploadAtlasRegion {
                    page_id,
                    x,
                    y,
                    width,
                    height,
                    bytes,
                } => {
                    if supports_bc7 {
                        if renderer
                            .stage_bc7_atlas_subregion(page_id.0, x, y, width, height, &bytes)
                            .is_ok()
                        {
                            needs_rebuild = true;
                        }
                    } else if let Some((_, canvas)) = self.bc7_fallback_pages.get_mut(&page_id) {
                        if let Ok(payload) =
                            CompressedTexturePayload::bc7_unorm(width, height, bytes)
                        {
                            let rgba = decode_bc7_to_rgba8_fallback(&payload);
                            let page_stride = (BC7_ATLAS_PAGE_SIZE as usize) * 4;
                            let row_bytes = (width as usize) * 4;
                            for row in 0..(height as usize) {
                                let dst = ((y as usize) + row) * page_stride + (x as usize) * 4;
                                let src = row * row_bytes;
                                if dst + row_bytes <= canvas.len() && src + row_bytes <= rgba.len()
                                {
                                    canvas[dst..dst + row_bytes]
                                        .copy_from_slice(&rgba[src..src + row_bytes]);
                                }
                            }
                            dirty_fallback_pages.insert(page_id);
                            needs_rebuild = true;
                        }
                    }
                }
                Bc7GpuOp::UploadStandalone {
                    asset_id,
                    width,
                    height,
                    bytes,
                } => {
                    if supports_bc7 {
                        match renderer.upload_external_bc7(width, height, &bytes) {
                            Ok(slot) => {
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_standalone_slot(asset_id, Ok(slot));
                                needs_rebuild = true;
                            }
                            Err(error) => {
                                self.runtime.asset_hub_mut().commit_bc7_standalone_slot(
                                    asset_id,
                                    Err(format!("{error:?}")),
                                );
                                needs_rebuild = true;
                            }
                        }
                    } else if let Ok(payload) =
                        CompressedTexturePayload::bc7_unorm(width, height, bytes)
                    {
                        let rgba = decode_bc7_to_rgba8_fallback(&payload);
                        let desc = ManagedExternalTextureDesc::new(width, height, false);
                        match renderer.upload_external_rgba8(desc, &rgba) {
                            Ok(slot) => {
                                self.bc7_fallback_standalone.insert(asset_id, slot);
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_standalone_slot(asset_id, Ok(slot));
                                needs_rebuild = true;
                            }
                            Err(error) => {
                                self.runtime.asset_hub_mut().commit_bc7_standalone_slot(
                                    asset_id,
                                    Err(format!("{error:?}")),
                                );
                                needs_rebuild = true;
                            }
                        }
                    }
                }
                Bc7GpuOp::DestroyAtlasPage { page_id } => {
                    if supports_bc7 {
                        let _ = renderer.destroy_bc7_atlas_page(page_id.0);
                    } else if let Some((slot, _)) = self.bc7_fallback_pages.remove(&page_id) {
                        let _ = renderer.destroy_external_texture(slot);
                    }
                    needs_rebuild = true;
                }
                Bc7GpuOp::DestroyStandalone { asset_id, slot } => {
                    if supports_bc7 {
                        let _ = renderer.destroy_external_bc7(slot);
                    } else if let Some(fb_slot) = self.bc7_fallback_standalone.remove(&asset_id) {
                        let _ = renderer.destroy_external_texture(fb_slot);
                    }
                    needs_rebuild = true;
                }
            }
        }

        for page_id in dirty_fallback_pages {
            if let Some((slot, canvas)) = self.bc7_fallback_pages.get(&page_id) {
                let _ = renderer.update_external_rgba8(*slot, canvas);
            }
        }
        needs_rebuild
    }

    fn dump_failure_png(&self, image: &RgbaImage, suffix: &str) -> Option<PathBuf> {
        let dir = self.failure_dump_dir.as_ref()?;
        let safe_name: String = suffix
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = dir.join(format!("{safe_name}.png"));
        if image.save_png(&path).is_ok() {
            Some(path)
        } else {
            None
        }
    }

    /// 断言最近一帧在指定矩形区域内不是纯色/空白；失败时按配置自动保存 PNG。
    pub fn assert_region_not_uniform(&self, rect: Rect, label: &str) {
        let frame = self
            .last_frame
            .as_ref()
            .expect("no rendered frame available; call render_frame first");
        if frame.region_is_uniform(rect) {
            let dump = self.dump_failure_png(frame, &format!("{label}_uniform"));
            panic!("offscreen region {rect:?} is uniform ({label}); dump={dump:?}");
        }
    }

    /// 断言最近一帧相对参考帧 `prev` 在指定区域内发生了足够比例的像素变化；失败时自动保存对比 PNG。
    pub fn assert_region_changed(
        &self,
        prev: &RgbaImage,
        rect: Rect,
        min_changed_ratio: f64,
        label: &str,
    ) {
        let frame = self
            .last_frame
            .as_ref()
            .expect("no rendered frame available; call render_frame first");
        let ratio = frame.diff_region_ratio(prev, rect, 2);
        if ratio < min_changed_ratio {
            let before_dump = self.dump_failure_png(prev, &format!("{label}_before"));
            let after_dump = self.dump_failure_png(frame, &format!("{label}_after"));
            panic!(
                "offscreen region {rect:?} changed ratio {ratio:.4} < {min_changed_ratio:.4} ({label}); before={before_dump:?}, after={after_dump:?}"
            );
        }
    }

    /// 断言最近一帧中指定像素坐标接近预期 RGBA8 颜色。
    pub fn assert_pixel_near(&self, x: u32, y: u32, expected: [u8; 4], tolerance: u8, label: &str) {
        let frame = self
            .last_frame
            .as_ref()
            .expect("no rendered frame available; call render_frame first");
        let actual = frame.pixel(x, y);
        let max_d = actual[0]
            .abs_diff(expected[0])
            .max(actual[1].abs_diff(expected[1]))
            .max(actual[2].abs_diff(expected[2]))
            .max(actual[3].abs_diff(expected[3]));
        if max_d > tolerance {
            let dump = self.dump_failure_png(frame, &format!("{label}_pixel"));
            panic!(
                "pixel ({x}, {y}) = {actual:?}, expected {expected:?} +- {tolerance} ({label}); dump={dump:?}"
            );
        }
    }
}

impl<A: Application> InteractionDriver for OffscreenTestHarness<A> {
    fn click_button(&mut self, pos: Vec2, button: PointerButton) -> DirtyFlags {
        self.runtime.click_button(pos, button)
    }

    fn drag_button(
        &mut self,
        from: Vec2,
        to: Vec2,
        button: PointerButton,
        steps: usize,
    ) -> DirtyFlags {
        self.runtime.drag_button(from, to, button, steps)
    }

    fn wheel(&mut self, pos: Vec2, delta: Vec2) -> DirtyFlags {
        self.runtime.wheel(pos, delta)
    }

    fn key_tap(&mut self, key: AppKey, modifiers: Modifiers) -> DirtyFlags {
        self.runtime.key_tap(key, modifiers)
    }

    fn text_input(&mut self, text: impl Into<String>) -> DirtyFlags {
        self.runtime.text_input(text)
    }

    fn resize(&mut self, size: Vec2) -> DirtyFlags {
        let w = size.x.max(1.0).round() as u32;
        let h = size.y.max(1.0).round() as u32;
        let dirty = self.runtime.resize(size);
        let _guard = GPU_SUBMIT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(renderer) = self.renderer.as_mut() {
            let _ = renderer.resize(w, h);
        }
        dirty
    }

    fn step_frames(&mut self, count: usize, dt: f32) {
        for _ in 0..count {
            let _ = self.render_frame(dt);
        }
    }
}

impl<A: Application> Drop for OffscreenTestHarness<A> {
    fn drop(&mut self) {
        if let Some(renderer) = self.renderer.take() {
            let _guard = GPU_SUBMIT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            renderer.destroy();
        }
    }
}
