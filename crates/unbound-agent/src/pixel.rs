//! 像素层：Agent 对界面像素的感知。
//!
//! 语义树告诉 Agent「界面上有什么」，像素层告诉它「界面长什么样」。
//! 两者缺一不可：语义能证明一个按钮**存在**，像素能证明它真的被**画出来**。
//!
//! 这一层**不抢鼠标、不占屏幕**。框架自带的离屏实现把场景渲染到独立的
//! 离屏目标再回读，全程不触碰宿主窗口——用户可以同时正常使用电脑。
//!
//! 离屏渲染与窗口渲染**共用同一套转换**（见 [`unbound_app::scene_upload`]），
//! 否则两边的像素不可比，离屏断言就失去意义。

use std::sync::Arc;

use unbound_app::{collect_backdrops, scene_upload, AppContext};
use unbound_gpu::atlas::{AtlasSet, AtlasUpload};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::{Camera, GpuError, Renderer};
use unbound_math::{Color, Vec2};
use unbound_theme::{ThemeSet, ThemeState};
use unbound_widgets::SceneCollector;

/// 一帧画面。
///
/// 像素以 **BGRA8** 紧凑排列（与 GPU 回读格式一致），每像素 4 字节。
#[derive(Clone, Debug, Default)]
pub struct CapturedFrame {
    /// 画面宽度（物理像素）。
    pub width: u32,
    /// 画面高度（物理像素）。
    pub height: u32,
    /// 逻辑像素到物理像素的比例。
    pub scale: f32,
    /// 紧凑排列的 BGRA8 像素。
    ///
    /// 用 `Arc` 包裹：同一帧常被多处引用（断言、存档、报告），
    /// 拷贝它只该付出引用计数的代价。
    pub pixels: Arc<[u8]>,
}

impl CapturedFrame {
    /// 采样指定逻辑坐标处的颜色。
    ///
    /// 坐标是**逻辑像素**——与语义树的矩形同一坐标系，节点矩形与像素区域
    /// 因此可以直接对账。
    #[must_use]
    pub fn color_at(&self, pos: Vec2) -> Option<Color> {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let x = (pos.x * scale).floor();
        let y = (pos.y * scale).floor();
        // 反向比较：`NaN` 也必须被判为越界，否则会落到 `as u32` 的未定义值上。
        if !(x >= 0.0 && y >= 0.0) {
            return None;
        }
        let x = x as u32;
        let y = y as u32;
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = (y as usize * self.width as usize + x as usize) * 4;
        let slice = self.pixels.get(index..index + 4)?;
        // 回读格式是 BGRA：蓝通道在前。
        Some(Color::from_srgb8(slice[2], slice[1], slice[0], slice[3]))
    }
}

/// 像素来源：框架级的画面采集能力。
///
/// 实现者负责把「当前画面」变成可采样的帧。框架自带离屏实现
/// （[`OffscreenPixelSource`]）；宿主也可以提供窗口回读实现。
pub trait PixelSource {
    /// 采集当前画面。
    ///
    /// 返回 `None` 表示当前无法采集：没有可用设备、尚未渲染、画面为空。
    fn capture(&mut self) -> Option<&CapturedFrame>;

    /// 最近一次采集的画面。
    ///
    /// 默认返回 `None`：不缓存结果的实现可以不提供它。
    fn last_frame(&self) -> Option<&CapturedFrame> {
        None
    }

    /// 采样指定逻辑坐标处的颜色。
    ///
    /// 默认从最近一帧取值。
    fn color_at(&self, pos: Vec2) -> Option<Color> {
        self.last_frame()?.color_at(pos)
    }
}

/// 框架自带的离屏像素来源。
///
/// 它持有**自己的** GPU 设备与渲染器：场景被渲染到离屏目标后回读，
/// 全程不触碰宿主窗口——不占屏幕、不抢鼠标、不改变用户窗口的状态。
///
/// 文字与主题色都依赖图集与查找表，因此本类型需要随应用状态同步：
/// 主题切换时重写 4 KiB 的查找表，字形增量每次渲染前上传。
pub struct OffscreenPixelSource {
    renderer: Renderer,
    atlases: Arc<AtlasSet>,
    /// 已上传查找表的主题键；主题未变时不重复上传。
    theme_key: Option<(
        unbound_theme::ThemeKind,
        unbound_theme::ProductPreset,
        unbound_theme::AccentBrand,
    )>,
    width: u32,
    height: u32,
    scale: f32,
    /// 上传代次：每帧递增，否则渲染器会认为场景未变而跳过上传。
    generation: u64,
    frame: Option<CapturedFrame>,
}

impl OffscreenPixelSource {
    /// 创建离屏像素源。
    ///
    /// `width` 与 `height` 是**物理像素**尺寸；`scale` 是逻辑像素到物理
    /// 像素的比例，通常等于显示缩放比。
    ///
    /// # Errors
    /// 透传 GPU 初始化错误：无 GPU 设备的环境会在这里失败。
    pub fn new(width: u32, height: u32, scale: f32) -> Result<Self, GpuError> {
        let gpu = Gpu::new(GpuConfig::default())?;
        Self::with_gpu(gpu, width, height, scale)
    }

    /// 用已有设备创建离屏像素源。
    ///
    /// 宿主若已经有 GPU 设备，就复用它——多开一个设备会白白吃掉显存。
    /// 测试则可以传入容量更小的设备，避免占用过多资源。
    ///
    /// # Errors
    /// 透传渲染器与图集的初始化错误。
    pub fn with_gpu(gpu: Gpu, width: u32, height: u32, scale: f32) -> Result<Self, GpuError> {
        let mut renderer = Renderer::new(gpu, width, height)?;
        let atlases = Arc::new(AtlasSet::new(&renderer.gpu, 64)?);
        renderer.install_atlases(atlases.clone())?;
        Ok(Self {
            renderer,
            atlases,
            theme_key: None,
            width,
            height,
            scale,
            generation: 0,
            frame: None,
        })
    }

    /// 离屏目标的物理像素宽度。
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// 离屏目标的物理像素高度。
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// 逻辑像素到物理像素的比例。
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// 同步应用状态：主题查找表与字形图集增量。
    ///
    /// 主题查找表只在主题键变化时重写——它是 4 KiB 的常量，不该每帧上传。
    ///
    /// # Errors
    /// 透传 GPU 上传错误。
    pub fn sync(
        &mut self,
        ctx: &AppContext,
        atlas_uploads: &[AtlasUpload],
    ) -> Result<(), GpuError> {
        let key = (ctx.theme, ctx.preset, ctx.accent);
        if self.theme_key != Some(key) {
            let theme = ThemeState::new(ThemeSet::for_product(ctx.preset, ctx.accent), ctx.theme);
            self.renderer.write_theme_lut(theme.lut())?;
            self.theme_key = Some(key);
        }
        if !atlas_uploads.is_empty() {
            self.atlases
                .upload_batch(&self.renderer.gpu, atlas_uploads)?;
        }
        Ok(())
    }

    /// 渲染场景到离屏目标。
    ///
    /// # Errors
    /// 透传 GPU 渲染错误。
    pub fn render(&mut self, scene: &SceneCollector) -> Result<(), GpuError> {
        let backdrops = collect_backdrops(&scene.backdrops, self.scale);
        let camera = Camera::ui((self.width as f32, self.height as f32), self.scale);
        // 上传代次必须每帧递增：版本号相同时渲染器会整帧跳过上传。
        self.generation = self.generation.wrapping_add(1).max(1);
        let upload = scene_upload(scene, &backdrops, camera, self.generation, &[], false);
        self.renderer.render(&upload)?;
        Ok(())
    }

    /// 采集当前画面。
    ///
    /// 画面为空或回读失败时返回 `None`。
    pub fn capture(&mut self) -> Option<&CapturedFrame> {
        let pixels = self.renderer.read_pixels().ok()?;
        self.frame = Some(CapturedFrame {
            width: self.width,
            height: self.height,
            scale: self.scale,
            pixels: Arc::from(pixels),
        });
        self.frame.as_ref()
    }
}

impl PixelSource for OffscreenPixelSource {
    fn capture(&mut self) -> Option<&CapturedFrame> {
        OffscreenPixelSource::capture(self)
    }

    fn last_frame(&self) -> Option<&CapturedFrame> {
        self.frame.as_ref()
    }
}
