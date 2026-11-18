# WinUI 3 Fluent 设计规律权威数值与映射参考

本文件收录从 `microsoft-ui-xaml` 主题资源与本仓库像素差分闭环中验证通过的设计规律，供校准基准控件与设计 Unbound 自有独有控件时直接查表使用。

---

## 1. 几何与尺寸常数阶梯（Geometry & Sizing）

| 维度 | 规范数值（逻辑像素 DIP） | 适用控件与场景 |
| --- | --- | --- |
| **标准交互控件高度** | `32.0 px` | `Button`、`ComboBox`、`TextBox`、`NumberBox`、`AutoSuggestBox`、`SplitButton`、`DatePicker`、`TimePicker`、`PinInput`、`HotkeyInput` |
| **紧凑控件高度** | `24.0 px` ~ `28.0 px` | `InfoBadge`、`Chip`、`BreadcrumbBar` 项、`PropertyGrid` 内嵌行、`StatusBar`（`28px`） |
| **列表/菜单标准行高** | `36.0 px`（菜单项 `32.0 px`） | `ListView`、`NavigationView` 条目、`MenuFlyout` 项、`CommandPalette` 候选行 |
| **容器与输入内边距** | 左右 `11.0 px` ~ `12.0 px`，上 `5.0 px`，下 `6.0 px` | `Button`、`ComboBox`、`TextBox`（上下不对称 `5/6` 用于抵消 Segoe UI / 思源黑体视觉重心偏下） |
| **图标与文字间距** | `8.0 px` | 按钮前置图标、菜单图标、导航图标与文本标签间距 |
| **控件级圆角（ControlCornerRadius）** | `4.0 px` | 所有内嵌交互控件（按钮、输入框、复选框 `4px`、工具栏按钮、代码行高亮、键帽） |
| **容器/浮层圆角（OverlayCornerRadius）** | `8.0 px` | `ContentDialog`、`Flyout`、`MenuFlyout`、`TeachingTip`、`Card`、`NodeGraph` 节点、`CommandPalette` |
| **全圆胶囊圆角（Pill Radius）** | `height * 0.5` | `ToggleSwitch` 轨道（`20px` 高 → `10px` 圆角）、`Slider` 轨道与拇指、`PersonPicture` |

---

## 2. 表面层级与薄纱透明度规律（Surface & Veil Hierarchy）

Win11 Fluent 不使用多档死板的实色灰，而是通过**在后景（云母/亚克力/基准面）上叠加半透明薄纱**构建视觉深度。由於 GPU 颜色附件在**线性空间**混合而 WinUI 在 **sRGB 空间**混合，所有半透明白/黑薄纱必须经 `unbound_theme::win11::veil()` 或 `light_base_veil()` 反解：

| 视觉角色 | WinUI 3 主题资源名 | Unbound `Token` | 浅色模式表现（基准 `#F3F3F3`） | 深色模式表现（基准 `#202020`） |
| --- | --- | --- | --- | --- |
| **窗口基准面** | `ApplicationPageBackgroundThemeBrush` | `Token::SURFACE` | `#F3F3F3`（不透明清屏底） | `#202020`（不透明清屏底） |
| **卡片/面板容器** | `CardBackgroundFillColorDefaultBrush` | `Token::CARD_BG` | 白色 70% 薄纱（高于基准面） | 白色 ~5% 薄纱（`#2A2A2A` 等效） |
| **次级凹陷区/槽位** | `LayerOnMicaBaseAltFillColorSecondary` | `Token::SURFACE_SUNKEN` | 黑色 3.5% 压暗薄纱 | `#1A1A1A` 深凹表面 |
| **标准控件静止填充** | `ControlFillColorDefaultBrush` | `Token::CONTROL_FILL` | 白色 70%（`#FFFFFFB3`） | 白色 6.05% 薄纱（高于卡片一档） |
| **标准控件悬停填充** | `ControlFillColorSecondaryBrush` | `Token::CONTROL_HOVER` / `Token::HOVER` | 白色 50% / 微压暗 | 白色 8.37% 薄纱（提亮一档） |
| **标准控件按下填充** | `ControlFillColorTertiaryBrush` | `Token::PRESSED` | 白色 30%（明显压暗） | 白色 3.26% 薄纱（回落压暗） |
| **标准控件禁用填充** | `ControlFillColorDisabledBrush` | `Token::CONTROL_DISABLED_BG` | 白色 30% | 白色 4.19% 薄纱 |
| **关闭态轨道/未勾选框** | `ControlAltFillColorSecondaryBrush` | `Token::CONTROL_ALT_FILL_SECONDARY` | 黑色 2.4% 薄纱（`#EDEDED`） | 黑色/暗部微压暗（`#1D1D1D`） |
| **强调色主填充** | `AccentFillColorDefaultBrush` | `Token::PRIMARY` | `#005FB8`（前景色 `ON_PRIMARY` 为白） | `#60CDFF`（前景色 `ON_PRIMARY` 为深黑 `#001420`） |
| **强调色悬停/按下** | `AccentFillColorSecondary / Tertiary` | `Token::PRIMARY_HOVER / PRESSED` | 90% / 80% 强调色混叠 | `#82D8FF` / `#4CC2FF` |

---

## 3. 描边、高光与指示器规律（Strokes & Indicators）

1. **普通控件外描边（`ControlStrokeColorDefault` → `Token::CONTROL_STROKE`）**：
   - 线宽固定为 `1.0 px`。
   - 浅色下为 `5.78%` 黑色薄纱（把白色控件从浅灰底上勾勒出来），深色下为 `6.98%` 白色薄纱。
2. **强对比结构描边（`ControlStrongStrokeColorDefault` → `Token::CONTROL_TRACK_STROKE`）**：
   - 用于 `CheckBox` 未选中框、`RadioButton` 外圈、`ToggleSwitch` 关闭态胶囊轨道。
   - 线宽 `1.0 px`，浅色为 `44.58%` 黑，深色为 `54.42%` 白；禁用时切换为 `Token::CONTROL_STRONG_STROKE_DISABLED`。
3. **文本输入类控件的“底边聚焦强调线”规律**：
   - `TextBox`、`NumberBox`、`AutoSuggestBox`、`ComboBox (Editable)` 以及自有的 `PinInput`、`HotkeyInput`、`AddressBar (Editing)`：
   - **未聚焦（Rest/Hover）**：四周 `1px` `CONTROL_STROKE`，底边叠加 `1px` 较深的 `OUTLINE_STRONG`。
   - **聚焦态（Focused）**：背景切换为实白/深黑 `INPUT_BG`，底部绘制 **`2.0 px` 高的 `Token::PRIMARY` 强调色底边**（随圆角内缩），不绘制外扩虚线框。
4. **导航与列表项的“左侧选中胶囊条（Selection Pill）”规律**：
   - `NavigationView`（`Sidebar` / `NavigationRail`）、`ListView`、`SelectorBar` 以及自有的 `Dock` 侧栏、`PropertyGrid` 激活行、`TimelineTrack` 选中轨：
   - 选中项左侧垂直居中绘制 **`3.0 px` 宽、`16.0 px` 高、圆角 `1.5 px` 的 `Token::PRIMARY` 胶囊条**；按下瞬间高度可微缩至 `10.0 px` 以提供弹性反馈。
5. **键盘焦点环（Focus Visual）**：
   - 非文本输入类控件通过 Tab 键聚焦时，在控件边界外扩 `3.0 px` 处绘制 `2.0 px` 宽、圆角 `radius + 2.0` 的 `Token::FOCUS_RING` 外框。

---

## 4. 交互状态机与动效规律（Behavior & Motion）

1. **四态单源派生**：
   - 控件自身的状态结构体只存业务值（如 `checked: bool`、`value: f32`、`selected: usize`）。
   - 视觉瞬态（`Idle` / `Hovered` / `Pressed`）与 `focused`、`disabled` 一律由 `ControlState` 传入，禁止在组件状态内维护独立的 `is_hovered` 字段。
2. **微动效物理特征**：
   - **旋钮/滑块拇指缩放**：`ToggleSwitch` 旋钮静止直径 `12px`，悬停放大至 `14px`，按下沿运动方向拉长为 `17×14px` 胶囊；`Slider` 拇指外环 `20px`，内芯静止 `12px`、悬停 `14px`、按下收缩至 `10px`。
   - **自有组件迁移法则**：`CurveEditor` 的控制点、`Gizmo` 的拖拽手柄、`NodeGraph` 的端口引脚（Pin）以及 `TimelineTrack` 的关键帧菱形，在 `Idle -> Hovered -> Pressed` 时同样遵循 **“静止基准 → 悬停外扩 `+2px` → 按下内缩 `-1px` 或加深填充”** 的触觉反馈规律。

---

## 5. 自有独有控件合规巡检映射表

在开发或重构以下自有独有控件时，必须对照左侧的 WinUI 3 基准原型复用 Token 与几何常数：

| 自有独有控件 | 对标的 WinUI 3 基准原型 | 必须复用的视觉与行为要素 |
| --- | --- | --- |
| `NodeGraph` / `PipelineView` | `Card` + `Button` + `CheckBox` | 节点背景 `CARD_BG` + `8px` 圆角；选中节点外描边 `PRIMARY`；端口悬停放大反馈同 `Slider` 拇指 |
| `CurveEditor` / `GradientEditor` | `Slider` + `Canvas` | 槽位底色 `SURFACE_SUNKEN`；网格线 `GRID`；控制柄与色标滑块完全复用 `Slider` 拇指的三态内外环缩放规律 |
| `TimelineTrack` / `Timeline` | `ListView` + `Slider` | 轨道行交替/悬停薄纱同 `ListView`；播放头（Playhead）使用 `PRIMARY` 竖线 + 顶部 `4px` 圆角五边形头 |
| `CommandPalette` | `Flyout` + `AutoSuggestBox` + `ListView` | 外框 `8px` 圆角 + `OVERLAY_BG` + 解析阴影；顶部搜索栏同 `AutoSuggestBox`；结果行高亮同 `ListView` |
| `CodeBlock` / `CodeEditor` / `TerminalView` | `ScrollView` + `TextBox` | 容器 `CODE_BG` + `CARD_BORDER` + `8px` 圆角；右上角复制按钮同 `Button (Subtle)`；行号槽使用 `TEXT_SECONDARY` |
| `PinInput` / `HotkeyInput` / `AddressBar` | `TextBox` + `BreadcrumbBar` | 高度严格 `32px`；圆角 `4px`；激活态底部绘制 `2px` `PRIMARY` 强调线 |
| `PropertyGrid` / `SchemaForm` / `QueryBuilder` | `Expander` + `TextBox` + `ComboBox` | 分组折叠头严格对齐 `Expander`；字段行高 `32px~36px`；内嵌输入控件与独立 `TextBox`/`ComboBox` 零差异 |
| `Dock` / `MultiSplit` | `TabView` + `GridSplitter` | 标签栏激活态圆角与关闭按钮同 `TabView`；分割条悬停显示 `PRIMARY` 指示条；拖拽落点预览框使用 `PRIMARY_SOFT` 填充 + `PRIMARY` 描边 |

---

## 6. 首批 6 个试点基准控件实测锁定值（Pilot Calibration）

以下数值由「控件 × 状态 × 明暗主题 × `solid` 后景」试点批次锁定，并由
`crates/unbound-widgets/tests/pilot_geometry.rs` 以几何断言持续看守：

| 控件 | 固有高度 | 圆角 | 关键边距与描边 | 状态差异 |
| --- | --- | --- | --- | --- |
| `Button` | `32.0 px` | `4.0 px` | 内边距 `11, 5, 11, 6`；1px `CONTROL_STROKE`；无外部投影 | 悬停/按下仅改变填充 Token，图元数恒定 |
| `CheckBox` | `32.0 px` | 方框 `4.0 px` | `MinWidth = 120 px`；方框列 `20 px` + 文字间距 `8 px` | 勾选仅改变勾形与填充，不增删图元 |
| `ToggleSwitch (Header)` | `53.0 px` | 轨道胶囊 `10.0 px` | 标签行 `20 px` + 间距 `13 px` + 轨道 `20 px`；轨道 `40 × 20`；旋钮 `12 px`、内缩 `4 px` | `Idle → Hovered` 旋钮 `12 → 14 px`，按下沿运动方向拉长 `3 px` |
| `Slider` | `32.0 px` | 轨道/拇指全圆 | 轨道高 `4 px`；拇指外环 `20 px`、内芯 `12 px` | 悬停内芯 `14 px`、按下回缩 `10 px` |
| `ComboBox` | `32.0 px` | `4.0 px` | 右侧 Chevron 中心距右沿 `20 px`；弹出列表行高后续批次单独校准 | 展开仅改变 Chevron 方向与焦点底边线 |
| `TextBox` | `32.0 px` | `4.0 px` | 聚焦态底部 `2 px` `Token::PRIMARY` 强调线；未聚焦底边 `1 px` `OUTLINE_STRONG` | 聚焦线随圆角内缩，不新增图元 |

**校准记录**：

1. `TextBox` 的固有高度由旧实现的 `34 px` 收敛到 `32 px`，与 `ComboBox`、`Button` 共行。
2. `Slider` 的固有高度由旧实现的 `28 px` 收敛到 `32 px`，避免被外部矩形掩盖。
3. 参考截图对 `Button` 显式使用 `4 px` 圆角、`14 px` 字号与零投影，绕过展台预览构造器的 `6 px` 网页风格圆角。
4. `ToggleSwitch` 的 Header 排版高度为 `53 px`，并非 `32 px` 单行——差分器必须按 `explicitWidth/Height` 判定其固有尺寸，而不能套用标准输入行高。

---

## 7. 容器、浮层与导航胶囊实测规律（Containers, Overlays & Selection Pills）

本节收录基于 #46 及 WinUI 3 像素差分闭环实测校准的容器类、浮层对话框类以及导航指示条设计规律：

### 1. 容器类（Card / Border）
* **圆角阶梯**：统一收敛至 `8.0 px`（`OverlayCornerRadius = 8`），与内嵌输入/按钮控件的 `4.0 px`（`ControlCornerRadius`）形成 2:1 层次嵌套比，杜绝内角顶死外框。
* **微描边规范**：统一为 `1.0 px` `CardStrokeColorDefaultBrush`（映射至 `Token::CARD_BORDER`，浅色为黑色 5.9% 薄纱，深色为白色 5.1% 薄纱），禁止使用实色灰（如 `Token::OUTLINE`）以免在亚克力/云母背板上变脏。
* **背景填充**：`Token::CARD_BG`（浅色为白色 70% 薄纱，深色为白色 ~5% 薄纱）。
* **海拔阶梯**：`CARD_ELEVATIONS = [0.0, 2.0, 6.0, 12.0, 20.0]`，静止卡片默认无阴影或 `2.0 px` 微阴影。

### 2. 浮层与对话框类（Flyout, MenuFlyout, TeachingTip, ContentDialog）
* **外圆角**：浮层与弹窗标准统一为 `8.0 px`（`OverlayCornerRadius = 8`）。
* **微描边**：统一采用 `1.0 px` `SurfaceStrokeColorFlyout`（`Token::CARD_BORDER`），与背景自然溶合。
* **表面底色**：统一使用 `Token::OVERLAY_BG`（`SolidBackgroundFillColorQuarternary`，浅色 `#F9F9F9`，深色 `#2C2C2C`）。
* **解析阴影阶梯（Elevation Shadow Hierarchy）**：
  * `MenuFlyout`：`18.0 px` 模糊半径（菜单层级阴影）；
  * `Flyout`（Popover / Callout）：`24.0 px` 模糊半径（标准锚定浮层阴影）；
  * `TeachingTip`（气泡引导）：`24.0 px` 模糊半径（浮层阴影，自带指示尖角）；
  * `ContentDialog`（Modal 模态对话框）：`40.0 px` 模糊半径（高海拔窗口级阴影，全视口覆盖 `Token::SCRIM` 遮罩）。
* **渲染层级管线**：解析阴影必须标记为 `SubElement::Shadow`（或 `PrimType::Shadow`）先于背景表面输出，确保在硬件片元着色器中执行正确的解析高斯盒模糊合成，避免深色阴影渗入抗锯齿边。

### 3. 导航与选择条选中胶囊条（Selection Pill Indicator）
* **适用控件**：`NavigationView`（侧边栏项）、`ListView`（列表行）、`SelectorBar`（水平选择条）、`Dock`（活动面板标签）。
* **几何规范**：固定为 **宽 `3.0 px`、高 `16.0 px`、全圆角 `1.5 px`**。
* **对齐与布局**：沿选中项边缘居中对齐（纵向列表居左内缩 `3.0 px`，横向选择条沿底边居中），按下反馈时高度可微缩至 `10.0 px`。
* **色彩映射**：激活态恒为 `Token::PRIMARY` 强调色；禁用态换为 `Token::TEXT_DISABLED`。

---

## 8. 多后景材质（云母/亚克力）叠色规律与全量 43 个自有独有控件终版巡检

本节收录基于 #50（多后景材质像素差分矩阵）与 #51（自有独有控件终版巡检与同化）实测锁定的终版规约：

### 8.1 多后景材质合成底板与半透明薄纱叠色规律

Windows 11 Fluent 体系的核心特征是窗口后景材质的多样性。控件在不同后景材质上必须保持层级清晰、边缘清晰、文字对比度达标，且不出现死灰或色污。

#### 1. 四种合成底板规格与基准色

| 合成底板 | 材质类型 | 浅色基准色 (sRGB) | 深色基准色 (sRGB) | 最低系统门槛 | 特性与适用场景 |
| --- | --- | --- | --- | --- | --- |
| **`solid`** | 纯色无材质 | `[243, 243, 243]` (`#F3F3F3`) | `[32, 32, 32]` (`#202020`) | 无门槛（Win10/Linux/macOS 默认回退） | 不透明窗口表面，基础对照基准 |
| **`mica`** | 标准云母 | `[238, 242, 249]` (`#EEF2F9`) | `[29, 32, 39]` (`#1D2027`) | Windows 11 Build 22621 (22H2) | 采样桌面壁纸经大半径高斯模糊后注入窗口，呈现柔和的主题色调 |
| **`mica_alt`** | 云母变体 | `[229, 235, 245]` (`#E5EBF5`) | `[22, 25, 32]` (`#161920`) | Windows 11 Build 22621 (22H2) | 专用于标头、侧边栏或选项卡等层叠结构，较标准云母色调更深、对比更强 |
| **`acrylic`** | 亚克力 | `[234, 240, 250]` (`#EAF0FA`) | `[36, 40, 48]` (`#242830`) | Windows 11 Build 22621 (22H2) | 双向实时模糊 + 噪点纹理，常用于暂时性浮层、弹出菜单与上下文界面 |

> **实验档补充**：`immersive`（沉浸式后景，最低系统门槛 26100，系统通道取值未公开）与 `acrylic_tint`（自定义色调亚克力，最低系统门槛 17134，走 `SetWindowCompositionAttribute` 的 accent policy 导出）。

#### 2. 半透明薄纱（Veil）在线性空间中的混合适配规律

* **物理空间换算原理**：
  - GPU 片元着色器管线在 **Linear sRGB 空间** 进行硬件 Alpha Blending，而 WinUI 3 XAML / Windows App SDK 资源中的 `#AARRGGBB` 颜色定义是在 **非线性 sRGB 空间** 混合。
  - 若直接将 sRGB 下的 Alpha 值传递给 GPU 混合，会导致浅色薄纱在深后景上变暗变脏、深色薄纱在浅后景上产生死灰光晕。
  - 必须统一通过 `unbound_theme::win11::veil()` 与 `light_base_veil()` 函数反解线性 Alpha 混合因子，保证在 `solid`、`mica`、`mica_alt`、`acrylic` 任意底板上，透过半透明薄纱后呈现的感知相对明度与官方 WinUI 3 像素差分完全重合。
* **分层叠色不变量**：
  - **`CARD_BG`（卡片表面）**：浅色模式下为白色 70% 薄纱，深色模式下为白色 ~5% 薄纱（`#2A2A2A` 等效）。在 `mica` / `mica_alt` 上，后景壁纸微弱透出提供环境连续性，但在 `solid` 上表现为柔和的卡片提亮。
  - **`CONTROL_FILL`（交互控件静止表面）**：浅色下 70% 白薄纱，深色下 6.05% 白薄纱（略高于卡片一档）。悬停态（`HOVER`）提亮（浅色 50% 压暗/深色 8.37% 提亮），按下态（`PRESSED`）压暗（浅色 30%/深色 3.26%）。
  - **`CARD_BORDER` / `CONTROL_STROKE`（微高光描边）**：浅色模式使用 5.78%~5.9% 黑色薄纱（`#0000000F`），深色模式使用 5.1%~6.98% 白色薄纱（`#FFFFFF12`）。禁止任何实色灰色硬编码描边，防止在亚克力噪点或云母彩色底板上显出突兀硬框。
  - **`PRIMARY` / `FOCUS_RING`（强调色与焦点）**：实色强调色（`#005FB8` / `#60CDFF`）恒为不透明，底边聚焦强调线固定 `2.0 px`，即使在极端彩色壁纸穿透下也能确保清晰的交互暗示。

#### 3. 自绘变体覆盖边界（SelfDrawnCoverage）

两轴模型（`MaterialKind` × `BackdropSource`）中，自绘路径（`BackdropSource::SelfDrawn`）为无 GPU 原生后景环境提供确定性外观。自绘变体的覆盖边界严格遵循物理渲染能力划分：

| 材质种类 | 自绘覆盖状态 (`SelfDrawnCoverage`) | 依据与渲染配方 | 显式请求自绘时的回退逻辑 |
| --- | --- | --- | --- |
| **`Mica`** | `Equivalent` (等价重现) | 壁纸大半径模糊 + 主题色调配方。磨砂 sigma 锚定屏幕物理像素常量，无头纹理统计锁住空间梯度能量与明度带宽 | 无回退，正常走自绘管线 |
| **`MicaAlt`** | `Equivalent` (等价重现) | 在壁纸模糊基础上叠加加深对比的配方，精准复现次级标头/侧栏层叠质感 | 无回退，正常走自绘管线 |
| **`Acrylic`** | `NoVariant` (无自绘变体) | 亚克力的本质特征是窗口后景实时双向模糊与动态噪点（Noise）纹理。静态离线壁纸采样无法重现动态穿透，强行自绘会造成误导性伪差分 | 判定为非法组合，自动回退（标明 `FallbackReason::NoSelfDrawnVariant`） |
| **`AcrylicTint`** | `NoVariant` (无自绘变体) | 同亚克力，依赖实时动态模糊与 accent policy 混合，无离线自绘变体 | 判定为非法组合，自动回退 |
| **`Immersive`** | `NoVariant` (无自绘变体) | 实验档，系统取值未确认，不存在可供对齐的原生基准 | 判定为非法组合，自动回退 |
| **`FollowSystem`** | `NoVariant` (无自绘变体) | 动态策略档，自绘引擎无法预判系统合成器行为 | 判定为非法组合，自动回退 |

---

### 8.2 全量 43 个 Unbound 自有独有控件终版巡检验收与同化方法

针对面向专业创作工具、IDE、节点编辑器、时间轴与数据工作台场景的全部 **43 个自有独有控件**（`ControlClass::Exclusive`），全面实施 Fluent 规律同化改造与自动化门禁固化，达成与 Win11 原生组件的无缝融合。

#### 1. 核心巡检验收指标与达标结论

经过 `tests/fluent_conformance.rs`、`tests/stable_shape.rs` 与 `tests/pilot_geometry.rs` 自动化看守，43 个独有控件全部达到以下终版指标：

| 巡检维度 | 严格规范要求 | 43 个自有独有控件验收结论 | 看守测试 |
| --- | --- | --- | --- |
| **零硬编码颜色** | 生产源码 100% 杜绝 `Color::rgb` / `Color::rgba` / `Color::from_rgba`，所有颜色必须绑定 `unbound_theme::Token` | **全部 43 个独有控件 100% 合规**。主题在浅色、深色与高对比度间切换时，几何拓扑零重排、GPU 实例缓冲零重传 | `fluent_conformance::exclusive_controls_never_hardcode_colors_or_off_ladder_radii` |
| **8px / 4px 圆角阶梯** | 严格遵循容器 `8.0 px`、控件 `4.0 px`、胶囊 `height * 0.5` 的阶梯。严禁 `3px / 6px / 10px / 12px` 等非标圆角字面量 | **100% 收敛至 0 / 4 / 8 阶梯**。外部容器卡片（`NodeGraph` 卡片、`Dock`、`CommandPalette`）均为 8px；内部触发器、槽位、键帽均为 4px | `fluent_conformance::exclusive_controls_never_hardcode_colors_or_off_ladder_radii` |
| **32 / 36 / 40px 行高阶梯** | 交互触发器与输入行固定 `32px`，紧凑数据/属性列表行固定 `36px`，主导航/工具条固定 `40px` | **100% 符合 Fluent 垂直韵律**。`PinInput`、`HotkeyInput`、`AddressBar` 均为 32px；`PropertyGrid`、`TreeTable` 均为 36px 行高 | `pilot_geometry.rs` 与布局回归测试 |
| **左侧 3×16px 胶囊条** | 凡具备选中/激活态的纵向列表项、轨道或标签页，必须呈现标准 `Token::PRIMARY` 胶囊条 | **全量对齐**。`Dock` 活动标签、`PropertyGrid` 激活行、`TimelineTrack` 选中轨、`TreeTable` 选中行均统一样式与弹簧反馈 | 视觉检查与组件渲染器断言 |
| **24 态稳定形状** | 在（Idle/Hovered/Pressed）×（Unfocused/Focused）×（Enabled/Disabled）×（Unselected/Selected）全状态下，发射图元数量恒定 | **全部 43 个独有控件图元数 100% 恒定**。状态流转仅改变 Token 槽位、Alpha 或几何坐标，增量 Diff 零增删图元 | `stable_shape.rs::all_catalog_widgets_have_constant_primitive_count_across_all_24_control_states` |

#### 2. 自有独有控件系统同化方法论（Assimilation Methodology）

将复杂专业组件分解为“基准原生规律的组合体”，使得独有控件在具备高密度专业功能的同时，手感与视觉与系统原生控件完全一致：

1. **复合卡片与画布图元同化（NodeGraph / PipelineView / Minimap / Gizmo）**：
   - 节点外框：采用 `Token::CARD_BG` + `8.0 px` 圆角 + `Token::CARD_BORDER` 微描边；
   - 节点选中：复用 `Token::PRIMARY` 描边与 `Token::PRIMARY_SOFT` 半透明选择光晕；
   - 端口（Pin）手感：端口引脚半径静止 `4px`、悬停外扩至 `5px`、连接中按下收缩至 `3.5px`，严格复用 `Slider` 拇指微动效规律。
2. **代码、终端与停靠面板同化（CodeBlock / CodeEditor / TerminalView / Dock）**：
   - 面板外壳：统一使用 `Token::SURFACE_SUNKEN` / `CODE_BG` 凹陷底板 + `8.0 px` 外圆角；
   - 停靠标签头（Dock Tab）：高度收敛至 `36.0 px`，激活态标签复用 `TabView` 顶层平底圆角（顶部 `4.0 px`、底部直角）及关闭按钮悬停微高光；
   - 分割拖拽条（MultiSplit）：拖拽条静止透明，悬停时高亮 `Token::PRIMARY` 细线，拖拽落点指示框使用 `PRIMARY_SOFT` 浅色薄纱。
3. **专业数据编辑与属性检视同化（PropertyGrid / SchemaForm / QueryBuilder / TreeTable）**：
   - 分组折叠头：对齐 `Expander`，左侧展开 Chevron 图标，右侧辅助标签字号 `12.0 px`（`TEXT_SECONDARY`）；
   - 键值编辑单元：属性行高固定 `36.0 px`，内嵌的数值输入、颜色选择、下拉枚举零二次封装，直接内联 `NumberBox`、`ColorPicker`、`ComboBox` 基准原型；
   - 选中行指示：激活行背景铺 `Token::CONTROL_FILL` 薄纱，左侧边界内缩 `3.0 px` 绘制 `3.0 × 16.0 px` `PRIMARY` 胶囊条。
4. **扩展输入与导航同化（PinInput / HotkeyInput / AddressBar / ShortcutBar）**：
   - 单元格外形：单格输入槽位固定高度 `32.0 px`、圆角 `4.0 px`、背景 `Token::CONTROL_FILL`；
   - 聚焦反馈：聚焦单元格四周不出现外扩实线框，而是在槽位底边呈现 **`2.0 px` 强调色 `Token::PRIMARY` 底边线**（与 `TextBox` 聚焦底边完全一致）；
   - 键帽提示：`ShortcutBar` 键帽复用 `Button (Subtle)` 轮廓，背景 `Token::SURFACE_SUNKEN`，文字居中加粗 `12.0 px`。
