# WinUI 3 参考截图宿主

为 unbound-ui 的像素级一致性校验提供**官方 WinUI 3 控件参考截图**。本目录是一个独立的 .NET 8 / WinUI 3 非打包应用，不属于 Rust workspace，`just build` 不会处理它，也不需要改动仓库任何现有文件。

## 环境要求

- .NET 8 SDK（`dotnet --list-sdks` 能看到 8.x）
- Windows 10 1809+（本机需能显示 WinUI 3 窗口）
- Windows App SDK 运行时：本工程按 `WindowsAppSDKSelfContained=false` 构建，运行机需要安装 Microsoft Windows App SDK 1.8 运行时；若只想单机临时跑，可在 `WinUiReference.csproj` 里加 `<WindowsAppSDKSelfContained>true</WindowsAppSDKSelfContained>` 改为自包含。

## 构建与运行

```powershell
# 在本目录执行（Project 已把默认 AnyCPU 提升为 x64）
dotnet build -c Release

# 产物路径形如：
# bin\x64\Release\net8.0-windows10.0.19041.0\WinUiReference.exe

.\bin\x64\Release\net8.0-windows10.0.19041.0\WinUiReference.exe `
    --out .\captures --control Button --state normal --theme light

# 批量采集全部试点组合（单进程、单窗口，不反复弹窗）
.\bin\x64\Release\net8.0-windows10.0.19041.0\WinUiReference.exe --batch --out .\captures
```

## 与 unbound-ui 的对照闭环

参考图采集完成后，回到仓库根目录用展厅的差分模式跑完整闭环：

```powershell
.\target\debug\showcase.exe --parity-run `
    --reference tools\winui-reference\captures `
    --out target\parity
```

该命令对每张参考图调用 `unbound-widgets` 的同一个控件，按参考 JSON 的
`controlBounds` 渲染到同尺寸画布，然后逐像素计算 CIEDE2000，输出：

- `target/parity/ours/<用例>.png`：本框架渲染结果（附同名 JSON）；
- `target/parity/heat/<用例>.png`：ΔE 热力图（黑→蓝→绿→黄→红，30 封顶）；
- `target/parity/report.html` / `report.json`：逐用例指标与通过状态。

判定阈值：内容包围盒必须逐边相等，全图 ΔE2000 均值 ≤ 1.0，p99 ≤ 3.0。

> 构建/还原由主代理统一执行；本目录的创建过程不运行任何构建命令。

## 参数

| 参数 | 取值 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--out <dir>` | 任意目录 | `captures` | 输出目录，不存在会自动创建 |
| `--control <name>` | `Button` `CheckBox` `ToggleSwitch` `Slider` `ComboBox` `TextBox` | `Button` | 大小写不敏感 |
| `--state <state>` | `normal` `disabled` `checked` `focused` | `normal` | `checked` 仅支持 CheckBox / ToggleSwitch |
| `--theme <theme>` | `light` `dark` | `light` | 通过 `Host.RequestedTheme` 切换 |
| `--width <N>` | 64..8192 | `420` | 窗口客户区宽度（物理像素） |
| `--height <N>` | 64..8192 | `160` | 窗口客户区高度（物理像素） |
| `--batch` | 开关 | 关闭 | 单进程跑完全部试点组合（normal / disabled / checked × 明暗），忽略 `--control/--state/--theme` |

未知控件会在错误信息中打印可用控件清单并以退出码 `2` 退出；参数错误同样退出码 `2`；截图过程异常退出码 `1`。错误信息同时写入可执行文件同目录的 `winui-reference.error.log`（WinExe 从控制台启动时不一定能拿到 stderr）。

## 输出结构

```
<out>/
  Button_normal_light.png      # <控件>_<状态>_<主题>.png
  Button_normal_light.json     # 同名元数据，见下
```

每个用例的 JSON 字段：

- `processBitness` / `processArchitecture`：进程位数与架构
- `osVersion` / `osVersionNumber`：操作系统版本
- `windowsAppSdkAssemblyVersion` / `windowsAppSdkAssemblyFullName`：WinUI 程序集版本
- `rasterizationScale`：`Host.XamlRoot.RasterizationScale`
- `hostActualWidth` / `hostActualHeight`：Host 的 DIP 尺寸
- `requestedWindowWidth` / `requestedWindowHeight`：命令行请求的物理像素尺寸
- `imagePixelWidth` / `imagePixelHeight`：PNG 实际像素尺寸
- `controlBounds`：控件相对 Host 的边界（`TransformToVisual(Host)`，DIP）
- `controlFocusState`：截图时控件的 `FocusState`
- `pngFile`：对应 PNG 文件名

## 已知限制

- **hover / pressed 未实现**：枚举里保留，传入会按参数错误退出（需要模拟指针进入/按下，暂不纳入试点）。
- **不实现弹层**：ComboBox 只截收起状态，不展开下拉；MenuFlyout 等同样不做。
- **DPI**：截图只在当前桌面会话的缩放比例下有意义。要得到 150% / 200% 的参考图，必须在对应缩放比例的桌面会话里运行（manifest 为 PerMonitorV2）。`--width/--height` 是物理像素，PNG 尺寸 = Host DIP × `rasterizationScale`。
- **窗口必须可见且未最小化**：`CompositionTarget.Rendering` 在最小化/锁屏时可能不触发，届时 10 秒兜底计时器会直接截图，结果可能不完整。
- **focus 状态**：`TextBox` 获得焦点后光标会闪烁，不同时刻截图可能不同；窗口未激活时程序化聚焦也可能失败，请以 JSON 的 `controlFocusState` 为准。
- **元数据按用例落盘**：每次运行只写 `<控件>_<状态>_<主题>.json`，批量采集到同一目录不会互相覆盖；差分器按同名查找。
- **强调色被钉死**：`CaptureWindow.xaml` 的 `Host.Resources` 把复选框 / 滑块 / 开关的强调色资源固定为 Windows 11 默认蓝（浅色 `#005FB8`、深色 `#60CDFF`）。WinUI 默认跟随用户个性化主题色，不钉死则基线会随截图机器漂移。
- **ToggleSwitch 使用 Header 布局**：`Header="ToggleSwitch"` 对应「文案在上、开关在下」的 WinUI 模板；unbound-ui 的 `SwitchDesc` 目前是「文案在左、开关在右」的行布局，包围盒差异会被差分器显式报出。

## 扩展方式

在 `Catalog.cs` 的 `ControlNameList` 与 `TryCreate` 中补一条即可，宿主与参数解析会自动跟进；若状态需要额外处理，在 `TryApplyState` / `ApplyFocus` 中扩展。
