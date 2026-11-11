//! The windowed component gallery.
//!
//! Run it with `cargo run -p showcase`. A window opens showing every
//! primitive type the engine can draw; the keyboard toggles the theme and the
//! mouse pans and zooms the canvas.
//!
//! This is the visible counterpart of `apps/gallery`: the gallery prints
//! measurements, the showcase shows them.

use showcase::gallery;

use std::sync::Arc;
use std::time::Instant;

use unbound_gpu::atlas::{AtlasKind, AtlasSet, AtlasUpload};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::surface::{Surface, Swapchain};
use unbound_gpu::swapchain::PresentConfig;
use unbound_gpu::Renderer;
use unbound_platform::backend::{Flow, PlatformApp, PlatformWindow};
use unbound_platform::window::{ButtonState, MouseButton, PlatformEvent, PlatformState, WindowConfig};

use gallery::{build, palette, theme_table, Gallery, Labels};

/// The window title.
const TITLE: &str = "Unbound UI — 组件展台";

/// How long to wait for a swapchain image, in nanoseconds.
const ACQUIRE_TIMEOUT_NS: u64 = 1_000_000_000;

/// The application state.
struct Showcase {
    /// The renderer, once the window exists.
    renderer: Option<Renderer>,
    /// The surface bound to the window.
    surface: Option<Surface>,
    /// The swapchain.
    swapchain: Option<Swapchain>,
    /// The bindless atlases.
    atlases: Option<Arc<AtlasSet>>,
    /// The label engine.
    labels: Labels,
    /// The current scene.
    scene: Gallery,
    /// Whether the dark theme is active.
    dark: bool,
    /// The canvas zoom.
    zoom: f32,
    /// The canvas pan, in logical pixels.
    pan: [f32; 2],
    /// Whether the left button is held.
    dragging: bool,
    /// The pointer position at the start of a drag.
    drag_origin: [f32; 2],
    /// The pan at the start of a drag.
    pan_origin: [f32; 2],
    /// The upload generation; bumping it re-stages the scene.
    generation: u64,
    /// The number of glyphs the atlas currently holds.
    glyphs: usize,
}

impl Showcase {
    /// Create the application.
    fn new() -> Self {
        Self {
            renderer: None,
            surface: None,
            swapchain: None,
            atlases: None,
            labels: Labels::new(),
            scene: Gallery::default(),
            dark: false,
            zoom: 1.0,
            pan: [0.0, 0.0],
            dragging: false,
            drag_origin: [0.0, 0.0],
            pan_origin: [0.0, 0.0],
            generation: 0,
            glyphs: 0,
        }
    }

    /// Rebuild the scene and push the new glyphs into the atlas.
    fn rebuild(&mut self, viewport: (f32, f32)) {
        let (scene, _) = build(&mut self.labels, &gallery::palette(), viewport);
        self.scene = scene;
        self.generation = self.generation.wrapping_add(1);
        self.upload_glyphs();
    }

    /// Upload the pending atlas updates.
    fn upload_glyphs(&mut self) {
        // The updates are only drained once there is somewhere to put them:
        // taking them before that would lose the glyphs.
        if self.atlases.is_none() || self.renderer.is_none() {
            return;
        }
        let updates = self.labels.cache.take_updates();
        if updates.is_empty() {
            return;
        }
        let Some(atlases) = &self.atlases else {
            return;
        };
        let Some(renderer) = &self.renderer else {
            return;
        };
        for update in updates {
            let kind = match update.target {
                unbound_text::AtlasTarget::Coverage => AtlasKind::R8,
                unbound_text::AtlasTarget::Color => AtlasKind::Rgba8,
            };
            let upload = AtlasUpload {
                kind,
                layer: update.layer,
                x: update.x,
                y: update.y,
                width: update.width,
                height: update.height,
                data: update.data,
            };
            if atlases.upload(&renderer.gpu, &upload).is_err() {
                break;
            }
        }
        self.glyphs = self.labels.cache.resident_glyphs();
    }

    /// Switch the theme and re-upload the 4 KiB colour table.
    fn toggle_theme(&mut self) {
        self.dark = !self.dark;
        if let Some(renderer) = &mut self.renderer {
            let table = theme_table(self.dark);
            let _ = renderer.write_theme(&table);
        }
    }
}

impl PlatformApp for Showcase {
    fn resumed(&mut self, state: &mut PlatformState, window: &winit::window::Window) {
        // The GPU needs the surface extensions, so the instance is created
        // with them enabled.
        let config = GpuConfig {
            instance_extensions: vec![
                "VK_KHR_surface".to_string(),
                "VK_KHR_win32_surface".to_string(),
            ],
            device_extensions: vec![
                "VK_KHR_swapchain".to_string(),
                "VK_EXT_swapchain_maintenance1".to_string(),
                // The low-latency present path: the CPU can wait for the
                // image to really be on the screen instead of for the render.
                "VK_KHR_present_wait".to_string(),
            ],
            ..Default::default()
        };
        let Ok(gpu) = Gpu::new(config) else {
            eprintln!("could not create a Vulkan device");
            return;
        };

        // The surface needs the native window handle.
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match window.window_handle().ok().map(|h| h.as_raw()) {
                Some(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
                _ => {
                    eprintln!("the window has no native handle");
                    return;
                }
            }
        };
        let Ok(surface) = Surface::new_win32(&gpu, hwnd) else {
            eprintln!("could not create a Vulkan surface");
            return;
        };

        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        // Tearing off: a torn frame is worse than a frame that arrives one
        // refresh cycle later.
        let present = PresentConfig::from_capabilities(&gpu.enabled_device_extensions, false);
        let Ok(swapchain) = Swapchain::new(&gpu, &surface, width, height, &present) else {
            eprintln!("could not create a swapchain");
            return;
        };

        // The device extensions are needed after the GPU has been moved into
        // the renderer, so they are copied out first.
        let device_extensions = gpu.enabled_device_extensions.clone();

        let mut renderer = match Renderer::new(gpu, width, height) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("could not create the renderer: {error}");
                return;
            }
        };

        // Install the real atlases so glyphs are not sampled from a dummy
        // 1x1 texture.
        let atlases = match AtlasSet::new(&renderer.gpu, 64) {
            Ok(atlases) => Arc::new(atlases),
            Err(error) => {
                eprintln!("could not create the atlases: {error}");
                return;
            }
        };
        if let Err(error) = renderer.install_atlases(atlases.clone()) {
            eprintln!("could not bind the atlases: {error}");
            return;
        }

        self.atlases = Some(atlases);
        self.swapchain = Some(swapchain);
        self.surface = Some(surface);
        self.renderer = Some(renderer);

        // The platform layer could not know the device extensions when the
        // window was created; now it can, and the presentation policy
        // follows the device instead of a hard-coded fallback.
        state.set_present_extensions(&device_extensions);

        // Build the first frame.
        let logical = (width as f32, height as f32);
        self.rebuild(logical);
        if let Some(renderer) = &mut self.renderer {
            let _ = renderer.write_theme(&theme_table(self.dark));
        }
        let _ = state;
    }

    fn event(
        &mut self,
        _state: &mut PlatformState,
        _window: &winit::window::Window,
        event: PlatformEvent,
    ) -> Flow {
        match event {
            PlatformEvent::KeyInput { state, key, .. } => {
                if state == ButtonState::Pressed {
                    match key {
                        unbound_platform::window::PlatformKey::Character(text)
                            if text.eq_ignore_ascii_case("t") =>
                        {
                            self.toggle_theme();
                        }
                        unbound_platform::window::PlatformKey::Named(name)
                            if name.eq_ignore_ascii_case("space") =>
                        {
                            self.toggle_theme();
                        }
                        _ => {}
                    }
                }
            }
            PlatformEvent::MouseInput { state, button } => {
                if button == MouseButton::Left {
                    self.dragging = state == ButtonState::Pressed;
                }
            }
            PlatformEvent::CursorMoved { x, y } => {
                if self.dragging {
                    let dx = x as f32 - self.drag_origin[0];
                    let dy = y as f32 - self.drag_origin[1];
                    self.pan = [self.pan_origin[0] + dx, self.pan_origin[1] + dy];
                } else {
                    self.drag_origin = [x as f32, y as f32];
                }
            }
            PlatformEvent::MouseWheel { delta_y, .. } => {
                // One notch up zooms in. The zoom is clamped so the gallery
                // can neither disappear nor turn into a single pixel.
                self.zoom = (self.zoom as f64 * 1.1f64.powf(delta_y)) as f32;
                self.zoom = self.zoom.clamp(0.25, 8.0);
            }
            PlatformEvent::CloseRequested => return Flow::Exit,
            _ => {}
        }
        Flow::Continue
    }

    fn frame(&mut self, state: &mut PlatformState, window: &winit::window::Window) -> Flow {
        let size = window.inner_size();
        let (window_width, window_height) = (size.width.max(1), size.height.max(1));

        // The swapchain follows the window only once the size has settled.
        //
        // Rebuilding it on every resize event is what produces black borders
        // and stretched frames: during a drag the compositor keeps showing
        // the last frame scaled to the new window size, and the swapchain is
        // rebuilt once, after the dust settles.
        let settled = state.poll_resize(Instant::now());
        if settled.is_some() {
            let (width, height) = (window_width, window_height);
            let Some(renderer) = &mut self.renderer else {
                return Flow::Continue;
            };
            let Some(surface) = &self.surface else {
                return Flow::Continue;
            };
            let present =
                PresentConfig::from_capabilities(&renderer.gpu.enabled_device_extensions, false);
            if let Ok(chain) = Swapchain::new(&renderer.gpu, surface, width, height, &present) {
                if let Some(old) = self.swapchain.replace(chain) {
                    old.destroy(&renderer.gpu);
                }
                if let Err(error) = renderer.resize(width, height) {
                    eprintln!("could not resize the renderer: {error}");
                }
                // The scene is laid out for the new extent.
                self.rebuild((width as f32, height as f32));
            }
        }

        let Some(renderer) = &mut self.renderer else {
            return Flow::Continue;
        };
        let Some(swapchain) = &mut self.swapchain else {
            return Flow::Continue;
        };

        // Draw at the *committed* extent, not at the window's current one:
        // while a resize is pending the swapchain still has the old size and
        // the compositor scales the frame to the window.
        let (width, height) = swapchain.extent();


        // Nothing to draw until the next frame is due.
        if !state.pacer.should_render(Instant::now()) {
            return Flow::Continue;
        }
        // The previous frame has to have retired before the next image is
        // acquired: the acquire signals the image-available semaphore, and a
        // binary semaphore may only carry one pending signal.
        if let Err(error) = renderer.wait_for_previous_frame() {
            eprintln!("waiting for the previous frame failed: {error}");
            return Flow::Continue;
        }


        // Acquire an image and draw into it.
        let acquired = swapchain.acquire(ACQUIRE_TIMEOUT_NS);

        if let Err(ref e) = acquired {
            eprintln!("DIAG acquire failed: {e:?}");
        }
        let Ok(image) = acquired else {
            // A lost surface or an out-of-date swapchain is recoverable: the
            // next frame rebuilds it.
            return Flow::Continue;
        };

        // The camera follows the pan and the zoom.
        //
        // `anchor` is the world point that lands in the middle of the
        // viewport: the shader maps it to the centre and scales everything
        // around it by the zoom, so panning by one screen pixel moves the
        // content by exactly one screen pixel at any zoom level.
        let zoom = self.zoom.clamp(0.25, 8.0);
        let anchor = [
            width as f32 * 0.5 - self.pan[0] / zoom,
            height as f32 * 0.5 - self.pan[1] / zoom,
        ];
        self.scene.camera = unbound_gpu::Camera::canvas([0, 0], anchor, zoom, 1.0);
        let scene = self.scene.upload(self.generation);
        if let Err(error) = renderer.render_to_view(
            &scene,
            image.image,
            image.view,
            width,
            height,
            image.available,
        ) {
            eprintln!("render failed: {error}");
            return Flow::Continue;
        }


        let finished = renderer.present_semaphore();
        if let Err(error) = swapchain.present(&renderer.gpu, image.index, finished) {
            eprintln!("present failed: {error}");
        }
        // Tell the pacer when the frame was shown, and ask the event loop for
        // another one.
        state.pacer.on_present(Instant::now());
        window.request_redraw();
        state.latency.on_present(Instant::now());
        Flow::Continue
    }
}

/// Render the gallery offscreen and write it to a PNG.
///
/// This is the headless twin of the window: same scene, same renderer, but
/// the picture lands in a file instead of on the screen.
fn render_to_png(path: &str, width: u32, height: u32, dark: bool) -> Result<(), String> {
    let gpu = Gpu::new(GpuConfig::default()).map_err(|e| e.to_string())?;
    let mut renderer = Renderer::new(gpu, width, height).map_err(|e| e.to_string())?;

    let atlases = Arc::new(AtlasSet::new(&renderer.gpu, 64).map_err(|e| e.to_string())?);
    renderer
        .install_atlases(atlases.clone())
        .map_err(|e| e.to_string())?;

    let mut labels = Labels::new();
    let (scene, _) = build(&mut labels, &palette(), (width as f32, height as f32));

    for update in labels.cache.take_updates() {
        let kind = match update.target {
            unbound_text::AtlasTarget::Coverage => AtlasKind::R8,
            unbound_text::AtlasTarget::Color => AtlasKind::Rgba8,
        };
        atlases
            .upload(
                &renderer.gpu,
                &AtlasUpload {
                    kind,
                    layer: update.layer,
                    x: update.x,
                    y: update.y,
                    width: update.width,
                    height: update.height,
                    data: update.data,
                },
            )
            .map_err(|e| e.to_string())?;
    }

    // Report the first glyph's UV so it can be compared with the atlas.
    renderer
        .write_theme(&theme_table(dark))
        .map_err(|e| e.to_string())?;
    renderer.render(&scene.upload(1)).map_err(|e| e.to_string())?;
    let pixels = renderer.read_pixels().map_err(|e| e.to_string())?;
    renderer.destroy();

    write_png(path, width, height, &pixels)
}

/// Write RGBA pixels to a PNG file.
fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    // The readback is BGRA; the PNG wants RGBA. Swapping the first and third
    // channels turns B,G,R,A into R,G,B,A.
    let mut rgba = rgba.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    writer
        .write_image_data(&rgba)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    // The headless twin: render the gallery to a PNG and exit.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--shot") {
        let path = args
            .windows(2)
            .find(|w| w[0] == "--shot")
            .map(|w| w[1].clone())
            .unwrap_or_else(|| "showcase.png".to_string());
        let dark = args.iter().any(|a| a == "--dark");
        match render_to_png(&path, 1280, 720, dark) {
            Ok(()) => println!("wrote {path}"),
            Err(error) => eprintln!("could not render: {error}"),
        }
        return;
    }

    let config = WindowConfig {
        title: TITLE.to_string(),
        width: 1280,
        height: 720,
        resizable: true,
        visible: true,
        ..Default::default()
    };
    let Ok(window) = PlatformWindow::new(&config) else {
        eprintln!("could not create the window");
        return;
    };
    if let Err(error) = window.run(Showcase::new()) {
        eprintln!("the event loop failed: {error}");
    }
}
