//! 模态确认对话框（AlertDialog）。
//!
//! AlertDialog 是具有固定三段式结构的 [`crate::modal::ModalDesc`]：
//! 图标与标题、正文消息、以及恰好包含两个操作的底部栏。这种固定结构使组件
//! 能够为调用方做出*安全*的默认选择：危险操作对话框会将初始键盘焦点放在
//! **取消**按钮上，防止误按 `Enter` 导致意外删除。
//!
//! 对话框会像通用模态框一样将遮罩层与卡片分开发射，因为两者都需要注册为命中区域。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, PORTAL_LAYER};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::modal::{MODAL_ACTION_GAP, MODAL_ACTION_WIDTH, MODAL_LAYER};
use crate::overlay::{BackdropRequest, OverlayEntry};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetOutput};

/// 对话框内容区内边距。
pub const DIALOG_PADDING: f32 = 20.0;
/// 底部栏高度。
pub const DIALOG_FOOTER: f32 = 56.0;
/// 操作按钮高度。
pub const DIALOG_BUTTON_HEIGHT: f32 = 32.0;
/// 严重级别图标直径。
pub const DIALOG_ICON: f32 = 22.0;
/// 默认圆角半径。
pub const DIALOG_RADIUS: f32 = 14.0;
const DIALOG_SHADOW: f32 = 40.0;

/// 确认操作的强调级别。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfirmTone {
    /// 中性、可逆的操作。
    #[default]
    Primary,
    /// 危险操作：显示为红色，且取消按钮获得初始焦点。
    Destructive,
}

impl ConfirmTone {
    /// 确认按钮填充色。
    #[must_use]
    pub fn fill(self) -> Token {
        match self {
            Self::Primary => Token::PRIMARY,
            Self::Destructive => Token::ERROR,
        }
    }

    /// 警告图标使用的强调色。
    #[must_use]
    pub fn accent(self) -> Token {
        match self {
            Self::Primary => Token::PRIMARY,
            Self::Destructive => Token::ERROR,
        }
    }
}

/// 哪个底部操作按钮持有键盘焦点。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlertFocus {
    /// 安全操作（取消）。
    Cancel,
    /// 提交操作（确认）。
    #[default]
    Confirm,
}

/// 对话框处理的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertDialogKey {
    /// 取消并关闭。
    Escape,
    /// 提交并关闭。
    Enter,
    /// 将焦点移至另一个操作。
    Tab,
    /// 反向移动焦点。
    ShiftTab,
}

/// 按键处理结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertDialogOutcome {
    /// 按键未被处理。
    Ignored,
    /// 对话框请求不提交直接关闭。
    Cancelled,
    /// 对话框请求提交并关闭。
    Confirmed,
    /// 焦点已移至另一个操作。
    FocusChanged(AlertFocus),
}

/// 确认对话框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct AlertDialogDesc {
    /// 对话框卡片矩形。
    pub rect: Rect,
    /// 主标题。
    pub title: String,
    /// 正文文案。
    pub message: String,
    /// 取消按钮标签。
    pub cancel_label: String,
    /// 确认按钮标签。
    pub confirm_label: String,
    /// 确认操作强调级别。
    pub tone: ConfirmTone,
    /// 圆角半径。
    pub radius: f32,
    /// 背景模糊半径；`0.0` 表示禁用模糊通道。
    pub backdrop_blur: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl AlertDialogDesc {
    /// 创建带标题的确认对话框。
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            title: title.into(),
            message: String::new(),
            cancel_label: "取消".to_owned(),
            confirm_label: "确认".to_owned(),
            tone: ConfirmTone::Primary,
            radius: DIALOG_RADIUS,
            backdrop_blur: 8.0,
            z: 960,
        }
    }

    /// 附加正文文案。
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// 设置明确的操作按钮文本对。
    #[must_use]
    pub fn with_actions(mut self, cancel: impl Into<String>, confirm: impl Into<String>) -> Self {
        self.cancel_label = cancel.into();
        self.confirm_label = confirm.into();
        self
    }

    /// 设为危险确认操作。
    #[must_use]
    pub fn destructive(mut self) -> Self {
        self.tone = ConfirmTone::Destructive;
        self
    }

    /// 对话框打开时的初始焦点。
    ///
    /// 危险操作绝不获取初始焦点：安全选项为默认值，这正是该对话框的设计初衷。
    #[must_use]
    pub fn initial_focus(&self) -> AlertFocus {
        match self.tone {
            ConfirmTone::Destructive => AlertFocus::Cancel,
            ConfirmTone::Primary => AlertFocus::Confirm,
        }
    }

    /// 初始焦点在 `[cancel, confirm]` 中的索引。
    #[must_use]
    pub fn initial_focus_index(&self) -> usize {
        match self.initial_focus() {
            AlertFocus::Cancel => 0,
            AlertFocus::Confirm => 1,
        }
    }

    /// 计算对话框各部分的几何布局。
    #[must_use]
    pub fn layout(&self) -> AlertDialogLayout {
        let pad = DIALOG_PADDING;
        let icon = Rect::new(
            self.rect.origin.x + pad,
            self.rect.origin.y + pad,
            DIALOG_ICON,
            DIALOG_ICON,
        );
        let title_x = icon.max().x + 12.0;
        let title = Rect::new(
            title_x,
            self.rect.origin.y + pad,
            (self.rect.max().x - pad - title_x).max(0.0),
            DIALOG_ICON,
        );
        let message = Rect::new(
            self.rect.origin.x + pad,
            icon.max().y + 14.0,
            (self.rect.size.x - pad * 2.0).max(0.0),
            (self.rect.max().y - DIALOG_FOOTER - icon.max().y - 14.0 - pad).max(0.0),
        );
        let footer = Rect::new(
            self.rect.origin.x,
            self.rect.max().y - DIALOG_FOOTER,
            self.rect.size.x,
            DIALOG_FOOTER,
        );
        let confirm = Rect::new(
            footer.max().x - pad - MODAL_ACTION_WIDTH,
            footer.origin.y + (DIALOG_FOOTER - DIALOG_BUTTON_HEIGHT) * 0.5,
            MODAL_ACTION_WIDTH,
            DIALOG_BUTTON_HEIGHT,
        );
        let cancel = Rect::new(
            confirm.origin.x - MODAL_ACTION_GAP - MODAL_ACTION_WIDTH,
            confirm.origin.y,
            MODAL_ACTION_WIDTH,
            DIALOG_BUTTON_HEIGHT,
        );
        AlertDialogLayout {
            icon,
            title,
            message,
            footer,
            cancel,
            confirm,
        }
    }
}

/// 确认对话框计算后的各矩形区域。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlertDialogLayout {
    /// 严重级别图标。
    pub icon: Rect,
    /// 主标题。
    pub title: Rect,
    /// 正文文案。
    pub message: Rect,
    /// 底部栏。
    pub footer: Rect,
    /// 安全操作。
    pub cancel: Rect,
    /// 提交操作。
    pub confirm: Rect,
}

/// 单个对话框的键盘状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertDialogState {
    focus: AlertFocus,
}

impl AlertDialogState {
    /// 使用描述符的安全默认焦点打开对话框。
    #[must_use]
    pub fn new(desc: &AlertDialogDesc) -> Self {
        Self {
            focus: desc.initial_focus(),
        }
    }

    /// 当前聚焦的操作。
    #[must_use]
    pub fn focus(&self) -> AlertFocus {
        self.focus
    }

    /// 显式移动焦点。
    pub fn set_focus(&mut self, focus: AlertFocus) {
        self.focus = focus;
    }

    /// 处理单个按键。
    pub fn handle_key(&mut self, key: AlertDialogKey) -> AlertDialogOutcome {
        match key {
            AlertDialogKey::Escape => AlertDialogOutcome::Cancelled,
            AlertDialogKey::Enter => AlertDialogOutcome::Confirmed,
            AlertDialogKey::Tab | AlertDialogKey::ShiftTab => {
                self.focus = match self.focus {
                    AlertFocus::Cancel => AlertFocus::Confirm,
                    AlertFocus::Confirm => AlertFocus::Cancel,
                };
                AlertDialogOutcome::FocusChanged(self.focus)
            }
        }
    }
}

/// 单个对话框的发射结果。
#[derive(Clone, Debug)]
pub struct AlertDialogOutput {
    /// 卡片、标题、消息与按钮。
    pub output: WidgetOutput,
    /// 遮罩层（在卡片之外单独安装）。
    pub scrim: HitRegion,
    /// 取消与确认命中区域（按此顺序）。
    pub actions: [HitRegion; 2],
    /// 调用方需推入的浮层条目。
    pub entry: OverlayEntry,
    /// 当 `backdrop_blur > 0` 时的模糊请求。
    pub backdrop: Option<BackdropRequest>,
    /// 计算后的布局。
    pub layout: AlertDialogLayout,
}

/// 确认对话框取消按钮的确定性子节点 ID。
#[must_use]
pub const fn alert_cancel_node(node: u64) -> u64 {
    node ^ (1 << 62)
}

/// 确认对话框确认按钮的确定性子节点 ID。
#[must_use]
pub const fn alert_confirm_node(node: u64) -> u64 {
    node ^ (1 << 61)
}

/// 发射确认对话框。
#[must_use]
pub fn emit_alert_dialog(
    node: u64,
    viewport: Rect,
    desc: &AlertDialogDesc,
    glyphs: &[GlyphQuad],
    control: ControlState,
) -> AlertDialogOutput {
    let layout = desc.layout();
    let accent = desc.tone.accent();
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        surface(viewport, 0.0, Token::SCRIM, desc.z),
        instance(
            desc.rect,
            [desc.radius; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            DIALOG_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(desc.rect, desc.radius, Token::SURFACE_RAISED, desc.z),
        instance(
            desc.rect,
            [desc.radius; 4],
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
        surface(layout.icon, layout.icon.size.x * 0.5, accent, desc.z + 1),
        surface(
            Rect::new(
                layout.icon.origin.x + layout.icon.size.x * 0.5 - 1.0,
                layout.icon.origin.y + 5.0,
                2.0,
                8.0,
            ),
            1.0,
            Token::ON_PRIMARY,
            desc.z + 2,
        ),
        surface(
            Rect::new(
                layout.icon.origin.x + layout.icon.size.x * 0.5 - 1.0,
                layout.icon.max().y - 7.0,
                2.0,
                2.0,
            ),
            1.0,
            Token::ON_PRIMARY,
            desc.z + 2,
        ),
        surface(
            Rect::new(
                layout.footer.origin.x,
                layout.footer.origin.y,
                layout.footer.size.x,
                1.0,
            ),
            0.0,
            Token::DIVIDER,
            desc.z + 1,
        ),
        // 取消按钮：描边表面。
        instance(
            layout.cancel,
            [8.0; 4],
            Token::SURFACE_RAISED,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Background,
            desc.z + 1,
        ),
        // 确认按钮：按色调填充。
        surface(layout.confirm, 8.0, desc.tone.fill(), desc.z + 1),
    ];
    let mut payloads = Vec::with_capacity(glyphs.len());
    crate::primitive::glyphs(
        glyphs,
        Token::TEXT_PRIMARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    let mut a11y =
        A11yNode::new(Role::AlertDialog, desc.title.clone(), desc.rect).disabled(control.disabled);
    a11y.description = desc.message.clone();

    let card_hit = HitRegion::new(node, MODAL_LAYER, u32::from(desc.z), desc.rect);
    let scrim = HitRegion::new(node ^ (1 << 63), PORTAL_LAYER, 0, viewport);
    let cancel_key = alert_cancel_node(node);
    let confirm_key = alert_confirm_node(node);
    let actions = [
        HitRegion::new(
            cancel_key,
            MODAL_LAYER,
            u32::from(desc.z) + 1,
            layout.cancel,
        )
        .with_cursor(CursorKind::Pointer),
        HitRegion::new(
            confirm_key,
            MODAL_LAYER,
            u32::from(desc.z) + 1,
            layout.confirm,
        )
        .with_cursor(CursorKind::Pointer),
    ];

    let mut output = WidgetOutput {
        primitives,
        payloads,
        hit: Some(card_hit),
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    };
    if !control.disabled {
        output.with_sub_control(
            cancel_key,
            actions[0].clone(),
            A11yNode::new(Role::Button, desc.cancel_label.clone(), layout.cancel).clickable(),
            true,
        );
        output.with_sub_control(
            confirm_key,
            actions[1].clone(),
            A11yNode::new(Role::Button, desc.confirm_label.clone(), layout.confirm).clickable(),
            true,
        );
    }

    AlertDialogOutput {
        output,
        scrim,
        actions,
        entry: OverlayEntry::modal(node, desc.rect, Vec::new()),
        backdrop: BackdropRequest::new(node, viewport, desc.backdrop_blur),
        layout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> AlertDialogDesc {
        AlertDialogDesc::new(Rect::new(340.0, 300.0, 420.0, 200.0), "Delete workspace?")
            .with_message("This cannot be undone.")
    }

    #[test]
    fn a_destructive_dialog_opens_on_cancel() {
        let desc = desc().destructive();
        assert_eq!(desc.initial_focus(), AlertFocus::Cancel);
        assert_eq!(desc.initial_focus_index(), 0);
        assert_eq!(AlertDialogState::new(&desc).focus(), AlertFocus::Cancel);
    }

    #[test]
    fn a_neutral_dialog_opens_on_confirm() {
        let desc = desc();
        assert_eq!(desc.initial_focus(), AlertFocus::Confirm);
        assert_eq!(desc.initial_focus_index(), 1);
    }

    #[test]
    fn enter_and_escape_report_the_two_outcomes() {
        let mut state = AlertDialogState::new(&desc());
        assert_eq!(
            state.handle_key(AlertDialogKey::Escape),
            AlertDialogOutcome::Cancelled
        );
        assert_eq!(
            state.handle_key(AlertDialogKey::Enter),
            AlertDialogOutcome::Confirmed
        );
    }

    #[test]
    fn tab_moves_between_the_two_actions() {
        let mut state = AlertDialogState::new(&desc());
        assert_eq!(
            state.handle_key(AlertDialogKey::Tab),
            AlertDialogOutcome::FocusChanged(AlertFocus::Cancel)
        );
        assert_eq!(
            state.handle_key(AlertDialogKey::Tab),
            AlertDialogOutcome::FocusChanged(AlertFocus::Confirm)
        );
        assert_eq!(
            state.handle_key(AlertDialogKey::ShiftTab),
            AlertDialogOutcome::FocusChanged(AlertFocus::Cancel)
        );
    }

    #[test]
    fn the_actions_sit_right_aligned_inside_the_footer() {
        let layout = desc().layout();
        assert!(desc().rect.contains_rect(&layout.cancel));
        assert!(desc().rect.contains_rect(&layout.confirm));
        assert_eq!(layout.confirm.max().x, desc().rect.max().x - DIALOG_PADDING);
        assert_eq!(layout.confirm.origin.y, layout.cancel.origin.y);
        assert!(layout.cancel.max().x < layout.confirm.origin.x);
        assert_eq!(layout.footer.size.x, desc().rect.size.x);
    }

    #[test]
    fn the_emitter_paints_a_scrim_card_and_two_buttons() {
        let output = emit_alert_dialog(
            7,
            Rect::new(0.0, 0.0, 1280.0, 720.0),
            &desc(),
            &[],
            ControlState::default(),
        );
        assert_eq!(output.scrim.layer, PORTAL_LAYER);
        assert_eq!(output.output.hit.as_ref().unwrap().layer, MODAL_LAYER);
        assert_eq!(output.actions.len(), 2);
        let a11y = output.output.a11y.expect("dialog");
        assert_eq!(a11y.role, Role::AlertDialog);
        assert_eq!(a11y.label, "Delete workspace?");
        assert!(!output.output.primitives.is_empty());
    }

    #[test]
    fn a_destructive_confirm_paints_the_error_fill() {
        let output = emit_alert_dialog(
            7,
            Rect::new(0.0, 0.0, 1280.0, 720.0),
            &desc().destructive(),
            &[],
            ControlState::default(),
        );
        let confirm = output.layout.confirm;
        let primitive = output
            .output
            .primitives
            .iter()
            .find(|primitive| {
                primitive.local_pos == [confirm.origin.x, confirm.origin.y]
                    && primitive.size == [confirm.size.x, confirm.size.y]
            })
            .expect("confirm surface");
        assert_eq!(primitive.fill_token_or_uv, u32::from(Token::ERROR.raw()));
    }

    #[test]
    fn the_backdrop_request_follows_the_blur_setting() {
        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let output = emit_alert_dialog(7, viewport, &desc(), &[], ControlState::default());
        assert_eq!(output.backdrop.map(|request| request.blur), Some(8.0));
        let sharp = AlertDialogDesc {
            backdrop_blur: 0.0,
            ..desc()
        };
        let output = emit_alert_dialog(7, viewport, &sharp, &[], ControlState::default());
        assert!(output.backdrop.is_none());
    }
}
