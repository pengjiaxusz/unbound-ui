# Unbound UI

下一代高性能 GPU 驱动 UI 与无限画布引擎的 Rust 实现。

设计核心原则：**把每一帧的成本压到与场景规模无关**。

---

## 核心特性

- **性能优先 (Performance-First)**：全链路 GPU 驱动渲染，视口粗筛、64 位基数排序与间接绘制（MDI）全交由 GPU Compute 闭环，核心路径零每帧内存分配（Zero Frame Allocation）。
- **面向数据设计 (DOD)**：基于 `SlotMap` 扁平拓扑状态树与侵入式链表，彻底废弃深层指针树与引用计数，CPU/GPU 缓存行极致友好。
- **无限画布与交互引擎**：松散线性四叉树空间索引、分块相对坐标解算、亚像素物理弹簧动画、视口智能磁吸与事务化局部撤销。
- **工业级文本与曲线矢量**：`cosmic-text` 双级缓存、GPU MTSDF 距离场字形生成、三次贝塞尔自适应降阶与解析解 SDF 光栅化。
- **Agent 双模与无头自检**：亚毫秒级纯内存无头闭环（脱离窗口/GPU 跑完排版与 5 项几何体检断言），精准定位源码，专为 AI Agent 自省与人机协作设计。

---

## 快速开始

推荐使用 `just` 统一任务入口（集成跨工作树编译排队与 CPU 降级防卡顿）：

```bash
# 启动组件展台（可视验收台：查看圆角、边框、解析阴影、描边、曲线与字形）
just showcase

# 运行全链路演示程序（附带验收红线计量报告）
just gallery -- --kpi

# 运行全工作区单元测试与 GPU 集成测试
just test
```

亦可使用原生 `cargo` 命令：

```bash
cargo run -p showcase
cargo run -p gallery -- --kpi
cargo test --workspace
```

> **注意**：GPU 集成测试与窗口运行需要 Vulkan 1.3 支持；无 Vulkan 设备时相关集成用例会自动跳过。

---

## 架构与技术文档

本项目遵循严格的架构先行与分层设计原则，完整设计规范与分卷文档详见：

- **架构主索引**：[`docs/architecture/README.md`](docs/architecture/README.md)（权威规范与模块映射）
- **主题域专卷**：[`docs/README.md`](docs/README.md)（含总览、图形底座、UI 范式、画布、输入、Agent 友好、工具链与量化红线）

---

## 许可证

MIT OR Apache-2.0
