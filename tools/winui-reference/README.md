# WinUI 3 参考截图宿主

为 unbound-ui 的像素级一致性校验提供**官方 WinUI 3 控件参考截图**。本目录是一个独立的 .NET 8 / WinUI 3 非打包应用，不属于 Rust workspace，`just build` 不会处理它，也不需要改动仓库任何现有文件。

## 核心特性（动态拼版页 + 2D 流式装箱 + 紧凑出血 + 指纹增量前置过滤）

1. **瞬态采集载体与切片独立产物解耦（ADR 0015）**：将多控件动态拼版页（Dynamic Parity Page）界定为运行时的瞬态采集载体，排版坐标漂移完全不影响切片产物的单图像素与单用例指纹，下游 Rust 差分器与四重门禁零改动 100% 向后兼容。
2. **多控件 2D 流式装箱与视口溢出安全分页（Dynamic Parity Page & Viewport Paging）**：在固定的确定性标准视口（`1920×1080 DIP`）内，按槽位期望高度降序（FFDH）紧凑流式装箱排布（自适应槽位测量、超宽自动换行）；当槽位行高度累加超过 1080 DIP 安全上限时自动截断当前页并顺延开启下一页。在同一个 Window 实例内主线程安全清空与挂载，无窗口重建与屏幕闪烁，单页仅需一次 `350ms` 动效收敛等待与单次 `RenderTargetBitmap` 导出，随后根据每个槽位的相对视觉几何边界（`TransformToVisual(Host)`）精准切出紧凑单图。
3. **用例指纹增量前置过滤（Fingerprint Pre-Filtering）**：每个用例 JSON 记录绑定了 `(WindowsAppSDKVersion, OSVersion, RasterizationScale, Control, State, Theme, Backdrop, SpecSignature, SourceHash)` 的 16 位 SHA-256 指纹。送入装箱规划器前执行前置比对：
   - 只有当控件内存在至少一个未命中状态或该控件被用户显式指定（或指定了 `--no-cache`）时，才将其送入装箱规划器；
   - 已全部命中缓存的控件完全不进入装箱规划器，不分配版面槽位，不参与可视树测量；
   - 若某主题下的所有控件均已命中缓存，系统直接打印 `[cache-hit]` 并跳过当前主题所有拼版页的渲染与等待，达到毫秒级退出。
4. **多控件联合指定**：`--control` 支持传入逗号分隔的多个控件名（如 `--control Button,CheckBox`），系统自动将所有指定控件的支持状态集中排入同一瞬态画布一次性导出。
5. **确定性后景合成底板（Synthetic Backdrop）**：通过 `--backdrop solid|mica|mica_alt|acrylic|all` 控制画布底板。默认激活 `solid` 纯色基准面，并为后续云母/亚克力全面完工预留确定性合成底板。

## 环境要求

- .NET 8 SDK（`dotnet --list-sdks` 能看到 8.x）
- Windows 10 1809+（本机需能显示 WinUI 3 窗口）
- Windows App SDK 运行时：`just parity capture` 以 `WindowsAppSDKSelfContained=true` 构建，运行机无需预装 Windows App SDK 1.8 运行时。

## 构建与运行

在仓库根目录执行。只有一个入口：`just parity <动词>`，它会自己完成构建（`_parity-build`，经 bque 全局排队）再运行宿主，
不要裸跑 `dotnet build` 或宿主 exe。

```powershell
# 人工翻页检视：打开窗口逐页看，不采图、不退出（推荐日常用这条）
just parity review

# 只检视某个控件（会跳到包含它的第一页，其余页仍可翻）
just parity review --control NavigationView --theme all

# 增量采集全量参考图（命中指纹缓存的用例毫秒级跳过）
just parity capture

# 强制重采并额外保留拼版总览图
just parity capture --refresh --keep-sheets

# 采集单个控件、单个状态
just parity capture --control Button --state normal --theme light
```

### 人工翻页检视（`--review`）

参考宿主原本只会「逐页 settle → 截图 → 退出」，`--review` 把它变成一张可以停留的检视台：

- 分页与自动批采完全一致：同一份 FFDH 装箱结果、同一张 1920×1080 DIP 画布、同一棵可视树。
- 检视条位于画布**之外**，因此永远不会进入 `RenderTargetBitmap` 的像素。
- 「截图本页」复用批采的切片路径（`page_{index}_{theme}_{backdrop}_sheet.png` + 坐标索引 JSON），
  导出前会把窗口钉回 1:1，导出后恢复窗口尺寸。
- 整页命中指纹缓存时页码旁显示「已缓存」，说明 `just parity diff` 会直接复用这一页。

| 交互 | 作用 |
| --- | --- |
| `←` / `→`、`PageUp` / `PageDown` | 上一页 / 下一页 |
| `Home` / `End` | 第一页 / 最后一页 |
| `Ctrl+S` | 导出当前页 |
| `Esc` | 退出 |
| 「跳到控件」下拉 | 跳到包含该控件的第一页 |
| 「适应窗口」/「实际大小」 | 整页缩放显示 / 1:1 像素级查看 |

## 与 unbound-ui 的对照闭环

参考图采集完成后，回到仓库根目录跑差分闭环（编译与离屏差分都经 bque 排队）：

```powershell
just parity
```

该命令在**全局复用同一个 Vulkan 渲染器实例**与**源码哈希缓存**的前提下，对每张参考图调用 `unbound-widgets` 的对应控件，并在**有效对比区（ROI：`union(reference_bbox, ours_bbox, controlBounds)` 外扩 2px）**内计算 CIEDE2000 与四重门禁，输出：

- `target/parity/reference/<用例>.png`：参考截图副本；
- `target/parity/ours/<用例>.png`：本框架渲染结果（附含源码指纹与固有尺寸的同名 JSON）；
- `target/parity/heat/<用例>.png`：ΔE 热力图（黑→蓝→绿→黄→红，30 封顶，并高亮 ROI 框）；
- `target/parity/report.json`：逐用例 ROI 指标、四重门禁与缓存命中状态。

**四重门禁判定阈值（ADR 0008）**：
1. 内容包围盒逐边相差 `≤ 1px`；
2. 自由维度上的控件固有尺寸（`intrinsic`）与参考尺寸相差 `≤ 1px`；
3. 有效对比区（ROI）内 `mean ΔE2000 ≤ 1.5` 且 `p95 ΔE2000 ≤ 4.0`；
4. ROI 内显著色差像素（`ΔE2000 > 5.0`）占比 `overLimitRatio ≤ 2.0%`。

## 参数

| 参数 | 取值 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--out <dir>` | 任意目录 | `captures` | 输出目录，不存在会自动创建；`just` 配方传绝对路径，与调用者工作目录无关 |
| `--control <names>` | 见下方「已接入控件」 | 全部 | 大小写不敏感；支持逗号分隔多个控件（如 `Button,CheckBox`） |
| `--state <state>` | `normal` `disabled` `checked` `focused` | 全部已实现态 | `checked` 仅支持 CheckBox / ToggleSwitch / ToggleButton / RadioButton 等 |
| `--theme <theme>` | `light` `dark` `all` | `light` | 通过 `Host.RequestedTheme` 切换；`all` 无需额外开关 |
| `--backdrop <kind>` | `solid` `mica` `mica_alt` `acrylic` `all` | `solid` | 确定性合成底板类型；`all` 无需额外开关 |
| `--review` | 开关 | 关闭 | 人工翻页检视：装载全量页面、不采图、不退出，见上文快捷键表 |
| `--refresh` | 开关 | 关闭 | 忽略 `fingerprint` 缓存，强制重新采集（旧名 `--no-cache` 仍可用但会提示废弃） |
| `--keep-sheets` | 开关 | 关闭 | 额外按页码输出 `page_{pageIndex:D2}_{theme}_{backdrop}_sheet.png` 拼版总览图及坐标索引 JSON（旧名 `--save-sheet`） |

> **画布尺寸与坐标确定性**：
> 在动态拼版架构（ADR 0015）下，瞬态排版载体固定为 1920×1080 DIP 的确定性画布进行 FFDH 流式装箱，杜绝窗口拉伸导致的子像素抗锯齿抖动与切片边缘截断。因此**不再提供 `--width` / `--height`**：画布尺寸是契约的一部分，能改它就能让历史参考图不可比。检视模式只是把显示层缩放到屏幕内，导出前仍会钉回 1:1。
>
> **已移除的旧参数**：`--batch`（现在采全部控件是默认行为，用 `--control` 收窄）、`--width` / `--height`（见上）。传入会得到明确的报错而不是被静默忽略。

### 已接入控件（64 个）

- **首批试点**：`Button` `CheckBox` `ToggleSwitch` `Slider` `ComboBox` `TextBox`
- **第一批（基础输入与文本）**：`ToggleButton` `SplitButton` `RadioButton` `RatingControl` `ColorPicker` `NumberBox` `AutoSuggestBox` `RichTextBlock` `ProgressBar` `ProgressRing` `InfoBadge` `ToolTip`
- **第二批（导航、浮层、集合与日期）**：`InfoBar` `TeachingTip` `ContentDialog` `Flyout` `MenuFlyout` `MenuBar` `CommandBar` `NavigationView` `TabView` `Pivot` `BreadcrumbBar` `SelectorBar` `PipsPager` `Expander` `SplitView` `Border` `Card` `StackPanel` `ScrollView` `TreeView` `ListView` `FlipView` `CalendarView` `TimePicker` `PersonPicture` `Image` `FontIcon` `TitleBar`
- **第三批（后续补齐的 WinUI 3 控件）**：`RichEditBox` `GridView` `ListBox` `CommandBarFlyout` `AppBarButton` `AppBarToggleButton` `AppBarSeparator` `SwipeControl` `RefreshContainer` `AnnotatedScrollBar` `ItemsView`
- **第四批（按钮与切换类补充）**：`HyperlinkButton` `RepeatButton` `DropDownButton` `ToggleSplitButton`
- **第五批（输入与日期类补充）**：`PasswordBox` `DatePicker` `CalendarDatePicker`

> 说明：`Flyout` 与 `MenuFlyout` 本身不是 `FrameworkElement`，静态拼版以其呈现器（`FlyoutPresenter` / `MenuFlyoutPresenter`）为表面；`ToolTip`、`TeachingTip`、`ContentDialog` 需要运行时宿主才能展开，静态采集只测量其内容盒。这三类浮层后续由结构快照与人工屏幕捕获补足。
