# 组件展台：自举式 UI Gallery

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `apps/showcase/src/{main,shell,runtime_app,state,canvas,widgets,target,gallery,pages/*}.rs` |
> | **关键类型** | `GalleryRuntime`、`AppRuntime<GalleryRuntime>`、`GalleryApp`、`ShellTree`、`ShellLayout`、`GalleryState`、`ExampleCard`（`PageLayout`/`CardLayout`）、`Canvas`、`Target`、`Palette`、`Labels` |
> | **测试** | `apps/showcase/tests/gallery.rs`（15 个）+ `apps/showcase/tests/shell.rs`（36 个）+ `runtime_migration.rs`（5 个），全部无头运行；布局体检走 `unbound-headless` |

`apps/showcase` 现在是一个**自举（dogfooding）的桌面应用**：它用引擎自己的布局、
输入、语义、动画与主题原语，搭出对标 WinUI 3 Gallery 的三段式壳层与交互式组件
沙盒。打开窗口看到的不再是一块测试画布，而是一个带自定义标题栏、可折叠导航树、
全局搜索、主题/缩放/HUD 拨杆和标准 `ExampleCard` 的完整软件。

## 为什么需要它

引擎此前的示例程序 `apps/gallery` 是一个**纯控制台**程序：它只打印测量数字，
不产生任何画面。GPU crate 本身是离屏渲染器，`unbound-gpu` 的图集槽位还绑在
1×1 的哑纹理上——文字甚至无法采样。于是「引擎到底画不画得对」这个问题，
在窗口里是看不到答案的。

展台把这条链路补齐了：

| 环节 | 落地位置 |
| :--- | :--- |
| Vulkan surface 与 swapchain | `crates/unbound-gpu/src/surface.rs` |
| 真实 bindless 图集与上传通路 | `crates/unbound-gpu/src/atlas.rs` |
| 窗口化呈现（acquire → render → present） | `apps/showcase/src/main.rs` |
| 应用壳层与导航（指针路由与焦点策略归 `unbound-app`） | `apps/showcase/src/{shell,runtime_app}.rs` |
| 页面目录、示例与 Knob 定义 | `apps/showcase/src/pages/` |
| 图元绘制与主题配色 | `apps/showcase/src/{canvas,widgets,gallery}.rs` |

## 三段式应用壳层

```text
┌──────────────────────────────────────────────────────────────┐
│ ☰  Unbound UI  › Collections › Virtualized ListView          │
│      [搜索 Ctrl+K]              [Theme] [Zoom] [Snap] [HUD]  │
├──────────────┬───────────────────────────────────────────────┤
│ Home         │  Page Header（标题 / 描述 / crate 徽章）        │
│ Design       │  ┌─────────────────────────────┬────────────┐  │
│ Basic Input  │  │  Live Stage（约 70%）       │ Knobs 30%  │  │
│ Layout       │  │                             │            │  │
│ Collections  │  └─────────────────────────────┴────────────┘  │
│ Canvas       │  │  </> Code & Schema（可展开）                │  │
│ Motion       │  └────────────────────────────────────────────┘  │
│──────────────│                                               │
│ Telemetry    │                                               │
│ Settings     │                                               │
└──────────────┴───────────────────────────────────────────────┘
```

* **顶部全局栏**：logo、按钮式面包屑、全局搜索（`Ctrl+K` 命令面板）、
  Light/Dark 切换、`100/125/150/200%` 缩放分段器、像素对齐开关与 HUD 开关。
* **左侧 NavigationView**：顶部固定 Home / Design Guidance，中部 6 大分类树，
  底部固定 Engine Telemetry / Settings。展开 248 px、折叠 68 px，宽度由
  `unbound-anim` 的弹簧驱动；壳层在窗格窄于 140 px 时提前切换到图标栏，
  避免标签被裁切。
* **右侧页面视口**：统一 `PageHeader` + 垂直排列的 `ExampleCard`；内容滚动、
  导航滚动与示例内部滚动（虚拟列表、画布缩放）分层处理。

### 状态机与渲染管线

`GalleryState` 是唯一事实来源：`active_page`、`nav_expanded`、`search_query`、
`theme`、`zoom_step`、`pixel_snap`、`show_hud`、滚动位置、Knob 值、展开的代码
面板与全部演示状态。页面是这份状态的**纯函数**。

演示状态里的 `frame_history` 是一个滚动窗口（`FRAME_HISTORY_CAP` 帧），由
`tick` 每帧压入真实的 `frame_ms` / `upload_bytes` / `draw_calls`。Telemetry 的
帧率图因此不再是手绘正弦曲线，而是把这份实测缓冲交给
`unbound-widgets::chart::emit_chart` 绘制——与组件画廊用的是同一个图表发射器。

同理，Springs 的缓动曲线不再是静态手绘贝塞尔：`DemoState.curve_editor`
（`CurveEditorState`）保存两个控制点，壳层在 `press_demo` / `pointer_move` /
`pointer_up` 里把指针归约成 `begin` / `drag_to` / `end`，由
`unbound-widgets::curve_editor::emit_curve_editor` 实时绘制；预设分段器载入引擎
的 `CubicBezier::EASE` / `EASE_OUT` / `LINEAR`。

`GalleryRuntime` 把 `GalleryApp` 适配成 `unbound_app::Application`，窗口壳层只
通过 `AppRuntime<GalleryRuntime>` 驱动，不再自己维护事件解析与帧差分：

1. `AppRuntime::tick(dt)` 归约 Tick 并推进导航弹簧与演示动画；静态页面不重建，
   动画页面按约 30 Hz 提升 revision，空闲时窗口保持低功耗。
2. `AppRuntime::frame()` 在脏标记非空时调用 `view`，把 `GalleryApp` 重建出的
   图元、语义树与焦点链搬进 `SceneCollector`，再与上一帧做双缓冲差分。
3. `GalleryApp::rebuild` 求解**布局树**（`ShellTree`：`UiWorld` 节点树 +
   `LayoutEngine`），读出 `ShellLayout` 与每张卡片的几何，再由 `Canvas` 发射
   图元、命中区域、语义节点与 RangeZone；`ExternalDiff` 让壳层跳过自己的差分，
   保证每帧只扫描一次场景。

### 布局树

壳层的几何不再由手算的矩形堆出来，而是一棵交给 `LayoutEngine` 求解的节点树：

```
Root (Column)
├── TopBar (Row, 高 56)          ← 顶栏控件为绝对定位子节点
└── Body (Row)
    ├── Nav (Column, 宽 = nav_width)
    └── Content (Column, CLIP|SCROLL_Y)
        └── Page (Column)         ← 页边距、卡片间距
            ├── Header (高 140)
            └── Cards (Column, gap 18)
                └── Card (Column, 定宽定高 + CLIP ⇒ RelayoutBoundary)
                    ├── Head (标题与描述占位)
                    ├── Body (Row, WRAP)   ← 舞台 + OPTIONS 面板
                    │   ├── Stage (flex_grow 1, basis 314)
                    │   └── Knobs (宽 30%, min 188, max 318)
                    ├── CodeBar (高 30)
                    └── CodePanel (展开时存在)
```

- **折行由引擎决定**：`Stage` 与 `Knobs` 同处一条 flex 线，线宽不够时
  `Knobs` 自动换行落到舞台下方——没有硬编码的断点，窗口连续缩放时逐帧
  重新求解即可自适应。
- **卡片是重排边界**：卡片尺寸固定且裁剪子节点，世界因此把它标记为
  `RelayoutBoundary`，卡内变化不会牵连页面重排。
- **滚动不触发重排**：滚动只平移页面内的读数，布局本身不受影响。

自建 Canvas 层使用绝对逻辑坐标与递增 z 序，因此缩放（逻辑坐标 = 物理 /
（OS Scale × App Zoom））与像素对齐（`snap`）只影响几何，不影响命中与语义；
Portal 层用于搜索面板、菜单、对话框与 Tooltip，绘制顺序即命中顺序。

### 上传路径

场景每帧由状态重建，但**不是每帧全量上传**。`AppRuntime::frame` 返回的
`FrameReport.diff` 已经给出三种结果：

| 差分 | 含义 | 上传 |
| :--- | :--- | :--- |
| `Identical` | 一个字节都没变 | 什么都不传 |
| `Ranges` | 只有图元**内容**变了（悬停、按下、聚焦、拖动） | 只补丁这些切片 |
| `Whole` | 场景**形状**变了（图元数 / 变换 / 裁剪 / 载荷表） | 全量，并推进 generation |

补丁路径是 R4 的核心：局部 Scatter 把每个脏切片从 transients 环拷进持久
池，`generation` 不动，所以 GPU 不会把整池当成新的。`unbound-widgets::diff`
提供算法，壳层只负责回答"形状变没变"（它知道自己的四张表）。

为了让悬停永远走补丁而不是全量，控件层有一条硬约束：

> **控件的外观状态只改字段，绝不改图元数量。**

焦点环因此恒常存在、只用不透明度表达聚焦（`focus_ring(..., focused)`）；
分段控件的药丸、导航行的选中条同理。这条约束由
`crates/unbound-widgets/tests/stable_shape.rs` 断言：11 个控件 × 24 种状态，
图元数量必须恒定，而悬停必须改变字段。

示例舞台（`Target::Demo`）是定制绘制，允许悬停时多画几笔（表格行高亮、
对话框阴影），这类变化会退回全量上传——正确，只是不便宜。

**补丁不越界**：每个控件在发射时登记自己占用的图元区间（`Canvas` 的
`node_spans`），于是"脏切片属于哪个控件"是可查的。`a_hover_patch_stays_inside_the_hovered_control`
断言悬停产生的每一段脏区间都落在被悬停控件的区间内——上传得少，而且要
上传的正是该上传的那一段。

这也顺手查出一处身份冲突：侧栏底部的 "Engine Telemetry" 快捷行原本复用了
导航树里 Telemetry 行的 `NavPage(19)`，两处共用一个身份，结果是悬停任一处
两行一起亮、语义树少一个节点、span 表只留最后一次登记。页脚改用
`Target::Footer(0/1)`（reducer 早已支持），并由
`every_hit_region_has_its_own_identity` 守住"一个控件一个身份"。

**零上传的重绘**还有两条更便宜的路：切主题只重写 4 KiB 的 `ThemeLut`
（图元一个字节不动），滚动只平移读数（布局不重解）。

### 呈现尺寸的单一事实源

展台的窗口路径有两条尺寸：**窗口的请求尺寸**与**交换链的实际尺寸**。后者是
驱动的答案（`resolve_extent` 取 `currentExtent`），可能与请求差一个像素。

因此分工是固定的：

| 量 | 来源 |
| :--- | :--- |
| 要不要重建交换链 | 窗口的**请求**尺寸 vs 上次重建时的请求尺寸 |
| 布局视口 | 交换链的**实际**尺寸 ÷ 有效缩放 |
| UI 相机 | 交换链的**实际**尺寸 |

拿链的实际尺寸去比请求尺寸，会让"差一像素"变成每帧重建、永不收敛；拿窗口
尺寸去布局、拿链尺寸去投影，则是两张不同的画面——右侧或底部留白/被裁。表现
出来的症状就是"点最大化后部分控件没跟着重排"。

这些规则收在 `apps/showcase/src/frame_plan.rs` 的三个纯函数里
（`frame_size` / `needs_rebuild` / `upload_plan`），由
`apps/showcase/tests/frame_plan.rs` 守住。

### 丢帧必须强制全量上传

`AppRuntime::frame` 每次重建都前移差分基线，**无论这一帧是否画了出来**。一
帧被丢掉（交换链过期、渲染失败、栅栏等待失败）时，池里的内容落在基线后面。
此时若按差分的切片去补丁，修的是错位的字节；下一帧又会报"没有变化"，那一
块就永远停在旧画面上。

所以宿主记着上一帧是否被丢：被丢过，下一次上传就走全量。渲染器侧同源——
`UploadGeneration` 把代次分成"已提交"与"已暂存"，只在提交成功之后才推进。

### 字形上传失败不许丢字形

字形图集的上传会失败（设备丢失、暂存缓冲耗尽）。若失败被吞掉，图集缓存却
认为字形已经在图集里，文字就会去采样从未写入的纹素——空白或乱码，而且永远
不会自愈。失败时整批更新被放回队列，下一帧重试。

### 图集淘汰时钟必须自己走

字形图集按**最近使用**淘汰，而"最近"是以帧计的。图集时钟原先挂在上传代次
上，而代次只在场景**形状**变化时才推进——于是时钟冻结，满图集时每个槽位看
起来一样旧，淘汰退化成任意。时钟现在独立于上传代次，每次重建都推进。

## 跑起来

仓库统一走 `just` 入口（内部经全局构建队列）：

```bash
just showcase
```

窗口里的操作：

| 输入 | 行为 |
| :--- | :--- |
| `Ctrl+K` | 打开全局搜索命令面板，按控件名或 `MTSDF` / `Quadtree` 等标签过滤 |
| `Ctrl+N` | 打开第二个窗口：独立 Surface / Swapchain，共享同一设备与渲染器 |
| `/` 后输入 | 在搜索框内输入，`Enter` 跳到首个结果 |
| `Tab` / `Shift+Tab` | 在焦点链里前后移动 |
| `Enter` / `Space` | 激活当前聚焦的控件 |
| `Esc` | 关闭搜索 / 菜单 / 对话框 |
| `T` | 切换 Light / Dark |
| `F12` | 开关 HUD |
| `Ctrl+滚轮` | 切换全局 UI 缩放（100 / 125 / 150 / 200 %），物理窗口随缩放调整 |
| 滚轮 | 滚页面、导航树，或缩放任一示例内部的画布 / 列表 |
| 左键拖拽 | 拖动 Knob 滑块、拖拽排序卡片、框选画布节点或平移画布 |
| 拖拽滚动条滑块 | 拖动内容 / 导航 / 虚拟列表滚动条；点击轨道上下方跳转（越过 3 px `drag_slop` 后仍连续跟随） |
| 拖拽标题栏 / 双击标题栏 | 移动窗口 / 最大化与还原 |
| 拖拽窗口边缘 | 从八向边缘缩放窗口（无边框 CSD） |

### 无头模式

CI 与本地排查没有窗口时，`--shot` 会把**同一份壳层**渲染到离屏目标并写成 PNG。
截图是构建产物，默认落在 `target/showcase/`，不进版本库：

```bash
just showcase -- --shot                  # target/showcase/gallery-light.png
just showcase -- --shot --dark           # target/showcase/gallery-dark.png
just showcase -- --shot out.png
just showcase -- --shot out.png --width 960 --height 600 --zoom 200 --hud
```

## ExampleCard 与 Knob

每张示例卡片由 `pages::ExampleDef` 静态描述：标题、说明、关联 crate、Rust 代码、
无障碍说明、`KnobSpec` 列表与 `DemoKind`。壳层负责统一外观：

* **Live Stage**：主题背景或网格背景上的真实交互画布。
* **Options / Knobs**：`Toggle`、`CheckBox`、`Slider`（带 RangeZone，指针可拖出
  行外仍保持捕获）与 `Segmented`（单选胶囊），右侧一改左侧立即重排。
* **Code & Schema**：折叠区展开后显示声明式 Rust 片段与 A11y 描述；展开高度
  影响页面总高度，由 `content_scroll` 统一滚动。

Knob 值以 `(page, card, knob)` 编码进 `GalleryState.knobs`，默认值来自
`KnobSpec`，因此页面定义保持只读，交互状态集中在状态机里。

### 八大板块

| 分类 | 页面 |
| :--- | :--- |
| Home | Hero、能力卡、全组件索引 |
| Design Guidance | Colors、Typography、Elevation、Vector |
| Basic Input | Buttons、Toggles、Sliders、Text Field |
| Layout & Portals | Flex、Overlays、Drag & Drop |
| Collections | Virtualized List、Asset Streaming |
| Infinite Canvas | Viewport & Quadtree、Marquee & Snapping、Undo / Redo |
| Motion, A11y & Telemetry | Springs、Accessibility、Telemetry、Settings |
| Components | 组件画廊：56 个标准组件各一张卡，全部由真实 `catalog::emit_widget` 发射 |

## 自动化验证

`tests/shell.rs` 不创建窗口，直接驱动 `GalleryApp`：

| 用例 | 验证的链路 |
| :--- | :--- |
| `every_page_passes_the_headless_sanitizers` | 23 个页面全部通过 `unbound-headless` 的布局体检 |
| `the_component_gallery_emits_every_catalog_widget` | 滚动画廊全页并集 `emitted_widgets`，断言 100% 覆盖 `WidgetCatalogId::ALL`（S2.4 运行时半区） |
| `navigation_updates_the_breadcrumb_and_active_page` | 导航行点击 → 面包屑与页面切换 |
| `the_navigation_pane_collapses_and_expands` | 折叠/展开改变导航宽度与图标栏 |
| `ctrl_k_searches_the_catalogue_by_crate_and_tag` | 按 `MTSDF` 等标签搜索并跳转 |
| `code_panels_change_the_page_extent` | 代码折叠区改变内容高度 |
| `knobs_are_live_and_route_through_the_reducer` | 滑块拖拽写入 Knob 并触发重排 |
| `theme_and_zoom_switches_are_one_click` | 主题 / 缩放分段器单击生效且不双触发 |
| `pointer_hits_navigation_rows_without_a_window` | 指针命中与 `Router` 热区一致 |
| `search_and_theme_keep_the_same_target_identity` | 稳定 `u64` 目标身份跨重建不变 |

`tests/frame_plan.rs` 覆盖窗口路径的三条不变式：

| 用例 | 验证的链路 |
| :--- | :--- |
| `the_layout_follows_the_committed_extent` | 布局视口来自链的实际尺寸，不是窗口尺寸 |
| `a_one_pixel_difference_does_not_rebuild_forever` | 重建判据只比较两个**请求**尺寸 |
| `a_dropped_frame_is_repaired_by_a_full_upload` | 丢帧后的下一次上传是全量 |
| `the_page_reflows_for_the_presented_size` | 页面按呈现尺寸重排 |

`tests/shell.rs` 另有两条守住控件层的形状纪律：
`the_maximise_icon_keeps_its_primitive_count`（最大化/还原的图元数量恒定）与
`the_atlas_eviction_clock_advances_on_every_rebuild`（图集淘汰时钟每次重建都
走）。

`tests/gallery.rs` 保留全部 13 个旧用例，继续覆盖按钮四态、像素差异与借用附件
路径；壳层重构没有改动底层渲染契约。

## 展台暴露出的引擎缺陷

展台不是一个「装饰品」，它是这些缺陷的发现方式——每一个都对应一处真实修复：

| 症状 | 根因 | 修复 |
| :--- | :--- | :--- |
| 色块整体偏移 `pad` 像素 | 片元着色器把局部坐标又减了一次阴影扩张量 | `p = v_local` |
| 边框变成 14 px 的粗环、阴影不出现 | `unpackHalf2x16` 的分量顺序取反 | 按规范取低位在 `.x` |
| 面板被描边「顶穿」、文字底下出现暗块 | 不透明内核预通道给所有图元都写了深度 | 预通道只写不透明容器的内核矩形 |
| 焦点环完全不可见 | 描边分支取了填充槽的颜色 | 描边取边框槽的颜色 |
| 阴影看不见 | 阴影色被硬编码在 LUT 第 1 槽，展台却放在第 10 槽 | 统一槽位约定 |

### 第二轮：窗口路径

展台的**窗口**也暴露了一整类缺陷——它们在离屏渲染里完全看不见，因为离屏
路径压根不碰交换链：

| 症状 | 根因 | 修复 |
| :--- | :--- | :--- |
| 窗口只画出三帧就停住 | winit 默认 `ControlFlow::Wait`，事件循环睡到下一个输入事件 | 平台层按帧节奏设 `WaitUntil`，应用呈现后 `request_redraw()` |
| 拖拽窗口边缘时黑边、拉伸 | 展台一看到尺寸变化就重建交换链，绕过了平台的延迟重建策略 | 接入 `ResizeTracker`，稳定后只重建一次 |
| 拖拽结束后尺寸停在旧值 | 稳定窗口的 tick 只在 resize 事件到达时触发，最后一条事件之后就再也不提交了 | `PlatformState::poll_resize` 每帧驱动 |
| 点最大化后部分控件没跟着重排 | 布局视口取自窗口尺寸、UI 相机取自交换链实际尺寸，两者差一像素就画出两张不同的画面 | 两者同源于已提交的链尺寸；重建判据改用请求尺寸 |
| 每帧重建交换链、永不收敛 | 重建判据拿"链的实际尺寸"比"窗口尺寸"，驱动裁剪出的那一像素差异被当成尺寸变化 | 只比较两个**请求**尺寸 |
| 丢帧后局部区域永远停在旧画面 | 运行时重建即前移差分基线，丢掉的那帧的补丁就此丢失 | 丢帧后的下一次上传强制全量；代次只在提交成功后推进 |
| 字形偶发空白/乱码且不自愈 | 字形上传失败被吞掉，缓存却认为字形已在图集里 | 失败时整批放回队列，下一帧重试 |
| 满图集时总淘汰同一批字形 | 图集淘汰时钟挂在上传代次上，而代次只在场景形状变化时推进 | 淘汰时钟独立于上传代次，每次重建都走 |
| 窗口可能渲染到未就绪的图像 | `vkAcquireNextImageKHR` 的信号量与栅栏**都传了空句柄**（非法用法，有的驱动会容忍） | 交换链持有信号量池，渲染提交等待 `image_available` |
| 交换链图像从未转出 `UNDEFINED` | 渲染器认为「外部附件由持有者负责转布局」，而展台没转 | `render_to_view` 接收图像句柄并自行转换 |
| 低延迟呈现把帧循环卡死 | `vkWaitForPresentKHR` 无限等待，而窗口没被合成时呈现永不完成 | 等待加上限 + 连续超时后自动关闭低延迟路径 |

窗口路径的正确性现在有一条像素级断言守着：
`the_borrowed_attachment_path_matches_the_offscreen_path` 用一张与交换链
同 usage 的图像当附件渲染，与离屏路径的产物**逐字节比对**——两条路径必须画出
同一张画面。

### 第三轮：分配器

展台之外，`crates/unbound-gpu/src/tlsf_probe.rs` 留下了一个自称"TEMPORARY
probe / Delete after audit"的探针。它的用例一直失败——**审计从未完成**。

顺着它查下去是一个真实的堆破坏：TLSF 按桶找空闲块，而桶只保证下界，于是
可能把一个比请求小的块交出去；切分时 `block.size - want` 在无符号整数上回绕，
"剩余块"的表头被写到竞技场之外。

探针文件已删除，缺陷由 `crates/unbound-gpu/src/tlsf.rs` 里的
`a_request_bigger_than_the_free_hole_is_refused` 守住。

### 第四轮：拖拽与滚动条全链路（P0）

交互审计暴露出一整类「拖拽断链」缺陷——离屏体检全绿，但真实指针一拖就失效：

| 症状 | 根因 | 修复 |
| :--- | :--- | :--- |
| 滚动条滑块 / Knob / 画布平移一拖就停 | 越过 3 px `drag_slop` 后路由器改发 `DragStart` / `Drag`，壳层却把这些事件丢弃 | `runtime_app` 把 `DragStart` / `Drag` 与 `PointerMove` 归约到同一 `pointer_move` |
| 点击滑块**上方**轨道后页面乱跳 | `begin_scrollbar_drag` 只判 `grab > thumb_h`，上方点击存下负 grab，下一次移动突然跳页 | 改用 `!(0.0..=thumb_h).contains(&grab)`，上方/下方轨道点击都把滑块中心对准指针 |
| 导航树溢出却滚不动 | 导航 pane 没有滚动条热区与拖拽状态 | 新增 `Target::NavScrollbar`（tag 20）+ `nav_scrollbar` 几何 + `begin_nav_scrollbar_drag` |
| 两个虚拟列表卡片共用一份滚动状态 | `list_scroll` / `list_selected` 是单值，卡片互相覆盖 | 改为 `[f32; 2]` / `[Option<usize>; 2]`，按 `ctx.card` 索引；滚轮按卡片分 tag（1 / 5） |
| 拖拽窗口边缘误触发缩放 | 标题栏按钮排除区混用物理 / 逻辑像素，且未排除正在捕获指针的控件 | `resize_edge_at` 统一物理像素，并在 `pressed` 或 `hover_captures_pointer` 时返回 `None` |
| 引擎滚动视图滑块拖不到底 | `drag_to` 用 `max_scroll / track_length` 作比例，忽略了滑块自身长度 | 比例改为 `max_scroll / (track_length - thumb_length)`，与 `thumb_rect` 同源 |

引擎侧 `unbound-widgets::scroll` 同步收敛为可复用契约：`ScrollViewOutput` 以
`hits: Vec<HitRegion>` 取代单个 `thumb`，新增 `ScrollPointerEvent`
（`Press` / `Drag` / `Release`）与 `ScrollState::reduce_pointer`，把「按下滑块 /
点击轨道 / 拖拽 / 释放」整套状态机收进组件层，宿主只需转发指针事件。

这条链路由三条无头回归测试守着（`tests/shell.rs`）：

| 用例 | 验证的链路 |
| :--- | :--- |
| `dragging_the_content_scrollbar_moves_the_page_across_drag_slop` | 按下内容滚动条滑块、分步拖拽越过 `drag_slop`，断言 `content_scroll` 逐帧精确递增；点击滑块上方轨道断言向上跳转 |
| `dragging_the_virtual_list_scrollbar_scrolls_to_100k_end` | 把 10 万行虚拟列表的滑块拖到轨道底端，断言滚动落在距理论最大值一行以内（末行可达） |
| `dragging_the_navigation_scrollbar_scrolls_the_tree` | 短窗口下导航树溢出，拖拽导航滚动条断言 `nav_scroll` 递增 |

### 主题切换为什么是「零上传」

亮暗两套主题**共用同一套 token 槽位**，只有颜色表里的值不同。切换主题因此只
是一次 4 KiB 的颜色表写入——图元数据一个字节都不用动，这与
[正交缩放与主题切换](../02-ui/theme-and-zoom.md) 描述的机制一致。

壳层把槽位扩展到 48 个，覆盖标题栏、导航、卡片、舞台、代码面板、弹层、Hero、
HUD 与语义色软背景。按钮和导航的状态色必须占用**独立槽位**：如果 hover 去改
`primary` 槽，同屏所有引用 primary 的图元会一起变色。

| 槽位 | 含义 |
| ---: | :--- |
| 0 | 无（恒透明，描边图元的填充槽用它） |
| 1 | 阴影色（引擎的 `SHADOW_TOKEN` 固定槽） |
| 2 – 4 | 页面背景 / 面板底色 / 面板边框 |
| 5 – 9 | primary / secondary / success / warning / danger |
| 10 / 11 | 正文 / 次要文字 |
| 12 – 16 | primary hover / pressed / on-primary / focus ring / disabled fill |
| 17 – 24 | 标题栏、导航（hover / selected / text）、卡片、舞台与网格 |
| 25 – 31 | 控件轨道 / 滑块拇指、代码面板、弹层遮罩与底色 |
| 32 – 48 | 语义软背景、Hero 渐变、选区、分隔线、HUD、强调与导航文字 |

---

**相关文档**：[Uber-Shader 与双通道深度](../01-graphics/uber-shader.md) ·
[解析解阴影](../01-graphics/shadow.md) · [工业级文字渲染](../01-graphics/text.md)

**返回**：[06-tooling](README.md) · [文档索引](../README.md)
