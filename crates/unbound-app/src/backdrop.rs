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

use std::time::{Duration, Instant};

use unbound_asset::AssetId;
use unbound_backdrop::{
    wallpaper_stamp, BackdropGenerator, MicaRequest, UvRect, WallpaperStamp, WallpaperTexture,
};
use unbound_math::Rect;
use unbound_widgets::ImageAssetResolver;

use crate::{AppEventSender, ViewBuilder};

pub mod controller;
pub use controller::{
    fallback_reason_zh, material_name_zh, plan_summary_zh, WindowBackdropController,
};

/// 云母纹理就绪事件 ID（后台线程 → UI 线程）。
pub const MICA_READY_EVENT_ID: u64 = unbound_backdrop::MICA_READY_EVENT_ID;

/// 该事件 ID 是否表示云母纹理已生成完毕。
#[must_use]
pub const fn is_mica_ready_event(id: u64) -> bool {
    id == MICA_READY_EVENT_ID
}

/// 自绘后景双纹理叠化的保底时长（秒）：即使后台纹理晚于前景过渡到达也不低于 150ms。
pub const MIN_BACKDROP_CROSSFADE_SECS: f32 = 0.15;

/// 整窗云母背板：持有生成器与已提交的纹理句柄，并在新代次纹理到达时与旧纹理叠化淡入。
#[derive(Debug)]
pub struct MicaBackdrop {
    generator: BackdropGenerator,
    /// 已解码并正在显示的纹理。
    asset: Option<AssetId>,
    /// 叠化过渡期间保留的上一代纹理；过渡收敛后立即释放。
    previous_asset: Option<AssetId>,
    /// 双纹理叠化的起始时间戳。
    crossfade_start: Option<Instant>,
    /// 本次双纹理叠化的总时长（秒，保底不低于 [`MIN_BACKDROP_CROSSFADE_SECS`]）。
    crossfade_duration_secs: f32,
    /// 新提交、仍在异步解码的纹理；解码完成前继续用旧纹理垫底。
    pending: Option<AssetId>,
    visible: bool,
    enabled: bool,
    /// 当前纹理在窗口内的归一化采样区域。
    uv: Rect,
    /// 缓存的壁纸源指纹：避免每帧查询注册表与文件元数据。
    wallpaper: Option<WallpaperStamp>,
    /// 上次查询壁纸指纹的时间。
    wallpaper_checked: Option<Instant>,
}

impl Default for MicaBackdrop {
    fn default() -> Self {
        Self {
            generator: BackdropGenerator::new(),
            asset: None,
            previous_asset: None,
            crossfade_start: None,
            crossfade_duration_secs: MIN_BACKDROP_CROSSFADE_SECS,
            pending: None,
            visible: false,
            enabled: true,
            uv: Rect::new(0.0, 0.0, 1.0, 1.0),
            wallpaper: None,
            wallpaper_checked: None,
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
            let now = Instant::now();
            let stale = self
                .wallpaper_checked
                .is_none_or(|checked| now.duration_since(checked) >= Duration::from_secs(2));
            if stale {
                self.wallpaper = wallpaper_stamp();
                self.wallpaper_checked = Some(now);
            }
            self.generator.sync(
                MicaRequest::from_texture(&texture, self.wallpaper),
                move || {
                    if let Some(sender) = wake {
                        sender.send_custom(MICA_READY_EVENT_ID, "");
                    }
                },
            );
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

        let mut just_promoted = false;
        if let Some(pending) = self.pending {
            if view.assets.resolve(pending).is_some() {
                let transition_enabled = view
                    .ctx
                    .theme_transition
                    .effective_duration_secs(view.ctx.motion_policy.reduced_motion)
                    > 0.0;
                if transition_enabled && self.asset.is_some() && self.asset != Some(pending) {
                    self.previous_asset = self.asset;
                    self.crossfade_start = Some(Instant::now());
                    self.crossfade_duration_secs = view
                        .ctx
                        .theme_transition_remaining_secs
                        .max(MIN_BACKDROP_CROSSFADE_SECS);
                } else {
                    self.previous_asset = None;
                    self.crossfade_start = None;
                }
                self.asset = Some(pending);
                self.pending = None;
                just_promoted = true;
            }
        }

        // 若存在进行中的双纹理叠化，按缓动进度以双图元透明度叠化绘制；收敛后立即释放旧纹理句柄。
        if let (Some(prev), Some(start)) = (self.previous_asset, self.crossfade_start) {
            let elapsed = if just_promoted {
                0.0
            } else {
                start.elapsed().as_secs_f32()
            };
            if let Some(alpha) = self.sample_crossfade_alpha(elapsed) {
                let base_key = key ^ 0x517C_C1B7_2722_0A95;
                let base_ok =
                    view.backdrop_image_sampled_with_opacity(base_key, prev, Some(self.uv), 1.0);
                let top_ok = self.asset.is_some_and(|asset| {
                    view.backdrop_image_sampled_with_opacity(key, asset, Some(self.uv), alpha)
                });
                self.visible = base_ok || top_ok;
                return self.visible;
            }
            // 叠化已收敛：释放旧纹理句柄并回落为单图元绘制。
            self.previous_asset = None;
            self.crossfade_start = None;
        }

        self.visible = self
            .asset
            .is_some_and(|asset| view.backdrop_image_sampled(key, asset, Some(self.uv)));
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
        if self.pending.is_some()
            || self.crossfade_start.is_some()
            || self.generator.is_pending()
            || self.generator.has_result()
        {
            return true;
        }
        self.asset.is_some() && !self.visible
    }

    /// 当前已提交的资产句柄。
    #[must_use]
    pub fn asset(&self) -> Option<AssetId> {
        self.asset
    }

    /// 叠化过渡期间保留的上一代资产句柄（收敛后自动归 `None`）。
    #[must_use]
    pub fn previous_asset(&self) -> Option<AssetId> {
        self.previous_asset
    }

    /// 计算给定经过时间下新纹理的叠化不透明度；返回 `None` 表示已收敛到达终态。
    #[must_use]
    pub fn sample_crossfade_alpha(&self, elapsed_secs: f32) -> Option<f32> {
        let dur = self
            .crossfade_duration_secs
            .max(MIN_BACKDROP_CROSSFADE_SECS);
        if elapsed_secs >= dur {
            None
        } else {
            let raw = (elapsed_secs / dur).clamp(0.0, 1.0);
            Some(unbound_theme::ease_out_cubic_bezier(raw))
        }
    }

    /// 启动一次由测试或宿主驱动的自绘双纹理叠化（采用前景剩余时长并保底不低于 150ms）。
    pub fn begin_crossfade_for_test(
        &mut self,
        previous: AssetId,
        next: AssetId,
        foreground_remaining_secs: f32,
    ) {
        self.previous_asset = Some(previous);
        self.asset = Some(next);
        self.crossfade_start = Some(Instant::now());
        self.crossfade_duration_secs = foreground_remaining_secs.max(MIN_BACKDROP_CROSSFADE_SECS);
    }

    /// 按确定性经过时间推进双纹理叠化并在收敛后释放旧纹理句柄。
    pub fn step_crossfade_for_test(&mut self, elapsed_secs: f32) -> Option<f32> {
        self.crossfade_start?;
        match self.sample_crossfade_alpha(elapsed_secs) {
            Some(alpha) => Some(alpha),
            None => {
                self.previous_asset = None;
                self.crossfade_start = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_backdrop::MaterialKind;

    /// 一份固定尺寸的自绘纹理规格。
    fn texture(material: MaterialKind) -> WallpaperTexture {
        WallpaperTexture {
            width: 1920,
            height: 1080,
            screen_width: 1920,
            screen_height: 1080,
            dpi_scale: 1.0,
            max_width: 640,
            sigma: 18.0,
            dark: false,
            material,
        }
    }

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
    fn a_window_move_only_updates_the_sample_rect() {
        let texture = texture(MaterialKind::Mica);
        let mut backdrop = MicaBackdrop::new();
        assert!(backdrop.sync(Some(texture), UvRect::new(0.25, 0.25, 0.5, 0.5), None));
        assert!(!backdrop.sync(Some(texture), UvRect::new(0.25, 0.25, 0.5, 0.5), None));
        assert!(
            backdrop.sync(None, UvRect::new(0.0, 0.0, 0.5, 0.5), None),
            "moving the window must invalidate the emitted overlay even if the texture is reused"
        );
    }

    #[test]
    fn switching_the_material_keeps_the_old_texture_until_the_new_one_is_ready() {
        let mut backdrop = MicaBackdrop::new();
        // 先把上一代纹理摆成「正在显示」。
        backdrop.asset = Some(AssetId(7));
        backdrop.visible = true;

        backdrop.sync(
            Some(texture(MaterialKind::Mica)),
            UvRect::new(0.0, 0.0, 1.0, 1.0),
            None,
        );
        // 切到云母变体：请求失效，生成器进入下一代。
        backdrop.sync(
            Some(texture(MaterialKind::MicaAlt)),
            UvRect::new(0.0, 0.0, 1.0, 1.0),
            None,
        );

        assert_eq!(
            backdrop.asset(),
            Some(AssetId(7)),
            "the old texture must survive until the new generation is decoded"
        );
        assert!(
            backdrop.visible(),
            "the window must keep drawing the old texture instead of flashing the solid color"
        );
        assert!(
            backdrop.needs_frame(),
            "the backdrop must keep requesting frames while the new texture is pending"
        );
    }

    #[test]
    fn only_the_ready_event_id_is_recognized() {
        assert!(is_mica_ready_event(MICA_READY_EVENT_ID));
        assert!(!is_mica_ready_event(MICA_READY_EVENT_ID + 1));
    }

    #[test]
    fn self_drawn_crossfade_enforces_150ms_floor_and_releases_previous_asset() {
        let mut mica = MicaBackdrop::new();
        let old_id = AssetId(1);
        let new_id = AssetId(2);
        // 即使前景仅剩 30ms（异步纹理迟到），叠化时长仍保底 150ms。
        mica.begin_crossfade_for_test(old_id, new_id, 0.03);
        assert!(mica.needs_frame());
        assert_eq!(mica.previous_asset(), Some(old_id));
        assert_eq!(mica.asset(), Some(new_id));

        let mid = mica
            .step_crossfade_for_test(0.075)
            .expect("mid-crossfade should still be active");
        assert!(mid > 0.0 && mid < 1.0);
        assert_eq!(mica.previous_asset(), Some(old_id));

        // 达到 150ms 后收敛并释放旧纹理句柄。
        assert!(mica.step_crossfade_for_test(0.15).is_none());
        assert_eq!(mica.previous_asset(), None);
        assert_eq!(mica.asset(), Some(new_id));
    }
}
