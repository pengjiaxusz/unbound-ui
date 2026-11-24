# ADR-0037: `WidgetOutput` 局部裁剪表与复合滚动控件自洽裁剪契约

Status: accepted

## 背景

`NavigationView` 的 Left 展开态窗格须保持透明以透出窗口级云母（Mica）后景（ADR-0026），因此不能通过给固定页脚（`footer_rect`）铺实色底板来遮盖上方主菜单的滚动溢出。然而 `WidgetOutput` 此前只携带平铺的 `primitives` 与 `payloads`，不携带局部 `GpuClipNode` 表，导致 `NavigationView` 仅靠行级 AABB 相交剔除发射主菜单行，溢出到 `FooterMenuItemsTopSeparator` 以下的图元直接叠在透明页脚上；同理，`CalendarView` 等复合控件也只能依赖调用方手工在外部包裹裁剪，容易遗漏。

## 决策

1. **在 `WidgetOutput` 中引入局部裁剪表（`clips: Vec<GpuClipNode>`）**：控件可在内部自由声明局部裁剪节点并将图元的 `clip_idx` 指向该表局部索引（未裁剪仍为 `CLIP_NONE`）。
2. **合入阶段自动重定位并求交**：`SceneCollector::push` 与展台 `Canvas::control_output` 在消费 `WidgetOutput` 时，按外层 `clip_stack` 栈顶对局部裁剪节点逐个求交并追加到全局裁剪表，再重定位图元的 `clip_idx`。求交规则为：AABB 取精确矩形交集；圆角 SDF 槽位按顺序填入空闲槽（`rounded_clip_0/1`），若外层已占满 2 槽则保留外层圆角并叠加精确 AABB 交集，全程零额外渲染 Pass。
3. **WinUI 3 硬边界视口裁剪**：`NavigationView` 主菜单行图元统一绑定 `GpuClipNode::from_aabb(layout.menu_viewport_rect)`，页脚、分隔线、滚动条与浮层保持在主菜单裁剪之外；`CalendarView` 内容网格外框同样通过 `WidgetOutput::clips` 自洽裁剪，不在滚动边界引入离屏渐隐遮罩 Pass。

## Considered Options

- **仅拆分 `NavigationViewOutput` 由调用方手工 `begin_clip`**：改动虽小，但所有带固定头/尾或内部滚动视口的复合控件仍须调用方逐处配合，无法从架构上根治漏裁剪问题。
- **为滚动边界引入边缘渐变淡出（alpha fade mask）离屏 Pass**：偏离 WinUI 3 原生 `NavigationView` 硬边界 + 1px 分隔线视觉规范，并增加离屏合成开销。

## Consequences

- `NavigationView` 在所有调用方（`apps/showcase`、业务应用、`parity.rs`）中默认无溢出且 100% 保留 Mica 透底。
- 未来任何带子视口的复合控件（如固定表头表格、带页脚列表、日历面板）都可在 `unbound-widgets` 内部自洽完成裁剪。
