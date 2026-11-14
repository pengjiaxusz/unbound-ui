//! 编译期状态 schema 指纹生成。
//!
//! 热重载快路径（`docs` §8.1）会比对 guest `State` 内存布局的 64 位哈希。
//! 哈希匹配时，旧 `State` 会原样移交给新的 `guest_gen_N.dll`——完全无需序列化。
//! 若不一致，host 则走 `migrate_export`/`migrate_import` 往返迁移，避免崩溃。
//!
//!
//!
//! 该哈希基于 `(field_name, type_name, offset)` 三元组按声明顺序计算 FNV-1a。
//! 整个计算由 [`impl_schema_hash!`](crate::impl_schema_hash) 宏在 `const` 上下文中完成，
//! 运行时零开销。
//!

/// FNV-1a 64 位偏移基数。
pub const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 位质数。
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 将字节切片混入当前 FNV-1a 哈希状态。
#[inline]
#[must_use]
pub const fn hash_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

/// 将 `&str` 混入当前哈希状态。
#[inline]
#[must_use]
pub const fn hash_str(hash: u64, s: &str) -> u64 {
    hash_bytes(hash, s.as_bytes())
}

/// 将 `u64` 混入当前哈希状态（小端序，顺序敏感）。
#[inline]
#[must_use]
pub const fn hash_u64(hash: u64, v: u64) -> u64 {
    hash_bytes(hash, &v.to_le_bytes())
}

/// 将字段描述符（名称、类型与字节偏移）混入哈希。
#[inline]
#[must_use]
pub const fn hash_field(hash: u64, name: &str, type_name: &str, offset: usize) -> u64 {
    let h = hash_str(hash, name);
    let h = hash_str(h, "\u{1f}");
    let h = hash_str(h, type_name);
    let h = hash_str(h, "\u{1f}");
    hash_u64(h, offset as u64)
}

/// 将嵌套类型自身的 schema 哈希混入父类型哈希。
///
/// 否则嵌套类型*内部*的变更（例如向 `Vec` 元素类型添加字段）将不会改变
/// 父类型指纹，导致 host 把旧内存当成布局仍匹配直接交给新 guest。
///
///
#[inline]
#[must_use]
pub const fn hash_nested(hash: u64, nested: u64) -> u64 {
    hash_u64(hash, nested)
}

/// 从进行中的状态生成最终哈希值。
#[inline]
#[must_use]
pub const fn finish_hash(hash: u64) -> u64 {
    // 最后一次雪崩（avalanche）步骤，确保仅单字节差异的小范围字段重排
    // 也能均匀扩散到整个 64 位空间。
    let mut h = hash;
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h
}

/// 由每个参与热重载的 guest `State` 实现。
///
/// # 布局必须固定
///
/// 指纹相等意味着“host 可将旧状态原样交给新 guest”。
/// 仅当两次构建就*字节*布局达成一致时才安全，因此状态类型必须具备
/// 固定布局：`#[repr(C)]`，或围绕单个字段的 `#[repr(transparent)]`。
///
///
/// 在默认的 `repr(Rust)` 下编译器可以（且确实会）重排字段，
/// 不同编译器版本的重排方式也可能不同。基于字段列表手写的指纹无法检测
/// 此类漂移：它会误判两个实际布局不同的状态为“兼容”。
///
///
pub trait StateSchema {
    /// 内存布局指纹。值相等即表示“二进制兼容”。
    const SCHEMA_HASH: u64;
}

/// 为类型手写 schema 哈希。
///
/// ```ignore
/// unbound_abi::impl_schema_hash!(MyState, 0x1234, "field_a", "u32", 0, "field_b", "f32", 4);
/// ```
///
/// `nested` 分段会混入状态所指向类型的 schema 哈希。
/// 无论 `Row` 长什么样，`Vec<Row>` 始终占 24 字节；若无此分段，
/// `Row` 内部的变更不会改变父级指纹，host 便会误将旧内存原样交给新 guest。
///
///
#[macro_export]
macro_rules! impl_schema_hash {
    ($ty:ty, $seed:expr, nested: [$($nested:ty),* $(,)?]
        $(, $name:literal, $tname:literal, $off:expr)* $(,)?) => {
        impl $crate::schema::StateSchema for $ty {
            const SCHEMA_HASH: u64 = {
                let h = $crate::schema::hash_u64($crate::schema::FNV_OFFSET_BASIS, $seed);
                $(
                    let h = $crate::schema::hash_nested(
                        h,
                        <$nested as $crate::schema::StateSchema>::SCHEMA_HASH,
                    );
                )*
                $(
                    let h = $crate::schema::hash_field(h, $name, $tname, $off);
                )*
                $crate::schema::finish_hash(h)
            };
        }
    };
    ($ty:ty, $seed:expr $(, $name:literal, $tname:literal, $off:expr)* $(,)?) => {
        impl $crate::schema::StateSchema for $ty {
            const SCHEMA_HASH: u64 = {
                let h = $crate::schema::hash_u64($crate::schema::FNV_OFFSET_BASIS, $seed);
                $(
                    let h = $crate::schema::hash_field(h, $name, $tname, $off);
                )*
                $crate::schema::finish_hash(h)
            };
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    struct A;
    struct B;
    impl_schema_hash!(A, 1, "x", "u32", 0, "y", "f32", 4);
    impl_schema_hash!(B, 1, "x", "u32", 0, "z", "f32", 4);

    #[test]
    fn known_fnv_value() {
        // FNV-1a("a") 是已知的标准测试向量。
        assert_eq!(hash_str(FNV_OFFSET_BASIS, "a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(hash_str(FNV_OFFSET_BASIS, "foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn field_rename_changes_hash() {
        assert_ne!(A::SCHEMA_HASH, B::SCHEMA_HASH);
    }

    #[test]
    fn hash_is_deterministic_across_calls() {
        assert_eq!(A::SCHEMA_HASH, A::SCHEMA_HASH);
        assert_ne!(A::SCHEMA_HASH, 0);
    }

    #[test]
    fn offset_change_changes_hash() {
        struct C;
        struct D;
        impl_schema_hash!(C, 7, "v", "u64", 0);
        impl_schema_hash!(D, 7, "v", "u64", 8);
        assert_ne!(C::SCHEMA_HASH, D::SCHEMA_HASH);
    }

    /// 无论 `T` 长什么样，`Vec<T>` 始终占 24 字节，因此 `T` 内部的变更
    /// 必须混入父级指纹——否则 host 会误将旧内存原样交给新 guest。
    ///
    #[test]
    fn a_change_in_a_nested_type_changes_the_parent_hash() {
        struct Inner;
        struct Outer;
        impl_schema_hash!(Inner, 1, "x", "u32", 0);
        impl_schema_hash!(Outer, 2, nested: [Inner], "rows", "Vec<Inner>", 0);

        struct InnerMoved;
        struct OuterMoved;
        impl_schema_hash!(InnerMoved, 1, "x", "u32", 8);
        impl_schema_hash!(OuterMoved, 2, nested: [InnerMoved], "rows", "Vec<Inner>", 0);

        assert_ne!(
            Outer::SCHEMA_HASH,
            OuterMoved::SCHEMA_HASH,
            "the parent's fingerprint must follow the nested type's layout"
        );
    }

    #[test]
    fn the_nested_hash_is_order_sensitive() {
        // `A` 与 `B` 仅在一个字段名上不同，因此哈希结果不同，
        // 且混入顺序的差异也是可观测的。
        struct Ab;
        struct Ba;
        impl_schema_hash!(Ab, 2, nested: [A, B], "v", "u32", 0);
        impl_schema_hash!(Ba, 2, nested: [B, A], "v", "u32", 0);
        assert_ne!(Ab::SCHEMA_HASH, Ba::SCHEMA_HASH);
    }
}
