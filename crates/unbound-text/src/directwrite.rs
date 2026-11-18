//! DirectWrite 字形光栅化后端。
//!
//! 只替换字形位图的生成：塑形、折行与命中测试仍由 `cosmic-text` 负责。
//! 这样可以保持排版语义不变，同时让字形覆盖率的抗锯齿行为与 WinUI 使用的
//! DirectWrite 路径同源。
//!
//! 设计取舍：
//!
//! * 单字形独立光栅化，复用现有八相位图集键，避免引入第二套缓存；
//! * 彩色字体（Emoji）与距离场仍走 `swash`，因为 DirectWrite 的彩色
//!   字形需要调色板遍历，距离场也需要轮廓数据；
//! * 字体按「家族 + 字重 + 斜体」解析，首次使用时缓存 `IDWriteFontFace`；
//! * 渲染模式取 `NATURAL_SYMMETRIC`（横竖双向抗锯齿）并关闭网格拟合：
//!   本模块按 ⅛ px 相位自行做亚像素定位，网格拟合会把每个相位重新量化到
//!   整像素，与相位缓存互相打架，小字号下表现为硬阶梯。

use core::mem::ManuallyDrop;

use rustc_hash::FxHashMap;
use windows::core::{Interface, BOOL, PCWSTR};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_TEXTURE_ALIASED_1x1, DWriteCreateFactory, IDWriteFactory, IDWriteFactory2, IDWriteFont,
    IDWriteFontCollection, IDWriteFontFace, IDWriteFontFace2, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_GLYPH_OFFSET, DWRITE_GLYPH_RUN, DWRITE_GRID_FIT_MODE_DISABLED,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
    DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};

/// 一次字形光栅化的结果。
pub(crate) struct RasterizedGlyph {
    pub width: u32,
    pub height: u32,
    /// 位图左边缘相对画笔原点的水平偏移。
    pub bearing_x: f32,
    /// 位图顶边缘相对基线的垂直偏移；采用 DirectWrite 的 y 向下坐标。
    pub bearing_y: f32,
    /// 单通道覆盖率位图，长度等于 `width * height`。
    pub data: Vec<u8>,
}

/// 字体面缓存键。
///
/// `fontdb` 为每个命名字体实例分配独立 ID，其家族、字重与斜体信息足以
/// 在系统字体集合中唯一定位同一个 `IDWriteFontFace`。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FaceKey {
    pub(crate) family: String,
    pub(crate) weight: u16,
    pub(crate) italic: bool,
}

/// DirectWrite 字形光栅化器。
pub(crate) struct DirectWriteRasterizer {
    factory: IDWriteFactory2,
    collection: IDWriteFontCollection,
    faces: FxHashMap<FaceKey, Option<IDWriteFontFace>>,
}

impl DirectWriteRasterizer {
    /// 创建共享 DirectWrite 工厂与系统字体集合。
    ///
    /// 返回 `None` 表示当前环境不可用（缺少 DirectWrite 或系统字体集合），
    /// 调用方应回退到 `swash`。
    pub fn new() -> Option<Self> {
        let factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.ok()?;
        let factory2: IDWriteFactory2 = factory.cast().ok()?;
        let mut collection = None;
        unsafe { factory.GetSystemFontCollection(&mut collection, false) }.ok()?;
        Some(Self {
            factory: factory2,
            collection: collection?,
            faces: FxHashMap::default(),
        })
    }

    /// 光栅化单个字形。
    ///
    /// `key` 指定字体家族、字重与斜体；`phase` 为 0..7 的亚像素相位，基线的
    /// 水平原点取 `phase * 0.125`。返回 `None` 时调用方应回退到 `swash`，包括：
    /// 字体家族缺失、彩色字体、或 DirectWrite 拒绝该字形。
    pub fn rasterize(
        &mut self,
        key: &FaceKey,
        glyph_id: u16,
        font_size: f32,
        advance: f32,
        phase: u8,
    ) -> Option<RasterizedGlyph> {
        let face = if let Some(cached) = self.faces.get(key) {
            cached.clone()?
        } else {
            let found = self.find_face(key);
            self.faces.insert(key.clone(), found.clone());
            found?
        };

        // 彩色字体需要调色板渲染，DirectWrite 的单通道纹理拿不到颜色层。
        if let Ok(face2) = face.cast::<IDWriteFontFace2>() {
            if unsafe { face2.IsColorFont() }.as_bool() {
                return None;
            }
        }

        let indices = [glyph_id];
        let advances = [advance.max(0.0)];
        let offsets = [DWRITE_GLYPH_OFFSET::default()];
        let mut run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(face)),
            fontEmSize: font_size,
            glyphCount: 1,
            glyphIndices: indices.as_ptr(),
            glyphAdvances: advances.as_ptr(),
            glyphOffsets: offsets.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };
        let analysis = unsafe {
            self.factory.CreateGlyphRunAnalysis(
                &run,
                None,
                DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_GRID_FIT_MODE_DISABLED,
                DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                f32::from(phase) * 0.125,
                0.0,
            )
        };
        // `DWRITE_GLYPH_RUN` 用 `ManuallyDrop` 持有接口，必须显式释放引用。
        unsafe { ManuallyDrop::drop(&mut run.fontFace) };
        let analysis = analysis.ok()?;

        // 纹理类型必须与抗锯齿模式自洽。
        //
        // `DWRITE_TEXTURE_CLEARTYPE_3x1` 只在 `DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE`
        // 下可用；在灰度模式下 `GetAlphaTextureBounds` 会返回空矩形，旧代码因此
        // 每次都落进下面这条回退分支，注释却声称走的是 ClearType。本管线在
        // R8 图集里只存单通道覆盖率、并在线性空间与任意后景混合，本来也用不上
        // 子像素配色，所以这里直接请求 8 位灰度覆盖率。
        let texture = DWRITE_TEXTURE_ALIASED_1x1;
        let bounds = unsafe { analysis.GetAlphaTextureBounds(texture) }.ok()?;
        let width = (bounds.right - bounds.left).max(0) as u32;
        let height = (bounds.bottom - bounds.top).max(0) as u32;
        if width == 0 || height == 0 {
            return Some(RasterizedGlyph {
                width: 0,
                height: 0,
                bearing_x: 0.0,
                bearing_y: 0.0,
                data: Vec::new(),
            });
        }
        let mut data = vec![0u8; (width * height) as usize];
        unsafe { analysis.CreateAlphaTexture(texture, &bounds, &mut data) }.ok()?;
        Some(RasterizedGlyph {
            width,
            height,
            bearing_x: bounds.left as f32,
            bearing_y: bounds.top as f32,
            data,
        })
    }

    fn find_face(&self, key: &FaceKey) -> Option<IDWriteFontFace> {
        // cosmic-text/fontdb 暴露的可变字体家族名是 `Segoe UI Variable`，
        // 而 DirectWrite 的系统集合按光学尺寸拆成 `... Text` / `... Display`。
        // 直接按 fontdb 名字查找会全部失败并静默回退到 swash，导致文字
        // 抗锯齿与 WinUI 不一致。这里按正文优先尝试别名。
        let mut candidates = Vec::with_capacity(4);
        if key.family.eq_ignore_ascii_case("Segoe UI Variable") {
            candidates.push("Segoe UI Variable Text");
            candidates.push("Segoe UI Variable Display");
        }
        candidates.push(key.family.as_str());
        if !key.family.eq_ignore_ascii_case("Segoe UI") {
            candidates.push("Segoe UI");
        }
        for family in candidates {
            if let Some(face) = self.find_face_in_family(key, family) {
                return Some(face);
            }
        }
        None
    }

    fn find_face_in_family(&self, key: &FaceKey, family_name: &str) -> Option<IDWriteFontFace> {
        let name = windows::core::HSTRING::from(family_name);
        let mut index = 0u32;
        let mut exists = BOOL(0);
        unsafe {
            self.collection
                .FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists)
        }
        .ok()?;
        if !exists.as_bool() {
            return None;
        }
        let family = unsafe { self.collection.GetFontFamily(index) }.ok()?;
        let style = if key.italic {
            DWRITE_FONT_STYLE_ITALIC
        } else {
            DWRITE_FONT_STYLE_NORMAL
        };
        let list = unsafe {
            family.GetMatchingFonts(
                DWRITE_FONT_WEIGHT(i32::from(key.weight)),
                DWRITE_FONT_STRETCH_NORMAL,
                style,
            )
        }
        .ok()?;
        if unsafe { list.GetFontCount() } == 0 {
            return None;
        }
        let font: IDWriteFont = unsafe { list.GetFont(0) }.ok()?;
        unsafe { font.CreateFontFace() }.ok()
    }
}

/// 在共享的 DirectWrite 文本布局上执行一次测量。
///
/// DirectWrite 的文本布局按真实字距（kerning）给出每个字形簇的水平原点和
/// 总宽度；`cosmic-text` 的前进宽度累计与之在 14px 正文上相差零点几像素，
/// 长串标签会逐步累积。控件布局与像素对齐都需要与 WinUI 使用同一套度量。
///
/// 字体格式按「家族 + 字重 + 斜体 + 字号」缓存；布局本身每次重建。
fn with_text_layout<R>(
    family: &str,
    weight: u16,
    italic: bool,
    font_size: f32,
    text: &str,
    f: impl FnOnce(&windows::Win32::Graphics::DirectWrite::IDWriteTextLayout) -> Option<R>,
) -> Option<R> {
    use std::sync::{Mutex, OnceLock};
    use windows::core::HSTRING;
    use windows::Win32::Graphics::DirectWrite::{IDWriteTextFormat, IDWriteTextLayout};

    struct MeasureCache {
        factory: IDWriteFactory2,
        collection: IDWriteFontCollection,
        formats: FxHashMap<(String, u16, bool, u32), IDWriteTextFormat>,
    }

    static CACHE: OnceLock<Mutex<Option<MeasureCache>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        let factory: Option<IDWriteFactory> =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.ok();
        let mut state = None;
        if let Some(factory) = factory {
            if let Ok(factory2) = factory.cast::<IDWriteFactory2>() {
                let mut collection = None;
                if unsafe { factory.GetSystemFontCollection(&mut collection, false) }.is_ok() {
                    if let Some(collection) = collection {
                        state = Some(MeasureCache {
                            factory: factory2,
                            collection,
                            formats: FxHashMap::default(),
                        });
                    }
                }
            }
        }
        Mutex::new(state)
    });
    let mut guard = cache.lock().ok()?;
    let cache = guard.as_mut()?;

    let locale = HSTRING::from("en-us");
    let key = (family.to_owned(), weight, italic, font_size.to_bits());
    if !cache.formats.contains_key(&key) {
        let style = if italic {
            DWRITE_FONT_STYLE_ITALIC
        } else {
            DWRITE_FONT_STYLE_NORMAL
        };
        // 与光栅化器一致：`fontdb` 暴露的 `Segoe UI Variable` 在 DirectWrite
        // 的系统集合里按光学尺寸拆成 `... Text` / `... Display`。
        let mut candidates: Vec<&str> = Vec::with_capacity(4);
        if family.eq_ignore_ascii_case("Segoe UI Variable") {
            candidates.push("Segoe UI Variable Text");
            candidates.push("Segoe UI Variable Display");
        }
        candidates.push(family);
        if !family.eq_ignore_ascii_case("Segoe UI") {
            candidates.push("Segoe UI");
        }
        let mut format: Option<IDWriteTextFormat> = None;
        for candidate in candidates {
            let name = HSTRING::from(candidate);
            let mut index = 0u32;
            let mut exists = BOOL(0);
            if unsafe {
                cache
                    .collection
                    .FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists)
            }
            .is_err()
                || !exists.as_bool()
            {
                continue;
            }
            if let Ok(created) = unsafe {
                cache.factory.CreateTextFormat(
                    PCWSTR(name.as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT(i32::from(weight)),
                    style,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    PCWSTR(locale.as_ptr()),
                )
            } {
                format = Some(created);
                break;
            }
        }
        let format = format?;
        cache.formats.insert(key.clone(), format);
    }
    let format = cache.formats.get(&key)?;

    let units: Vec<u16> = text.encode_utf16().collect();
    let layout: IDWriteTextLayout = unsafe {
        cache
            .factory
            .CreateTextLayout(&units, format, f32::MAX, f32::MAX)
    }
    .ok()?;
    f(&layout)
}

/// 每个字形簇的水平原点与布局总宽度。
///
/// `utf16_indices` 给出各字形簇在 UTF-16 文本中的起始下标（与 `text` 同一
/// 字符串）。返回的 x 相对布局原点，单位与 `font_size` 相同。
pub(crate) fn layout_cluster_positions(
    family: &str,
    weight: u16,
    italic: bool,
    font_size: f32,
    text: &str,
    utf16_indices: &[u32],
) -> Option<(Vec<f32>, f32)> {
    with_text_layout(family, weight, italic, font_size, text, |layout| {
        use windows::Win32::Graphics::DirectWrite::{DWRITE_HIT_TEST_METRICS, DWRITE_TEXT_METRICS};
        let mut positions = Vec::with_capacity(utf16_indices.len());
        for &index in utf16_indices {
            let mut x = 0.0f32;
            let mut y = 0.0f32;
            let mut hit = DWRITE_HIT_TEST_METRICS::default();
            unsafe { layout.HitTestTextPosition(index, false, &mut x, &mut y, &mut hit) }.ok()?;
            positions.push(x);
        }
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut metrics) }.ok()?;
        Some((positions, metrics.widthIncludingTrailingWhitespace))
    })
}
