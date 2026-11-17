//! 扁平按钮（Button）：首个标准控件。
//!
//! 按钮仅由三部分组成：
//!
//! * 纯数据描述符 [`ButtonDesc`]，
//! * 派生自指针路由器的视觉状态（[`ControlState`]），
//! * 纯发射器 [`emit_button`]：`(descriptor, state) -> 图元 + 命中区域 + 语义节点`。
//!
//! 交互瞬态（悬停 / 按下）绝不落入 `bool` 字段：每帧从路由器派生，
//! 因此“按下、拖出、释放”不会导致两个状态相互冲突。
//!
//! 图元通过 [`SubElement`] 分层
//! （`Shadow -> Background -> Content -> Border -> FocusRing`），而非手动调整
//! `z` 偏移，因此由 64 位排序键决定绘制顺序。

use accesskit::Role;
use slotmap::Key as _;
use unbound_a11y::A11yNode;
use unbound_abi::{NodeFlags, RawNodeDesc};
use unbound_gpu::data::{
    BlendMode, GpuPayload, GpuPrimitiveInstance, PrimFlags, PrimType, SubElement, CLIP_NONE,
};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{f16, Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_world::{NodeId, UiWorld};

use crate::image::IconDesc;
use crate::primitive::focus_ring;
use crate::{ControlState, PointerPhase, WidgetDesc, WidgetOutput};

/// 按钮标签字体大小。字形预热必须使用相同字号，否则数字会被光栅化两次。
pub const BUTTON_FONT_SIZE: f32 = 14.0;

/// 按钮圆角半径。对齐 WinUI `ControlCornerRadius`（4）。
pub const BUTTON_RADIUS: f32 = 4.0;

/// 按钮标准内边距（左右、上、下），对齐 WinUI `ButtonPadding = 11,5,11,6`。
///
/// 垂直方向不对称：文字基线略偏上，是 Win11 按钮的排版特征。
pub const BUTTON_PADDING: (f32, f32, f32, f32) = (11.0, 5.0, 11.0, 6.0);

/// 按钮描边宽度，对齐 WinUI `ButtonBorderThemeThickness = 1`。
pub const BUTTON_BORDER: f32 = 1.0;

/// 标签与前置/后置图标之间的间距。
pub const BUTTON_CONTENT_GAP: f32 = 8.0;

/// 加载指示器直径。
pub const BUTTON_LOADING_SIZE: f32 = 12.0;

/// 按钮高度（逻辑像素）。
///
/// 尺寸属于元数据：已完成布局的调用方保留自身矩形，但工具栏可请求标准高度而非硬编码数值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    /// 紧凑操作按钮，高 28 px。
    Sm,
    /// 标准操作按钮，高 32 px（WinUI `Button` 的标准高度）。
    #[default]
    Md,
    /// 醒目操作按钮，高 44 px。
    Lg,
}

impl ButtonSize {
    /// 该尺寸对应的控件高度。
    #[must_use]
    pub const fn height(self) -> f32 {
        match self {
            Self::Sm => 28.0,
            Self::Md => 32.0,
            Self::Lg => 44.0,
        }
    }
}

/// 按钮命中区域所在的 UI 层级（`0` 画布、`1` UI、`2..` 浮层）。
pub const BUTTON_LAYER: u8 = 1;

/// 解析后阴影的模糊半径。`PrimType::Shadow` 仅绘制阴影，不绘制填充。
const BUTTON_SHADOW: f32 = 16.0;

/// 焦点环向按钮外侧延伸的距离。
const FOCUS_RING_GAP: f32 = 3.0;

/// 按钮的视觉角色。
///
/// 变体决定按钮使用*哪些*颜色槽位；主题决定这些槽位的外观。因此新增变体无需改动任何图元。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// 主行动号召：实心高对比度表面。
    #[default]
    Primary,
    /// 辅助操作：描边表面。
    Secondary,
    /// 低强调操作：无填充、无边框。
    Subtle,
    /// 紧凑的纯图标操作。
    Icon,
    /// 绘制在主视觉渐变背景上的按钮。
    OnHero,
}

impl ButtonVariant {
    /// 此变体绘制使用的颜色槽位。
    ///
    /// 每种状态都有独立槽位，因此悬停按钮不会重写实例数据——仅改变槽位解析出的颜色。
    #[must_use]
    pub fn tokens(self) -> ButtonTokens {
        match self {
            ButtonVariant::Primary => ButtonTokens {
                fill: Token::PRIMARY,
                fill_hover: Token::PRIMARY_HOVER,
                fill_pressed: Token::PRIMARY_PRESSED,
                fill_disabled: Token::DISABLED,
                border: Token::PRIMARY_PRESSED,
                label: Token::ON_PRIMARY,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            },
            ButtonVariant::Secondary => ButtonTokens {
                fill: Token::SURFACE_RAISED,
                fill_hover: Token::CONTROL_HOVER,
                fill_pressed: Token::SURFACE_SUNKEN,
                fill_disabled: Token::DISABLED,
                border: Token::OUTLINE,
                label: Token::ON_SURFACE,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            },
            ButtonVariant::Subtle => ButtonTokens {
                fill: Token::NONE,
                fill_hover: Token::CONTROL_HOVER,
                fill_pressed: Token::SURFACE_SUNKEN,
                fill_disabled: Token::NONE,
                border: Token::NONE,
                label: Token::ON_SURFACE_MUTED,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            },
            ButtonVariant::Icon => ButtonTokens {
                fill: Token::NONE,
                fill_hover: Token::CONTROL_HOVER,
                fill_pressed: Token::SURFACE_SUNKEN,
                fill_disabled: Token::NONE,
                border: Token::NONE,
                label: Token::ON_SURFACE,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            },
            ButtonVariant::OnHero => ButtonTokens {
                fill: Token::HERO_SCRIM,
                fill_hover: Token::ON_HERO,
                fill_pressed: Token::HERO_START,
                fill_disabled: Token::HERO_SCRIM,
                border: Token::ON_HERO,
                label: Token::ON_HERO,
                label_disabled: Token::ON_HERO,
                focus_ring: Token::ON_HERO,
            },
        }
    }
}

/// 按钮绘制使用的颜色槽位。
///
/// 发射器只认槽位，主题决定颜色。每种状态在扁平颜色表中占据独立槽位，
/// 因此切换主题与悬停状态均无需重新上传任何图元。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonTokens {
    /// 静息状态填充色。
    pub fill: Token,
    /// 悬停状态填充色。
    pub fill_hover: Token,
    /// 按下状态填充色。
    pub fill_pressed: Token,
    /// 禁用状态填充色。
    pub fill_disabled: Token,
    /// 轮廓线颜色。
    pub border: Token,
    /// 标签文字颜色。
    pub label: Token,
    /// 禁用状态下的标签文字颜色。
    pub label_disabled: Token,
    /// 键盘焦点环颜色。
    pub focus_ring: Token,
}

/// 按钮的纯数据描述符。
///
/// 无任何可变方法，不携带闭包或类型擦除：这是 `RawNodeDesc` 收敛的目标形态，
/// 第二个控件只需编写同类结构体即可。
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonDesc {
    /// 按钮矩形区域。
    pub rect: Rect,
    /// 按钮标签文本。
    pub label: String,
    /// 圆角半径。
    pub radius: f32,
    /// 标签字号。
    pub font_size: f32,
    /// 绘制顺序键。
    pub z: u16,
    /// 视觉角色。
    pub variant: ButtonVariant,
    /// 标准尺寸（28 / 36 / 44 px）。
    pub size: ButtonSize,
    /// 绘制在标签旁的图标。
    pub icon: Option<IconDesc>,
    /// 将图标绘制在标签之后而非之前。
    pub icon_trailing: bool,
    /// 显示加载指示器并禁用交互。
    pub loading: bool,
    /// 解析投影模糊半径。`0.0` 表示不绘制阴影。
    pub shadow: f32,
    /// 按墨水边界居中标签。
    pub centre_label: bool,
    /// 焦点环向按钮外侧延伸的距离。
    pub focus_ring_gap: f32,
    /// 文本左对齐（适用于侧边栏菜单项、列表行按钮与下拉触发器）。
    pub left_align: bool,
    /// 是否处于激活/选中态（渲染选中高亮表面与左侧指示条）。
    pub active_selected: bool,
    /// 左对齐时标签与按钮左沿的间距。
    pub label_inset: f32,
    /// 覆盖变体默认的颜色槽位。
    ///
    /// 变体只决定「用哪一组槽位」，主题决定「槽位长什么样」；
    /// 但 Win11 的下拉列表项并不属于任何现成变体——它的选中态是
    /// 「浅一档的高亮块 + 左侧强调条」，而不是按钮的填充/描边。
    /// 因此这里允许调用方直接给出槽位集合。
    pub tokens_override: Option<ButtonTokens>,
}

impl ButtonDesc {
    /// 使用默认度量的主按钮。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            radius: BUTTON_RADIUS,
            font_size: BUTTON_FONT_SIZE,
            z: 703,
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            icon: None,
            icon_trailing: false,
            loading: false,
            shadow: BUTTON_SHADOW,
            centre_label: true,
            focus_ring_gap: FOCUS_RING_GAP,
            left_align: false,
            active_selected: false,
            label_inset: 12.0,
            tokens_override: None,
        }
    }

    /// 将标签设为左对齐。
    #[must_use]
    pub fn left_align(mut self) -> Self {
        self.left_align = true;
        self.centre_label = false;
        self
    }

    /// 设置按钮激活/选中高亮态。
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.active_selected = selected;
        self
    }

    /// 紧凑无阴影按钮样式（28px 高，6px 圆角）。
    #[must_use]
    pub fn compact(mut self) -> Self {
        self.size = ButtonSize::Sm;
        self.radius = 6.0;
        self.font_size = 12.5;
        self.shadow = 0.0;
        self
    }

    /// 自定义圆角半径。
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// 自定义字体大小。
    #[must_use]
    pub fn with_font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    /// 侧边栏一级导航菜单项。
    #[must_use]
    pub fn nav_item(label: impl Into<String>, active: bool) -> Self {
        Self::new(Rect::ZERO, label)
            .with_variant(if active {
                ButtonVariant::Secondary
            } else {
                ButtonVariant::Subtle
            })
            .with_size(ButtonSize::Sm)
            .with_shadow(0.0)
            .with_radius(6.0)
            .with_font_size(12.5)
            .left_align()
            .selected(active)
    }

    /// 侧边栏二级子菜单项。
    #[must_use]
    pub fn nav_sub_item(label: impl Into<String>, active: bool) -> Self {
        Self::nav_item(label, active).with_font_size(12.0)
    }

    /// 顶部/页内分段标签页按钮。
    #[must_use]
    pub fn tab_item(label: impl Into<String>, active: bool) -> Self {
        Self::new(Rect::ZERO, label)
            .with_variant(if active {
                ButtonVariant::Secondary
            } else {
                ButtonVariant::Subtle
            })
            .with_size(ButtonSize::Sm)
            .with_shadow(0.0)
            .with_radius(6.0)
            .with_font_size(12.5)
            .selected(active)
    }

    /// 胶囊筛选按钮（如命令面板分类、通信通道筛选器）。
    #[must_use]
    pub fn pill_filter(label: impl Into<String>, active: bool) -> Self {
        Self::new(Rect::ZERO, label)
            .with_variant(if active {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .with_size(ButtonSize::Sm)
            .with_shadow(0.0)
            .with_radius(11.0)
            .with_font_size(11.5)
            .selected(active)
    }

    /// 单选列表项（WinUI `ListViewItem` 选中态）。
    ///
    /// Win11 的单选列表用**浅底 + 左侧 3px 强调条**标出当前项，
    /// 文字始终是正文色；它不是实心填充的按钮，也不是胶囊。
    #[must_use]
    pub fn list_item(label: impl Into<String>, active: bool) -> Self {
        let slot = if active {
            ButtonTokens {
                fill: Token::NAV_SELECTED,
                fill_hover: Token::CONTROL_HOVER,
                fill_pressed: Token::SURFACE_SUNKEN,
                fill_disabled: Token::NONE,
                border: Token::NONE,
                label: Token::TEXT_PRIMARY,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::NONE,
            }
        } else {
            ButtonTokens {
                fill: Token::NONE,
                fill_hover: Token::CONTROL_HOVER,
                fill_pressed: Token::SURFACE_SUNKEN,
                fill_disabled: Token::NONE,
                border: Token::NONE,
                label: Token::TEXT_PRIMARY,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::NONE,
            }
        };
        Self::new(Rect::ZERO, label)
            .variant(ButtonVariant::Subtle)
            .with_size(ButtonSize::Sm)
            .with_shadow(0.0)
            .with_radius(4.0)
            .left_align()
            .selected(active)
            .with_tokens(slot)
    }

    /// Win11 切换按钮（`ToggleButton`）。
    ///
    /// Win11 的 ToggleButton 有两副面孔：
    /// - **未按下**：透明底 + 一圈淡描边，正文色文字；
    /// - **按下**：整颗按钮被强调色填满，文字换成强调色的反色。
    ///
    /// 用普通按钮的 Secondary 变体冒充它，会丢掉「当前处于开启态」这层信息——
    /// 开关按下去之后看上去和按之前一模一样。
    #[must_use]
    pub fn toggle_button(label: impl Into<String>, on: bool) -> Self {
        let slot = if on {
            ButtonTokens {
                fill: Token::PRIMARY,
                fill_hover: Token::PRIMARY_HOVER,
                fill_pressed: Token::PRIMARY_PRESSED,
                fill_disabled: Token::DISABLED,
                border: Token::NONE,
                label: Token::ON_PRIMARY,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            }
        } else {
            ButtonTokens {
                fill: Token::NONE,
                fill_hover: Token::CONTROL_HOVER,
                fill_pressed: Token::SURFACE_SUNKEN,
                fill_disabled: Token::NONE,
                border: Token::CONTROL_STROKE,
                label: Token::TEXT_PRIMARY,
                label_disabled: Token::ON_DISABLED,
                focus_ring: Token::FOCUS_RING,
            }
        };
        Self::new(Rect::ZERO, label)
            .variant(ButtonVariant::Subtle)
            .with_size(ButtonSize::Sm)
            .with_shadow(0.0)
            .with_radius(4.0)
            .with_tokens(slot)
    }

    /// 紧凑图标/工具栏按钮。
    #[must_use]
    pub fn icon_btn(label: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, label)
            .with_variant(ButtonVariant::Subtle)
            .with_size(ButtonSize::Sm)
            .with_shadow(0.0)
            .with_radius(5.0)
            .with_font_size(12.0)
    }

    /// 修改标签的同款按钮：点击计数器只需重建描述符，无需重建标识。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 切换视觉角色的同款按钮。
    #[must_use]
    pub fn with_variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// 切换标准尺寸的同款按钮。
    #[must_use]
    pub fn with_size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// 在标签旁附加图标的同款按钮。
    #[must_use]
    pub fn with_icon(mut self, icon: IconDesc) -> Self {
        self.icon = Some(icon);
        self
    }

    /// 将图标置于标签之后而非之前。
    #[must_use]
    pub fn with_icon_trailing(mut self, trailing: bool) -> Self {
        self.icon_trailing = trailing;
        self
    }

    /// 处于加载状态的同款按钮：不可点击，左侧显示指示器。
    #[must_use]
    pub fn with_loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// 修改阴影模糊半径的同款按钮（`0.0` 表示无阴影）。
    #[must_use]
    pub fn with_shadow(mut self, shadow: f32) -> Self {
        self.shadow = shadow;
        self
    }

    /// 按调用方布局位置绘制标签，不重新居中。
    #[must_use]
    pub fn with_centre_label(mut self, centre: bool) -> Self {
        self.centre_label = centre;
        self
    }

    /// 修改焦点环外扩间距的同款按钮。
    #[must_use]
    pub fn with_focus_ring_gap(mut self, gap: f32) -> Self {
        self.focus_ring_gap = gap;
        self
    }

    /// 此按钮绘制使用的颜色槽位。
    #[must_use]
    pub fn tokens(&self) -> ButtonTokens {
        if let Some(override_tokens) = self.tokens_override {
            return override_tokens;
        }
        let mut t = self.variant.tokens();
        if self.active_selected && self.variant != ButtonVariant::Primary {
            t.fill = Token::NAV_SELECTED;
            t.fill_hover = Token::CONTROL_HOVER;
            t.border = Token::PRIMARY;
            t.label = Token::TEXT_PRIMARY;
        }
        t
    }

    /// 覆盖颜色槽位（绕过变体默认）。
    #[must_use]
    pub fn with_tokens(mut self, tokens: ButtonTokens) -> Self {
        self.tokens_override = Some(tokens);
        self
    }

    /// 设置左对齐标签与按钮左沿的间距。
    #[must_use]
    pub fn with_label_inset(mut self, inset: f32) -> Self {
        self.label_inset = inset;
        self
    }

    /// 指示器矩形：左对齐且垂直居中。
    #[must_use]
    pub fn loading_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + BUTTON_CONTENT_GAP,
            self.rect.origin.y + (self.rect.size.y - BUTTON_LOADING_SIZE) * 0.5,
            BUTTON_LOADING_SIZE,
            BUTTON_LOADING_SIZE,
        )
    }
}

/// 将键盘激活折叠进视觉状态。
///
/// 指针路由器不会接收 Space 或 Enter。用这两个键激活聚焦按钮时，
/// 其绘制效果必须与指针按下完全一致，因此调用方在发射按钮前先经由此处路由按键；
/// 发射器只需响应 [`PointerPhase::Pressed`] 即可。
#[must_use]
pub fn keyboard_activation(state: ControlState, space_or_enter_down: bool) -> ControlState {
    if space_or_enter_down && state.focused && !state.disabled {
        ControlState {
            phase: PointerPhase::Pressed,
            ..state
        }
    } else {
        state
    }
}

/// 绑定了身份标识的按钮。
///
/// 描述符是纯数据，不含标识；测试工具与场景收集器操作的是此包装类型。
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonWidget {
    /// 控件标识。
    pub node: u64,
    /// 按钮描述符。
    pub desc: ButtonDesc,
}

impl WidgetDesc for ButtonWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, glyphs: &mut [GlyphQuad]) -> WidgetOutput {
        emit_button(self.node, &self.desc, state, glyphs)
    }
}

/// 在 [`UiWorld`] 中注册按钮标识并返回其 [`NodeId`]。
///
/// 此处仅注册标识与布局意图：第一步直接按描述符确定按钮尺寸，完整布局引擎留待后续。
pub fn insert_button(world: &mut UiWorld, rect: Rect) -> NodeId {
    let desc = RawNodeDesc::container()
        .size(rect.width(), rect.height())
        .radius(BUTTON_RADIUS)
        .flags(NodeFlags::INTERACTIVE | NodeFlags::FOCUSABLE);
    let root = world.root();
    world.insert(root, desc, core::panic::Location::caller())
}

/// [`NodeId`] 与输入层、语义层共享的 `u64` 标识之间的桥梁。
#[must_use]
pub fn node_u64(node: NodeId) -> u64 {
    node.data().as_ffi()
}

/// 纯发射器：相同输入始终产出相同的图元、命中区域与语义节点。
///
/// 传入的标签为未排版字形（原点靠近左上角）；发射器将其在按钮矩形内居中，
/// 因此标签宽度变化不会破坏按钮自身布局，调用方仍能获得最终场景坐标。
#[must_use]
pub fn emit_button(
    node: u64,
    desc: &ButtonDesc,
    state: ControlState,
    label: &mut [GlyphQuad],
) -> WidgetOutput {
    emit_button_with_tokens(node, desc, state, desc.tokens(), label)
}

/// 使用显式颜色槽位集合的纯发射器。
#[must_use]
pub fn emit_button_with_tokens(
    node: u64,
    desc: &ButtonDesc,
    state: ControlState,
    tokens: ButtonTokens,
    label: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    let z = desc.z;
    let mut primitives = Vec::with_capacity(8 + label.len());
    let mut payloads = Vec::with_capacity(label.len() + 1);

    // 加载中的按钮不是交互目标，也不显示焦点环：旋转指示器表示“请等待”，此时不应再接受第二次激活。
    let blocked = state.disabled || desc.loading;

    // 1. 阴影：专用 Shadow 图元，填充与边框均透明，因此仅绘制解析阴影。
    primitives.push(instance(
        rect,
        radii,
        Token::NONE,
        Token::NONE,
        0.0,
        desc.shadow,
        PrimType::Shadow,
        SubElement::Shadow,
        z,
    ));

    // 2. 背景：状态颜色来自颜色表槽位。
    primitives.push(instance(
        rect,
        radii,
        state_fill(state, tokens),
        Token::NONE,
        0.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Background,
        z,
    ));

    // 3. 图标、加载指示器与标签共享同一居中内容行。指示器始终前置，
    //    因此加载状态切换时按钮不会产生水平跳动。
    let ink = ink_bounds(label);
    let leading_icon = desc.icon.is_some() && !desc.icon_trailing;
    let mut content_width = ink.map_or(0.0, |bounds| bounds.width());
    if desc.loading {
        content_width += BUTTON_LOADING_SIZE + BUTTON_CONTENT_GAP;
    } else if leading_icon {
        content_width += desc.font_size + BUTTON_CONTENT_GAP;
    }
    if desc.icon.is_some() && desc.icon_trailing {
        content_width += desc.font_size + BUTTON_CONTENT_GAP;
    }
    let mut cursor = if desc.left_align {
        rect.origin.x + desc.label_inset
    } else {
        rect.center().x - content_width * 0.5
    };

    if desc.loading {
        let indicator = desc.loading_rect();
        let color = if state.disabled {
            tokens.label_disabled
        } else {
            tokens.label
        };
        primitives.push(instance(
            indicator,
            [BUTTON_LOADING_SIZE * 0.5; 4],
            Token::NONE,
            color,
            2.0,
            0.0,
            PrimType::Stroke,
            SubElement::Content,
            z,
        ));
        cursor += BUTTON_LOADING_SIZE + BUTTON_CONTENT_GAP;
    } else if leading_icon {
        if let Some(icon) = &desc.icon {
            icon_at(
                icon,
                cursor,
                rect.center().y,
                desc.font_size,
                z,
                &mut primitives,
                &mut payloads,
            );
            cursor += desc.font_size + BUTTON_CONTENT_GAP;
        }
    }

    // 标签：与按钮同 `z`，稍后通过 `SubElement::Content` 绘制。
    if let Some(ink) = ink {
        if desc.centre_label || desc.left_align {
            let offset = Vec2::new(
                cursor - ink.min().x,
                rect.center().y - (ink.min().y + ink.height() * 0.5),
            );
            for quad in label.iter_mut() {
                quad.rect.origin.x += offset.x;
                quad.rect.origin.y += offset.y;
            }
        }
        let color = if state.disabled {
            tokens.label_disabled
        } else {
            tokens.label
        };
        for quad in label {
            let payload_idx = payloads.len() as u32;
            payloads.push(GpuPayload {
                p0: [quad.uv[0], quad.uv[1]],
                p1: [quad.uv[2], quad.uv[3]],
                p2: [0.0, 0.0],
                line_width: 0.0,
                extra: 0,
                flags: 0,
                _reserved: [0; 3],
            });
            let mut glyph = instance(
                quad.rect,
                [0.0; 4],
                color,
                Token::NONE,
                0.0,
                0.0,
                if quad.color {
                    PrimType::GlyphColor
                } else {
                    PrimType::Glyph
                },
                SubElement::Content,
                z,
            );
            // 图集层通过边框槽位传递，背景不使用该槽位。
            glyph.border_token_or_tex = quad.layer;
            glyph.extra_payload_idx = payload_idx;
            if quad.msdf {
                glyph.prim_type_and_flags |= u32::from(PrimFlags::MSDF.bits()) << 16;
            }
            primitives.push(glyph);
        }
        cursor += ink.width();
    }

    if desc.icon.is_some() && desc.icon_trailing {
        if let Some(icon) = &desc.icon {
            icon_at(
                icon,
                cursor + BUTTON_CONTENT_GAP,
                rect.center().y,
                desc.font_size,
                z,
                &mut primitives,
                &mut payloads,
            );
        }
    }

    // 4. 边框：空心圆角矩形，属于独立子元素。
    primitives.push(instance(
        rect,
        radii,
        Token::NONE,
        tokens.border,
        BUTTON_BORDER,
        0.0,
        PrimType::RoundedRect,
        SubElement::Border,
        z,
    ));

    if desc.active_selected && desc.left_align {
        let bar_h = (rect.size.y * 0.52).clamp(10.0, 16.0);
        let bar_rect = Rect::new(
            rect.origin.x + 2.0,
            rect.origin.y + (rect.size.y - bar_h) * 0.5,
            3.0,
            bar_h,
        );
        primitives.push(instance(
            bar_rect,
            [1.5; 4],
            Token::PRIMARY,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            z,
        ));
    }

    // 5. 焦点环：外扩描边，位于最顶层。它始终被发射——聚焦仅改变其不透明度——
    //    因此聚焦按钮只需重绘一个图元，而不会打乱图元池。
    primitives.push(focus_ring(
        rect.inflate(desc.focus_ring_gap),
        desc.radius + desc.focus_ring_gap,
        tokens.focus_ring,
        2.0,
        z,
        state.focused && !desc.loading,
    ));

    let hit = (!blocked).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Button, desc.label.clone(), rect)
        .clickable()
        .disabled(blocked);
    // 选中态必须进入语义：药丸筛选、分段「多选一」这类按钮，Agent 需要知道
    // 「当前选的是哪一个」。丢掉它，Agent 就只能靠读视觉样式去猜。
    a11y.selected = desc.active_selected;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !blocked,
        ..Default::default()
    }
}

/// 发射单个图标图元，锚定于 `left` 并垂直居中。
#[allow(clippy::too_many_arguments)]
fn icon_at(
    icon: &IconDesc,
    left: f32,
    center_y: f32,
    side: f32,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let icon_rect = Rect::new(left, center_y - side * 0.5, side, side);
    let payload_idx = payloads.len() as u32;
    payloads.push(GpuPayload {
        p0: [0.0, 0.0],
        p1: [1.0, 1.0],
        p2: [0.0, 0.0],
        line_width: 0.0,
        extra: 0,
        flags: 0,
        _reserved: [0; 3],
    });
    let mut primitive = instance(
        icon_rect,
        [0.0; 4],
        icon.tint,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Image,
        SubElement::Content,
        z,
    );
    primitive.border_token_or_tex = icon.texture_layer;
    primitive.extra_payload_idx = payload_idx;
    primitives.push(primitive);
}

/// 状态绘制使用的填充槽位。
fn state_fill(state: ControlState, tokens: ButtonTokens) -> Token {
    if state.disabled {
        return tokens.fill_disabled;
    }
    match state.phase {
        PointerPhase::Idle => tokens.fill,
        PointerPhase::Hovered => tokens.fill_hover,
        PointerPhase::Pressed => tokens.fill_pressed,
    }
}

/// 字形串的墨水边界框，用于居中计算及无头文本测试。
#[must_use]
pub fn ink_bounds(glyphs: &[GlyphQuad]) -> Option<Rect> {
    let mut bounds = glyphs.first()?.rect;
    for glyph in &glyphs[1..] {
        bounds = bounds.union(&glyph.rect);
    }
    Some(bounds)
}

#[allow(clippy::too_many_arguments)]
fn instance(
    rect: Rect,
    radii: [f32; 4],
    fill: Token,
    border: Token,
    border_width: f32,
    shadow: f32,
    kind: PrimType,
    sub: SubElement,
    z: u16,
) -> GpuPrimitiveInstance {
    GpuPrimitiveInstance {
        chunk_id: [0, 0],
        local_pos: [rect.origin.x, rect.origin.y],
        size: [rect.size.x, rect.size.y],
        corner_radii: f16::pack_radii(radii),
        border_width: f16::f32_to_f16(if border == Token::NONE {
            0.0
        } else {
            border_width
        }),
        shadow_blur: f16::f32_to_f16(shadow),
        transform_idx: 0,
        clip_idx: CLIP_NONE,
        fill_token_or_uv: u32::from(fill.raw()),
        border_token_or_tex: u32::from(border.raw()),
        prim_type_and_flags: GpuPrimitiveInstance::pack_type(kind, BlendMode::Normal, sub as u16),
        z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
        extra_payload_idx: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WidgetHarness;

    /// 位于视口中央的按钮描述符。
    fn desc() -> ButtonDesc {
        ButtonDesc::new(Rect::new(100.0, 100.0, 160.0, 44.0), "Press me")
    }

    /// 背景图元的填充令牌。
    fn fill_token(output: &WidgetOutput) -> u16 {
        output
            .primitives
            .iter()
            .find(|primitive| primitive.prim_type() == PrimType::RoundedRect)
            .map(|primitive| primitive.fill_token_or_uv as u16)
            .expect("the button always paints a background")
    }

    #[test]
    fn a_button_paints_the_slot_of_its_state() {
        let tokens = ButtonVariant::Primary.tokens();
        let mut glyphs: Vec<GlyphQuad> = Vec::new();

        let idle = emit_button(1, &desc(), ControlState::default(), &mut glyphs);
        assert_eq!(fill_token(&idle), tokens.fill.raw());

        let hovered = emit_button(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut glyphs,
        );
        assert_eq!(fill_token(&hovered), tokens.fill_hover.raw());

        let pressed = emit_button(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Pressed,
                ..ControlState::default()
            },
            &mut glyphs,
        );
        assert_eq!(fill_token(&pressed), tokens.fill_pressed.raw());
    }

    #[test]
    fn a_disabled_button_leaves_the_hit_set() {
        let mut glyphs: Vec<GlyphQuad> = Vec::new();
        let output = emit_button(
            1,
            &desc(),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &mut glyphs,
        );
        assert!(
            output.hit.is_none(),
            "non-interactive controls must disappear from the hit set"
        );
        assert!(output.a11y.is_some());
        assert!(!output.focusable);
    }

    #[test]
    fn every_variant_paints_a_distinguishable_combination() {
        let variants = [
            ButtonVariant::Primary,
            ButtonVariant::Secondary,
            ButtonVariant::Subtle,
            ButtonVariant::Icon,
            ButtonVariant::OnHero,
        ];
        let mut seen: Vec<ButtonTokens> = Vec::new();
        for variant in variants {
            let tokens = variant.tokens();
            assert!(
                !seen.contains(&tokens),
                "{:?} shares its palette with another variant and cannot be told apart",
                variant
            );
            seen.push(tokens);
        }
    }

    #[test]
    fn a_state_never_paints_the_resting_fill() {
        for variant in [
            ButtonVariant::Primary,
            ButtonVariant::Secondary,
            ButtonVariant::Subtle,
            ButtonVariant::Icon,
            ButtonVariant::OnHero,
        ] {
            let tokens = variant.tokens();
            assert_ne!(
                tokens.fill_hover, tokens.fill,
                "{:?} hover fill must differ from the resting fill, otherwise hover gives no feedback",
                variant
            );
            assert_ne!(
                tokens.fill_pressed, tokens.fill_hover,
                "{:?} pressed fill must differ from the hover fill",
                variant
            );
        }
    }

    #[test]
    fn the_harness_drives_a_button_through_the_router() {
        let widget = ButtonWidget {
            node: 42,
            desc: desc(),
        };
        let mut harness = WidgetHarness::new(widget, Vec::new());
        assert_eq!(harness.state().phase, PointerPhase::Idle);

        harness.hover();
        assert_eq!(harness.state().phase, PointerPhase::Hovered);
        assert_eq!(
            fill_token(&harness.output),
            ButtonVariant::Primary.tokens().fill_hover.raw()
        );

        harness.press();
        assert_eq!(harness.state().phase, PointerPhase::Pressed);

        // 指针已被捕获，因此即使移出较远仍保持按下状态。
        harness.hover_at(Vec2::new(0.0, 0.0));
        assert_eq!(harness.state().phase, PointerPhase::Pressed);

        harness.release();
        assert_eq!(harness.state().phase, PointerPhase::Idle);
    }

    #[test]
    fn a_focused_button_emits_an_outer_stroke() {
        let widget = ButtonWidget {
            node: 7,
            desc: desc(),
        };
        let mut harness = WidgetHarness::new(widget, Vec::new());
        harness.focus();
        assert!(harness.state().focused);
        assert!(harness
            .output
            .primitives
            .iter()
            .any(|primitive| primitive.prim_type() == PrimType::Stroke));
    }

    #[test]
    fn left_aligned_button_anchors_label_near_left_edge() {
        let mut glyphs = vec![GlyphQuad {
            rect: Rect::new(0.0, 0.0, 30.0, 14.0),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        }];
        let d = desc().left_align();
        let _ = emit_button(1, &d, ControlState::default(), &mut glyphs);
        assert!(
            (glyphs[0].rect.origin.x - (d.rect.origin.x + 12.0)).abs() < 0.01,
            "left_align button should place label at rect.origin.x + 12.0, got {}",
            glyphs[0].rect.origin.x
        );
    }
}
