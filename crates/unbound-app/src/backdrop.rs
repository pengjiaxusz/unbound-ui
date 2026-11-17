//! Mica 背板：把 [`unbound_backdrop`] 的壁纸纹理接到 [`ViewBuilder`] 上。
//!
//! 分工刻意保持清晰：
//!
//! - `unbound-backdrop` 只管生成（壁纸 → 模糊 → 色调 → PNG），不依赖应用运行时；
//! - 本模块管状态（何时重建、纹理是否可见）与提交（`request_png` / `backdrop_image`）。
//!
//! 应用只需三步：
//!
//! ```ignore
//! // init: 交给宿主自动处理（默认启用）
//! // view:
//! self.mica.sync(viewport, dark, view.ctx.event_sender());
//! let mica_visible = self.mica.draw(view, ui_key!("app_mica_backdrop"));
//! // 重建场景时用 `mica_visible` 决定根背景是否透明
//! // reduce:
//! AppEvent::Custom { id, .. } if unbound_app::is_mica_ready_event(id) => DirtyFlags::ALL,
//! ```

use unbound_asset::AssetId;
use unbound_backdrop::{BackdropGenerator, MicaRequest};

use crate::{AppEventSender, ViewBuilder};

/// 云母纹理就绪事件 ID（后台线程 → UI 线程）。
pub const MICA_READY_EVENT_ID: u64 = unbound_backdrop::MICA_READY_EVENT_ID;

/// 该事件 ID 是否表示云母纹理已生成完毕。
#[must_use]
pub const fn is_mica_ready_event(id: u64) -> bool {
    id == MICA_READY_EVENT_ID
}

/// 整窗云母背板：持有生成器与已提交的纹理句柄。
#[derive(Debug)]
pub struct MicaBackdrop {
    generator: BackdropGenerator,
    asset: Option<AssetId>,
    visible: bool,
    enabled: bool,
}

impl Default for MicaBackdrop {
    fn default() -> Self {
        Self {
            generator: BackdropGenerator::new(),
            asset: None,
            visible: false,
            enabled: true,
        }
    }
}

impl MicaBackdrop {
    /// 创建默认启用云母的背板。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 开关云母。关闭会立即丢弃纹理，下一帧根背景回到不透明纯色。
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.asset = None;
        self.visible = false;
    }

    /// 云母是否启用。
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 让纹理与当前视口 / 主题保持同步。
    ///
    /// `viewport` 为逻辑像素尺寸；`wake` 用于生成完成后唤醒 UI 线程，
    /// 通常是 [`crate::AppContext::event_sender`] 的返回值。
    pub fn sync(&mut self, viewport: (f32, f32), dark: bool, wake: Option<AppEventSender>) {
        if !self.enabled {
            return;
        }
        let width = viewport.0.max(1.0).round() as u32;
        let height = viewport.1.max(1.0).round() as u32;
        self.generator
            .sync(MicaRequest::new(width, height, dark), move || {
                if let Some(sender) = wake {
                    sender.send_custom(MICA_READY_EVENT_ID, "");
                }
            });
    }

    /// 提交新纹理并把背板画在整窗最底层。
    ///
    /// 返回 `true` 表示本帧有纹理覆盖视口；场景重建应据此把根背景设为透明，
    /// 否则不透明底色会把云母盖住。
    pub fn draw(&mut self, view: &mut ViewBuilder<'_>, key: u64) -> bool {
        if !self.enabled {
            self.visible = false;
            return false;
        }
        if let Some(image) = self.generator.take() {
            self.asset = Some(view.request_png(image.png));
        }
        self.visible = self
            .asset
            .is_some_and(|asset| view.backdrop_image(key, asset));
        self.visible
    }

    /// 上一帧绘制后云母是否可见（纹理已解码并提交）。
    #[must_use]
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// 是否还需要后续帧才能完成云母呈现。
    ///
    /// 自绘宿主（不走 `run_windowed`）没有事件驱动时，应在每帧末尾据此
    /// 继续请求重绘并 `invalidate()`，否则纹理可能停在「已生成但从未提交」。
    #[must_use]
    pub fn needs_frame(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if self.generator.is_pending() || self.generator.has_result() {
            return true;
        }
        self.asset.is_some() && !self.visible
    }

    /// 当前已提交的资产句柄。
    #[must_use]
    pub fn asset(&self) -> Option<AssetId> {
        self.asset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mica_is_enabled_by_default() {
        let backdrop = MicaBackdrop::new();
        assert!(backdrop.is_enabled());
        assert!(!backdrop.visible());
    }

    #[test]
    fn disabling_drops_the_texture() {
        let mut backdrop = MicaBackdrop::new();
        backdrop.set_enabled(false);
        assert!(!backdrop.is_enabled());
        assert!(backdrop.asset().is_none());
        assert!(!backdrop.visible());
    }

    #[test]
    fn only_the_ready_event_id_is_recognized() {
        assert!(is_mica_ready_event(MICA_READY_EVENT_ID));
        assert!(!is_mica_ready_event(MICA_READY_EVENT_ID + 1));
    }
}
