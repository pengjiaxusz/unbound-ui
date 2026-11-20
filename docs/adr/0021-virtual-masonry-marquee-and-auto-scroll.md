# ADR-0021：`VirtualMasonry` 跨滚动橡皮筋拉框多选与边缘自动滚屏

- **状态**：已采纳（对应 `GAP-UI-007`）
- **影响 crate**：`unbound-widgets`、`unbound-app`

## 背景

`crates/unbound-widgets/src/virtual_masonry.rs` 当前仅提供按列数与宽高比排布的视口裁剪发射，不维护框选状态。桌面文件浏览器在数万条目虚拟滚动下必须支持“按下拖拽拉框 + 拖至上下边缘自动滚屏 + Ctrl/Shift 增量选区”。

## 决策

1. **纯值状态机下沉至 `unbound-widgets::virtual_masonry`**：
   - 新增 `MarqueeSelectMode { Replace, Toggle, Add }` 与 `VirtualMarqueeState`（记录内容坐标系下的 `anchor_content: Vec2`、`current_content: Vec2`、`active: bool`、`mode: MarqueeSelectMode`）；
   - 将拉框锚点固定在**内容坐标系**（含滚动偏移 `scroll_y`）而非视口坐标系，从而在边缘自动滚屏时选区自然延伸覆盖经过的所有条目。
2. **边缘自动滚屏速度曲线**：
   - 新增 `pub fn compute_edge_autoscroll(viewport: Rect, pointer: Vec2, edge_zone_px: f32, max_speed_px_per_sec: f32, dt: f32) -> f32`：指针进入顶/底边缘热区（默认 36px）时按线性深度比例产出每帧滚动增量。
3. **几何相交与图元发射**：
   - 新增 `pub fn hit_items_in_marquee(item_rects: &[Rect], marquee_content: Rect) -> Vec<usize>`；
   - `VirtualMasonryDesc` 增补可选 `marquee_viewport_rect: Option<Rect>` 字段，活跃时以半透明 `Token::ACCENT` 填充 + 1px 描边绘制在条目层之上，不改变未激活时的图元稳定性。

## 后果

- 框选几何、滚屏延伸与 Ctrl/Shift 语义均可在 `unbound-headless` 中零 GPU 精确回归验证。
