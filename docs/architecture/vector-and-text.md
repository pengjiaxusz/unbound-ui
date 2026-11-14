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


## 阶段 1 扩展能力（WP3：中英文拼音检索引擎与矢量图标库）

* **全字库全拼 + 6 套双拼 + 全拼/双拼首字母运行时可调检索引擎（`unbound_text::PinyinMatcher`）**：
  * **全量字库覆盖**：通过编译期生成的 `u16` 静态音节索引表（`pinyin_data.rs`）以 `O(1)` 零堆分配覆盖全部 20,992 个 Unicode CJK 基本汉字（`U+4E00..=U+9FFF`）、6,592 个扩展 A 区汉字（`U+3400..=U+4DBF`）、`〇`（`U+3007`）及 200+ 常见多音字全读音表（`CJK_POLYPHONES`）；
  * **四维检索与多双拼方案**：同时支持全拼、双拼、全拼首字母与双拼首字母缩写匹配；内置 6 套主流双拼方案（`ShuangpinScheme::{Xiaohe, Natural, Microsoft, Sogou, Ziguang, SmartAbc}`），**默认开启小鹤双拼（`ShuangpinScheme::Xiaohe`）**；
  * **运行时动态调整**：支持通过 `set_global_shuangpin_scheme` / `set_global_pinyin_config` 全局原子无锁切换，或通过 `PinyinMatcher::with_config` / `set_scheme` 实例级调整，并返回综合得分与合并后的字符高亮区间 `Vec<(usize, usize)>`；
  * **英文误命中抑制**：大小写不敏感的直接子串匹配优先级最高；离散子序列首字母缩写仅在至少一次命中来自汉字读音首字母时才成立，且英文直接命中只取词首字母，因此 `Accordion` 之类的纯英文单词不会被逐字母拆成「缩写」而误命中任意长英文文本，中英文混合查询仍可跨分隔符命中。
* **内置 Material 矢量图标与 22 类文件图标（`unbound_vector::icons`）**：`MaterialIcon`（32 款核心 UI/调试器图标）与 `FileTypeIcon`（覆盖 `KTX2`、`DDS`、`OBJ`、`FBX`、`GLTF`、`HLSL`、`GLSL`、`SPIRV`、`RDC` 等 22 类专业资产扩展名）均以 `24×24` 规范 SVG 路径定义，直接由 `parse_svg_path` 编译为 GPU 解析曲线。
