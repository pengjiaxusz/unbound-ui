# Unbound UI

下一代高性能 GPU 驱动 UI 与无限画布引擎的 Rust 实现，架构依据
[`docs/gpu_ui_v2_0.md`](docs/gpu_ui_v2_0.md) 的 V2.0 生产级定稿。

设计目标只有一句话：**把每一帧的成本压到与场景规模无关**。其余所有取舍
（面向数据布局、零每帧分配、GPU 端剔除与排序、事务化撤销、无头验证）都是
这句话的推论。

```
crates/
  unbound-math      基础数学：Vec2 / Rect / Affine2 / Color / 半精度打包
  unbound-abi       稳定 ABI：扁平 POD 描述符、动作、宿主/插件 C 接口
  unbound-gpu       Vulkan 1.3 后端：TLSF 显存池、瞬态环、GPU 剔除与排序、Uber 着色器
  unbound-vector    三次贝塞尔自适应降阶、紧致 OBB、解析距离求解
  unbound-text      cosmic-text 塑形、双级缓存、双图集、MTSDF 距离场
  unbound-world     SlotMap 扁平状态树、三级脏标记、虚拟化容器
  unbound-layout    Taffy 增量排版、RelayoutBoundary 隔离求解
  unbound-anim      物理弹簧引擎、贝塞尔缓动、像素网格吸附
  unbound-canvas    松散线性四叉树、智能磁吸、框选变换、事务化撤销
  unbound-input     双层拾取、指针捕获、确定性光标栈、文件拖拽
  unbound-a11y      AccessKit 增量语义树
  unbound-theme     语义设计令牌、4 KiB 颜色表、三层正交缩放
  unbound-graph     渲染图瞬态纹理生命周期校验与毛玻璃离屏池
  unbound-asset     后台解码线程池、CPU 暂存环、传输队列交接
  unbound-platform  窗口平台层：非阻塞缩放、低延迟呈现、延迟计量
  unbound-headless  无头验证器：五项几何体检与 rkyv 时间旅行快照
  unbound-host      Host-Guest C-ABI 热重载、分配器劫持、状态迁移
  unbound-kpi       验收红线实测
  app-guest         可热重载的业务层 cdylib
apps/
  gallery           全链路演示程序
```

## 快速开始

```bash
cargo test --workspace          # 全部单元测试与 GPU 集成测试
cargo run -p gallery            # 全链路演示
cargo run -p gallery -- --kpi   # 附带验收红线报告
```

GPU 集成测试需要一块 Vulkan 1.3 设备；没有设备时相关用例会自行跳过，而不是
失败。

## 五个里程碑

| 阶段 | 交付内容 | 落地位置 |
| :--- | :--- | :--- |
| **M1** | Vulkan 1.3 底座、TLSF 池 + Epoch 隔离回收、瞬态环、Compute 增量 Scatter、GPU 64 位基数排序、双通道 Uber 着色器、RenderDoc 钩子 | `unbound-gpu` |
| **M2** | cosmic-text 双级缓存、`Atlas_R8` + `Atlas_RGBA8`、GPU MTSDF 字形生成器、三次贝塞尔降阶与紧致 OBB、FS 解析解 SDF | `unbound-text`、`unbound-vector` |
| **M3** | SlotMap 扁平树、Taffy + RelayoutBoundary、Rayon 并行求解、环形槽位虚拟化、弹簧动画、像素吸附、主题 LUT、渲染图与毛玻璃离屏池 | `unbound-world`、`unbound-layout`、`unbound-anim`、`unbound-theme`、`unbound-graph` |
| **M4** | 松散线性四叉树、双层拾取与指针捕获、确定性光标栈、视口级智能磁吸、事务化撤销、CJK IME 候选框、富文本圆角选区、非阻塞窗口缩放 | `unbound-canvas`、`unbound-input`、`unbound-platform` |
| **M5** | AccessKit 增量语义、无头验证器（5 项体检 + `#[track_caller]` + rkyv 快照）、Host-Guest C-ABI 热重载与双模状态迁移 | `unbound-a11y`、`unbound-headless`、`unbound-host` |

## 实测验收红线

在 RTX 5070 / Vulkan 1.4 上以 `--release` 复现：

| 场景 | 预算 | 实测 |
| :--- | :--- | :--- |
| 10 万图元同屏渲染 | ≤ 1.5 ms | **0.67 ms**（1 次 MDI 绘制调用） |
| 1 万条三次贝塞尔连线 | ≤ 0.8 ms | **0.52 ms**（12.4 万段，OBB 紧致包围盒） |
| 100 万实体视口粗筛 | ≤ 0.15 ms | 见 `cargo run -p gallery -- --kpi` |
| 单次点选命中测试 | ≤ 5 µs | 见 `cargo run -p gallery -- --kpi` |
| 视口智能磁吸 | ≤ 0.05 ms | 见 `cargo run -p gallery -- --kpi` |
| 单个 RelayoutBoundary 求解 | ≤ 0.2 ms | 见 `cargo run -p gallery -- --kpi` |
| 无头单帧验证 | ≤ 1.0 ms | 见 `cargo run -p gallery -- --kpi` |
| 主题切换 | ≤ 0.05 ms，0 次重排 | **4 KiB 单次写入，0 次重排** |
| 输入到光子延迟 | ≤ 8.3 ms | 由 `unbound-platform` 逐帧计量 |

GPU 时间通过 `VK_QUERY_TYPE_TIMESTAMP` 实测，差值在 64 位原始计数上求取后
再换算，避免绝对时间戳转浮点时丢失精度。

## 几个关键决策

**为什么每帧都不分配内存。** 持久驻留池由 TLSF 管理，回收走 Epoch 隔离队列：
显存只有在 GPU 时间线确认"没人再读它"之后才会被复用。帧内的临时数据走独立的
瞬态环，每帧一个槽位，溢出会显式报错而不是悄悄写坏显存。

**为什么剔除和排序都放在 GPU 上。** CPU 只做粗粒度的四叉树裁剪；逐图元的
视口判定、64 位排序键生成和基数排序都在 Compute 通道完成，最终只发出一次
`vkCmdDrawIndirectCount`。10 万图元塌缩成 1 次绘制调用。

**为什么撤销是事务而不是快照。** 每次编辑记录的是一条补丁（几何增量、文本
区间），撤销栈保存补丁而不是场景副本。文本输入在时间窗内自动合并成一条
事务。

**为什么要有无头验证器。** 几何错误在屏幕上表现为"看起来有点歪"，但在
无头环境里是一次精确的断言失败。`unbound-headless` 实现五项几何体检，
每一条都带 `#[track_caller]`，失败位置直接指向源码行。

## 许可证

MIT OR Apache-2.0
