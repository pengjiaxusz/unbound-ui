//! 纵横比容器（AspectRatioBox）与自适应多列瀑布流布局（MasonryGrid）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::{focus_ring, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_MASONRY_ITEMS: usize = 6;

/// [`AspectRatioBox`] 容器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct AspectRatioBoxDesc {
    /// 可用外层边界。
    pub bounds: Rect,
    /// 目标 `width / height` 宽高比。
    pub ratio: f32,
}

impl AspectRatioBoxDesc {
    /// 在 `bounds` 内计算符合 `ratio` 的最大居中矩形。
    #[must_use]
    pub fn fit(&self) -> Rect {
        let r = self.ratio.max(0.05);
        let bw = self.bounds.width().max(1.0);
        let bh = self.bounds.height().max(1.0);
        let (w, h) = if bw / bh > r {
            (bh * r, bh)
        } else {
            (bw, bw / r)
        };
        Rect::new(
            self.bounds.origin.x + (bw - w) * 0.5,
            self.bounds.origin.y + (bh - h) * 0.5,
            w,
            h,
        )
    }
}

/// 单个瀑布流图块规格。
#[derive(Clone, Debug, PartialEq)]
pub struct MasonryItem {
    /// 图块高度（或纵横比高度）。
    pub height: f32,
}

/// [`MasonryGrid`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct MasonryDesc {
    /// 容器矩形。
    pub rect: Rect,
    /// 列数（>= 1）。
    pub columns: usize,
    /// 图块间距（逻辑像素）。
    pub gap: f32,
    /// 待排列的图块列表。
    pub items: Vec<MasonryItem>,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl MasonryDesc {
    /// 创建瀑布流网格描述符。
    #[must_use]
    pub fn new(rect: Rect, columns: usize, items: Vec<MasonryItem>) -> Self {
        Self {
            rect,
            columns: columns.max(1),
            gap: 6.0,
            items,
            label: "Masonry Grid".to_owned(),
            z: 600,
        }
    }
}

/// 纯瀑布流最短列放置算法。
#[must_use]
pub fn layout_masonry(desc: &MasonryDesc) -> Vec<Rect> {
    let cols = desc.columns.max(1);
    let col_w = ((desc.rect.width() - desc.gap * (cols as f32 + 1.0)) / cols as f32).max(8.0);
    let mut col_y = vec![desc.rect.origin.y + desc.gap; cols];
    let mut out = Vec::with_capacity(desc.items.len());
    for item in &desc.items {
        let (best_col, &best_y) = col_y
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .expect("cols >= 1");
        let x = desc.rect.origin.x + desc.gap + best_col as f32 * (col_w + desc.gap);
        let h = item.height.max(12.0);
        out.push(Rect::new(x, best_y, col_w, h));
        col_y[best_col] = best_y + h + desc.gap;
    }
    out
}

/// 发射固定形状的瀑布流网格（1 背景 + 6 图块槽位 + 1 焦点环 = 8 个图元）。
#[must_use]
pub fn emit_masonry(node: u64, desc: &MasonryDesc, state: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(8);
    let payloads = Vec::new();

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        Token::SURFACE_SUNKEN,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let rects = layout_masonry(desc);
    for idx in 0..MAX_MASONRY_ITEMS {
        let r_opt = rects.get(idx).copied();
        let r =
            r_opt.unwrap_or_else(|| Rect::new(desc.rect.origin.x, desc.rect.origin.y, 12.0, 12.0));
        let mut tile_inst = outlined(
            r,
            4.0,
            if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE
            },
            Token::CARD_BORDER,
            1.0,
            0.0,
            desc.z + 1,
        );
        if r_opt.is_none() {
            tile_inst.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(tile_inst.depth_u16(), 0.0);
        }
        primitives.push(tile_inst);
    }

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Default)
    });
    let a11y = A11yNode::new(Role::Grid, desc.label.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_ratio_box_centers_and_fits_bounds() {
        let fit = AspectRatioBoxDesc {
            bounds: Rect::new(0.0, 0.0, 200.0, 100.0),
            ratio: 1.0,
        }
        .fit();
        assert_eq!(fit, Rect::new(50.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn masonry_balances_shortest_column() {
        let desc = MasonryDesc::new(
            Rect::new(0.0, 0.0, 220.0, 200.0),
            2,
            vec![
                MasonryItem { height: 60.0 },
                MasonryItem { height: 20.0 },
                MasonryItem { height: 30.0 },
            ],
        );
        let rects = layout_masonry(&desc);
        assert_eq!(rects.len(), 3);
        assert!(rects[2].origin.x > rects[0].origin.x);
        assert_eq!(rects[2].origin.y, 32.0);
    }
}
