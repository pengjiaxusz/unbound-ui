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
    let visible = || {
        layers
            .iter()
            .filter(|l| l.color.a > 0.0 && l.width > 0 && l.height > 0)
    };
    if visible().next().is_none() {
        return CompositeOutcome::NotColor;
    }

    let mut min_left = i32::MAX;
    let mut min_top = i32::MAX;
    let mut max_right = i32::MIN;
    let mut max_bottom = i32::MIN;
    for layer in visible() {
        min_left = min_left.min(layer.left);
        min_top = min_top.min(layer.top);
        max_right = max_right.max(layer.left.saturating_add(layer.width as i32));
        max_bottom = max_bottom.max(layer.top.saturating_add(layer.height as i32));
    }

    let width = (max_right - min_left).max(0) as u32;
    let height = (max_bottom - min_top).max(0) as u32;
    if width == 0 || height == 0 {
        return CompositeOutcome::NotColor;
    }

    // 预乘累加缓冲：每像素 4 个 f32（r·a, g·a, b·a, a）。
    let mut acc = vec![0.0f32; (width as usize) * (height as usize) * 4];
    let stride = width as usize;

    for layer in visible() {
        debug_assert_eq!(
            layer.coverage.len(),
            (layer.width as usize) * (layer.height as usize),
            "coverage bitmap size does not match the declared rect"
        );
        if layer.coverage.len() != (layer.width as usize) * (layer.height as usize) {
            continue;
        }
        let dx = (layer.left - min_left) as usize;
        let dy = (layer.top - min_top) as usize;
        for y in 0..layer.height as usize {
            let dst_row = (dy + y) * stride + dx;
            let src_row = y * layer.width as usize;
            for x in 0..layer.width as usize {
                let c = layer.coverage[src_row + x];
                if c == 0 {
                    continue;
                }
                let sa = layer.color.a * (f32::from(c) / 255.0);
                if sa <= 0.0 {
                    continue;
                }
                let inv = 1.0 - sa;
                let i = (dst_row + x) * 4;
                acc[i] = layer.color.r * sa + acc[i] * inv;
                acc[i + 1] = layer.color.g * sa + acc[i + 1] * inv;
                acc[i + 2] = layer.color.b * sa + acc[i + 2] * inv;
                acc[i + 3] = sa + acc[i + 3] * inv;
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

fn to_u8(v: f32) -> u8 {
    (v * 255.0).round().clamp(0.0, 255.0) as u8
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
