//! 零依赖的 `bitflags` 替代方案。
//!
//! 从 crate 根部导出，供下游 crate 复用，避免引入过程宏依赖
//! （保持 `app_guest` 快速编译，见 `docs` §7.1）。
//!

/// 基于整数声明 `#[repr(transparent)]` 位标志新类型（newtype）。
#[macro_export]
macro_rules! bitflags_like {
    (
        $(#[$meta:meta])*
        pub struct $name:ident : $ty:ty {
            $(
                $(#[$fmeta:meta])*
                const $flag:ident = $value:expr;
            )*
        }
    ) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord,
                 $crate::__private::Pod, $crate::__private::Zeroable)]
        pub struct $name(pub $ty);

        impl $name {
            pub const EMPTY: Self = Self(0);

            $(
                $(#[$fmeta])*
                pub const $flag: Self = Self($value);
            )*

            #[inline]
            #[must_use]
            pub const fn bits(self) -> $ty {
                self.0
            }

            #[inline]
            #[must_use]
            pub const fn from_bits_truncate(bits: $ty) -> Self {
                Self(bits)
            }

            #[inline]
            #[must_use]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            #[inline]
            #[must_use]
            pub const fn intersects(self, other: Self) -> bool {
                self.0 & other.0 != 0
            }

            #[inline]
            pub fn insert(&mut self, other: Self) {
                self.0 |= other.0;
            }

            #[inline]
            pub fn remove(&mut self, other: Self) {
                self.0 &= !other.0;
            }

            #[inline]
            #[must_use]
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }
        }

        impl core::ops::BitOr for $name {
            type Output = Self;
            #[inline]
            fn bitor(self, rhs: Self) -> Self {
                Self(self.0 | rhs.0)
            }
        }

        impl core::ops::BitOrAssign for $name {
            #[inline]
            fn bitor_assign(&mut self, rhs: Self) {
                self.0 |= rhs.0;
            }
        }

        impl core::ops::BitAndAssign for $name {
            #[inline]
            fn bitand_assign(&mut self, rhs: Self) {
                self.0 &= rhs.0;
            }
        }

        impl core::ops::BitAnd for $name {
            type Output = Self;
            #[inline]
            fn bitand(self, rhs: Self) -> Self {
                Self(self.0 & rhs.0)
            }
        }

        impl core::ops::Not for $name {
            type Output = Self;
            #[inline]
            fn not(self) -> Self {
                Self(!self.0)
            }
        }
    };
}
