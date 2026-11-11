# 架构定位与核心设计准则

> **实现状态**：`蓝图` —— 本篇是全部设计的出发点，其约束在代码中以具体形式
> 落地，参见 [crate 地图](crate-map.md) 与各模块文档的状态标注。

本项目的目标只有一句话：**把每一帧的成本压到与场景规模无关**。

其余所有取舍（面向数据布局、零每帧分配、GPU 端剔除与排序、事务化撤销、
无头验证）都是这句话的推论。下面的四条准则是推论过程中的中间结论。

## 性能绝对优先 (Performance-First)

- **核心路径零每帧内存分配（Zero Frame Allocation）**。
  帧内的临时数据走独立的瞬态环，每帧一个槽位；溢出会显式报错而不是悄悄
  写坏显存。
- **全数据连续布局且严格对齐 CPU/GPU 缓存行**（$64\text{B}$ Cache-Line
  Friendly）。`GpuPrimitiveInstance` 恰好 $64\text{B}$，占满一条缓存行。
- **全链路 GPU 驱动渲染（GPU-Driven Rendering）**：视口粗筛、排序键生成、
  基数排序与间接绘制命令组装均在 GPU 端完成。CPU 只做粗粒度裁剪。

## 面向数据设计 (DOD)

- 彻底废弃面向对象的指针树与引用计数结构，全面采用扁平结构体数组（SoA）、
  连续物理缓冲与代际索引（`SlotMap<NodeId, T>` 与 `SecondaryMap`）。
- 空间层次与动态集合内部全面采用固定数组侵入式链表（Intrusive Linked
  List），消灭嵌套 `Vec<Vec<T>>` 带来的二级堆分配与缓存未命中。

## 无锁响应式调度与超低延迟呈现

- 主线程仅处理轻量 UI 意图、输入路由与瞬态更新；排版求解、字形轮廓提取、
  资源解码与后台运算均通过任务系统（Rayon）完全并行化。
- 引入交换链完成等待信号量与独立呈现通道，将输入到光子延迟（Input-to-
  Photon Latency）严格控制在 $1$ 帧以内（$120\text{Hz}$ 下小于
  $8.3\text{ms}$）。

## Agent 机器与人类双模友好

- 统一以 `AccessKit` 语义树作为人机交互与 AI Agent 自省的一级输出底座，
  无需维护第二套自定义 AST。
- 提供小于 $1\text{ms}$ 的纯内存无头闭环验证（Headless Verification）、
  内置 $5$ 项几何体检断言与小于 $600\text{ms}$ 的 C-ABI 热重载开发闭环。

## 这四条准则的代价

约束不是免费的，架构中显式承担了下列代价：

| 代价 | 换来什么 | 落地位置 |
| :--- | :--- | :--- |
| 放弃 OO 继承与引用计数 | 缓存友好、无 GC 停顿 | [`unbound-world`](../02-ui/dod-model.md) |
| 组件回调只能是可序列化枚举 | 状态可迁移、可回放 | [`unbound-abi`](../05-agent/layering.md) |
| 禁用 FS `discard` | Early-Z 管线不被破坏 | [`unbound-graphics/clipping`](../01-graphics/clipping.md) |
| 撤销存补丁而非场景副本 | 百万节点下单步撤销增量仅数 KB | [`unbound-canvas`](../03-canvas/interaction.md) |
| 图元必须能解析求距 | 缩放到 6400% 仍不重采样 | [`unbound-vector`](../01-graphics/curves.md) |

---

**相关文档**：[端到端数据流](dataflow.md) · [crate 地图](crate-map.md)

**返回**：[00-overview](README.md) · [文档索引](../README.md)
