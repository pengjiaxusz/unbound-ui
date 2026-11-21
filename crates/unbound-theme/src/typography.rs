//! WinUI 3 排版标度与几何阶梯。
//!
//! Windows 11 的界面之所以「一眼可辨」，靠的不是某一个具体控件，而是
//! **一整套稳定的尺寸约定**：正文 14px、说明 12px、大标题 28px 半粗、
//! 卡片圆角 4px、导航项 34px 高、图标 16px……这些数字在系统设置、文件
//! 资源管理器、终端里全都一样。
//!
//! 本模块把这套约定收敛成常量，供 `unbound-app`（页面骨架）、
//! `unbound-widgets`（控件几何）与 launcher（页面排版）共同消费：
//! 尺寸只在这里定义一次，任何一处偏离都会立刻在视觉上露馅。
//!
//! 全部数值均为 **100% 缩放下的逻辑像素**，由 `ui_scale` 统一缩放。

/// 字重阶梯。
///
/// WinUI 几乎不使用 700 粗体：强调靠 **半粗（600）** 表达，
/// 700 属于旧式 Win32 观感，是「不像 Win11」的典型来源。
pub mod weight {
    /// 常规字重（正文、说明文字）。
    pub const REGULAR: u16 = 400;
    /// 半粗字重（标题、选中项、强调文本）。
    pub const SEMIBOLD: u16 = 600;
}

/// 字号阶梯。
pub mod font_size {
    /// 页面大标题：Win11 内容页顶部的 28px 半粗标题。
    pub const PAGE_TITLE: f32 = 28.0;
    /// 弹窗 / 抽屉标题。
    pub const TITLE_LARGE: f32 = 20.0;
    /// 次级标题（卡片内的小节标题、抽屉内的分组标题）。
    pub const SUBTITLE: f32 = 16.0;
    /// 卡片分组标题（Win11 设置页里的小节标题）。
    pub const SECTION_TITLE: f32 = 15.0;
    /// 正文与卡片行标题：Win11 界面最常见的字号。
    pub const BODY: f32 = 14.0;
    /// 次级说明文字（卡片行副标题、路径、提示）。
    pub const CAPTION: f32 = 12.0;
    /// 微型标签（徽标、状态栏、表格脚注）。
    pub const MICRO: f32 = 11.0;
}

/// 几何阶梯。
pub mod geometry {
    /// 卡片圆角半径。
    ///
    /// Win11 的分组卡片是 **近乎方角** 的 4px 圆角 + 极细描边，
    /// 靠「同一层云母上叠一层薄纱」制造层次，而不是靠大圆角加阴影——
    /// 后者是 Material / Fluent 旧版观感。
    pub const CARD_RADIUS: f32 = 4.0;
    /// 控件圆角半径（按钮、输入框、下拉框）。
    pub const CONTROL_RADIUS: f32 = 4.0;
    /// 导航项圆角半径（Win11 选中态的圆角矩形）。
    pub const NAV_RADIUS: f32 = 4.0;
    /// 胶囊 / 徽标圆角半径（高度的一半即胶囊）。
    pub const CAPSULE_RADIUS: f32 = 8.0;

    /// 导航项高度。
    pub const NAV_ITEM_HEIGHT: f32 = 34.0;
    /// 导航项矢量图标边长。
    pub const NAV_ICON_SIZE: f32 = 16.0;
    /// 导航项图标与文本之间的留白。
    pub const NAV_ICON_GAP: f32 = 12.0;
    /// 导航窗格左内边距。
    pub const NAV_PADDING_X: f32 = 8.0;
    /// 导航选中指示条宽度。
    pub const NAV_INDICATOR_WIDTH: f32 = 3.0;

    /// Win11 Pivot（页签条）高度。
    ///
    /// Pivot 是二级导航，不是文档标签：纯文字标签 + 选中项下方 2px 强调色下划线，
    /// 高度与 Win11 设置页的页签条一致。
    pub const PIVOT_HEIGHT: f32 = 40.0;
    /// Pivot 标签之间的水平间距。
    pub const PIVOT_GAP: f32 = 24.0;
    /// Pivot 选中指示器厚度。
    pub const PIVOT_INDICATOR_HEIGHT: f32 = 2.0;
    /// Pivot 标签自动测宽时左右各留出的内边距。
    pub const PIVOT_LABEL_PADDING: f32 = 12.0;

    /// 卡片内边距（Win11 设置卡片的标准内边距）。
    pub const CARD_PADDING: f32 = 16.0;
    /// 卡片内行间距。
    pub const CARD_GAP: f32 = 12.0;
    /// 页面内容区外边距。
    pub const PAGE_PADDING: f32 = 32.0;
    /// 内容区与页面外边距之间的间距。
    pub const PAGE_GAP: f32 = 24.0;
    /// 可读表单与设置页主内容列的最大宽度（Win11 设置页实测 1000px 限宽）。
    ///
    /// 在宽视口（如 1440p / 4K / 超宽屏）下将单列卡片与设置行钳制在该宽度内，
    /// 避免左侧标签与右侧控件被过度拉扯；在窄视口下配合 `width: 100%` 自动收缩。
    pub const READABLE_FORM_MAX_WIDTH: f32 = 1000.0;

    /// 开关轨道宽度（Win11 ToggleSwitch 为 40 × 20）。
    pub const TOGGLE_TRACK_W: f32 = 40.0;
    /// 开关轨道高度。
    pub const TOGGLE_TRACK_H: f32 = 20.0;
    /// 开关滑块直径。
    pub const TOGGLE_THUMB: f32 = 12.0;

    /// 按钮高度（标准尺寸）。
    pub const BUTTON_HEIGHT: f32 = 32.0;
    /// 按钮高度（小尺寸）。
    pub const BUTTON_HEIGHT_SM: f32 = 28.0;
    /// 按钮左右内边距。
    pub const BUTTON_PADDING_X: f32 = 12.0;
    /// 输入框高度。
    pub const INPUT_HEIGHT: f32 = 32.0;

    /// Win11 设置行内文字相对**卡片左边缘**的缩进。
    ///
    /// 实测 Win11「个性化 > 颜色」：卡片左边缘 x=450，行标题文字起点 x=507，
    /// 恒定 57px——不论该行有没有图标，文字都对齐在同一条竖线上。
    pub const SETTINGS_ROW_TEXT_INSET: f32 = 57.0;
    /// Win11 设置行前置图标槽的边长。
    pub const SETTINGS_ROW_ICON_SIZE: f32 = 16.0;
    /// 图标与文字之间的间距。
    pub const SETTINGS_ROW_ICON_GAP: f32 = 22.0;
    /// 带图标行的左内边距（= 文字缩进 − 图标 − 图标间距）。
    pub const SETTINGS_ROW_ICON_INSET: f32 =
        SETTINGS_ROW_TEXT_INSET - SETTINGS_ROW_ICON_SIZE - SETTINGS_ROW_ICON_GAP;
    /// Win11 设置颜色色块边长（实测 50px 见方）。
    pub const COLOR_SWATCH_SIZE: f32 = 48.0;
    /// 相邻色块之间的间距。
    pub const COLOR_SWATCH_GAP: f32 = 4.0;
}

/// 标题栏高度。
///
/// Win11 原生应用的标题栏约 48px（含窗口控制按钮的拖拽区），
/// 但那是「带系统标题栏」的形态；无边框窗口需要更紧凑的 40px，
/// 才能让内容区的大标题像 Win11 那样贴近顶部。
pub const TITLE_BAR_HEIGHT: f32 = 40.0;

/// 状态栏高度。
pub const STATUS_BAR_HEIGHT: f32 = 28.0;
