# 架构：widgets
状态：已落地
路径：`crates/unbound-widgets/`
入口：`crates/unbound-widgets/src/lib.rs::WidgetCatalogId`、`SceneCollector`、`diff`、`TooltipDesc`、`place_tooltip_in_window`
守卫：`cargo test -p unbound-widgets --test stable_shape --test health --test overlay_placement`

## 范围
- [负责] 标准与高阶组件库（`define_widget_catalog!` 统一注册 106 个目录组件）、`ControlState`（24 种状态解析）、`TooltipDesc` 窗口边界防溢出自动定位、`WidgetOutput` 场景发射与帧差分补丁
- [不负责] 业务状态持有与异步网络 -> 委托给 `crates/unbound-app/`；底层 Vulkan 渲染 -> 委托给 `crates/unbound-gpu/`

## 拓扑与流程
- 目录生成：`define_widget_catalog!` 宏唯一声明所有组件模块、`WidgetCatalogId`（无 `#[non_exhaustive]`）及 `pub(crate)` 见证令牌 `WidgetShowcaseWitness`
- 发射链路：`*Desc` 扁平 POD 描述符 + `*State` -> `emit_*` 纯函数 -> `WidgetOutput`（`SceneCollector` 图元 + payload + 热区 + `A11yNode`） -> `diff` 局部补丁上传
- Tooltip 窗口内防溢出自动定位：`overlay::place_tooltip_in_window` 与 `tooltip::TooltipDesc` 支持全部 12 种 `Placement` 偏好方向，按“首选方向 -> 主轴对侧翻转 -> 交叉轴正反方向 -> 安全视口内缩边距（`TOOLTIP_VIEWPORT_MARGIN = 6.0px`）平移钳制”的四级策略自动计算气泡矩形与指示箭头锚点（`arrow_tip`），确保靠近窗口四角或边缘触发的 Tooltip 100% 停留在窗口安全区内；同时实现 `UiWidget` trait 并通过 `ViewBuilder::tooltip` 提供声明式挂载
- 三级门禁：`define_widget_catalog!` -> `catalog::emit_widget`（`#[deny(unreachable_patterns)]`） -> `tests/stable_shape.rs`
- payload 索引：`SceneCollector::push` 以 `self.payloads.len()` 为基址重定位每个 widget 的局部 `extra_payload_idx` -> 否则第二段文本会读第一段字形 UV（文字串叠）
- 组件增量 API：`code_block::CodeLanguage`（7 语言关键字表）、`GenericDataTableState::export_csv`（RFC 4180 转义，仅导出当前过滤/排序后的可见行）、`modal::ModalSizePreset`（Sm/Md/Lg/Xl/Full 五档居中矩形，按视口 -32px 钳制）、`schema_form::FieldInheritanceState`（默认值/继承/覆写徽章）-> 均为无状态纯数据/纯函数，不改 24 态 Stable Shape 契约
- 声明式容器与标签 API：`ViewBuilder` 新增 `card`/`panel_card`/`sunken_box`/`row_between`/`row_center`/`row_wrap`/`col_stretch`/`spacer` 与 `text_inline`/`text_primary`/`text_muted`/`text_accent`，由 `unbound-app` 组合 `RawNodeDesc` 与 `BoxSurface`/`TextInline` 发射，不新增目录组件；`ButtonDesc` 增加 `left_align`/`active_selected` 及 `nav_item`/`nav_sub_item`/`tab_item`/`pill_filter`/`icon_btn` 预设，`BadgeDesc` 收紧至 11.5px 字号 / 20px 最小高；`primitive::{instance, surface, outlined, focus_ring, glyphs, curve_stroke, gradient_surface}` 由 `pub(crate)` 提升为 `pub`，供 app 层直接发射图元 -> [守卫] `crates/unbound-widgets/tests/stable_shape.rs`
- 声明式控件契约补齐：`ergonomics` 为 `HotkeyInputDesc`、`ColorPickerDesc`、`CodeBlockDesc`、`NumberInputDesc`、`ComboboxDesc`、`AutoCompleteDesc`、`TabsDesc`、`ChartDesc`、`SplitPaneDesc` 补齐无 `Rect` 构造函数与 `UiWidget` 实现（`initial_state`/`intrinsic_size`/`reduce`/`emit_laid_out`）；`GenericDataTableDesc` 与 `SchemaFormDesc` 补齐单元格/汇总行文本排版及 `FieldInheritanceState` 继承徽标的点击/键盘归约；`unbound-app::prelude` 全量重导出上述 `Desc`/`State` 与 `AppEventSender`、主题预设类型 -> [守卫] `just check`

## 取舍（禁止重构）
- [代价] 组件全部采用扁平 `*Desc` POD + 纯函数 `emit_*`，禁止面向对象组件实例树 -> [原因] 零虚表、零堆分配、天然支持 `diff` 与热重载 -> [位置] `crates/unbound-widgets/src/lib.rs`
- [代价] 同一组件在 24 种 `ControlState`（Normal/Hovered/Pressed/Focused/Disabled × Checked 等）下**图元数量必须严格恒定**（Stable Shape） -> [原因] 状态切换时 GPU 仅做原地局部补丁更新（0 次缓冲重建） -> [位置] `crates/unbound-widgets/tests/stable_shape.rs`
- [代价] `TooltipDesc` 在所有放置方向与状态下的背景图元恒定为 2 个（阴影 + 圆角表面），定位翻转仅修改几何坐标而不增减图元 -> [原因] 严格遵守 24 态 Stable Shape 契约，同时保证永不溢出窗口边界 -> [位置] `crates/unbound-widgets/src/tooltip.rs`

## 不变量与红线
- [禁止] 在 `emit_*` 中根据 `hovered`/`disabled` 条件增减图元个数（隐藏态须用零透明或零尺寸占位） -> [原因] 破坏 Stable Shape 恒定契约 -> [守卫] `crates/unbound-widgets/tests/stable_shape.rs`
- [必须] 声明式浮层容器按 ABI 现有构造器组合：全屏遮罩用 `size(w, h)` + `z_index(i32)`（无 `bounds()`），阴影半径写 `shadow_blur` 字段或走 `shadow(color, blur, dy)`（无 `shadow_blur()` 方法），右对齐用 `Justify::FlexEnd`（无 `End` 变体） -> [原因] 臆造不存在的链式方法或枚举变体会直接 E0599；`ViewBuilder::container` 依赖 `shadow_blur > 0` 判断是否发射表面图元、依赖 `z_index >= 1000` 抬升浮层 Z 基线并命中遮罩 -> [守卫] `just check`
- [必须] 任意锚点位置与任意 `Placement` 下的 `TooltipDesc::rect()` 必须完全落在窗口安全视口内 -> [原因] 防止边缘控件的提示框被窗口边界裁断 -> [守卫] `crates/unbound-widgets/tests/overlay_placement.rs::tooltip_in_window_auto_flips_and_clamps_inside_safe_viewport`
- [必须] 新增组件必须接入 `define_widget_catalog!` 并在 `catalog::emit_widget` 实现真实发射 -> [原因] 编译期穷举门禁防遗漏 -> [守卫] `crates/unbound-widgets/src/catalog.rs`
- [例外] `title_bar` 提供 `TitleBarDesc` 描述符、保留节点 ID 与纯装饰控件 `TitleBarChrome`（实现 `UiWidget`，绘制 `HEADER_BG` 背景与 1px `DIVIDER` 分隔线，无独立 `emit_*`），场景由 `unbound-app` 的 `ViewBuilder` 组合既有容器/徽章/按钮生成，故按基础设施模块列入 `build.rs` 白名单，不纳入展示目录 -> [原因] 纳入穷举目录会迫使伪造发射分支 -> [守卫] `crates/unbound-widgets/build.rs`
- [必须] 默认标题栏以 `Dim::px(viewport.x)` 显式铺满视口，且 `TITLE_BAR_DRAG_ID` 挂在覆盖整条栏的绝对定位背景控件上 -> [原因] 应用根容器为 `AUTO`，百分比宽度会退化成内容宽度，使标题与窗口按钮挤在左上角；栏内标题现为 `TextInline` 行内文本而非徽标，空白区拖拽改由运行器在 `hovered == TITLE_BAR_DRAG_ID` 时调用 `drag_window()`（`FrameHitMetrics::title_bar_height` 恒为 0 以保留栏内控件点击），标题矩形经 `widget_rect` 的布局树回退仍可被测试断言 -> [守卫] `crates/unbound-app/tests/declarative_ergonomics.rs`
- [必须] 追加 `WidgetOutput` 时重定位全部 `extra_payload_idx` -> [原因] 着色器按全局索引访问 payload 数组，不做偏移换算 -> [守卫] `crates/unbound-widgets/src/lib.rs::payload_indices_are_rebased_for_every_widget`
- [新增] `title_bar` 模块提供与 Showcase 1:1 一致的 `WindowCaptionButton`（46px 贴边几何图元：像素对齐最小化横线、双图元最大化/还原框、解析交叉关闭线及 `Token::ERROR` 悬停警示），并由 `unbound-app` 的 `ViewBuilder::title_bar_custom` + `TitleBarSlots`（`left` / `center` / `actions`）与 `AppContext::window_maximized` 为所有窗口应用提供统一且可自定义扩展的默认标题栏 -> [位置] `crates/unbound-widgets/src/title_bar.rs`、`crates/unbound-app/src/lib.rs`
- [新增] `ergonomics` 中全部 16 个声明式适配器与 `WidgetCatalogId` 统一通过 `ViewCtx::shape` 完成全量文本字形排版，并完整闭环指针/键盘状态归约；`catalog::emit_widget_with_shaper` 统一透传 `CatalogParams`（含 `toggled` 与 `click_count`），补齐全部 106 个组件在亮/暗主题与 24 态下的几何与字形渲染 -> [位置] `crates/unbound-widgets/src/{catalog.rs, ergonomics.rs, pin_input.rs}`


## 组件系统大一统与 106 组件全覆盖门禁

* **消除多维度组件包，统一单一组件目录（`WidgetCatalogId::COUNT = 106`）**：
  * 彻底移除原 `cha_kit` 与 `schema_kit` 独立工具包模块及 `build.rs` 中的 `TOOLKIT_MODS` 白名单豁免；
  * **重复能力归并入标准组件**：原磁吸滑块（`SnapSlider`）与双滑块区间（`RangeSlider`）统一收敛进 `slider::SliderDesc`（`with_snap_points` / `with_range_start`）；原只读复制框（`ReadOnlyCopyInput`）统一收敛进 `text_input::TextInputDesc`（`read_only(true).copyable(true)`）；`DraggableModalState` 并入 `modal`、`SmoothWheelHandler` 并入 `scroll`、`ReorderableListState` 并入 `virtual_views`、`ThemeSettingsState` 并入 `color_picker`；
  * **7 款高阶复合组件转正为一等公民**：`address_bar`（`AddressBar`）、`generic_data_table`（`GenericDataTable`）、`pipeline_view`（`PipelineView`）、`query_builder`（`QueryBuilder`）、`schema_form`（`SchemaForm`）、`shortcut_bar`（`ShortcutBar`）、`task_hud`（`TaskHud`）全部纳入 `define_widget_catalog!`、`catalog::emit_widget`、`UiWidget` 声明式挂载、`tests/stable_shape.rs` 24 态探针与 `apps/showcase` 组件展厅。
