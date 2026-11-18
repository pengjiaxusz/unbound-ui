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
    DWRITE_TEXTURE_ALIASED_1x1, DWRITE_TEXTURE_CLEARTYPE_3x1, DWriteCreateFactory, IDWriteFactory,
    IDWriteFactory2, IDWriteFont, IDWriteFontCollection, IDWriteFontFace, IDWriteFontFace2,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_GLYPH_OFFSET, DWRITE_GLYPH_RUN,
    DWRITE_GRID_FIT_MODE_DEFAULT, DWRITE_MEASURING_MODE_NATURAL, DWRITE_RENDERING_MODE_NATURAL,
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
                DWRITE_RENDERING_MODE_NATURAL,
                DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_GRID_FIT_MODE_DEFAULT,
                DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                f32::from(phase) * 0.125,
                0.0,
            )
        };
        // `DWRITE_GLYPH_RUN` 用 `ManuallyDrop` 持有接口，必须显式释放引用。
        unsafe { ManuallyDrop::drop(&mut run.fontFace) };
        let analysis = analysis.ok()?;

        // 灰度抗锯齿优先取 ClearType 3x1 纹理再折算成单通道：
        // `DWRITE_TEXTURE_ALIASED_1x1` 只给 1-bit 覆盖率，会让字形边缘出现
        // 参考截图里没有的锯齿，是跨渲染器文字差异的主要来源。
        // 部分渲染模式/字体下 ClearType 纹理不可用，此时回退到 1x1 覆盖率，
        // 保证字形不会整块丢失。
        let texture = DWRITE_TEXTURE_CLEARTYPE_3x1;
        if let Ok(bounds) = unsafe { analysis.GetAlphaTextureBounds(texture) } {
            let width = (bounds.right - bounds.left).max(0) as u32;
            let height = (bounds.bottom - bounds.top).max(0) as u32;
            if width > 0 && height > 0 {
                let mut rgb = vec![0u8; (width * height * 3) as usize];
                if unsafe { analysis.CreateAlphaTexture(texture, &bounds, &mut rgb) }.is_ok() {
                    // ClearType 的三个子像素通道按人眼亮度权重合成灰度覆盖率。
                    let data = rgb
                        .chunks_exact(3)
                        .map(|px| {
                            ((u16::from(px[0]) * 3 + u16::from(px[1]) * 6 + u16::from(px[2])) / 10)
                                as u8
                        })
                        .collect();
                    return Some(RasterizedGlyph {
                        width,
                        height,
                        bearing_x: bounds.left as f32,
                        bearing_y: bounds.top as f32,
                        data,
                    });
                }
            }
        }

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
