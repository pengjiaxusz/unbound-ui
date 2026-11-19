# ADR 0017：彩色字形由 DirectWrite 层合成，并以不含 swash 的构建为验收锚点

- 状态：已采纳
- 日期：2026-11-20
- 关联：issue #53（原「待决策」小节）、子工单 #86 / #87 / #88、ADR 0013（文字抗锯齿仅灰度）、矢量与文本架构文档、`GLOSSARY.md`（彩色层 / 彩色簇）

## 背景

issue #53 把「彩色 Emoji 完整走 DirectWrite」列为 `deferred`，理由是"收益不是画质，而是依赖收敛"，成本估为三百到五百行。两条前提在本机实测后都需要修正。

**成本被高估。** `seguiemj.ttf` 的 `COLR` 确实是 version 1（画笔树），按原计划要写解释器；但 `IDWriteFactory4::TranslateColorGlyphRun` 会自行把 COLRv1 摊平成实色矢量层再返回：

| 字形 | 层数 | 层格式 | `runColor` |
| :--- | ---: | :--- | :--- |
| 😀 U+1F600 | 6 | `TT\|COLR` | alpha 恒为 1 |
| 🚀 U+1F680 | 7 | `TT\|COLR` | alpha 恒为 1 |
| 🧑 U+1F9D1 | 16 | `TT\|COLR` | alpha 恒为 1 |

零个 `COLR_PAINT_TREE`、零个 SVG、零个 PNG 层，所有层的 `baselineOrigin` 都是 (0,0)。本机也没有安装 CBDT / sbix / SVG 彩色字体。

**"依赖收敛"这个收益不成立。** `cosmic-text` 是塑形的非可选依赖，`backend-swash` 只是它的一个 feature；摘掉 swash 不会让主干依赖消失，只少一个传递 crate。而且非 Windows 平台上 swash 是唯一的光栅器，本来就不可能摘。

**真正的缺口在构建矩阵。** `default = ["backend-native"]`，`backend-native = ["backend-directwrite"]`，而 `color_glyphs` 只在 `backend-swash` 下为 `Available`。于是 `unbound-app` 与 `unbound-widgets` 的默认构建里彩色 Emoji 一条路径都没有，只有开 `full` 的展台看得到彩色——同一个框架在不同构建里给出两种观感。

另有四条实测事实直接约束实现：

- **层会跨字形**。把 👨‍👩‍👧 的五个码位当成一个 run 喂进去，首个层的 `glyphCount` 是 5。合成单元不能是单字形，否则会裁掉跨界层。
- **`GetGlyphImageFormats` 不可用**。本机对*所有*字形（包括 `A`）都返回 `NONE(0x0)`，无法据此判断"有没有彩色层"或"有没有轮廓"；唯一可靠判据是 `TranslateColorGlyphRun` 是否返回 `DWRITE_E_NOCOLOR`。
- **基字形有轮廓**。😀 的覆盖率包围盒是 34×35，非空。所以单色路径不是空白，而是**实心剪影**——这与 swash 侧"彩色位图取亮度"是两张不同的图。
- **默认字体只有一个调色板**。`CPAL` 为 v0，1 个调色板、64418 个色条目（COLRv1 把渐变摊进了调色板）。

## 决策

1. **立项理由与验收锚点**：这件事的目的是给不含 swash 的构建补上彩色路径，不是"画质更好"。验收标准是 `--no-default-features --features backend-directwrite` 的构建里彩色 Emoji 落在 RGBA8 图集、且运行期报告不记回退。
2. **合成单元是彩色簇**：一簇产出恰好一个四边形与一个图集条目。`GlyphKey` 增加簇标识维度（簇内字形 id 的哈希），单字形路径该维度取 0。
3. **彩色簇锁相位 0**：四边形原点取 `round(pen.x)`，不是 `quantize_x` 返回的 floor 原点，误差上界 0.5 px。理由是层数——🧑 单字形 16 层，家庭序列更多，8 相位会把 analysis 次数与 RGBA 显存同时乘 8。
4. **回退逐簇，不逐 run**：按 `EmojiFallback` 设置走；每簇记一次回退，状态栏可展开到"哪个簇、因为什么"。跨簇的层整层丢弃而不裁切——裁切会画出半对半错的图；丢弃后包围盒为空则该簇判为不可合成。
5. **`Mono` 统一为基字形剪影**：DirectWrite 侧已经是这条路（`mono = true` 时直接取基字形覆盖率），不用动；swash 侧的目标是改掉现在的"彩色位图取亮度"，按第 7 条另开工单。决定性理由不是观感：`Mono` 同时是彩色失败时的兜底（`EmojiFallback::Mono`），而"取亮度"必须先有彩色层——彩色层拿不到时这条路自相矛盾。没有基轮廓的纯位图字体落占位框并记账。
6. **RGBA 图集继续使用直通 alpha**：合成内部用预乘做 over，落盘前除回 alpha。`uber.frag` 的 `PRIM_GLYPH_COLOR` 分支直接取 `t.rgb`、混合用 `SRC_ALPHA / ONE_MINUS_SRC_ALPHA`，整条管线假定直通；改预乘要同时动 shader 与所有 RGBA 生产者，本轮不值。
7. **本期只做矢量层**：位图层（CBDT / PNG / JPEG）与 swash 侧剪影改造各开独立工单。本机没有任何位图或 SVG 彩色字体，位图层写出来无法验收；swash 剪影要绕开 `SwashCache`、自行光栅化基轮廓，属于另一条光栅入口的体量。期间 swash 的 `Mono` 仍是亮度，必须在报告里标为已知差异。
8. **调色板只透传索引 0**：`GlyphKey::palette_index` 真正写进键（字段已存在但一直取 0）；能力查询读 `GetColorPaletteCount()`，为 1 时返回"字体只提供一个调色板"并灰化该维度。不做双调色板 UI——没有字体能验。
9. **`EmojiFallback::Swash` 在无 swash 构建里按 `Mono` 执行**：设置默认值随能力翻转（lean 构建的默认回退是 `Mono`）；显式写入 `Swash` 时运行时按 `Mono` 执行并记一次回退，原因写"此构建未包含 swash"。不静默忽略——否则状态栏的"实际生效"会再次失真。
10. **验收以无头断言为主**：层数、合成包围盒、`emoji_path`、回退计数，另加一条色度统计（彩色 Emoji 的非灰像素占比有下限，证明真的上色而不是画成剪影）。不进截图基线——字形随系统字体版本漂移，基线会长期红，那不是回归。

## 备选方案

- **维持 `deferred`**：等 #53 的触发条件 1（"swash 成为唯一无法替换的重依赖"）成立。它并不成立，而"默认构建没有彩色 Emoji"是当下就在发生的问题，不会自己消失。
- **8 相位**：位置精度最好（误差 ≤ 1/16 px），代价是每簇 8 份 RGBA 与 8 倍 `CreateGlyphRunAnalysis`；按实测层数，家庭 Emoji 首次出现会做到数百次 analysis。彩色字形是大色块，0.5 px 的定位误差肉眼不可辨，不值这个代价。
- **逐 run 回退**：整段观感一致，但一个簇画不出来会让整段文字变单色，定位成本高。
- **静默空白**（现状）：最差，因为"这个 Emoji 不见了"和"这个 Emoji 本来就是空白字形"在报告上无法区分。
- **`Mono` 取合成后亮度**：与 swash 现状一致，但作为兜底路径存在循环依赖，且要付出整簇合成的成本才能画一个单色图标。
- **预乘 alpha**：对彩色图集将来参与缩放采样更正确（直通 alpha 在双线性过滤时会向透明黑渗出），但要改 shader 混合与全部 RGBA 生产者；等真有缩放采样的需求再立独立决策。

## 后果

- Windows 上不再有"彩色字形必须回退 swash"这条规则。swash 仍留在依赖树里（塑形与非 Windows 平台需要），但不再是彩色 Emoji 的唯一来源。
- 彩色簇首次出现要串行做数十次 `CreateGlyphRunAnalysis` 再加一次位图合成，之后命中图集；相位 0 让这个成本不乘 8。
- `EmojiPresentation::Mono` 的语义目标是基字形剪影。DirectWrite 侧已经满足（`mono = true` 时直接取基字形覆盖率），swash 侧在独立工单落地前仍是"彩色位图取亮度"——这段时间里两个后端对同一设置会给出不同图，报告必须把这条标为已知差异，不能让面板假装一致。
- `color_directwrite` 从 `Unavailable("Requires layer traversal (deferred)")` 转为 `Available`，"呈现 = 彩色 + 光栅器 = DirectWrite"的组合解锁。面板不需要改判定逻辑，只需改悬停文案。
- 彩色路径放弃"每 ⅛ px 一份位图"的位置精度，换来彩色 Emoji 在常用字号上的一致形状与可控显存。
- 调色板维度在默认字体上永远是灰的。`palette_index` 虽然进了键，本轮却取不到 0 以外的值；真正出现双调色板字体时，接线点已经就位，不用改键结构。
