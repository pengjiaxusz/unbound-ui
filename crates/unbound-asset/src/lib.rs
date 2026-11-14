//! 异步资产流式加载（`docs` §8.2）。
//!
//! 该管线共四个阶段，本 crate 建模了位于驱动外部的前三个阶段：
//!
//!
//! 1. **Fetch（获取）** — 应用将已读取的字节交给管线，或由管线从磁盘读取。
//!    （不阻塞渲染线程。）
//! 2. **Decode（解码）** — 工作线程池将字节解码为 GPU 就绪的载荷，
//!    所有解码操作均不在渲染线程上执行。
//! 3. **Stage（暂存）** — 载荷写入多槽位 CPU 暂存环形缓冲区，
//!    传输队列通过 PCIe 从中读取。
//! 4. **Transfer（传输）** — 专用传输队列将载荷拷贝到 GPU 显存并触发时间线信号量；
//!    图形队列在采样纹理前等待该信号量值。
//!
//!
//! 贯穿整体设计的核心约束：**帧耗时绝不能受资产管线影响**。无法入队的请求会被拒绝，
//! 解码失败会上报错误，未就绪的资产则绘制占位符。任何情况都不会阻塞渲染循环。
//!
//!

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod decode;
pub mod pipeline;
pub mod ring;
pub mod transfer;

pub use decode::{Decoder, KindDecoder, PngDecoder, RawDecoder};
pub use pipeline::{AssetPipeline, Completed, PipelineConfig};
pub use ring::{StagingError, StagingRef, StagingRing};
pub use transfer::{
    ownership_barrier, AssetError, AssetId, AssetKind, AssetStatus, DecodedAsset, ImageLayout,
    OwnershipTransfer, Placeholder, QueueFamily, TimelineGate, TransferBarrier,
};

/// 在渲染线程之外从磁盘读取文件。
///
/// 该函数有意设计为阻塞调用——它只应在工作线程中调用，绝不可在帧循环中调用。
///
///
/// # Errors
/// 当文件无法读取时返回 I/O 错误。
pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<Vec<u8>, AssetError> {
    std::fs::read(path).map_err(|error| AssetError::Decode(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_reported_rather_than_panicking() {
        let error = read_file("this/file/does/not/exist.bin").unwrap_err();
        assert!(matches!(error, AssetError::Decode(_)));
    }

    #[test]
    fn a_file_on_disk_is_read_into_the_pipeline() {
        let path = std::env::temp_dir().join("unbound-asset-test.bin");
        std::fs::write(&path, b"payload").unwrap();
        let bytes = read_file(&path).unwrap();
        assert_eq!(bytes, b"payload");
        let _ = std::fs::remove_file(&path);
    }
}
