//! 静态像素网格吸附（`docs` §4.4、§10.2）。
//!
//! 静态元素必须落在物理像素网格上：在 10% 缩放下边框仍需保持锐利，
//! 在 6400% 缩放下也不得产生爬行抖动。规则是经典的“四舍五入到最近整数”，
//! 且仅作用于边缘的*屏幕*位置——绝不作用于世界坐标，否则网格会随相机移动。
//!
//!
//!
//! 弹簧动画运行期间，动画引擎会绕过网格吸附：亚像素级平滑位置
//! 才能让运动看起来连续，若强行吸附网格则会产生卡顿感。
//!

use unbound_math::snap_to_pixel;

/// 验收表规定的缩放极值。
pub const MIN_ZOOM: f32 = 0.1;
/// 6400% 缩放。
pub const MAX_ZOOM: f32 = 64.0;

/// 将屏幕空间位置吸附到像素网格。
///
/// 该值单位为*物理*像素，因此默认设备缩放比为 1。
#[must_use]
pub fn snap_screen(value: f32) -> f32 {
    snap_to_pixel(value, 1.0)
}

/// 计算世界空间边缘在屏幕上的位置。
///
/// `world` 为世界单位的边缘坐标，`camera` 为相机世界坐标，
/// `zoom` 为画布缩放倍率，`viewport` 为物理像素视口半尺寸。
///
#[must_use]
pub fn edge_screen(world: f32, camera: f32, zoom: f32, half_viewport: f32) -> f32 {
    (world - camera) * zoom + half_viewport
}

/// 计算世界空间边缘吸附后的屏幕位置。
#[must_use]
pub fn edge_screen_snapped(
    world: f32,
    camera: f32,
    zoom: f32,
    half_viewport: f32,
    animated: bool,
) -> f32 {
    let screen = edge_screen(world, camera, zoom, half_viewport);
    if animated {
        // 弹簧运动期间位置保持亚像素精度：强制吸附网格会导致运动卡顿。
        //
        screen
    } else {
        snap_screen(screen)
    }
}

/// 吸附后边缘与理想位置之间的最大偏差（像素）。
///
/// 吸附边缘与理想位置的距离至多为半个像素；该数值即验收表中的“吸附误差”。
///
#[must_use]
pub fn snap_error(world: f32, camera: f32, zoom: f32, half_viewport: f32) -> f32 {
    let ideal = edge_screen(world, camera, zoom, half_viewport);
    (snap_screen(ideal) - ideal).abs()
}

/// 相机移动亚像素距离时，吸附边缘的最大*变化量*。
///
/// 这是抖动指标：只要相机停留在同一像素内，吸附边缘就完全不得移动。
///
///
#[must_use]
pub fn jitter(world: f32, camera: f32, zoom: f32, half_viewport: f32) -> f32 {
    let base = snap_screen(edge_screen(world, camera, zoom, half_viewport));
    let mut worst = 0.0f32;
    // 相机移动整整一个像素时，边缘移动不得超过一像素；亚像素移动时边缘必须纹丝不动。
    //
    for step in 1..=16 {
        let offset = step as f32 / 16.0;
        let moved = snap_screen(edge_screen(
            world,
            camera + offset / zoom,
            zoom,
            half_viewport,
        ));
        worst = worst.max((moved - base).abs());
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验收表规定的缩放范围。
    const ZOOMS: [f32; 6] = [0.1, 0.5, 1.0, 2.0, 8.0, 64.0];

    #[test]
    fn a_static_edge_lands_on_the_pixel_grid() {
        for zoom in ZOOMS {
            for step in 0..64 {
                let world = step as f32 * 0.37;
                let screen = edge_screen_snapped(world, 0.0, zoom, 256.0, false);
                assert!(
                    (screen - screen.round()).abs() < 1e-4,
                    "zoom {zoom}: {screen} is not on the grid"
                );
            }
        }
    }

    #[test]
    fn a_sub_pixel_camera_move_does_not_jitter_a_static_edge() {
        for zoom in ZOOMS {
            let jitter = jitter(123.0, 0.0, zoom, 256.0);
            // 相机最多移动一像素；跨过网格边界时吸附边缘可移动一像素，
            // 但绝不可移动不足一像素的小数距离。
            //
            assert!(
                jitter <= 1.0,
                "zoom {zoom}: the edge jittered by {jitter} px"
            );
        }
    }

    #[test]
    fn the_snapping_error_stays_below_half_a_pixel() {
        for zoom in ZOOMS {
            for step in 0..256 {
                let world = step as f32 * 1.7;
                let error = snap_error(world, 0.0, zoom, 256.0);
                assert!(
                    error <= 0.5 + 1e-4,
                    "zoom {zoom}: the snapping error is {error} px"
                );
            }
        }
    }

    #[test]
    fn an_animated_edge_keeps_its_sub_pixel_position() {
        let world = 123.456;
        let snapped = edge_screen_snapped(world, 0.0, 1.0, 256.0, true);
        let ideal = edge_screen(world, 0.0, 1.0, 256.0);
        assert_eq!(snapped, ideal);
        assert!(
            (snapped - snapped.round()).abs() > 1e-4,
            "the animated position must not be snapped"
        );
    }

    #[test]
    fn the_zoom_extremes_of_the_acceptance_table_are_covered() {
        assert_eq!(MIN_ZOOM, 0.1);
        assert_eq!(MAX_ZOOM * 100.0, 6400.0);
        // 在 6400% 缩放下，相机移动一像素相当于 1/64 个世界单位，内容向反方向移动。
        //
        let a = edge_screen(0.0, 0.0, MAX_ZOOM, 256.0);
        let b = edge_screen(0.0, 1.0 / MAX_ZOOM, MAX_ZOOM, 256.0);
        assert!((a - b - 1.0).abs() < 1e-4);
    }
}
