# 架构：input-and-a11y
状态：部分落地
路径：`crates/unbound-input/`、`crates/unbound-a11y/`
入口：`crates/unbound-input/src/lib.rs::pick_stack`、`crates/unbound-a11y/src/lib.rs::A11yNode`
守卫：`cargo test -p unbound-input` / `cargo test -p unbound-a11y` / `cargo test -p unbound-platform --test a11y_roundtrip`

## 范围
- [负责] 双层拾取（UI 树 + 画布空间）、指针捕获、Portal 浮层（`PORTAL_LAYER`）深度排序、焦点管理/漫游、光标栈、文件拖拽与 AccessKit 语义节点
- [不负责] OS 原生窗口消息与 IME 底层转发 -> 委托给 `crates/unbound-platform/`

## 拓扑与流程
- 拾取链路：`PointerEvent` -> `pick_stack` 沿 z-index / `PORTAL_LAYER` 逆序命中测试（支持 `rounded_rect_sdf` / `point_in_polygon`） -> `PointerState` 捕获与 `CursorStack` 确定性解析
- 焦点与快捷键：`FocusManager` 处理 `FocusGroup` 焦点陷阱（模态框/浮层）与 `RovingRegistry` 方向键漫游；`Keymap` 匹配 `KeyChord`
- 无障碍与 IME：`WidgetOutput` 产出 `A11yNode` 增量同步至 AccessKit；`unbound-text::ime_cursor_area` 计算候选框锚定矩形（实机 IME 预编辑流仍待全量打通）

## 取舍（禁止重构）
- [代价] Portal 浮层在逻辑树中仍归属父节点，仅渲染与拾取层逃逸至 `PORTAL_LAYER` 64 位排序键 -> [原因] 保留主题继承与 `FocusGroup` 焦点归属 -> [位置] `crates/unbound-input/src/hit.rs::PORTAL_LAYER`
- [代价] 光标形状使用显式栈 `CursorStack` 按 `CursorSource` 优先级解析 -> [原因] 防止子控件移出/进入竞态覆盖父级拖拽光标 -> [位置] `crates/unbound-input/src/cursor.rs::CursorStack`

## 不变量与红线
- [禁止] 模态框或弹出菜单打开时允许 `Tab` 焦点逃逸至背景层 -> [原因] 破坏无障碍与键盘导航契约 -> [守卫] `crates/unbound-app/tests/overlay_focus_trap.rs`
- [必须] 单次 Hit-Test 耗时 <= 5µs（百万实体下实测 0.8µs） -> [守卫] `crates/unbound-kpi/src/lib.rs::hit_test`
