//! lang-allow: 中文注释与文档
//!
//! 确定性分层键作用域（Key Scope）与类型化 Action 交互接缝。
//!
//! 消除动态列表与组件嵌套中的手动基址算术（如 `10_000 + idx * 30 + offset`），
//! 在声明期（`view`）与归约期（`reduce`）提供确定性、无碰撞的命名空间推导与强类型 Action 分发。

use crate::{mix_key, KeyDiscriminator};

/// 确定性分层键作用域（Key Scope）。
///
/// 封装 64 位命名空间哈希，基于 FNV-1a 提供零堆分配、常数开销的层级键推导。
///
/// # 核心价值
/// 1. **消除乘法算术键**：在循环或树形结构中，调用方无需手动维护 `base + idx * 30 + offset`。
/// 2. **跨生命周期一致性**：在 `view()` 声明期与 `reduce()` 归约期，相同的路径保证推导出完全相同的 64 位键。
/// 3. **零碰撞保障**：每一层作用域将父哈希与判别符混合，产生高雪崩效应的 64 位整型空间。
///
/// # 示例
///
/// ```rust,no_run
/// use unbound_app::KeyScope;
///
/// let category_scope = KeyScope::new("settings");
/// let item_scope = category_scope.scope(0);
/// let button_key = item_scope.key("save");
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct KeyScope {
    scope: u64,
}

impl KeyScope {
    /// 根级作用域（无前缀，保持原始键不变）。
    #[must_use]
    pub const fn root() -> Self {
        Self { scope: 0 }
    }

    /// 从原始 64 位数值包装作用域。
    #[must_use]
    pub const fn from_raw(scope: u64) -> Self {
        Self { scope }
    }

    /// 从判别值创建新的一级作用域。
    #[must_use]
    pub fn new(discriminator: impl KeyDiscriminator) -> Self {
        Self {
            scope: Self::derive(0, discriminator),
        }
    }

    /// 编译期常量推导作用域哈希。
    #[must_use]
    pub const fn const_derive(parent_scope: u64, discriminator: u64) -> u64 {
        const FNV_OFFSET_BASIS: u64 = 0xCBF2_9CE4_8422_2325;
        const FNV_PRIME: u64 = 0x100000001b3;
        let base = if parent_scope == 0 {
            FNV_OFFSET_BASIS
        } else {
            parent_scope
        };
        let mut hash = base ^ discriminator;
        hash = hash.wrapping_mul(FNV_PRIME);
        if hash == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            hash
        }
    }

    /// 编译期常量推导叶子键。
    #[must_use]
    pub const fn const_key(scope: u64, discriminator: u64) -> u64 {
        if scope == 0 {
            discriminator
        } else {
            mix_key(scope, discriminator)
        }
    }

    /// 纯函数：将父作用域哈希与新判别值混合生成新的非零作用域哈希。
    #[inline]
    pub fn derive(parent_scope: u64, discriminator: impl KeyDiscriminator) -> u64 {
        Self::const_derive(parent_scope, discriminator.into_u64())
    }

    /// 基于当前作用域派生子作用域。
    #[must_use]
    pub fn scope(&self, discriminator: impl KeyDiscriminator) -> Self {
        Self {
            scope: Self::derive(self.scope, discriminator),
        }
    }

    /// 在当前作用域下推导叶子控件键。
    ///
    /// 若当前处于根作用域（`scope == 0`），则直接返回判别值的数值；
    /// 否则混合当前作用域哈希与子判别值，生成无碰撞的 64 位全局键。
    #[must_use]
    pub fn key(&self, discriminator: impl KeyDiscriminator) -> u64 {
        let disc = discriminator.into_u64();
        if self.scope == 0 {
            disc
        } else {
            mix_key(self.scope, disc)
        }
    }

    /// 提取作用域的底层 64 位哈希值。
    #[must_use]
    pub const fn raw(&self) -> u64 {
        self.scope
    }

    /// 是否处于根作用域（无前缀）。
    #[must_use]
    pub const fn is_root(&self) -> bool {
        self.scope == 0
    }

    /// 检查指定的节点键是否属于该作用域下的某个判别值。
    #[inline]
    #[must_use]
    pub fn matches(&self, node: u64, discriminator: impl KeyDiscriminator) -> bool {
        self.key(discriminator) == node
    }

    /// 在 `0..count` 连续索引中查找匹配 `node` 的索引项。
    #[must_use]
    pub fn find_index(&self, node: u64, count: usize) -> Option<usize> {
        (0..count).find(|&idx| self.key(idx) == node)
    }
}
