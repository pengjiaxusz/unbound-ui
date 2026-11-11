# 显存分层与多帧并发安全

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-gpu/src/pool.rs`、`ring.rs`、`quarantine.rs`、`tlsf.rs`、`crates/unbound-graph/src/pool.rs` |
> | **关键类型** | `PersistentPool`、`TransientRing`、`EpochQuarantine`、`MemoryKind` |
> | **测试** | TLSF 分配回收、环形槽位复用、Epoch 隔离三组用例 |

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                              Vulkan 物理显存管理体系                              │
├───────────────────────────┬──────────────────────────┬───────────────────────────┤
│   持久驻留池 (TLSF Pool)   │   帧瞬态环 (Ring Buffer)  │   离屏特效池 (TexturePool) │
│ • 静态 UI / 分块画布实体   │ • 每帧相机/PushConstants  │ • 背景毛玻璃双パス模糊 RT  │
│ • 双格式多页字形图集       │ • 高频动画独立变换矩阵    │ • 图层组透明度离屏合成 RT  │
│ • Compute Scatter 增量更新│ • 瞬态框选框/对齐红线/补丁│ • 尺寸分级按帧复用池       │
│ • Epoch 延迟隔离回收队列   │ • Staging 异步上传暂存区  │ • Render Graph 自动屏障    │
└───────────────────────────┴──────────────────────────┴───────────────────────────┘
```

## 持久驻留池 (Persistent Pool)

基于 TLSF (Two-Level Segregated Fit) 在 `DEVICE_LOCAL` 显存上自管理大块连续
`VkBuffer`。存放无限画布持久实体、静态 UI 节点、裁剪节点与曲线控制点数据。

### 桶只保证下界：搜索必须核对块的大小

TLSF 的二级桶按「大小落在哪个区间」归类，因此**一个桶只保证下界**：桶里的
块可能比落在同一桶的请求还小。

如果按桶直接取链表头，就可能把一个比请求小的块交给上层。切分时
`block.size - want` 是在无符号整数上做的减法——一旦块比请求小，这个差值会
**回绕成四十亿**，随后那个"剩余块"的表头会被写到竞技场之外，变成静默的堆
破坏。

因此分配器的查找分两步：

1. 在**请求自己所在的桶**里走一遍链表，取第一个 `size >= want` 的块；
2. 桶里的块都不够大时，才去看更高的桶——更高的桶下界严格大于请求，那里的
   块一定装得下。

这条不变量由 `a_request_bigger_than_the_free_hole_is_refused` 守着：请求比
唯一的空洞还大时，分配必须**失败**，而不是把空洞撑破。

### 局部增量修补 (Compute Incremental Scatter)

当部分节点属性发生微量变化时，CPU 仅将变更的稀疏槽位索引与新数据打包写入
当前帧的瞬态环，由主渲染队列在帧首派发单次 Compute Shader 执行 Scatter 拷贝
至持久池，紧接 `VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT` 到渲染阶段的管线屏障，
严禁全量重传。

> 实现中 `scatter.comp` 采用工作组协作拷贝。早期版本用单线程串行拷贝整个
> 场景（6.4 MB 走一条 lane），单帧耗时 131 ms；并行化后降至 16 ms。

### 多帧并发安全回收 (Epoch Quarantine Free-List)

为防止双缓冲/三缓冲（Frames-in-Flight）下 GPU 正在读取第 $N$ 帧的持久池
地址、而 CPU 在第 $N+1$ 帧将其释放重分配，被删除或扩容搬迁的旧 TLSF 内存块
**禁止立即归还空闲链表**，而是推入绑定当前帧 Timeline Semaphore 值的延迟回收
队列 `pending_frees[frame_epoch]`，待 GPU 确认第 $N$ 帧执行完毕后才真正回收
复用。

## 帧瞬态环 (Transient Ring Buffer)

采用 `HOST_VISIBLE | HOST_COHERENT | DEVICE_LOCAL`（ReBAR / Smart Access Memory
优先）的按帧轮转环形缓冲。存放每帧活跃动画变换矩阵、高频鼠标指示线、框选
虚线框、增量 Scatter 补丁包与瞬态绘制参数。

环形槽位复用规则：只有当整环**绕回**一圈后，最早的槽位才可再次写入。GPU
可能仍在读旧槽位，提前复用会读到撕裂的数据。

## 离屏特效池 (Texture Pool) 与轻量渲染图 (Render Graph)

- 维护按 $2$ 的幂次或屏幕比例分级的离屏纹理对象池。
- 由轻量级 **Render Graph** 统一分析依赖并自动插入图像布局转换屏障
  (Layout Transitions)，支持背景毛玻璃（Dual-Kawase Backdrop Blur）与混合
  透明度图层组（Group Opacity）的按需申请与单帧内原地复用。
- 渲染图对瞬态纹理的存活期做校验：跨越过期的纹理引用会被拒绝，防止读回
  已回收的显存。

## 色彩管理与宽色域管线

- Swapchain 统一优先采用 `VK_FORMAT_B8G8R8A8_SRGB`（或 HDR 宽色域
  `VK_EXT_swapchain_colorspace`）。
- 全局颜色 Token 存储于线性空间（或 Oklab 感知均匀空间），确保半透明叠加、
  渐变插值与边缘抗锯齿在线性光强下完成物理正确混合，消除暗边（Dark
  Fringe）。

## 设备丢失恢复

全局捕获 `VK_ERROR_DEVICE_LOST`。保持 Rust 纯内存领域状态树完好，原子化销毁
并重建 `VkDevice`、交换链、显存池与管线，将内存状态树原地重新灌注至新持久池
并重栅格化可见字形，实现用户无感知热恢复。

---

**相关文档**：[核心 API](api-and-features.md) · [异步资源流送](../06-tooling/asset-streaming.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)
