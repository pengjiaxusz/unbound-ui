//! 彩色层合成的无头断言。
//!
//! 只断言外部可观察结果：像素值、包围盒、簇标识与簇分组。
//! 不依赖 GPU、不依赖 COM，可在无显示器环境运行。

use unbound_text::{
    bgra_premul_to_rgba, cluster_hash, cluster_ranges, composite, composite_layers, BitmapLayer,
    ColorLayer, CompositeLayer, CompositeOutcome, LayerColor, PositionedGlyph,
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

// ---------------------------------------------------------------------------
// 位图层（CBDT / PNG / JPEG）：与矢量层共用同一套 over 与包围盒。
//
// 夹具策略：这一组只喂**已解码的像素**，不依赖任何字体，因此可以在任何
// 机器上跑。真正的 DirectWrite 解码入口见 `color_bitmap_layer.rs`，那里
// 按「系统里有没有位图彩色字体」决定跑还是跳过。
// ---------------------------------------------------------------------------

/// 全不透明位图层：输出必须与输入逐字节相同（直通 alpha 的恒等路径）。
#[test]
fn opaque_bitmap_layer_passes_through_unchanged() {
    let rgba = [
        255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 9, 9, 9, 255,
    ];
    let outcome = composite_layers(&[CompositeLayer::Bitmap(BitmapLayer {
        rgba: &rgba,
        width: 2,
        height: 2,
        left: 0,
        top: 0,
    })]);
    let CompositeOutcome::Color(bitmap) = outcome else {
        panic!("an opaque bitmap layer must composite");
    };
    assert_eq!(bitmap.width, 2);
    assert_eq!(bitmap.height, 2);
    assert_eq!(bitmap.data, rgba.to_vec());
}

/// 半透明位图层压在不透明实色层上：按 source-over 手算。
#[test]
fn translucent_bitmap_over_solid_matches_manual_blend() {
    let solid_layer = CompositeLayer::Solid(ColorLayer {
        color: LayerColor::new(0.0, 0.0, 1.0, 1.0),
        coverage: &[255],
        width: 1,
        height: 1,
        left: 0,
        top: 0,
    });
    // 红色，alpha = 128（直通）。
    let rgba = [255u8, 0, 0, 128];
    let bitmap_layer = CompositeLayer::Bitmap(BitmapLayer {
        rgba: &rgba,
        width: 1,
        height: 1,
        left: 0,
        top: 0,
    });
    let CompositeOutcome::Color(bitmap) = composite_layers(&[solid_layer, bitmap_layer]) else {
        panic!("the stack must composite");
    };
    // sa = 128/255 ≈ 0.50196
    assert_eq!(bitmap.data[0], 128, "red is scaled by the source alpha");
    assert_eq!(bitmap.data[1], 0);
    assert_eq!(bitmap.data[2], 127, "blue is scaled by 1 - source alpha");
    assert_eq!(bitmap.data[3], 255, "the opaque base keeps alpha at full");
}

/// 预乘 BGRA 必须先展开成直通 RGBA，否则半透明边会发暗。
#[test]
fn premultiplied_bgra_is_expanded_to_straight_rgba() {
    // 预乘蓝，alpha = 128：分量已经是 1.0 × 0.50196。
    let bgra = [128u8, 0, 0, 128];
    let rgba = bgra_premul_to_rgba(&bgra, 1, 1);
    assert_eq!(rgba, vec![0, 0, 255, 128]);

    // 完全不透明时是纯换通道。
    let bgra = [10u8, 20, 30, 255];
    assert_eq!(bgra_premul_to_rgba(&bgra, 1, 1), vec![30, 20, 10, 255]);

    // 全透明像素保持全零，避免除零。
    assert_eq!(bgra_premul_to_rgba(&[0, 0, 0, 0], 1, 1), vec![0, 0, 0, 0]);

    // 长度不符时返回空向量，由调用方判为不可合成。
    assert!(bgra_premul_to_rgba(&[1, 2, 3, 4], 2, 2).is_empty());
}

/// 位图层与矢量层共享一个并集包围盒，各自落在自己的偏移上。
#[test]
fn bitmap_and_vector_layers_share_one_bounding_box() {
    let vector = CompositeLayer::Solid(ColorLayer {
        color: LayerColor::new(1.0, 0.0, 0.0, 1.0),
        coverage: &[255],
        width: 1,
        height: 1,
        left: 0,
        top: 0,
    });
    let rgba = [0u8, 0, 255, 255];
    let bitmap = CompositeLayer::Bitmap(BitmapLayer {
        rgba: &rgba,
        width: 1,
        height: 1,
        left: 3,
        top: 2,
    });
    let CompositeOutcome::Color(out) = composite_layers(&[vector, bitmap]) else {
        panic!("mixed layers must composite");
    };
    assert_eq!((out.width, out.height), (4, 3));
    assert_eq!((out.left, out.top), (0, 0));
    // 左下角来自矢量层，右上角来自位图层，中间两处透明。
    let at = |row: usize, col: usize| (row * 4 + col) * 4;
    assert_eq!(&out.data[at(0, 0)..at(0, 0) + 4], &[255, 0, 0, 255]);
    let idx = at(2, 3);
    assert_eq!(&out.data[idx..idx + 4], &[0, 0, 255, 255]);
    let idx = at(0, 3);
    assert_eq!(out.data[idx + 3], 0, "the gap must stay transparent");
}

/// 零尺寸与全透明位图层不参与包围盒，也不会把整簇判成 NotColor。
#[test]
fn invisible_bitmap_layers_are_ignored() {
    let empty = CompositeLayer::Bitmap(BitmapLayer {
        rgba: &[],
        width: 0,
        height: 0,
        left: 0,
        top: 0,
    });
    assert_eq!(composite_layers(&[empty]), CompositeOutcome::NotColor);

    let transparent = [0u8, 0, 0, 0];
    let solid = CompositeLayer::Solid(ColorLayer {
        color: LayerColor::new(0.0, 1.0, 0.0, 1.0),
        coverage: &[255],
        width: 1,
        height: 1,
        left: 5,
        top: 5,
    });
    let layer = CompositeLayer::Bitmap(BitmapLayer {
        rgba: &transparent,
        width: 1,
        height: 1,
        left: 0,
        top: 0,
    });
    let CompositeOutcome::Color(out) = composite_layers(&[layer, solid]) else {
        panic!("the solid layer must still composite");
    };
    assert_eq!((out.width, out.height), (1, 1));
    assert_eq!((out.left, out.top), (5, 5));
}
