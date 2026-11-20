# 搜索覆盖层就地修复而非复用 CommandPalette 组件

Ctrl+K 搜索覆盖层的键盘导航、点击选中与排序缺陷在应用层 `GalleryState`/`Shell` 就地修复，排序直接消费 `PinyinMatch.score` 做页面与组件的全局混排；不复用 `unbound-widgets` 中已存在、仅挂在组件展厅 demo 上的 `CommandPalette`。

## Status

accepted

## Considered Options

- **接入 `CommandPalette` 组件**：它已有 `selected`、`move_selection` 与评分排序，但它只支持前缀/子串/子序列，不支持拼音与双拼；且它的 `Command` 数据源与当前页面/组件两级目录不一致，改造面更大。
- **就地修复应用层（采纳）**：保留 `PinyinMatcher` 的中英文/拼音/双拼能力，改动集中在 `search_hits` 排序、选中索引与点击分发三处；代价是与 `CommandPalette` 的交互逻辑短期并存。

## Consequences

搜索交互的真相暂时有两处：`CommandPalette` 服务组件展厅的静态 demo，应用层 `search_hits` + `Shell::key` 服务顶栏 Ctrl+K。后续若要做统一命令面板，应把应用层的评分与选中语义迁回组件，而不是让两处各自演化。
