# 架构：input-and-a11y
状态：已落地（含统一逻辑父链冒泡）
路径：`crates/unbound-input/`、`crates/unbound-a11y/`、`crates/unbound-app/src/lib.rs`
入口：`crates/unbound-input/src/lib.rs::pick_stack`、`crates/unbound-app/src/lib.rs::AppRuntime::dispatch_bubbling`、`crates/unbound-a11y/src/lib.rs::A11yNode`
守卫：`cargo test -p unbound-input` / `cargo test -p unbound-a11y` / `cargo test -p unbound-app --test event_bubbling` / `cargo test -p unbound-platform --test a11y_roundtrip`

## 范围
- [负责] 双层拾取（UI 树 + 画布空间）、指针捕获、Portal 浮层（`PORTAL_LAYER`）深度排序、统一逻辑父链冒泡（`Propagation::{Bubble, Stop}`）、焦点管理/漫游、光标栈、文件拖拽与 AccessKit 语义节点
- [不负责] OS 原生窗口消息与 IME 底层转发 -> 委托给 `crates/unbound-platform/`

## 拓扑与流程
- 拾取链路：`PointerEvent` -> `pick` / `pick_stack` 沿 z-index / `PORTAL_LAYER` 逆序命中测试（支持 `rounded_rect_sdf` / `point_in_polygon`） -> `PointerState` 捕获与 `CursorStack` 确定性解析
- 事件传播分层：
  - 高频连续事件（`PointerMove`、`DragStart`、`Drag`）：单目标直达或由 `PointerState` 独占捕获，不做祖先链冒泡以保住拾取与帧预算
  - 低频离散事件（`PointerDown`、`PointerUp`、`Click`、`DoubleClick`、`ContextMenu`、`Wheel`）：统一沿 `UiWorld` 逻辑父链（`ancestors_of`）自内向外冒泡，遇到 `Propagation::Stop`（含内置 `UiWidget::reduce` 消费）或 Portal 边界（`z_index >= 1000` / `OverlayStack`）立即截断；仅非声明式裸 `WidgetOutput` 滚轮回落 `pick_stack`
- 焦点与快捷键：`AppRuntime::key` 先沿当前 `focused` 节点的逻辑父链冒泡（通过 `AppContext::event_target()` 暴露当前祖先），未被 `Propagation::Stop` 消费时再交由 `OverlayManager::handle_escape`、`FocusManager`（`FocusGroup` 焦点陷阱与 `RovingRegistry` 方向键漫游）及 `Keymap` 兜底
- 无障碍与 IME：`WidgetOutput` 产出 `A11yNode` 增量同步至 AccessKit；`unbound-text::ime_cursor_area` 计算候选框锚定矩形（实机 IME 预编辑流仍待全量打通）

## 取舍（禁止重构）
- [代价] 不引入 Web DOM 式的全阶段 Capture（隧道）回调，且对 `PointerMove`/`Drag` 禁用冒泡 -> [原因] 保留单目标 `pick` + `CursorStack` + `PointerCapture` 的微秒级性能与确定性 -> [位置] `crates/unbound-app/src/lib.rs::AppRuntime::reduce_pointer`
- [代价] Portal 浮层在逻辑树中仍归属父节点，仅渲染与拾取层逃逸至 `PORTAL_LAYER` 64 位排序键，并在 `ancestors_of` 中作为冒泡边界截断 -> [原因] 保留主题继承与 `FocusGroup` 焦点归属，同时阻断弹窗事件渗透至背景页面 -> [位置] `crates/unbound-input/src/hit.rs::PORTAL_LAYER`、`crates/unbound-app/src/lib.rs::AppRuntime::is_portal_boundary`
- [代价] 光标形状使用显式栈 `CursorStack` 按 `CursorSource` 优先级解析 -> [原因] 防止子控件移出/进入竞态覆盖父级拖拽光标 -> [位置] `crates/unbound-input/src/cursor.rs::CursorStack`

## 不变量与红线
- [禁止] 模态框或弹出菜单打开时允许 `Tab` 焦点逃逸或离散点击/滚轮事件穿透至背景层 -> [原因] 破坏无障碍与模态隔离契约 -> [守卫] `crates/unbound-app/tests/overlay_focus_trap.rs`、`crates/unbound-app/tests/event_bubbling.rs`
- [必须] 单次 Hit-Test 耗时 <= 5µs（百万实体下实测 0.8µs） -> [守卫] `crates/unbound-kpi/src/lib.rs::hit_test`
