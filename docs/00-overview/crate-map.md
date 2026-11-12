# crate 地图

> **实现状态**：`已落地` —— 本表与仓库实际结构一一对应，由
> `crates/*/src/lib.rs` 的公开接口核对而来。

20 个 crate 按依赖方向分层。**上层可以依赖下层，反向依赖被物理禁止**——
这不是约定，而是由 [`app-guest` 的分层护栏测试](../05-agent/layering.md)
在 CI 中强制检查的。

## 分层

```
┌─────────────────────────────────────────────────────────────┐
│  apps/showcase           自举式 UI Gallery：壳层 + 组件沙盒         │
│  apps/gallery            全链路演示程序                       │
├─────────────────────────────────────────────────────────────┤
│  app-guest               可热重载的业务层 cdylib（纯数据）      │
├─────────────────────────────────────────────────────────────┤
│  unbound-host            Host-Guest C-ABI 热重载与状态迁移      │
├─────────────────────────────────────────────────────────────┤
│  unbound-widgets         标准控件：描述符、视觉状态、场景发射    │
├─────────────────────────────────────────────────────────────┤
│  unbound-abi             稳定 ABI：POD 描述符、Action、插件接口 │
├─────────────────────────────────────────────────────────────┤
│  unbound-world           SlotMap 扁平树、脏标记、虚拟化容器      │
│  unbound-layout          Taffy 增量排版、RelayoutBoundary       │
│  unbound-anim            物理弹簧、贝塞尔缓动、像素吸附         │
│  unbound-canvas          松散四叉树、磁吸、框选、事务化撤销     │
│  unbound-input           双层拾取、指针捕获、光标栈、文件拖拽   │
│  unbound-a11y            AccessKit 增量语义树                  │
│  unbound-theme           语义令牌、4 KiB 颜色表、三层缩放       │
│  unbound-text            塑形、双级缓存、双图集、MTSDF          │
│  unbound-vector          贝塞尔降阶、紧致 OBB、解析距离         │
│  unbound-graph           渲染图、瞬态纹理生命周期、离屏池       │
│  unbound-asset           后台解码池、暂存环、传输队列交接        │
│  unbound-platform        窗口平台层：缩放、呈现策略、延迟计量    │
├─────────────────────────────────────────────────────────────┤
│  unbound-math            基础数学：Vec2/Rect/Affine2/Color/f16  │
└─────────────────────────────────────────────────────────────┘
```

## 依赖方向表

| crate | 行数 | 职责 | 依赖 |
| :--- | ---: | :--- | :--- |
| `unbound-math` | 1337 | 基础数学类型与半精度打包 | 无 |
| `unbound-abi` | 1573 | 稳定 ABI：扁平 POD 描述符、Action、插件虚表 | `unbound-math` |
| `unbound-gpu` | 10826 | Vulkan 1.3 后端：显存池、瞬态环、剔除排序、Uber 着色器 | `unbound-math`、`unbound-theme`、`unbound-vector` |
| `unbound-vector` | 804 | 三次贝塞尔自适应降阶、紧致 OBB、解析距离求解 | `unbound-math` |
| `unbound-text` | 2448 | cosmic-text 塑形、双级缓存、双图集、MTSDF 距离场 | `unbound-math` |
| `unbound-world` | 1674 | SlotMap 扁平状态树、三级脏标记、虚拟化容器 | `unbound-math`、`unbound-abi` |
| `unbound-layout` | 854 | Taffy 增量排版、RelayoutBoundary 隔离求解 | `unbound-world`、`unbound-math` |
| `unbound-anim` | 679 | 物理弹簧引擎、贝塞尔缓动、像素网格吸附 | `unbound-math` |
| `unbound-canvas` | 2913 | 松散四叉树、智能磁吸、框选变换、事务化撤销 | `unbound-world`、`unbound-math` |
| `unbound-input` | 1798 | 双层拾取、指针捕获、确定性光标栈、文件拖拽 | `unbound-world`、`unbound-math` |
| `unbound-a11y` | 713 | AccessKit 增量语义树 | `unbound-math`、`unbound-abi` |
| `unbound-theme` | 978 | 语义设计令牌、颜色表、三层正交缩放、像素吸附 | `unbound-math` |
| `unbound-graph` | 857 | 渲染图瞬态纹理生命周期校验、毛玻璃离屏池 | `unbound-math` |
| `unbound-asset` | 1222 | 后台解码线程池、CPU 暂存环、传输队列交接 | `unbound-math` |
| `unbound-platform` | 1562 | 窗口平台层：非阻塞缩放、低延迟呈现、延迟计量 | `unbound-math`、`unbound-gpu` |
| `unbound-widgets` | 1075 | 标准控件层：ControlState、WidgetOutput、SceneCollector、WidgetHarness | `unbound-gpu`、`unbound-input`、`unbound-a11y`、`unbound-theme`、`unbound-text`、`unbound-world`、`unbound-headless` |
| `unbound-headless` | 2248 | 无头验证器：五项几何体检、`TestApp` 交互测试驱动、rkyv 时间旅行快照 | `unbound-math`、`unbound-world`、`unbound-input`、`unbound-a11y` |
| `unbound-host` | 909 | Host-Guest C-ABI 热重载、分配器劫持、状态迁移 | `unbound-abi` |
| `unbound-kpi` | 398 | 验收红线实测 | 多数 crate |
| `app-guest` | 529 | 可热重载的业务层 cdylib | `unbound-abi` |
| `apps/showcase` | 10527 | 自举式 UI Gallery：三段式壳层、21 页示例卡、Knob 调参、代码/A11y 面板、搜索与 HUD | `unbound-gpu`、`unbound-text`、`unbound-platform`、`unbound-input`、`unbound-world`、`unbound-a11y`、`unbound-anim`、`unbound-headless` |
| `apps/gallery` | 474 | 全链路演示程序（控制台测量） | `unbound-gpu` |

## 三条刻意的依赖禁令

| 禁令 | 原因 | 执行者 |
| :--- | :--- | :--- |
| `app-guest` 不得依赖图形栈 | 业务层重编译必须快，图形栈会拖慢数十倍 | `app-guest/tests/layering.rs` |
| `unbound-gpu` 不得依赖上层框架 | 渲染器必须能独立做无头测试 | 结构性保证 |
| `unbound-abi` 不得依赖 `rkyv`/`postcard` 之外的序列化库 | ABI 稳定性要求格式可预测 | 编译期约束 |

## 规模概览

| 指标 | 数值 |
| :--- | ---: |
| crate 数 | 19 + 2 个应用 |
| Rust 源文件 | 92 |
| Rust 代码行 | 约 36 300 |
| 着色器文件 | 13 |
| 自动化测试 | 568 项（debug 与 release 双跑全绿） |

---

**相关文档**：[端到端数据流](dataflow.md) · [依赖分层护栏](../05-agent/layering.md)

**返回**：[00-overview](README.md) · [文档索引](../README.md)
