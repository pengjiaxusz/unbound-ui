//! The async image pipeline, end to end minus the GPU.
//!
//! A PNG is submitted, decoded on a worker thread, packed into the atlas and
//! resolved by the widget layer. The first paint is a placeholder with the
//! final box; the second is a textured quad sampling exactly the packed rect.

use std::time::Duration;

use unbound_app::{AssetHub, AssetResidency};
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType};
use unbound_math::{Rect, Vec2};
use unbound_text::TextEngine;
use unbound_widgets::{emit_image_with_assets, ControlState, ImageDesc, ObjectFit};

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![200u8; (width * height * 4) as usize])
            .unwrap();
    }
    bytes
}

fn wait_until_ready(hub: &mut AssetHub, text: &mut TextEngine, id: unbound_asset::AssetId) {
    for _ in 0..400 {
        hub.poll(text);
        if hub.residency(id) != Some(AssetResidency::Loading) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the asset never finished decoding");
}

#[test]
fn async_png_asset_transitions_from_placeholder_to_textured_quad() {
    let mut hub = AssetHub::new();
    let mut text = TextEngine::new();

    let id = hub.request_png(png_bytes(8, 8));
    let mut desc =
        ImageDesc::new(Rect::new(20.0, 20.0, 160.0, 80.0), Vec2::new(8.0, 8.0), 0).with_asset(id);
    desc.fit = ObjectFit::Fill;

    // Frame 1: decoding has not started, so the widget paints the placeholder.
    let placeholder = emit_image_with_assets(7, &desc, ControlState::default(), &hub);
    assert_eq!(placeholder.primitives.len(), 1);
    assert_eq!(placeholder.primitives[0].prim_type(), PrimType::RoundedRect);
    assert_eq!(placeholder.primitives[0].size, [160.0, 80.0]);

    wait_until_ready(&mut hub, &mut text, id);
    assert!(
        matches!(hub.residency(id), Some(AssetResidency::Ready { .. })),
        "decode failed: {:?}",
        hub.failures()
    );

    // Frame 2: the same widget now emits a textured quad over the atlas rect.
    let textured = emit_image_with_assets(7, &desc, ControlState::default(), &hub);
    let quad: &GpuPrimitiveInstance = &textured.primitives[0];
    assert_eq!(quad.prim_type(), PrimType::Image);
    assert_eq!(quad.local_pos, [20.0, 20.0]);
    assert_eq!(quad.size, [160.0, 80.0]);
    assert_eq!(textured.payloads.len(), 1);
    assert_eq!(textured.payloads[0].p0, [0.0, 0.0]);
    assert_eq!(textured.payloads[0].p1, [8.0 / 2048.0, 8.0 / 2048.0]);

    let uploads = hub.take_uploads();
    assert_eq!(uploads.len(), 1);
    assert_eq!((uploads[0].width, uploads[0].height), (8, 8));
    assert_eq!(uploads[0].data.len(), 8 * 8 * 4);
    assert!(hub.take_uploads().is_empty(), "uploads are drained");
}
