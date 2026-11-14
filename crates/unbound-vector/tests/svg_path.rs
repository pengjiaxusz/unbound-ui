//! SVG Path 解析器与二次降阶的集成测试（S5.6）。

use unbound_math::Vec2;
use unbound_vector::{parse_svg_path, SvgPathCache};

#[test]
fn parses_all_svg_path_commands_into_quadratic_segments() {
    let d = "M 2 2 L 22 2 H 24 V 10 C 24 16 18 22 12 22 S 2 16 2 10 Q 2 6 6 4 T 14 4 A 4 4 0 0 1 20 8 Z";
    let parsed = parse_svg_path(d, 0.25).expect("valid SVG path");
    assert_eq!(parsed.subpaths.len(), 1);
    assert!(parsed.subpaths[0].closed);
    assert!(
        parsed.segments.len() >= 10,
        "expected >= 10 quadratic segments, got {}",
        parsed.segments.len()
    );
    assert!(parsed.bounds.1.x >= 22.0 && parsed.bounds.1.y >= 20.0);

    let scaled = parsed.scaled_into(Vec2::new(10.0, 20.0), Vec2::new(48.0, 48.0), 24.0);
    assert_eq!(scaled.len(), parsed.segments.len());
}

#[test]
fn svg_path_cache_reuses_parsed_paths() {
    let mut cache = SvgPathCache::default();
    let p1 = cache
        .get_or_parse("M 4 12 L 10 18 L 20 6", 0.25)
        .unwrap()
        .clone();
    let p2 = cache.get_or_parse("M 4 12 L 10 18 L 20 6", 0.25).unwrap();
    assert_eq!(&p1, p2);
}
