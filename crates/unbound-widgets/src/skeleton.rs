//! 骨架屏占位符。
//!
//! 骨架屏故意*不是*加载转圈：它镜像即将到达内容的形状，使数据加载完成时布局不会跳动。
//! 其动画仅为单条移动高光，纯属装饰性——控件不输出语义节点，
//! 因此辅助技术会播报外围的“加载中”提示，而非朗读一个矩形。
//!
//!

use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::surface;
use crate::WidgetOutput;

/// 扫光宽度占骨架宽度的比例。
pub const SKELETON_SHIMMER: f32 = 0.32;
/// 默认速度下的扫光周期（秒）。
pub const SKELETON_PERIOD: f32 = 1.6;

/// 单个骨架块描述符。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkeletonDesc {
    /// 骨架块矩形。
    pub rect: Rect,
    /// 圆角半径；默认使用较小的块级圆角。
    pub radius: f32,
    /// 是否启用扫光动画。
    pub animated: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl SkeletonDesc {
    /// 创建静态骨架块。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            radius: 6.0,
            animated: false,
            z: 400,
        }
    }

    /// 启用扫光动画。
    #[must_use]
    pub fn animated(mut self) -> Self {
        self.animated = true;
        self
    }

    /// 指定圆角半径。
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius.max(0.0);
        self
    }

    /// 计算在 `phase` 相位下往返于两端之间的扫光带矩形。
    #[must_use]
    pub fn shimmer_rect(&self, phase: f32) -> Rect {
        let width = (self.rect.size.x * SKELETON_SHIMMER)
            .min(self.rect.size.x)
            .max(0.0);
        let travel = (self.rect.size.x - width).max(0.0);
        let t = phase.rem_euclid(1.0);
        let t = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
        Rect::new(
            self.rect.origin.x + travel * t,
            self.rect.origin.y,
            width,
            self.rect.size.y,
        )
    }
}

/// 单个骨架块的动画时钟。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkeletonState {
    phase: f32,
}

impl SkeletonState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前扫光相位。
    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// 直接设置相位（用于恢复或捕获帧）。
    pub fn set_phase(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }

    /// 推进相位；返回是否发生了移动。
    pub fn tick(&mut self, dt: f32, animated: bool) -> bool {
        if !animated || dt <= 0.0 {
            return false;
        }
        self.phase = (self.phase + dt / SKELETON_PERIOD).rem_euclid(1.0);
        true
    }
}

/// 输出骨架块。
#[must_use]
pub fn emit_skeleton(desc: &SkeletonDesc, state: &SkeletonState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2);
    primitives.push(surface(
        desc.rect,
        desc.radius,
        Token::SURFACE_SUNKEN,
        desc.z,
    ));
    if desc.animated {
        primitives.push(surface(
            desc.shimmer_rect(state.phase()),
            desc.radius,
            Token::CONTROL_HOVER,
            desc.z + 1,
        ));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        // 装饰性控件：由外围实时区域（live region）负责播报。
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> SkeletonDesc {
        SkeletonDesc::new(Rect::new(10.0, 20.0, 200.0, 16.0))
    }

    #[test]
    fn a_static_block_paints_one_surface() {
        let output = emit_skeleton(&desc(), &SkeletonState::new());
        assert_eq!(output.primitives.len(), 1);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            u32::from(Token::SURFACE_SUNKEN.raw())
        );
        assert!(output.a11y.is_none());
        assert!(output.hit.is_none());
    }

    #[test]
    fn an_animated_block_adds_a_shimmer() {
        let output = emit_skeleton(&desc().animated(), &SkeletonState::new());
        assert_eq!(output.primitives.len(), 2);
        assert_eq!(
            output.primitives[1].fill_token_or_uv,
            u32::from(Token::CONTROL_HOVER.raw())
        );
    }

    #[test]
    fn the_shimmer_bounces_between_the_edges() {
        let desc = desc();
        let start = desc.shimmer_rect(0.0);
        let end = desc.shimmer_rect(0.5);
        let back = desc.shimmer_rect(1.0);
        assert_eq!(start.origin.x, desc.rect.origin.x);
        assert!((end.max().x - desc.rect.max().x).abs() < 0.001);
        assert_eq!(back.origin.x, start.origin.x);
        let middle = desc.shimmer_rect(0.25);
        assert!(middle.origin.x > start.origin.x);
        assert!(middle.origin.x < end.origin.x);
    }

    #[test]
    fn the_shimmer_never_escapes_the_block() {
        let desc = desc();
        for phase in [0.0, 0.1, 0.33, 0.5, 0.75, 0.99, 7.4] {
            let shimmer = desc.shimmer_rect(phase);
            assert!(desc.rect.inflate(0.01).contains_rect(&shimmer));
        }
    }

    #[test]
    fn only_an_animated_block_advances_its_clock() {
        let mut state = SkeletonState::new();
        assert!(!state.tick(0.5, false));
        assert_eq!(state.phase(), 0.0);
        assert!(state.tick(SKELETON_PERIOD * 0.5, true));
        assert!((state.phase() - 0.5).abs() < 0.001);
    }

    #[test]
    fn a_zero_radius_block_is_a_plain_rectangle() {
        let desc = desc().with_radius(-4.0);
        assert_eq!(desc.radius, 0.0);
    }
}
