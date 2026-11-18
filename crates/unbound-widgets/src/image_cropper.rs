//! ImageCropper：图像裁剪框与九宫格手柄。
//!
//! 裁剪框内部绘制图像表面与四条遮罩带；框边输出 8 个手柄（四角 + 四边中点）。
//! 手柄数量固定，悬停只改变框边与手柄的填充令牌。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 手柄边长。
pub const CROP_HANDLE_SIZE: f32 = 10.0;
/// 遮罩带厚度。
pub const CROP_SCRIM_BAND: f32 = 10.0;

/// 图像裁剪器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ImageCropperDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 裁剪框相对控件矩形的内缩量。
    pub inset: f32,
    /// 手柄边长。
    pub handle: f32,
    /// 无障碍标签。
    pub label: String,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl ImageCropperDesc {
    /// 创建内缩 24px 的裁剪框。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            inset: 24.0,
            handle: CROP_HANDLE_SIZE,
            label: "Crop image".to_owned(),
            disabled: false,
            z: 728,
        }
    }

    /// 覆盖裁剪框内缩量。
    #[must_use]
    pub fn with_inset(mut self, inset: f32) -> Self {
        self.inset = inset.max(0.0);
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 裁剪框矩形。
    #[must_use]
    pub fn crop_rect(&self) -> Rect {
        let inset = self
            .inset
            .min(self.rect.width() * 0.25)
            .min(self.rect.height() * 0.25);
        Rect::new(
            self.rect.origin.x + inset,
            self.rect.origin.y + inset,
            (self.rect.width() - inset * 2.0).max(0.0),
            (self.rect.height() - inset * 2.0).max(0.0),
        )
    }

    /// 第 `index` 个手柄的矩形（0..4 为四角，4..8 为四边中点）。
    #[must_use]
    pub fn handle_rect(&self, index: usize) -> Rect {
        let crop = self.crop_rect();
        let size = self.handle.max(0.0);
        let half = size * 0.5;
        let (x, y) = match index % 8 {
            0 => (crop.origin.x, crop.origin.y),
            1 => (crop.max().x, crop.origin.y),
            2 => (crop.max().x, crop.max().y),
            3 => (crop.origin.x, crop.max().y),
            4 => (crop.center().x, crop.origin.y),
            5 => (crop.max().x, crop.center().y),
            6 => (crop.center().x, crop.max().y),
            _ => (crop.origin.x, crop.center().y),
        };
        Rect::new(x - half, y - half, size, size)
    }

    /// 遮罩带矩形：上、下、左、右四条。
    #[must_use]
    pub fn scrim_rects(&self) -> [Rect; 4] {
        let crop = self.crop_rect();
        [
            Rect::new(
                self.rect.origin.x,
                self.rect.origin.y,
                self.rect.width(),
                (crop.origin.y - self.rect.origin.y).max(0.0),
            ),
            Rect::new(
                self.rect.origin.x,
                crop.max().y,
                self.rect.width(),
                (self.rect.max().y - crop.max().y).max(0.0),
            ),
            Rect::new(
                self.rect.origin.x,
                crop.origin.y,
                (crop.origin.x - self.rect.origin.x).max(0.0),
                crop.height(),
            ),
            Rect::new(
                crop.max().x,
                crop.origin.y,
                (self.rect.max().x - crop.max().x).max(0.0),
                crop.height(),
            ),
        ]
    }
}

/// 输出图像裁剪器的图元、命中区域与语义节点。
#[must_use]
pub fn emit_image_cropper(
    node: u64,
    desc: &ImageCropperDesc,
    control: ControlState,
    label_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;

    let mut primitives = Vec::with_capacity(13 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    primitives.push(surface(rect, 8.0, Token::SURFACE_SUNKEN, z));
    for band in desc.scrim_rects() {
        primitives.push(surface(band, 0.0, Token::SCRIM, z + 1));
    }
    primitives.push(outlined(
        desc.crop_rect(),
        0.0,
        Token::NONE,
        if disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if hovered {
            Token::PRIMARY_HOVER
        } else {
            Token::PRIMARY
        },
        2.0,
        0.0,
        z + 2,
    ));
    for index in 0..8 {
        primitives.push(surface(
            desc.handle_rect(index),
            2.0,
            if disabled {
                Token::CONTROL_DISABLED_BG
            } else {
                Token::PRIMARY
            },
            z + 3,
        ));
    }
    emit_glyphs(
        label_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 4,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect,
        8.0,
        Token::FOCUS_RING,
        2.0,
        z + 5,
        control.focused && !disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect).with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Image, desc.label.clone(), rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ImageCropperDesc {
        ImageCropperDesc::new(Rect::new(0.0, 0.0, 320.0, 220.0))
    }

    #[test]
    fn the_crop_frame_is_inset_from_the_image() {
        let d = desc();
        let crop = d.crop_rect();
        assert!(d.rect.contains_rect(&crop));
        assert_eq!(crop.origin.x, d.rect.origin.x + d.inset);
        assert_eq!(crop.origin.y, d.rect.origin.y + d.inset);
    }

    #[test]
    fn eight_handles_ring_the_crop_frame() {
        let d = desc();
        let crop = d.crop_rect();
        for index in 0..8 {
            let handle = d.handle_rect(index);
            assert_eq!(handle.size.x, d.handle);
            // 手柄以裁剪框边界为中心，因此最多外溢半个手柄。
            assert!(handle.center().x >= crop.origin.x - 0.01);
            assert!(handle.center().x <= crop.max().x + 0.01);
        }
    }

    #[test]
    fn the_scrim_stays_outside_the_crop_frame() {
        let d = desc();
        let crop = d.crop_rect();
        let [top, bottom, left, right] = d.scrim_rects();
        assert!(top.max().y <= crop.origin.y + 0.01);
        assert!(bottom.origin.y >= crop.max().y - 0.01);
        assert!(left.max().x <= crop.origin.x + 0.01);
        assert!(right.origin.x >= crop.max().x - 0.01);
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc();
        let mut counts = Vec::new();
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let state = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        counts.push(emit_image_cropper(1, &d, state, &mut []).primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}
