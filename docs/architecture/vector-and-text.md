# 架构：vector-and-text
状态：已落地
路径：`crates/unbound-vector/`、`crates/unbound-text/`
入口：`crates/unbound-vector/src/lib.rs::cubic_to_instances`、`crates/unbound-text/src/lib.rs::TextEngine`
守卫：`cargo test -p unbound-vector` / `cargo test -p unbound-text`

## 范围
- [负责] 三次贝塞尔自适应降阶、紧致 OBB 包围盒、SVG 路径解析、cosmic-text 文本塑形、双级缓存、亚像素相位图集与 GPU MTSDF 距离场
- [不负责] GPU 着色器提交与图集显存上传 -> 委托给 `crates/unbound-gpu/src/atlas.rs::AtlasSet`

## 拓扑与流程
- 矢量链路：`SvgPath` / 三次贝塞尔 -> `flatten_into` 自适应降阶为二次段 `QuadSegment` -> `Obb::from_quad` 生成紧致有向包围盒 -> `CurveInstance` 提交至 Uber-Shader 解析 SDF 光栅化
- 文本链路：`TextEngine`（`cosmic-text`）塑形与换行 -> 双级缓存命中查询 -> 未命中则按 `SUBPIXEL_PHASES` 光栅化或生成 `SdfBitmap`（MTSDF） -> `AtlasUpdate` 增量上传双图集（R8 覆盖率 + RGBA MTSDF）

## 取舍（禁止重构）
- [代价] 三次贝塞尔先降阶为二次段再套紧致 OBB，而非直接片元求三次方程根 -> [原因] 紧致 OBB 大幅降低长跨度斜线的过绘制（Overdraw），且二次 SDF 存在闭式解析解 -> [位置] `crates/unbound-vector/src/lib.rs::Obb`
- [代价] 文本同时维护小字号亚像素相位覆盖率图集与大字号/缩放 MTSDF 图集 -> [原因] 小字号保持 ClearType 级锐利，极端缩放（10%~6400%）不模糊不重栅格化 -> [位置] `crates/unbound-text/src/atlas.rs::GlyphAtlas`

## 不变量与红线
- [禁止] 在 6400% 等高倍画布缩放时每帧重新调用 CPU 字形光栅化 -> [原因] 破坏 MTSDF 缩放无关性能预算（<= 0.05ms） -> [守卫] `crates/unbound-text/src/glyph.rs::GlyphCache`
- [必须] 曲线拟合几何误差 <= 0.05px -> [守卫] `crates/unbound-vector/tests/svg_path.rs`
