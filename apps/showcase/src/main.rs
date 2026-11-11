//! The windowed component gallery.
//!
//! Run it with `cargo run -p showcase`. A window opens showing every
//! primitive type the engine can draw; the keyboard toggles the theme, the
//! mouse pans and zooms the canvas, and the button in the "Button" card is the
//! first fully interactive widget: hover, press, click, focus and keyboard
//! activation all go through `unbound-input`.
//!
//! This is the visible counterpart of `apps/gallery`: the gallery prints
//! measurements, the showcase shows them.

use showcase::button::{self, ButtonDesc, ButtonVisualState};
use showcase::gallery;

use std::sync::Arc;
use std::time::Instant;

use unbound_a11y::Semantics;
use unbound_gpu::atlas::{AtlasKind, AtlasSet, AtlasUpload};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::surface::{Surface, Swapchain};
use unbound_gpu::swapchain::PresentConfig;
use unbound_gpu::Renderer;
use unbound_input::{CursorKind, PointerConfig, PointerEvent, Router};
use unbound_math::Vec2;
use unbound_platform::backend::{Flow, PlatformApp, PlatformWindow};
use unbound_platform::window::{
    ButtonState, MouseButton, PlatformEvent, PlatformKey, PlatformState, WindowConfig,
};
use unbound_world::{NodeId, UiWorld};
use winit::window::Window;

use gallery::{build, build_interactive, palette, theme_table, Gallery, Labels};

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
    /// The button's stable identity, allocated by `unbound-world`.
    button: NodeId,
    /// The pointer router: hover, capture and click synthesis.
    router: Router,
    /// The focused node, if any.
    focused: Option<NodeId>,
    /// The visual state that is currently baked into the scene.
    visual: ButtonVisualState,
    /// How many times the button was activated.
    click_count: u32,
    /// The click count baked into the current scene.
    rendered_clicks: u32,
    /// The semantic tree built with the current scene.
    semantics: Option<Semantics>,
    /// The last screen-space pointer position.
    pointer: Option<[f32; 2]>,
    /// Whether the left button currently pans the canvas.
    canvas_dragging: bool,
    /// The cursor currently applied to the window.
    cursor: CursorKind,
    /// The time origin for pointer timing.
    clock: Instant,
    /// Whether the dark theme is active.
    dark: bool,
    /// The canvas zoom.
    zoom: f32,
    /// The canvas pan, in logical pixels.
    pan: [f32; 2],
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
        // The world is the identity allocator; the `NodeId` it hands out is
        // what the router, the hit region and the semantic tree share. The
        // demo keeps no mutable widget state of its own beyond the click
        // count — hover and press are derived from the router.
        let mut world = UiWorld::new();
        let button = button::insert_button(&mut world, gallery::button_rect());
        Self {
            renderer: None,
            surface: None,
            swapchain: None,
            atlases: None,
            labels: Labels::new(),
            scene: Gallery::default(),
            button,
            router: Router::new(PointerConfig::default()),
            focused: None,
            visual: ButtonVisualState::default(),
            click_count: 0,
            rendered_clicks: 0,
            semantics: None,
            pointer: None,
            canvas_dragging: false,
            cursor: CursorKind::Default,
            clock: Instant::now(),
            dark: false,
            zoom: 1.0,
            pan: [0.0, 0.0],
            drag_origin: [0.0, 0.0],
            pan_origin: [0.0, 0.0],
            generation: 0,
            glyphs: 0,
        }
    }

    /// Rebuild the scene and push the new glyphs into the atlas.
    ///
    /// Only called on a discrete state transition (or a resize): panning,
    /// zooming and theme switches never get here.
    fn rebuild(&mut self, viewport: (f32, f32)) {
        let desc = ButtonDesc::new(
            gallery::button_rect(),
            format!("Clicked: {}", self.click_count),
        );
        let (scene, _, regions, mut semantics) = build_interactive(
            &mut self.labels,
            &palette(),
            viewport,
            &desc,
            button::node_u64(self.button),
            self.visual,
            self.click_count,
        );
        self.scene = scene;
        self.router.set_regions(regions);
        if let Some(focused) = self.focused {
            semantics.set_focus(button::node_u64(focused));
        }
        self.semantics = Some(semantics);
        self.rendered_clicks = self.click_count;
        self.generation = self.generation.wrapping_add(1);
        self.upload_glyphs();
    }

    /// Move keyboard focus and mirror it into the semantic tree.
    fn set_focus(&mut self, node: Option<NodeId>) {
        self.focused = node;
        if let Some(semantics) = &mut self.semantics {
            let target = node.map_or(semantics.root(), button::node_u64);
            semantics.set_focus(target);
        }
    }

    /// Activate the button: the only business state the demo owns.
    fn activate(&mut self) {
        self.click_count = self.click_count.wrapping_add(1);
    }

    /// Map a window position (physical pixels) to canvas coordinates.
    ///
    /// The shader maps `screen = (world - anchor) * zoom + viewport / 2`, so
    /// the inverse is `world = (screen - viewport / 2) / zoom + anchor`. Doing
    /// this before feeding the router is what keeps the button's hit region
    /// glued to its pixels after any pan or zoom.
    fn canvas_position(&self, x: f32, y: f32, window: &Window) -> Vec2 {
        let size = window.inner_size();
        canvas_at(
            [x, y],
            (size.width.max(1) as f32, size.height.max(1) as f32),
            self.zoom,
            self.pan,
        )
    }

    /// Route keyboard input.
    ///
    /// `T` always switches the theme. `Space` activates the focused button and
    /// otherwise keeps its old "switch theme" meaning; `Enter` activates the
    /// focused button; `Tab` focuses it and `Escape` clears focus.
    fn on_key(&mut self, key: PlatformKey) {
        match key {
            PlatformKey::Character(text) if text.eq_ignore_ascii_case("t") => self.toggle_theme(),
            PlatformKey::Named(name) if name.eq_ignore_ascii_case("tab") => {
                self.set_focus(Some(self.button));
            }
            PlatformKey::Named(name) if name.eq_ignore_ascii_case("escape") => self.set_focus(None),
            PlatformKey::Named(name) if name.eq_ignore_ascii_case("enter") => {
                if self.focused.is_some() {
                    self.activate();
                }
            }
            PlatformKey::Named(name) if name.eq_ignore_ascii_case("space") => {
                if self.focused.is_some() {
                    self.activate();
                } else {
                    self.toggle_theme();
                }
            }
            _ => {}
        }
    }

    /// Pointer arbitration: a hit on the button beats panning the canvas.
    fn on_left_button(&mut self, pressed: bool, window: &Window) {
        let Some([x, y]) = self.pointer else {
            return;
        };
        let position = self.canvas_position(x, y, window);
        let now = self.clock.elapsed().as_secs_f64();
        let button = button::node_u64(self.button);
        if pressed {
            self.router.pointer_down(position, now);
            if self.router.pointer().pressed() == Some(button) {
                // The router captured the pointer: this is a click, not a pan.
                self.set_focus(Some(self.button));
                self.canvas_dragging = false;
            } else {
                self.canvas_dragging = true;
                self.drag_origin = [x, y];
                self.pan_origin = self.pan;
            }
        } else {
            for event in self.router.pointer_up(position, now) {
                if matches!(event, PointerEvent::Click { target, .. } if target == button) {
                    self.activate();
                }
            }
            self.canvas_dragging = false;
        }
    }

    /// Route pointer movement.
    fn on_cursor_moved(&mut self, x: f32, y: f32, window: &Window) {
        self.pointer = Some([x, y]);
        if self.canvas_dragging {
            let dx = x - self.drag_origin[0];
            let dy = y - self.drag_origin[1];
            self.pan = [self.pan_origin[0] + dx, self.pan_origin[1] + dy];
        } else {
            // Only the inverse-transformed point may enter the router.
            let position = self.canvas_position(x, y, window);
            self.router.pointer_move(position);
        }
    }

    /// Push the resolved cursor to the window, but only when it changes.
    fn apply_cursor(&mut self, window: &Window) {
        let kind = if self.canvas_dragging {
            CursorKind::Grabbing
        } else {
            self.router.cursor()
        };
        if kind != self.cursor {
            self.cursor = kind;
            window.set_cursor(cursor_icon(kind));
        }
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

/// Map an engine cursor to a winit one.
fn cursor_icon(kind: CursorKind) -> winit::window::CursorIcon {
    use winit::window::CursorIcon as Icon;
    match kind {
        CursorKind::Pointer => Icon::Pointer,
        CursorKind::Text => Icon::Text,
        CursorKind::Grab => Icon::Grab,
        CursorKind::Grabbing => Icon::Grabbing,
        CursorKind::Crosshair => Icon::Crosshair,
        CursorKind::Move => Icon::Move,
        CursorKind::ResizeNorthSouth => Icon::NsResize,
        CursorKind::ResizeEastWest => Icon::EwResize,
        CursorKind::ResizeNorthWestSouthEast => Icon::NwseResize,
        CursorKind::ResizeNorthEastSouthWest => Icon::NeswResize,
        CursorKind::NotAllowed => Icon::NotAllowed,
        CursorKind::Default | CursorKind::Custom(_) => Icon::Default,
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
        window: &winit::window::Window,
        event: PlatformEvent,
    ) -> Flow {
        match event {
            PlatformEvent::KeyInput {
                state: ButtonState::Pressed,
                key,
                repeat: false,
            } => self.on_key(key),
            PlatformEvent::MouseInput {
                state,
                button: MouseButton::Left,
            } => {
                self.on_left_button(state == ButtonState::Pressed, window);
            }
            PlatformEvent::CursorMoved { x, y } => {
                self.on_cursor_moved(x as f32, y as f32, window);
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
        self.apply_cursor(window);
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

        // Hover / press / focus / click are re-derived every frame, but the
        // scene is only rebuilt on a discrete transition. Moving the mouse
        // inside the button, panning, zooming and switching themes all leave
        // `generation` untouched, so they upload no primitive bytes.
        let visual = ButtonVisualState::resolve(self.button, &self.router, self.focused, false);
        if visual != self.visual || self.click_count != self.rendered_clicks {
            self.visual = visual;
            self.rebuild((window_width as f32, window_height as f32));
            self.apply_cursor(window);
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
        let Ok(image) = swapchain.acquire(ACQUIRE_TIMEOUT_NS) else {
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
        if let Err(error) = renderer.render_to_swapchain(
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

/// Map a screen point to canvas coordinates.
///
/// The shader maps `screen = (world - anchor) * zoom + viewport / 2`, where the
/// anchor is the world point under the viewport centre. Panning by `pan` screen
/// pixels puts `viewport/2 - pan/zoom` at the centre, so the inverse is
/// `world = (screen - viewport/2)/zoom + anchor`. Feeding screen points to the
/// pointer router without this inverse leaves every hit region offset.
fn canvas_at(screen: [f32; 2], viewport: (f32, f32), zoom: f32, pan: [f32; 2]) -> Vec2 {
    let zoom = zoom.clamp(0.25, 8.0);
    let (width, height) = viewport;
    let anchor = [width * 0.5 - pan[0] / zoom, height * 0.5 - pan[1] / zoom];
    Vec2::new(
        (screen[0] - width * 0.5) / zoom + anchor[0],
        (screen[1] - height * 0.5) / zoom + anchor[1],
    )
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

    // Give the glyph atlas the colour table and draw.
    renderer
        .write_theme(&theme_table(dark))
        .map_err(|e| e.to_string())?;
    renderer.render(&scene.upload(1)).map_err(|e| e.to_string())?;
    let pixels = renderer.read_pixels().map_err(|e| e.to_string())?;
    renderer.destroy();

    write_png(path, width, height, &pixels)
}

/// Write RGBA pixels to a PNG file, creating the parent directory if needed.
fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
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
    // The headless twin: render the gallery to a PNG and exit. Shots are
    // build artifacts, not sources: they land under `target/` by default so
    // they never end up in a commit.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--shot") {
        let dark = args.iter().any(|a| a == "--dark");
        let path = args
            .windows(2)
            .find(|w| w[0] == "--shot" && !w[1].starts_with("--"))
            .map_or_else(
                || {
                    let name = if dark {
                        "showcase-dark.png"
                    } else {
                        "showcase-light.png"
                    };
                    format!("target/showcase/{name}")
                },
                |w| w[1].clone(),
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader's forward mapping, repeated here so the inverse can be
    /// checked against the formula the GPU actually applies.
    fn screen_point(world: Vec2, viewport: (f32, f32), zoom: f32, pan: [f32; 2]) -> [f32; 2] {
        let (width, height) = viewport;
        let anchor = [width * 0.5 - pan[0] / zoom, height * 0.5 - pan[1] / zoom];
        [
            (world.x - anchor[0]) * zoom + width * 0.5,
            (world.y - anchor[1]) * zoom + height * 0.5,
        ]
    }

    #[test]
    fn the_inverse_camera_maps_the_viewport_centre_to_itself() {
        let viewport = (1280.0, 720.0);
        let world = canvas_at([640.0, 360.0], viewport, 1.0, [0.0, 0.0]);
        assert!((world.x - 640.0).abs() < f32::EPSILON);
        assert!((world.y - 360.0).abs() < f32::EPSILON);
    }

    #[test]
    fn panning_moves_canvas_coordinates_by_the_pan_amount() {
        let viewport = (1280.0, 720.0);
        let origin = canvas_at([640.0, 360.0], viewport, 1.0, [0.0, 0.0]);
        let panned = canvas_at([640.0, 360.0], viewport, 1.0, [120.0, -40.0]);
        assert!((panned.x - (origin.x - 120.0)).abs() < 1e-4);
        assert!((panned.y - (origin.y + 40.0)).abs() < 1e-4);
    }

    /// A click is only routed correctly if the screen point under the cursor
    /// survives the round trip through the camera at every zoom and pan.
    #[test]
    fn a_button_click_survives_pan_and_zoom() {
        let rect = showcase::gallery::button_rect();
        let viewport = (1280.0, 720.0);
        for (zoom, pan) in [
            (1.0, [0.0, 0.0]),
            (2.0, [60.0, -30.0]),
            (0.5, [-120.0, 40.0]),
        ] {
            let centre = rect.center();
            let screen = screen_point(centre, viewport, zoom, pan);
            let world = canvas_at(screen, viewport, zoom, pan);
            assert!((world.x - centre.x).abs() < 1e-2, "zoom {zoom} pan {pan:?}");
            assert!((world.y - centre.y).abs() < 1e-2, "zoom {zoom} pan {pan:?}");
        }
    }
}
