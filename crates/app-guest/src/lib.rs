//! 业务层 Guest（`docs` §8.1）。
//!
//! 本 crate 即为业务层：一个可在 host 保持窗口、Vulkan 设备及所有 GPU 池
//! 存活的情况下被挂载、卸载与重新挂载的 `cdylib`。它不接触任何图形 API，
//! 仅依赖轻量级 POD/`Action` crate `unbound-abi`——这使得 guest 的重新编译
//! 耗时保持在数百毫秒而非数十秒。
//!
//!
//! 两种机制保证了重新挂载的安全性：
//!
//! * **分配器劫持。** [`ForwardingAllocator`] 替换 guest 的全局分配器并
//!   转发至 host 的虚表，使 host 传入并由 guest 填充的 `Vec<u8>` 位于同一堆
//!   而非两个堆。否则 Windows 会在 `migrate_export` 写入字节时立即损坏堆。
//! *
//! * **Schema 指纹。** [`State`] 实现 `StateSchema`；哈希相等时 host 可
//!   直接移交旧指针，哈希变更时则经由 `migrate_export`/`migrate_import` 迁移。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::alloc::{GlobalAlloc, Layout, System};

use serde::{Deserialize, Serialize};
use unbound_abi::action::{ActionKind, RawAction};
use unbound_abi::desc::{RawNodeDesc, RawNodeId, NO_NODE};
use unbound_abi::migration::{
    encode, encode_list, migrate, migrate_list, Field, FieldSpec, MigratedField, MigrationError,
};
use unbound_abi::plugin::{
    ByteBuffer, GuestPluginVTable, GuestState, HostAllocatorVTable, HostRenderContext, ABI_VERSION,
};
use unbound_abi::schema::StateSchema;

// ---------------------------------------------------------------------------
// 分配器劫持
// ---------------------------------------------------------------------------

/// 由 `on_load` 安装的 host 分配器。此前为空指针，因此下方需要回退逻辑：
/// `on_load` 本身也可能需要分配内存。
static HOST_ALLOC: AtomicPtr<HostAllocatorVTable> = AtomicPtr::new(core::ptr::null_mut());

/// 将所有分配请求转发至 host 的 guest 分配器。
struct ForwardingAllocator;

impl ForwardingAllocator {
    fn table() -> *mut HostAllocatorVTable {
        HOST_ALLOC.load(Ordering::Acquire)
    }
}

unsafe impl GlobalAlloc for ForwardingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let table = Self::table();
        if table.is_null() {
            // SAFETY: 系统分配器始终有效。
            return unsafe { System.alloc(layout) };
        }
        // SAFETY: host 在 `on_load` 中安装了虚表，并在整个挂载期间保持其存活。
        //
        unsafe { ((*table).alloc)(layout.size(), layout.align()) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let table = Self::table();
        if table.is_null() {
            // SAFETY: 虚表为空，说明该内存块由 `System` 分配。
            return unsafe { System.dealloc(ptr, layout) };
        }
        // SAFETY: 与 `alloc` 使用相同的分配来源。
        unsafe { ((*table).dealloc)(ptr, layout.size(), layout.align()) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let table = Self::table();
        if table.is_null() {
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        // SAFETY: 与 `alloc` 使用相同的分配来源。
        unsafe { ((*table).realloc)(ptr, layout.size(), layout.align(), new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: ForwardingAllocator = ForwardingAllocator;

// ---------------------------------------------------------------------------
// 状态定义
// ---------------------------------------------------------------------------

/// 示例列表中的单行数据。
///
/// 此处的 `repr(C)` 并非可选。当 schema 哈希匹配时，host 会将*旧*状态
/// 原样交给*新* guest，因此两次构建必须就字节布局达成一致。在默认 `repr(Rust)`
/// 下编译器可自由重排字段——实际上它也确实将 `State::count` 移到了偏移 48——
/// 且不同编译器版本的重排方式可能不同。手写指纹无法防住这种漂移；
/// 只有固定布局才能保证安全。
///
///
///
/// `priority` 与 `tags` 在 V1 发布后新增。每行各自作为一条 UBMF 信封传输
/// （见 [`encode_rows`]），因此 V1 负载仅缺少字段 3 和 4，新 guest 会填充
/// 默认值而不会丢失行数据。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct Row {
    pub label: String,
    pub done: bool,
    pub priority: u8,
    pub tags: Vec<String>,
}

// 行自身的布局也是状态布局的一部分：无论 `Row` 长什么样，`Vec<Row>`
// 始终占 24 字节；若无此项，`Row` 新增字段时指纹不会变化——host 便会
// 误将旧内存当成布局仍匹配直接交给新 guest。
//
//
// `Row` 改用 `repr(C)` 时更新了种子值：字段列表未变，但字节布局已变，
// 指纹必须反映这一点。
unbound_abi::impl_schema_hash!(
    Row,
    0x524f_5745_4e43, // "ROWNB" + repr(C)
    "label",
    "String",
    0,
    "done",
    "bool",
    24,
    "priority",
    "u8",
    25,
    "tags",
    "Vec<String>",
    32,
);

/// guest 的完整业务状态。
///
/// 为何使用 `repr(C)` 请参见 [`Row`]。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct State {
    pub count: i64,
    pub title: String,
    pub rows: Vec<Row>,
}

// 上方写入 schema 哈希的偏移量即契约：若其与真实布局产生偏移，
// 指纹便不再描述真实布局，host 的零拷贝快路径也会变得不安全。
// 在此钉死偏移量，使错误的偏移在编译期失败而非静默削弱哈希。
//
const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(Row, label) == 0);
    assert!(
        offset_of!(Row, done) == 24,
        "Row::done follows a 24-byte String"
    );
    assert!(offset_of!(State, count) == 0);
    assert!(offset_of!(State, title) == 8);
    assert!(offset_of!(State, rows) == 32);
    assert!(offset_of!(Row, priority) == 25);
    assert!(offset_of!(Row, tags) == 32);
    assert!(core::mem::size_of::<Row>() == 56);
    assert!(core::mem::size_of::<State>() == 56);
};

// 字段顺序与偏移量即契约：新增 `rows` 会改变哈希，host 将回退到
// `migrate_*` 而非重新解释旧内存。
//
// `Row` 作为嵌套类型列出：其内部变更同样会改变状态布局，
// 尽管 `Vec<Row>` 本身大小永不改变。
//
// `State` 改用 `repr(C)` 时更新了种子值，原因同 `Row`：
// 字段列表未变，但字节布局已变。
unbound_abi::impl_schema_hash!(
    State,
    0x5542_4f55_4e45, // "UNBOUND" + repr(C)
    nested: [Row],
    "count",
    "i64",
    0,
    "title",
    "String",
    8,
    "rows",
    "Vec<Row>",
    32,
);

impl State {
    fn seed() -> Self {
        Self {
            count: 0,
            title: "Hot reload demo".to_owned(),
            rows: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// 可迁移状态
// ---------------------------------------------------------------------------

/// 版本化信封的稳定字段 ID。绝不可复用。
const FIELD_COUNT: u16 = 1;
const FIELD_TITLE: u16 = 2;
const FIELD_ROWS: u16 = 3;
/// 空 `String` / `Vec` / 零值 `u8` 的 `postcard` 编码。
const EMPTY_COLLECTION: &[u8] = &[0];
/// 空行列表：`encode_list` 头部为 `u32` 零值。
const EMPTY_ROWS: &[u8] = &[0, 0, 0, 0];

/// 状态的新形态。旧负载中缺失的字段取默认值；缺失 `count` 将返回 host 可见的错误。
///
const STATE_FIELDS: &[FieldSpec] = &[
    FieldSpec::required(FIELD_COUNT),
    FieldSpec::optional(FIELD_TITLE, EMPTY_COLLECTION),
    FieldSpec::optional(FIELD_ROWS, EMPTY_ROWS),
];

// 行字段 ID。V1 行仅携带 1 和 2；3 和 4 为本版本新增字段，
// V1 负载在迁移时会取其默认值。
const ROW_LABEL: u16 = 1;
const ROW_DONE: u16 = 2;
const ROW_PRIORITY: u16 = 3;
const ROW_TAGS: u16 = 4;

const ROW_FIELDS: &[FieldSpec] = &[
    FieldSpec::required(ROW_LABEL),
    FieldSpec::required(ROW_DONE),
    FieldSpec::optional(ROW_PRIORITY, EMPTY_COLLECTION),
    FieldSpec::optional(ROW_TAGS, EMPTY_COLLECTION),
];

/// 将行列表编码为自描述信封列表。
///
/// # Errors
/// 透传无法序列化的字段错误。
pub fn encode_rows(rows: &[Row]) -> Result<Vec<u8>, MigrationError> {
    encode_list(rows, |row| {
        let label = postcard::to_allocvec(&row.label).map_err(|_| MigrationError::FieldTooLarge)?;
        let done = postcard::to_allocvec(&row.done).map_err(|_| MigrationError::FieldTooLarge)?;
        let priority =
            postcard::to_allocvec(&row.priority).map_err(|_| MigrationError::FieldTooLarge)?;
        let tags = postcard::to_allocvec(&row.tags).map_err(|_| MigrationError::FieldTooLarge)?;
        encode(&[
            Field::new(ROW_LABEL, &label),
            Field::new(ROW_DONE, &done),
            Field::new(ROW_PRIORITY, &priority),
            Field::new(ROW_TAGS, &tags),
        ])
    })
}

/// 解码行列表，为旧行中缺失的字段填充默认值。
///
/// # Errors
/// 当列表或某行损坏时返回结构性 [`MigrationError`]，缺失必需字段时
/// 返回 [`MigrationError::MissingField`]。
///
pub fn decode_rows(bytes: &[u8]) -> Result<Vec<Row>, MigrationError> {
    migrate_list(bytes, ROW_FIELDS, |fields| {
        Ok(Row {
            label: row_field(fields, ROW_LABEL)?,
            done: row_field(fields, ROW_DONE)?,
            priority: row_field(fields, ROW_PRIORITY)?,
            tags: row_field(fields, ROW_TAGS)?,
        })
    })
}

fn row_field<T: serde::de::DeserializeOwned>(
    fields: &[MigratedField<'_>],
    id: u16,
) -> Result<T, MigrationError> {
    let field = fields
        .iter()
        .find(|field| field.id == id)
        .ok_or(MigrationError::MissingField(id))?;
    postcard::from_bytes(&field.bytes).map_err(|_| MigrationError::Truncated)
}

// ---------------------------------------------------------------------------
// Guest 生命周期
// ---------------------------------------------------------------------------

unsafe extern "C" fn on_load(alloc: *const HostAllocatorVTable) {
    HOST_ALLOC.store(alloc.cast_mut(), Ordering::Release);
}

unsafe extern "C" fn on_unload() {
    // 保留指针原位：卸载 guest 不会使 host 虚表失效，再次 `on_load` 直接覆盖即可。
    //
}

unsafe extern "C" fn init_state() -> GuestState {
    Box::into_raw(Box::new(State::seed())).cast::<c_void>()
}

unsafe extern "C" fn drop_state(state: GuestState) {
    if state.is_null() {
        return;
    }
    // SAFETY: `state` 来自本 crate 的 `Box::into_raw`（或布局相同的上一代 guest，
    // 这正是 ZeroCopy 的意义所在）。
    drop(unsafe { Box::from_raw(state.cast::<State>()) });
}

unsafe extern "C" fn adopt_state(_state: GuestState) {
    // 无需操作：host 继续持有该指针并在每次调用时传回。
    // schema 哈希已证明布局完全一致。
}

unsafe extern "C" fn migrate_export(state: GuestState, out: *mut ByteBuffer) {
    if state.is_null() || out.is_null() {
        return;
    }
    // SAFETY: 两个指针均由 host 提供且在本次调用期间存活。
    let state = unsafe { &*state.cast::<State>() };
    let out = unsafe { &mut *out };
    // 每个字段独立编码，以便新增字段时无需重排旧字段的字节。
    //
    let (Ok(count), Ok(title), Ok(rows)) = (
        postcard::to_allocvec(&state.count),
        postcard::to_allocvec(&state.title),
        encode_rows(&state.rows),
    ) else {
        out.len = 0;
        return;
    };
    let envelope = unbound_abi::migration::encode(&[
        Field::new(FIELD_COUNT, &count),
        Field::new(FIELD_TITLE, &title),
        Field::new(FIELD_ROWS, &rows),
    ]);
    let Ok(bytes) = envelope else {
        // 状态无法序列化。报告“无需空间”以免 host 为不存在的负载分配缓冲区。
        //
        out.len = 0;
        return;
    };
    write_bytes(out, &bytes);
}

unsafe extern "C" fn migrate_import(ptr: *const u8, len: usize) -> GuestState {
    if ptr.is_null() {
        // SAFETY: `init_state` 仅执行分配。
        return unsafe { init_state() };
    }
    // SAFETY: host 传入的正是 `migrate_export` 填充的缓冲区。
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    // 解码失败必须*可见*：返回空指针时 host 会回退到全新状态，
    // 这比静默返回空种子假装迁移成功要好得多。
    //
    let Ok(fields) = migrate(bytes, STATE_FIELDS) else {
        return core::ptr::null_mut();
    };
    let decode = |id: u16| fields.iter().find(|field| field.id == id);
    let Some(count) =
        decode(FIELD_COUNT).and_then(|field| postcard::from_bytes::<i64>(&field.bytes).ok())
    else {
        return core::ptr::null_mut();
    };
    let Some(title) =
        decode(FIELD_TITLE).and_then(|field| postcard::from_bytes::<String>(&field.bytes).ok())
    else {
        return core::ptr::null_mut();
    };
    let Some(rows) = decode(FIELD_ROWS).and_then(|field| decode_rows(&field.bytes).ok()) else {
        return core::ptr::null_mut();
    };
    let state = State { count, title, rows };
    Box::into_raw(Box::new(state)).cast::<c_void>()
}

/// 将 `bytes` 写入 host 缓冲区。
///
/// host 拥有内存分配；guest 仅负责填充。缓冲区过小时 guest 不写入数据
/// 并报告所需大小，以便 host 分配更大缓冲区后重试。
///
fn write_bytes(out: &mut ByteBuffer, bytes: &[u8]) {
    let needed = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    if out.capacity < needed || out.ptr.is_null() {
        // 空间不足：报告 host 需要提供的大小。
        out.len = needed;
        return;
    }
    // SAFETY: host 保证在 `ptr` 处有 `capacity` 字节可写。
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), out.ptr, bytes.len());
    }
    out.len = needed;
}

unsafe extern "C" fn reduce(state: GuestState, action: *const RawAction) {
    if state.is_null() || action.is_null() {
        return;
    }
    // SAFETY: host 在调用期间保证两个指针均有效。
    let state = unsafe { &mut *state.cast::<State>() };
    let action = unsafe { &*action };
    match action.kind() {
        ActionKind::Press => {
            state.count += 1;
            state.rows.push(Row {
                label: format!("item {}", state.count),
                done: false,
                priority: 0,
                tags: Vec::new(),
            });
        }
        ActionKind::Undo => {
            if state.rows.pop().is_some() {
                state.count -= 1;
            }
        }
        ActionKind::Custom => {
            let index = action.payload[0] as usize;
            if let Some(row) = state.rows.get_mut(index) {
                row.done = !row.done;
            }
        }
        ActionKind::SetTheme => {
            // 证明整个业务层本质上只是纯数据。
            state.count = 0;
            state.rows.clear();
        }
        _ => {}
    }
}

unsafe extern "C" fn build_view(state: GuestState, ctx: *mut HostRenderContext) {
    if state.is_null() || ctx.is_null() {
        return;
    }
    // SAFETY: host 传入其拥有的状态及本次调用有效的上下文。
    let state = unsafe { &*state.cast::<State>() };
    let ctx = unsafe { &*ctx };

    let root_desc = RawNodeDesc::text().size(720.0, 480.0).row();
    let root = unsafe { ctx.push(RawNodeId(NO_NODE), &root_desc) };

    let title = format!("{} — {} items", state.title, state.rows.len());
    let title_desc = RawNodeDesc::text().size(720.0, 32.0);
    let title_node = unsafe { ctx.push(root, &title_desc) };
    unsafe { ctx.text(title_node, title.as_ptr(), title.len()) };

    for (index, row) in state.rows.iter().enumerate() {
        let desc = RawNodeDesc::text().size(720.0, 24.0);
        let node = unsafe { ctx.push(root, &desc) };
        let marker = if row.done { "[x]" } else { "[ ]" };
        let label = format!("{marker} {}", row.label);
        unsafe { ctx.text(node, label.as_ptr(), label.len()) };
        ctx.bind(
            node,
            RawAction {
                kind: ActionKind::Custom as u32,
                target: node.0,
                payload: [index as u64, 0, 0],
                timestamp_us: 0,
            },
        );
    }
    ctx.done(root);
}

unsafe extern "C" fn snapshot(state: GuestState, out: *mut ByteBuffer) {
    // SAFETY: 契约同 `migrate_export`。
    unsafe { migrate_export(state, out) };
}

/// host 通过 [`unbound_abi::plugin::GUEST_ENTRY_SYMBOL`] 读取的虚表。
static VTABLE: GuestPluginVTable = GuestPluginVTable {
    abi_version: ABI_VERSION,
    struct_size: core::mem::size_of::<GuestPluginVTable>() as u32,
    state_schema_hash: <State as StateSchema>::SCHEMA_HASH,
    on_load,
    on_unload,
    init_state,
    drop_state,
    adopt_state,
    migrate_export,
    migrate_import,
    reduce,
    build_view,
    snapshot,
};

/// 唯一导出符号。
///
/// # Safety
/// 调用方必须在已加载库的整个生命周期内保持返回指针有效；
/// 它指向 `static`，因此始终有效。
#[no_mangle]
pub extern "C" fn unbound_guest_entry() -> *const GuestPluginVTable {
    &VTABLE
}

/// 供静态链接的 host 与测试使用的虚表。
#[must_use]
pub fn vtable() -> GuestPluginVTable {
    VTABLE
}
