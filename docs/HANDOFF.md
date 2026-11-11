# 交接文档：unbound-ui 缺陷修复

> 面向接手的 agent。读完这一份就能接着干，不用重跑审计。
>
> 生成时间：2026-11-11 晚。仓库：`D:\pengj\unbound-ui`，分支 `main`。

---

## 0. 三分钟上手

```bash
cd /d/pengj/unbound-ui

just check          # clippy（全局排队）
just test           # 全量单测
just q-run cargo run -p showcase -- --shot /tmp/out.png   # 无头出图
just q-run cargo run -p gallery -- --kpi                 # 六项 KPI 红线
```

**构建纪律（`AGENTS.md` 硬性要求）**：所有编译 / 检查 / 单测 / 跑分必须走
`just` 或 `just q-run <cmd>`（或 `bque run -- <cmd>`），**严禁裸跑
`cargo build` / `cargo test` / `ninja`**。`bque` 是 Windows 全局锁，
机器上已装好（`just queue-doctor` 自检）。`just fmt` = `cargo fmt --check`。

**提交纪律**：本轮所有改动在收尾时必须 commit **并 push**。提交信息走
约定式提交：`type(scope): 中文标题`，type 为
`feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert`。
有 lefthook + commitlint 钩子，格式不对会被拒。

---

## 1. 当前进度一览

### 1.1 已完成并已提交（7 个提交，全部验证过）

| commit | 内容 |
| :--- | :--- |
| `2712dd5` | 显存池越界写、四叉树丢实体、裁剪节点布局错位 |
| `bab9868` | 距离场编码、图集键、字形基线 |
| `95aee14` | 显存池隔离回收接线 + 若干越界/同步缺陷 |
| `1312d77` | 次键中止拖拽、捕获丢失无通知、NaN 弹簧永不休眠 |
| `eecc7ca` | 撤销语义、图集驱逐回滚、渲染路径零分配 |
| `4c9803c` | 选区边界、吸附排序、样式哈希、IME 锚点 |
| `e6239e1` | 去掉多余的 `mut` 限定 |

这 7 个提交对应的每个缺陷都**先写了会失败的回归测试**，再改实现。
`cargo test --workspace` 全绿，`cargo clippy --all-targets --all-features
-D warnings` 零告警，展台出图与修复前**逐字节一致**
（`docs/assets/showcase-light.png` 的 SHA256 未变），
`gallery --kpi` 六项红线全部照旧（100k 图元 0.666 ms / 1 次 draw call）。

### 1.2 未提交：ABI v4 改造（**有 1 个已知失败测试**）

工作区有 5 个文件已改但**未 commit**：

```
 M crates/unbound-abi/src/plugin.rs     ABI v4：struct_size 协商、ByteBuffer 取代 Vec<u8>
 M crates/unbound-abi/src/schema.rs     嵌套类型的 schema 哈希（hash_nested + 新宏分支）
 M crates/unbound-host/src/lib.rs       vtable 先读头再拷贝、on_load 前置、on_unload 补调用、
                                        ROOT 父节点、影子副本清理、export_state 两段式缓冲
 M crates/unbound-host/src/tests.rs      新 ABI 签名、计数器加锁消除竞态
 M crates/app-guest/src/lib.rs          ByteBuffer 写入、migrate_import 失败返回 null、
                                        Row/State 嵌套 schema 哈希
```

**状态**：`cargo test --workspace` 里**只有一个测试失败**：

```
unbound-host::tests::a_reload_with_a_new_schema_migrates_the_state
  crates/unbound-host/src/tests.rs:253
  assertion `left == right` failed:  left: 0, right: 1
```

---

## 2. 接手第一件事：修掉这 1 个失败测试

### 症状

`EXPORTED` 计数器是 0，期望 1。也就是说 **迁移路径根本没调用
`migrate_export`**。

### 根因

`crates/unbound-host/src/lib.rs:600` 的 `export_state` 辅助函数**写错了
回调**：

```rust
fn export_state(vtable: &GuestPluginVTable, state: GuestState) -> Vec<u8> {
    ...
    (vtable.snapshot)(state, &mut view);      // ← 错：迁移要用 migrate_export
    needed = view.len;
    ...
    (vtable.snapshot)(state, &mut view);      // ← 同样错
    ...
}
```

`export_state` 被两处调用：

- `GuestHost::snapshot()` —— 这里用 `snapshot` **是对的**；
- `GuestHost::reload_vtable()` 的 `StateHandoff::Migrate` 分支 —— 这里必须用
  **`migrate_export`**，因为迁移要走"导出给下一个 generation"的语义。

`app-guest` 的 `snapshot` 和 `migrate_export` 目前实现相同（都调
`write_bytes`），所以真实 guest 不会出错；但 `unbound-host` 的假 guest
故意让两者行为不同（`guest_export` 累加 `EXPORTED`，`guest_snapshot`
不累加），测试正是靠这个区分来验证"迁移真的走了 migrate_export"。

### 建议修法

给 `export_state` 加一个参数，指明用哪个回调：

```rust
fn export_state(
    vtable: &GuestPluginVTable,
    state: GuestState,
    export: unsafe extern "C" fn(GuestState, *mut ByteBuffer),
) -> Vec<u8> {
    // ... 两段式：先用 capacity=0 探长度，再用真缓冲填充
}

export_state(&vtable, self.state, vtable.snapshot)   // snapshot()
export_state(&old, old_state, old.migrate_export)   // 迁移路径
```

### 顺带要确认

`export_state` 里第一段用 `capacity: 0` + `ptr: null` 探长度，
第二段用真实缓冲填充。**第一段写进去的 `view.len` 必须是"所需长度"**，
所以 guest 侧的 `write_bytes` 在装不下时要把 `out.len` 设为需要的长度
（`app-guest` 与 `unbound-host` 的假 guest 都已这样实现）。
修完请确认 `export_state` 第二次调用后 `buffer.truncate(view.len)`
拿到的是真实写入长度。

### 还有一个待确认的隐患

`crates/unbound-host/src/lib.rs` 的 `export_state` 目前**只用于 snapshot
与 migrate**。`GuestHost::snapshot()` 返回的字节必须是 guest 状态的全量
序列化结果。`app-guest` 的 `snapshot` 转发到 `migrate_export`（同一个
`write_bytes`），行为正确。

---

## 3. ABI v4 改造的设计要点（给接手者的上下文）

### 3.1 为什么要有 `struct_size`

**原来的致命问题**（`crates/unbound-host/src/lib.rs` 的 `read_vtable`）：
宿主先把 guest 的**整个** `GuestPluginVTable` 结构体拷贝出来，**然后**才
检查 ABI 版本。旧版 guest 的 vtable 更小，拷贝就越界了——在版本检查有机会
拒绝它之前就已经读了越界内存。`check_abi` 的整个意义（"拒绝不兼容的
guest 而不是弄坏堆"）因此被架空。

**现在**：先只读结构体头两个 `u32`（`abi_version`、`struct_size`），
校验通过后才整体拷贝。

⚠️ **注意 `VTableHeader` 与 `GuestPluginVTable` 的前两个字段必须一致**：
`abi_version: u32` 在 offset 0，`struct_size: u32` 在 offset 4。
`GuestPluginVTable` 里 `struct_size` 紧跟 `abi_version` 就是为此。
如果以后往 vtable **头部**加字段，`VTableHeader` 必须同步更新。

### 3.2 `ByteBuffer` 取代 `*mut Vec<u8>`

`Vec<u8>` 不能过 ABI：它不是 `#[repr(C)]`，布局不在 Rust 稳定性保证内，
而 guest 恰恰是最可能用不同工具链编译的那一方。

现在：

```rust
#[repr(C)]
pub struct ByteBuffer {
    pub ptr: *mut u8,      // 宿主分配
    pub capacity: u32,     // 容量
    pub len: u32,          // 实际写入长度
}
```

协议：guest 装不下时**不写**，把所需长度放进 `len`；宿主再分配一次。
这样 guest 从不参与宿主的分配器，宿主用自己的分配器释放，两边不打架。

### 3.3 `on_load` 必须最先调用

原来的 `reload_vtable` 在**新 guest 已经分配完 state 之后**才调
`on_load`。而 guest 的 `#[global_allocator]` 在 `HOST_ALLOC` 为 null 时
回落到 `System`：于是新 guest 的 state 由 `System` 分配，将来却由宿主
的分配器释放——**跨分配器释放**。现在 `on_load` 提到了最前面。

### 3.4 `on_unload` 以前在 reload 时根本不调

旧 guest 的 DLL 被卸载了，它的 `on_unload` 却从没跑过。现在
`reload_vtable` 的 Migrate / Fresh 分支都补了 `(old.on_unload)()`。

### 3.5 `migrate_import` 失败返回 null

原来 `postcard::from_bytes` 失败会**静默塞一个空 seed**（用户数据全丢，
宿主还以为迁移成功了）。现在返回 null，宿主看到 null 就 fallback 到
`init_state()`。

⚠️ 但 `postcard` 是**位置式**格式，`#[serde(default)]` 对新增字段无效。
文档 `docs/06-tooling/hot-reload.md` 声称用 MessagePack / FlexBuffers
（自描述）且靠 `#[serde(default)]` 填默认值——**与实现不符**。
接手者若要真正支持"加字段不丢数据"，需要换自描述格式
（如 `rmp-serde` / `flexbuffers`，当前 `Cargo.lock` 里只有 `postcard`）。

### 3.6 嵌套类型的 schema 哈希

`State { rows: Vec<Row> }` 里 `Vec<Row>` 永远 24 字节，**改 `Row` 的布局
不会改变 `State` 的哈希** → 宿主以为布局没变，走 ZeroCopy 路径把旧内存
交给新 guest → 用 32 字节的 stride 读 40 字节 stride 分配的缓冲区。

现在 `impl_schema_hash!` 支持 `nested: [Row]` 分支，把嵌套类型自己的
`SCHEMA_HASH` 混进来；`app-guest` 给 `Row` 也写了 schema 哈希。

### 3.7 `HostRenderContext` 也加了 `struct_size`

同一个道理：宿主可能比 guest 老，context 结构体可能已经变大。guest 侧
`is_connected()` 现在会检查 `struct_size >= size_of::<Self>()`。

### 3.8 计数器竞态已修

`unbound-host/src/tests.rs` 原来用 6 个进程级 `AtomicU32` + 每个测试
开头 `reset_counters()`，而 `cargo test` 默认并行 → 互相踩。现在加了
`COUNTER_LOCK: Mutex<()>`，`reset_counters()` 返回 `MutexGuard`，各测试
用 `let _guard = reset_counters();` 持锁。

### 3.9 ⚠️ 分配器 vtable 的 `Box::leak` 是**故意保留**的

我一度把它改成 `Box<HostAllocatorVTable>` 存进结构体，结果
`app-guest` 的测试**段错误崩溃**（`STATUS_ACCESS_VIOLATION`）。

原因：guest 把分配器指针缓存在**进程级 static** 里（`HOST_ALLOC`），
而 `GuestHost` 是会被移动的（`GuestHost::new()` 返回值、`reload` 里的
`self`）。把 vtable 放在结构体里 → 结构体一动，guest 手里的指针就
指向已释放的内存。

**结论**：这个泄漏是 ABI 设计的一部分，不是 bug。字段文档已写明
"the leak is deliberate"。若要真正消除泄漏，正确的做法是让 guest
**复制** vtable 的内容而不是保存指针，但那要改 ABI 语义，属于更大的
改动，本次没做。

---

## 4. 剩余待办（按优先级）

### 4.1 高：文档与实现不一致（`docs/06-tooling/hot-reload.md`）

该文档仍描述着**改之前**的 ABI，需要同步：

| 位置 | 现状（文档） | 应改为 |
| :--- | :--- | :--- |
| L24-30 | vtable 结构体只有 `abi_version` / `state_schema_hash` | 加上 `struct_size`；说明"先读头再拷贝" |
| L29 | `migrate_export(..., out_buf: *mut Vec<u8>)` | `*mut ByteBuffer` |
| L27 | `on_load: extern "C" fn(...)` | 保持，但补充"reload 时最先调用" |
| L60-61 | "自描述 / MessagePack / FlexBuffers / `#[serde(default)]`" | 实际是 `postcard`（位置式），**没有** `#[serde(default)]` |
| L67-68 | `StateHandoff` 是 `Fast`/`Migrated` | 代码里是 `ZeroCopy`/`Migrate`/`Fresh` |
| L7 | 源路径 `crates/app_guest/src/main.rs` | 实际是 `crates/app-guest/src/lib.rs` |
| 全篇 | 未提 `nested:` schema 哈希 | 补充"嵌套类型也参与指纹" |

另外 `crates/unbound-abi/src/schema.rs:11` 提到 "`unbound-derive` 宏"，
**该 crate 不存在**，要删掉这句。

### 4.2 高：`docs/01-graphics/gpu-data-layout.md` 的 `GpuClipNode` 已过期

文档（L64-79）仍写着**旧的**字段顺序：

```rust
pub struct GpuClipNode {
    pub aabb_clip: [f32; 4],
    pub rounded_clip_0: [f32; 4],
    pub radii_0: [u16; 4],        // ← 旧顺序
    pub rounded_clip_1: [f32; 4],
    pub radii_1: [u16; 4],
}
```

实际（`crates/unbound-gpu/src/data.rs`，已修）：

```rust
pub struct GpuClipNode {
    pub aabb_clip: [f32; 4],      // offset 0
    pub rounded_clip_0: [f32; 4], // offset 16
    pub rounded_clip_1: [f32; 4], // offset 32
    pub radii_0: [u16; 4],        // offset 48
    pub radii_1: [u16; 4],        // offset 56
}
```

理由：std430 强制 `vec4` 16 字节对齐，交替排布会让 shader 侧步长变成
80 字节而 Rust 侧是 64 字节，索引 ≥1 的裁剪节点全部读错。

### 4.3 中：`app-guest` 的 `migrate_export` 语义仍是"位置式"

见 §3.5。真正的"加字段不丢数据"需要换格式。

### 4.4 中：`unbound-host` 的 `read_vtable` 仍有一个边界

`struct_size` 检查用的是 `header.struct_size < size_of::<GuestPluginVTable>()`，
即"guest 的表至少和 host 一样大"。但**版本号相同**才走到这里，所以
只要 ABI_VERSION 每次布局变更都 +1，这个检查实际上永远成立。它防的是
"有人改了 vtable 却忘了 bump 版本"。可以保留，也可以改成更严格的
"版本相同则 struct_size 必须相等"。

### 4.5 低：其它审计里提到、但**判定为误报或不值得改**的点

- `unbound-abi/src/plugin.rs` 的 `is_connected()` 原本是死代码——现在
  guest 侧真正开始用它了。
- `unbound-host` 的 `NodeArena::push` 用 `RawNodeId::ROOT` 作父节点会被
  静默孤立（已修：`parent.is_none() || parent == RawNodeId::ROOT`）。
- `app-guest/tests/host_roundtrip.rs` 的
  `the_dynamic_library_exports_the_entry_point` 在 DLL 不存在时**静默
  跳过**，可能空跑通过。DLL 找不到时应该 `panic!` 而不是 `eprintln!` 后
  return——这个测试的意义就在于验证真实 DLL 路径。
- `app-guest/tests/host_roundtrip.rs` 的
  `a_migration_round_trip_preserves_every_row` 是靠**伪造哈希**
  （`fresh.state_schema_hash ^= 0xFFFF`）强制走 Migrate 的，而真实
  schema 并没有变——所以 postround 往返必然成功。真正的 schema 变更
  场景（§3.5）没有被测到。

---

## 5. 已修缺陷的完整清单（供交叉核对，勿重复劳动）

### 5.1 `unbound-gpu`（GPU 后端）

| # | 缺陷 | 位置 | 修法 |
| :--- | :--- | :--- | :--- |
| 1 | **显存池越界写（严重）**：容量用"图元个数"比"字节数"，2000 图元的场景把 128 KB 写进 64 KB 块，覆盖 transform/clip/payload 数组 | `src/frame.rs` `ensure_pool_capacity` | 字节换算下沉到 `PersistentPool::ensure_capacity`，容量计数一律以字节计；新增 `Which::stride()` |
| 2 | **裁剪节点布局错位（严重）**：GLSL std430 步长 80 vs Rust 64 | `shaders/common.glsl` + `src/data.rs` | 宽成员排在一起，两侧统一 64 字节；重编 12 个 SPIR-V；加单元测试**直接解析 common.glsl** 校对成员顺序 |
| 3 | **设备扩展依赖缺失**：`VK_EXT_swapchain_maintenance1` 缺实例扩展 `VK_EXT_surface_maintenance1`，`VK_KHR_present_wait` 缺 `VK_KHR_present_id` → `vkCreateDevice` 非法用法 | `src/device.rs` | 按需补齐依赖扩展 |
| 4 | **presentWait/presentId 特性未启用** | `src/device.rs` `src/surface.rs` | 启用特性；`surface.rs` 的 `has_present_wait` 改为查特性而非扩展名 |
| 5 | **图集显存泄漏**：132 个对象（2 图集 × 64 层视图 + 2 image + 2 memory） | `src/atlas.rs` `src/frame.rs` | 图集持设备生命周期句柄，释放幂等；渲染器拆设备前显式释放 |
| 6 | **read_pixels 越界读**：固定 16 MiB staging 无边界检查 | `src/frame.rs` | 越界返回错误 |
| 7 | **隔离回收从未接线**：`pool.reclaim()` 在渲染循环里一次没调 | `src/frame.rs` | 每帧 `begin_frame` + `reclaim`，完成进度由帧栅栏推导 |
| 8 | **时间戳重置 panic** | `src/timing.rs` `src/frame.rs` | 无 `hostQueryReset` 时把重置记进命令缓冲 |
| 9 | **write_host 不 flush**：非一致性内存上 GPU 读到旧字节 | `src/device.rs` | DeviceLocal 也要求 HOST_COHERENT；写完统一 flush |
| 10 | **渲染路径每帧堆分配** | `src/frame.rs` | 等待信号量数组、布局屏障、暂存字节与补丁全部改为栈数组/复用缓冲 |

### 5.2 `unbound-canvas`

| # | 缺陷 | 位置 | 修法 |
| :--- | :--- | :--- | :--- |
| 11 | **四叉树丢实体（严重）**：`detach` 把表头改成被删元素的*后继*，删一个实体就抹掉同节点所有实体 | `src/quadtree.rs` | 链表改双向，删除 O(1) |
| 12 | **撤销删除只能重建空实体** | `src/undo.rs` | `Edit::Exist` 带上 bounds |
| 13 | **abort 只丢历史不回滚状态** | `src/undo.rs` | `abort()` 返回反转后的编辑列表 |
| 14 | **选区精确命中排他**，与 `contains_point` 包容不一致 | `src/quadtree.rs` + `unbound-math` | 新增 `Rect::touches`（重叠或相接），精确测试用它 |
| 15 | **吸附排序混单位**：世界单位² 面积 ÷ 世界单位距离，分数随缩放变 | `src/snap.rs` | 面积与距离都换算到屏幕像素 |
| 16 | **合并行盒可能产出负高度** | `unbound-text/src/selection.rs` | 要求方向向下且高度为正 |
| 17 | `level_bytes` 把 `Occupied`（20 字节）按 4 字节算 | `src/quadtree.rs` | 用 `size_of::<Occupied>()` |

### 5.3 `unbound-text`

| # | 缺陷 | 位置 | 修法 |
| :--- | :--- | :--- | :--- |
| 18 | **距离场编码整个反了**：GPU 约定"高 = 内部"，`pad_field` 用 255 当"far outside" → 每个 SDF 字形带一圈实心光晕；`from_coverage` 无边缘分支也反 | `src/glyph.rs` `src/sdf.rs` | 按 GPU 约定统一：padding 用 0，全覆盖用 255，空图用 0 |
| 19 | **图集键漏相位**：`allocate_color` 把 phase 硬编码 0 | `src/atlas.rs` `src/glyph.rs` | 相位进键 |
| 20 | **图集键漏字重/斜体**：`FAKE_ITALIC` 斜体与正体共用一张位图 | `src/atlas.rs` | `GlyphKey` 加 `weight` / `italic` 字段 |
| 21 | **图集驱逐不同步字形缓存**：矩形被回收后缓存条目仍指着它 → 采到别人的纹素 | `src/atlas.rs` `src/glyph.rs` | 驱逐时上报键，缓存据此删条目 |
| 22 | **图集驱逐是破坏性的**：重试仍失败时白死一个活字形 | `src/atlas.rs` | 驱逐改成"重试成功才提交"，失败放回原位 |
| 23 | **字形基线用 font_size 当上伸部**：既不是基线也忽略行的位置，多行文本第二行压在第一行上 | `src/shaping.rs` | 改为 `run.line_y + g.y` |
| 24 | **换行字节区间按行长累加**，漏掉换行符，每行偏移一字节 | `src/shaping.rs` | 按源文本行起始偏移换算，行内用 cluster 区间 |
| 25 | **样式哈希把 family 按字节混入**：None 与 Some("") 同哈希 | `src/shaping.rs` | 长度一起混入，None 用哨兵值 |
| 26 | **IME 锚点比光标大两像素**：原点下取整、尺寸上取整 | `src/selection.rs` | 两端都取整 |
| 27 | **字形字节命中用 "+8" 窗口猜测** | `src/shaping.rs` | 取最后一个 cluster ≤ 字节的字形；补 `caret_at_byte` |

### 5.4 `unbound-input` / `unbound-anim`

| # | 缺陷 | 位置 | 修法 |
| :--- | :--- | :--- | :--- |
| 28 | **次键释放中止主键拖拽** | `src/pointer.rs` | 记录按下时的按钮，只有它能结束交互 |
| 29 | **次键释放污染双击时钟** | `src/pointer.rs` | 次键既不算点击也不动时钟 |
| 30 | **抢走/释放捕获不发 CaptureLost** | `src/pointer.rs` | 两个方法都返回 `CaptureLost` 事件 |
| 31 | **多边形命中坐标系文档与实现不符** | `src/hit.rs` | 实现是对的，改文档 |
| 32 | **NaN 弹簧永不休眠**：与 NaN 的比较恒为假 → 调度器永不休，NaN 变换上屏 | `unbound-anim/src/lib.rs` | 非有限状态视为已静止；`step` 遇 NaN 目标或发散就归位 |

---

## 6. 踩过的坑（省接手者的时间）

1. **glslc / spirv-val 路径**：`C:\VulkanSDK\1.4.350.0\Bin`。改了
   `shaders/*.glsl` 必须重编 12 个 SPIR-V 并提交
   （`crates/unbound-gpu/shaders/spv/*.spv`）：
   ```bash
   PATH="$PATH:/c/VulkanSDK/1.4.350.0/Bin" bash scripts/compile-shaders.sh
   ```
   `build.rs` 优先用 `glslc`，没有才回退到签入的 blob——**不回退就编译不过**。

2. **spirv-dis 验证布局**：
   ```bash
   spirv-dis --no-color crates/unbound-gpu/shaders/spv/cull.comp.spv | grep -E "Offset|ArrayStride"
   ```
   改 `ClipNode` 时用它确认 `ArrayStride 64`。

3. **验证层跑法**（确认 Vulkan 侧无报错）：
   ```bash
   VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation just q-run cargo run -p showcase -- --shot out.png
   ```
   报错会打在 stderr 里。修完泄漏后应当**零 Validation Error**。

4. **`unbound-gpu` 的测试会打大量 Vulkan Loader 日志**，过滤：
   `just q-run cargo test -p unbound-gpu 2>&1 | grep -E "^test result:|FAILED"`.

5. **单测里 GPU 时间预算是 flaky 的**：
   `sdf::tests::a_glyph_tile_becomes_a_distance_field_in_time` 偶尔因为
   0.0500 ms > 0.05 ms 预算失败。**重跑即可**，不是回归。

6. **`unbound-gpu` 的 `FrameStats::draw_calls` 有两个含义**（API 调用数 vs
   间接绘制轮数），`read_pixels()` 之后会变成后者。属已知设计，未改。

7. **`just q-run` 后面不要带 `--`**：`just q-run -- cargo ...` 会把 `--`
   当成子命令报 `Failed to start child process '--'`。

---

## 7. 建议的收尾顺序

1. 修 §2 的 `export_state` 回调错用 → `unbound-host` 测试全绿。
2. 跑 `just check`（clippy 零告警）+ `just test`（全绿）。
3. 同步文档：
   - `docs/06-tooling/hot-reload.md`（ABI v4 全套，见 §4.1 表）
   - `docs/01-graphics/gpu-data-layout.md` 的 `GpuClipNode`（见 §4.2）
   - `crates/unbound-abi/src/schema.rs:11` 删掉不存在的 `unbound-derive`
4. 提交 ABI v4 改造：
   ```
   fix(abi): 给 vtable 加尺寸协商，用 ByteBuffer 取代 Vec<u8>，修正挂载时序
   ```
5. push。
6. （可选）做 §4.5 里那两个测试的加固：DLL 找不到时 `panic!` 而不是静默跳过；
   补一个真正 schema 变更的迁移用例。

---

## 8. 附：本次修复的验证基线

接手者改完后，应当能复现以下结果：

```
just test        → 全部 ok，0 failed
just check       → 无 error，无 warning
just q-run cargo run -p showcase -- --shot /tmp/o.png
                 → 与 docs/assets/showcase-light.png 字节一致
                   （SHA256 8E65000D3C56D38D6B04CFB6CBBF77C8E43B960811BD1206B16BC18BF3C843A4）
just q-run cargo run -p gallery -- --kpi
                 → gpu: 100k primitives in ~0.67 ms, 1 draw call(s), 99840 visible
                   六项 KPI 全部 ok
```
