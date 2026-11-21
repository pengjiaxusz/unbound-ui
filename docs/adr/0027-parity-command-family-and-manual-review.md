# ADR 0027：parity 命令族统一与人工翻页检视模式

- 状态：已采纳
- 日期：2026-11-22
- 关联：ADR 0008, ADR 0015, ADR 0016

## 背景

WinUI 3 参考图闭环经过三个月的迭代，命令面长出了四层不一致：

1. **命名与直觉相反**：`just parity` 跑的是差分，`just parity-capture` 才是采参考图。「parity」这个词本身既指整条闭环，也指其中一段。
2. **同名旗标两种含义**：两侧都有 `--no-cache`，但 C# 宿主关的是用例指纹缓存，Rust 差分器关的是渲染结果缓存。ADR 记录过一次事故（handoff §6.5.1）：操作者切了系统强调色后只跑 `just parity --no-cache`，参考图仍复用了被污染的旧图。
3. **死旗标与过度耦合**：`--batch` 的真实语义只是「采全部控件」，却同时决定 `--theme all` / `--backdrop all` 是否合法；`--width` / `--height` 在画布固定 1920×1080 之后只剩「写进 JSON 元数据」的作用，却仍被文档当成交互参数介绍。
4. **差分不可聚焦**：`png_diff::run_batch_with_options` 只接受「参考目录 + 输出目录 + 缓存开关」，没有控件/主题/状态筛选。改一个控件要等全量 214 例。

同时，参考宿主只有一条运行路径：`PlanPages → AdvanceToNextUncachedOrStart → 350ms 收敛 → RenderTargetBitmap → Application.Current.Exit()`。
分页规划器（FFDH 装箱、视口溢出分页）早已存在，但「停在第 N 页」只存在于私有字段 `_sheetIndex`，
人工无法逐页比对——而这恰恰是像素对齐最容易卡住的地方：差分只告诉你哪一例超限，不告诉你参考图长什么样。

## 决策

### 决策一：`just parity` 作为唯一入口，下设五个动词

```
just parity            # 全闭环：增量采集 → 差分（缺省 all）
just parity capture    # 只采 WinUI 3 参考图
just parity diff       # 只跑 ROI 四门禁差分
just parity review     # 人工翻页检视
just parity report     # 打开展台 Pixel parity 页
```

`just parity` 本身只做一行委派（`just "parity-<动词>"`），实现留在各自配方里，
避免在 justfile 里堆条件分支；产物目录固定为绝对路径，与调用者工作目录无关。

### 决策二：旗标同名同义，删掉死参数

- 两侧统一为 `--refresh`（不要相信缓存）、`--control` / `--state` / `--theme` / `--backdrop`（用例筛选）。
- `--keep-sheets` 取代 `--save-sheet`；旧名保留一版并打印废弃提示。
- 删除 `--batch`（全量是默认，`--control` 收窄）与 `--width` / `--height`（画布尺寸是契约的一部分）。
- 显式拒绝已被删除的参数，而不是静默忽略：旧脚本必须立刻暴露问题。

### 决策三：检视模式与批采共用分页规划与画布

`--review` 复用 `BuildCandidateBatches` / `PlanPages` / `LoadSheet`，只替换「产出」这一步：
不采图、不退出，把同一份规划结果摆进窗口等用户翻页。

- 检视条放在 `Host` **之外**（窗口两行 Grid：`ReviewBar` + `ScrollViewer > Viewbox > Host`），
  因此永远不会进入 `RenderTargetBitmap` 的像素；`Host` 仍是显式 1920×1080，`TransformToVisual(Host)` 与切片坐标不变。
- 缩放只发生在显示层（`Viewbox`，`Stretch=Uniform` + `DownOnly`）；「截图本页」先把窗口钉回 1:1
  再走与批采完全相同的切片路径，导出后恢复窗口尺寸。所见即所采，不引入第二条像素路径。
- 整页命中指纹缓存时页码旁标 `cached`，让评审知道差分器会直接复用这一页。
- 翻页顺序固定为「后景 → 主题 → 页」，与自动批采一致；页号同时给出全局序号与本批次页号。

### 决策四：差分器接受筛选项与失败清单

`run_batch_filtered(reference, out, allow_cache, &ParityFilter)` 在解析参考图元数据后、渲染之前过滤，
未命中的用例不进入渲染器、不写产物。`--only-failed` 读取上一份 `report.json` 的 `fail` / `invalid` 清单
填入 `ParityFilter::names`，形成「评审发现问题 → 只重跑这几例」的短路。

## 备选方案

- **独立检视小程序**：另起一个只读 PNG 的浏览窗口实现简单，但会与宿主的装箱规划脱钩——
  人工看到的排版与批采排版一旦不同，「我没看到问题」就不再是可采信的证据。
- **在展台里做拼版总览页**：展台是 Rust 框架的自绘界面，天然拿不到 WinUI 3 原生控件的实况；
  拼版总览图（`--keep-sheets`）可以进报告页，但不能替代原生宿主里的人工比对。
- **保留 `--batch` 以兼容旧脚本**：一个只表示「全部」的开关会继续诱导使用者以为存在「单例模式」，
  并在文档里制造第二套语义；一次性删除并明确报错的迁移成本更低。

## 后果

- 一条命令族覆盖「采集 / 差分 / 检视 / 报告」，`--refresh` 在两侧含义一致，历史事故的触发条件消失。
- 人工可以停在任何一页上比对，不必再靠差分报告反推参考图内容；发现问题的页可一键留证。
- 差分可以按控件/状态/主题/后景收窄，也可以用 `--only-failed` 直击失败清单。
- 需要回归验证一次：`Host` 外包一层 `Viewbox` 后，`RenderTargetBitmap` 的输出必须与改造前逐像素一致
  （导出路径会先把窗口钉回 1:1，因此该风险被限制在「显示层」）。
