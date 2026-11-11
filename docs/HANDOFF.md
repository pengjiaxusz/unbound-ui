# 交接文档：unbound-ui 缺陷修复

> 面向接手的 agent。读完这一份就能接着干，不用重跑审计。
>
> 最后更新：ABI v4 改造完成并提交后。仓库：`D:\pengj\unbound-ui`，分支 `main`。

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

### 1.1 已完成并已提交

| commit | 内容 |
| :--- | :--- |
| `2712dd5` | 显存池越界写、四叉树丢实体、裁剪节点布局错位 |
| `bab9868` | 距离场编码、图集键、字形基线 |
| `95aee14` | 显存池隔离回收接线 + 若干越界/同步缺陷 |
| `1312d77` | 次键中止拖拽、捕获丢失无通知、NaN 弹簧永不休眠 |
| `eecc7ca` | 撤销语义、图集驱逐回滚、渲染路径零分配 |
| `4c9803c` | 选区边界、吸附排序、样式哈希、IME 锚点 |
| `e6239e1` | 去掉多余的 `mut` 限定 |
| 本次提交 | **ABI v4 改造**（见 §1.2） |

这批提交对应的每个缺陷都**先写了会失败的回归测试**，再改实现。

### 1.2 ABI v4 改造（本次提交，已完成）

涉及 7 个文件：

```
crates/unbound-abi/src/plugin.rs    struct_size 协商、ByteBuffer 取代 Vec<u8>
crates/unbound-abi/src/schema.rs    嵌套类型的 schema 哈希（hash_nested + 新宏分支）
crates/unbound-host/src/lib.rs       先读头再拷贝、on_load 前置、on_unload 补调用、
                                    ROOT 父节点、影子副本清理、export_state 两段式缓冲
crates/unbound-host/src/tests.rs     新 ABI 签名、计数器加锁、EXPORTED 只数真正导出的那次
crates/app-guest/src/lib.rs          ByteBuffer 写入、migrate_import 失败返回 null、
                                    Row/State 嵌套 schema 哈希、repr(C) 固定布局
crates/app-guest/tests/host_roundtrip.rs  DLL 缺失改为硬失败、按新鲜度挑 DLL
docs/06-tooling/hot-reload.md        全文重写，与 ABI v4 对齐
docs/01-graphics/gpu-data-layout.md  GpuClipNode 字段顺序纠正
```

**本轮修掉的 4 个问题**（后两个是接手时文档没提到的**新发现**）：

1. **`export_state` 调错了回调**（交接文档 §2 点名的那个失败测试）。
   `export_state` 被 `snapshot()` 和 `reload_vtable()` 的 Migrate 分支共用，
   但两处该走的回调不同：前者是 `snapshot`，后者必须是 `migrate_export`。
   修法是给 `export_state` 传入要用的回调。

   连带一处语义修正：两段式缓冲协议下 guest 回调会被调**两次**（先探长度，
   再填充），所以假 guest 的 `EXPORTED` 计数器改成**只数真正写出了字节的那次**，
   否则同一个测试会看到 2 而不是 1。另加了一个 `SNAPSHOTTED` 计数器，让
   "迁移走的不是 snapshot"成为一条显式断言，而不是靠计数器的缺席来推断。

2. **`Row::done` 的 schema 偏移量写错了**（16 → 24）。
   `String` 是 24 字节，`done: bool` 在 offset 24，不是在 16。

3. **guest 的 `State` / `Row` 缺 `#[repr(C)]`——指纹与真实布局脱节（新发现，严重）**。
   默认 `repr(Rust)` 下编译器**会重排字段**：`State { count, title, rows }` 实测被排成
   `title@0, rows@24, count@48`，而手写的 schema 指纹记的是声明顺序
   `count@0, title@8, rows@32`。

   指纹的作用是"两个构建的字节布局一致"，而 `repr(Rust)` 连**这一条**都不保证：
   编译器可以重排，换一版编译器还可能排得不一样。手写指纹**检测不出**这种漂移，
   会为两个布局不同的 state 报"兼容"→ 宿主走零拷贝通道 → 用错误的 stride 读旧内存。

   修法：`State` 与 `Row` 加 `#[repr(C)]`，偏移量由**编译期断言**钉死
   （写错直接编译失败），并把两个类型的 schema 种子一并改掉——字段清单没变但
   字节布局变了，指纹必须跟着变，否则新旧 DLL 会被判为"兼容"。
   同时在 `StateSchema` 的文档里写明了这条约束。

4. **端到端测试会拿过期的 DLL 去比对（新发现）**。
   `cargo test` 把 cdylib 产出到 `target/<profile>/deps/`，`cargo build` 产出到
   `target/<profile>/`；`locate_guest_dll` 按路径顺序取第一个，于是经常拿到旧的
   那份。修法是**取最新的那个候选**，并且 DLL 找不到时**硬失败**（原来是
   `eprintln!` + `return`，测试会空跑通过）。

**验证结果**：

```
just test        → 全部 ok，0 failed
just check       → exit 0，零告警
just q-run cargo run -p showcase -- --shot out.png
                 → 与 docs/assets/showcase-light.png 字节一致
                   （SHA256 8E65000D3C56D38D6B04CFB6CBBF77C8E43B960811BD1206B16BC18BF3C843A4）
just q-run cargo run -p gallery -- --kpi
                 → gpu: 100k primitives in 0.667 ms, 1 draw call(s), 99840 visible
                   六项 KPI 全部 ok
```

---

## 2. 剩余待办（按优先级）

### 2.1 中：迁移格式仍是位置式，`postcard` 不支持"加字段不丢数据"

`postcard` 按字段顺序编码，`#[serde(default)]` 对**新增字段无效**——老数据缺字段
时解码直接失败。

`migrate_import` 解码失败会返回 **null**，宿主回落到 `init_state()`：失败是可见的，
但用户数据仍然是丢的。要真正做到"加字段不丢数据"，需要换自描述格式
（`rmp-serde` / `flexbuffers`，当前 `Cargo.lock` 里只有 `postcard`）。

`docs/06-tooling/hot-reload.md` 已经把这个真相写进文档，不再宣称"彻底告别
热重载必崩溃"。

### 2.2 中：`app-guest` 的端到端迁移测试靠伪造哈希

`a_migration_round_trip_preserves_every_row` 靠 `fresh.state_schema_hash ^= 0xFFFF`
强制走 Migrate 分支，但真实 schema 并没变——所以往返必然成功，真正的 schema 变更
场景没有被测到。

要覆盖它需要**第二个 schema 不同的 guest DLL**（一个新 crate 或一个 feature 开关
编译出两种 `State` 布局）。

### 2.3 低：`read_vtable` 的尺寸检查方向

`header.struct_size < size_of::<GuestPluginVTable>()`：guest 的表比 host 大则**放行**
（尾部多出来的成员不读，是有意的前向兼容），比 host 小则拒绝。

但**版本号相同**才走到这里，所以只要 `ABI_VERSION` 每次布局变更都 +1，这个检查实际
上只在"改了布局却忘了 bump 版本"时才起作用。往虚表**中间**插字段会挪动后面所有
成员，任何尺寸检查都拦不住——这才是版本号必须 bump 的原因（已在代码注释里写明）。

### 2.4 低：`cargo fmt --check` 在 HEAD 上就不通过（与本次改动无关）

仓库里有 **43 个文件**存在 rustfmt 差异，包括本次完全没碰的文件
（`apps/gallery/src/main.rs`、`crates/unbound-gpu/src/timing.rs` 等）。
这是 **rustfmt 版本漂移**（工具链 1.98.1 / rustfmt 1.9.0-stable，仓库是按更早的
版本格式化的），不是代码风格问题。

本轮的处理方式：**只保证本次改动引入的行是 fmt-clean 的**，不重排全仓
（那会产生一个巨大的无关 diff）。所以 `just fmt` 目前仍然失败，但失败点与本次
改动无关。

---

## 3. 已修缺陷的完整清单（供交叉核对，勿重复劳动）

### 3.1 `unbound-gpu`（GPU 后端）

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

### 3.2 `unbound-canvas`

| # | 缺陷 | 位置 | 修法 |
| :--- | :--- | :--- | :--- |
| 11 | **四叉树丢实体（严重）**：`detach` 把表头改成被删元素的*后继*，删一个实体就抹掉同节点所有实体 | `src/quadtree.rs` | 链表改双向，删除 O(1) |
| 12 | **撤销删除只能重建空实体** | `src/undo.rs` | `Edit::Exist` 带上 bounds |
| 13 | **abort 只丢历史不回滚状态** | `src/undo.rs` | `abort()` 返回反转后的编辑列表 |
| 14 | **选区精确命中排他**，与 `contains_point` 包容不一致 | `src/quadtree.rs` + `unbound-math` | 新增 `Rect::touches`（重叠或相接），精确测试用它 |
| 15 | **吸附排序混单位**：世界单位² 面积 ÷ 世界单位距离，分数随缩放变 | `src/snap.rs` | 面积与距离都换算到屏幕像素 |
| 16 | **合并行盒可能产出负高度** | `unbound-text/src/selection.rs` | 要求方向向下且高度为正 |
| 17 | `level_bytes` 把 `Occupied`（20 字节）按 4 字节算 | `src/quadtree.rs` | 用 `size_of::<Occupied>()` |

### 3.3 `unbound-text`

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

### 3.4 `unbound-input` / `unbound-anim`

| # | 缺陷 | 位置 | 修法 |
| :--- | :--- | :--- | :--- |
| 28 | **次键释放中止主键拖拽** | `src/pointer.rs` | 记录按下时的按钮，只有它能结束交互 |
| 29 | **次键释放污染双击时钟** | `src/pointer.rs` | 次键既不算点击也不动时钟 |
| 30 | **抢走/释放捕获不发 CaptureLost** | `src/pointer.rs` | 两个方法都返回 `CaptureLost` 事件 |
| 31 | **多边形命中坐标系文档与实现不符** | `src/hit.rs` | 实现是对的，改文档 |
| 32 | **NaN 弹簧永不休眠**：与 NaN 的比较恒为假 → 调度器永不休，NaN 变换上屏 | `unbound-anim/src/lib.rs` | 非有限状态视为已静止；`step` 遇 NaN 目标或发散就归位 |

### 3.5 `unbound-abi` / `unbound-host` / `app-guest`（ABI v4）

| # | 缺陷 | 位置 | 修法 |
| :--- | :--- | :--- | :--- |
| 33 | **宿主先拷整个 vtable 再查版本**：老 guest 的 vtable 更小，拷贝就越界——"拒绝不兼容 guest 而不是弄坏堆"被架空 | `unbound-host/src/lib.rs` `read_vtable` | 先只读头两个 `u32`（`abi_version`/`struct_size`），校验通过才整体拷贝 |
| 34 | **`Vec<u8>` 过 ABI**：不是 `#[repr(C)]`，布局不在 Rust 稳定性保证内，而 guest 最可能用别的工具链编译 | `unbound-abi/src/plugin.rs` | 换成 `#[repr(C)]` 的 `ByteBuffer`，两段式：探长度 → 宿主分配 → 填充 |
| 35 | **`on_load` 在新 guest 分配 state 之后才调**：guest 的 `HOST_ALLOC` 为 null 时回落到 `System`，state 由 `System` 分配却由宿主分配器释放——跨分配器释放 | `unbound-host/src/lib.rs` `reload_vtable` | `on_load` 提到最前面 |
| 36 | **reload 时旧 guest 的 `on_unload` 根本不调** | 同上 | Migrate / Fresh 分支都补上 |
| 37 | **`migrate_import` 解码失败静默塞空 seed**：用户数据全丢，宿主还以为迁移成功 | `app-guest/src/lib.rs` | 返回 null，宿主回落到 `init_state()` |
| 38 | **嵌套类型不参与 schema 哈希**：`Vec<Row>` 永远 24 字节，改 `Row` 布局不改 `State` 哈希 → 宿主走零拷贝，用 32 字节 stride 读 40 字节 stride 的缓冲 | `unbound-abi/src/schema.rs` | `impl_schema_hash!` 加 `nested: [Row]` 分支 |
| 39 | **`Row::done` 偏移量写成 16**（实际 24） | `app-guest/src/lib.rs` | 改为 24，并加编译期断言钉死所有偏移 |
| 40 | **`State` / `Row` 缺 `repr(C)`，指纹与真实布局脱节（严重）** | `app-guest/src/lib.rs` | 加 `#[repr(C)]`、改 schema 种子、偏移量编译期断言、`StateSchema` 文档写明约束 |
| 41 | **`export_state` 在迁移路径上错调 `snapshot`** | `unbound-host/src/lib.rs` | `export_state` 接收要用的回调 |
| 42 | **端到端测试拿过期 DLL 比对指纹 / DLL 缺失时静默跳过** | `app-guest/tests/host_roundtrip.rs` | 取最新候选；缺失时硬失败 |
| 43 | **影子副本从不清理**：一次开发会话几百次重载 → 临时目录几百份 DLL | `unbound-host/src/lib.rs` | 宿主记录副本，卸载时删除 |

---

## 4. 踩过的坑（省接手者的时间）

1. **`export_state` 的两段式协议**：guest 回调会被调两次（先探长度，再填充）。
   写假 guest 时，计数器要数**真正写出字节的那次**，否则会看到 2 而不是 1。

2. **`repr(Rust)` 会重排字段**：`State { count: i64, title: String, rows: Vec<Row> }`
   实测排成 `title@0, rows@24, count@48`。**跨构建共享内存的状态结构体必须
   `#[repr(C)]`**，否则 schema 指纹的保护形同虚设。

3. **glslc / spirv-val 路径**：`C:\VulkanSDK\1.4.350.0\Bin`。改了
   `shaders/*.glsl` 必须重编 12 个 SPIR-V 并提交
   （`crates/unbound-gpu/shaders/spv/*.spv`）：
   ```bash
   PATH="$PATH:/c/VulkanSDK/1.4.350.0/Bin" bash scripts/compile-shaders.sh
   ```
   `build.rs` 优先用 `glslc`，没有才回退到签入的 blob——**不回退就编译不过**。

4. **spirv-dis 验证布局**：
   ```bash
   spirv-dis --no-color crates/unbound-gpu/shaders/spv/cull.comp.spv | grep -E "Offset|ArrayStride"
   ```
   改 `ClipNode` 时用它确认 `ArrayStride 64`。

5. **验证层跑法**（确认 Vulkan 侧无报错）：
   ```bash
   VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation just q-run cargo run -p showcase -- --shot out.png
   ```
   报错会打在 stderr 里。修完泄漏后应当**零 Validation Error**。

6. **`unbound-gpu` 的测试会打大量 Vulkan Loader 日志**，过滤：
   `just q-run cargo test -p unbound-gpu 2>&1 | grep -E "^test result:|FAILED"`.

7. **单测里 GPU 时间预算是 flaky 的**：
   `sdf::tests::a_glyph_tile_becomes_a_distance_field_in_time` 偶尔因为
   0.0500 ms > 0.05 ms 预算失败。**重跑即可**，不是回归。

8. **`unbound-gpu` 的 `FrameStats::draw_calls` 有两个含义**（API 调用数 vs
   间接绘制轮数），`read_pixels()` 之后会变成后者。属已知设计，未改。

9. **`just q-run` 后面不要带 `--`**：`just q-run -- cargo ...` 会把 `--`
   当成子命令报 `Failed to start child process '--'`。

10. **`just` 不允许重定义 recipe**：`justfile` 里再写一个 `test:` 会直接报错
    （`recipe \`test\` first defined on line 23 is redefined on line 35`）。
    要改模板自带的 recipe，只能改 `just/rust-workspace.just`——而那是**模板托管区**，
    不能动。项目专属的解决办法是新增**不同名**的 recipe。

11. **`cargo test` 与 `cargo build` 的 cdylib 落在不同目录**：前者是
    `target/<profile>/deps/`，后者是 `target/<profile>/`。找 DLL 的代码要按
    **新鲜度**挑，别按路径顺序挑。

12. **`cargo fmt --check` 在 HEAD 上就不通过**（rustfmt 版本漂移，43 个文件）。
    见 §2.4。

---

## 5. 验证基线（接手者改完后应当能复现）

```
just test        → 全部 ok，0 failed
just check       → exit 0，无 error，无 warning
just q-run cargo run -p showcase -- --shot /tmp/o.png
                 → 与 docs/assets/showcase-light.png 字节一致
                   （SHA256 8E65000D3C56D38D6B04CFB6CBBF77C8E43B960811BD1206B16BC18BF3C843A4）
just q-run cargo run -p gallery -- --kpi
                 → gpu: 100k primitives in ~0.67 ms, 1 draw call(s), 99840 visible
                   六项 KPI 全部 ok
```
