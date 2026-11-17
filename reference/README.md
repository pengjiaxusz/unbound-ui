# Reference checkouts

WinUI 3 / Fluent 的**只读参考仓库**，用于像素级校准控件的样式与行为。
克隆体被 [`clone.ps1`](clone.ps1) 拉取到本目录下，并由 `.gitignore` 整体忽略；
仓库里只跟踪本说明、脚本与 `.gitignore`，因此不会把几十万行外部代码带进历史。

## 为什么要单独放这里

- 这些仓库是**研究材料**，不是依赖：不参与编译、不进入 `Cargo.toml`、不嵌套 submodule。
  嵌套 submodule 会让 `git clone --recursive` 变成数 GB 的拉取，并且外部仓库的
  所有权/历史与本仓库无关。
- 抄的是**可验证的数值与结构**（模板里的 `CornerRadius`、`MinHeight`、状态画刷的
  alpha、动画时长与缓动），落点是 `crates/unbound-theme` 与 `crates/unbound-widgets`。
  不要把外部代码整段搬进来，许可证与风格都不同。
- 需要追溯来源时，用每个克隆体的 `git log -1` 记下 commit，写在对应的改动说明里。

## 用法

```powershell
# 全部克隆（浅克隆 + 大仓库稀疏检出）
pwsh -NoProfile -File reference/clone.ps1

# 只补其中几个
pwsh -NoProfile -File reference/clone.ps1 -Only microsoft-ui-xaml,WinUI-Gallery

# 重新拉取（会先删除旧目录）
pwsh -NoProfile -File reference/clone.ps1 -Force -Only FluentAvalonia
```

脚本按 `--depth 1` 浅克隆；巨型仓库（`microsoft-ui-xaml`、`PowerToys`、`Files`、
`fluentui`）使用 `--filter=blob:none --sparse`，只检出与控件外观相关的目录。

## 清单

| 目录 | 仓库 | 用途 |
| --- | --- | --- |
| `microsoft-ui-xaml` | microsoft/microsoft-ui-xaml | WinUI 3 官方 XAML 模板、主题资源、动画视觉。**样式数值的第一事实源**：`controls/dev`（每个控件的 `*_themeresources.xaml`）与 `dxaml/xcp/dxaml/themes` |
| `WinUI-Gallery` | microsoft/WinUI-Gallery | 官方控件画廊示例：控件的实际用法、示例页布局、交互组合 |
| `WindowsAppSDK-Samples` | microsoft/WindowsAppSDK-Samples | 官方 SDK 示例：Mica/Backdrop、窗口化、标题栏与服务集成 |
| `fluentui` | microsoft/fluentui | Fluent 2 设计令牌与 React 控件实现：间距、圆角、字体阶、状态色 |
| `fluentui-system-icons` | microsoft/fluentui-system-icons | 官方图标字体与 SVG 资产（只检出 `fonts`） |
| `FluentAvalonia` | amwx/FluentAvalonia | 社区最完整的 WinUI 风格跨平台复刻，含自制控件与主题资源 |
| `wpfui` | lepoco/wpfui | WPF 上的 WinUI 3 复刻：控件模板、导航、Snackbar 等 |
| `ModernWpf` | Kinnara/ModernWpf | 早期 WinUI 风格 WPF 实现，模板结构清晰，便于对照 |
| `Windows` | CommunityToolkit/Windows | Windows Community Toolkit：设置卡片、数据表格等复合控件 |
| `Labs-Windows` | CommunityToolkit/Labs-Windows | 实验性控件（Segmented、Sizer 等）的交互细节 |
| `DevWinUI` | ghost1372/DevWinUI | 社区 WinUI 3 控件库：额外控件与主题资源 |
| `WinUI.TableView` | w-ahmad/WinUI.TableView | 高性能 TableView：行高、选中、排序的视觉规则 |
| `WinUIEx` | dotMorten/WinUIEx | 窗口与 Mica 背板、标题栏扩展的工程实现 |
| `MicaForEveryone` | MicaForEveryone/MicaForEveryone | 系统级 Mica 材料规则与回退策略 |
| `Files` | files-community/Files | 真实 Win11 应用：导航、列表、右键菜单、工具栏的完整外观 |
| `PowerToys` | microsoft/PowerToys | 真实 Win11 应用：设置页、模块 UI、控件主题资源 |
| `DevToys` | DevToys-app/DevToys | WinUI 3 工具类应用：卡片、输入区、结果区布局 |

## 已知边界

- `microsoft-ui-xaml` 的 XAML 模板按 `VisualState` 组织，读的时候先找
  `CommonStyles/*.xaml` 里的 `Style TargetType=...`，再看 `ThemeResources` 里的
  画刷字典；两者合起来才是最终像素。
- 社区库（FluentAvalonia / wpfui / ModernWpf）会按自己的渲染器做取舍，
  出现分歧时以 `microsoft-ui-xaml` 为准。
- `WinUICommunity` 已停止维护，`Uno.Themes`（Material/Cupertino）与本项目目标无关，
  都不在清单里。
