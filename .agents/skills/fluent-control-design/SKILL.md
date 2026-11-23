---
name: fluent-control-design
description: >-
  总结并应用 Windows 11 (WinUI 3) Fluent 控件的外观与交互行为规律。当校准 WinUI 3 基准对齐控件、将已验证的设计规律沉淀为规范，或将 Fluent 风格与交互一致性迁移到 Unbound 自有独有组件时使用。
---

# WinUI 3 Fluent 控件设计规律（外观与行为）

> **规范状态**：**终版固化（Finalized）**<br/>
> 全量 133 个目录变体（70 个 WinUI 3 基准对齐控件 + 20 个 Community Toolkit 扩展控件 + 43 个 Unbound 自有独有控件）已完成 Fluent 设计规律同化、多后景材质适配、控件能力边界编译期声明与自动化合规门禁看守。

本技能用于将从 **WinUI 3 基准对齐控件**（经过 `showcase --parity-run` 像素差分校准）与多后景材质体系中提炼出的视觉和行为规律，体系化地应用到所有组件——尤其是 Win11 官方不存在的 **Unbound 自有独有控件**（见 `docs/winui3-control-inventory.md` 第五节）。

## 核心工作流

### 阶段 A：从已对齐的基准控件中提炼规律（总结沉淀）

每完成一批基准对齐控件（如输入类、选择类、浮层类、导航类）的像素级校准，必须向 [REFERENCE.md](./REFERENCE.md) 回填以下六个维度的实测不变量：

1. **薄纱表面阶梯（Elevation & Veil Layering）**：在纯色、标准云母、云母变体与亚克力底板上，`Rest -> Hover -> Pressed -> Disabled` 四态的填充 Token 与线性空间 alpha 换算规律（`veil()` / `light_base_veil()`）。
2. **微描边与高光方向（Elevation Border & Bottom Accent）**：Win11 按钮/下拉框的微高光描边（浅色底边加深、深色顶边提亮）与输入框底部强调线（未聚焦 `1px` 次级线、聚焦 `2px` 强调色底边）的绘制规律。
3. **几何尺寸与圆角阶梯（Geometry & Corner Radius）**：标准控件高度（`32px`）、紧凑高度（`24px/28px`）、内边距非对称基线补偿（如 `11, 5, 11, 6`）、控件内圆角（`4px`）与容器/浮层外圆角（`8px`）。
4. **排版与字阶（Typography & Baseline）**：正文字号（`14px`）、辅助/表头字号（`12px`）、标题字号（`16px/20px`）及与图标并排时的 `8px` 间距。
5. **状态机与指针/键盘交互规律（Interaction & Focus）**：
   - 瞬态由路由器派生（`PointerPhase::Idle / Hovered / Pressed`），严禁落入持久布尔值；
   - 按下时的微缩反馈（如 `ToggleSwitch` 旋钮拉长、`Button` 文本/图标压暗或位移、指示条收缩）；
   - 键盘焦点环（外扩 `3px`、`2px` 宽 `Token::FOCUS_RING`）。
6. **物理弹簧动效（Spring Motion）**：无超调临界阻尼弹簧参数，确保所有开关、滑块、展开面板与独有工作台组件手感如出一辙。

### 阶段 B：将规律应用到 Unbound 自有独有控件（迁移巡检）

在新建或改造任何 **Unbound 自有独有控件**（如 `NodeGraph`、`CurveEditor`、`TimelineTrack`、`CommandPalette`、`Dock`、`PropertyGrid`、`PinInput`、`QueryBuilder` 等）或扩展既有目录控件时，逐项执行以下检查清单：

- [ ] **禁止硬编码 RGB 颜色**：所有图元必须绑定 `unbound_theme::Token`，保证浅色 / 深色 / 高对比度三套主题切换零重排、零实例重传。
- [ ] **同构子部件复用同一规律**：
  - 凡是“可点击触发器/胶囊”（如 `ShortcutBar` 键帽、`AddressBar` 分段、`QueryBuilder` 算子按钮），其四态填充与描边必须与 `Button (Secondary/Subtle)` 规律一致，圆角固定 `4px`。
  - 凡是“可输入/可聚焦槽位”（如 `PinInput` 单元格、`HotkeyInput` 录制框、`PropertyGrid` 数值格），必须遵循 `TextBox` 的 `CONTROL_FILL` 背景与聚焦底部 `2px` `PRIMARY` 强调线规律。
  - 凡是“浮动面板/画布工具栏”（如 `CanvasToolbar`、`CommandPalette`、`TaskHud`、`Minimap`），外框圆角必须为 `8px`，使用 `CARD_BG` / `OVERLAY_BG` 与 `CARD_BORDER`，并挂载标准解析阴影。
  - 凡是“行列表/树表/时间轴轨道”（如 `TreeTable`、`TimelineTrack`、`TerminalView`），行高、悬停薄纱（`Token::HOVER`）与左侧 `3px × 16px` 选中指示条（Selection Pill）必须与 `NavigationView` / `ListView` 对齐。
- [ ] **24 态图元数恒定（Stable Shape）**：同一组件在（Idle / Hovered / Pressed）×（Unfocused / Focused）×（Enabled / Disabled）×（Unselected / Selected）共 24 种控制状态组合下，发射的图元数量必须恒定（仅改变 Token、透明度或几何坐标），确保增量 diff 零抖动。
- [ ] **能力边界维度核对（溢出 / 截断 / 空态 / 键盘 / 贴边翻转，ADR-0031）**：
  - 在 `crates/unbound-widgets/src/canonical.rs` 为该 `WidgetCatalogId` 显式声明 `WidgetCapabilities`（无通配符穷举 `match`）；
  - **溢出策略（`OverflowStrategy`）**：明确裁决 `None`、`OverlayScrollbarVertical`、`OverlayScrollbarBoth`、`TabStripButtons`、`MenuOverflowPopup` 或 `ExternalScrollView`；凡接入按需叠加滚动条（`emit_scrollbar`）的组件，必须满足「未溢出 = 0 滚动条图元与 0 命中区；溢出 = 产出可见滚动条图元与轨道/滑块命中区」并补入运行期参数化契约测试；
  - **文本截断**：声明 `text_elidable` 的标签与行项在窄宽度下须正确省略或裁剪，不得溢出邻接列或容器边界；
  - **空态表现**：空数据/零项场景不 panic、不渲染残影或悬空滚动条；
  - **键盘可达性**：交互控件完整覆盖焦点环、方向键/激活键/Esc 导航契约；
  - **贴边翻转**：声明 `overlay_flippable` 的浮层/菜单/提示框在贴近视口边缘时必须自动翻转或夹紧在安全区内。
- [ ] **动画中间帧几何不变量（Animation Mid-Frame Geometry Invariants）**：凡声明 `animates_geometry: true` 的组件（如 `NavigationView` 折叠展开、`ToggleSwitch` 旋钮、`Expander` 面板等），必须在过渡动画的任意中间采样点保持图标、选中指示条与子元素几何连续、无瞬时跳变、无越界裁剪，并由自动化测试覆盖关键中间帧。

### 阶段 C：自动化合规巡检与终版固化（Finalized）

阶段 B 的清单已全面机械化看守，以下编译期门禁与测试在每次 `just check` / `just test-fast` 时自动执行，确保规律零回退：

| 门禁 / 测试 | 覆盖内容 | 固化状态 |
| --- | --- | --- |
| `canonical::WidgetCapabilities`（编译期穷举） | 全部 133 个 `WidgetCatalogId` 无通配符穷举声明 `OverflowStrategy` 及能力维度，新增控件漏声明直接触发 `E0004` | 已固化看守 |
| `tests/integration/capability_scrollbar_contract.rs` | 自动遍历所有声明 `OverlayScrollbar*` 的目录组件及 `VirtualGrid`，双态断言未溢出零滚动条图元/命中区、溢出产出可见滑块与轨道/滑块命中区，并排除 `TabView`（`TabStripButtons`） | 已固化看守 |
| `tests/integration/navigation_view.rs` | 动画中间帧几何不变量（折叠/展开全程图标与指示条零跳变）及 Left 窗格溢出按需滚动条契约 | 已固化看守 |
| `tests/integration/stable_shape.rs` | 全部 133 个目录组件（含 43 个自有独有控件）在 24 种 `ControlState` 组合下图元数量严格恒定 | 已固化看守 |
| `tests/integration/pilot_geometry.rs` | 试点基准控件与标准输入行的 `32px` 高度、`4px` 圆角、`ToggleSwitch(Header)` `53px` 尺寸 | 已固化看守 |
| `tests/integration/fluent_conformance.rs` | 43 个自有独有控件的分类与计数（43 个 `Exclusive`）、零硬编码颜色构造、生产代码圆角字面量仅允许 `0/4/8` | 已固化看守 |

巡检结果以 `cargo test -p unbound-widgets` 为准；新增控件或修改既有控件时必须让上述编译期门禁与集成测试 100% 通过，并对照 `REFERENCE.md` 第 8 节的规约执行验收。

## 详细数值与映射参考

具体的 Token 映射表、WinUI 3 XAML 资源对照表、几何常数与自有组件映射矩阵见 [REFERENCE.md](./REFERENCE.md)。
