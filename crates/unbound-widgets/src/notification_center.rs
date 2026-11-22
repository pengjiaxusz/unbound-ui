//! NotificationCenter 抽屉，聚合 Toast 历史、严重级别过滤与已读状态。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuClipNode, GpuPrimitiveInstance};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::alert::Severity;
use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{AlignX, AlignY, ControlState, PointerPhase, SceneCollector, TextCtx, WidgetOutput};

const MAX_ITEMS: usize = 4;

/// 面板圆角半径。对齐 WinUI `OverlayCornerRadius = 8`（浮层面板的 `CornerRadius`）。
const PANEL_RADIUS: f32 = 8.0;
/// 面板阴影模糊半径，与菜单/Toast 的屏幕级浮层阴影同档。
const PANEL_SHADOW: f32 = 18.0;
/// 面板四周边距及列表项之间的垂直间距。
const PANEL_PADDING: f32 = 6.0;
/// 通知项圆角半径。对齐 WinUI `ControlCornerRadius = 4`（列表项）。
const ROW_RADIUS: f32 = 4.0;
/// 通知项标题文本的左侧内边距（对齐面板内边距后起点为 `rect.origin.x + 14.0`）。
const ITEM_TEXT_PAD_LEFT: f32 = 8.0;
/// 未读圆点直径。对齐 WinUI `InfoBadge` 点状规格 `InfoBadgeMinWidth/Height = 4`。
const UNREAD_DOT_SIZE: f32 = 4.0;
/// 未读圆点右缘距离通知卡片右边界的留白。
const UNREAD_DOT_RIGHT_MARGIN: f32 = 10.0;
/// 通知卡片右侧为未读圆点及其右边距预留的总宽度（`10 + 4 = 14`）。
const UNREAD_DOT_RESERVED_RIGHT: f32 = UNREAD_DOT_RIGHT_MARGIN + UNREAD_DOT_SIZE;
/// 标题文本右缘与未读圆点左缘之间的安全间距，防止长文本穿透未读指示点。
const ITEM_TEXT_DOT_GAP: f32 = 8.0;

/// 单条已归档的通知项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationItem {
    /// 稳定 ID。
    pub id: u64,
    /// 标题文本。
    pub title: String,
    /// 严重级别。
    pub severity: Severity,
    /// 是否已读。
    pub read: bool,
}

/// [`NotificationCenterDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationCenterState {
    /// 已归档的通知列表。
    pub items: Vec<NotificationItem>,
    /// 可选严重级别过滤器。
    pub filter: Option<Severity>,
}

impl NotificationCenterState {
    /// 将所有通知标记为已读。
    pub fn mark_all_read(&mut self) {
        for item in &mut self.items {
            item.read = true;
        }
    }

    /// 清空所有通知。
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// 统计未读项数量。
    #[must_use]
    pub fn unread_count(&self) -> usize {
        self.items.iter().filter(|i| !i.read).count()
    }
}

/// [`NotificationCenter`] 的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationCenterDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 可见项列表。
    pub items: Vec<NotificationItem>,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl NotificationCenterDesc {
    /// 创建通知中心描述符。
    #[must_use]
    pub fn new(rect: Rect, items: Vec<NotificationItem>) -> Self {
        Self {
            rect,
            items,
            label: "Notification Center".to_owned(),
            z: 600,
        }
    }

    /// 计算第 `index` 条通知卡片的布局矩形（严格约束在容器 `self.rect` 内）。
    #[must_use]
    pub fn item_rect(&self, index: usize) -> Option<Rect> {
        if index >= MAX_ITEMS {
            return None;
        }
        let w = self.rect.width();
        let h = self.rect.height();
        if !w.is_finite() || w <= 0.0 || !h.is_finite() || h <= 0.0 {
            return Some(Rect::new(self.rect.origin.x, self.rect.origin.y, 0.0, 0.0));
        }
        let row_w = (w - PANEL_PADDING * 2.0).max(0.0);
        let row_h = ((h - PANEL_PADDING * (MAX_ITEMS as f32 + 1.0)) / MAX_ITEMS as f32).max(0.0);
        let x = (self.rect.origin.x + PANEL_PADDING).min(self.rect.max().x);
        let y = (self.rect.origin.y + PANEL_PADDING + index as f32 * (row_h + PANEL_PADDING))
            .min(self.rect.max().y);
        Some(Rect::new(
            x,
            y,
            row_w.min((self.rect.max().x - x).max(0.0)),
            row_h.min((self.rect.max().y - y).max(0.0)),
        ))
    }

    /// 计算第 `index` 条通知右侧未读指示点的布局矩形（严格约束在卡片内）。
    #[must_use]
    pub fn item_dot_rect(&self, index: usize) -> Option<Rect> {
        let r = self.item_rect(index)?;
        let dot_size = UNREAD_DOT_SIZE.min(r.width()).min(r.height()).max(0.0);
        let dot_x = (r.max().x - UNREAD_DOT_RESERVED_RIGHT)
            .max(r.origin.x)
            .min(r.max().x - dot_size);
        let dot_y = r.origin.y + (r.height() - dot_size) * 0.5;
        Some(Rect::new(dot_x, dot_y, dot_size, dot_size))
    }

    /// 计算单条通知标题的最大可用宽度。
    ///
    /// 依次扣除面板左右内边距、卡片左侧文本内边距、右侧未读圆点预留区及文本与圆点间距，
    /// 保证标题文本不会穿透未读指示点或溢出卡片右边界。
    #[must_use]
    pub fn item_text_max_width(&self) -> f32 {
        let w = self.rect.width();
        if !w.is_finite() || w <= 0.0 {
            return 0.0;
        }
        let row_w = (w - PANEL_PADDING * 2.0).max(0.0);
        (row_w - ITEM_TEXT_PAD_LEFT - UNREAD_DOT_RESERVED_RIGHT - ITEM_TEXT_DOT_GAP).max(0.0)
    }

    /// 计算第 `index` 条通知标题的排版矩形（严格位于左侧内边距与未读圆点预留区之间）。
    #[must_use]
    pub fn item_text_rect(&self, index: usize) -> Option<Rect> {
        let r = self.item_rect(index)?;
        let max_w = self.item_text_max_width();
        let x = (r.origin.x + ITEM_TEXT_PAD_LEFT).min(r.max().x);
        Some(Rect::new(x, r.origin.y, max_w, r.height()))
    }

    /// 生成对齐通知中心面板矩形的 GPU 容器裁剪节点（供 [`SceneCollector::push_clip`] 使用）。
    #[must_use]
    pub fn clip_node(&self) -> GpuClipNode {
        GpuClipNode::from_aabb(self.rect.to_min_max_array())
    }

    /// 使用 [`TextCtx::ellipsize_line`] 对可见通知标题逐行省略截断并排版为字形列表。
    #[must_use]
    pub fn shape_item_glyphs(
        &self,
        text_ctx: &mut TextCtx<'_>,
        font_size: f32,
        weight: u16,
    ) -> Vec<GlyphQuad> {
        let max_w = self.item_text_max_width();
        if !font_size.is_finite() || font_size <= 0.0 || max_w <= 0.0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (i, item) in self.items.iter().take(MAX_ITEMS).enumerate() {
            let Some(text_rect) = self.item_text_rect(i) else {
                continue;
            };
            if !text_rect.height().is_finite() || text_rect.height() <= 0.0 {
                continue;
            }
            let fitted = text_ctx.ellipsize_line(&item.title, font_size, weight, max_w);
            if fitted.is_empty() {
                continue;
            }
            out.extend(text_ctx.shape_line(
                &fitted,
                text_rect,
                font_size,
                weight,
                AlignX::Left,
                AlignY::Middle,
            ));
        }
        out
    }

    /// 在容器 `self.rect` 的 `push_clip` / `pop_clip` 裁剪作用域内向 [`SceneCollector`] 提交通知中心。
    ///
    /// 自动对超长标题执行 [`TextCtx::ellipsize_line`] 省略截断并施加容器级硬裁剪。
    pub fn collect_scoped(
        &self,
        collector: &mut SceneCollector,
        node: u64,
        state: ControlState,
        text_ctx: &mut TextCtx<'_>,
        font_size: f32,
        weight: u16,
    ) {
        let label_glyphs = self.shape_item_glyphs(text_ctx, font_size, weight);
        collector.push_clip(self.clip_node(), self.rect);
        collector.push(
            node,
            emit_notification_center(node, self, state, &label_glyphs),
        );
        collector.pop_clip();
    }
}

/// 通知卡片项的确定性子节点 ID（`0..4`）。
#[must_use]
pub const fn notification_item_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0xC00 + index as u64)
}

/// 输出结构稳定的通知中心（1 个背景 + 4 张项卡片 + 4 个未读圆点 + 1 个焦点环 = 10 个图元）。
///
/// 发射层会对传入的 `label_glyphs` 施加容器与行标题右边界约束：任何超出容器矩形或侵入右侧
/// 未读圆点预留区的字形都会被过滤，确保即使调用方未提前省略截断，也不会穿透圆点或溢出卡片。
#[must_use]
pub fn emit_notification_center(
    node: u64,
    desc: &NotificationCenterDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(10 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    // 面板：`FlyoutPresenter` 的 8px 圆角、1px 浮层描边与屏幕级阴影；
    // 悬停与焦点交给各自的列表项高亮与焦点环表达。
    primitives.push(outlined(
        desc.rect,
        PANEL_RADIUS,
        Token::SURFACE_RAISED,
        Token::CONTROL_STROKE,
        1.0,
        PANEL_SHADOW,
        desc.z,
    ));

    for i in 0..MAX_ITEMS {
        let item = desc.items.get(i);
        let r = desc
            .item_rect(i)
            .unwrap_or_else(|| Rect::new(desc.rect.origin.x, desc.rect.origin.y, 0.0, 0.0));
        // 列表项：4px 圆角、1px 描边；悬停改用 `SubtleFillColorSecondary` 的薄纱。
        let mut card = outlined(
            r,
            ROW_RADIUS,
            if hovered {
                Token::HOVER
            } else {
                Token::SURFACE_SUNKEN
            },
            Token::CONTROL_STROKE,
            1.0,
            0.0,
            desc.z + 1,
        );
        if item.is_none() {
            card.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(card.depth_u16(), 0.0);
        }
        primitives.push(card);

        let dot_r = desc
            .item_dot_rect(i)
            .unwrap_or_else(|| Rect::new(r.origin.x, r.origin.y, 0.0, 0.0));
        let unread = item.is_some_and(|it| !it.read);
        let dot_tok = match item.map(|it| it.severity).unwrap_or(Severity::Info) {
            Severity::Error => Token::ERROR,
            Severity::Warning => Token::WARNING,
            Severity::Success => Token::SUCCESS,
            Severity::Info => Token::PRIMARY,
        };
        let mut dot = surface(dot_r, dot_r.height() * 0.5, dot_tok, desc.z + 2);
        if !unread {
            dot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(dot.depth_u16(), 0.0);
        }
        primitives.push(dot);
    }

    primitives.push(focus_ring(
        desc.rect,
        PANEL_RADIUS,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    let text_max_w = desc.item_text_max_width();
    if !label_glyphs.is_empty()
        && text_max_w > 0.0
        && desc.rect.height().is_finite()
        && desc.rect.height() > 0.0
    {
        let min_x = desc.rect.origin.x;
        let max_x = desc.rect.origin.x + PANEL_PADDING + ITEM_TEXT_PAD_LEFT + text_max_w;
        let min_y = desc.rect.origin.y;
        let max_y = desc.rect.max().y;
        let bounded_glyphs: Vec<GlyphQuad> = label_glyphs
            .iter()
            .copied()
            .filter(|q| {
                let r = q.rect;
                r.origin.x.is_finite()
                    && r.origin.y.is_finite()
                    && r.width().is_finite()
                    && r.height().is_finite()
                    && r.origin.x >= min_x
                    && r.max().x <= max_x
                    && r.origin.y >= min_y
                    && r.max().y <= max_y
            })
            .collect();
        if !bounded_glyphs.is_empty() {
            glyphs(
                &bounded_glyphs,
                Token::TEXT_PRIMARY,
                desc.z + 4,
                &mut primitives,
                &mut payloads,
            );
        }
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Region, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for (i, item) in desc.items.iter().take(MAX_ITEMS).enumerate() {
            let Some(r) = desc.item_rect(i) else {
                continue;
            };
            let key = notification_item_node(node, i);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::ListItem, item.title.clone(), r).clickable(),
                true,
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_text_ctx<'a>(
        engine: &'a mut unbound_text::TextEngine,
        cache: &'a mut unbound_text::GlyphCache,
    ) -> TextCtx<'a> {
        TextCtx {
            engine,
            cache,
            atlas_clock: 1,
            raster_scale: 1.0,
        }
    }

    fn make_glyph_quad(rect: Rect) -> GlyphQuad {
        GlyphQuad {
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        }
    }

    #[test]
    fn notification_center_item_text_max_width_and_geometry_bounds() {
        let rect = Rect::new(10.0, 20.0, 260.0, 120.0);
        let desc = NotificationCenterDesc::new(rect, Vec::new());

        // 260 - 12 (panel padding) - 8 (left text pad) - 14 (dot reserved) - 8 (gap) = 218
        assert_eq!(desc.item_text_max_width(), 218.0);

        for i in 0..MAX_ITEMS {
            let card = desc.item_rect(i).expect("row within MAX_ITEMS");
            let dot = desc.item_dot_rect(i).expect("dot within MAX_ITEMS");
            let text = desc.item_text_rect(i).expect("text rect within MAX_ITEMS");

            // 卡片、未读圆点与标题区域均严格位于容器内，且标题右缘与圆点左缘保持安全间距。
            assert!(card.origin.x >= rect.origin.x);
            assert!(card.max().x <= rect.max().x);
            assert!(card.origin.y >= rect.origin.y);
            assert!(card.max().y <= rect.max().y);
            assert!(dot.origin.x >= card.origin.x);
            assert!(dot.max().x <= card.max().x);
            assert!(text.max().x + ITEM_TEXT_DOT_GAP <= dot.origin.x + 1e-4);
            assert!(text.max().x < dot.origin.x);
        }
        assert!(desc.item_rect(MAX_ITEMS).is_none());

        // 极窄或非有限容器宽度下安全回退到 0.0。
        let narrow = NotificationCenterDesc::new(Rect::new(0.0, 0.0, 32.0, 100.0), Vec::new());
        assert_eq!(narrow.item_text_max_width(), 0.0);
        let zero = NotificationCenterDesc::new(Rect::new(0.0, 0.0, 0.0, 100.0), Vec::new());
        assert_eq!(zero.item_text_max_width(), 0.0);
        let nan_rect =
            NotificationCenterDesc::new(Rect::new(0.0, 0.0, f32::NAN, 100.0), Vec::new());
        assert_eq!(nan_rect.item_text_max_width(), 0.0);

        // 容器裁剪节点严格对齐面板矩形。
        assert_eq!(
            desc.clip_node(),
            GpuClipNode::from_aabb(rect.to_min_max_array())
        );
    }

    #[test]
    fn notification_center_ellipsize_long_ascii_and_cjk_titles_within_dot_bound() {
        let rect = Rect::new(0.0, 0.0, 220.0, 116.0);
        let long_ascii =
            "ShaderPipelineCompilationFinishedWithExtremelyVerboseDiagnosticPayloadThatWouldOverflow";
        let long_cjk = "渲染管线着色器编译完成并且携带超长诊断日志详情绝对不能穿透未读圆点或溢出通知卡片右边界"; // lang-ok: CJK overflow regression sample

        let desc = NotificationCenterDesc::new(
            rect,
            vec![
                NotificationItem {
                    id: 1,
                    title: long_ascii.to_owned(),
                    severity: Severity::Info,
                    read: false,
                },
                NotificationItem {
                    id: 2,
                    title: long_cjk.to_owned(),
                    severity: Severity::Warning,
                    read: false,
                },
            ],
        );

        let max_w = desc.item_text_max_width();
        assert_eq!(max_w, 178.0);

        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut text_ctx = make_text_ctx(&mut engine, &mut cache);

        for title in [long_ascii, long_cjk] {
            let full_w = text_ctx.measure_line(title, 12.0, 400).x;
            let fitted = text_ctx.ellipsize_line(title, 12.0, 400, max_w);
            let fitted_w = text_ctx.measure_line(&fitted, 12.0, 400).x;
            assert!(
                fitted_w <= max_w,
                "fitted width {fitted_w} exceeded {max_w}"
            );
            if full_w > max_w {
                assert!(fitted.ends_with('…'));
                assert!(fitted.chars().count() < title.chars().count());
            }
        }

        // 通过便捷排版与容器裁剪收集器提交，验证字形右缘不穿透未读圆点且容器裁剪节点已入栈。
        let dot0 = desc.item_dot_rect(0).expect("first row dot");
        let shaped = desc.shape_item_glyphs(&mut text_ctx, 12.0, 400);
        for q in &shaped {
            assert!(
                q.rect.max().x <= dot0.origin.x,
                "glyph right {} crossed unread dot left {}",
                q.rect.max().x,
                dot0.origin.x
            );
            assert!(q.rect.max().x <= rect.max().x);
        }

        let mut collector = SceneCollector::default();
        desc.collect_scoped(
            &mut collector,
            101,
            ControlState::default(),
            &mut text_ctx,
            12.0,
            400,
        );
        assert_eq!(collector.clips.len(), 1);
        assert_eq!(collector.clips[0], desc.clip_node());
        // 每个发射图元都绑定到容器裁剪节点。
        assert!(collector.primitives.iter().all(|p| p.clip_idx == 0));
    }

    #[test]
    fn notification_center_emit_filters_overflowing_glyphs_before_unread_dot() {
        let rect = Rect::new(0.0, 0.0, 200.0, 110.0);
        let desc = NotificationCenterDesc::new(
            rect,
            vec![NotificationItem {
                id: 1,
                title: "Overflow guard".to_owned(),
                severity: Severity::Error,
                read: false,
            }],
        );
        let dot = desc.item_dot_rect(0).expect("row 0 dot");
        let text_r = desc.item_text_rect(0).expect("row 0 text rect");

        let inside_glyph = make_glyph_quad(Rect::new(
            text_r.origin.x,
            text_r.origin.y + 4.0,
            24.0,
            12.0,
        ));
        // 侵入未读圆点区域的字形。
        let dot_crossing_glyph = make_glyph_quad(Rect::new(
            dot.origin.x - 2.0,
            text_r.origin.y + 4.0,
            10.0,
            12.0,
        ));
        // 溢出通知面板右边界的字形。
        let card_overflow_glyph = make_glyph_quad(Rect::new(
            rect.max().x - 4.0,
            text_r.origin.y + 4.0,
            20.0,
            12.0,
        ));

        let out = emit_notification_center(
            7,
            &desc,
            ControlState::default(),
            &[inside_glyph, dot_crossing_glyph, card_overflow_glyph],
        );

        // 10 个固定结构图元 + 仅 1 个合法范围内的字形图元；越界字形均被过滤。
        assert_eq!(out.primitives.len(), 11);
        assert_eq!(out.payloads.len(), 1);
    }
}
