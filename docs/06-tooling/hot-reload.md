# 代码级热重载与双模状态迁移

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-host/src/lib.rs`、`crates/unbound-abi/src/plugin.rs`、`crates/app-guest/src/main.rs` |
> | **关键类型** | `GuestHost`、`GuestPluginVTable`、`HostAllocatorVTable`、`StateHandoff`、`StateSchema` |

## C-ABI 物理隔离边界

`Host`（持有窗口、Vulkan 实例、显存池与主循环）与 `Guest`（编译为 `cdylib` 动态
库的业务逻辑）之间严格通过 `#[repr(C)]` 稳定函数指针虚表通信：

```rust
#[repr(C)]
pub struct HostAllocatorVTable {
    pub alloc: extern "C" fn(size: usize, align: usize) -> *mut u8,
    pub dealloc: extern "C" fn(ptr: *mut u8, size: usize, align: usize),
    pub realloc: extern "C" fn(ptr: *mut u8, old_size: usize, align: usize, new_size: usize) -> *mut u8,
}

#[repr(C)]
pub struct GuestPluginVTable {
    pub abi_version: u32,
    pub state_schema_hash: u64, // 编译期由派生宏计算的 State 内存布局指纹
    pub on_load: extern "C" fn(host_alloc: *const HostAllocatorVTable),
    pub init_state: extern "C" fn() -> *mut core::ffi::c_void,
    pub migrate_export: extern "C" fn(state: *const core::ffi::c_void, out_buf: *mut Vec<u8>),
    pub migrate_import: extern "C" fn(buf_ptr: *const u8, buf_len: usize) -> *mut core::ffi::c_void,
    pub reduce: extern "C" fn(state: *mut core::ffi::c_void, action: *const RawAction),
    pub build_view: extern "C" fn(state: *const core::ffi::c_void, ctx: *mut HostRenderContext),
}
```

## Host 全局分配器统一劫持

在 `app_guest` 动态库中，通过自定义 `#[global_allocator]` 将 Guest 内部所有的
Rust 堆内存分配（`Box`, `Vec`, `String`, `Arc`）全部转发至
`HostAllocatorVTable` 指向的宿主进程唯一堆分配器。

这彻底杜绝了 Windows 下不同 DLL 拥有独立 CRT 堆导致的跨边界释放崩溃（Heap
Corruption）。

## Schema Hash 双模状态迁移

派生宏在编译期递归计算业务 `State` 结构体所有字段的名称、类型与偏移量哈希值
`state_schema_hash: u64`。

### 快速通道（`old_hash == new_hash`）

当开发者仅修改了 UI 布局、配色、动画参数或事件处理逻辑而未改动 `State`
结构体字段定义时，直接将旧 `State` 裸指针原封不动移交给新加载的 `Guest.dll`。

**热重载状态交接耗时小于 $0.01\text{ms}$（真正零拷贝）。**

### 安全容错通道（`old_hash != new_hash`）

当开发者增删或修改了 `State` 结构体字段时，自动触发旧 DLL 的 `migrate_export`
导出为带字段标识的自描述紧凑二进制流（MessagePack / FlexBuffers），并由新 DLL
的 `migrate_import` 结合 `#[serde(default)]` 完成字段默认值填充与结构演进。

彻底告别"修改状态结构体后热重载必崩溃"的顽疾。

### 交接结果的显式建模

`StateHandoff` 把交接结果表达为数据：`Fast`（零拷贝移交）或
`Migrated`（经过序列化迁移）。这让"这次热重载走了哪条路"可以被断言，而不是靠
观察行为猜测。

## 影子副本与链接策略

- Windows 强制配置 `lld-link`（或 `radlink`），Linux 配置 `mold`；开发期配置
  `split-debuginfo = "unpacked"` 与 `debug = "line-tables-only"`。
- 内置文件监听器检测到 `guest.dll` 编译完成后，先等待所有后台 Rayon 任务栅栏
  清空，随后自动复制为带递增序号的 `guest_gen_N.dll` 副本并调用 `LoadLibraryW`
  重新挂载。

影子副本这一步是必须的：Windows NTFS 对已加载的二进制文件和 `.pdb` 符号文件有
写锁定，直接覆盖会失败。`GuestHost::reload` 因此先复制再加载。

---

**相关文档**：[编译时效与依赖分层](../05-agent/layering.md) · [调试工具链](debugging.md)

**返回**：[06-tooling](README.md) · [文档索引](../README.md)
