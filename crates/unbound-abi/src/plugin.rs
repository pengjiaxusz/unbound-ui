//! Host ↔ Guest C-ABI。
//!
//! `docs` §8.1 要求严格使用 `#[repr(C)]` 函数指针虚表，使 guest 可在
//! host 保持窗口、Vulkan 设备及内存池存活的同时卸载并重新构建。
//!

use core::ffi::c_void;

use crate::action::RawAction;
use crate::desc::{RawNodeDesc, RawNodeId};

/// 每当本 crate 中任意 POD 的*布局*发生非追加式变更时递增。
/// host 会拒绝加载主版本号不匹配的 guest，而非破坏其堆内存。
///
///
/// 该版本号是单纯计数器，而非 major/minor 组合：guest 要么 ABI 兼容，
/// 要么不兼容。每次布局变更都会递增，host 还会比对 guest 的 `struct_size`，
/// 因此导致结构体增大的追加式变更会由大小检查捕获，而非依赖版本号。
///
pub const ABI_VERSION: u32 = 4;

/// 由 guest 填充的可增长字节缓冲区。
///
/// `Vec<u8>` 不能跨越 ABI 边界：其布局既非 `#[repr(C)]`，也不受 Rust
/// 稳定性保证，不同编译器构建的 guest 对其解释可能不同。内存分配由 host
/// 拥有，guest 仅向其中写入数据。
///
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct ByteBuffer {
    /// 字节指针。`capacity` 为零时为空指针。
    pub ptr: *mut u8,
    /// 缓冲区可容纳的字节数。
    pub capacity: u32,
    /// 已写入的字节数。
    pub len: u32,
}

/// 加载时移交给 guest 的分配器虚表。
///
/// guest 安装一个转发至此虚表的 `#[global_allocator]`，使 `guest.dll` 内
/// 分配的每个 `Box`/`Vec`/`String` 都位于 host 的单一堆（`mimalloc`）中。
/// 这消除了 Windows 跨 CRT 堆损坏这类缺陷（`docs` §8.1）。
///
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct HostAllocatorVTable {
    /// 分配按 `align` 对齐的 `size` 字节内存。失败返回空指针。
    pub alloc: unsafe extern "C" fn(size: usize, align: usize) -> *mut u8,
    /// 释放先前由 `alloc`/`realloc` 返回的内存块。
    pub dealloc: unsafe extern "C" fn(ptr: *mut u8, size: usize, align: usize),
    /// 原地扩缩容或重分配。失败返回空指针（按 `GlobalAlloc` 要求，旧块保持有效）。
    ///
    pub realloc: unsafe extern "C" fn(
        ptr: *mut u8,
        old_size: usize,
        align: usize,
        new_size: usize,
    ) -> *mut u8,
}

/// host 暴露给 guest 的接收端（sink），使 guest 无需分配 host 所属内存或
/// 了解 host 内部类型即可发出视图树。
///
/// 每个函数均为普通 `extern "C"` 指针：无 Rust trait 对象跨越动态库边界，
/// 从而跨编译器版本保持 ABI 稳定。
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct HostRenderContext {
    pub abi_version: u32,
    /// 此结构体的字节大小。
    ///
    /// guest 在读取任何字段前会检查该值：host 可能比 guest 更旧，
    /// 结构体此后可能已扩展。
    pub struct_size: u32,
    /// 每帧由 host 递增；guest 将其用于记忆化视图。
    pub state_generation: u64,
    /// 不透明的 host 指针，原样传回。
    pub ctx: *mut c_void,

    /// 在 `parent` 下追加子节点并返回其 ID。根节点传入 `parent = RawNodeId(NO_NODE)`。
    ///
    pub push_node: unsafe extern "C" fn(
        ctx: *mut c_void,
        parent: RawNodeId,
        desc: *const RawNodeDesc,
    ) -> RawNodeId,
    /// 为文本节点附加 UTF-8 文本（由 host 复制）。
    pub set_text:
        unsafe extern "C" fn(ctx: *mut c_void, node: RawNodeId, ptr: *const u8, len: usize),
    /// 将强类型动作绑定到节点的激活事件。
    pub bind_action: unsafe extern "C" fn(ctx: *mut c_void, node: RawNodeId, action: RawAction),
    /// 声明构建完成并移交根节点。
    pub finish: unsafe extern "C" fn(ctx: *mut c_void, root: RawNodeId),
}

impl HostRenderContext {
    /// 安全封装：在 `parent` 下追加节点。
    ///
    /// # Safety
    /// 调用期间 `desc` 必须指向有效的 `RawNodeDesc`。
    #[inline]
    pub unsafe fn push(&self, parent: RawNodeId, desc: *const RawNodeDesc) -> RawNodeId {
        // SAFETY: 转发至 host 提供的函数，host 会自行校验 `ctx`；
        // 按签名约定，`desc` 由调用方负责保证有效性。
        unsafe { (self.push_node)(self.ctx, parent, desc) }
    }

    /// 安全封装：为节点附加 UTF-8 文本。
    ///
    /// # Safety
    /// `ptr`/`len` 必须描述已初始化的 UTF-8 字节切片。
    #[inline]
    pub unsafe fn text(&self, node: RawNodeId, ptr: *const u8, len: usize) {
        // SAFETY: 同上。
        unsafe { (self.set_text)(self.ctx, node, ptr, len) }
    }

    /// 安全封装：为节点绑定动作。
    #[inline]
    pub fn bind(&self, node: RawNodeId, action: RawAction) {
        // SAFETY: host 负责校验 `ctx`；action 为 POD 值。
        unsafe { (self.bind_action)(self.ctx, node, action) }
    }

    /// 安全封装：完成构建过程。
    #[inline]
    pub fn done(&self, root: RawNodeId) {
        // SAFETY: host 负责校验 `ctx`。
        unsafe { (self.finish)(self.ctx, root) }
    }

    /// 当 host 安装了真实回调*且*该上下文为当前 guest 所理解时返回 true。
    ///
    /// 大小检查至关重要：host 可能比 guest 更旧，上下文结构体此后可能已扩展。
    /// 读取超出 host 结构体末尾的字段将构成未定义行为。
    ///
    ///
    #[inline]
    #[must_use]
    pub fn is_connected(&self) -> bool {
        !self.ctx.is_null()
            && self.abi_version == ABI_VERSION
            && self.struct_size as usize >= core::mem::size_of::<Self>()
    }
}

impl Default for HostRenderContext {
    fn default() -> Self {
        Self {
            abi_version: ABI_VERSION,
            struct_size: core::mem::size_of::<Self>() as u32,
            state_generation: 0,
            ctx: core::ptr::null_mut(),
            push_node: no_push,
            set_text: no_set_text,
            bind_action: no_bind,
            finish: no_finish,
        }
    }
}

unsafe extern "C" fn no_push(
    _ctx: *mut c_void,
    _parent: RawNodeId,
    _desc: *const RawNodeDesc,
) -> RawNodeId {
    RawNodeId(crate::desc::NO_NODE)
}
unsafe extern "C" fn no_set_text(
    _ctx: *mut c_void,
    _node: RawNodeId,
    _ptr: *const u8,
    _len: usize,
) {
}
unsafe extern "C" fn no_bind(_ctx: *mut c_void, _node: RawNodeId, _action: RawAction) {}
unsafe extern "C" fn no_finish(_ctx: *mut c_void, _root: RawNodeId) {}

/// 不透明的 guest 状态句柄。
pub type GuestState = *mut c_void;

/// guest 导出的虚表。通过符号名 [`GUEST_ENTRY_SYMBOL`] 加载。
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GuestPluginVTable {
    pub abi_version: u32,
    /// 此结构体的字节大小。
    ///
    /// host 会在复制虚表*之前*读取它，从而拒绝基于更旧 ABI 构建的 guest
    /// （其结构体更小），避免越界读取。
    ///
    pub struct_size: u32,
    /// 编译期生成的 guest `State` 内存布局指纹。哈希相等即可启用
    /// 零拷贝热重载快路径（`docs` §8.1）。
    pub state_schema_hash: u64,

    pub on_load: unsafe extern "C" fn(host_alloc: *const HostAllocatorVTable),
    pub on_unload: unsafe extern "C" fn(),

    /// 创建全新状态。host 拥有返回指针的所有权，并通过*当前* guest 调用
    /// [`GuestPluginVTable::drop_state`] 加以释放。
    pub init_state: unsafe extern "C" fn() -> GuestState,
    pub drop_state: unsafe extern "C" fn(state: GuestState),

    /// 快路径：原样接管现有状态指针（schema 哈希相同）。
    pub adopt_state: unsafe extern "C" fn(state: GuestState),
    /// 安全路径：将状态导出为自描述二进制缓冲区，供下一代 guest
    /// 逐字段迁移。
    ///
    /// guest 最多写入 `out.capacity` 字节并在 `out.len` 中报告写入量。
    /// 若缓冲区过小，guest 不写入任何数据并将 `out.len` 设为所需大小，
    /// 以便 host 分配更大缓冲区后重试。
    ///
    pub migrate_export: unsafe extern "C" fn(state: GuestState, out: *mut ByteBuffer),
    /// 安全路径：从 [`Self::migrate_export`] 输出重建状态，为旧 schema 中
    /// 不存在的字段应用 `serde` 默认值。
    ///
    /// 当缓冲区无法解码时返回空指针；host 随后会回退到全新状态，
    /// 而非静默丢失数据。
    pub migrate_import: unsafe extern "C" fn(buf_ptr: *const u8, buf_len: usize) -> GuestState,

    pub reduce: unsafe extern "C" fn(state: GuestState, action: *const RawAction),
    pub build_view: unsafe extern "C" fn(state: GuestState, ctx: *mut HostRenderContext),
    /// 序列化完整状态用于时间旅行快照。
    pub snapshot: unsafe extern "C" fn(state: GuestState, out: *mut ByteBuffer),
}

// SAFETY: 该虚表仅由函数指针与整数构成。
unsafe impl Send for GuestPluginVTable {}
unsafe impl Sync for GuestPluginVTable {}

impl core::fmt::Debug for GuestPluginVTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GuestPluginVTable")
            .field("abi_version", &self.abi_version)
            .field(
                "state_schema_hash",
                &format_args!("{:#018x}", self.state_schema_hash),
            )
            .finish_non_exhaustive()
    }
}

/// 每个 guest `cdylib` 必须提供的唯一导出符号名。
pub const GUEST_ENTRY_SYMBOL: &[u8] = b"unbound_guest_entry\0";

/// [`GUEST_ENTRY_SYMBOL`] 的签名。
pub type GuestEntryFn = unsafe extern "C" fn() -> *const GuestPluginVTable;

/// 在调用任何虚表字段前执行的兼容性检查。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AbiCompat {
    /// 主版本相同：可安全调用。
    Ok,
    /// host 比 guest 构建版本更旧。
    HostTooOld { host: u32, guest: u32 },
    /// guest 比 host 构建版本更旧。
    GuestTooOld { host: u32, guest: u32 },
}

#[must_use]
pub fn check_abi(guest_version: u32) -> AbiCompat {
    if guest_version == ABI_VERSION {
        AbiCompat::Ok
    } else if guest_version > ABI_VERSION {
        AbiCompat::HostTooOld {
            host: ABI_VERSION,
            guest: guest_version,
        }
    } else {
        AbiCompat::GuestTooOld {
            host: ABI_VERSION,
            guest: guest_version,
        }
    }
}

/// 重载后 host 对 guest 状态应采取的处理方式。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StateHandoff {
    /// schema 哈希匹配：直接移交原始指针，零拷贝。
    ZeroCopy,
    /// schema 已变更：通过 `migrate_export`/`migrate_import` 往返迁移。
    Migrate,
    /// 无先前状态（首次加载）。
    Fresh,
}

/// 仅根据两个 schema 哈希决定移交模式。
#[must_use]
pub fn classify_handoff(old_hash: Option<u64>, new_hash: u64) -> StateHandoff {
    match old_hash {
        None => StateHandoff::Fresh,
        Some(h) if h == new_hash => StateHandoff::ZeroCopy,
        Some(_) => StateHandoff::Migrate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_checks() {
        assert_eq!(check_abi(ABI_VERSION), AbiCompat::Ok);
        assert!(matches!(
            check_abi(ABI_VERSION + 1),
            AbiCompat::HostTooOld { .. }
        ));
        assert!(matches!(
            check_abi(ABI_VERSION - 1),
            AbiCompat::GuestTooOld { .. }
        ));
    }

    #[test]
    fn handoff_classification() {
        assert_eq!(classify_handoff(None, 42), StateHandoff::Fresh);
        assert_eq!(classify_handoff(Some(42), 42), StateHandoff::ZeroCopy);
        assert_eq!(classify_handoff(Some(41), 42), StateHandoff::Migrate);
    }

    #[test]
    fn default_context_is_inert_but_callable() {
        let ctx = HostRenderContext::default();
        assert_eq!(ctx.abi_version, ABI_VERSION);
        // # Safety: 默认函数在构造上均为无操作（no-op）。
        assert!(!ctx.is_connected());
        unsafe {
            assert!(ctx.push(RawNodeId::ROOT, core::ptr::null()).is_none());
            ctx.text(RawNodeId::ROOT, core::ptr::null(), 0);
        }
        ctx.bind(RawNodeId::ROOT, RawAction::NONE);
        ctx.done(RawNodeId::ROOT);
    }

    #[test]
    fn entry_symbol_is_nul_terminated() {
        assert_eq!(*GUEST_ENTRY_SYMBOL.last().unwrap(), 0);
        assert_eq!(
            &GUEST_ENTRY_SYMBOL[..GUEST_ENTRY_SYMBOL.len() - 1],
            b"unbound_guest_entry"
        );
    }
}
