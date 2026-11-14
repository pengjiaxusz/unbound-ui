//! 滑块控件：轨道、已填充部分与滑块拇指。
//!
//! 滑块是唯一具有*连续*值的控件，因此也是唯一需要将指针位置映射为数值的控件。
//! 该映射仅存在于 [`value_from_pointer`]：应用请求数值，控件负责绘制。
//!
//!

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认轨道高度。
pub const SLIDER_TRACK: f32 = 4.0;

/// 默认滑块拇指尺寸。
pub const SLIDER_THUMB: f32 = 16.0;

/// 滑块的运行方向。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// 轨道从左到右。
    #[default]
    Horizontal,
    /// 轨道从下到上。
    Vertical,
}

/// 滑块可识别的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderKey {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

/// 滑块绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliderTokens {
    /// 滑块背后的轨道。
    pub track: Token,
    /// 轨道的已填充部分。
    pub fill: Token,
    /// 指针悬停在滑块上时的已填充部分。
    ///
    /// 滑块必须在被抓取前就响应指针，否则在开始拖拽前轨道看起来毫无反应。
    /// 状态仅改变填充解析到的颜色槽位，因此悬停只是修补单个图元，而非新建图元。
    pub fill_hover: Token,
    /// 拖拽滑块时的已填充部分。
    pub fill_pressed: Token,
    /// 滑块拇指。
    pub thumb: Token,
    /// 滑块拇指边框。
    pub thumb_border: Token,
    /// 禁用滑块的轨道。
    pub track_disabled: Token,
    /// 禁用滑块的已填充部分。
    pub fill_disabled: Token,
    /// 禁用滑块的拇指。
    pub thumb_disabled: Token,
    /// 禁用拇指的边框。
    pub thumb_border_disabled: Token,
    /// 标签。
    pub label: Token,
    /// 禁用滑块的标签。
    pub label_disabled: Token,
    /// 键盘焦点环。
    pub focus_ring: Token,
}

impl SliderTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            track: Token::CONTROL_TRACK,
            fill: Token::PRIMARY,
            fill_hover: Token::PRIMARY_HOVER,
            fill_pressed: Token::PRIMARY_PRESSED,
            thumb: Token::CONTROL_THUMB,
            thumb_border: Token::OUTLINE,
            track_disabled: Token::CONTROL_DISABLED_BG,
            fill_disabled: Token::CONTROL_DISABLED_BG,
            thumb_disabled: Token::CONTROL_DISABLED_THUMB,
            thumb_border_disabled: Token::CONTROL_DISABLED_BORDER,
            label: Token::TEXT_SECONDARY,
            label_disabled: Token::TEXT_DISABLED,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 滑块的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SliderDesc {
    /// 滑块占用的行矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 当前数值。
    pub value: f32,
    /// 最小值。
    pub min: f32,
    /// 最大值。
    pub max: f32,
    /// 步长；0 表示连续滑动。
    pub step: f32,
    /// 轨道高度（垂直模式下为轨道宽度）。
    pub track_height: f32,
    /// 滑块拇指尺寸。
    pub thumb_size: f32,
    /// 轨道是水平还是垂直排列。
    pub orientation: Orientation,
    /// 双滑块区间的下限值（若有）。
    pub range_start: Option<f32>,
    /// 是否在每个步进位置绘制刻度。
    pub show_ticks: bool,
    /// 数值提示气泡是否跟随拇指。
    pub show_value_tooltip: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl SliderDesc {
    /// 使用默认尺寸创建的滑块。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>, value: f32, min: f32, max: f32) -> Self {
        Self {
            rect,
            label: label.into(),
            value: value.clamp(min.min(max), max.max(min)),
            min,
            max,
            step: 0.0,
            track_height: SLIDER_TRACK,
            thumb_size: SLIDER_THUMB,
            orientation: Orientation::Horizontal,
            range_start: None,
            show_ticks: false,
            show_value_tooltip: false,
            z: 700,
        }
    }

    /// 设置步长。
    #[must_use]
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// 设置排列方向。
    #[must_use]
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// 添加第二个下限滑块拇指。
    #[must_use]
    pub fn with_range_start(mut self, range_start: f32) -> Self {
        self.range_start = Some(range_start);
        self
    }

    /// 在每个步进位置绘制刻度。
    #[must_use]
    pub fn with_ticks(mut self, show_ticks: bool) -> Self {
        self.show_ticks = show_ticks;
        self
    }

    /// 启用数值提示气泡。
    #[must_use]
    pub fn with_value_tooltip(mut self, show_value_tooltip: bool) -> Self {
        self.show_value_tooltip = show_value_tooltip;
        self
    }

    /// 将数值归一化到 `[0, 1]`。
    #[must_use]
    pub fn ratio_of(&self, value: f32) -> f32 {
        let span = self.max - self.min;
        if span.abs() < f32::EPSILON {
            0.0
        } else {
            ((value - self.min) / span).clamp(0.0, 1.0)
        }
    }

    /// 归一化到 `[0, 1]` 的当前值。
    #[must_use]
    pub fn ratio(&self) -> f32 {
        self.ratio_of(self.value)
    }

    /// 双滑块区间的下限值，夹紧至 `[min, value]`。
    #[must_use]
    pub fn range_start_ratio(&self) -> Option<f32> {
        self.range_start
            .map(|start| self.ratio_of(start.min(self.value)))
    }

    /// 在行内居中的轨道矩形。
    #[must_use]
    pub fn track_rect(&self) -> Rect {
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                self.rect.origin.x,
                self.rect.origin.y + self.rect.size.y * 0.5 - self.track_height * 0.5,
                self.rect.size.x,
                self.track_height,
            ),
            Orientation::Vertical => Rect::new(
                self.rect.origin.x + self.rect.size.x * 0.5 - self.track_height * 0.5,
                self.rect.origin.y,
                self.track_height,
                self.rect.size.y,
            ),
        }
    }

    /// 轨道的已填充部分矩形。
    ///
    /// 存在区间起点时填充跨越两个拇指；否则从轨道起点延伸到当前值。
    #[must_use]
    pub fn fill_rect(&self) -> Rect {
        let track = self.track_rect();
        let high = self.ratio();
        let low = self
            .range_start_ratio()
            .map_or(0.0, |start| start.min(high));
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                track.origin.x + track.size.x * low,
                track.origin.y,
                track.size.x * (high - low),
                track.size.y,
            ),
            Orientation::Vertical => Rect::new(
                track.origin.x,
                track.origin.y + track.size.y * (1.0 - high),
                track.size.x,
                track.size.y * (high - low),
            ),
        }
    }

    /// 在 `ratio` 比例位置处的拇指矩形。
    #[must_use]
    pub fn thumb_rect_at(&self, ratio: f32) -> Rect {
        let track = self.track_rect();
        let half = self.thumb_size * 0.5;
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                track.origin.x + track.size.x * ratio - half,
                track.origin.y + track.size.y * 0.5 - half,
                self.thumb_size,
                self.thumb_size,
            ),
            Orientation::Vertical => Rect::new(
                track.origin.x + track.size.x * 0.5 - half,
                track.origin.y + track.size.y * (1.0 - ratio) - half,
                self.thumb_size,
                self.thumb_size,
            ),
        }
    }

    /// 当前拇指矩形。
    #[must_use]
    pub fn thumb_rect(&self) -> Rect {
        self.thumb_rect_at(self.ratio())
    }

    /// 双滑块区间的下限拇指矩形。
    #[must_use]
    pub fn range_start_thumb_rect(&self) -> Option<Rect> {
        self.range_start_ratio()
            .map(|ratio| self.thumb_rect_at(ratio))
    }

    /// 刻度标记所处的归一化比例列表。
    ///
    /// 步进滑块在每个步进处绘制刻度（上限 64 个）；连续滑块在四分位点绘制刻度。
    #[must_use]
    pub fn tick_ratios(&self) -> Vec<f32> {
        let span = (self.max - self.min).abs();
        if span < f32::EPSILON {
            return Vec::new();
        }
        if self.step > 0.0 {
            let count = (span / self.step).round() as usize;
            if (1..=64).contains(&count) {
                return (0..=count)
                    .map(|index| index as f32 / count as f32)
                    .collect();
            }
        }
        vec![0.25, 0.5, 0.75]
    }

    /// `ratio` 位置处的单个刻度矩形。
    #[must_use]
    pub fn tick_rect(&self, ratio: f32) -> Rect {
        let track = self.track_rect();
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                track.origin.x + track.size.x * ratio - 1.0,
                track.origin.y + track.size.y * 0.5 - 4.0,
                2.0,
                8.0,
            ),
            Orientation::Vertical => Rect::new(
                track.origin.x + track.size.x * 0.5 - 4.0,
                track.origin.y + track.size.y * (1.0 - ratio) - 1.0,
                8.0,
                2.0,
            ),
        }
    }

    /// 拇指上方（或侧方）的提示气泡矩形。
    #[must_use]
    pub fn tooltip_rect(&self) -> Rect {
        let thumb = self.thumb_rect();
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                thumb.origin.x + thumb.size.x * 0.5 - 20.0,
                thumb.origin.y - 26.0,
                40.0,
                20.0,
            ),
            Orientation::Vertical => Rect::new(
                thumb.max().x + 8.0,
                thumb.origin.y + thumb.size.y * 0.5 - 10.0,
                40.0,
                20.0,
            ),
        }
    }
}

/// 指针位置对应的数值。
///
/// 指针被夹紧到轨道范围，原始值按步长吸附（若设置了步长），最终结果夹紧到 `[min, max]`。
/// 步长*相对于 `min`* 应用，因此从 `10` 开始、步长 `0.5` 会产生 `10.0, 10.5, 11.0 …` 而不会漂移。
#[must_use]
pub fn value_from_pointer(pointer: Vec2, desc: &SliderDesc) -> f32 {
    let track = desc.track_rect();
    let ratio = match desc.orientation {
        Orientation::Horizontal => {
            if track.size.x.abs() < f32::EPSILON {
                0.0
            } else {
                ((pointer.x - track.origin.x) / track.size.x).clamp(0.0, 1.0)
            }
        }
        Orientation::Vertical => {
            if track.size.y.abs() < f32::EPSILON {
                0.0
            } else {
                (1.0 - (pointer.y - track.origin.y) / track.size.y).clamp(0.0, 1.0)
            }
        }
    };
    let raw = desc.min + ratio * (desc.max - desc.min);
    let stepped = if desc.step > 0.0 {
        ((raw - desc.min) / desc.step).round() * desc.step + desc.min
    } else {
        raw
    };
    stepped.clamp(desc.min.min(desc.max), desc.max.max(desc.min))
}

/// 单次键盘事件将滑块移动到的目标值。
///
/// Left/Down 减小，Right/Up 增大，PageUp/PageDown 移动十个步长，Home/End 跳至两端。
/// 连续滑块每次移动总跨度的 1%。
#[must_use]
pub fn reduce_slider_key(desc: &SliderDesc, key: SliderKey) -> f32 {
    let span = desc.max - desc.min;
    let step = if desc.step > 0.0 {
        desc.step
    } else {
        span.abs() * 0.01
    };
    let value = match key {
        SliderKey::Left | SliderKey::Down => desc.value - step,
        SliderKey::Right | SliderKey::Up => desc.value + step,
        SliderKey::PageDown => desc.value - step * 10.0,
        SliderKey::PageUp => desc.value + step * 10.0,
        SliderKey::Home => desc.min.min(desc.max),
        SliderKey::End => desc.max.max(desc.min),
    };
    value.clamp(desc.min.min(desc.max), desc.max.max(desc.min))
}

/// 滑块当前是否处于拖拽状态。
///
/// 路由器在按下时会捕获指针，因此即使指针离开轨道返回值仍保持为 true——与原生滑块一致。
#[must_use]
pub fn is_dragging(router: &unbound_input::Router, node: u64) -> bool {
    router.pointer().pressed() == Some(node)
}

/// 输出滑块：轨道、已填充部分、拇指与焦点环。
#[must_use]
pub fn emit_slider(
    node: u64,
    desc: &SliderDesc,
    state: ControlState,
    label: &[GlyphQuad],
    tokens: SliderTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(8 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;

    primitives.push(surface(
        desc.track_rect(),
        desc.track_height * 0.5,
        if state.disabled {
            tokens.track_disabled
        } else {
            tokens.track
        },
        z,
    ));
    primitives.push(surface(
        desc.fill_rect(),
        desc.track_height * 0.5,
        fill_slot(state, tokens),
        z,
    ));

    if desc.show_ticks {
        for ratio in desc.tick_ratios() {
            primitives.push(surface(
                desc.tick_rect(ratio),
                1.0,
                if state.disabled {
                    tokens.label_disabled
                } else {
                    tokens.label
                },
                z,
            ));
        }
    }

    let thumb_fill = if state.disabled {
        tokens.thumb_disabled
    } else {
        tokens.thumb
    };
    let thumb_border = if state.disabled {
        tokens.thumb_border_disabled
    } else {
        tokens.thumb_border
    };

    if let Some(start) = desc.range_start_thumb_rect() {
        primitives.push(outlined(
            start,
            desc.thumb_size * 0.5,
            thumb_fill,
            thumb_border,
            1.0,
            4.0,
            z,
        ));
    }

    let thumb = desc.thumb_rect();
    primitives.push(outlined(
        thumb,
        desc.thumb_size * 0.5,
        thumb_fill,
        thumb_border,
        1.0,
        if state.phase == PointerPhase::Pressed {
            8.0
        } else {
            4.0
        },
        z,
    ));

    if desc.show_value_tooltip {
        primitives.push(outlined(
            desc.tooltip_rect(),
            6.0,
            thumb_fill,
            thumb_border,
            1.0,
            4.0,
            z,
        ));
    }

    glyphs(
        label,
        if state.disabled {
            tokens.label_disabled
        } else {
            tokens.label
        },
        z,
        &mut primitives,
        &mut payloads,
    );

    // 焦点环始终输出：焦点变化仅改变其不透明度而非图元池结构，因此聚焦只是绘制变更而非重传。
    //
    primitives.push(focus_ring(
        thumb.inflate(3.0),
        desc.thumb_size * 0.5 + 3.0,
        tokens.focus_ring,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded {
                radii: [desc.thumb_size * 0.5; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Slider, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(state.disabled);
    a11y.value = format!("{}", desc.value);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

/// 已填充轨道绘制所用的颜色槽位。
fn fill_slot(state: ControlState, tokens: SliderTokens) -> Token {
    if state.disabled {
        return tokens.fill_disabled;
    }
    match state.phase {
        PointerPhase::Idle => tokens.fill,
        PointerPhase::Hovered => tokens.fill_hover,
        PointerPhase::Pressed => tokens.fill_pressed,
    }
}

/// 正在拖拽中的滑块状态。
#[must_use]
pub fn pressed_state(dragging: bool) -> ControlState {
    ControlState {
        phase: if dragging {
            PointerPhase::Pressed
        } else {
            PointerPhase::Idle
        },
        ..ControlState::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> SliderDesc {
        SliderDesc::new(
            Rect::new(0.0, 0.0, 200.0, 32.0),
            "Density",
            50.0,
            0.0,
            100.0,
        )
    }

    #[test]
    fn the_thumb_follows_the_value() {
        let mut desc = desc();
        desc.value = 0.0;
        assert_eq!(desc.thumb_rect().origin.x, -8.0);
        desc.value = 100.0;
        assert_eq!(desc.thumb_rect().origin.x, 192.0);
        desc.value = 50.0;
        assert_eq!(desc.thumb_rect().origin.x, 92.0);
    }

    #[test]
    fn the_pointer_maps_onto_the_value_range() {
        let desc = desc();
        assert_eq!(value_from_pointer(Vec2::new(0.0, 0.0), &desc), 0.0);
        assert_eq!(value_from_pointer(Vec2::new(100.0, 0.0), &desc), 50.0);
        assert_eq!(value_from_pointer(Vec2::new(200.0, 0.0), &desc), 100.0);
    }

    #[test]
    fn the_pointer_is_clamped_to_the_track() {
        let desc = desc();
        assert_eq!(value_from_pointer(Vec2::new(-500.0, 0.0), &desc), 0.0);
        assert_eq!(value_from_pointer(Vec2::new(500.0, 0.0), &desc), 100.0);
    }

    #[test]
    fn a_step_snaps_relative_to_the_minimum() {
        let desc = SliderDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Size", 0.0, 10.0, 20.0)
            .with_step(0.5);
        assert_eq!(value_from_pointer(Vec2::new(50.0, 0.0), &desc), 12.5);
        assert_eq!(value_from_pointer(Vec2::new(52.0, 0.0), &desc), 12.5);
        assert_eq!(value_from_pointer(Vec2::new(56.0, 0.0), &desc), 13.0);
    }

    #[test]
    fn a_stepped_slider_never_leaves_the_range() {
        let desc = SliderDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Size", 0.0, 10.0, 20.0)
            .with_step(3.0);
        let value = value_from_pointer(Vec2::new(200.0, 0.0), &desc);
        assert!(value <= 20.0, "{value} 超出了上界");
        assert!(value >= 10.0);
    }

    #[test]
    fn the_fill_grows_with_the_value() {
        let mut desc = desc();
        desc.value = 25.0;
        assert_eq!(desc.fill_rect().size.x, 50.0);
        desc.value = 75.0;
        assert_eq!(desc.fill_rect().size.x, 150.0);
    }

    #[test]
    fn the_semantic_node_carries_the_value() {
        let tokens = SliderTokens::standard();
        let output = emit_slider(1, &desc(), ControlState::default(), &[], tokens);
        let a11y = output.a11y.expect("a slider is always reachable");
        assert_eq!(a11y.role, Role::Slider);
        assert_eq!(a11y.value, "50");
    }

    #[test]
    fn a_disabled_slider_leaves_the_hit_set() {
        let tokens = SliderTokens::standard();
        let output = emit_slider(
            1,
            &desc(),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }

    #[test]
    fn a_focused_slider_emits_a_focus_ring() {
        let tokens = SliderTokens::standard();
        let output = emit_slider(
            1,
            &desc(),
            ControlState {
                focused: true,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert!(output
            .primitives
            .iter()
            .any(|primitive| primitive.prim_type() == unbound_gpu::data::PrimType::Stroke));
    }
}
