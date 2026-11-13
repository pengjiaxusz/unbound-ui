# 编译时效与依赖分层

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/app-guest/tests/layering.rs`、`crates/unbound-abi/src/{desc,action,plugin,schema}.rs` |
> | **实测** | 业务层增量编译 282 ms（预算 800 ms） |

## Workspace 物理分层与依赖下沉

| 层 | 内容 | 编译期行为 |
| :--- | :--- | :--- |
| `engine_core` 基础库 | Vulkan 1.3 底座、TLSF 显存池、Taffy、`cosmic-text`、AccessKit、松散四叉树 | 编译一次，业务开发期长期静态缓存 |
| `app_guest` 业务层 | 轻量纯数据结构定义（POD）、`Action` 枚举、声明式构建器接口 | 零重型宏、零图形库依赖 |
| `unbound-app` 运行时 | `Application` 契约、事件归约、场景差分 | 只依赖组件层，不接触窗口 / 显存；可无头测试 |

物理分层的意义在于**编译缓存的失效范围**：改一处业务逻辑只会让 `app_guest`
与它依赖的少数几个 crate 失效，图形栈原封不动。

### 护栏测试

`crates/app-guest/tests/layering.rs` 在 CI 中检查 `app-guest` 的依赖树，**禁止**
它引入图形栈或重型序列化库：

```rust
const ALLOWED: [&str; 3] = ["unbound-abi", "serde", "postcard"];
```

这不是代码评审约定，是会失败的测试。

## 扁平具体类型 (Flat Concrete Types)

全面禁止深层嵌套的泛型 Typestate 视图树（如 `VStack<HStack<Button<...>>>`），
统一采用扁平描述符结构体与轻量 `NodeId` 句柄。

这杜绝了 Rust 泛型单态化（Monomorphization）代码膨胀与 LLVM 优化瓶颈——
嵌套泛型会让每个实例化组合都生成一份代码。

## 正交极简 API (Orthogonal Minimal API)

每个布局属性、样式约束与交互行为**仅提供唯一一种正交声明方式**，拒绝语法糖
宏，最小化 API 表面积。

这条约束直接针对大模型生成代码时的 API 幻觉：只有一种写法，模型就不可能
"记错"另一种。

## ABI 稳定性

`unbound-abi` 定义了扁平 POD 描述符、`Action` 枚举与插件虚表。业务层的状态
结构体布局指纹由编译期宏 `impl_schema_hash!` 计算出的 `state_schema_hash` 记录，
是[热重载](../06-tooling/hot-reload.md)双模迁移的依据。

该指纹只在状态结构体布局**固定**（`#[repr(C)]`）时才有意义：默认的 `repr(Rust)`
允许编译器重排字段，两次构建可能排得不一样，而手写的指纹检测不出这种漂移。

## 组件目录与编译期 Showcase 门禁

`unbound-widgets` 的业务组件不再手写零散 `pub mod`，而是由 `define_widget_catalog!`
宏统一声明（`crates/unbound-widgets/src/lib.rs`）。宏一次性产出：

1. 全部 56 个业务组件的 `pub mod` 导出（`diff` / `primitive` / `catalog` 是基础设施，不入目录）；
2. **无 `#[non_exhaustive]`** 的穷举枚举 `WidgetCatalogId` 及 `ALL` / `COUNT` /
   `module_name()` / `from_index()`；
3. 见证令牌 `WidgetShowcaseWitness`：构造函数 `pub(crate)`，**应用层无法伪造**，
   只有 crate 内部跑通真实发射器才能产出。

由此形成三级编译期强约束：

| 级 | 位置 | 约束 |
| :--- | :--- | :--- |
| 目录 | `define_widget_catalog!` | 新增组件模块即新增枚举变体 |
| 探针 | `catalog::probe_widget`（`#[deny(unreachable_patterns)]`，无 `_ =>`） | 每个变体必须调用**真实** `emit_*` 发射器并产出见证；漏写直接 `E0004` |
| 体检 | `tests/stable_shape.rs` | 对 `WidgetCatalogId::ALL` × 24 种 `ControlState` 断言图元数恒定（Stable Shape / R4），并断言交互组件悬停必有差异 |

`catalog::probe_widget(id, state)` 为每个组件构造最小合法描述符、调用其真实发射器，
返回 `CatalogProbe { witness, primitives }`。它既是 Stable Shape 全量体检的底座，
也是 Showcase 覆盖率门禁（`apps/showcase` 侧 `showcase_entry_for_widget` 穷举映射）
的防伪锚点：任何「组件已定义却未在展台展示」或「手绘矩形冒充组件」都会在编译期或
无头测试中被拒绝。

### 全输出发射器与组件画廊

`catalog::emit_widget(id, node, state, rect, glyphs)` 是同一穷举匹配的**全输出形态**：
它把组件摆放在调用方给定的 `rect`（_overlay 组件同时以其为放置视口_）、绑定真实
`node` 身份、用调用方整形的字形渲染主标签，返回完整 `WidgetOutput`（图元 + payload +
热区 + 语义节点）。`probe_widget` 委托给它，因此**编译期穷举门禁只有一处源头**。

`apps/showcase` 的「Components」分类据此挂载**组件画廊**：`pages/components.rs` 用
`define_showcase_components!` 宏从单一 id 列表同时生成 56 张示例卡与穷举函数
`showcase_entry_for_widget`，每张卡调用 `emit_widget` 并经 `Canvas::widget_output`
登记到当帧 `emitted_widgets: FxHashSet<WidgetCatalogId>`。由此 S2.4 门禁分两半落地：

| 半 | 位置 | 约束 |
| :--- | :--- | :--- |
| 编译期 | `pages/components.rs` 的 `const _: () = { … }` | 遍历 `WidgetCatalogId::ALL`，断言每个变体都被其映射页面实际挂载；新增变体未入画廊即编译失败 |
| 运行时 | `apps/showcase/tests/shell.rs` | 滚动画廊全页（卡片离屏即被裁剪），并集每帧 `emitted_widgets`，断言 100% 覆盖 `WidgetCatalogId::ALL` |

手绘冒充者不会向 `emitted_widgets` 插入 id，因此无法通过运行时半区；新增组件未接真实
发射器则在 `emit_widget` 处 `E0004`。两半合起来保证「目录里的每个组件都被真实发射器画过」。

---

**相关文档**：[无头闭环验证](headless.md) · [热重载](../06-tooling/hot-reload.md)

**返回**：[05-agent](README.md) · [文档索引](../README.md)
