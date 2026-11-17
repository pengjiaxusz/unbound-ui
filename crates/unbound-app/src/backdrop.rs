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
//! self.mica.sync(plan.texture, plan.uv, view.ctx.event_sender());
//! let mica_visible = self.mica.draw(view, ui_key!("app_mica_backdrop"));
//! // 重建场景时用 `mica_visible` 决定根背景是否透明
//! // reduce:
//! AppEvent::Custom { id, .. } if unbound_app::is_mica_ready_event(id) => DirtyFlags::ALL,
//! ```

use unbound_asset::AssetId;
use unbound_backdrop::{BackdropGenerator, MicaRequest, UvRect, WallpaperTexture};
use unbound_math::Rect;

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
    /// 已解码并正在显示的纹理。
    asset: Option<AssetId>,
    /// 新提交、仍在异步解码的纹理；解码完成前继续用旧纹理垫底。
    pending: Option<AssetId>,
    visible: bool,
    enabled: bool,
    /// 当前纹理在窗口内的归一化采样区域。
    uv: Rect,
}

impl Default for MicaBackdrop {
    fn default() -> Self {
        Self {
            generator: BackdropGenerator::new(),
            asset: None,
            pending: None,
            visible: false,
            enabled: true,
            uv: Rect::new(0.0, 0.0, 1.0, 1.0),
        }
    }
}

impl MicaBackdrop {
    /// 创建默认启用云母的背板。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 开关云母。关闭只停止绘制，保留上一代纹理与生成器状态。
    ///
    /// 原生与关闭模式都不需要壁纸纹理，但用户切回自绘时必须立刻复用旧纹理：
    /// 若关闭时丢弃纹理，重新生成期间根背景会先闪回不透明纯色，与「新纹理
    /// 就绪前保留旧纹理」的不变量冲突。只有从未生成成功过（没有旧纹理可复用）
    /// 时才重置生成器，让重新启用后能再次请求。
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.visible = false;
        if enabled && self.asset.is_none() {
            self.generator = BackdropGenerator::new();
        }
    }

    /// 云母是否启用。
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 让纹理与后景计划保持同步，并返回采样区域是否变化。
    ///
    /// `texture` 为计划给出的纹理规格；`uv` 为窗口在纹理中的归一化子矩形。
    /// 采样区域变化不需要重新生成纹理，但会改变叠加图元，调用方应据此重建场景。
    /// `wake` 用于生成完成后唤醒 UI 线程，通常是 [`crate::AppContext::event_sender`]
    /// 的返回值。
    pub fn sync(
        &mut self,
        texture: Option<WallpaperTexture>,
        uv: UvRect,
        wake: Option<AppEventSender>,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(texture) = texture {
            self.generator
                .sync(MicaRequest::from_texture(&texture), move || {
                    if let Some(sender) = wake {
                        sender.send_custom(MICA_READY_EVENT_ID, "");
                    }
                });
        }
        let next = Rect::new(uv.x, uv.y, uv.width, uv.height);
        let changed = next != self.uv;
        self.uv = next;
        changed
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
            self.pending = Some(view.request_png(image.png));
        }

        if let Some(pending) = self.pending {
            if view.backdrop_image_sampled(key, pending, Some(self.uv)) {
                self.asset = Some(pending);
                self.pending = None;
                self.visible = true;
            } else {
                // 新纹理还在解码：继续显示旧纹理，避免主题切换闪一帧纯色。
                self.visible = self
                    .asset
                    .is_some_and(|asset| view.backdrop_image_sampled(key, asset, Some(self.uv)));
            }
        } else {
            self.visible = self
                .asset
                .is_some_and(|asset| view.backdrop_image_sampled(key, asset, Some(self.uv)));
        }
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
        if self.pending.is_some() || self.generator.is_pending() || self.generator.has_result() {
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
    fn disabling_hides_the_texture_but_keeps_it_for_a_flash_free_reenable() {
        let mut backdrop = MicaBackdrop::new();
        backdrop.asset = Some(AssetId(7));
        backdrop.visible = true;

        backdrop.set_enabled(false);
        assert!(!backdrop.is_enabled());
        assert!(!backdrop.visible());
        assert_eq!(
            backdrop.asset(),
            Some(AssetId(7)),
            "the previous texture must survive so re-enabling can draw it immediately"
        );

        backdrop.set_enabled(true);
        assert!(backdrop.is_enabled());
        assert_eq!(
            backdrop.asset(),
            Some(AssetId(7)),
            "re-enabling must not drop the texture that is about to be drawn again"
        );
    }

    #[test]
    fn only_the_ready_event_id_is_recognized() {
        assert!(is_mica_ready_event(MICA_READY_EVENT_ID));
        assert!(!is_mica_ready_event(MICA_READY_EVENT_ID + 1));
    }
}
