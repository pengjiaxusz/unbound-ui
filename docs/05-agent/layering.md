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

---

**相关文档**：[无头闭环验证](headless.md) · [热重载](../06-tooling/hot-reload.md)

**返回**：[05-agent](README.md) · [文档索引](../README.md)
