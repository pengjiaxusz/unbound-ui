#![cfg(all(windows, feature = "backend-directwrite"))]

//! DirectWrite 字形光栅化后端。
//!
//! 只替换字形位图的生成：塑形、折行与命中测试仍由 `cosmic-text` 负责。
//! 这样可以保持排版语义不变，同时让字形覆盖率的抗锯齿行为与 WinUI 使用的
//! DirectWrite 路径同源。
//!
//! 设计取舍：
//!
//! * 单字形独立光栅化，复用现有八相位图集键，避免引入第二套缓存；
//! * 彩色字体（Emoji）走 `TranslateColorGlyphRun`：它把 COLR（含 v1）摊平成
//!   一串实色矢量层，每层复用同一套覆盖率分析，最后交给 [`crate::composite`]
//!   合成。位图层（CBDT / PNG）与 SVG 层不在白名单内，遇到即整簇回退（ADR 0017）；
//! * 彩色层按**簇**翻译：一簇一次调用、一簇一个位图。层跨出本簇字形集合时
//!   整层丢弃而不是裁切——裁切会画出半对半错的图；
//! * 距离场与光栅器无关：`sdf::from_coverage` 的输入就是覆盖率位图。
//!   `glyph.rs` 里的 `!sdf_mode` 是早期接线遗留，解除后 DirectWrite 位图
//!   同样可以进 EDT；
//! * 字体按「家族 + 字重 + 斜体」解析，首次使用时缓存 `IDWriteFontFace`；
//! * 渲染模式取 `NATURAL_SYMMETRIC`（横竖双向抗锯齿）并关闭网格拟合：
//!   本模块按 ⅛ px 相位自行做亚像素定位，网格拟合会把每个相位重新量化到
//!   整像素，与相位缓存互相打架，小字号下表现为硬阶梯。

use core::mem::ManuallyDrop;

use rustc_hash::FxHashMap;
use windows::core::{Interface, BOOL, PCWSTR};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_TEXTURE_ALIASED_1x1, DWriteCreateFactory, IDWriteFactory, IDWriteFactory2,
    IDWriteFactory4, IDWriteFont, IDWriteFontCollection, IDWriteFontFace, IDWriteFontFace2,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_GLYPH_IMAGE_FORMATS_COLR,
    DWRITE_GLYPH_IMAGE_FORMATS_TRUETYPE, DWRITE_GLYPH_OFFSET, DWRITE_GLYPH_RUN,
    DWRITE_GRID_FIT_MODE_DISABLED, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC, DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows_numerics::Vector2;

/// `DWRITE_E_NOCOLOR`：该字形没有彩色层（`windows` crate 未导出这个常量）。
const DWRITE_E_NOCOLOR: windows::core::HRESULT = windows::core::HRESULT(0x8898_500C_u32 as i32);

/// 一次字形光栅化的结果。
pub struct RasterizedGlyph {
    pub width: u32,
    pub height: u32,
    /// 位图左边缘相对画笔原点的水平偏移。
    pub bearing_x: f32,
    /// 位图顶边缘相对基线的垂直偏移；采用 DirectWrite 的 y 向下坐标。
    pub bearing_y: f32,
    /// 单通道覆盖率位图，长度等于 `width * height`。
    pub data: Vec<u8>,
}

/// 一簇里参与彩色层翻译的字形。
#[derive(Copy, Clone, Debug)]
pub struct ClusterGlyph {
    pub glyph_id: u16,
    /// 步进宽度；DWrite 用它推进画笔。
    pub advance: f32,
    /// 相对步进位置的额外水平偏移。
    pub offset_x: f32,
    /// 垂直偏移（向下为正）。
    pub offset_y: f32,
}

/// 一层彩色字形的光栅化结果。
pub struct ColorLayerRaster {
    /// 直通 alpha 的实色，分量为 `0..=1`。
    pub color: [f32; 4],
    pub width: u32,
    pub height: u32,
    /// 位图左缘相对簇原点的水平偏移。
    pub left: i32,
    /// 位图顶缘相对簇原点的垂直偏移（向下为正）。
    pub top: i32,
    /// 单通道覆盖率位图，长度等于 `width * height`。
    pub data: Vec<u8>,
}

/// 彩色簇的翻译结论。
pub enum ColorClusterOutcome {
    /// 翻译出的层序列，交给 [`crate::composite::composite`] 合成。
    Layers(Vec<ColorLayerRaster>),
    /// 该 run 不是彩色字形（`DWRITE_E_NOCOLOR`）：按普通单色 run 处理，不计回退。
    NotColor,
    /// 确实是彩色字体，但这一簇翻译不出可合成的层（格式白名单外 / 跨簇层 / 空包围盒）。
    Unsupported(&'static str),
}

/// 字体面缓存键。
///
/// `fontdb` 为每个命名字体实例分配独立 ID，其家族、字重与斜体信息足以
/// 在系统字体集合中唯一定位同一个 `IDWriteFontFace`。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FaceKey {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
}

/// DirectWrite 字形光栅化器。
pub struct DirectWriteRasterizer {
    factory: IDWriteFactory2,
    collection: IDWriteFontCollection,
    faces: FxHashMap<FaceKey, Option<IDWriteFontFace>>,
    /// 每个字体面是否为彩色字体；`IsColorFont` 每次调用都要过 COM，缓存一次。
    color_faces: FxHashMap<FaceKey, bool>,
}

/// DirectWrite 字形光栅化引擎（别名）。
pub type DirectWriteEngine = DirectWriteRasterizer;

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
            color_faces: FxHashMap::default(),
        })
    }

    /// 解析（并缓存）字体面对应的 `IDWriteFontFace`。
    fn face_for(&mut self, key: &FaceKey) -> Option<IDWriteFontFace> {
        if let Some(cached) = self.faces.get(key) {
            return cached.clone();
        }
        let found = self.find_face(key);
        self.faces.insert(key.clone(), found.clone());
        found
    }

    /// 该字体面是否提供彩色字形。
    ///
    /// `GetGlyphImageFormats` 在本机对所有字形都返回 `NONE`，不能当判据；
    /// `IsColorFont` 只看字体级声明，恰好是这里需要的粒度。
    pub fn is_color_face(&mut self, key: &FaceKey) -> bool {
        if let Some(v) = self.color_faces.get(key) {
            return *v;
        }
        let Some(face) = self.face_for(key) else {
            return false;
        };
        let is_color = match face.cast::<IDWriteFontFace2>() {
            Ok(face2) => unsafe { face2.IsColorFont() }.as_bool(),
            Err(_) => false,
        };
        self.color_faces.insert(key.clone(), is_color);
        is_color
    }

    /// 该字体面提供的调色板数量。
    ///
    /// 面板用它决定「调色板」维度是否灰化：只有一个调色板时无从选择。默认的
    /// `seguiemj.ttf` 是 `CPAL` v0、单调色板（COLRv1 把渐变摊进了调色板条目），
    /// 所以这个维度在本机永远是灰的（ADR 0017 第 8 条）。
    pub fn color_palette_count(&mut self, key: &FaceKey) -> Option<u32> {
        let face = self.face_for(key)?;
        let face2 = face.cast::<IDWriteFontFace2>().ok()?;
        Some(unsafe { face2.GetColorPaletteCount() })
    }

    /// 光栅化单个字形。
    ///
    /// `key` 指定字体家族、字重与斜体；`phase_offset` 为水平亚像素偏移量。
    /// 返回 `None` 时调用方应回退到 `swash`，包括：
    /// 字体家族缺失、彩色字体、或 DirectWrite 拒绝该字形。
    pub fn rasterize(
        &mut self,
        key: &FaceKey,
        glyph_id: u16,
        font_size: f32,
        advance: f32,
        phase_offset: f32,
        mono: bool,
    ) -> Option<RasterizedGlyph> {
        let face = self.face_for(key)?;

        // 彩色字体需要调色板渲染，DirectWrite 的单通道纹理拿不到颜色层。
        // 在单色模式下允许覆盖率光栅化。
        if !mono {
            if let Ok(face2) = face.cast::<IDWriteFontFace2>() {
                if unsafe { face2.IsColorFont() }.as_bool() {
                    return None;
                }
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
                phase_offset,
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

    /// 翻译一簇彩色字形的层序列。
    ///
    /// `glyphs` 只含本簇的字形，调用方负责按簇切分；`palette_index` 透传给
    /// `TranslateColorGlyphRun`（本轮恒为 0，见 ADR 0017）。
    ///
    /// 层的包围盒来自 `CreateGlyphRunAnalysis`，坐标系与单字形路径一致：
    /// 基线为原点、Y 向上，因此这里统一翻转成「相对簇原点、Y 向下」。
    pub fn rasterize_color_cluster(
        &mut self,
        key: &FaceKey,
        glyphs: &[ClusterGlyph],
        font_size: f32,
        palette_index: u32,
    ) -> ColorClusterOutcome {
        if glyphs.is_empty() {
            return ColorClusterOutcome::NotColor;
        }
        if !self.is_color_face(key) {
            return ColorClusterOutcome::NotColor;
        }
        let Some(face) = self.face_for(key) else {
            return ColorClusterOutcome::Unsupported("font face resolution failed");
        };
        let Ok(factory4) = self.factory.cast::<IDWriteFactory4>() else {
            return ColorClusterOutcome::Unsupported(
                "this DirectWrite version does not expose color layer traversal",
            );
        };

        let indices: Vec<u16> = glyphs.iter().map(|g| g.glyph_id).collect();
        let advances: Vec<f32> = glyphs.iter().map(|g| g.advance.max(0.0)).collect();
        let offsets: Vec<DWRITE_GLYPH_OFFSET> = glyphs
            .iter()
            .map(|g| DWRITE_GLYPH_OFFSET {
                advanceOffset: g.offset_x,
                ascenderOffset: g.offset_y,
            })
            .collect();

        let mut run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(face)),
            fontEmSize: font_size,
            glyphCount: indices.len() as u32,
            glyphIndices: indices.as_ptr(),
            glyphAdvances: advances.as_ptr(),
            glyphOffsets: offsets.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };

        let translated = unsafe {
            factory4.TranslateColorGlyphRun(
                Vector2 { X: 0.0, Y: 0.0 },
                &run,
                None,
                DWRITE_GLYPH_IMAGE_FORMATS_TRUETYPE | DWRITE_GLYPH_IMAGE_FORMATS_COLR,
                DWRITE_MEASURING_MODE_NATURAL,
                None,
                palette_index,
            )
        };
        unsafe { ManuallyDrop::drop(&mut run.fontFace) };

        let enumerator = match translated {
            Ok(en) => en,
            Err(e) if e.code() == DWRITE_E_NOCOLOR => return ColorClusterOutcome::NotColor,
            Err(_) => return ColorClusterOutcome::Unsupported("TranslateColorGlyphRun failed"),
        };

        let allowed = DWRITE_GLYPH_IMAGE_FORMATS_TRUETYPE.0 | DWRITE_GLYPH_IMAGE_FORMATS_COLR.0;
        let mut layers = Vec::new();
        loop {
            let Ok(has) = (unsafe { enumerator.MoveNext() }) else {
                break;
            };
            if !has.as_bool() {
                break;
            }
            let Ok(current) = (unsafe { enumerator.GetCurrentRun() }) else {
                break;
            };
            if current.is_null() {
                break;
            }
            let layer = unsafe { &*current };

            // 白名单之外（SVG / PNG / JPEG / 画笔树）没有对应的光栅化路径，
            // 整簇判为不可合成，交给上层按回退策略处理。
            if layer.glyphImageFormat.0 & !allowed != 0 {
                return ColorClusterOutcome::Unsupported(
                    "color layer format is outside the TT|COLR whitelist",
                );
            }

            // 层里的 glyph id **未必**等于请求的字形：COLRv1 的复合画笔会引用
            // 字体里的其它字形来拼装这一层。所以“是否跨簇”不能靠 id 子集判断；
            // 按簇构造 run 已经从源头保证 DWrite 看不到别的簇——真出现跨簇层时，
            // 它引用的是字体内部字形，画出来仍属于本簇。
            if layer.Base.glyphRun.glyphCount == 0 {
                continue;
            }

            let color = layer.Base.runColor;
            if color.a <= 0.0 {
                continue;
            }

            let analysis = unsafe {
                self.factory.CreateGlyphRunAnalysis(
                    &layer.Base.glyphRun,
                    None,
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                    DWRITE_MEASURING_MODE_NATURAL,
                    DWRITE_GRID_FIT_MODE_DISABLED,
                    DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                    0.0,
                    0.0,
                )
            };
            let Ok(analysis) = analysis else {
                return ColorClusterOutcome::Unsupported("layer coverage analysis failed");
            };
            let Ok(bounds) =
                (unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1) })
            else {
                return ColorClusterOutcome::Unsupported("layer coverage bounds unavailable");
            };
            let width = (bounds.right - bounds.left).max(0) as u32;
            let height = (bounds.bottom - bounds.top).max(0) as u32;
            if width == 0 || height == 0 {
                continue;
            }
            let mut data = vec![0u8; (width * height) as usize];
            if unsafe {
                analysis.CreateAlphaTexture(DWRITE_TEXTURE_ALIASED_1x1, &bounds, &mut data)
            }
            .is_err()
            {
                return ColorClusterOutcome::Unsupported("layer coverage texture read failed");
            }

            layers.push(ColorLayerRaster {
                color: [color.r, color.g, color.b, color.a],
                width,
                height,
                left: bounds.left + layer.Base.baselineOriginX.round() as i32,
                top: -(bounds.top + layer.Base.baselineOriginY.round() as i32),
                data,
            });
        }

        if layers.is_empty() {
            ColorClusterOutcome::Unsupported("no composable color layers")
        } else {
            ColorClusterOutcome::Layers(layers)
        }
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
