def patch(path, pairs):
    s = open(path, encoding='utf-8').read()
    for old, new in pairs:
        assert old in s, f"{path}: {old[:90]!r}"
        s = s.replace(old, new, 1)
    open(path, 'w', encoding='utf-8').write(s)
    print("patched", path)


patch('crates/unbound-gpu/src/frame.rs', [(
"""    /// `docs` §3.4: 10 000 bézier connectors ...""",
"""    /// `docs` §3.4: a curve is rasterised by the *analytic* distance to the
    /// quadratic Bézier, not by a polyline.
    ///
    /// The test draws a single arc and checks that the pixel on the curve is
    /// lit while a pixel well above it stays dark.
    #[test]
    fn a_curve_is_rasterised_analytically() {
        let _guard = crate::device::gpu_test_guard();
        let Some(gpu) = crate::device::test_device() else {
            return;
        };
        let mut renderer = Renderer::new(gpu, 128, 128).expect("renderer");
        // A quadratic arc from (10, 10) to (100, 10) bulging up to y = 45.
        let instance = unbound_vector::CurveInstance {
            origin: [10.0, 10.0],
            size: [90.0, 70.0],
            p0: [0.0, 0.0],
            p1: [40.0, 70.0],
            p2: [90.0, 0.0],
        };
        let draw = crate::curve::CurveDraw::new([0, 0], &instance, 2, 4.0, 100);
        let prims = vec![draw.primitive];
        let payloads = vec![draw.payload];
        let transforms = vec![GpuTransform::IDENTITY];
        renderer.write_theme(&{
            let mut colors = vec![[0u16; 4]; 512];
            colors[2] = unbound_math::Color::from_srgb8(255, 255, 255, 255).to_rgba16f();
            colors
        })
        .unwrap();

        let mut upload = scene(&prims, &transforms, 1);
        upload.payloads = &payloads;
        renderer.render(&upload).expect("render");
        let pixels = renderer.read_pixels().expect("readback");
        assert_eq!(pixels.len(), 128 * 128 * 4);

        // The curve passes through (55, 45) at t = 0.5.
        let on_curve = (45 * 128 + 55) * 4;
        // Ten pixels above the curve there is nothing.
        let off_curve = (30 * 128 + 55) * 4;
        assert!(
            pixels[on_curve] > 200,
            "the curve must be lit at its midpoint: {:?}",
            &pixels[on_curve..on_curve + 4]
        );
        assert!(
            pixels[off_curve] < 120,
            "nothing may be drawn above the curve: {:?}",
            &pixels[off_curve..off_curve + 4]
        );
        assert_eq!(renderer.stats.visible, 1);
        renderer.destroy();
    }

    /// `docs` §3.4: 10 000 bézier connectors ...""")])
print("done")
