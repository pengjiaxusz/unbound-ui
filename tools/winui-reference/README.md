# WinUI 3 参考截图宿主

为 unbound-ui 的像素级一致性校验提供**官方 WinUI 3 控件参考截图**。本目录是一个独立的 .NET 8 / WinUI 3 非打包应用，不属于 Rust workspace，`just build` 不会处理它，也不需要改动仓库任何现有文件。

## 核心特性（v2 拼版 + 紧凑出血 + 指纹缓存）

1. **状态拼版采集（Contact Sheet Batching）**：在 `--batch` 模式下，按「控件 × 主题 × 后景」为单位将同一控件支持的所有状态（如 `normal` / `disabled` / `checked`）并排放入同一张水平网格画布，共享一次 `350ms` 动画收敛与单次 `RenderTargetBitmap` 导出，随后自动按每个控件的 `controlBounds` + `16 DIP` 出血边距切分为紧凑单图。
2. **版本与规格指纹缓存（Fingerprint Cache）**：每个用例 JSON 记录绑定了 `(WindowsAppSDKVersion, OSVersion, RasterizationScale, Control, State, Theme, Backdrop, SpecSignature)` 的 16 位 SHA-256 指纹。下次运行时若指纹一致且 PNG 存在，直接毫秒级跳过该拼版（可用 `--no-cache` 强制重截）。
3. **确定性后景合成底板（Synthetic Backdrop）**：通过 `--backdrop solid|mica|mica_alt|acrylic|all` 控制画布底板。默认激活 `solid` 纯色基准面，并为后续云母/亚克力全面完工预留确定性合成底板。

## 环境要求

- .NET 8 SDK（`dotnet --list-sdks` 能看到 8.x）
- Windows 10 1809+（本机需能显示 WinUI 3 窗口）
- Windows App SDK 运行时：本工程按 `WindowsAppSDKSelfContained=false` 构建，运行机需要安装 Microsoft Windows App SDK 1.8 运行时；若只想单机临时跑，可在 `WinUiReference.csproj` 里加 `<WindowsAppSDKSelfContained>true</WindowsAppSDKSelfContained>` 改为自包含。

## 构建与运行

```powershell
# 在本目录执行（Project 已把默认 AnyCPU 提升为 x64）
dotnet build -c Release

# 采集单个控件用例
.\bin\x64\Release\net8.0-windows10.0.19041.0\WinUiReference.exe `
    --out .\captures --control Button --state normal --theme light

# 批量状态拼版采集（单进程、状态并排拼版、命中指纹缓存自动跳过）
.\bin\x64\Release\net8.0-windows10.0.19041.0\WinUiReference.exe --batch --out .\captures

# 强制忽略缓存重截并同时保存 _sheet.png 拼版总览图
.\bin\x64\Release\net8.0-windows10.0.19041.0\WinUiReference.exe --batch --no-cache --save-sheet --out .\captures
```

## 与 unbound-ui 的对照闭环

参考图采集完成后，回到仓库根目录用展厅的差分模式跑完整闭环：

```powershell
.\target\debug\showcase.exe --parity-run `
    --reference tools\winui-reference\captures `
    --out target\parity
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
| `--out <dir>` | 任意目录 | `captures` | 输出目录，不存在会自动创建 |
| `--control <name>` | 见下方「已接入控件」 | `Button` | 大小写不敏感；与 `--batch` 同用时只采集该控件 |
| `--state <state>` | `normal` `disabled` `checked` `focused` | `normal` | `checked` 仅支持 CheckBox / ToggleSwitch / ToggleButton / RadioButton |
| `--theme <theme>` | `light` `dark` | `light` | 通过 `Host.RequestedTheme` 切换 |
| `--backdrop <kind>` | `solid` `mica` `mica_alt` `acrylic` `all` | `solid` | 确定性合成底板类型（`all` 仅限 `--batch`） |
| `--width <N>` | 64..8192 | `420` | 单例模式窗口宽度（批量模式按状态槽位自动计算并裁剪为紧凑出血画布） |
| `--height <N>` | 64..8192 | `160` | 单例模式窗口高度 |
| `--batch` | 开关 | 关闭 | 按「控件 × 主题 × 后景」生成状态拼版并自动切出紧凑单图 |
| `--no-cache` | 开关 | 关闭 | 忽略 `fingerprint` 缓存，强制重新采集全部用例 |
| `--save-sheet` | 开关 | 关闭 | 除切分后的单用例 PNG 外，额外保留 `<Control>_<theme>_<backdrop>_sheet.png` |

### 已接入控件（57 个）

- **首批试点**：`Button` `CheckBox` `ToggleSwitch` `Slider` `ComboBox` `TextBox`
- **第一批（基础输入与文本）**：`ToggleButton` `SplitButton` `RadioButton` `RatingControl` `ColorPicker` `NumberBox` `AutoSuggestBox` `RichTextBlock` `ProgressBar` `ProgressRing` `InfoBadge` `ToolTip`
- **第二批（导航、浮层、集合与日期）**：`InfoBar` `TeachingTip` `ContentDialog` `Flyout` `MenuFlyout` `MenuBar` `CommandBar` `NavigationView` `TabView` `Pivot` `BreadcrumbBar` `SelectorBar` `PipsPager` `Expander` `SplitView` `Border` `Card` `StackPanel` `ScrollView` `TreeView` `ListView` `FlipView` `CalendarView` `TimePicker` `PersonPicture` `Image` `FontIcon` `TitleBar`
- **第三批（后续补齐的 WinUI 3 控件）**：`RichEditBox` `GridView` `ListBox` `CommandBarFlyout` `AppBarButton` `AppBarToggleButton` `AppBarSeparator` `SwipeControl` `RefreshContainer` `AnnotatedScrollBar` `ItemsView`

> 说明：`Flyout` 与 `MenuFlyout` 本身不是 `FrameworkElement`，静态拼版以其呈现器（`FlyoutPresenter` / `MenuFlyoutPresenter`）为表面；`ToolTip`、`TeachingTip`、`ContentDialog` 需要运行时宿主才能展开，静态采集只测量其内容盒。这三类浮层后续由结构快照与人工屏幕捕获补足。
