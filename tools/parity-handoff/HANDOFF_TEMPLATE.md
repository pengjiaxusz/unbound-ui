# WinUI 3 像素对齐失败用例交接包（Parity Handoff Bundle）

> 生成时间（UTC）：{{GENERATED_AT}}
> 代码基线：`{{GIT_HEAD}}`
> 本包用途：把 **WinUI 3 官方参考截图 vs Unbound UI 实渲染** 的全部对比证据交给外部专业 Agent，定位并修复未通过的控件对齐问题。

---

## 0. 一句话结论

本轮在 **禁用双端指纹缓存**（`--no-cache`）的前提下重新采集参考图、重新离屏渲染并重跑四重门禁，共 **{{TOTAL}}** 个用例：

**{{COUNTS}}**

需要优先处理的是 `fail` 与 `invalid`，合计 **{{ACTIONABLE_COUNT}}** 个，逐例证据位于 `cases/fail/` 与 `cases/invalid/`。

---

## 1. 目录结构

```text
parity_bundle/
├── HANDOFF.md              # 本交接文档
├── environment.txt         # 生成环境、命令、report 哈希
├── report.json             # 原始差分报告（未经裁剪）
├── summary.json            # 汇总：状态计数、门禁分布、控件聚类、逐例索引
├── summary.csv             # 逐例扁平表，便于表格筛选
├── actionable_cases.json   # 全部 fail + invalid 的完整诊断（含源码定位）
├── cases/
│   ├── fail/<用例名>/       # compare.png / roi_zoom.png / reference.png / ours.png / heat.png / *.json / metrics.json
│   ├── invalid/<用例名>/    # 同上，附 invalidReason
│   ├── known-aa/<用例名>/   # 已复核的白名单，通常无需修复
│   └── pass/<用例名>/       # 通过用例，用于回归对照
```

每个用例目录的 `metrics.json` 是最权威的单例入口，包含：

- `report`：原始报告记录（四门禁指标、包围盒、ROI、固有尺寸）；
- `diagnosis.failedGates` / `failedGateDetails` / `primaryHint`：自动归因；
- `referenceMetadata` / `oursMetadata`：双端指纹、捕获参数、控件边界；
- `source`：该控件对应的组件源码、差分发射点、参考宿主源码路径。

---

## 2. 如何读对比图

### 2.1 `compare.png`（四联图）

| 面板 | 含义 |
| --- | --- |
| WinUI 3 reference | 官方 WinUI 3 采集截图（真机、真实 Fluent 模板） |
| Unbound ours | 本框架在 **相同画布、相同 controlBounds** 下的离屏渲染 |
| deltaE heat | 有效对比区（ROI）内的 ΔE2000 热力图，越亮差异越大 |
| 50/50 overlay | 两图半透明叠加，用于肉眼判断**位置偏移**还是**颜色/形状差异** |

叠加的包围盒颜色：

- **青色** = 参考图内容包围盒；
- **橙色** = 我方内容包围盒；
- **品红** = ROI（`union(参考bbox, 我方bbox, controlBounds) + 2px`）。

面板下方直接列出该例失败的门禁与实测指标。

### 2.2 `roi_zoom.png`（三联放大图）

只截取 ROI 区域，按最近邻放大（小区域 4×，大区域 2×），用于观察 1px 描边、圆角、字形覆盖率和基线偏移。**请优先看这张图做根因判断**，整图面板容易让细节被面积稀释。

---

## 3. 四重门禁口径（`apps/showcase/src/png_diff.rs`）

| 门禁 | 判定 | 阈值 |
| --- | --- | --- |
| 内容包围盒 | `referenceBbox` 与 `oursBbox` 逐边差 | ≤ 1 px |
| 固有尺寸 | 我方 `intrinsic` 与参考 `controlBounds` 尺寸；显式固定宽度的控件只校验高度 | ≤ 1 px |
| ROI ΔE2000 | `meanDe` ≤ 1.5 且 `p95De` ≤ 4.0 | 1.5 / 4.0 |
| 显著色差像素 | ROI 内 `ΔE2000 > 5.0` 的像素占比 | ≤ 2.0% |

补充规则：

- 任一侧内容像素占比 < 0.1%、内容包围盒为空、`controlBounds` 越界 → `invalid`，**不得计入 pass**（ADR 0009）。
- `known-aa` 是经放大图复核、仅剩轮廓抗锯齿差异的显式白名单，豁免条件为几何/尺寸已对齐且 `meanDe ≤ 2.0`，新增条目需按 ADR 0010 留复核记录。
- ROI 以双端内容包围盒与控件边界的并集计算，避免把大面积背景稀释进统计（ADR 0008）。

### 3.1 本轮失败门禁分布（一个用例可命中多个门禁）

{{GATE_TABLE}}

### 3.2 按控件聚类（优先修 fail+invalid 数量多的控件）

{{CONTROL_TABLE}}

### 3.3 优先处理的单例（按失败门禁数、显著色差占比排序）

{{TOP_CASES_TABLE}}

---

## 4. `invalid` 用例

{{INVALID_LIST}}

这些不是"画得不对"，而是**参考采集或内容有效性契约未满足**（例如 `controlBounds` 内无非背景像素）。请先检查 `tools/winui-reference/Catalog.cs` 与 `CaptureWindow.xaml.cs` 的槽位/等待逻辑，不要直接改渲染代码去迎合空图。

## 5. `known-aa` 白名单

{{KNOWN_AA_LIST}}

这些用例已人工复核，仅剩 1px 边缘抗锯齿差异，**通常不需要修复**。若要动字形覆盖率逻辑，必须重新复核整组。

---

## 6. 源码定位与修复建议

### 6.1 通用路径

| 关注点 | 文件 |
| --- | --- |
| 我方用例发射（描述符、几何、令牌） | `apps/showcase/src/parity.rs` 的 `emit_case()`，逐例行号见 `metrics.json` → `source.emitCaseLine` |
| 控件真实发射器 | `metrics.json` → `source.widgetSource`（`crates/unbound-widgets/src/<module>.rs`） |
| 图元与描边 | `crates/unbound-widgets/src/primitive.rs`、`crates/unbound-widgets/src/slot_fixture.rs` |
| 调色板 / Fluent 令牌 | `crates/unbound-theme/src/win11.rs` |
| 文本栅格化 | `crates/unbound-text/src/directwrite.rs`、`glyph.rs`、`shaping.rs`、`atlas.rs` |
| 参考宿主 | `tools/winui-reference/Catalog.cs`、`CaptureWindow.xaml(.cs)` |
| 门禁与统计 | `apps/showcase/src/png_diff.rs` |
| Fluent 设计规律 | `.agents/skills/fluent-control-design/REFERENCE.md`（亦见 `docs/agents/handoff-winui3-parity.md`） |

### 6.2 建议顺序

1. **先几何后颜色**：按 `diagnosis.failedGates` 中 `contentBBox` / `intrinsicSize` 优先；位置错了再调色毫无意义。
2. **按控件批量修**：同控件的 light/dark、normal/disabled/checked 往往共享同一处模板或令牌错误，修一处可清掉 4~6 例；参考 `summary.json` → `controls`。
3. **再看 ROI 放大图**：`significantPixelRatio` 高而包围盒/尺寸已对齐时，通常是描边宽度、圆角半径、文字基线或图标字形覆盖率问题。
4. **不要降低门槛**：不要修改 `png_diff.rs` 阈值、不要扩展 `KNOWN_AA_CASES`、不要改参考截图或缓存指纹来"让测试变绿"。
5. **每批修复后全量回归**：先 `--no-cache` 重跑，确认没有把原本 pass 的用例打回 fail。

### 6.3 环境注意事项

- 参考图来自真机 WinUI 3，部分控件引用系统强调色（采集机为红色 `#C21C14`），差分层通过 `SYSTEM_ACCENT_CONTROLS` 还原；不要误把它当成 Fluent 默认蓝。
- 本机 .NET 8 SDK 在 `C:\Users\pengj\.dotnet-sdk`，默认 PATH 可能只有运行时，需显式设置 `DOTNET_ROOT` 与 `PATH`。
- 生成时缺失的图像文件数：{{MISSING_IMAGES}}（若为 0 则全部用例都有完整四联图）。

---

## 7. 复现与验收命令

在仓库根目录（PowerShell）：

```powershell
$env:DOTNET_ROOT = "C:\Users\pengj\.dotnet-sdk"
$env:PATH = "C:\Users\pengj\.dotnet-sdk;" + $env:PATH

# 1. 重新采集 WinUI 3 参考图（全量，跳过双端指纹缓存）
just parity-capture --no-cache --theme all

# 2. 重新离屏渲染 + ROI 差分（全量，跳过缓存）
just parity --no-cache

# 3. 验证（重型命令统一经 just 排队）
just check
just test
just gate-check
```

产物路径：`target/parity/report.json`、`target/parity/ours/`、`target/parity/heat/`、`tools/winui-reference/captures/`。

**验收标准**：`just parity --no-cache` 的 `fail` 与 `invalid` 均为 0（`known-aa` 允许存在但必须保持白名单原意），且 `just check` / `just test` / `just gate-check` 全绿。

---

## 8. 给接手 Agent 的交付检查清单

- [ ] 先读 `actionable_cases.json` 与 `summary.json` 的 `gateHistogram`，确认主要矛盾是几何还是颜色。
- [ ] 每个待修用例至少查看 `compare.png` 与 `roi_zoom.png` 各一次。
- [ ] 修改前记录原指标，修改后记录新指标，写进提交说明。
- [ ] 不修改参考截图、门禁阈值、白名单与指纹版本。
- [ ] 修复完成后用上面的命令全量重跑并重新打包交接包。
