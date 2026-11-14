# 架构：widgets
状态：已落地
路径：`crates/unbound-widgets/`
入口：`crates/unbound-widgets/src/lib.rs::WidgetCatalogId`、`SceneCollector`、`diff`
守卫：`cargo test -p unbound-widgets --test stable_shape --test health`

## 范围
- [负责] 标准与高阶组件库（`define_widget_catalog!` 统一注册）、`ControlState`（24 种状态解析）、`WidgetOutput` 场景发射与帧差分补丁
- [不负责] 业务状态持有与异步网络 -> 委托给 `crates/unbound-app/`；底层 Vulkan 渲染 -> 委托给 `crates/unbound-gpu/`

## 拓扑与流程
- 目录生成：`define_widget_catalog!` 宏唯一声明所有组件模块、`WidgetCatalogId`（无 `#[non_exhaustive]`）及 `pub(crate)` 见证令牌 `WidgetShowcaseWitness`
- 发射链路：`*Desc` 扁平 POD 描述符 + `*State` -> `emit_*` 纯函数 -> `WidgetOutput`（`SceneCollector` 图元 + payload + 热区 + `A11yNode`） -> `diff` 局部补丁上传
- 三级门禁：`define_widget_catalog!` -> `catalog::emit_widget`（`#[deny(unreachable_patterns)]`） -> `tests/stable_shape.rs`
- payload 索引：`SceneCollector::push` 以 `self.payloads.len()` 为基址重定位每个 widget 的局部 `extra_payload_idx` -> 否则第二段文本会读第一段字形 UV（文字串叠）

## 取舍（禁止重构）
- [代价] 组件全部采用扁平 `*Desc` POD + 纯函数 `emit_*`，禁止面向对象组件实例树 -> [原因] 零虚表、零堆分配、天然支持 `diff` 与热重载 -> [位置] `crates/unbound-widgets/src/lib.rs`
- [代价] 同一组件在 24 种 `ControlState`（Normal/Hovered/Pressed/Focused/Disabled × Checked 等）下**图元数量必须严格恒定**（Stable Shape） -> [原因] 状态切换时 GPU 仅做原地局部补丁更新（0 次缓冲重建） -> [位置] `crates/unbound-widgets/tests/stable_shape.rs`

## 不变量与红线
- [禁止] 在 `emit_*` 中根据 `hovered`/`disabled` 条件增减图元个数（隐藏态须用零透明或零尺寸占位） -> [原因] 破坏 Stable Shape 恒定契约 -> [守卫] `crates/unbound-widgets/tests/stable_shape.rs`
- [必须] 新增组件必须接入 `define_widget_catalog!` 并在 `catalog::emit_widget` 实现真实发射 -> [原因] 编译期穷举门禁防遗漏 -> [守卫] `crates/unbound-widgets/src/catalog.rs`
- [例外] `title_bar` 仅提供 `TitleBarDesc` 描述符与保留节点 ID，场景由 `unbound-app` 的 `ViewBuilder` 组合既有容器/徽章/按钮生成，故按基础设施模块列入 `build.rs` 白名单，不纳入展示目录 -> [原因] 无独立 `emit_*`，纳入穷举目录会迫使伪造发射分支 -> [守卫] `crates/unbound-widgets/build.rs`
- [必须] 追加 `WidgetOutput` 时重定位全部 `extra_payload_idx` -> [原因] 着色器按全局索引访问 payload 数组，不做偏移换算 -> [守卫] `crates/unbound-widgets/src/lib.rs::payload_indices_are_rebased_for_every_widget`
