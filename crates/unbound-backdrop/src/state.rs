//! 后台生成状态机：请求去重、代次淘汰与就绪唤醒。

use std::sync::{Arc, Mutex};
use std::thread;

use crate::mica::{generate, MicaImage, MicaRequest};

/// 云母纹理生成完成后投递的自定义事件 ID。
///
/// 生成发生在后台线程，而应用只在「有脏标记」的帧才重建场景：
/// 若生成完成后没人唤醒 UI，纹理永远停在「已生成但从未上传」的状态，
/// 整窗退回纯色底。宿主应把该 ID 的 [`crate::state::MICA_READY_EVENT_ID`]
/// 事件映射为一次全量重建。
pub const MICA_READY_EVENT_ID: u64 = 0xC4A0_0C1A;

/// 后台线程与主线程共享的代次结果槽：`(代次, 图像)`。
///
/// 图像为 `None` 表示生成失败，与“尚未结算”区分。
type SharedSlot = Arc<Mutex<Option<(u64, Option<MicaImage>)>>>;

/// 云母背板的异步生成器。
///
/// 壁纸解码、模糊与 PNG 编码都在工作线程完成；主线程只做「就绪后取走」
/// 与「交给资产中心」。主题或窗口尺寸变化时旧结果按代次自动淘汰。
#[derive(Debug, Default)]
pub struct BackdropGenerator {
    request: Option<MicaRequest>,
    /// 当前代次的结果槽：`None` 图像表示生成失败（拿不到壁纸），
    /// 同样视为“已结算”，调用方据此停止请求后续帧。
    slot: Option<SharedSlot>,
    generation: u64,
    delivered: bool,
}

impl BackdropGenerator {
    /// 创建空生成器（首次 [`Self::sync`] 触发首帧生成）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当请求与当前纹理不等价时启动后台生成（幂等）。
    ///
    /// `wake` 在结果就绪后于工作线程执行，应尽量轻量且线程安全；
    /// 典型实现是向 UI 线程投递 [`MICA_READY_EVENT_ID`] 事件。
    pub fn sync<F>(&mut self, request: MicaRequest, wake: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if let Some(current) = self.request {
            if current.matches(&request) {
                return;
            }
        }
        self.request = Some(request);
        self.generation = self.generation.wrapping_add(1);
        self.delivered = false;

        let generation = self.generation;
        let slot = Arc::new(Mutex::new(None));
        self.slot = Some(Arc::clone(&slot));
        thread::spawn(move || {
            // 生成失败（含解码 panic）也要落盘结果并唤醒：否则状态永远停在
            // “生成中”，自绘宿主会为一张不会到来的纹理持续请求帧。
            let image =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| generate(&request)))
                    .ok()
                    .flatten();
            if let Ok(mut guard) = slot.lock() {
                *guard = Some((generation, image));
            }
            wake();
        });
    }

    /// 取走已生成的结果（每代次只返回一次）。
    ///
    /// 被新请求取代的旧结果会被丢弃，避免主题切换时闪回旧色调。
    pub fn take(&mut self) -> Option<MicaImage> {
        if self.delivered {
            return None;
        }
        let slot = self.slot.as_ref()?;
        let (generation, image) = slot.lock().ok()?.take()?;
        if generation != self.generation {
            return None;
        }
        self.delivered = true;
        image
    }

    /// 当前代次是否已有结果等待取走（成功或失败）。
    #[must_use]
    pub fn has_result(&self) -> bool {
        let Some(slot) = &self.slot else {
            return false;
        };
        match slot.lock() {
            Ok(guard) => matches!(
                guard.as_ref(),
                Some((generation, _)) if *generation == self.generation
            ),
            Err(_) => false,
        }
    }

    /// 当前代次是否已结算（结果已取走，或正等待取走）。
    ///
    /// 失败同样算结算：没有壁纸时继续请求帧只会空转。
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.delivered || self.has_result()
    }

    /// 当前是否仍在生成（尚未结算）。
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.request.is_some() && !self.is_settled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_generator_has_nothing_to_deliver() {
        let mut generator = BackdropGenerator::new();
        assert!(!generator.is_pending());
        assert!(generator.take().is_none());
    }

    #[test]
    fn syncing_the_same_request_does_not_start_a_second_generation() {
        let mut generator = BackdropGenerator::new();
        generator.sync(MicaRequest::new(800, 600, true), || {});
        let generation = generator.generation;
        generator.sync(MicaRequest::new(800, 600, true), || {});
        assert_eq!(
            generator.generation, generation,
            "a near-identical request must be reused"
        );
    }

    #[test]
    fn a_theme_change_supersedes_the_previous_generation() {
        let mut generator = BackdropGenerator::new();
        generator.sync(MicaRequest::new(800, 600, true), || {});
        let generation = generator.generation;
        generator.sync(MicaRequest::new(800, 600, false), || {});
        assert_eq!(generator.generation, generation + 1);
        assert!(!generator.delivered);
    }
}
