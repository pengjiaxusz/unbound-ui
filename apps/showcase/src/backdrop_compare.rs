//! 原生后景与自绘后景的并排对照。
//!
//! 对照必须来自**屏幕合成结果**：渲染器回读只能看到本进程提交的像素，看不到
//! 桌面合成器把系统材质与壁纸合成之后的样子。因此本模块在同一窗口、同一位置下
//! 先切到系统原生后景并等待稳定，再从屏幕设备上下文取样；随后切到引擎自绘后景，
//! 等纹理真正可见后再取样，最后合成并排图与差分热力图。
//!
//! 坐标约定：捕获结果使用屏幕物理像素，并带上实际取样原点；自绘纹理以主屏矩形
//! 为画布，屏幕像素映射到 UV 时必须先减去主屏原点。详细换算见平台层屏幕捕获模块。
//!
//! 本模块只服务人工验收：它需要真实窗口与桌面合成器，不参与任何自动化门禁。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use unbound_backdrop::{BackdropSource, MaterialKind};
use unbound_platform::screen_capture::{
    capture_screen_region, screen_rect_uv, CaptureError, CapturedScreen,
};
use unbound_platform::{ScreenRect, Window};

use crate::png_diff::heat_color;
use crate::shell::GalleryApp;
use crate::shot;

/// 切换到一条路径后至少等待的渲染帧数。
const WARMUP_FRAMES: u32 = 10;
/// 切换到一条路径后至少等待的墙钟时间，覆盖桌面合成器的过渡动画。
const WARMUP_TIME: Duration = Duration::from_millis(600);

/// 纯 CPU 的 BGRA 图像，供并排与差分合成使用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BgraImage {
    /// 像素宽度。
    pub width: u32,
    /// 像素高度。
    pub height: u32,
    /// 行优先 BGRA 字节，长度 = `width * height * 4`。
    pub bgra: Vec<u8>,
}

impl BgraImage {
    /// 从一次屏幕捕获复制出图像。
    #[must_use]
    pub fn from_captured(captured: &CapturedScreen) -> Self {
        Self {
            width: captured.width,
            height: captured.height,
            bgra: captured.bgra.clone(),
        }
    }

    fn empty_like(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bgra: vec![0; width as usize * height as usize * 4],
        }
    }

    fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0 || self.bgra.is_empty()
    }
}

/// 把两张同高的 BGRA 图左右拼接。
///
/// 共同原点取两张图各自的左上角。尺寸不一致时直接报错，而不是裁剪或缩放：
/// 对照的前提是两次捕获覆盖同一块屏幕区域，尺寸不同说明前提已经失效。
///
/// # Errors
/// 任一图像为空，或高度不一致时返回错误说明。
pub fn side_by_side(left: &BgraImage, right: &BgraImage) -> Result<BgraImage, String> {
    if left.is_empty() || right.is_empty() {
        return Err("side-by-side composition needs two non-empty images".to_string());
    }
    if left.height != right.height {
        return Err(format!(
            "side-by-side height mismatch: {} vs {}",
            left.height, right.height
        ));
    }
    let width = left
        .width
        .checked_add(right.width)
        .ok_or_else(|| "side-by-side width overflow".to_string())?;
    let height = left.height;
    let mut bgra = vec![0_u8; width as usize * height as usize * 4];
    let out_stride = width as usize * 4;
    let left_stride = left.width as usize * 4;
    let right_stride = right.width as usize * 4;
    for row in 0..height as usize {
        let out = &mut bgra[row * out_stride..(row + 1) * out_stride];
        out[..left_stride].copy_from_slice(&left.bgra[row * left_stride..(row + 1) * left_stride]);
        out[left_stride..left_stride + right_stride]
            .copy_from_slice(&right.bgra[row * right_stride..(row + 1) * right_stride]);
    }
    Ok(BgraImage {
        width,
        height,
        bgra,
    })
}

/// 计算两张同尺寸 BGRA 图的差分热力图。
///
/// 每个像素取 B/G/R 三个通道差值的最大值，映射到与一致性差分相同的
/// 黑→蓝→绿→黄→红配色，`30` 为满量程。尺寸不一致时直接报错。
///
/// # Errors
/// 任一图像为空，或尺寸不一致时返回错误说明。
pub fn difference_heat(reference: &BgraImage, candidate: &BgraImage) -> Result<BgraImage, String> {
    if reference.is_empty() || candidate.is_empty() {
        return Err("difference heat needs two non-empty images".to_string());
    }
    if reference.width != candidate.width || reference.height != candidate.height {
        return Err(format!(
            "difference heat size mismatch: {}x{} vs {}x{}",
            reference.width, reference.height, candidate.width, candidate.height
        ));
    }
    let mut heat = BgraImage::empty_like(reference.width, reference.height);
    for (index, pixel) in heat.bgra.chunks_exact_mut(4).enumerate() {
        let base = index * 4;
        let diff = (0..3)
            .map(|channel| reference.bgra[base + channel].abs_diff(candidate.bgra[base + channel]))
            .max()
            .unwrap_or(0);
        let delta = f64::from(diff) * 30.0 / 255.0;
        let [red, green, blue] = heat_color(delta);
        pixel[0] = blue;
        pixel[1] = green;
        pixel[2] = red;
        pixel[3] = 255;
    }
    Ok(heat)
}

/// 一次对照运行当前所处的阶段。
enum Phase {
    /// 写入材质选择与原生来源，等待宿主解析。
    Prepare,
    /// 原生路径预热，等待桌面合成器稳定。
    NativeWarmup { frames: u32, since: Instant },
    /// 自绘路径预热，等待壁纸纹理真正可见。
    SelfDrawnWarmup { frames: u32, since: Instant },
}

/// 对照运行的结果。
pub struct CompareReport {
    /// 产物目录。
    pub out_dir: PathBuf,
    /// 原生后景截图。
    pub native: PathBuf,
    /// 自绘后景截图。
    pub self_drawn: PathBuf,
    /// 并排对比图。
    pub side_by_side: PathBuf,
    /// 差分热力图。
    pub heat: PathBuf,
    /// 捕获区域的屏幕物理原点（两次捕获一致）。
    pub capture_origin: ScreenRect,
    /// 捕获尺寸。
    pub capture_size: (u32, u32),
    /// 捕获区域在主屏纹理中的归一化 UV。
    pub texture_uv: [f32; 4],
}

/// 单帧推进的结果。
pub enum CompareStep {
    /// 还在等待渲染或纹理就绪。
    Pending,
    /// 已完成；`Err` 表示无法完成对照。
    Finished(Result<CompareReport, String>),
}

/// 原生与自绘后景的并排对照运行器。
pub struct BackdropCompareRun {
    name: String,
    material: MaterialKind,
    out_dir: PathBuf,
    phase: Phase,
    native: Option<CapturedScreen>,
}

impl BackdropCompareRun {
    /// 构造一次对照运行。
    ///
    /// `name` 是产物文件名前缀，`material` 是被对照的材质种类，`out_dir` 是产物目录。
    #[must_use]
    pub fn new(name: String, material: MaterialKind, out_dir: PathBuf) -> Self {
        Self {
            name,
            material,
            out_dir,
            phase: Phase::Prepare,
            native: None,
        }
    }

    /// 在每个已呈现的帧末尾推进一次。
    ///
    /// 调用方应在返回 [`CompareStep::Pending`] 时继续请求下一帧。
    pub fn step(&mut self, app: &mut GalleryApp, window: &Window) -> CompareStep {
        match self.phase {
            Phase::Prepare => self.prepare(app),
            Phase::NativeWarmup { frames, since } => self.step_native(app, window, frames, since),
            Phase::SelfDrawnWarmup { frames, since } => {
                self.step_self_drawn(app, window, frames, since)
            }
        }
    }

    fn prepare(&mut self, app: &mut GalleryApp) -> CompareStep {
        let material = self.material;
        app.state.set_backdrop_material(material);
        if app.state.backdrop_selection.material != material {
            return CompareStep::Finished(Err(format!(
                "the {} material is not selectable on this machine",
                material.label()
            )));
        }
        app.state.set_backdrop_source(BackdropSource::Native);
        app.sync_backdrop_plan();
        app.state.bump();
        self.phase = Phase::NativeWarmup {
            frames: 0,
            since: Instant::now(),
        };
        CompareStep::Pending
    }

    fn step_native(
        &mut self,
        app: &mut GalleryApp,
        window: &Window,
        frames: u32,
        since: Instant,
    ) -> CompareStep {
        if frames < WARMUP_FRAMES || since.elapsed() < WARMUP_TIME {
            self.phase = Phase::NativeWarmup {
                frames: frames + 1,
                since,
            };
            return CompareStep::Pending;
        }
        if app.state.backdrop_plan.source != BackdropSource::Native {
            return CompareStep::Finished(Err(format!(
                "the native backdrop is unavailable on this machine: {:?}",
                app.state.backdrop_plan.unavailable
            )));
        }
        let captured = match capture_window(window) {
            Ok(captured) => captured,
            Err(error) => return CompareStep::Finished(Err(error)),
        };
        self.native = Some(captured);
        app.state.set_backdrop_source(BackdropSource::SelfDrawn);
        app.sync_backdrop_plan();
        app.state.bump();
        self.phase = Phase::SelfDrawnWarmup {
            frames: 0,
            since: Instant::now(),
        };
        CompareStep::Pending
    }

    fn step_self_drawn(
        &mut self,
        app: &mut GalleryApp,
        window: &Window,
        frames: u32,
        since: Instant,
    ) -> CompareStep {
        let ready = app.state.backdrop_plan.source == BackdropSource::SelfDrawn
            && app.state.backdrop_plan.texture.is_some()
            && app.mica.visible();
        if frames < WARMUP_FRAMES || since.elapsed() < WARMUP_TIME || !ready {
            self.phase = Phase::SelfDrawnWarmup {
                frames: frames + 1,
                since,
            };
            return CompareStep::Pending;
        }
        let Some(native) = self.native.take() else {
            return CompareStep::Finished(Err(
                "the native capture is missing before the self-drawn capture".to_string(),
            ));
        };
        let self_drawn = match capture_window(window) {
            Ok(captured) => captured,
            Err(error) => return CompareStep::Finished(Err(error)),
        };
        CompareStep::Finished(self.write_outputs(&native, &self_drawn))
    }

    fn write_outputs(
        &self,
        native: &CapturedScreen,
        self_drawn: &CapturedScreen,
    ) -> Result<CompareReport, String> {
        let native_image = BgraImage::from_captured(native);
        let self_drawn_image = BgraImage::from_captured(self_drawn);
        let side = side_by_side(&native_image, &self_drawn_image)?;
        let heat = difference_heat(&native_image, &self_drawn_image)?;

        let native_path = self.out_dir.join(format!("{}-native.png", self.name));
        let self_drawn_path = self.out_dir.join(format!("{}-self-drawn.png", self.name));
        let side_path = self.out_dir.join(format!("{}-side-by-side.png", self.name));
        let heat_path = self.out_dir.join(format!("{}-diff-heat.png", self.name));

        shot::write_bgra_png(
            &native_path.to_string_lossy(),
            native.width,
            native.height,
            &native.bgra,
        )?;
        shot::write_bgra_png(
            &self_drawn_path.to_string_lossy(),
            self_drawn.width,
            self_drawn.height,
            &self_drawn.bgra,
        )?;
        shot::write_bgra_png(
            &side_path.to_string_lossy(),
            side.width,
            side.height,
            &side.bgra,
        )?;
        shot::write_bgra_png(
            &heat_path.to_string_lossy(),
            heat.width,
            heat.height,
            &heat.bgra,
        )?;

        let primary = unbound_backdrop::primary_screen_rect()
            .map(|rect| ScreenRect::new(rect.x, rect.y, rect.width, rect.height));
        let texture_uv = primary.map_or([0.0, 0.0, 1.0, 1.0], |screen| {
            screen_rect_uv(native.origin, screen)
        });

        Ok(CompareReport {
            out_dir: self.out_dir.clone(),
            native: native_path,
            self_drawn: self_drawn_path,
            side_by_side: side_path,
            heat: heat_path,
            capture_origin: native.origin,
            capture_size: (native.width, native.height),
            texture_uv,
        })
    }
}

fn capture_window(window: &Window) -> Result<CapturedScreen, String> {
    capture_screen_region(window.client_rect_on_screen()).map_err(|error| match error {
        CaptureError::Unsupported(reason) | CaptureError::Failed(reason) => {
            format!("screen capture failed: {reason}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, pixel: [u8; 4]) -> BgraImage {
        BgraImage {
            width,
            height,
            bgra: pixel.repeat(width as usize * height as usize),
        }
    }

    #[test]
    fn side_by_side_places_the_right_image_after_the_left() {
        let left = image(2, 1, [10, 20, 30, 255]);
        let right = image(3, 1, [40, 50, 60, 255]);
        let composed = side_by_side(&left, &right).expect("matching heights compose");
        assert_eq!((composed.width, composed.height), (5, 1));
        assert_eq!(&composed.bgra[..8], &[10, 20, 30, 255, 10, 20, 30, 255]);
        assert_eq!(&composed.bgra[8..12], &[40, 50, 60, 255]);
    }

    #[test]
    fn side_by_side_rejects_mismatched_heights() {
        let left = image(2, 1, [0, 0, 0, 255]);
        let right = image(2, 2, [0, 0, 0, 255]);
        assert!(side_by_side(&left, &right).is_err());
    }

    #[test]
    fn identical_images_produce_black_heat() {
        let reference = image(2, 2, [12, 34, 56, 255]);
        let heat = difference_heat(&reference, &reference).expect("identical sizes compose");
        assert!(heat
            .bgra
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 0, 255]));
    }

    #[test]
    fn maximum_difference_maps_to_red() {
        let reference = image(1, 1, [0, 0, 0, 255]);
        let candidate = image(1, 1, [255, 255, 255, 255]);
        let heat = difference_heat(&reference, &candidate).expect("identical sizes compose");
        assert_eq!(&heat.bgra[..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn difference_heat_rejects_size_mismatch() {
        let reference = image(2, 2, [0, 0, 0, 255]);
        let candidate = image(2, 3, [0, 0, 0, 255]);
        assert!(difference_heat(&reference, &candidate).is_err());
    }
}
