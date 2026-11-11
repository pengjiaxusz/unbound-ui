//! The gallery must actually draw something.
//!
//! The showcase is a windowed program, so this test renders the same scene
//! offscreen: a gallery that draws nothing is not a gallery.

use showcase::gallery::{build, palette, theme_table, Labels};

#[test]
fn the_gallery_draws_something() {
    let Some(gpu) = unbound_gpu::device::test_device() else {
        // No Vulkan device: the test cannot run, but it must not fail.
        return;
    };
    let mut renderer = unbound_gpu::Renderer::new(gpu, 640, 400).expect("renderer");

    // The real atlases, so the labels are not sampled from a dummy 1x1
    // texture.
    let atlases = std::sync::Arc::new(
        unbound_gpu::atlas::AtlasSet::new(&renderer.gpu, 4).expect("atlas set"),
    );
    renderer.install_atlases(atlases.clone()).expect("install");

    let mut labels = Labels::new();
    let (scene, _) = build(&mut labels, &palette(), (640.0, 400.0));
    assert!(
        scene.primitives.len() > 10,
        "the gallery must contain many primitives, found {}",
        scene.primitives.len()
    );

    // Upload the glyphs the labels produced.
    for update in labels.cache.take_updates() {
        let kind = match update.target {
            unbound_text::AtlasTarget::Coverage => unbound_gpu::atlas::AtlasKind::R8,
            unbound_text::AtlasTarget::Color => unbound_gpu::atlas::AtlasKind::Rgba8,
        };
        atlases
            .upload(
                &renderer.gpu,
                &unbound_gpu::atlas::AtlasUpload {
                    kind,
                    layer: update.layer,
                    x: update.x,
                    y: update.y,
                    width: update.width,
                    height: update.height,
                    data: update.data,
                },
            )
            .expect("upload");
    }

    renderer.write_theme(&theme_table(false)).expect("theme");
    renderer.render(&scene.upload(1)).expect("render");
    let pixels = renderer.read_pixels().expect("readback");

    // The picture must not be uniformly the background: the swatches, the
    // card and the curves have to be there.
    let mut distinct = 0usize;
    let mut previous = &pixels[0..4];
    for pixel in pixels.chunks_exact(4) {
        if pixel != previous {
            distinct += 1;
        }
        previous = pixel;
    }
    assert!(
        distinct > 500,
        "the gallery must draw more than a flat background, found {distinct} colour changes"
    );
    renderer.destroy();
}
