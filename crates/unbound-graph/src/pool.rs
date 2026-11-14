//! 纹理描述与离屏纹理池（`docs` §10.1，M3）。
//!
//! 毛玻璃效果需要一张临时纹理：先拷贝背景，再执行模糊与合成。
//! 若每帧都重新分配纹理会导致驱动抖动，因此由纹理池分发与请求描述匹配的*复用*目标。
//!
//!
//!
//! 纹理池是纯 CPU 结构：它只决定逻辑纹理存放在*哪个*物理槽位，驱动侧的图像分配
//! 仍保留在拥有图像资源的 GPU crate 中。
//!

use std::collections::VecDeque;

use rustc_hash::FxHashMap;

/// 离屏渲染目标的像素格式。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    /// 每通道 8 位无符号归一化格式。
    Rgba8,
    /// 每通道 16 位浮点格式——模糊 Pass 的理想格式，可避免超过 1.0 的中间值被截断。
    ///
    Rgba16f,
    /// 单通道 8 位格式。
    R8,
}

impl TextureFormat {
    /// 每像素字节数。
    #[must_use]
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgba8 | Self::Rgba16f => 4,
            Self::R8 => 1,
        }
    }
}

/// 离屏渲染目标的描述信息。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    /// 宽度（像素）。
    pub width: u32,
    /// 高度（像素）。
    pub height: u32,
    /// 像素格式。
    pub format: TextureFormat,
    /// 采样数；1 表示无多重采样。
    pub samples: u32,
}

impl TextureDesc {
    /// 创建单采样纹理描述。
    #[must_use]
    pub const fn new(width: u32, height: u32, format: TextureFormat) -> Self {
        Self {
            width,
            height,
            format,
            samples: 1,
        }
    }

    /// 纹理占用的字节大小。
    #[must_use]
    pub const fn byte_size(self) -> u64 {
        self.width as u64 * self.height as u64 * self.format.bytes_per_pixel() as u64
    }

    /// 当描述表示一个可用纹理时返回 true。
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.width > 0 && self.height > 0 && self.samples > 0
    }
}

/// 池化渲染目标：包含槽位索引及其创建时的纹理描述。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PooledTexture {
    /// 纹理池中的物理槽位索引。
    pub slot: u32,
    /// 该槽位当前持有的纹理描述。
    pub desc: TextureDesc,
}

/// 离屏渲染目标复用池。
///
/// 按纹理描述匹配目标：再次请求相同尺寸与格式将返回同一槽位，实现稳态帧零分配。
///
#[derive(Debug, Default)]
pub struct OffscreenPool {
    /// 每个槽位对应的纹理描述。
    slots: Vec<Option<TextureDesc>>,
    /// 空闲槽位索引列表。
    free: VecDeque<u32>,
    /// 使用中的槽位及其纹理描述。
    live: FxHashMap<u32, TextureDesc>,
    /// 槽位被复用（而非新建）的次数。
    reuses: u64,
    /// 空闲槽位被*不同*描述重新占用的次数。
    rebinds: u64,
    /// 必须新建槽位的次数。
    creations: u64,
    /// 纹理池当前持有的总字节数。
    bytes: u64,
}

impl OffscreenPool {
    /// 创建空纹理池。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 获取匹配 `desc` 的渲染目标。
    ///
    /// 优先复用描述相同的现有活跃槽位；否则认领空闲槽位或新建槽位。
    ///
    pub fn acquire(&mut self, desc: TextureDesc) -> PooledTexture {
        // 描述相同的活跃槽位可直接共享复用。
        if let Some((slot, existing)) = self
            .live
            .iter()
            .find(|(_, existing)| **existing == desc)
            .map(|(slot, existing)| (*slot, *existing))
        {
            self.reuses += 1;
            return PooledTexture {
                slot,
                desc: existing,
            };
        }
        let slot = match self.free.pop_front() {
            Some(slot) => {
                let previous = self.slots[slot as usize];
                if previous == Some(desc) {
                    // 图像规格未变，无需重建。
                    self.reuses += 1;
                } else {
                    // 槽位规格变化，旧图像作废。
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

    /// 释放一个渲染目标。
    pub fn release(&mut self, texture: PooledTexture) {
        if self.live.remove(&texture.slot).is_some() {
            self.free.push_back(texture.slot);
        }
    }

    /// 纹理池持有的总槽位数。
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// 当前已分配出去的槽位数。
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// 纹理池持有的总字节数。
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// 从纹理池中直接复用满足的获取次数。
    #[must_use]
    pub const fn reuse_count(&self) -> u64 {
        self.reuses
    }

    /// 必须新建的槽位次数。
    #[must_use]
    pub const fn creation_count(&self) -> u64 {
        self.creations
    }

    /// 空闲槽位以*不同*描述被复用的次数。
    #[must_use]
    pub const fn rebind_count(&self) -> u64 {
        self.rebinds
    }

    /// 查询指定槽位的纹理描述。
    #[must_use]
    pub fn slot_desc(&self, slot: u32) -> Option<TextureDesc> {
        self.slots.get(slot as usize).copied().flatten()
    }
}

/// 用于毛玻璃模糊的可分离高斯核。
///
/// 毛玻璃效果需要大半径且低开销的模糊。真正的高斯模糊在大半径下开销过高，
/// 因此实现采用小半径高斯后接盒式模糊（经典“stack blur”技巧），无论半径多大
/// 均为每像素 O(1) 复杂度。
///
///
/// 本函数返回该组合中的*高斯*部分：供着色器相乘的小卷积核权重。
///
#[must_use]
pub fn gaussian_kernel(radius: u32) -> Vec<f32> {
    if radius == 0 {
        return vec![1.0];
    }
    // 经典的 3 次盒式近似高斯使用 `radius / 3` 的 sigma；下方小核即采样自该高斯分布。
    //
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

/// 将小高斯模糊扩展为大范围模糊所需的盒式模糊 Pass 数。
///
/// 每次 Pass 将有效半径减半，因此 Pass 数为 `log2(radius)`。
///
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

/// 卷积核的总权重；必须等于 1。
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
