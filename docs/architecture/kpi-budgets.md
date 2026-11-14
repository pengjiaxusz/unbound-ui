# 架构：kpi-budgets
状态：已落地
路径：`crates/unbound-kpi/`、`apps/gallery/`
入口：`crates/unbound-kpi/src/lib.rs::measure_all`、`apps/gallery/src/main.rs`
守卫：`just gallery -- --kpi` / `cargo run -p gallery --release -- --kpi`

## 范围
- [负责] 量化性能验收红线、GPU 时间戳采集与 CPU 中位数基准报告
- [不负责] 日常功能单元测试 -> 委托给各 crate `tests/`

## 拓扑与流程
- 主链路：`measure_all` 依次执行 `cull` / `cull_4k` / `hit_test` / `snap` / 曲线 / MTSDF / 排版 / 主题切换 -> `Report::to_markdown` / `to_text`
- 红线预算（含实测基准）：
  - 10 万带圆角/阴影图元：GPU <= 1.5ms，MDI DrawCall <= 4，持久池每帧 PCIe 上传 = 0B（512x512 实测 0.67ms，1 次 MDI；4K 待复测）
  - 1 万长跨度三次贝塞尔连线：GPU <= 0.8ms，拟合误差 <= 0.05px（视口局部实测 0.52ms）
  - 100 万实体空间查询：四叉树粗筛 <= 0.15ms（实测 17.5µs），Hit-Test <= 5µs（实测 0.8µs），磁吸 <= 0.05ms（实测 16.8µs）
  - `RelayoutBoundary` 局部求解 <= 0.2ms（实测 3.2µs）；主题切换 <= 0.05ms 且 0 次重排；无头单帧 <= 1.0ms（实测 544µs）

## 取舍（禁止重构）
- [代价] CPU 指标强制多次采样取**中位数**并保留原始样本，禁止取最快值 -> [原因] 消除共享机器调度抖动带来的虚假最优成绩 -> [位置] `crates/unbound-kpi/src/lib.rs::Measurement`
- [代价] GPU 时间戳差值在 `u64` 原始计数上相减后再转毫秒 -> [原因] 避免 `f32` 绝对时间戳减法精度丢失 -> [位置] `crates/unbound-gpu/src/frame.rs`

## 不变量与红线
- [禁止] 在 KPI 报告中硬编码节点数量或帧耗时常量 -> [原因] 所有计数必须源自引擎真实返回值 -> [守卫] `crates/unbound-kpi/src/lib.rs::Report`
- [必须] 稳态指标从第 2 帧起采样 -> [原因] 剥离首帧全量缓冲上传开销 -> [守卫] `crates/unbound-kpi/src/lib.rs`
