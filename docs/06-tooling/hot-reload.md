# 代码级热重载与双模状态迁移

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-host/src/lib.rs`、`crates/unbound-abi/src/plugin.rs`、`crates/app-guest/src/lib.rs` |
> | **关键类型** | `GuestHost`、`GuestPluginVTable`、`HostAllocatorVTable`、`ByteBuffer`、`StateHandoff`、`StateSchema` |

## C-ABI 物理隔离边界

`Host`（持有窗口、Vulkan 实例、显存池与主循环）与 `Guest`（编译为 `cdylib` 动态
库的业务逻辑）之间严格通过 `#[repr(C)]` 稳定函数指针虚表通信。当前 ABI 版本为
**`ABI_VERSION = 4`**：

```rust
#[repr(C)]
pub struct ByteBuffer {
    pub ptr: *mut u8,   // 宿主分配
    pub capacity: u32,  // 容量
    pub len: u32,       // 实际写入长度
}

#[repr(C)]
pub struct GuestPluginVTable {
    pub abi_version: u32,
    pub struct_size: u32,      // 本结构体字节数
    pub state_schema_hash: u64, // 编译期计算的 State 内存布局指纹
    pub on_load: unsafe extern "C" fn(host_alloc: *const HostAllocatorVTable),
    pub on_unload: unsafe extern "C" fn(),
    pub init_state: unsafe extern "C" fn() -> *mut core::ffi::c_void,
    pub drop_state: unsafe extern "C" fn(state: *mut core::ffi::c_void),
    pub adopt_state: unsafe extern "C" fn(state: *mut core::ffi::c_void),
    pub migrate_export: unsafe extern "C" fn(state: *mut core::ffi::c_void, out: *mut ByteBuffer),
    pub migrate_import: unsafe extern "C" fn(buf_ptr: *const u8, buf_len: usize) -> *mut core::ffi::c_void,
    pub reduce: unsafe extern "C" fn(state: *mut core::ffi::c_void, action: *const RawAction),
    pub build_view: unsafe extern "C" fn(state: *mut core::ffi::c_void, ctx: *mut HostRenderContext),
    pub snapshot: unsafe extern "C" fn(state: *mut core::ffi::c_void, out: *mut ByteBuffer),
}
```

### 结构体尺寸协商：先读头，再拷贝

宿主**不能**一上来就把 guest 的整个虚表结构体拷过来。老 ABI 的 guest 虚表更小，
先拷贝就等于在版本检查有机会拒绝它之前，读越界内存——"拒绝不兼容的 guest 而不是
弄坏堆"这件事就被架空了。

于是虚表头两个 `u32`（`abi_version` 在 offset 0，`struct_size` 在 offset 4）被单独
定义成一个 `#[repr(C)]` 的 `VTableHeader`：宿主先只读这两个字段，校验通过之后才把
整个虚表拷过来。

> ⚠️ **约束**：往虚表**头部**加字段时，`VTableHeader` 必须同步更新，否则尺寸协商
> 读到的是错位的值。

`HostRenderContext` 同理也带 `struct_size`：宿主可能比 guest 老，上下文结构体可能
已经变大。guest 侧的 `is_connected()` 会检查 `struct_size >= size_of::<Self>()`，
否则读取越界字段就是未定义行为。

### `ByteBuffer` 取代 `*mut Vec<u8>`

`Vec<u8>` 不能过 ABI：它不是 `#[repr(C)]`，布局不在 Rust 的稳定性保证内，而 guest
恰恰是最可能用不同工具链编译的那一方。改用上面那个 `#[repr(C)]` 的 `ByteBuffer`。

协议是**两段式**的：宿主先给一个 `capacity = 0` 的空缓冲问"你需要多少字节"，guest
把所需长度写进 `len` 而不写数据；宿主据此分配真正的缓冲，再调一次让它填充。

这样 guest 从不参与宿主的分配器，宿主用自己的分配器释放，两边不打架。

## Host 全局分配器统一劫持

在 `app_guest` 动态库中，通过自定义 `#[global_allocator]` 将 Guest 内部所有的
Rust 堆内存分配（`Box`, `Vec`, `String`, `Arc`）全部转发至
`HostAllocatorVTable` 指向的宿主进程唯一堆分配器。

这彻底杜绝了 Windows 下不同 DLL 拥有独立 CRT 堆导致的跨边界释放崩溃（Heap
Corruption）。

## 挂载时序：`on_load` 必须最先，`on_unload` 必须调用

重载时的顺序是有讲究的：

1. **新 guest 的 `on_load` 最先调用**。guest 的 `#[global_allocator]` 在
   `HOST_ALLOC` 为 null 时会回落到 `System`，于是新 guest 的 state 由 `System`
   分配，将来却由宿主的分配器释放——**跨分配器释放**。把 `on_load` 提到最前面，
   新 guest 分配任何东西之前，宿主的分配器就已经接管了。
2. **旧 guest 的 `on_unload` 在卸载前调用**。旧 DLL 的 `on_unload` 以前在重载时
   根本不跑，它持有的东西永远不释放。现在 `Migrate` 与 `Fresh` 两个分支都会调用它。

## Schema Hash 双模状态迁移

`impl_schema_hash!` 在编译期计算业务 `State` 结构体所有字段的名称、类型与偏移量
哈希值 `state_schema_hash: u64`。嵌套类型也要参与：`Vec<Row>` 永远是 24 字节，
**改 `Row` 的布局不会改变 `State` 的哈希**，宿主会以为布局没变而走零拷贝通道，
于是用 32 字节的 stride 去读 40 字节 stride 分配的缓冲区。宏的 `nested: [Row]`
分支把嵌套类型自己的指纹混进来，堵住这个洞。

```rust
unbound_abi::impl_schema_hash!(
    State,
    0x5542_4f55_4e45,
    nested: [Row],
    "count", "i64", 0,
    "title", "String", 8,
    "rows",  "Vec<Row>", 32,
);
```

### 布局必须是固定的：`repr(C)`

指纹相等意味着"宿主可以把旧 state 原封不动交给新 guest"。这只在两次构建的**字节
布局一致**时才成立，而默认的 `repr(Rust)` 不给这个保证：编译器可以重排字段（实测
它把 `State::count` 挪到了 offset 48），并且换一版编译器可能排得不一样。指纹是按
字段清单手写的，**检测不出这种漂移**——它会为两个布局不同的 state 报"兼容"。

所以业务 `State` 必须用 `#[repr(C)]` 固定布局，`app-guest` 的 `State` 与 `Row` 都是
如此。字段偏移量则由编译期断言钉死，写错直接编译失败：

```rust
const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(State, count) == 0);
    assert!(offset_of!(State, title) == 8);
    assert!(offset_of!(State, rows) == 32);
};
```

### 快速通道（`old_hash == new_hash`）

当开发者仅修改了 UI 布局、配色、动画参数或事件处理逻辑而未改动 `State`
结构体字段定义时，直接将旧 `State` 裸指针原封不动移交给新加载的 `Guest.dll`。

**热重载状态交接耗时小于 $0.01\text{ms}$（真正零拷贝）。**

### 安全容错通道（`old_hash != new_hash`）

当开发者增删或修改了 `State` 结构体字段时，自动触发旧 DLL 的 `migrate_export`
导出为**字段化信封**，再由新 DLL 的 `migrate_import` 解回。

信封格式（小端，`crates/unbound-abi/src/migration.rs`）：

```text
"UBMF" | u16 版本 | u16 字段数 | 字段数 × { u16 字段 id, u32 长度, 长度字节 }
```

每个字段的负载仍由 guest 自己用 `postcard` 编码，但**字段之间是独立的**：

- 新版本新增字段：旧负载里没有该 id，`migrate` 取 `FieldSpec::optional` 的默认值；
- 新版本删除字段：旧负载里的未知 id 被丢弃，其余字段按其 id 重新排布；
- 缺失 `FieldSpec::required` 字段：返回 `MissingField`，`migrate_import` 返回
  **null**，而不是悄悄塞一个空 seed。宿主看到 null 就回落到 `init_state()`，
  失败是可见的，而不是"用户数据全丢、宿主还以为迁移成功了"。

信封头部带版本号，未来格式升级时可以显式拒绝或转换旧版本；
`app-guest/tests/schema_evolution.rs` 用 V1/V2 两个结构体验证了"加字段取默认、
删字段丢弃、缺必填字段报错"三条路径。

另外，迁移通道走的是 `migrate_export` 而不是 `snapshot`：前者是"导出给下一代"的
语义，guest 可以在导出时按新 schema 重塑自己的状态；`snapshot` 仍保持单块
`postcard`，供调试工具读取当前状态。

### 交接结果的显式建模

`StateHandoff` 把交接结果表达为数据：

- `ZeroCopy`——指纹相同，旧指针原封不动移交；
- `Migrate`——指纹不同，走 `migrate_export`/`migrate_import` 往返；
- `Fresh`——没有旧状态（首次加载）。

这让"这次热重载走了哪条路"可以被断言，而不是靠观察行为猜测。

## 影子副本与链接策略

- Windows 强制配置 `lld-link`（或 `radlink`），Linux 配置 `mold`；开发期配置
  `split-debuginfo = "unpacked"` 与 `debug = "line-tables-only"`。
- 内置文件监听器检测到 `guest.dll` 编译完成后，先等待所有后台 Rayon 任务栅栏
  清空，随后自动复制为带递增序号的 `guest_gen_N.dll` 副本并调用 `LoadLibraryW`
  重新挂载。

影子副本这一步是必须的：Windows NTFS 对已加载的二进制文件和 `.pdb` 符号文件有
写锁定，直接覆盖会失败。`GuestHost::reload` 因此先复制再加载。

副本的生命周期由宿主管着：每个影子副本都被记在 `GuestHost` 里，卸载时删掉。否则
一次开发会话里的几百次重载，会在临时目录里留下几百份 DLL 副本。

---

**相关文档**：[编译时效与依赖分层](../05-agent/layering.md) · [调试工具链](debugging.md)

**返回**：[06-tooling](README.md) · [文档索引](../README.md)
