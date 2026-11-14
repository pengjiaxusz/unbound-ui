//! 宿主侧热重载（`docs` §8.1）。
//!
//! host 在卸载、重编译并重新挂载 guest（即整个业务层）期间保持窗口、
//! Vulkan 设备与持久内存池存活。三项机制保证了该过程的安全性：
//!
//!
//! 1. **带版本检查的 `#[repr(C)]` 虚表**（[`unbound_abi::plugin`]）：
//!    版本不匹配的 guest 会被直接拒绝，绝不调用。
//! 2. **单一堆。** host 向 guest 传入分配器虚表，guest 安装转发至此的
//!    `#[global_allocator]`，从而使 guest 构建的 `Vec<u8>` 可由 host 释放
//!    （消除经典的 Windows 跨 CRT 堆损坏缺陷）。
//!
//! 3. **基于 schema 哈希的状态移交决策**，而非凭空猜测：
//!    哈希相等则直接接管旧指针，哈希不同则经由自描述缓冲区往返迁移。
//!
//!
//! 在 Windows 上已加载的 `.dll` 会被锁定，重编译无法覆盖它。
//! 因此 [`GuestHost::reload`] 会先将新产物影子拷贝（shadow-copy）到唯一临时路径，
//! 即文档所述的“Shadow Copy 自动重挂载”。

#![forbid(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;
use std::alloc::GlobalAlloc as _;
use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};
use rustc_hash::FxHashMap;
use unbound_abi::action::RawAction;
use unbound_abi::desc::{RawNodeDesc, RawNodeId};
use unbound_abi::plugin::{
    check_abi, classify_handoff, AbiCompat, ByteBuffer, GuestEntryFn, GuestPluginVTable,
    GuestState, HostAllocatorVTable, HostRenderContext, ABI_VERSION, GUEST_ENTRY_SYMBOL,
};

pub use unbound_abi::plugin::{StateHandoff, ABI_VERSION as GUEST_ABI_VERSION};

/// 挂载 guest 时可能发生的各类错误。
#[derive(Debug)]
pub enum HostError {
    /// 无法打开动态库。
    Load(libloading::Error),
    /// 动态库未导出 [`GUEST_ENTRY_SYMBOL`]。
    MissingEntry,
    /// guest 基于不兼容的 ABI 构建。
    Abi(AbiCompat),
    /// 影子拷贝时发生文件系统错误。
    Io(std::io::Error),
    /// 当前未挂载任何 guest。
    NoGuest,
}

impl core::fmt::Display for HostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Load(error) => write!(f, "failed to load guest library: {error}"),
            Self::MissingEntry => write!(f, "guest does not export unbound_guest_entry"),
            Self::Abi(compat) => write!(f, "guest ABI mismatch: {compat:?}"),
            Self::Io(error) => write!(f, "shadow copy failed: {error}"),
            Self::NoGuest => write!(f, "no guest is mounted"),
        }
    }
}

impl std::error::Error for HostError {}

impl From<libloading::Error> for HostError {
    fn from(error: libloading::Error) -> Self {
        Self::Load(error)
    }
}

impl From<std::io::Error> for HostError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// guest 请求 host 创建的单个节点。
#[derive(Clone, Debug)]
pub struct HostNode {
    pub id: RawNodeId,
    pub parent: RawNodeId,
    pub desc: RawNodeDesc,
    pub text: String,
    pub action: Option<RawAction>,
    pub children: Vec<RawNodeId>,
}

/// 由 guest 的 `build_view` 过程填充的扁平节点 arena。
#[derive(Default, Debug)]
pub struct NodeArena {
    nodes: Vec<HostNode>,
    index: FxHashMap<u32, usize>,
    roots: Vec<RawNodeId>,
    finished_root: Option<RawNodeId>,
}

impl NodeArena {
    /// 丢弃上一帧数据但保留底层内存分配。
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.index.clear();
        self.roots.clear();
        self.finished_root = None;
    }

    /// 本帧构建的节点数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 按构建顺序排列的节点。
    #[must_use]
    pub fn nodes(&self) -> &[HostNode] {
        &self.nodes
    }

    /// 根节点列表（以空父节点推入的节点）。
    #[must_use]
    pub fn roots(&self) -> &[RawNodeId] {
        &self.roots
    }

    /// 由 `finish` 移交的根节点。
    #[must_use]
    pub fn finished_root(&self) -> Option<RawNodeId> {
        self.finished_root
    }

    #[must_use]
    pub fn get(&self, id: RawNodeId) -> Option<&HostNode> {
        self.index.get(&id.0).map(|slot| &self.nodes[*slot])
    }

    fn push(&mut self, parent: RawNodeId, desc: RawNodeDesc) -> RawNodeId {
        // ID 从 1 开始：`RawNodeId::ROOT` (0) 是“虚拟”根，guest 可将其
        // 用作父节点而无需自行持有对应节点。
        let id = RawNodeId(self.nodes.len() as u32 + 1);
        if parent.is_none() || parent == RawNodeId::ROOT {
            // `ROOT` 是合法父节点：该节点成为树的根节点。
            // 过去会落入 `else` 分支并查找 ID 0（必然失败）——导致节点既不在
            // 根列表中也不在任何子节点列表中，静默从树中消失。
            //
            self.roots.push(id);
        } else if let Some(slot) = self.index.get(&parent.0).copied() {
            self.nodes[slot].children.push(id);
        }
        self.index.insert(id.0, self.nodes.len());
        self.nodes.push(HostNode {
            id,
            parent,
            desc,
            text: String::new(),
            action: None,
            children: Vec::new(),
        });
        id
    }

    fn set_text(&mut self, id: RawNodeId, text: &str) {
        if let Some(slot) = self.index.get(&id.0).copied() {
            self.nodes[slot].text.clear();
            self.nodes[slot].text.push_str(text);
        }
    }

    fn bind(&mut self, id: RawNodeId, action: RawAction) {
        if let Some(slot) = self.index.get(&id.0).copied() {
            self.nodes[slot].action = Some(action);
        }
    }

    fn finish(&mut self, root: RawNodeId) {
        self.finished_root = Some(root);
    }
}

/// host 对单个已挂载 guest 的视图。
pub struct GuestHost {
    library: Option<Library>,
    vtable: Option<GuestPluginVTable>,
    state: GuestState,
    schema_hash: Option<u64>,
    /// 虚表生命周期必须长于 guest：静态链接的 guest 将指针保存在进程级静态变量中，
    /// 因此 host 绝不能传出指向自身栈帧的指针。
    ///
    ///
    /// 此处的内存泄漏是刻意为之。guest 在挂载期间持续缓存该指针，且静态链接的
    // guest 在进程内所有 host 实例间共享同一静态变量——因此该地址必须在整个
    // 进程生命周期内保持有效，而非仅限单次挂载。将虚表存放在 host 内部的 `Box`
    // 中行不通：box 会随 host 释放，下一个 host 的分配便会经过悬垂指针。
    ///
    ///
    alloc: &'static HostAllocatorVTable,
    arena: NodeArena,
    generation: u64,
    loads: u32,
    shadow_counter: u32,
    /// 本 host 创建的影子拷贝列表，在 guest 卸载时删除。
    ///
    /// 若不清理，长时间开发会话经历数百次重载后会在临时目录留下数百份 DLL 副本。
    ///
    shadows: Vec<PathBuf>,
    /// 当前已加载库所源自的影子拷贝。
    ///
    /// 仅此文件在库加载期间不可删除：只要动态库处于映射状态，文件即被锁定。
    ///
    library_shadow: Option<PathBuf>,
}

impl core::fmt::Debug for GuestHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GuestHost")
            .field("mounted", &self.vtable.is_some())
            .field("schema_hash", &self.schema_hash)
            .field("generation", &self.generation)
            .field("loads", &self.loads)
            .field("nodes", &self.arena.len())
            .finish_non_exhaustive()
    }
}

impl Default for GuestHost {
    fn default() -> Self {
        Self::new()
    }
}

impl GuestHost {
    /// 使用默认（系统）分配器虚表创建空 host。
    #[must_use]
    pub fn new() -> Self {
        Self {
            library: None,
            vtable: None,
            state: core::ptr::null_mut(),
            schema_hash: None,
            alloc: &SYSTEM_ALLOCATOR,
            arena: NodeArena::default(),
            generation: 0,
            loads: 0,
            shadow_counter: 0,
            shadows: Vec::new(),
            library_shadow: None,
        }
    }

    /// 创建由 `alloc` 提供 guest 分配服务的 host。
    ///
    /// 虚表被刻意泄漏——原因见字段文档：guest 将指针保存在进程级静态变量中。
    ///
    #[must_use]
    pub fn with_allocator(alloc: HostAllocatorVTable) -> Self {
        let mut host = Self::new();
        host.alloc = Box::leak(Box::new(alloc));
        host
    }

    /// 先影子拷贝再从动态库挂载 guest。
    ///
    /// # Errors
    /// 当拷贝、加载、ABI 检查或 guest 自身的 `on_load` 失败时返回 [`HostError`]。
    ///
    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<(), HostError> {
        let shadow = self.shadow_copy(path.as_ref())?;
        // SAFETY: 路径为 cargo 生成的有效动态库；只要使用虚表，库就保持存活。
        //
        let library = unsafe { Library::new(&shadow)? };
        let vtable = unsafe { read_vtable(&library)? };
        self.mount(library, vtable)?;
        self.library_shadow = Some(shadow);
        Ok(())
    }

    /// 挂载已在内存中的虚表（用于测试、静态链接应用）。
    ///
    /// # Errors
    /// 版本不匹配时返回 [`HostError::Abi`]。
    pub fn mount_vtable(&mut self, vtable: GuestPluginVTable) -> Result<(), HostError> {
        self.mount_linked(vtable)
    }

    fn mount(&mut self, library: Library, vtable: GuestPluginVTable) -> Result<(), HostError> {
        self.mount_linked(vtable)?;
        self.library = Some(library);
        Ok(())
    }

    fn mount_linked(&mut self, vtable: GuestPluginVTable) -> Result<(), HostError> {
        let compat = check_abi(vtable.abi_version);
        if compat != AbiCompat::Ok {
            return Err(HostError::Abi(compat));
        }
        self.unmount();
        unsafe {
            // SAFETY: ABI 版本已匹配，因此签名与己方一致。
            (vtable.on_load)(self.alloc);
            self.state = (vtable.init_state)();
        }
        self.schema_hash = Some(vtable.state_schema_hash);
        self.vtable = Some(vtable);
        self.loads += 1;
        Ok(())
    }

    /// 从新构建的产物热重载。
    ///
    /// 新 guest 通过验证前旧 guest 保持挂载，随后按 [`classify_handoff`] 移交状态。
    ///
    ///
    /// # Errors
    /// 失败原因同 [`Self::load`]；出错时旧 guest 保持原样不受影响。
    ///
    pub fn reload(&mut self, path: impl AsRef<Path>) -> Result<StateHandoff, HostError> {
        let shadow = self.shadow_copy(path.as_ref())?;
        // SAFETY: 见 `load`。
        let library = unsafe { Library::new(&shadow)? };
        let fresh = unsafe { read_vtable(&library)? };
        let handoff = self.reload_vtable(fresh)?;
        // 旧库已由 `reload_vtable` 卸载，因此旧影子拷贝已解锁并可删除。
        //
        self.release_library_shadow();
        self.library = Some(library);
        self.library_shadow = Some(shadow);
        Ok(handoff)
    }

    /// 对已读取的虚表执行状态移交。
    ///
    /// 从 [`Self::reload`] 拆分出来，使静态链接的 guest（及测试）无需动态库
    /// 即可演练同一代码路径。
    ///
    /// # Errors
    /// 当新 guest 基于其他 ABI 版本构建时返回 [`HostError::Abi`]；此时旧 guest 保持挂载。
    ///
    pub fn reload_vtable(&mut self, fresh: GuestPluginVTable) -> Result<StateHandoff, HostError> {
        let compat = check_abi(fresh.abi_version);
        if compat != AbiCompat::Ok {
            return Err(HostError::Abi(compat));
        }
        let handoff = classify_handoff(self.schema_hash, fresh.state_schema_hash);

        let old = self.vtable;
        let old_state = core::mem::replace(&mut self.state, core::ptr::null_mut());
        unsafe {
            // 新 guest 的分配器必须在它分配任何内存*之前*安装：`init_state` 与
            // `migrate_import` 均通过 guest 的全局分配器分配，该分配器转发至 host 虚表。
            // 若未安装转发表，guest 将回退到另一个堆，后续通过 host 分配器释放状态
            // 时便会发生跨分配器释放。
            //
            //
            (fresh.on_load)(self.alloc);
            // SAFETY: `old` 与 `old_state` 配对，且此时旧库仍处于加载状态。
            //
            match (handoff, old) {
                (StateHandoff::ZeroCopy, Some(_old)) => {
                    // 布局相同：直接移交指针。绝对不能运行旧的 `drop_state`——
                    // 新 guest 现在拥有它——host 则继续使用同一指针。
                    //
                    (fresh.adopt_state)(old_state);
                    self.state = old_state;
                }
                (StateHandoff::Migrate, Some(old)) => {
                    // 迁移通过 *migration* 钩子导出，而非 `snapshot`：
                    // guest 可在导出时重塑状态以匹配新 schema。
                    //
                    let buffer = export_state(old_state, old.migrate_export);
                    (old.drop_state)(old_state);
                    // 旧 guest 已完成使命：在卸载其动态库前先让它释放所持资源。
                    //
                    (old.on_unload)();
                    self.state = (fresh.migrate_import)(buffer.as_ptr(), buffer.len());
                    if self.state.is_null() {
                        // guest 无法解码缓冲区。回退到全新状态比向 host 交出空指针
                        // 导致解引用崩溃要安全得多。
                        //
                        self.state = (fresh.init_state)();
                    }
                }
                _ => {
                    if let Some(old) = old {
                        (old.drop_state)(old_state);
                        (old.on_unload)();
                    }
                    self.state = (fresh.init_state)();
                }
            }
        }
        // 仅当确认不再有任何调用会进入旧产物时才释放它。
        self.library = None;
        self.vtable = Some(fresh);
        self.schema_hash = Some(fresh.state_schema_hash);
        self.loads += 1;
        self.arena.clear();
        Ok(handoff)
    }

    /// 卸载 guest，并通过其自身虚表释放其状态。
    pub fn unmount(&mut self) {
        if let Some(vtable) = self.vtable.take() {
            unsafe {
                // SAFETY: 虚表存活且 `state` 由其生成。
                if !self.state.is_null() {
                    (vtable.drop_state)(self.state);
                }
                (vtable.on_unload)();
            }
            self.state = core::ptr::null_mut();
        }
        self.library = None;
        self.schema_hash = None;
        self.remove_shadows();
    }

    /// 删除本 host 创建的影子拷贝。
    ///
    /// 此时动态库已卸载，文件不再被锁定。
    fn remove_shadows(&mut self) {
        for path in self.shadows.drain(..) {
            let _ = std::fs::remove_file(path);
        }
        self.library_shadow = None;
    }

    /// 删除已卸载库所源自的影子拷贝。
    fn release_library_shadow(&mut self) {
        if let Some(path) = self.library_shadow.take() {
            let _ = std::fs::remove_file(&path);
            self.shadows.retain(|p| p != &path);
        }
    }

    /// 已挂载 guest 时返回 true。
    #[must_use]
    pub fn is_mounted(&self) -> bool {
        self.vtable.is_some()
    }

    /// 当前已挂载 guest 的 schema 哈希。
    #[must_use]
    pub fn schema_hash(&self) -> Option<u64> {
        self.schema_hash
    }

    /// guest 被挂载的累计次数。
    #[must_use]
    pub fn loads(&self) -> u32 {
        self.loads
    }

    /// 上一次 [`Self::build`] 构建的节点。
    #[must_use]
    pub fn arena(&self) -> &NodeArena {
        &self.arena
    }

    /// 向 guest 分发单个动作。
    ///
    /// # Errors
    /// 未挂载时返回 [`HostError::NoGuest`]。
    pub fn reduce(&mut self, action: &RawAction) -> Result<(), HostError> {
        let Some(vtable) = self.vtable else {
            return Err(HostError::NoGuest);
        };
        unsafe {
            // SAFETY: 虚表存活且状态归属于它。
            (vtable.reduce)(self.state, action);
        }
        Ok(())
    }

    /// 运行 guest 的视图构建并将结果写入全新 arena。
    ///
    /// # Errors
    /// 未挂载时返回 [`HostError::NoGuest`]。
    pub fn build(&mut self) -> Result<(), HostError> {
        let Some(vtable) = self.vtable else {
            return Err(HostError::NoGuest);
        };
        self.arena.clear();
        self.generation += 1;
        let mut context = HostRenderContext {
            abi_version: ABI_VERSION,
            struct_size: core::mem::size_of::<HostRenderContext>() as u32,
            state_generation: self.generation,
            ctx: core::ptr::addr_of_mut!(self.arena).cast::<c_void>(),
            push_node: host_push_node,
            set_text: host_set_text,
            bind_action: host_bind_action,
            finish: host_finish,
        };
        unsafe {
            // SAFETY: 回调仅访问我们刚刚传入的 arena 指针，且 `self.arena` 的借用
            // 不会逃逸出此代码块。
            (vtable.build_view)(self.state, &mut context);
        }
        Ok(())
    }

    /// 向 guest 请求二进制状态快照。
    ///
    /// # Errors
    /// 未挂载时返回 [`HostError::NoGuest`]。
    pub fn snapshot(&mut self) -> Result<Vec<u8>, HostError> {
        let Some(vtable) = self.vtable else {
            return Err(HostError::NoGuest);
        };
        Ok(export_state(self.state, vtable.snapshot))
    }

    /// 将 `path` 拷贝到唯一临时文件，以便原文件可被重新编译。
    ///
    /// 拷贝路径会被记录下来，以便在卸载 guest 时删除：
    /// 否则经历数百次重载的长开发会话会在临时目录留下数百份 DLL 副本。
    ///
    fn shadow_copy(&mut self, path: &Path) -> Result<PathBuf, HostError> {
        if !path.exists() {
            // 无需拷贝（例如测试夹具）；交由加载器报告错误。
            return Ok(path.to_path_buf());
        }
        self.shadow_counter += 1;
        let file_name = path.file_name().map_or_else(
            || "guest.dll".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let shadow = std::env::temp_dir().join(format!(
            "unbound-shadow-{}-{}-{file_name}",
            std::process::id(),
            self.shadow_counter
        ));
        std::fs::copy(path, &shadow)?;
        self.shadows.push(shadow.clone());
        Ok(shadow)
    }
}

impl Drop for GuestHost {
    fn drop(&mut self) {
        self.unmount();
    }
}

/// 从刚打开的库中读取 guest 虚表。
///
/// 必须*先*从结构体头部读取 ABI 版本与结构体大小，仅当两者均校验通过后
/// 才复制完整虚表。若先复制，对于基于更旧 ABI 构建的 guest（其虚表更小），
/// 在 host 拒绝前就会发生越界读取。
///
///
/// # Safety
/// 在使用返回虚表的整个期间 `library` 必须保持加载状态，且必须按约定签名
/// 导出 [`GUEST_ENTRY_SYMBOL`]。
unsafe fn read_vtable(library: &Library) -> Result<GuestPluginVTable, HostError> {
    // 符号名不包含 ABI 常量所携带的末尾 NUL。
    let name = &GUEST_ENTRY_SYMBOL[..GUEST_ENTRY_SYMBOL.len() - 1];
    let entry: Symbol<'_, GuestEntryFn> =
        unsafe { library.get(name) }.map_err(|_| HostError::MissingEntry)?;
    let pointer = unsafe { entry() };
    if pointer.is_null() {
        return Err(HostError::MissingEntry);
    }
    // 前两个字为版本号与大小。每个 ABI 版本都包含这两个字段，
    // 因此读取它们绝不会越过 guest 结构体末尾。
    //
    let header = unsafe { *(pointer.cast::<VTableHeader>()) };
    let compat = check_abi(header.abi_version);
    if compat != AbiCompat::Ok {
        return Err(HostError::Abi(compat));
    }
    let expected = core::mem::size_of::<GuestPluginVTable>();
    if (header.struct_size as usize) < expected {
        return Err(HostError::Abi(AbiCompat::GuestTooOld {
            host: ABI_VERSION,
            guest: header.abi_version,
        }));
    }
    // 有意接受虚表*大于*己方的 guest：多余成员位于末尾，仅复制本 host
    // 已知的字段依然正确——这正是向前兼容的情形。
    // 拒绝虚表更小的 guest：复制操作会越过其末尾。
    // 若在*中间*插入成员会移动后续所有字段，任何大小检查都无法捕获——
    // 因此每次布局变更都必须递增 `ABI_VERSION`。
    //
    //
    Ok(unsafe { *pointer })
}

/// [`GuestPluginVTable`] 的前导字，在读取整个结构体前先读取。
///
/// 将其置于独立的 `#[repr(C)]` 结构体中可显式固定版本号与大小的偏移量，
/// 并保证读取它们不会触及 guest 虚表之外的任何内存。
///
#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct VTableHeader {
    abi_version: u32,
    struct_size: u32,
}

/// 将 guest 状态导出到 host 拥有的字节缓冲区。
///
/// guest 仅向缓冲区写入，从不自行分配。缓冲区通过询问 guest 所需空间来增长，
/// 因此不涉及 guest 分配器，host 可用自己的分配器释放该内存。
///
///
/// `export` 选择要运行的 guest 回调，两个调用方需要不同的回调：
///
///
/// * [`GuestPluginVTable::snapshot`] 用于普通的时间旅行快照；
/// * [`GuestPluginVTable::migrate_export`] 用于将字节移交至下一代——
///   迁移必须经由导出钩子，以便 guest 为新 schema 重塑状态。
///
///
/// 回调运行两次：首次探测所需大小（不提供缓冲区，guest 仅报告所需长度），
/// 第二次填充 host 为其分配的缓冲区。
///
fn export_state(
    state: GuestState,
    export: unsafe extern "C" fn(GuestState, *mut ByteBuffer),
) -> Vec<u8> {
    let mut buffer: Vec<u8> = Vec::new();
    // 缓冲区过小时，guest 报告其所需大小。
    let needed: u32 = unsafe {
        let mut view = ByteBuffer {
            ptr: core::ptr::null_mut(),
            capacity: 0,
            len: 0,
        };
        export(state, &mut view);
        view.len
    };
    if needed == 0 {
        return buffer;
    }
    buffer.resize(needed as usize, 0);
    unsafe {
        let mut view = ByteBuffer {
            ptr: buffer.as_mut_ptr(),
            capacity: needed,
            len: 0,
        };
        export(state, &mut view);
        buffer.truncate(view.len as usize);
    }
    buffer
}

// ---- host → guest 回调 -------------------------------------------------

unsafe extern "C" fn host_push_node(
    ctx: *mut c_void,
    parent: RawNodeId,
    desc: *const RawNodeDesc,
) -> RawNodeId {
    if ctx.is_null() || desc.is_null() {
        return RawNodeId::ROOT;
    }
    let arena = unsafe { &mut *ctx.cast::<NodeArena>() };
    let desc = unsafe { *desc };
    arena.push(parent, desc)
}

unsafe extern "C" fn host_set_text(ctx: *mut c_void, node: RawNodeId, ptr: *const u8, len: usize) {
    if ctx.is_null() || (ptr.is_null() && len > 0) {
        return;
    }
    let arena = unsafe { &mut *ctx.cast::<NodeArena>() };
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(ptr, len) }
    };
    if let Ok(text) = core::str::from_utf8(bytes) {
        arena.set_text(node, text);
    }
}

unsafe extern "C" fn host_bind_action(ctx: *mut c_void, node: RawNodeId, action: RawAction) {
    if ctx.is_null() {
        return;
    }
    let arena = unsafe { &mut *ctx.cast::<NodeArena>() };
    arena.bind(node, action);
}

unsafe extern "C" fn host_finish(ctx: *mut c_void, root: RawNodeId) {
    if ctx.is_null() {
        return;
    }
    let arena = unsafe { &mut *ctx.cast::<NodeArena>() };
    arena.finish(root);
}

// ---- 默认分配器 ------------------------------------------------------

/// 由系统分配器支撑的分配器虚表。
///
/// 替换为 `mimalloc` 只需一行改动：guest 始终只看到这三个函数指针。
///
#[must_use]
pub fn system_allocator() -> HostAllocatorVTable {
    SYSTEM_ALLOCATOR
}

/// 通过 `System` 而非 `std::alloc::alloc` 访问进程堆。
///
/// `std::alloc::alloc` 会路由至*二进制文件*的 `#[global_allocator]`，而在进程内
/// 测试中那正是 guest 自身的转发分配器——从而造成无限递归。
/// `System` 始终直接与操作系统交互，这正是架构所需的单一堆。
///
const SYSTEM: std::alloc::System = std::alloc::System;

unsafe extern "C" fn sys_alloc(size: usize, align: usize) -> *mut u8 {
    let Ok(layout) = core::alloc::Layout::from_size_align(size.max(1), align.max(1)) else {
        return core::ptr::null_mut();
    };
    unsafe { SYSTEM.alloc(layout) }
}

unsafe extern "C" fn sys_dealloc(ptr: *mut u8, size: usize, align: usize) {
    if ptr.is_null() {
        return;
    }
    let Ok(layout) = core::alloc::Layout::from_size_align(size.max(1), align.max(1)) else {
        return;
    };
    unsafe { SYSTEM.dealloc(ptr, layout) }
}

unsafe extern "C" fn sys_realloc(
    ptr: *mut u8,
    old_size: usize,
    align: usize,
    new_size: usize,
) -> *mut u8 {
    let Ok(layout) = core::alloc::Layout::from_size_align(old_size.max(1), align.max(1)) else {
        return core::ptr::null_mut();
    };
    if ptr.is_null() {
        return unsafe { sys_alloc(new_size, align) };
    }
    unsafe { SYSTEM.realloc(ptr, layout, new_size.max(1)) }
}

/// 提升为 `'static` 的默认分配器虚表，供 guest 缓存使用。
static SYSTEM_ALLOCATOR: HostAllocatorVTable = HostAllocatorVTable {
    alloc: sys_alloc,
    dealloc: sys_dealloc,
    realloc: sys_realloc,
};

#[cfg(test)]
mod tests;
