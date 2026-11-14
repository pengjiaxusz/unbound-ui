# Unbound UI

下一代高性能 GPU 驱动 UI 与无限画布引擎的 Rust 实现。

设计核心原则：**把每一帧的成本压到与场景规模无关**。

---

## 核心特性

- **性能优先 (Performance-First)**：全链路 GPU 驱动渲染，视口粗筛、64 位基数排序与间接绘制（MDI）全交由 GPU Compute 闭环，核心路径零每帧内存分配（Zero Frame Allocation）。
- **面向数据设计 (DOD)**：基于 `SlotMap` 扁平拓扑状态树与侵入式链表，彻底废弃深层指针树与引用计数，CPU/GPU 缓存行极致友好。
- **全体系 99 款标准与高阶组件**：涵盖工作区容器、导航命令、高阶表单、富文本与数据表格、树表、多分割窗格、ANSI 终端、虚拟瀑布流与无限画布套件，全量遵循 24 状态 Stable Shape 图元恒定契约与零每帧堆分配。
- **Windows 10/11 原生深度集成**：Win32 Subclassing 无边框深色/浅色圆角阴影、Snap Layouts 贴靠菜单 (`HTMAXBUTTON`)、IME 预编辑事件流对齐、系统字体回退链、文件打开/保存原生对话框、任务栏进度与托盘通知、富剪贴板与 Vulkan TDR 掉线自愈。
- **无限画布与交互引擎**：松散线性四叉树空间索引、分块相对坐标解算、亚像素物理弹簧动画、视口智能磁吸与事务化局部撤销。
- **工业级文本与曲线矢量**：`cosmic-text` 双级缓存、GPU MTSDF 距离场字形生成、三次贝塞尔自适应降阶与解析解 SDF 光栅化。
- **Agent 双模与无头自检**：亚毫秒级纯内存无头闭环（脱离窗口/GPU 跑完排版与 5 项几何体检断言），精准定位源码，专为 AI Agent 自省与人机协作设计。
- **10 大渐进式标杆示例**：从极简计数器到 IDE 停靠工作区、无限画布节点编辑器、插件热重载与 Win11 生产力桌面外壳。

---

## 快速开始

推荐使用 `just` 统一任务入口（集成跨工作树编译排队与 CPU 降级防卡顿）：

```bash
# 启动组件展台（可视验收台：查看圆角、边框、解析阴影、描边、曲线与字形）
just showcase

# 运行全链路演示程序（附带验收红线计量报告）
just gallery -- --kpi

# 运行 4K UHD 极速压测演示
just gallery -- --kpi --4k

# 运行指定标杆示例程序 (01 ~ 10)
just example 01_hello_counter
just example 10_win11_native_desktop_shell

# 运行所有示例程序的无头自检套件
just test-examples

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

本项目遵循 `arch-align` 智能体原生架构规范，统一入口为：

- **架构主索引**：[`docs/architecture/README.md`](docs/architecture/README.md)（权威规范、任务路由与模块地图）

---

## 许可证

MIT OR Apache-2.0
