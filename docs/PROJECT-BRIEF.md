# Unbound UI · 项目简报（PROJECT BRIEF）

> **这份文件是什么**：整个项目的**稳定底座说明**，供不了解本仓库的第三方 AI / 架构顾问在不打开源码、
> 看不到任何对话历史的前提下，准确理解 Unbound UI 是什么、已经有什么、不能破什么。
>
> **配套文件**：外部征询时与一份「提问文件」（`docs/decisions/*.md`）**一起发出**。
> 本文件只讲项目事实，提出问题、验收定义与输出格式一律写在提问文件里。
> 一次决策换一份提问文件，本文件不随决策变动。
>
> **章节号是稳定契约**：提问文件按 `第 N 节` 引用本文件。增删内容时请保留既有编号，
> 新增内容追加到末尾或作为子节，不要整体重排。
>
> **标注口径**：【事实】= 已从源码、配置或文档逐条核对；【推断】= 附依据的推理；
> 【未知】= 未查到、需要另行核实。文档自称的数据不等于已复核事实，会单独标明来源。
>
> **快照日期**：2026-11-13，基于当前工作树。文末第 14 节说明答复纪律。

---

## 0. 三十秒读懂

- 【事实】这是一个用 Rust 写的 **GPU 驱动 UI 引擎 + 无限画布**，目标是把每一帧的成本压到与场景规模无关。
  它不是应用框架，也不是组件库，是一套渲染、布局、输入、语义与验证的底座。
- 【事实】渲染底座、增量排版、命中测试、动画、无障碍语义、无头验证等**引擎能力层已全部落地**（M1–M5）。
- 【事实】组件层已经成型：`unbound-widgets` 提供扁平描述符 + 纯发射函数的
  **59 个模块、40+ 标准组件**（工作区容器、导航命令、高阶表单、富文本与数据集合、
  反馈浮层、无限画布套件），由 `AppRuntime` 统一接线，见第 6、7 节。
- 【事实】运行时 DevTools Inspector（`F12`）与每帧 5 项无头体检闭环已落地，见第 7.3 节。
- 【事实】两个应用：`apps/showcase` 是开窗的**渲染验收台**，`apps/gallery` 是**纯控制台**
  的全链路演示，只打印数字、不出画面。

---

## 1. 项目定位与规模

| 项 | 内容 | 来源 |
| :--- | :--- | :--- |
| 定位 | 下一代高性能 GPU 驱动 UI 与无限画布引擎的 Rust 实现 | `README.md` |
| 第一原则 | 把每一帧的成本压到与场景规模无关 | `docs/00-overview/principles.md` |
| 四条准则 | 性能绝对优先 / 面向数据设计（DOD）/ 无锁响应式调度与超低延迟呈现 / Agent 与人类双模友好 | `docs/00-overview/principles.md` |
| Workspace | 21 个 `crates/*` + 2 个 `apps/*`，共 23 个成员 | `Cargo.toml` `[workspace].members` |
| 语言与工具链 | edition 2021，`rust-version = "1.85"`，MIT OR Apache-2.0 | `Cargo.toml` `[workspace.package]` |
| 代码规模 | 约 88 个 Rust 源文件、约 35 200 行、13 个着色器、562 项自动化测试 | `docs/00-overview/crate-map.md` 自述，**未逐项复核** |

> 【事实】`docs/00-overview/principles.md` 自己的「实现状态」标为 `蓝图`，但文中约束已在代码里以具体形式落地。
> 仓库文档的状态标注体系见第 11 节；读文档时不要仅凭标题的状态标记判断代码是否存在。

---

## 2. 技术栈与工程规范

**主要依赖**（【事实】，来自 `README.md` / `Cargo.toml` / `docs/00-overview/crate-map.md`）：
Vulkan 1.3（`ash`，强制 BDA / Bindless / Dynamic Rendering / Synchronization2）、`winit 0.30`、
Taffy 0.15（flex / grid 布局）、`cosmic-text 0.19`、AccessKit 0.24、`slotmap`、`rayon`、
`rkyv` / `postcard`。

**工程规范**（【事实】，`AGENTS.md` 托管块）：

1. 所有编译、检查、单测、跑分必须走 `just`（`just check` / `just test` / `just bench` / `just showcase`）
   或 `just q-run <cmd>`；**严禁裸跑 `cargo` / `ninja` / `cmake`**，以保证跨工作树排队调度与 CPU 降级防卡顿。
2. `cargo clippy --all-targets --all-features -- -D warnings` 要求 warnings 清零；`cargo fmt --check` 在 CI 校验。
3. 命名：`snake_case` 变量/函数/模块、`CamelCase` 类型/枚举、`SCREAMING_SNAKE_CASE` 常量。
4. `Cargo.toml`、`.cargo/config.toml`、`rust-toolchain.toml` 属于模板托管，改编译选项需同步模板；
   `src/main.rs` 归项目所有。

> 【事实·文档冲突】`README.md` 第 34 行起提供了「亦可使用原生 `cargo` 命令」的示例，与 `AGENTS.md`
> 第 8 行的禁令冲突。**执行时以 `AGENTS.md` 为准**，README 的示例属于快速上手说明、未同步规范。
> 【推断】给答复者的命令示例若写 `cargo test` 而不写 `just test`，本地会直接违反规范。

**可测性约定**（【事实】）：GPU 集成与窗口测试需要 Vulkan 1.3 设备；无设备时相关用例自动跳过而非失败。
核心正确性靠离屏像素断言与纯内存无头断言保证。

---

## 3. 架构分层与 crate 地图

【事实】19 个 crate 按依赖方向分层，上层可依赖下层、反向依赖由
`app-guest/tests/layering.rs` 在 CI 物理强制检查（`docs/00-overview/crate-map.md`）。

```
apps/showcase        开窗渲染验收台          apps/gallery        纯控制台全链路演示
app-guest            可热重载业务层 cdylib（纯数据）
unbound-host         Host-Guest C-ABI 热重载与状态迁移
unbound-abi          稳定 ABI：POD 描述符、Action、插件虚表
unbound-world / layout / anim / canvas / input / a11y / theme / text /
vector / graph / asset / platform / headless / kpi    引擎能力层
unbound-gpu          Vulkan 1.3 后端（依赖 math / theme / vector）
unbound-math         基础数学：Vec2 / Rect / Affine2 / Color / f16 打包
```

| crate | 一句话职责 | 主要依赖 |
| :--- | :--- | :--- |
| `unbound-math` | 基础数学类型与半精度打包 | 无 |
| `unbound-abi` | 扁平 POD 描述符、`ActionKind`、插件虚表 | `math` |
| `unbound-gpu` | Vulkan 后端：显存池、瞬态环、剔除排序、Uber 着色器、surface / swapchain | `math`、`theme`、`vector` |
| `unbound-vector` | 贝塞尔降阶、紧致 OBB、解析距离求解 | `math` |
| `unbound-text` | cosmic-text 塑形、双级缓存、双图集、MTSDF | `math` |
| `unbound-world` | SlotMap 扁平状态树、三级脏标记、虚拟化容器 | `math`、`abi` |
| `unbound-layout` | Taffy 增量排版、`RelayoutBoundary` 隔离求解 | `world`、`math` |
| `unbound-anim` | 物理弹簧、贝塞尔缓动、像素网格吸附 | `math` |
| `unbound-canvas` | 松散四叉树、智能磁吸、框选变换、事务化撤销 | `world`、`math` |
| `unbound-input` | 双层拾取、指针捕获、确定性光标栈、文件拖拽 | `world`、`math` |
| `unbound-a11y` | AccessKit 增量语义树 | `math`、`abi` |
| `unbound-theme` | 语义设计令牌、4 KiB 颜色表、三层正交缩放 | `math` |
| `unbound-widgets` | 扁平描述符 + 纯发射函数的组件库与 `SceneCollector` | `math`、`abi`、`theme`、`input`、`a11y`、`canvas`、`vector`、`text` |
| `unbound-app` | `AppRuntime`：世界/布局/文本/输入/状态池/Inspector 的统一接线 | `world`、`layout`、`theme`、`input`、`widgets`、`headless` |
| `unbound-graph` | 渲染图瞬态纹理生命周期、毛玻璃离屏池 | `math` |
| `unbound-asset` | 后台解码线程池、CPU 暂存环、传输队列交接 | `math` |
| `unbound-platform` | 窗口平台层：非阻塞缩放、低延迟呈现、延迟计量 | `math`、`gpu` |
| `unbound-headless` | 无头验证器：5 项几何体检、rkyv 时间旅行快照 | `world`、`layout` |
| `unbound-host` | C-ABI 热重载、分配器劫持、状态迁移 | `abi` |
| `unbound-kpi` | 验收红线实测 | 多数 crate |
| `app-guest` | 可热重载的业务层 cdylib | `abi` |

**三条刻意的依赖禁令**（【事实】，`docs/00-overview/crate-map.md`）：

| 禁令 | 原因 |
| :--- | :--- |
| `app-guest` 不得依赖图形栈 | 业务层重编译必须快；由 `app-guest/tests/layering.rs` 检查白名单 `["unbound-abi", "serde", "postcard"]` |
| `unbound-gpu` 不得依赖上层框架 | 渲染器必须能独立做无头测试 |
| `unbound-abi` 不得引入 `rkyv` / `postcard` 之外的序列化库 | ABI 稳定性要求格式可预测 |

---

## 4. 一帧的数据流

【事实】以下链路来自 `docs/00-overview/dataflow.md`（标注 `已落地`），是从 OS 输入到 GPU 呈现的运行期视角：

```
OS 输入 / 文件拖拽 / Agent 驱动
  → 双层拾取与输入路由：Portal 逆序检测 → UI 树双槽位圆角裁剪拾取 → 松散四叉树粗筛
    → 窄相精确几何判定（贝塞尔解析距离 / 逆变换局部坐标内点判定）
    → 瞬态交互态直写帧瞬态环（绕过 Reducer）
  → 派发可序列化 Action → COW Reducer 状态演进（语义树增量同步、无头模式触发 5 项体检）
  → 分叉：
      Level 2/3 脏 → Taffy 增量排版（RelayoutBoundary 阻断冒泡、Rayon 并行）
      Level 1 脏   → 独立变换/外观通道（Spring 更新 GpuTransform、主题只写 4 KiB LUT）
  → CPU 折叠嵌套裁剪 + 曲线降阶
  → GPU Compute：Incremental Scatter 修补持久池 → 分块相对坐标解算 + 视口剔除
                → OneSweep 64 位基数排序 + 生成 MDI 间接绘制命令
  → Vulkan 双通道 Uber-Shader：Pass A 不透明内核预通道只写 Reverse-Z 深度
                              Pass B 单次 MDI 从后向前绘制（Early-Z）
  → Present Wait 低延迟同步
```

**三级脏标记**（【事实】，`docs/00-overview/dataflow.md`）：

| 级别 | 触发 | 路径 | 排版开销 |
| :--- | :--- | :--- | :--- |
| Level 1 | 颜色、透明度、变换 | 直写瞬态环或 Compute Scatter 修补持久池 | 零 |
| Level 2 | 隔离边界内布局变化 | 只重算最近的 `RelayoutBoundary` 子树 | 局部 |
| Level 3 | 窗口尺寸或顶层分割线变化 | 全量重排 | 全局 |

**三条刻意绕过 Reducer 的旁路**（【事实】，同上）：瞬态交互态（框选 / 磁吸红线）直写帧瞬态环；
主题切换只改 4 KiB 颜色表；摄像机平移缩放只改 16 B Push Constants，画布实体在 GPU 侧原位解算。

---

## 5. 关键不变量与隐性约定

> 这一节列的都是「对方一旦不知道就会给错建议」的约定。修改它们等于改引擎语义。

| 约定 | 内容 | 来源 |
| :--- | :--- | :--- |
| 扁平具体类型 | 【事实】全面禁止深层嵌套泛型 Typestate 视图树（如 `VStack<HStack<Button<...>>>`），统一扁平描述符 + 轻量 `NodeId`；理由是避免泛型单态化膨胀与 LLVM 优化瓶颈 | `docs/05-agent/layering.md` |
| 正交极简 API | 【事实】每个布局属性 / 样式约束 / 交互行为**只有一种**正交声明方式，拒绝语法糖宏；专门用来防大模型生成代码时的 API 幻觉 | `docs/05-agent/layering.md` |
| Action as Data | 【事实】业务层只放 POD 状态、`ActionKind` / `RawAction` 记录、声明式构建器 | `docs/02-ui/dod-model.md` |
| 零每帧分配 | 【事实】核心路径帧内临时数据走帧瞬态环（每帧一个槽位），溢出显式报错；`GpuPrimitiveInstance` 恰好 64 B，占满一条缓存行 | `docs/00-overview/principles.md` |
| 主题零上传 | 【事实】亮暗两套主题共用同一套 token 槽位，切主题只写一次 4 KiB 颜色表（512 × RGBA16F），图元数据一字节不动，0 次重排 | `docs/02-ui/theme-and-zoom.md`、`docs/06-tooling/showcase.md` |
| token 槽位 | 【事实】0 = none（恒透明，描边图元的填充槽用）、1 = 阴影色（引擎 `SHADOW_TOKEN` 固定槽）、2 = 页面背景、3 = 面板底色、4 = 面板边框、5–9 = primary/secondary/success/warning/danger、10/11 = 正文/次要文字；**12–15 已被第一个 Button 的状态色占用**（hover / pressed / disabled / focus ring），展台注释称前 17 个槽位承载语义 token，其余为空。亮暗两套主题共用同一套槽位 | `apps/showcase/src/gallery.rs:87`、`docs/06-tooling/showcase.md` |
| 64 位排序键 | 【事实】图元绘制顺序由 64 位排序键决定；`PassLayer` 5 级（Canvas / Ui / Overlay / Modal / Debug）+ Portal；`SubElement` 6 层叠放（Shadow → Background → Content → InnerShadow → Border → FocusRing）；反向 Z 深度 | `crates/unbound-gpu/src/data.rs`、`docs/04-input/portal-and-sorting.md` |
| 场景上传 | 【事实】`SceneUpload { primitives, transforms, clips, payloads, generation, camera, key_prefix }`；**`generation` 相等则整帧跳过 GPU 上传**，这是平移/缩放免费的原因 | `crates/unbound-gpu/src/frame.rs:98` |
| 三级脏标记 | 【事实】`DirtyLevel::{Clean, Paint, Layout, Topology}`；颜色/变换只脏 paint 直写瞬态环，布局变化由 `RelayoutBoundary` 阻断向根冒泡，窗口尺寸变化才全量重排 | `unbound-world`、`docs/00-overview/dataflow.md` |
| 无头 5 项体检 | 【事实】`NoNaNOrInf / TextNotClipped / InteractiveTargetSize / NoSiblingOverlap / ViewportContainment` | `crates/unbound-headless/src/rules.rs` |

---

## 6. 已实现的公开能力清单（按 crate）

> 【事实】下表 API 名称来自各 crate 源码核对，属于**已经存在、可直接复用**的能力，不是规划。
> 这一节的作用是防止答复者建议重复造轮子。不确定是否存在的类型一律不要假设，见第 14 节。

| crate | 关键 API / 类型 |
| :--- | :--- |
| `unbound-gpu` | `PrimType` 8 种（`RoundedRect / Image / Glyph / GlyphColor / Curve / Stroke / Shadow / Line`）、`SubElement`、`BlendMode` 4 种、`PassLayer` 5 级、`GpuClipNode`（AABB + 两级圆角父裁剪）、`GpuTransform`、`GpuPayload`、`GpuPrimitiveInstance`、`SceneUpload`、`CLIP_NONE`、`SHADOW_TOKEN` |
| `unbound-abi` | `RawNodeDesc`（扁平 POD：fill / border_color / text_color / shadow_color / radius / border_width / shadow_blur / opacity / transform / z_index / key / payload）、`NodeTag` 7 种（`Container / Text / Image / Canvas / Scroll / Portal / Separator`）、`NodeFlags`（`INTERACTIVE / FOCUSABLE / HIDDEN / CLIP / WRAP / ANIMATED`…）、`Display` 4 种、`ActionKind` 31 种（`Press / PointerDown / PointerUp / Focus / Blur / Undo / Redo / SetTheme`…） |
| `unbound-input` | `HitRegion::{new, with_shape, with_cursor, with_clip}`、`Shape::{Rect, Rounded, Polygon, Bezier}`、`pick / pick_index / pick_stack`、`Router::{new, set_regions, hover, cursor, pointer_down, pointer_down_button, pointer_move, pointer_up, pointer_up_button, cancel}`、`Keymap::{bind, bind_sequence, feed}`、`KeyChord`、`PointerState`、`PointerEvent::{Down, Move, DragStart, Drag, Up, Click, DoubleClick, CaptureLost}`、`CursorStack`、`DragPayload` |
| `unbound-world` | `UiWorld`（SlotMap 状态树）、`NodeId`、`DirtyLevel::{Clean, Paint, Layout, Topology}`、`VirtualList / VirtualGrid / VirtualTree` |
| `unbound-layout` | `LayoutEngine::{new, sync, solve, solve_boundaries, solve_subtree, rect}`（Taffy flex/grid，`RelayoutBoundary` 增量求解） |
| `unbound-anim` | `Spring`、`Spring2`、`SpringPreset::{Snappy, Gentle, Heavy, Precise}`、`CubicBezier`、`Animation`、`Animator`、`FrameScheduler`、`should_snap` |
| `unbound-theme` | `Token`、`ThemeKind::{Light, Dark, HighContrast}`、`Palette`、`ThemeSet`、`ThemeLut`、`ThemeState::{switch, toggle}`、`SwitchOutcome`、`ZoomLayers`、`snap_screen` |
| `unbound-a11y` | `A11yNode { role: accesskit::Role, ... }`、`Semantics`（首次完整 TreeUpdate，后续 dirty / structural / removed delta）、`A11yAction::{Click, Focus, Blur, SetValue, Expand …}` |
| `unbound-text` | `TextEngine::{shape, wrap, measure}`、`TextStyle`、`GlyphCache`、`GlyphQuad`、`merge_line_rects`、`ime_cursor_area` |
| `unbound-vector` | 三次贝塞尔自适应降阶、紧致 OBB、二次曲线解析 SDF |
| `unbound-canvas` | `Selection`、`Marquee`、`HandleDrag`、`Snapper`、`UndoStack`、`LooseQuadtree` |
| `unbound-headless` | `Rule::ALL`、`Harness::{run, assert_clean}`、`Diagnostic::to_json`（含源码行号与约束链）、`TimeTravel` 快照 |
| `unbound-host` / `app-guest` | `GuestHost::{load, reload, reduce, build, snapshot}`、`state_schema_hash` 状态迁移 |
| `unbound-graph` / `unbound-asset` / `unbound-kpi` | `RenderGraph`、`OffscreenPool`；`AssetPipeline`、`StagingRing`；`measure_all().to_text()` |
| `unbound-platform` | `PlatformApp`（含 `next_wake` / `attach_waker`）、`PlatformEvent`（`KeyInput / MouseInput / CursorMoved / MouseWheel / Focused / ModifiersChanged / Resized / ScaleFactorChanged / ThemeChanged / RedrawRequested / RecreateSwapchain / CloseRequested`…）、`ResizeTracker`、`PlatformState::poll_resize`、`LatencyTracker` |

---

## 7. 组件层现状

【事实】通用组件层已经成立，位于 `crates/unbound-widgets`：59 个源码模块、40+ 标准组件，
统一遵循「扁平描述符 + 纯发射函数」的契约，所有颜色取 `unbound_theme::Token`。

### 7.1 组件契约

- 每个组件一个模块，导出 `XxxDesc`（纯数据）与 `emit_xxx(...) -> WidgetOutput`。
- `WidgetOutput { primitives, payloads, hit, a11y, focusable }`：图元、曲线/渐变 payload、
  命中区域、无障碍节点、是否进入 Tab 序。
- 组件不持有状态；交互瞬态由调用方传入的 `ControlState` / 组件自己的 `XxxState` 表达。
- `SceneCollector::push` 汇总图元与语义树，`push_clip` 负责圆角裁剪槽的折叠继承。

### 7.2 已交付范围

工作区容器（#1–#6）、导航命令（#7–#12）、高阶表单与录入（#13–#21）、
富文本/数据集合/反馈浮层（#22–#35）、无限画布套件（#36–#41），
以及 `unbound-gpu` 的渐变、内阴影、多层圆角裁剪与外部纹理绑定。

### 7.3 运行时接线与 DevTools

`AppRuntime` 把 `UiWorld`、`LayoutEngine`、文本、`Router`、状态池、TimerQueue 与
`SceneCollector` 串成一个每帧闭环，并提供：

- `F12` 切换的 DevTools Inspector：盒模型四色描边、`SourceLoc` 与违规面板，
  以最高 `z` 追加到场景末尾（调试通道）。
- `enable_debug_sanitizers(true)`：每帧把世界镜像成 `unbound_headless::Frame`
  （节点 id 即语义 key），跑 5 项 sanitizer，违规项以闪烁错误框回显。

【未知】组件数量继续增长后的按需重绘开销、以及 Inspector 面板文本的宿主字体接线，
尚无实测数据；后者目前通过 `InspectorScene::lines` 交给宿主绘制。

---

## 8. 两个应用的现状

### 8.1 `apps/showcase`（开窗渲染验收台）

【事实】定位是「一次看全所有图元」的可视验收台，现在额外承载第一个可交互按钮。源码：
`src/main.rs`（`Showcase` 状态、Vulkan surface / swapchain / atlases 初始化、事件循环、每帧渲染、`--shot` 无头 PNG）、
`src/gallery.rs`（`Gallery` 场景容器、`Palette`、`theme_table`、`Labels`、`build()` 与交互式构建入口）、
`src/button.rs`（Button 描述符与发射器）、`src/lib.rs`、`tests/gallery.rs`。

【事实】`Gallery::build()` 仍是一串硬编码绝对坐标调用，辅助方法 `rect()` / `stroke()` / `glyph()` / `curve()`；
`build_interactive()` 在此基础上按 `ButtonVisualState` 发射按钮图元。展品：全屏背景、标题、5 个色板、
一张卡片、蓝色焦点环、3 条贝塞尔彩线、底部提示行，外加一个按钮与点击计数标签。
`PrimType` 8 种里只覆盖 `RoundedRect / Stroke / Glyph 系 / Curve`；`Image / Shadow / Line` 未展示。

【事实】`Showcase` 实现 `PlatformApp`，事件循环现在经 `Router` 分发：

| 输入 / 事件 | 行为 | 是否 bump `generation` |
| :--- | :--- | :--- |
| `T` | 切主题（写 4 KiB 颜色表） | 否 |
| `Tab` / `Escape` / `Space` / `Enter` | 焦点切换与键盘激活 | 是（经由 `visual` 跃迁） |
| 左键按下命中按钮 | `Router` 捕获，视为 click，禁用 pan | 否 |
| 左键按下未命中 | 进入 `canvas_dragging` | 否 |
| 指针移动 / 滚轮 | 逆变换后喂 `Router`；拖拽时改 `pan` / `zoom` | 否 |
| hover 进出 / pressed / 点击计数变化 | `rebuild()` 整场重建 + `generation += 1` | 是 |
| 窗口尺寸稳定后 | `rebuild()` + 上传新字形 | 是 |

【事实】`unbound-theme` 仍在依赖里，但 `theme_table()` 是展台自己手写的 `Vec<[u16; 4]>`（512 槽），
没有调用 `ThemeSet` / `ThemeLut`；`unbound-input / world / a11y / abi / headless` 已加入依赖且被代码实际使用。

### 8.2 `apps/gallery`

【事实】纯控制台全链路演示与 KPI 报告（`just gallery -- --kpi`），只打印测量数字，不产生画面。
它已串联 `UiWorld → 增量 Taffy 布局 → 四叉树视口裁剪 → 命中测试 → 智能吸附 → 事务化撤销 →
AccessKit 语义 → 5 项无头体检`，但里面的 toolbar + 4 个「button」文本节点只是 `NodeId` 变量 +
文本节点 + action target，不是控件。

---

## 9. 稳定红线（假定不能破）

1. 不引入深层嵌套泛型视图树 / Typestate；保持扁平 POD + `NodeId` / 句柄。
2. 不破坏核心路径「零每帧分配」；每帧不能因为一个控件产生堆分配。
3. 不破坏主题切换的零图元上传机制（4 KiB 颜色表语义不变，slot 0 / 1 是引擎固定槽）。
4. 保持 64 位排序键 / 反向 Z / `SubElement` 叠放顺序语义；Portal 层保留给下拉 / tooltip / 对话框。
5. 保持「无 Vulkan 设备时测试自动跳过而非失败」与离屏像素可断言。
6. 保持 warnings 清零（`clippy --all-targets --all-features -- -D warnings`）与 `cargo fmt --check`。
7. 编译 / 测试统一走 `just`，不裸跑底层构建工具。
8. `app-guest` 的依赖白名单护栏不允许被绕过。

---

## 10. 测试、验收与工具链

| 手段 | 位置 / 命令 | 说明 |
| :--- | :--- | :--- |
| 展台离屏断言 | `apps/showcase/tests/gallery.rs::the_gallery_draws_something` | 640×400 离屏，断言 `primitives.len() > 10` 且读回像素颜色变化次数 `> 500`；无 Vulkan 设备提前 return |
| 按钮交互断言 | `apps/showcase/tests/gallery.rs`（提交 `4852108`，407 行 / 8 条） | `Fixture` 驱动 `Router` / `ButtonVisualState` / `Semantics` / `Harness` 的 CPU 侧状态机；`the_button_passes_the_five_sanitizers` 跑 5 项体检；像素差异断言在无 GPU 时跳过 |
| 双路径一致性 | `crates/unbound-gpu` 的 `the_borrowed_attachment_path_matches_the_offscreen_path` | 借用附件路径与离屏路径产物逐字节比对 |
| 无头出图 | `just showcase -- --shot shot.png [--dark]` | 同一份场景渲染到离屏目标写 PNG，供 CI / 文档逐像素比对 |
| 几何体检 | `unbound-headless` 的 `Harness::run/assert_clean` | 5 项规则，脱离 GPU 的纯内存断言 |
| 分层护栏 | `app-guest/tests/layering.rs` | CI 检查依赖白名单 |
| 量化红线 | `just gallery -- --kpi` | 见第 11 节 |
| 统一入口 | `just check` / `just test` / `just showcase` / `just q-run <cmd>` | 见第 2 节 |

---

## 11. 里程碑与未闭环项

【事实】`docs/07-roadmap/milestones.md` 把工程分五阶段，均标注 `已落地`：
M1 Vulkan 1.3 GPU 驱动底座与双通道 Uber-Shader；M2 双图集文字与二次贝塞尔 OBB；
M3 DOD UI 内核、增量排版隔离与虚拟化；M4 无限画布空间索引、全栈交互与低延迟窗口同步；
M5 无障碍语义、无头验证与极速热重载。

**文档自述的未闭环项**（【事实】，来自 `milestones.md` 与 `acceptance-kpis.md`，未逐项复核）：

| 项 | 状态 |
| :--- | :--- |
| Vulkan surface 与真实 swapchain | `milestones.md` 写「未创建」，但 `crates/unbound-gpu/src/surface.rs`、`swapchain.rs` 存在，且 `docs/06-tooling/showcase.md` 描述窗口呈现已落地并列出修复记录 → **两处文档不同步，以代码与 showcase.md 为准** |
| IME 预编辑事件流 | 未接入平台层 |
| 4K 分辨率下 10 万图元帧耗时 | 未复测；现有 0.67 ms 是 512×512 口径 |
| Input-to-Photon 真实端到端延迟 | 计量框架已实现，真实呈现路径的实测未给出 |
| 静止态 CPU/GPU 占用率 0.0% | 帧挂起机制已实现，未做长时运行统计 |

【事实】`docs/README.md` 的状态标注约定：`已落地` = 已进入代码且有自动化测试覆盖，
`部分落地` = 核心机制已实现但有明确缺口，`蓝图` = 已定义尚未进入代码。
其中 `02-ui/dod-model.md` 等篇章标 `已落地`，`01-graphics/window-presentation.md` 与
`04-input/ime-and-selection.md` 标 `部分落地`，`00-overview/principles.md` 标 `蓝图`。

【事实】KPI 实测摘录（`docs/07-roadmap/acceptance-kpis.md` 自述，口径见该文第 2 节）：
主题切换 4 KiB 单次写入 / 0 次重排 / 0 次实例重传；单次局部 `RelayoutBoundary` 求解 3.2 µs；
无头单帧 544 µs；业务层增量编译 282 ms。CPU 红线报**中位数**而非最快值。

---

## 12. 关键文件索引

| 主题 | 路径 |
| :--- | :--- |
| 工程规范 | `AGENTS.md`、`justfile`、`just/rust-workspace.just` |
| Workspace 成员与依赖 | `Cargo.toml` |
| 架构主索引 / 文档索引 | `docs/architecture/README.md`、`docs/README.md` |
| 设计准则 / 数据流 / crate 地图 | `docs/00-overview/{principles,dataflow,crate-map}.md` |
| 分层与 API 约束 | `docs/05-agent/layering.md` |
| 无头 5 项体检 | `crates/unbound-headless/src/rules.rs`、`docs/05-agent/headless.md` |
| 图元数据与排序键 | `crates/unbound-gpu/src/data.rs` |
| 场景上传与 generation | `crates/unbound-gpu/src/frame.rs:98` |
| 扁平节点描述符 / Action | `crates/unbound-abi/src/desc.rs:359`、`crates/unbound-abi/src/action.rs:13` |
| 命中测试与路由 | `crates/unbound-input/src/{hit,router}.rs` |
| 展台场景 / 窗口与事件 / 测试 | `apps/showcase/src/{gallery,main,lib,button}.rs`、`apps/showcase/tests/gallery.rs` |
| 按钮描述符与发射器 | `apps/showcase/src/button.rs`（`ButtonDesc` / `ButtonVisualState` / `emit_button`） |
| 按钮状态色与交互式构建 | `apps/showcase/src/gallery.rs`（`build_interactive` / `button_frame` / `button_rect` / `button_tokens`） |
| 展台设计文档 | `docs/06-tooling/showcase.md` |
| 全链路控制台演示 | `apps/gallery/src/main.rs` |
| 里程碑 / 量化红线 | `docs/07-roadmap/{milestones,acceptance-kpis}.md` |

---

## 13. 术语表

| 术语 | 含义 |
| :--- | :--- |
| `generation` | `SceneUpload` 的代际号；相等则整帧跳过 GPU 上传 |
| token / LUT | 语义颜色槽位与 512 × RGBA16F = 4 KiB 的颜色查找表 |
| `SubElement` | 单个视觉元素的 6 层叠放顺序，由图元排序键消费 |
| 反向 Z / Reverse-Z | z 越大越近、越晚画 |
| `RelayoutBoundary` | 阻断布局脏向根冒泡的隔离边界 |
| Action as Data | 交互以可序列化 POD 记录表达，业务层不持有闭包 |
| Portal | 逃逸父裁剪的浮层通道（下拉 / tooltip / 对话框） |
| 零上传 | 交互只改 Push Constants 或颜色表，不重传图元实例 |
| 展台 / showcase | 开窗渲染验收台，不是组件库展示页 |
| 提问文件 | `docs/decisions/*.md`，与本简报一起发给外部 Agent 的本次决策材料 |

---

## 14. 事实口径与答复纪律

- 本文件【事实】条目均给出来源（路径 / 行号 / 符号名）；标注「文档自述，未复核」的数字不要当作实测结论。
- 【推断】条目附了依据，可以被推翻；【未知】条目不要自行补齐。
- 如果不确定某个类型 / API / 路径是否存在，**请直接写「不确定」**，不要推测、不要编造名称。
- 第 7.2 节的未提交文档与截图更新属于快照事实；组件层的下一步方向应以提问文件为准，
  不要把「已落地一个 Button」误读成「组件框架已经成型」。
- 本文件不含任何待决策问题、验收定义与输出格式要求；那些内容在配套提问文件里。

---

**本文件的维护约定**：只在项目事实发生变化时增量更新（新 crate、新依赖、红线变更、里程碑闭环、
应用结构变化），保留章节编号。不要因为一次具体征询而改写本文件；一次征询对应一份新的提问文件。
