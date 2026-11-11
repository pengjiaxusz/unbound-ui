//! Two-Level Segregated Fit (TLSF) allocator over a single contiguous arena.
//!
//! `docs` §2.2 requires the persistent pool to be a self-managed large
//! `VkBuffer` carved with TLSF so that:
//!
//! * allocation and free are **O(1)** (two bitmap probes + one list pop),
//! * there is **zero per-frame allocation** — the free lists live in fixed
//!   arrays, not in a heap structure,
//! * freed blocks coalesce immediately, keeping fragmentation bounded for the
//!   insert/remove churn of an editor session.
//!
//! The implementation is 100% safe Rust: the arena is a `Vec<u128>` (guaranteed
//! 16-byte alignment) reinterpreted as bytes with `bytemuck`, and blocks are
//! addressed by `u32` byte offsets. Nothing here aliases.

use bytemuck::{Pod, Zeroable};

/// Minimum block size: 64B header + 64B payload.
///
/// A 64-byte header (instead of the usual 16/32) buys a property the renderer
/// relies on: **every payload offset is a multiple of 64**, i.e. every
/// allocation is at least `GpuPrimitiveInstance`-aligned without any per-alloc
/// fix-up. The 32 bytes of padding are pure waste on a 4 GiB pool (~0.5% for
/// 64B allocations) and are more than paid back by removing an alignment
/// branch from the hot path.
pub const MIN_BLOCK: u32 = 128;

/// Second-level subdivision bits: 16 buckets per first level.
const SL_SHIFT: u32 = 4;
const SL_COUNT: usize = 1 << SL_SHIFT;
/// First level covers `size < 2^32`, so 32 buckets are enough.
const FL_COUNT: usize = 32;

const NONE: u32 = u32::MAX;

/// 64-byte block header, stored inline at the start of every block.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct BlockHeader {
    /// Total block size **including** this header.
    size: u32,
    /// Size of the physically previous block (0 for the first block).
    prev_size: u32,
    /// Next block in the same free list.
    next_free: u32,
    /// Previous block in the same free list.
    prev_free: u32,
    /// 1 when free.
    flags: u32,
    _pad: [u32; 11],
}

const HEADER_SIZE: u32 = 64;
const FLAG_FREE: u32 = 1;

/// Why an allocation failed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AllocError {
    /// No free block large enough after searching every bucket.
    OutOfMemory {
        request: u32,
        largest_free: u32,
    },
    /// Request exceeds the arena capacity.
    TooLarge {
        request: u32,
        capacity: u32,
    },
    /// Alignment is not a power of two.
    InvalidAlignment(usize),
}

impl core::fmt::Display for AllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfMemory {
                request,
                largest_free,
            } => write!(
                f,
                "TLSF out of memory: requested {request}B, largest free block {largest_free}B"
            ),
            Self::TooLarge { request, capacity } => {
                write!(f, "TLSF request {request}B exceeds arena capacity {capacity}B")
            }
            Self::InvalidAlignment(a) => write!(f, "invalid alignment {a}"),
        }
    }
}

impl std::error::Error for AllocError {}

/// A snapshot of allocator health, surfaced in the debug overlay and asserted
/// by CI (fragmentation must not creep during steady-state editing).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TlsfStats {
    pub capacity: u32,
    pub allocated: u32,
    pub free: u32,
    pub largest_free: u32,
    pub block_count: u32,
    pub free_block_count: u32,
    pub successful_allocations: u64,
    pub failed_allocations: u64,
}

impl TlsfStats {
    /// `1 - largest_free / total_free`. 0 means perfectly coalesced.
    #[must_use]
    pub fn fragmentation(&self) -> f32 {
        if self.free == 0 {
            0.0
        } else {
            1.0 - (self.largest_free as f32 / self.free as f32)
        }
    }

    #[must_use]
    pub fn utilisation(&self) -> f32 {
        if self.capacity == 0 {
            0.0
        } else {
            self.allocated as f32 / self.capacity as f32
        }
    }
}

/// Free-list index tables. 32 × 16 buckets ≡ 512 `u32` heads, 2 KiB total.
struct FreeLists {
    fl_bitmap: u32,
    sl_bitmap: [u32; FL_COUNT],
    heads: [[u32; SL_COUNT]; FL_COUNT],
}

impl FreeLists {
    fn new() -> Self {
        Self {
            fl_bitmap: 0,
            sl_bitmap: [0; FL_COUNT],
            heads: [[NONE; SL_COUNT]; FL_COUNT],
        }
    }

    fn clear(&mut self) {
        self.fl_bitmap = 0;
        self.sl_bitmap = [0; FL_COUNT];
        self.heads = [[NONE; SL_COUNT]; FL_COUNT];
    }
}

/// Map a block size to `(first_level, second_level)`.
#[inline]
fn mapping(size: u32) -> (usize, usize) {
    let size = size.max(MIN_BLOCK);
    let fl = 31 - size.leading_zeros(); // floor(log2(size))
    let fl_usize = (fl as usize).min(FL_COUNT - 1);
    if fl < SL_SHIFT {
        return (0, (size as usize) & (SL_COUNT - 1));
    }
    let sl = ((size >> (fl - SL_SHIFT)) & (SL_COUNT as u32 - 1)) as usize;
    (fl_usize, sl)
}

#[inline]
const fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

#[inline]
fn read_header(arena: &[u8], off: u32) -> BlockHeader {
    let start = off as usize;
    *bytemuck::from_bytes::<BlockHeader>(&arena[start..start + HEADER_SIZE as usize])
}

#[inline]
fn write_header(arena: &mut [u8], off: u32, h: BlockHeader) {
    let start = off as usize;
    arena[start..start + HEADER_SIZE as usize].copy_from_slice(bytemuck::bytes_of(&h));
}

#[inline]
fn patch_header(arena: &mut [u8], off: u32, f: impl FnOnce(&mut BlockHeader)) {
    let mut h = read_header(arena, off);
    f(&mut h);
    write_header(arena, off, h);
}

// ---------------------------------------------------------------------------
// Free-list primitives (free functions so the borrow checker sees disjoint
// field borrows of `Tlsf`).
// ---------------------------------------------------------------------------

fn list_insert(arena: &mut [u8], lists: &mut FreeLists, off: u32) {
    let h = read_header(arena, off);
    let (fl, sl) = mapping(h.size);
    let head = lists.heads[fl][sl];
    patch_header(arena, off, |b| {
        b.next_free = head;
        b.prev_free = NONE;
        b.flags |= FLAG_FREE;
    });
    if head != NONE {
        patch_header(arena, head, |b| b.prev_free = off);
    }
    lists.heads[fl][sl] = off;
    lists.sl_bitmap[fl] |= 1 << sl;
    lists.fl_bitmap |= 1 << fl;
}

fn list_remove(arena: &mut [u8], lists: &mut FreeLists, off: u32) {
    let h = read_header(arena, off);
    let (fl, sl) = mapping(h.size);
    let (next, prev) = (h.next_free, h.prev_free);

    if prev != NONE {
        patch_header(arena, prev, |b| b.next_free = next);
    } else {
        debug_assert_eq!(lists.heads[fl][sl], off);
        lists.heads[fl][sl] = next;
    }
    if next != NONE {
        patch_header(arena, next, |b| b.prev_free = prev);
    }

    if lists.heads[fl][sl] == NONE {
        lists.sl_bitmap[fl] &= !(1 << sl);
        if lists.sl_bitmap[fl] == 0 {
            lists.fl_bitmap &= !(1 << fl);
        }
    }
}

fn list_find(lists: &FreeLists, size: u32) -> Option<u32> {
    let (fl, sl) = mapping(size);
    // Same first level: look for a bucket at or above `sl`.
    let sl_map = lists.sl_bitmap[fl] & (!0u32 << sl);
    if sl_map != 0 {
        let bucket = sl_map.trailing_zeros() as usize;
        let off = lists.heads[fl][bucket];
        return (off != NONE).then_some(off);
    }
    // Otherwise round up to the next non-empty first level.
    if fl + 1 >= FL_COUNT {
        return None;
    }
    let fl_map = lists.fl_bitmap & (!0u32 << (fl + 1));
    if fl_map == 0 {
        return None;
    }
    let next_fl = fl_map.trailing_zeros() as usize;
    let sl_map = lists.sl_bitmap[next_fl];
    if sl_map == 0 {
        return None;
    }
    let bucket = sl_map.trailing_zeros() as usize;
    let off = lists.heads[next_fl][bucket];
    (off != NONE).then_some(off)
}

fn largest_free(arena: &[u8], lists: &FreeLists) -> u32 {
    if lists.fl_bitmap == 0 {
        return 0;
    }
    let fl = 31 - lists.fl_bitmap.leading_zeros();
    let sl_map = lists.sl_bitmap[fl as usize];
    if sl_map == 0 {
        return 0;
    }
    let sl = 31 - sl_map.leading_zeros();
    let off = lists.heads[fl as usize][sl as usize];
    if off == NONE {
        0
    } else {
        read_header(arena, off).size
    }
}

/// Split `off` so that it holds a block of exactly `want` bytes, returning the
/// remainder as a free block.
fn split_block(arena: &mut [u8], lists: &mut FreeLists, off: u32, want: u32) {
    let h = read_header(arena, off);
    debug_assert!(h.size >= want);
    let rest = h.size - want;
    if rest < MIN_BLOCK {
        // Not worth splitting: hand the whole block to the caller. Internal
        // fragmentation is bounded by MIN_BLOCK.
        return;
    }
    let rest_off = off + want;
    patch_header(arena, off, |b| b.size = want);
    write_header(
        arena,
        rest_off,
        BlockHeader {
            size: rest,
            prev_size: want,
            next_free: NONE,
            prev_free: NONE,
            flags: FLAG_FREE,
            _pad: [0; 11],
        },
    );
    // The physically next block after the split must learn the new prev_size.
    let after = rest_off + rest;
    if (after as usize) < arena.len() {
        patch_header(arena, after, |b| b.prev_size = rest);
    }
    list_insert(arena, lists, rest_off);
}

// ---------------------------------------------------------------------------
// The allocator
// ---------------------------------------------------------------------------

/// TLSF allocator over a single contiguous arena.
pub struct Tlsf {
    /// Backing storage. `u128` so the base pointer is 16-byte aligned.
    storage: Vec<u128>,
    len: u32,
    lists: FreeLists,
    stats: TlsfStats,
}

impl core::fmt::Debug for Tlsf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tlsf")
            .field("capacity", &self.len)
            .field("stats", &self.stats)
            .finish()
    }
}

impl Tlsf {
    /// Create an allocator over `capacity` bytes (rounded up to 16).
    ///
    /// # Panics
    /// Panics if `capacity` is smaller than [`MIN_BLOCK`] or does not fit `u32`.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity >= MIN_BLOCK as usize, "arena too small");
        let len = align_up_checked(capacity);
        let mut storage = vec![0u128; (len as usize).div_ceil(16)];
        {
            let bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut storage);
            let bytes = &mut bytes[..len as usize];
            write_header(
                bytes,
                0,
                BlockHeader {
                    size: len,
                    prev_size: 0,
                    next_free: NONE,
                    prev_free: NONE,
                    flags: FLAG_FREE,
                    _pad: [0; 11],
                },
            );
            let mut lists = FreeLists::new();
            list_insert(bytes, &mut lists, 0);
            Self {
                storage,
                len,
                lists,
                stats: TlsfStats {
                    capacity: len,
                    free: len,
                    largest_free: len,
                    block_count: 1,
                    free_block_count: 1,
                    ..Default::default()
                },
            }
        }
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        let all: &[u8] = bytemuck::cast_slice(&self.storage);
        &all[..self.len as usize]
    }

    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.len
    }

    /// Recompute and return allocator statistics (walks the block chain).
    #[must_use]
    pub fn stats(&mut self) -> TlsfStats {
        self.refresh_stats();
        self.stats
    }

    fn refresh_stats(&mut self) {
        let len = self.len;
        let (block_count, free_block_count, free_bytes, largest) = {
            let arena = self.bytes();
            let mut off = 0u32;
            let mut block_count = 0u32;
            let mut free_block_count = 0u32;
            let mut free_bytes = 0u32;
            while (off as usize) + HEADER_SIZE as usize <= arena.len() {
                let h = read_header(arena, off);
                if h.size == 0 {
                    break;
                }
                block_count += 1;
                if h.flags & FLAG_FREE != 0 {
                    free_block_count += 1;
                    free_bytes += h.size;
                }
                off += h.size;
            }
            (block_count, free_block_count, free_bytes, largest_free(arena, &self.lists))
        };
        self.stats.block_count = block_count;
        self.stats.free_block_count = free_block_count;
        self.stats.free = free_bytes;
        self.stats.allocated = len - free_bytes;
        self.stats.largest_free = largest;
    }

    /// Allocate `size` bytes with the arena's natural 16-byte alignment.
    ///
    /// # Errors
    /// Returns [`AllocError`] when the request cannot be satisfied.
    pub fn allocate(&mut self, size: u32) -> Result<u32, AllocError> {
        self.allocate_aligned(size, 16)
    }

    /// Allocate `size` bytes aligned to `align` (power of two).
    ///
    /// Returns the offset of the payload, guaranteed to satisfy
    /// `offset % align == 0` with at least `size` usable bytes.
    ///
    /// # Errors
    /// Returns [`AllocError`] when the request cannot be satisfied.
    pub fn allocate_aligned(&mut self, size: u32, align: usize) -> Result<u32, AllocError> {
        if !align.is_power_of_two() {
            self.stats.failed_allocations += 1;
            return Err(AllocError::InvalidAlignment(align));
        }
        if size > self.len {
            self.stats.failed_allocations += 1;
            return Err(AllocError::TooLarge {
                request: size,
                capacity: self.len,
            });
        }
        let len = self.len;
        // Header + payload rounded to the 16B granule, plus slack so an
        // over-aligned payload always has room to be nudged forward.
        let align32 = align as u32;
        let slack = if align32 > HEADER_SIZE { align32 } else { 0 };
        let total = align_up(size.saturating_add(HEADER_SIZE).saturating_add(slack), 64)
            .max(MIN_BLOCK);

        let result = {
            let Self { storage, lists, .. } = &mut *self;
            let all: &mut [u8] = bytemuck::cast_slice_mut(storage);
            let arena = &mut all[..len as usize];

            match list_find(lists, total) {
                None => Err(AllocError::OutOfMemory {
                    request: size,
                    largest_free: largest_free(arena, lists),
                }),
                Some(block) => {
                    list_remove(arena, lists, block);
                    // The block is live now: clearing FLAG_FREE is essential,
                    // otherwise the next neighbour free() would try to unlink a
                    // block that is not in any free list.
                    patch_header(arena, block, |b| b.flags &= !FLAG_FREE);
                    split_block(arena, lists, block, total);

                    let cur_size = read_header(arena, block).size;
                    let mut payload = block + HEADER_SIZE;

                    if align32 > HEADER_SIZE {
                        // We must align the *payload*, not the block: the
                        // payload sits HEADER_SIZE bytes into the block, so the
                        // leading block has to absorb that bias.
                        let aligned = align_up(payload, align32);
                        let lead = aligned - HEADER_SIZE - block;
                        // Only carve a leading free block when both sides stay
                        // at least MIN_BLOCK wide.
                        if lead >= MIN_BLOCK && cur_size > lead + MIN_BLOCK {
                            patch_header(arena, block, |b| b.size = lead);
                            let new_block = block + lead;
                            let new_size = cur_size - lead;
                            write_header(
                                arena,
                                new_block,
                                BlockHeader {
                                    size: new_size,
                                    prev_size: lead,
                                    next_free: NONE,
                                    prev_free: NONE,
                                    flags: 0,
                                    _pad: [0; 11],
                                },
                            );
                            let after = new_block + new_size;
                            if (after as usize) < arena.len() {
                                patch_header(arena, after, |b| b.prev_size = new_size);
                            }
                            list_insert(arena, lists, block);
                            payload = new_block + HEADER_SIZE;
                        }
                    }

                    // Zero the handed-out region: a GPU buffer holding a
                    // previous node's bytes is the worst class of heisenbug.
                    let start = payload as usize;
                    let end = (start + size as usize).min(arena.len());
                    arena[start..end].fill(0);
                    Ok(payload)
                }
            }
        };

        match result {
            Ok(p) => {
                self.stats.successful_allocations += 1;
                Ok(p)
            }
            Err(e) => {
                self.stats.failed_allocations += 1;
                Err(e)
            }
        }
    }

    /// Free a previously allocated payload offset.
    ///
    /// Double frees are detected in debug builds and rejected (`false`) in
    /// release builds, so a corrupt guest cannot abort the host.
    pub fn deallocate(&mut self, payload: u32) -> bool {
        if payload < HEADER_SIZE || payload as usize >= self.len as usize {
            debug_assert!(false, "deallocate with out-of-range offset {payload}");
            return false;
        }
        let len = self.len;
        let Self { storage, lists, .. } = &mut *self;
        let all: &mut [u8] = bytemuck::cast_slice_mut(storage);
        let arena = &mut all[..len as usize];

        let block = payload - HEADER_SIZE;
        let h = read_header(arena, block);
        if h.flags & FLAG_FREE != 0 {
            debug_assert!(false, "double free of block at {block}");
            return false;
        }

        // Mark free before merging so `list_insert` sees a consistent header.
        patch_header(arena, block, |b| b.flags |= FLAG_FREE);
        let mut cur = block;
        let mut cur_size = h.size;

        // Merge with the physically previous block.
        if h.prev_size != 0 {
            let prev = block - h.prev_size;
            let ph = read_header(arena, prev);
            if ph.flags & FLAG_FREE != 0 {
                list_remove(arena, lists, prev);
                cur = prev;
                cur_size += ph.size;
                patch_header(arena, cur, |b| b.size = cur_size);
            }
        }

        // Merge with the physically next block.
        let next = cur + cur_size;
        if (next as usize) + HEADER_SIZE as usize <= arena.len() {
            let nh = read_header(arena, next);
            if nh.size != 0 && nh.flags & FLAG_FREE != 0 {
                list_remove(arena, lists, next);
                cur_size += nh.size;
                patch_header(arena, cur, |b| b.size = cur_size);
            }
        }

        // The block after the merged region must learn the new prev_size.
        let after = cur + cur_size;
        if (after as usize) < arena.len() {
            patch_header(arena, after, |b| b.prev_size = cur_size);
        }

        list_insert(arena, lists, cur);
        true
    }

    /// Payload capacity of an allocation (may exceed the requested size).
    #[must_use]
    pub fn usable_size(&self, payload: u32) -> u32 {
        if payload < HEADER_SIZE || payload as usize >= self.len as usize {
            return 0;
        }
        let arena = self.bytes();
        let h = read_header(arena, payload - HEADER_SIZE);
        h.size.saturating_sub(HEADER_SIZE)
    }

    /// Debug helper: every block as `(offset, size, is_free)`.
    #[must_use]
    pub fn blocks(&self) -> Vec<(u32, u32, bool)> {
        let arena = self.bytes();
        let mut out = Vec::new();
        let mut off = 0u32;
        while (off as usize) + HEADER_SIZE as usize <= arena.len() {
            let h = read_header(arena, off);
            if h.size == 0 {
                break;
            }
            out.push((off, h.size, h.flags & FLAG_FREE != 0));
            off += h.size;
        }
        out
    }

    /// Wipe the arena back to a single free block (device-lost re-inflation).
    pub fn reset(&mut self) {
        let len = self.len;
        let successful = self.stats.successful_allocations;
        let failed = self.stats.failed_allocations;
        {
            let Self {
                storage, lists, stats, ..
            } = &mut *self;
            let all: &mut [u8] = bytemuck::cast_slice_mut(storage);
            let arena = &mut all[..len as usize];
            arena.fill(0);
            write_header(
                arena,
                0,
                BlockHeader {
                    size: len,
                    prev_size: 0,
                    next_free: NONE,
                    prev_free: NONE,
                    flags: FLAG_FREE,
                    _pad: [0; 11],
                },
            );
            lists.clear();
            list_insert(arena, lists, 0);
            *stats = TlsfStats {
                capacity: len,
                allocated: 0,
                free: len,
                largest_free: len,
                block_count: 1,
                free_block_count: 1,
                successful_allocations: successful,
                failed_allocations: failed,
            };
        }
    }
}

#[inline]
fn align_up_checked(capacity: usize) -> u32 {
    assert!(
        capacity <= u32::MAX as usize - 16,
        "arena must fit in u32 (4 GiB)"
    );
    align_up(capacity as u32, 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_aligned(v: u32, align: u32) -> bool {
        v % align == 0
    }

    #[test]
    fn fresh_arena_is_one_free_block() {
        let mut t = Tlsf::new(4096);
        assert_eq!(t.capacity(), 4096);
        let s = t.stats();
        assert_eq!(s.free, 4096);
        assert_eq!(s.block_count, 1);
        assert_eq!(s.fragmentation(), 0.0);
        assert_eq!(t.blocks(), vec![(0, 4096, true)]);
    }

    #[test]
    fn allocate_then_free_returns_to_single_block() {
        let mut t = Tlsf::new(8192);
        let a = t.allocate(1000).unwrap();
        let b = t.allocate(2000).unwrap();
        let c = t.allocate(3000).unwrap();
        assert!(is_aligned(a, 16) && is_aligned(b, 16) && is_aligned(c, 16));
        assert!(t.deallocate(b));
        assert!(t.deallocate(a));
        assert!(t.deallocate(c));
        let s = t.stats();
        assert_eq!(
            s.block_count,
            1,
            "blocks must fully coalesce: {:?}",
            t.blocks()
        );
        assert_eq!(s.free, 8192);
        assert_eq!(s.fragmentation(), 0.0);
    }

    #[test]
    fn allocations_never_overlap() {
        let mut t = Tlsf::new(1 << 20);
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        let mut order: Vec<u32> = Vec::new();
        let mut seed = 0x1234_5678u32;
        for i in 0..500 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let size = 16 + (seed % 4000);
            match t.allocate(size) {
                Ok(p) => {
                    let end = p + size;
                    for (s, e) in &ranges {
                        assert!(
                            end <= *s || p >= *e,
                            "allocation {i} [{p},{end}) overlaps [{s},{e})"
                        );
                    }
                    ranges.push((p, end));
                    order.push(p);
                }
                Err(_) => break,
            }
        }
        assert!(order.len() > 100, "only {} allocations fit", order.len());
        for (i, p) in order.iter().enumerate() {
            if i % 3 == 0 {
                assert!(t.deallocate(*p));
            }
        }
        // Integrity: the block chain must still cover the whole arena exactly.
        let total: u32 = t.blocks().iter().map(|b| b.1).sum();
        assert_eq!(total, 1 << 20, "block chain must tile the arena");
        // The freed holes must be reusable.
        let mut reused = 0;
        for _ in 0..10 {
            if t.allocate(64).is_ok() {
                reused += 1;
            }
        }
        assert!(reused > 0, "freed holes should accept new allocations");
        assert!(t.stats().largest_free > 0);
    }

    #[test]
    fn alignment_is_respected_for_large_alignments() {
        let mut t = Tlsf::new(64 * 1024);
        // Force a misaligned payload first so the aligned allocator has work.
        let _pad = t.allocate(40).unwrap();
        for align in [32usize, 64, 256, 4096] {
            let p = t.allocate_aligned(256, align).unwrap();
            assert_eq!(p % align as u32, 0, "align {align} -> {p}");
            assert!(t.usable_size(p) >= 256);
        }
    }

    #[test]
    fn oom_is_reported_with_largest_free() {
        let mut t = Tlsf::new(1024);
        let _a = t.allocate(900).unwrap();
        let err = t.allocate(900).unwrap_err();
        match err {
            AllocError::OutOfMemory {
                request,
                largest_free,
            } => {
                assert_eq!(request, 900);
                assert!(largest_free < 900);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(t.stats().failed_allocations, 1);
    }

    #[test]
    fn too_large_is_rejected_before_searching() {
        let mut t = Tlsf::new(1024);
        assert!(matches!(t.allocate(4096), Err(AllocError::TooLarge { .. })));
        assert!(matches!(
            t.allocate_aligned(16, 3),
            Err(AllocError::InvalidAlignment(3))
        ));
    }

    #[test]
    fn freed_memory_is_zeroed_on_reuse() {
        let mut t = Tlsf::new(4096);
        let p = t.allocate(64).unwrap();
        {
            let Tlsf { storage, .. } = &mut t;
            let bytes: &mut [u8] = bytemuck::cast_slice_mut(storage);
            bytes[p as usize..p as usize + 64].fill(0xAB);
        }
        assert!(t.deallocate(p));
        let q = t.allocate(64).unwrap();
        let arena = t.bytes();
        assert!(arena[q as usize..q as usize + 64].iter().all(|b| *b == 0));
    }

    #[test]
    fn alternating_free_keeps_fragmentation_bounded() {
        let mut t = Tlsf::new(256 * 1024);
        let ptrs: Vec<u32> = (0..64).map(|_| t.allocate(1024).unwrap()).collect();
        for (i, p) in ptrs.iter().enumerate() {
            if i % 2 == 0 {
                assert!(t.deallocate(*p));
            }
        }
        let s = t.stats();
        assert!(s.fragmentation() > 0.0, "expected holes");
        assert!(
            s.fragmentation() < 0.5,
            "fragmentation too high: {}",
            s.fragmentation()
        );
        for (i, p) in ptrs.iter().enumerate() {
            if i % 2 == 1 {
                assert!(t.deallocate(*p));
            }
        }
        assert_eq!(t.stats().block_count, 1);
    }

    #[test]
    fn many_small_allocations_are_exact() {
        let mut t = Tlsf::new(8 << 20);
        let mut ps = Vec::new();
        for _ in 0..10_000 {
            ps.push(t.allocate(48).unwrap());
        }
        for p in ps {
            assert!(t.deallocate(p));
        }
        assert_eq!(t.stats().block_count, 1);
        assert_eq!(t.stats().successful_allocations, 10_000);
    }

    #[test]
    fn usable_size_is_at_least_requested() {
        let mut t = Tlsf::new(1 << 16);
        for size in [1u32, 17, 64, 1000, 4096] {
            let p = t.allocate(size).unwrap();
            assert!(t.usable_size(p) >= size, "size {size}");
            t.deallocate(p);
        }
    }

    #[test]
    fn reset_restores_a_pristine_arena() {
        let mut t = Tlsf::new(4096);
        for _ in 0..4 {
            t.allocate(512).unwrap();
        }
        t.reset();
        assert_eq!(t.blocks(), vec![(0, 4096, true)]);
        assert_eq!(t.stats().free, 4096);
        assert!(t.allocate(4000).is_ok());
    }

    #[test]
    fn every_payload_is_64_byte_aligned() {
        let mut t = Tlsf::new(64 * 1024);
        for i in 0..64 {
            let p = t.allocate(1 + (i * 37) % 500).unwrap();
            assert_eq!(p % 64, 0, "payload {p} not 64B aligned");
        }
    }

    #[test]
    fn mapping_is_monotonic() {
        let mut prev = (0usize, 0usize);
        let mut size = MIN_BLOCK;
        while size < (1 << 28) {
            let m = mapping(size);
            assert!(m >= prev, "mapping not monotonic at {size}: {m:?} < {prev:?}");
            prev = m;
            size *= 2;
        }
    }
}
