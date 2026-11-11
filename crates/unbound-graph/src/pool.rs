//! Texture descriptions and the offscreen pool (`docs` §10.1, M3).
//!
//! The frosted-glass effect needs a scratch texture: the backdrop is copied
//! out, blurred and then composited. Doing that with a fresh allocation every
//! frame would thrash the driver, so the pool hands out a *recycled* target
//! that matches the requested description.
//!
//! The pool is a pure CPU structure: it decides *which* physical slot a
//! logical texture lives in. The driver-side allocation stays in the GPU
//! crate, which owns the images.

use std::collections::VecDeque;

use rustc_hash::FxHashMap;

/// The pixel format of an offscreen target.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    /// 8 bits per channel, unsigned, normalised.
    Rgba8,
    /// 16-bit float per channel — the format a blur wants, because it keeps
    /// the intermediate values above 1.0 from clipping.
    Rgba16f,
    /// A single 8-bit channel.
    R8,
}

impl TextureFormat {
    /// Bytes per pixel.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgba8 | Self::Rgba16f => 4,
            Self::R8 => 1,
        }
    }
}

/// The description of an offscreen target.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The pixel format.
    pub format: TextureFormat,
    /// The number of samples; 1 means no multisampling.
    pub samples: u32,
}

impl TextureDesc {
    /// A single-sampled description.
    #[must_use]
    pub const fn new(width: u32, height: u32, format: TextureFormat) -> Self {
        Self {
            width,
            height,
            format,
            samples: 1,
        }
    }

    /// The size of the texture in bytes.
    #[must_use]
    pub const fn byte_size(self) -> u64 {
        self.width as u64 * self.height as u64 * self.format.bytes_per_pixel() as u64
    }

    /// True when the description describes a usable texture.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.width > 0 && self.height > 0 && self.samples > 0
    }
}

/// A pooled target: a slot index plus the description it was created with.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PooledTexture {
    /// The physical slot in the pool.
    pub slot: u32,
    /// The description the slot holds.
    pub desc: TextureDesc,
}

/// A recycling pool of offscreen targets.
///
/// Targets are matched by description: asking for the same size and format
/// again returns the same slot, so a steady-state frame allocates nothing.
#[derive(Debug, Default)]
pub struct OffscreenPool {
    /// The description of every slot.
    slots: Vec<Option<TextureDesc>>,
    /// Free slot indices.
    free: VecDeque<u32>,
    /// Slots that are in use, with their description.
    live: FxHashMap<u32, TextureDesc>,
    /// How many times a slot was reused instead of created.
    reuses: u64,
    /// How many times a free slot was taken with a *different* description.
    rebinds: u64,
    /// How many times a slot had to be created.
    creations: u64,
    /// The total number of bytes the pool holds.
    bytes: u64,
}

impl OffscreenPool {
    /// Create an empty pool.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire a target matching `desc`.
    ///
    /// An existing live slot with the same description is reused; otherwise a
    /// free slot is claimed, or a new one is created.
    pub fn acquire(&mut self, desc: TextureDesc) -> PooledTexture {
        // A live slot with the same description can be shared outright.
        if let Some((slot, existing)) = self
            .live
            .iter()
            .find(|(_, existing)| **existing == desc)
            .map(|(slot, existing)| (*slot, *existing))
        {
            self.reuses += 1;
            return PooledTexture { slot, desc: existing };
        }
        let slot = match self.free.pop_front() {
            Some(slot) => {
                let previous = self.slots[slot as usize];
                if previous == Some(desc) {
                    // The image is still there: nothing to recreate.
                    self.reuses += 1;
                } else {
                    // The slot changes shape: the old image is gone.
                    self.rebinds += 1;
                }
                self.bytes -= previous.map_or(0, |old| old.byte_size());
                self.slots[slot as usize] = Some(desc);
                self.bytes += desc.byte_size();
                self.live.insert(slot, desc);
                slot
            }
            None => {
                let slot = self.slots.len() as u32;
                self.slots.push(Some(desc));
                self.live.insert(slot, desc);
                self.creations += 1;
                self.bytes += desc.byte_size();
                slot
            }
        };
        PooledTexture { slot, desc }
    }

    /// Release a target.
    pub fn release(&mut self, texture: PooledTexture) {
        if self.live.remove(&texture.slot).is_some() {
            self.free.push_back(texture.slot);
        }
    }

    /// Number of slots the pool holds.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Number of slots currently handed out.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// Total number of bytes the pool holds.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// How many acquisitions were served from the pool.
    #[must_use]
    pub const fn reuse_count(&self) -> u64 {
        self.reuses
    }

    /// How many slots had to be created.
    #[must_use]
    pub const fn creation_count(&self) -> u64 {
        self.creations
    }

    /// How many times a free slot was taken with a different description.
    #[must_use]
    pub const fn rebind_count(&self) -> u64 {
        self.rebinds
    }

    /// The description of a slot.
    #[must_use]
    pub fn slot_desc(&self, slot: u32) -> Option<TextureDesc> {
        self.slots.get(slot as usize).copied().flatten()
    }
}

/// A separable Gaussian kernel for the frosted-glass blur.
///
/// The frosted-glass effect is a wide, cheap blur. A true Gaussian needs a
/// large radius to look smooth, so the implementation uses a small Gaussian
/// followed by a box blur — the classic "stack blur" trick — which is
/// O(1) per pixel regardless of the radius.
///
/// This function returns the *Gaussian* half of that stack: the weights of a
/// small kernel that a shader can multiply by.
#[must_use]
pub fn gaussian_kernel(radius: u32) -> Vec<f32> {
    if radius == 0 {
        return vec![1.0];
    }
    // The classic 3-box approximation of a Gaussian uses a sigma of
    // `radius / 3`; the small kernel below is sampled from that Gaussian.
    let sigma = f64::from(radius) / 3.0;
    let size = (radius as usize) * 2 + 1;
    let mut weights = Vec::with_capacity(size);
    let mut sum = 0.0f64;
    for offset in -(radius as i32)..=(radius as i32) {
        let x = f64::from(offset);
        let value = (-(x * x) / (2.0 * sigma * sigma)).exp();
        weights.push(value);
        sum += value;
    }
    weights
        .into_iter()
        .map(|weight| (weight / sum) as f32)
        .collect()
}

/// The box-blur passes that turn the small Gaussian into a wide blur.
///
/// Each pass halves the effective radius, so the number of passes is
/// `log2(radius)`.
#[must_use]
pub fn box_blur_passes(radius: u32) -> u32 {
    if radius == 0 {
        return 0;
    }
    let mut passes = 0;
    let mut r = radius;
    while r > 1 {
        r /= 2;
        passes += 1;
    }
    passes.max(1)
}

/// The total weight of a kernel; it must be one.
#[must_use]
pub fn kernel_weight(kernel: &[f32]) -> f32 {
    kernel.iter().sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pool_reuses_a_target_with_the_same_description() {
        let mut pool = OffscreenPool::new();
        let desc = TextureDesc::new(512, 512, TextureFormat::Rgba16f);
        let first = pool.acquire(desc);
        let second = pool.acquire(desc);
        assert_eq!(first.slot, second.slot);
        assert_eq!(pool.creation_count(), 1);
        assert_eq!(pool.reuse_count(), 1);
    }

    #[test]
    fn a_different_description_gets_its_own_slot() {
        let mut pool = OffscreenPool::new();
        let a = pool.acquire(TextureDesc::new(512, 512, TextureFormat::Rgba16f));
        let b = pool.acquire(TextureDesc::new(512, 512, TextureFormat::Rgba8));
        assert_ne!(a.slot, b.slot);
        assert_eq!(pool.creation_count(), 2);
    }

    #[test]
    fn a_released_slot_is_reused_for_the_next_request() {
        let mut pool = OffscreenPool::new();
        let desc = TextureDesc::new(64, 64, TextureFormat::Rgba8);
        let first = pool.acquire(desc);
        pool.release(first);
        let second = pool.acquire(TextureDesc::new(128, 128, TextureFormat::Rgba8));
        assert_eq!(second.slot, first.slot, "the freed slot must be reused");
        assert_eq!(pool.creation_count(), 1, "no new allocation is needed");
    }

    #[test]
    fn the_pool_accounts_for_its_memory() {
        let mut pool = OffscreenPool::new();
        pool.acquire(TextureDesc::new(64, 64, TextureFormat::Rgba16f));
        assert_eq!(pool.bytes(), 64 * 64 * 4);
        pool.acquire(TextureDesc::new(64, 64, TextureFormat::R8));
        assert_eq!(pool.bytes(), 64 * 64 * 4 + 64 * 64);
    }

    #[test]
    fn a_steady_state_frame_allocates_nothing() {
        let mut pool = OffscreenPool::new();
        let desc = TextureDesc::new(256, 256, TextureFormat::Rgba16f);
        for _ in 0..10 {
            let texture = pool.acquire(desc);
            pool.release(texture);
        }
        assert_eq!(pool.creation_count(), 1);
        assert_eq!(pool.reuse_count(), 9);
    }

    #[test]
    fn the_gaussian_kernel_is_normalised() {
        for radius in [1u32, 2, 4, 8, 16] {
            let kernel = gaussian_kernel(radius);
            assert_eq!(kernel.len(), radius as usize * 2 + 1);
            assert!(
                (kernel_weight(&kernel) - 1.0).abs() < 1e-5,
                "radius {radius} sums to {}",
                kernel_weight(&kernel)
            );
        }
    }

    #[test]
    fn the_gaussian_kernel_is_symmetric() {
        let kernel = gaussian_kernel(8);
        let half = kernel.len() / 2;
        for i in 0..half {
            assert!((kernel[i] - kernel[kernel.len() - 1 - i]).abs() < 1e-7);
        }
    }

    #[test]
    fn a_zero_radius_is_the_identity_kernel() {
        assert_eq!(gaussian_kernel(0), vec![1.0]);
        assert_eq!(box_blur_passes(0), 0);
    }

    #[test]
    fn the_box_blur_pass_count_grows_with_the_radius() {
        assert!(box_blur_passes(64) >= 6);
        assert!(box_blur_passes(1) >= 1);
    }
}
