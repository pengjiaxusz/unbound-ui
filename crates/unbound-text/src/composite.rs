//! 彩色簇的层合成：把一簇彩色字形摊平后的实色层压成一张直通 alpha 的 RGBA8 位图。
//!
//! 本模块是纯值层：不碰 COM、不碰 GPU、不依赖平台，可在无显示器环境单测。
//! 平台侧只负责把「一层实色 + 一份覆盖率 + 一个偏移」交进来，合成规则只有这一份。
//!
//! 坐标系与文本四边形一致：原点在簇的画笔原点（基线），Y 轴向下增长。
//!
//! 合成内部按预乘 alpha 做 over，写出前除回 alpha——图集契约是**直通 alpha**
//! （见 ADR 0017），`uber.frag` 的彩色分支直接取 `t.rgb`，不能喂预乘值。

use core::hash::{Hash, Hasher};
use core::ops::Range;

use crate::shaping::PositionedGlyph;

/// 一层的实色，分量为 `0..=1` 的直通 alpha 颜色。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LayerColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl LayerColor {
    /// 构造一层实色。
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

/// 一层已光栅化的覆盖率位图，以及它在簇坐标系里的位置。
#[derive(Copy, Clone, Debug)]
pub struct ColorLayer<'a> {
    /// 该层的实色。
    pub color: LayerColor,
    /// 单通道覆盖率，长度必须是 `width * height`。
    pub coverage: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// 位图左上角相对簇原点的水平偏移。
    pub left: i32,
    /// 位图左上角相对簇原点的垂直偏移（向下为正）。
    pub top: i32,
}

/// 一层已解码的彩色位图，以及它在簇坐标系里的位置。
///
/// 与 [`ColorLayer`] 的差别只在像素来源：位图层的颜色写在像素里，没有
/// 「实色 × 覆盖率」这层抽象。`rgba` 必须是**直通 alpha 的 RGBA8**，平台侧
/// 负责把 `BGRA` / 预乘的输入转到位（见 `directwrite::decode_bitmap_layer`）。
#[derive(Copy, Clone, Debug)]
pub struct BitmapLayer<'a> {
    /// 直通 alpha 的 RGBA8，长度必须是 `width * height * 4`。
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// 位图左上角相对簇原点的水平偏移。
    pub left: i32,
    /// 位图左上角相对簇原点的垂直偏移（向下为正）。
    pub top: i32,
}

/// 一簇里的一层，按像素来源分两种。
///
/// 矢量层（COLR / `TrueType` 轮廓）是「实色 × 覆盖率」，位图层（CBDT / PNG /
/// JPEG）是「像素自带颜色与 alpha」。两者只在这一处不同，之后的拼装、over
/// 与包围盒计算完全共用。
#[derive(Copy, Clone, Debug)]
pub enum Layer<'a> {
    /// 实色 × 单通道覆盖率。
    Solid(ColorLayer<'a>),
    /// 已解码的彩色位图。
    Bitmap(BitmapLayer<'a>),
}

impl Layer<'_> {
    /// 层矩形是否可见。零尺寸层与全透明层不参与包围盒。
    ///
    /// 位图层没有「实色 alpha」可查，只能扫一遍像素：全透明的位图若参与
    /// 包围盒，会把合成结果的尺寸撑大而内容仍是空白，白占一块图集。
    fn is_visible(&self) -> bool {
        match self {
            Self::Solid(l) => l.color.a > 0.0 && l.width > 0 && l.height > 0,
            Self::Bitmap(b) => {
                b.width > 0
                    && b.height > 0
                    && b.rgba.len() == (b.width as usize) * (b.height as usize) * 4
                    && b.rgba.chunks_exact(4).any(|px| px[3] > 0)
            }
        }
    }

    fn rect(&self) -> (i32, i32, u32, u32) {
        match self {
            Self::Solid(l) => (l.left, l.top, l.width, l.height),
            Self::Bitmap(b) => (b.left, b.top, b.width, b.height),
        }
    }
}

/// 合成产物：直通 alpha 的 RGBA8 位图。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositeBitmap {
    pub width: u32,
    pub height: u32,
    /// 行优先 RGBA8，直通 alpha。
    pub data: Vec<u8>,
    /// 位图左上角相对簇原点的水平偏移。
    pub left: i32,
    /// 位图左上角相对簇原点的垂直偏移（向下为正）。
    pub top: i32,
}

/// 合成结论。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompositeOutcome {
    /// 至少有一层可见。
    Color(CompositeBitmap),
    /// 没有任何可见层：调用方应把这一簇当作普通单色 run 处理，不计回退。
    NotColor,
}

/// 把一簇的层序列压成一张位图。
///
/// 包围盒取各层矩形的并集；完全透明的层（`a == 0`）与零尺寸层被跳过。
/// 重叠处按顺序做 over：靠后的层在上。
#[must_use]
pub fn composite(layers: &[ColorLayer<'_>]) -> CompositeOutcome {
    let mapped: Vec<Layer<'_>> = layers.iter().map(|l| Layer::Solid(*l)).collect();
    composite_layers(&mapped)
}

/// 把一簇的混合层序列（矢量层 + 位图层）压成一张位图。
///
/// 与 [`composite`] 同一套规则：并集包围盒、按序 over、内部预乘、落盘直通。
/// 位图层直接取像素里的 alpha，不再乘一个实色。
#[must_use]
pub fn composite_layers(layers: &[Layer<'_>]) -> CompositeOutcome {
    if !layers.iter().any(Layer::is_visible) {
        return CompositeOutcome::NotColor;
    }

    let mut min_left = i32::MAX;
    let mut min_top = i32::MAX;
    let mut max_right = i32::MIN;
    let mut max_bottom = i32::MIN;
    for layer in layers.iter().filter(|l| l.is_visible()) {
        let (left, top, w, h) = layer.rect();
        min_left = min_left.min(left);
        min_top = min_top.min(top);
        max_right = max_right.max(left.saturating_add(w as i32));
        max_bottom = max_bottom.max(top.saturating_add(h as i32));
    }

    let width = (max_right - min_left).max(0) as u32;
    let height = (max_bottom - min_top).max(0) as u32;
    if width == 0 || height == 0 {
        return CompositeOutcome::NotColor;
    }

    // 预乘累加缓冲：每像素 4 个 f32（r·a, g·a, b·a, a）。
    let mut acc = vec![0.0f32; (width as usize) * (height as usize) * 4];
    let stride = width as usize;

    for layer in layers.iter().filter(|l| l.is_visible()) {
        let (left, top, lw, lh) = layer.rect();
        let dx = (left - min_left) as usize;
        let dy = (top - min_top) as usize;
        match layer {
            Layer::Solid(l) => {
                debug_assert_eq!(
                    l.coverage.len(),
                    (lw as usize) * (lh as usize),
                    "coverage bitmap size does not match the declared rect"
                );
                if l.coverage.len() != (lw as usize) * (lh as usize) {
                    continue;
                }
                for y in 0..lh as usize {
                    let dst_row = (dy + y) * stride + dx;
                    let src_row = y * lw as usize;
                    for x in 0..lw as usize {
                        let c = l.coverage[src_row + x];
                        if c == 0 {
                            continue;
                        }
                        let sa = l.color.a * (f32::from(c) / 255.0);
                        if sa <= 0.0 {
                            continue;
                        }
                        blend(
                            &mut acc,
                            (dst_row + x) * 4,
                            l.color.r,
                            l.color.g,
                            l.color.b,
                            sa,
                        );
                    }
                }
            }
            Layer::Bitmap(b) => {
                debug_assert_eq!(
                    b.rgba.len(),
                    (lw as usize) * (lh as usize) * 4,
                    "rgba bitmap size does not match the declared rect"
                );
                if b.rgba.len() != (lw as usize) * (lh as usize) * 4 {
                    continue;
                }
                for y in 0..lh as usize {
                    let dst_row = (dy + y) * stride + dx;
                    let src_row = y * lw as usize;
                    for x in 0..lw as usize {
                        let i = (src_row + x) * 4;
                        let sa = f32::from(b.rgba[i + 3]) / 255.0;
                        if sa <= 0.0 {
                            continue;
                        }
                        blend(
                            &mut acc,
                            (dst_row + x) * 4,
                            f32::from(b.rgba[i]) / 255.0,
                            f32::from(b.rgba[i + 1]) / 255.0,
                            f32::from(b.rgba[i + 2]) / 255.0,
                            sa,
                        );
                    }
                }
            }
        }
    }

    let mut data = vec![0u8; (width as usize) * (height as usize) * 4];
    for (px, chunk) in acc.chunks_exact(4).zip(data.chunks_exact_mut(4)) {
        let a = px[3];
        if a <= 0.0 {
            continue;
        }
        chunk[0] = to_u8(px[0] / a);
        chunk[1] = to_u8(px[1] / a);
        chunk[2] = to_u8(px[2] / a);
        chunk[3] = to_u8(a);
    }

    CompositeOutcome::Color(CompositeBitmap {
        width,
        height,
        data,
        left: min_left,
        top: min_top,
    })
}

/// 把一层直通 alpha 的实色按 `sa` 叠加到预乘累加缓冲上（source-over）。
#[inline]
fn blend(acc: &mut [f32], i: usize, r: f32, g: f32, b: f32, sa: f32) {
    let inv = 1.0 - sa;
    acc[i] = r * sa + acc[i] * inv;
    acc[i + 1] = g * sa + acc[i + 1] * inv;
    acc[i + 2] = b * sa + acc[i + 2] * inv;
    acc[i + 3] = sa + acc[i + 3] * inv;
}

fn to_u8(v: f32) -> u8 {
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

/// 把「预乘 alpha 的 BGRA8」转成图集契约的「直通 alpha RGBA8」。
///
/// DirectWrite 的 `GetGlyphImageData` 对 PNG / JPEG 层返回的是预乘 BGRA——
/// 这是 WIC 的解码格式，与字体里存的是 PNG 无关。图集契约要求直通 alpha
/// （ADR 0017 第 6 条），因此在这里一次转到位：换通道 + 除以 alpha。
///
/// 输入长度不足或与 `width * height` 不符时返回空向量，由调用方按"解不出层"
/// 处理，而不是画出半张图。
#[must_use]
pub fn bgra_premul_to_rgba(bgra: &[u8], width: u32, height: u32) -> Vec<u8> {
    let pixels = (width as usize) * (height as usize);
    if bgra.len() != pixels * 4 {
        return Vec::new();
    }
    let mut out = vec![0u8; pixels * 4];
    for (src, dst) in bgra.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let a = src[3];
        if a == 0 {
            continue;
        }
        // 预乘 → 直通：分量除以 alpha。a == 255 时是恒等变换，跳过除法。
        if a == 255 {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = 255;
            continue;
        }
        let scale = 255.0 / f32::from(a);
        for (channel, value) in [src[2], src[1], src[0]].into_iter().enumerate() {
            dst[channel] = (f32::from(value) * scale).round().clamp(0.0, 255.0) as u8;
        }
        dst[3] = a;
    }
    out
}

/// 彩色簇的缓存标识：簇内字形 id 的哈希。
///
/// 单字形簇返回 `0`——它的字形 id 已经唯一确定位图，不需要额外的簇维度。
/// 多字形簇的返回值保证非零，好让 `0` 在缓存键里始终只表示“单字形路径”。
#[must_use]
pub fn cluster_hash(glyph_ids: &[u16]) -> u64 {
    if glyph_ids.len() <= 1 {
        return 0;
    }
    let mut hasher = rustc_hash::FxHasher::default();
    glyph_ids.hash(&mut hasher);
    match hasher.finish() {
        0 => 1,
        v => v,
    }
}

/// 把一段字形按簇切成连续区间。
///
/// 同一簇的字形在塑形结果里必然相邻，因此只比对相邻项的 `cluster` 即可；
/// 不做全局分组，避免把不同位置的同簇字形误并到一起。
#[must_use]
pub fn cluster_ranges(glyphs: &[PositionedGlyph]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;
    for (i, g) in glyphs.iter().enumerate() {
        if i == 0 {
            continue;
        }
        if g.cluster != glyphs[start].cluster {
            out.push(start..i);
            start = i;
        }
    }
    if !glyphs.is_empty() {
        out.push(start..glyphs.len());
    }
    out
}
