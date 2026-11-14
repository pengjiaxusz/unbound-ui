//! 模态对话框（Modal）：遮罩层、卡片与焦点作用域。
//!
//! 模态框不仅是一张浮动卡片。它同时拥有三项要素：
//!
//! * 覆盖全视口的**遮罩层（scrim）**命中区域，吞掉所有指向底层 UI 的指针事件，
//! * 接收正常输入的**卡片**及其子控件，
//! * **焦点作用域**，由调用方安装到 [`unbound_input::FocusManager::set_trap`]，防止 `Tab` 键逃逸到背景。
//!
//! 将三者置于同一发射器中使其绝不会彼此脱节：没有遮罩层或没有焦点作用域的模态框在此无法构造。

use accesskit::Role;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{FocusCause, FocusManager, HitRegion, PORTAL_LAYER};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::overlay::{BackdropRequest, OverlayEntry};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// 模态框的命中测试层级：高于普通浮层。
pub const MODAL_LAYER: u8 = PORTAL_LAYER + 1;
const MODAL_SHADOW: f32 = 40.0;
/// 标准标题栏关闭按钮尺寸。
pub const MODAL_CLOSE_SIZE: f32 = 24.0;
/// 默认底部操作按钮宽度。
pub const MODAL_ACTION_WIDTH: f32 = 88.0;
/// 两个底部操作按钮之间的间距。
pub const MODAL_ACTION_GAP: f32 = 8.0;
/// 底部栏水平内边距。
pub const MODAL_FOOTER_PADDING: f32 = 16.0;

/// 模态对话框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ModalDesc {
    /// 对话框卡片矩形。
    pub rect: Rect,
    /// 无障碍标题。
    pub label: String,
    /// 圆角半径。
    pub radius: f32,
    /// 绘制顺序键。
    pub z: u16,
    /// 遮罩层背后的背景模糊半径；`0.0` 表示禁用。
    pub backdrop_blur: f32,
    /// 是否绘制标准标题栏关闭按钮。
    pub close_button: bool,
    /// 关闭按钮在其中居中的标题栏高度。
    pub title_bar_height: f32,
    /// 底部操作栏高度；`0.0` 表示隐藏底部栏。
    pub footer_height: f32,
}

impl ModalDesc {
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            radius: 14.0,
            z: 950,
            backdrop_blur: 0.0,
            close_button: false,
            title_bar_height: 44.0,
            footer_height: 0.0,
        }
    }

    /// 设置无障碍标题。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 启用标准标题栏关闭按钮。
    #[must_use]
    pub fn with_close_button(mut self) -> Self {
        self.close_button = true;
        self
    }

    /// 启用指定高度的底部操作栏。
    #[must_use]
    pub fn with_footer(mut self, height: f32) -> Self {
        self.footer_height = height;
        self
    }

    /// 标题栏矩形。
    #[must_use]
    pub fn title_bar_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.width(),
            self.title_bar_height,
        )
    }

    /// 关闭按钮矩形（若已启用）。
    #[must_use]
    pub fn close_button_rect(&self) -> Option<Rect> {
        self.close_button.then(|| {
            Rect::new(
                self.rect.max().x - 12.0 - MODAL_CLOSE_SIZE,
                self.rect.origin.y + (self.title_bar_height - MODAL_CLOSE_SIZE) * 0.5,
                MODAL_CLOSE_SIZE,
                MODAL_CLOSE_SIZE,
            )
        })
    }

    /// 底部栏矩形（若配置了底部栏高度）。
    #[must_use]
    pub fn footer_rect(&self) -> Option<Rect> {
        (self.footer_height > 0.0).then(|| {
            Rect::new(
                self.rect.origin.x,
                self.rect.max().y - self.footer_height,
                self.rect.width(),
                self.footer_height,
            )
        })
    }

    /// `count` 个等宽底部操作按钮的矩形列表。
    ///
    /// 操作按钮在底部栏中右对齐，按调用方发射顺序从左到右排列，因此主操作位于最后一个矩形。
    #[must_use]
    pub fn footer_actions(&self, count: usize) -> Vec<Rect> {
        let Some(footer) = self.footer_rect() else {
            return Vec::new();
        };
        if count == 0 {
            return Vec::new();
        }
        let total = count as f32 * MODAL_ACTION_WIDTH + (count - 1) as f32 * MODAL_ACTION_GAP;
        let first_x = footer.max().x - MODAL_FOOTER_PADDING - total;
        let y = footer.origin.y + (footer.height() - 32.0) * 0.5;
        (0..count)
            .map(|index| {
                Rect::new(
                    first_x + index as f32 * (MODAL_ACTION_WIDTH + MODAL_ACTION_GAP),
                    y,
                    MODAL_ACTION_WIDTH,
                    32.0,
                )
            })
            .collect()
    }
}

/// 一个或多个打开模态框的焦点栈。
///
/// 打开时压入原焦点节点；关闭时弹出并恢复。该栈支持嵌套，
/// 因此从另一个对话框打开“确定要继续吗？”对话框后关闭时，键盘焦点会回到第一个对话框。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModalFocus {
    saved: Vec<Option<u64>>,
}

impl ModalFocus {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 安装焦点陷阱并记录当前焦点。
    pub fn open(&mut self, focus: &mut FocusManager, scope: &[u64]) {
        self.saved.push(focus.focused());
        focus.set_trap(scope.iter().copied());
    }

    /// 移除陷阱并恢复 [`Self::open`] 保存的焦点。
    ///
    /// 返回恢复的节点（若存在）。
    pub fn close(&mut self, focus: &mut FocusManager) -> Option<u64> {
        focus.clear_trap();
        match self.saved.pop() {
            Some(Some(key)) => {
                focus.focus(key, FocusCause::Programmatic);
                Some(key)
            }
            Some(None) => {
                focus.blur();
                None
            }
            None => None,
        }
    }

    /// 已保存的焦点槽位，最外层在前。
    #[must_use]
    pub fn saved(&self) -> &[Option<u64>] {
        &self.saved
    }

    /// 当前打开的模态框数量。
    #[must_use]
    pub fn depth(&self) -> usize {
        self.saved.len()
    }
}

/// 绑定了身份标识的模态框描述符。
#[derive(Clone, Debug)]
pub struct ModalWidget {
    /// 卡片节点。
    pub node: u64,
    /// 遮罩层覆盖的视口。
    pub viewport: Rect,
    /// 模态框描述符。
    pub desc: ModalDesc,
    /// 对话框内部的可聚焦节点列表。
    pub focus_scope: Vec<u64>,
}

impl WidgetDesc for ModalWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_modal(
            self.node,
            self.viewport,
            &self.desc,
            &self.focus_scope,
            state,
        )
        .output
    }
}

/// 单个模态框的发射结果。
#[derive(Clone, Debug)]
pub struct ModalOutput {
    /// 卡片及其命中区域。
    pub output: WidgetOutput,
    /// 遮罩层（必须在卡片**之外**单独安装）。
    pub scrim: HitRegion,
    /// 调用方需推入的浮层栈条目。
    pub entry: OverlayEntry,
    /// 当 `backdrop_blur > 0` 时的模糊请求。
    pub backdrop: Option<BackdropRequest>,
}

/// 发射模态对话框。
#[must_use]
pub fn emit_modal(
    node: u64,
    viewport: Rect,
    desc: &ModalDesc,
    focus_scope: &[u64],
    state: ControlState,
) -> ModalOutput {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    // 依次绘制遮罩层、阴影、卡片及其边框。
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        surface(viewport, 0.0, Token::SCRIM, desc.z),
        instance(
            rect,
            radii,
            Token::NONE,
            Token::NONE,
            0.0,
            MODAL_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(rect, desc.radius, Token::SURFACE_RAISED, desc.z),
        instance(
            rect,
            radii,
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];
    let mut payloads: Vec<GpuPayload> = Vec::new();

    if let Some(button) = desc.close_button_rect() {
        let inset = 6.0;
        let a = (button.origin.x + inset, button.origin.y + inset);
        let b = (button.max().x - inset, button.max().y - inset);
        let c = (button.max().x - inset, button.origin.y + inset);
        let d = (button.origin.x + inset, button.max().y - inset);
        for (from, to) in [(a, b), (c, d)] {
            let (primitive, payload) = close_stroke(from, to, Token::TEXT_SECONDARY, desc.z);
            primitives.push(primitive);
            payloads.push(payload);
        }
    }

    let card_hit = HitRegion::new(node, MODAL_LAYER, u32::from(desc.z), rect);
    let scrim = HitRegion::new(node ^ (1 << 63), PORTAL_LAYER, 0, viewport);
    let a11y = unbound_a11y::A11yNode::new(Role::Dialog, desc.label.clone(), rect)
        .disabled(state.disabled);
    let entry = OverlayEntry::modal(node, rect, focus_scope.to_vec());
    let backdrop = BackdropRequest::new(node, viewport, desc.backdrop_blur);

    ModalOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: Some(card_hit),
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        scrim,
        entry,
        backdrop,
    }
}

/// 关闭按钮叉号的单段二次描边。
fn close_stroke(
    from: (f32, f32),
    to: (f32, f32),
    color: Token,
    z: u16,
) -> (GpuPrimitiveInstance, GpuPayload) {
    let origin = Rect::new(
        from.0.min(to.0),
        from.1.min(to.1),
        (from.0 - to.0).abs().max(1.0),
        (from.1 - to.1).abs().max(1.0),
    );
    let primitive = instance(
        origin,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    let payload = GpuPayload::curve(
        [from.0 - origin.origin.x, from.1 - origin.origin.y],
        [
            (from.0 + to.0) * 0.5 - origin.origin.x,
            (from.1 + to.1) * 0.5 - origin.origin.y,
        ],
        [to.0 - origin.origin.x, to.1 - origin.origin.y],
        1.5,
    );
    (primitive, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 1280.0, 720.0)
    }

    #[test]
    fn a_modal_paints_a_scrim_card_and_border() {
        let desc = ModalDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0)).with_label("Settings");
        let output = emit_modal(5, viewport(), &desc, &[10, 11], ControlState::default());
        assert_eq!(output.output.primitives.len(), 4);
        assert_eq!(output.scrim.bounds, viewport());
        assert_eq!(output.scrim.layer, PORTAL_LAYER);
        assert_eq!(output.output.hit.as_ref().unwrap().layer, MODAL_LAYER);
        assert_eq!(output.entry.focus_scope, vec![10, 11]);
        assert_eq!(
            output.output.a11y.as_ref().map(|node| node.role),
            Some(Role::Dialog)
        );
    }

    #[test]
    fn a_blur_request_is_only_emitted_when_asked_for() {
        let desc = ModalDesc::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(
            emit_modal(1, viewport(), &desc, &[], ControlState::default())
                .backdrop
                .is_none()
        );
        let blurred = ModalDesc {
            backdrop_blur: 12.0,
            ..desc
        };
        let output = emit_modal(1, viewport(), &blurred, &[], ControlState::default());
        assert_eq!(output.backdrop.map(|request| request.blur), Some(12.0));
    }
}
