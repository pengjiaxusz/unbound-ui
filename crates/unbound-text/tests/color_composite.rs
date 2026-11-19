//! 彩色层合成的无头断言。
//!
//! 只断言外部可观察结果：像素值、包围盒、簇标识与簇分组。
//! 不依赖 GPU、不依赖 COM，可在无显示器环境运行。

use unbound_text::{
    cluster_hash, cluster_ranges, composite, ColorLayer, CompositeOutcome, LayerColor,
    PositionedGlyph,
};

fn solid(w: u32, h: u32) -> Vec<u8> {
    vec![255u8; (w * h) as usize]
}

fn glyph(cluster: u32) -> PositionedGlyph {
    PositionedGlyph {
        glyph_id: 1,
        face_index: 0,
        x: 0.0,
        y: 0.0,
        w: 10.0,
        font_size: 16.0,
        cluster,
        level: 0,
        _pad: [0; 3],
    }
}

/// 读取位图中的一个像素（行优先，直通 alpha）。
fn px(bitmap: &unbound_text::CompositeBitmap, row: usize, col: usize) -> [u8; 4] {
    let i = (row * bitmap.width as usize + col) * 4;
    let mut out = [0u8; 4];
    out.copy_from_slice(&bitmap.data[i..i + 4]);
    out
}

#[test]
fn opaque_layer_over_opaque_layer_keeps_top_color() {
    let top_cov = solid(1, 1);
    let bottom_cov = solid(1, 1);
    let layers = [
        ColorLayer {
            color: LayerColor {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            coverage: &bottom_cov,
            width: 1,
            height: 1,
            left: 0,
            top: 0,
        },
        ColorLayer {
            color: LayerColor {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            },
            coverage: &top_cov,
            width: 1,
            height: 1,
            left: 0,
            top: 0,
        },
    ];
    let CompositeOutcome::Color(bitmap) = composite(&layers) else {
        panic!("expected a color composite");
    };
    assert_eq!((bitmap.width, bitmap.height), (1, 1));
    assert_eq!(bitmap.data, vec![0, 0, 255, 255]);
}

#[test]
fn half_transparent_layer_over_opaque_layer_is_straight_alpha() {
    let cov = solid(1, 1);
    let layers = [
        ColorLayer {
            color: LayerColor {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            coverage: &cov,
            width: 1,
            height: 1,
            left: 0,
            top: 0,
        },
        ColorLayer {
            color: LayerColor {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 0.5,
            },
            coverage: &cov,
            width: 1,
            height: 1,
            left: 0,
            top: 0,
        },
    ];
    let CompositeOutcome::Color(bitmap) = composite(&layers) else {
        panic!("expected a color composite");
    };
    // 预乘 over：src=(0,0,0.5,0.5)，dst=(1,0,0,1) → pm=(0.5,0,0.5)，a=1
    // 除回 alpha 后仍是 (0.5,0,0.5)，即 128,0,128。
    assert_eq!(bitmap.data, vec![128, 0, 128, 255]);
}

#[test]
fn coverage_scales_alpha_but_not_hue() {
    let cov = vec![128u8];
    let layers = [ColorLayer {
        color: LayerColor {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        coverage: &cov,
        width: 1,
        height: 1,
        left: 0,
        top: 0,
    }];
    let CompositeOutcome::Color(bitmap) = composite(&layers) else {
        panic!("expected a color composite");
    };
    assert_eq!(bitmap.data, vec![255, 0, 0, 128]);
}

#[test]
fn fully_transparent_layers_report_not_color() {
    let cov = solid(2, 2);
    let zero = ColorLayer {
        color: LayerColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        },
        coverage: &cov,
        width: 2,
        height: 2,
        left: 0,
        top: 0,
    };
    assert!(matches!(composite(&[zero]), CompositeOutcome::NotColor));
}

#[test]
fn bounding_box_is_the_union_of_layer_rects() {
    let a = solid(2, 2);
    let b = solid(1, 1);
    let layers = [
        ColorLayer {
            color: LayerColor {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            coverage: &a,
            width: 2,
            height: 2,
            left: 0,
            top: 0,
        },
        ColorLayer {
            color: LayerColor {
                r: 0.0,
                g: 1.0,
                b: 0.0,
                a: 1.0,
            },
            coverage: &b,
            width: 1,
            height: 1,
            left: 2,
            top: 1,
        },
    ];
    let CompositeOutcome::Color(bitmap) = composite(&layers) else {
        panic!("expected a color composite");
    };
    assert_eq!((bitmap.width, bitmap.height), (3, 2));
    assert_eq!((bitmap.left, bitmap.top), (0, 0));
    // 右下角像素只由第二层覆盖。
    assert_eq!(px(&bitmap, 1, 2), [0, 255, 0, 255]);
    // 左下角只被第一层覆盖，保持不透明红。
    assert_eq!(px(&bitmap, 1, 0), [255, 0, 0, 255]);
    // 右上角落在并集包围盒里，但没有任何层覆盖它——必须保持全透明。
    assert_eq!(px(&bitmap, 0, 2)[3], 0);
}

#[test]
fn negative_offsets_shift_the_bounding_box_origin() {
    let cov = solid(1, 1);
    let layers = [ColorLayer {
        color: LayerColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        coverage: &cov,
        width: 1,
        height: 1,
        left: -3,
        top: -4,
    }];
    let CompositeOutcome::Color(bitmap) = composite(&layers) else {
        panic!("expected a color composite");
    };
    assert_eq!((bitmap.left, bitmap.top), (-3, -4));
    assert_eq!((bitmap.width, bitmap.height), (1, 1));
}

#[test]
fn cluster_hash_is_zero_for_single_glyph_and_stable_otherwise() {
    assert_eq!(cluster_hash(&[42]), 0);
    assert_eq!(cluster_hash(&[]), 0);

    let a = cluster_hash(&[1284, 389, 1464]);
    let b = cluster_hash(&[1284, 389, 1464]);
    let c = cluster_hash(&[1284, 389, 1272]);
    assert_eq!(a, b, "the same cluster must hash to the same id");
    assert_ne!(a, c, "clusters sharing a prefix must not collide");
    assert_ne!(
        a, 0,
        "a multi-glyph cluster must not hash to the single-glyph path"
    );
}

#[test]
fn cluster_ranges_group_consecutive_equal_clusters() {
    let glyphs = [
        glyph(0),
        glyph(0),
        glyph(4),
        glyph(10),
        glyph(10),
        glyph(10),
    ];
    let ranges = cluster_ranges(&glyphs);
    let spans: Vec<(usize, usize)> = ranges.iter().map(|r| (r.start, r.end)).collect();
    assert_eq!(spans, vec![(0, 2), (2, 3), (3, 6)]);
}

#[test]
fn cluster_ranges_of_empty_slice_is_empty() {
    assert!(cluster_ranges(&[]).is_empty());
}
