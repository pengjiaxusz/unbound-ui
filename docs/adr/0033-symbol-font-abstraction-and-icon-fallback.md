# ADR 0033：平台感知符号字体抽象与跨平台矢量图标回退

- 状态：已采纳
- 日期：2026-11-24
- 关联：扩展 ADR 0025（按钮原生矢量图标）、ADR 0028（通用豁免登记）、ADR 0032（Linux 原生后端与运行时 `Segoe Fluent Icons` 清理）

## 背景

ADR 0032 一期为打通 Linux / WSLg 闭环，清除了控件层对 `Segoe Fluent Icons` 的硬编码，并在 `crates/unbound-widgets/src/shell.rs` 中以静态单测 `runtime_widgets_contain_no_segoe_fluent_icons_hardcoding` 守住防线。但在后续基准对齐与全平台观感复核中，暴露出四类结构性矛盾：

1. **Windows 基准对齐与跨平台兼容割裂**：WinUI 3 的 `FontIcon`、`CommandBar`、`AppBarButton`、`AppBarToggleButton`、`RatingControl`、`CalendarDatePicker` 等控件依赖 Fluent 符号字形；直接回退到临时借用的 `ShellIcon`（如 `Sparkles` 替代 `AllApps`、`Package` 替代 `Save`、`Sliders` 替代 `Bold`）导致 Windows 与非 Windows 均偏离语义，迫使 `apps/showcase/src/parity.rs` 在多处堆砌 `Some("Segoe Fluent Icons")` 硬编码旁路。
2. **缺少平台感知的符号字体事实源**：`unbound-text` 已维护平台 UI/CJK 字体回退链，却没有符号字体候选链与前置可用性探测。`DirectWriteRasterizer::find_face` 在目标字体缺失时会静默落回 `Segoe UI`，直接请求 PUA 码位会画出缺字豆腐块，无法依靠渲染后端报错触发回退。
3. **跨平台矢量回退轮廓不完整或失真**：`ShellIcon::Settings` 仅有 4 个轴向齿且缺少中心轴孔，小尺寸下观感近似十字准星；常用 Fluent 图标缺少 `Save`、`Calendar`、`Bold`、`Star`、`StarFilled`、`AllApps` 等语义变体。
4. **缺少无头双路径测试接缝**：同一宿主机上无法在单测中确定性地关闭或指定符号字体，难以在 CI（含 Windows 与 Ubuntu）稳定回归「符号字形路径」与「矢量回退路径」。

## 决策

- **在 `unbound-text` 建立平台感知符号字体抽象**：
  - 在 `crates/unbound-text/src/shaping.rs` 定义并导出 Windows 符号字体候选链 `WINDOWS_SYMBOL_FALLBACKS: &[&str] = &["Segoe Fluent Icons", "Segoe MDL2 Assets"]`（Win11 优先 `Segoe Fluent Icons`，Win10 回退 `Segoe MDL2 Assets`）。
  - 新增 `SymbolFontPolicy { Auto, Force(&'static str), Disabled }`（默认 `Auto`），并在 `TextEngine` 暴露 `symbol_font_family(&self) -> Option<&str>`、`has_symbol_font(&self) -> bool`、`symbol_font_policy(&self) -> SymbolFontPolicy`、`set_symbol_font_policy(&mut self, policy: SymbolFontPolicy)`（策略变化时调用 `clear_caches()`）。
  - `Auto` 模式下：`#[cfg(windows)]` 按 `WINDOWS_SYMBOL_FALLBACKS` 依次调用 `font_family_available` 返回首个已安装字体族；`#[cfg(not(windows))]` 恒返回 `None`。
  - 同步在 `crates/unbound-text/src/directwrite.rs` 抽出 `is_symbol_font_family(family: &str) -> bool`（忽略大小写匹配 `WINDOWS_SYMBOL_FALLBACKS`），保证 Win10 `Segoe MDL2 Assets` 与 Win11 `Segoe Fluent Icons` 享受一致的 `NATURAL_SYMMETRIC` 大字号光栅化模式。
- **在 `TextCtx` 暴露统一符号字形塑形入口**：
  - 在 `crates/unbound-widgets/src/ergonomics.rs` 为 `TextCtx` 暴露 `symbol_font_family`、`has_symbol_font` 与 `shape_symbol_glyph(&mut self, glyph: &str, rect: Rect, size: f32) -> Option<Vec<GlyphQuad>>`。
  - `shape_symbol_glyph` 先通过 `symbol_font_family()` 前置判定可用性（不可用或空字形直接返回 `None`），再以 `line_height == font_size` 按测量尺寸在 `rect` 内精确居中，避免复用正文 `AlignY::Middle` 的 `-1.0` 行盒偏置；控件层全程零 `Some("Segoe Fluent Icons")` 硬编码。
- **非 Windows 或无符号字体时自动回退校准后的 `ShellIcon` 矢量几何**：
  - 保留 `ShellIcon` 跨状态图元数恒定与纯语义令牌着色契约；将 `ShellIcon::Settings` 校准为 8 齿齿轮（含 45° 对角齿与中心小圆孔），并补齐 `Save`、`Calendar`、`Bold`、`Star`、`StarFilled`、`AllApps` 等 Fluent 回退变体。
  - 基准对齐控件优先调用 `text_ctx.shape_symbol_glyph(...)` 输出系统原生符号字形；当返回 `None` 时无缝落回对应语义的 `ShellIcon` 矢量几何，彻底消除豆腐块与借用错位图标。
- **坚持零内嵌字体膨胀**：
  - 不向仓库或编译产物内嵌数 MB 的第三方符号字体文件，避免二进制膨胀与字体许可证合规风险；Windows 直接复用系统预装字体，Linux/macOS 直接走轻量解析几何回退。
- **内置 `TextEngine` 测试接缝**：
  - 通过 `TextEngine::set_symbol_font_policy(SymbolFontPolicy::Disabled)` 或 `Force(...)`，无头单元测试可在任意操作系统宿主上双向验证符号字形路径与 `ShellIcon` 矢量回退路径，且不破坏既有 `runtime_widgets_contain_no_segoe_fluent_icons_hardcoding` 静态门禁。

## 备选方案

- **仓库内嵌开源符号字体（如 Fluent System Icons TTF）**：跨平台外观最接近，但会引入数 MB 静态资产与额外许可维护成本，且与 Windows 原生 `Segoe Fluent Icons` 的 PUA 码位与度量仍存在细微偏差，无法取代系统字体完成 WinUI 3 像素级对齐。
- **全平台一律放弃符号字体、纯用 `ShellIcon` 矢量绘制**：可消除平台分支，但复杂 Fluent 字形（如 `AllApps`、`Save`、`Bold`、描边五角星等）在 1:1 像素差分中难以与 DirectWrite 光栅化结果对齐，会扩大 ADR 0028 豁免范围或迫使 `parity.rs` 长期保留硬编码旁路。
- **依赖 DirectWrite / `cosmic-text` 渲染失败后再回退**：`DirectWriteRasterizer::find_face` 缺字时会静默落回 `Segoe UI`，无法可靠区分「命中真实符号字形」与「回退到非符号字体」，必须前置基于 `fontdb` 的可用性探测。

## 影响

- `crates/unbound-text` 新增 `WINDOWS_SYMBOL_FALLBACKS`、`SymbolFontPolicy` 与 `TextEngine` 符号字体查询/策略方法；
- `crates/unbound-widgets` 新增 `TextCtx::shape_symbol_glyph`、校准后的 `ShellIcon::Settings` 与 6 个 Fluent 回退变体，首个曳光弹 `FontIcon` 率先完成端到端双路径打通；
- `apps/showcase/src/parity.rs` 逐步收敛符号字体硬编码旁路，使参考对齐与运行时控件共享同一套平台感知符号字体与矢量回退管线。
