# 架构：animation-and-theme
状态：已落地
路径：`crates/unbound-anim/`、`crates/unbound-theme/`、`crates/unbound-math/`
入口：`crates/unbound-anim/src/lib.rs::Spring`、`crates/unbound-theme/src/lib.rs::ThemeLut`
守卫：`cargo test -p unbound-anim` / `cargo test -p unbound-theme` / `apps/showcase/tests/theme.rs`

## 范围
- [负责] 物理弹簧动画、亚像素旁路、像素网格吸附、三层正交缩放（`ZoomLayers`）与 4 KiB 颜色 LUT 主题切换
- [不负责] 帧调度唤醒循环 -> 委托给 `crates/unbound-app/`

## 拓扑与流程
- 动画链路：`Spring::set_target` -> 每帧 `Spring::step` 积分（`is_settled` 达稳态后停止唤醒） -> `snap_screen` / `edge_screen_snapped` 像素吸附
- 缩放链路：`ZoomLayers`（OS DPI × 用户 UI 缩放 × 画布视口缩放）折叠为统一 `Affine2`，确保文字与 1px 边框在 10%~6400% 下零抖动
- 主题链路：`ThemeSet::palette` -> `ThemeLut` 4 KiB -> 宿主每帧比对 `ctx.theme` 后 `write_theme_lut`（0 次重排、0 次图元重建）；`Renderer::new` 只写 5 个占位槽，`SURFACE(2)` 为白、其余 507 token 透明

## 取舍（禁止重构）
- [代价] 图元实例只存 `Token` 颜色索引而非 RGBA 颜色值，通过 4 KiB `ThemeLut` 间接查表 -> [原因] 一键切换深浅/高对比度主题实现 <= 0.05ms、0 次重排 -> [位置] `crates/unbound-theme/src/lib.rs::ThemeLut`
- [代价] 弹簧动画只驱动位置/变换矩阵，不触发 `LayoutEngine` 重排（亚像素旁路） -> [原因] 保证 120FPS 动画期间无 Taffy 排版抖动 -> [位置] `crates/unbound-anim/src/lib.rs::Spring`

## 不变量与红线
- [禁止] 切换 `ThemeKind` 时标脏 `UiWorld` 布局或重传持久池图元 -> [原因] 违背 0 次重排红线 -> [守卫] `crates/unbound-kpi/src/lib.rs`
- [必须] 边框与矩形边缘输出前经 `edge_screen_snapped` 对齐物理像素网格（`snap_error == 0.0px`） -> [守卫] `crates/unbound-theme/src/snapping.rs`
- [必须] 窗口宿主首帧前上传完整 512 槽 LUT，设备重建后重写 -> [原因] 缺省 LUT 使清屏白 + 控件全透明（白屏），且不触发任何 VUID -> [守卫] `crates/unbound-app/src/window_runner.rs::sync_theme`


## 阶段 1 扩展能力（WP3：OKLCH 双产品主题预设与 7 档规范缩放）

* **感知均匀 OKLCH 色彩空间（`unbound_math::Oklch`）**：支持 `Oklch <-> Oklab <-> Linear sRGB` 无损往返转换。
* **双产品主题与 10 套品牌强调色（`unbound_theme::presets`）**：提供 `ProductPreset`（`Standard`、`Dunting` 中性暗房灰阶、`RenderDebuggerLauncher` 高对比度工程冷灰）与 `AccentBrand::ALL`（10 套 OKLCH 强调色），编译为标准 4 KiB RGBA16F LUT，切换时保持 **0 字节图元重传、0 重新布局**。
* **7 档规范 UI 缩放阶梯（`CANONICAL_SCALE_STEPS`）**：对齐 `[0.75, 0.875, 1.0, 1.125, 1.25, 1.5, 2.0]`，提供 `snap_ui_scale` 与 `step_ui_scale`。
