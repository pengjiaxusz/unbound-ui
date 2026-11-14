//! 窗口化的 Unbound UI 展厅（Gallery）。
//!
//! 库部分暴露应用外壳、页面目录与场景绘制器，以便在测试中无头驱动展厅；
//! 二进制部分负责打开窗口并显示。
//!

/// 首个交互式控件：扁平的纯函数按钮。
pub mod button;
/// 画布：图元、裁剪、命中区域与语义。
pub mod canvas;
/// 帧计划：呈现尺寸与上传计划。
pub mod frame_plan;
/// 底层场景绘制器与调色板。
pub mod gallery;
/// 窗口与无头测试共用的键盘动作。
pub mod keys;
/// 外壳的布局树：`LayoutEngine` 求解的节点树。
pub mod layout_tree;
/// 页面目录与实时示例渲染器。
pub mod pages;
/// 适配到通用应用运行时的外壳。
pub mod runtime_app;
/// 自举（dogfooded）的应用外壳及其状态机。
pub mod shell;
/// 导航、旋钮（knob）与演示状态。
pub mod state;
/// 稳定的控件标识。
pub mod target;
/// 按钮的无头测试装置（test harness）。
pub mod testing;
/// showcase 的品牌调色板，打包进引擎的颜色查找表（LUT）。
pub mod theme;
/// 可复用的展厅控件（按钮、开关、滑块、胶囊标签）。
pub mod widgets;
