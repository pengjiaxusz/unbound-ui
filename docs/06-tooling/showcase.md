# 组件展台：一次看全所有图元

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `apps/showcase/src/{main,gallery,button}.rs`、`apps/showcase/tests/gallery.rs` |
> | **关键类型** | `Gallery`、`Palette`、`Labels`、`Showcase`、`ButtonDesc`、`ButtonVisualState`、`Router` |
> | **测试** | `apps/showcase/tests/gallery.rs`；渲染正确性由 `crates/unbound-gpu` 的像素断言覆盖 |

`apps/showcase` 是引擎的**可视验收台**：一个真实开窗的 Vulkan 程序，把引擎能画
的每一种图元都摆在一屏里。启动它不需要任何资产、不需要配置文件，窗口一开就能
看到圆角矩形、边框、解析阴影、描边环、贝塞尔曲线、MTSDF 字形与双主题配色。

![亮色主题的组件展台](../assets/showcase-light.png)

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
| 图元排布与主题配色 | `apps/showcase/src/gallery.rs` |

## 跑起来

```bash
cargo run -p showcase --release
```

窗口里的操作：

| 输入 | 行为 |
| :--- | :--- |
| `T` | 在亮色与暗色主题之间切换 |
| `Space` | 有焦点时激活按钮；无焦点时切换主题 |
| `Enter` | 激活当前聚焦的按钮 |
| `Tab` / `Esc` | 聚焦按钮 / 取消焦点 |
| 滚轮 | 以视口中心为锚点缩放（0.25× – 8×） |
| 左键拖拽 | 在空白处拖动平移画布；按在按钮上则是点击而不是平移 |

### 无头模式

CI 与文档流水线没有窗口，`--shot` 会把**同一份场景**渲染到离屏目标并写成 PNG：

```bash
cargo run -p showcase --release -- --shot shot.png          # 亮色
cargo run -p showcase --release -- --shot shot.png --dark   # 暗色
```

![暗色主题的组件展台](../assets/showcase-dark.png)

这让「渲染结果」变成可以进版本库、可以逐像素比对的东西，而不是只能靠人眼看。

## 场景里摆了什么

`gallery.rs` 的 `build()` 用引擎自己的公开 API 拼出一屏场景，每个元素都对应
一类图元：

| 元素 | 覆盖的引擎能力 |
| :--- | :--- |
| 页面背景 | 全屏圆角矩形、z 序、反向 Z 深度 |
| 五个色块 | 填充 + 1 px 边框 + `erf` 解析阴影 |
| 白色卡片 | 圆角 + 边框 + 阴影，圆角半径 16 |
| 蓝色焦点环 | `PrimType::Stroke` 的描边环 |
| 三条彩线 | 二次贝塞尔曲线的解析 SDF |
| 标题与各处标签 | 双图集 + MTSDF 字形采样 |
| 底部提示行 | 深色主题下的文字极性补偿 |
| Button 卡片 | 第一个可交互控件：hover / press / focus / click |

### 第一个可交互控件

`button.rs` 给出组件的形态：**POD 描述符 + 派生状态 + 纯函数发射器**，而不是
一个带方法的状态容器。

* `ButtonDesc` 只描述几何与文案；组件身份来自 `unbound-world` 的 `NodeId`。
* hover / pressed **不落任何 `bool` 字段**，每帧由 `unbound-input::Router` 的
  捕获状态派生。按下后拖出按钮仍然保持 pressed（指针被捕获），松开越界不会
  合成点击——这套经典场景不会出现两套状态失步。
* 焦点与指针正交：`ButtonVisualState { phase, focused, disabled }`，所以
  "Tab 聚焦后又悬停"能同时画出 hover 底色与外部焦点环。
* 图元按 `SubElement`（`Shadow → Background → Content → Border → FocusRing`）
  分层，五层共享同一个 `z`，由 64 位排序键决定顺序。

窗口管线里有三处必须对齐：

1. **屏幕坐标 → 画布坐标**：`Camera` 的映射是
   `screen = (world - anchor) * zoom + viewport/2`，喂给 `Router` 之前必须做
   逆变换，否则缩放/平移后热区会与画面错位。
2. **点击优先于平移**：左键按下先问路由器是否命中 `HitRegion`；命中则捕获
   指针并聚焦按钮，未命中才进入画布拖拽。
3. **离散状态跃迁守卫**：只有 `(视觉态, 点击计数)` 变化的那一帧才重建场景并
   递增 `generation`。平移、缩放、切主题、鼠标在按钮内部滑行都不触发上传。

### 主题切换为什么是「零上传」

亮暗两套主题**共用同一套 token 槽位**，只有颜色表里的值不同。切换主题因此只
是一次 4 KiB 的颜色表写入——图元数据一个字节都不用动，这与
[正交缩放与主题切换](../02-ui/theme-and-zoom.md) 描述的机制一致。

按钮的状态色也必须占用**独立槽位**：如果 hover 去改槽位 5（`primary`），
同屏所有引用 primary 的色块会一起变色。槽位 12–16 就是为此预留的。

| 槽位 | 含义 |
| ---: | :--- |
| 0 | 无（恒透明，描边图元的填充槽用它） |
| 1 | 阴影色（引擎的 `SHADOW_TOKEN` 固定槽） |
| 2 | 页面背景 |
| 3 | 面板底色 |
| 4 | 面板边框 |
| 5 – 9 | primary / secondary / success / warning / danger |
| 10 / 11 | 正文 / 次要文字 |
| 12 / 13 | primary hover / pressed（按钮状态底色） |
| 14 | on-primary（按钮文字） |
| 15 | focus ring（焦点环） |
| 16 | disabled fill（禁用底色） |

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

---

**相关文档**：[Uber-Shader 与双通道深度](../01-graphics/uber-shader.md) ·
[解析解阴影](../01-graphics/shadow.md) · [工业级文字渲染](../01-graphics/text.md)

**返回**：[06-tooling](README.md) · [文档索引](../README.md)
