//! 分页控件：页码按钮、省略号折叠、每页条数选择器与跳转。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_PAGE_SLOTS: usize = 7;
/// 圆点分页最多同时显示的圆点数量。
const MAX_PIPS: usize = 7;

/// 分页外观。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaginationStyle {
    /// 数字页码按钮 + 上一页 / 下一页。
    #[default]
    Numbers,
    /// `PipsPager` 形态的纯圆点指示器：选中圆点放大，其余为小圆点。
    Dots,
}

/// [`Pagination`] 分页栏的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PaginationDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 总页数（>= 1）。
    pub total_pages: usize,
    /// 当前激活页码（从 1 开始）。
    pub current_page: usize,
    /// 每页条数（作为元数据显示）。
    pub page_size: usize,
    /// 无障碍标签。
    pub label: String,
    /// 外观形态。
    pub style: PaginationStyle,
    /// 基础 z-index。
    pub z: u16,
}

impl PaginationDesc {
    /// 创建分页描述符。
    #[must_use]
    pub fn new(rect: Rect, total_pages: usize, current_page: usize) -> Self {
        let total = total_pages.max(1);
        Self {
            rect,
            total_pages: total,
            current_page: current_page.clamp(1, total),
            page_size: 20,
            label: "Pagination".to_owned(),
            style: PaginationStyle::Numbers,
            z: 600,
        }
    }

    /// 设置每页条数。
    #[must_use]
    pub fn with_page_size(mut self, page_size: usize) -> Self {
        self.page_size = page_size.max(1);
        self
    }

    /// 设置外观形态。
    #[must_use]
    pub fn with_style(mut self, style: PaginationStyle) -> Self {
        self.style = style;
        self
    }

    /// 切换为纯圆点形态（WinUI `PipsPager`）。
    #[must_use]
    pub fn dots(self) -> Self {
        self.with_style(PaginationStyle::Dots)
    }

    /// 计算 7 个可见页码槽位（`Some(page)` 为页码，`None` 为省略号）。
    #[must_use]
    pub fn visible_slots(&self) -> [Option<usize>; MAX_PAGE_SLOTS] {
        let total = self.total_pages.max(1);
        let cur = self.current_page.clamp(1, total);
        let mut out = [None; MAX_PAGE_SLOTS];
        if total <= MAX_PAGE_SLOTS {
            for (i, slot) in out.iter_mut().take(total).enumerate() {
                *slot = Some(i + 1);
            }
            return out;
        }
        out[0] = Some(1);
        out[MAX_PAGE_SLOTS - 1] = Some(total);
        if cur <= 4 {
            out[1] = Some(2);
            out[2] = Some(3);
            out[3] = Some(4);
            out[4] = Some(5);
            out[5] = None;
        } else if cur >= total - 3 {
            out[1] = None;
            out[2] = Some(total - 4);
            out[3] = Some(total - 3);
            out[4] = Some(total - 2);
            out[5] = Some(total - 1);
        } else {
            out[1] = None;
            out[2] = Some(cur - 1);
            out[3] = Some(cur);
            out[4] = Some(cur + 1);
            out[5] = None;
        }
        out
    }
}

/// 上一页按钮的确定性子节点 ID。
#[must_use]
pub const fn pagination_prev_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x200
}

/// 下一页按钮的确定性子节点 ID。
#[must_use]
pub const fn pagination_next_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x201
}

/// 指定页码按钮（从 1 开始）的确定性子节点 ID。
#[must_use]
pub const fn pagination_page_node(node: u64, page: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x300 + page as u64)
}

/// 分页导航的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaginationState {
    /// 当前页码（从 1 开始）。
    pub page: usize,
}

impl PaginationState {
    /// 在指定 `page` 处创建状态。
    #[must_use]
    pub fn new(page: usize) -> Self {
        Self { page: page.max(1) }
    }

    /// 跳转到 `page`，夹紧至 `1..=total_pages`。
    pub fn go_to(&mut self, page: usize, total_pages: usize) -> bool {
        let next = page.clamp(1, total_pages.max(1));
        if next != self.page {
            self.page = next;
            true
        } else {
            false
        }
    }

    /// 前进一页。
    pub fn next(&mut self, total_pages: usize) -> bool {
        self.go_to(self.page.saturating_add(1), total_pages)
    }

    /// 后退一页。
    pub fn prev(&mut self, total_pages: usize) -> bool {
        self.go_to(self.page.saturating_sub(1), total_pages)
    }
}

/// 输出分页控件：`Numbers` 绘制数字页码栏，`Dots` 绘制纯圆点指示器。
#[must_use]
pub fn emit_pagination(
    node: u64,
    desc: &PaginationDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    match desc.style {
        PaginationStyle::Numbers => emit_number_pager(node, desc, state, label_glyphs),
        PaginationStyle::Dots => emit_dot_pager(node, desc, state),
    }
}

/// 输出结构稳定的数字分页栏
/// （1 个容器 + 9 个槽位按钮 + 9 条选中下划线 + 1 个焦点环 = 20 个图元）。
fn emit_number_pager(
    node: u64,
    desc: &PaginationDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(20 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    let border = if state.disabled {
        Token::DISABLED
    } else if hovered || state.focused {
        Token::FOCUS_RING
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(
        desc.rect,
        // WinUI `ControlCornerRadius = 4`。
        4.0,
        Token::SURFACE_SUNKEN,
        border,
        1.0,
        0.0,
        desc.z,
    ));

    let total_slots = MAX_PAGE_SLOTS + 2;
    // WinUI `PagerControl` 的页码面板 `StackLayout.Spacing = 5`。
    let gap = 5.0;
    let pad = 4.0;
    let slot_w = ((desc.rect.width() - pad * 2.0 - gap * (total_slots - 1) as f32)
        / total_slots as f32)
        .max(18.0);
    let slot_h = (desc.rect.height() - pad * 2.0).max(18.0);
    let slots = desc.visible_slots();

    for idx in 0..total_slots {
        let x = desc.rect.origin.x + pad + idx as f32 * (slot_w + gap);
        let r = Rect::new(x, desc.rect.origin.y + pad, slot_w, slot_h);
        let (active, visible) = if idx == 0 || idx == total_slots - 1 {
            (false, true)
        } else {
            let page_opt = slots[idx - 1];
            let is_cur = page_opt == Some(desc.current_page);
            let is_vis = idx - 1 < desc.total_pages || desc.total_pages > MAX_PAGE_SLOTS;
            (is_cur, is_vis)
        };
        // `PagerControl` 的页码按钮是透明底，选中态只靠底部的 2px 强调色下划线表达。
        let fill = if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE
        };
        let mut inst = outlined(
            r,
            // WinUI `ControlCornerRadius = 4`。
            4.0,
            fill,
            Token::CARD_BORDER,
            if visible { 1.0 } else { 0.0 },
            0.0,
            desc.z + 1,
        );
        if !visible {
            inst.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(inst.depth_u16(), 0.0);
        }
        primitives.push(inst);

        // `NumberPanelCurrentPageIndicator`：高 2、圆角 1 的强调色下划线。
        let underline = Rect::new(r.origin.x, r.max().y - 2.0, r.width(), 2.0);
        let mut indicator = surface(underline, 1.0, Token::PRIMARY, desc.z + 2);
        if !(active && visible) {
            indicator.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(indicator.depth_u16(), 0.0);
        }
        primitives.push(indicator);
    }

    primitives.push(focus_ring(
        desc.rect,
        4.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Group, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for idx in 0..total_slots {
            let x = desc.rect.origin.x + pad + idx as f32 * (slot_w + gap);
            let r = Rect::new(x, desc.rect.origin.y + pad, slot_w, slot_h);
            if idx == 0 {
                let key = pagination_prev_node(node);
                out.with_sub_control(
                    key,
                    HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::Button, "Previous Page", r).clickable(),
                    true,
                );
            } else if idx == total_slots - 1 {
                let key = pagination_next_node(node);
                out.with_sub_control(
                    key,
                    HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::Button, "Next Page", r).clickable(),
                    true,
                );
            } else if let Some(page) = slots[idx - 1] {
                let is_vis = idx - 1 < desc.total_pages || desc.total_pages > MAX_PAGE_SLOTS;
                if is_vis {
                    let key = pagination_page_node(node, page);
                    out.with_sub_control(
                        key,
                        HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                            .with_cursor(CursorKind::Pointer),
                        A11yNode::new(Role::Button, format!("Page {page}"), r).clickable(),
                        true,
                    );
                }
            }
        }
    }

    out
}

/// 输出纯圆点形态的分页指示器（WinUI `PipsPager`）。
///
/// 可见圆点最多 [`MAX_PIPS`] 个并围绕当前页居中；选中圆点直径 10px，
/// 其余为 6px。图元数量只取决于可见圆点数，与交互状态无关。
fn emit_dot_pager(node: u64, desc: &PaginationDesc, state: ControlState) -> WidgetOutput {
    let total_pages = desc.total_pages.max(1);
    let visible = total_pages.min(MAX_PIPS);
    let current = desc.current_page.clamp(1, total_pages);
    let start = if total_pages <= MAX_PIPS {
        0
    } else {
        (current - 1)
            .saturating_sub(visible / 2)
            .min(total_pages - visible)
    };

    let border = if state.disabled {
        Token::DISABLED
    } else if state.phase == PointerPhase::Hovered || state.focused {
        Token::FOCUS_RING
    } else {
        Token::CARD_BORDER
    };

    let mut primitives = Vec::with_capacity(visible + 2);
    primitives.push(outlined(
        desc.rect,
        // WinUI `ControlCornerRadius = 4`。
        4.0,
        Token::NONE,
        border,
        1.0,
        0.0,
        desc.z,
    ));

    let step = desc.rect.width() / visible as f32;
    let cy = desc.rect.center().y;
    for index in 0..visible {
        let page = start + index + 1;
        let is_current = page == current;
        let size = if is_current { 10.0 } else { 6.0 };
        let cx = desc.rect.origin.x + step * (index as f32 + 0.5);
        let dot = Rect::new(cx - size * 0.5, cy - size * 0.5, size, size);
        let fill = if state.disabled {
            Token::CONTROL_DISABLED_THUMB
        } else if is_current {
            Token::PRIMARY
        } else {
            Token::CONTROL_TRACK
        };
        primitives.push(surface(dot, size * 0.5, fill, desc.z + 1));
    }

    primitives.push(focus_ring(
        desc.rect,
        4.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused && !state.disabled,
    ));

    WidgetOutput {
        primitives,
        hit: (!state.disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Group, desc.label.clone(), desc.rect).disabled(state.disabled),
        ),
        focusable: !state.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_slots_fold_middle_with_ellipsis() {
        let desc = PaginationDesc::new(Rect::new(0.0, 0.0, 320.0, 36.0), 20, 10);
        let slots = desc.visible_slots();
        assert_eq!(slots[0], Some(1));
        assert_eq!(slots[1], None);
        assert_eq!(slots[3], Some(10));
        assert_eq!(slots[5], None);
        assert_eq!(slots[6], Some(20));
    }

    #[test]
    fn pagination_state_navigates_within_bounds() {
        let mut st = PaginationState::new(1);
        assert!(!st.prev(5));
        assert!(st.next(5));
        assert_eq!(st.page, 2);
        assert!(st.go_to(99, 5));
        assert_eq!(st.page, 5);
    }

    #[test]
    fn the_dots_style_draws_one_pip_per_visible_page() {
        let desc = PaginationDesc::new(Rect::new(0.0, 0.0, 140.0, 24.0), 5, 3).dots();
        assert_eq!(desc.style, PaginationStyle::Dots);
        let output = emit_pagination(1, &desc, ControlState::default(), &[]);
        // 容器 + 5 个圆点 + 焦点环。
        assert_eq!(output.primitives.len(), 7);
    }

    #[test]
    fn the_selected_pip_is_larger_and_primary() {
        let desc = PaginationDesc::new(Rect::new(0.0, 0.0, 140.0, 24.0), 5, 3).dots();
        let output = emit_pagination(1, &desc, ControlState::default(), &[]);
        let selected = &output.primitives[3];
        let neighbour = &output.primitives[2];
        assert!(selected.size[0] > neighbour.size[0]);
        assert_eq!(selected.fill_token_or_uv as u16, Token::PRIMARY.raw());
        assert_eq!(
            neighbour.fill_token_or_uv as u16,
            Token::CONTROL_TRACK.raw()
        );
    }

    #[test]
    fn the_dots_style_keeps_a_constant_primitive_count() {
        let desc = PaginationDesc::new(Rect::new(0.0, 0.0, 140.0, 24.0), 20, 10).dots();
        let mut counts = Vec::new();
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let state = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        counts.push(emit_pagination(1, &desc, state, &[]).primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}
