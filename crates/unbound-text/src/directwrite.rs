//! DirectWrite 字形光栅化后端。
//!
//! 只替换字形位图的生成：塑形、折行与命中测试仍由 `cosmic-text` 负责。
//! 这样可以保持排版语义不变，同时让覆盖率位图的抗锯齿、网格拟合与
//! WinUI 使用的 DirectWrite 渲染路径保持一致。
//!
//! 设计取舍：
//!
//! * 单字形独立光栅化，复用现有四相位图集键，避免引入第二套缓存；
//! * 彩色字体（Emoji）与距离场仍走 `swash`，因为 DirectWrite 的彩色
//!   字形需要调色板遍历，距离场也需要轮廓数据；
//! * 字体按「家族 + 字重 + 斜体」解析，首次使用时缓存 `IDWriteFontFace`。

use core::mem::ManuallyDrop;

use rustc_hash::FxHashMap;
use windows::core::{Interface, BOOL, PCWSTR};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_TEXTURE_ALIASED_1x1, DWriteCreateFactory, IDWriteFactory, IDWriteFactory2, IDWriteFont,
    IDWriteFontCollection, IDWriteFontFace, IDWriteFontFace2, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_GLYPH_OFFSET, DWRITE_GLYPH_RUN, DWRITE_GRID_FIT_MODE_DEFAULT,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_RENDERING_MODE_NATURAL,
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
struct FaceKey {
    family: String,
    weight: u16,
    italic: bool,
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
    /// `phase` 为 0..3 的亚像素相位，基线的水平原点取 `phase * 0.25`。
    /// 返回 `None` 时调用方应回退到 `swash`，包括：字体家族缺失、彩色字体、
    /// 或 DirectWrite 拒绝该字形。
    pub fn rasterize(
        &mut self,
        family: &str,
        weight: u16,
        italic: bool,
        glyph_id: u16,
        font_size: f32,
        advance: f32,
        phase: u8,
    ) -> Option<RasterizedGlyph> {
        let key = FaceKey {
            family: family.to_owned(),
            weight,
            italic,
        };
        let face = if let Some(cached) = self.faces.get(&key) {
            cached.clone()?
        } else {
            let found = self.find_face(&key);
            self.faces.insert(key, found.clone());
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
                DWRITE_RENDERING_MODE_NATURAL,
                DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_GRID_FIT_MODE_DEFAULT,
                DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                f32::from(phase) * 0.25,
                0.0,
            )
        };
        // `DWRITE_GLYPH_RUN` 用 `ManuallyDrop` 持有接口，必须显式释放引用。
        unsafe { ManuallyDrop::drop(&mut run.fontFace) };
        let analysis = analysis.ok()?;

        let bounds = unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1) }.ok()?;
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
        unsafe { analysis.CreateAlphaTexture(DWRITE_TEXTURE_ALIASED_1x1, &bounds, &mut data) }
            .ok()?;
        Some(RasterizedGlyph {
            width,
            height,
            bearing_x: bounds.left as f32,
            bearing_y: bounds.top as f32,
            data,
        })
    }

    fn find_face(&self, key: &FaceKey) -> Option<IDWriteFontFace> {
        let name = windows::core::HSTRING::from(key.family.as_str());
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
