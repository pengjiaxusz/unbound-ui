//! Compile-time state schema fingerprinting.
//!
//! The hot-reload fast path (`docs` §8.1) compares a 64-bit hash of the guest's
//! `State` memory layout. When it matches, the old `State` is handed to the new
//! `guest_gen_N.dll` verbatim — no serialisation at all. When it differs, the
//! host pays for a `migrate_export`/`migrate_import` round trip instead of
//! crashing.
//!
//! The hash is FNV-1a over `(field_name, type_name, offset)` triples, mixed in
//! declaration order. It is computed entirely in `const` context by the
//! `unbound-derive` macro, so it costs nothing at runtime.

/// FNV-1a 64-bit offset basis.
pub const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Mix a byte slice into an in-progress FNV-1a hash.
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

/// Mix a `&str` into an in-progress hash.
#[inline]
#[must_use]
pub const fn hash_str(hash: u64, s: &str) -> u64 {
    hash_bytes(hash, s.as_bytes())
}

/// Mix a `u64` into an in-progress hash (little-endian, order-sensitive).
#[inline]
#[must_use]
pub const fn hash_u64(hash: u64, v: u64) -> u64 {
    hash_bytes(hash, &v.to_le_bytes())
}

/// Mix a field descriptor — name, type and byte offset — into the hash.
#[inline]
#[must_use]
pub const fn hash_field(hash: u64, name: &str, type_name: &str, offset: usize) -> u64 {
    let h = hash_str(hash, name);
    let h = hash_str(h, "\u{1f}");
    let h = hash_str(h, type_name);
    let h = hash_str(h, "\u{1f}");
    hash_u64(h, offset as u64)
}

/// Finished hash value from an in-progress state.
#[inline]
#[must_use]
pub const fn finish_hash(hash: u64) -> u64 {
    // One last avalanche step so that small field reorderings that only differ
    // in one byte still spread across the whole 64-bit space.
    let mut h = hash;
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h
}

/// Implemented by every guest `State` that participates in hot reload.
pub trait StateSchema {
    /// Fingerprint of the memory layout. Equal values mean "binary compatible".
    const SCHEMA_HASH: u64;
}

/// Hand-write a schema hash for a type without the derive macro.
///
/// ```ignore
/// unbound_abi::impl_schema_hash!(MyState, 0x1234, "field_a", "u32", 0, "field_b", "f32", 4);
/// ```
#[macro_export]
macro_rules! impl_schema_hash {
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
        // FNV-1a("a") is a well-known test vector.
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
}
