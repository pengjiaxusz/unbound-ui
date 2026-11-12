//! Builds the component gallery scene.
//!
//! Every entry here corresponds to a primitive type the uber shader knows
//! about, so the window shows the whole rendering vocabulary at once: rounded
//! rectangles, borders, analytic shadows, curves, images and glyphs.
//!
//! The scene is rebuilt when a discrete interaction state changes (hover /
//! press / focus / click count), not when the canvas camera moves. Panning,
//! zooming and switching themes only touch the camera uniform or the 4 KiB
//! colour table, so the primitive buffers stay untouched — that is the
//! "0 byte upload" claim the engine makes for those operations.

use accesskit::Role;
use unbound_a11y::{A11yNode, Semantics};
use unbound_gpu::data::{
    BlendMode, GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform, PrimFlags, PrimType,
    SubElement,
};
use unbound_gpu::{Camera, SceneUpload};
use unbound_headless::{Frame, FrameBuilder, NodeKind};
use unbound_input::HitRegion;
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphCache, GlyphQuad, ShapedText, TextEngine, TextStyle};
use unbound_theme::Token;

use crate::button::{self, ButtonDesc, ButtonVisualState};

/// The colour tokens the gallery uses.
///
/// The theme LUT is written once per theme; the primitives only ever store
/// these indices.
///
/// Both themes use the **same** slot layout — only the colours differ — which
/// is what makes a theme switch a single 4 KiB overwrite of the table. Slot 0
/// is reserved for "no colour" and slot 1 is the engine's shadow slot
/// (`SHADOW_TOKEN` in `common.glsl`).
#[derive(Copy, Clone, Debug)]
pub struct Palette {
    /// The page background.
    pub background: u16,
    /// The panel surface.
    pub surface: u16,
    /// The panel border.
    pub border: u16,
    /// The primary accent.
    pub primary: u16,
    /// The secondary accent.
    pub secondary: u16,
    /// The success colour.
    pub success: u16,
    /// The warning colour.
    pub warning: u16,
    /// The danger colour.
    pub danger: u16,
    /// The body text colour.
    pub text: u16,
    /// The muted text colour.
    pub text_muted: u16,
    /// The shadow colour.
    pub shadow: u16,
    /// The button fill while hovered.
    pub primary_hover: u16,
    /// The button fill while pressed.
    pub primary_pressed: u16,
    /// The button label colour.
    pub on_primary: u16,
    /// The focus ring colour.
    pub focus_ring: u16,
    /// The disabled fill.
    pub disabled_fill: u16,
    /// The title bar / global header surface.
    pub header_bg: u16,
    /// The navigation pane surface.
    pub nav_bg: u16,
    /// A hovered navigation row.
    pub nav_hover: u16,
    /// The selected navigation row.
    pub nav_selected: u16,
    /// The example card surface.
    pub card_bg: u16,
    /// The example card outline.
    pub card_border: u16,
    /// The live stage background.
    pub stage_bg: u16,
    /// The stage's grid dots.
    pub stage_grid: u16,
    /// Slider / switch tracks.
    pub control_track: u16,
    /// Slider thumbs and switch knobs.
    pub control_thumb: u16,
    /// A hovered control surface.
    pub control_hover: u16,
    /// The code panel background.
    pub code_bg: u16,
    /// The code panel foreground.
    pub code_text: u16,
    /// The modal scrim.
    pub overlay_scrim: u16,
    /// The overlay surface.
    pub overlay_bg: u16,
    /// A 12 % primary tint.
    pub primary_soft: u16,
    /// A 12 % success tint.
    pub success_soft: u16,
    /// A 12 % warning tint.
    pub warning_soft: u16,
    /// A 12 % danger tint.
    pub danger_soft: u16,
    /// Hero gradient start.
    pub hero_start: u16,
    /// Hero gradient end.
    pub hero_end: u16,
    /// Text and icons painted on the hero gradient.
    pub on_hero: u16,
    /// Text selection fill.
    pub selection_fill: u16,
    /// Hairline dividers.
    pub divider: u16,
    /// The performance HUD background.
    pub hud_bg: u16,
    /// The performance HUD foreground.
    pub hud_text: u16,
    /// A 12 % accent tint.
    pub accent_soft: u16,
    /// Canvas / stage grid dots.
    pub grid_dot: u16,
    /// A 12 % secondary tint.
    pub secondary_soft: u16,
    /// Navigation primary text.
    pub nav_text: u16,
    /// Navigation secondary text.
    pub nav_text_muted: u16,
    /// The scrim behind a call-to-action on the hero surface.
    pub hero_scrim: u16,
}

impl Palette {
    /// The colour swatches the Design Guidance page displays.
    #[must_use]
    pub fn swatches(&self) -> [(&'static str, u16); 12] {
        [
            ("Surface", self.surface),
            ("Raised", self.card_bg),
            ("Primary", self.primary),
            ("Primary soft", self.primary_soft),
            ("Accent", self.secondary),
            ("Success", self.success),
            ("Warning", self.warning),
            ("Danger", self.danger),
            ("Text", self.text),
            ("Muted", self.text_muted),
            ("Outline", self.border),
            ("Shadow", self.shadow),
        ]
    }
}

/// The token layout, shared by both themes.
///
/// | slot | meaning |
/// | ---: | :--- |
/// | 0 | none (fully transparent) |
/// | 1 | shadow — fixed by the engine's `SHADOW_TOKEN` |
/// | 2 | page background |
/// | 3 | panel surface |
/// | 4 | panel border |
/// | 5..9 | primary, secondary, success, warning, danger |
/// | 10 | body text |
/// | 11 | muted text |
/// | 12 | button primary — hover |
/// | 13 | button primary — pressed |
/// | 14 | button label (on primary) |
/// | 15 | focus ring |
/// | 16 | disabled fill |
/// The tokens the gallery uses.
///
/// Both themes share the same slots — only the colours in the table differ —
/// so switching themes never touches the scene data. The slot numbers come
/// from the engine's semantic vocabulary ([`unbound_theme::Token`]); the
/// colours themselves live in [`crate::theme`].
#[must_use]
pub const fn palette() -> Palette {
    Palette {
        background: Token::SURFACE.raw(),
        surface: Token::SURFACE_RAISED.raw(),
        border: Token::OUTLINE.raw(),
        primary: Token::PRIMARY.raw(),
        secondary: Token::SECONDARY.raw(),
        success: Token::SUCCESS.raw(),
        warning: Token::WARNING.raw(),
        danger: Token::ERROR.raw(),
        text: Token::TEXT_PRIMARY.raw(),
        text_muted: Token::TEXT_SECONDARY.raw(),
        shadow: Token::SHADOW.raw(),
        primary_hover: Token::PRIMARY_HOVER.raw(),
        primary_pressed: Token::PRIMARY_PRESSED.raw(),
        on_primary: Token::ON_PRIMARY.raw(),
        focus_ring: Token::FOCUS_RING.raw(),
        disabled_fill: Token::DISABLED.raw(),
        header_bg: Token::HEADER_BG.raw(),
        nav_bg: Token::NAV_BG.raw(),
        nav_hover: Token::NAV_HOVER.raw(),
        nav_selected: Token::NAV_SELECTED.raw(),
        card_bg: Token::CARD_BG.raw(),
        card_border: Token::CARD_BORDER.raw(),
        stage_bg: Token::CANVAS_BACKDROP.raw(),
        stage_grid: Token::GRID.raw(),
        control_track: Token::CONTROL_TRACK.raw(),
        control_thumb: Token::CONTROL_THUMB.raw(),
        control_hover: Token::CONTROL_HOVER.raw(),
        code_bg: Token::CODE_BG.raw(),
        code_text: Token::CODE_TEXT.raw(),
        overlay_scrim: Token::SCRIM.raw(),
        overlay_bg: Token::OVERLAY_BG.raw(),
        primary_soft: Token::PRIMARY_SOFT.raw(),
        success_soft: Token::SUCCESS_SOFT.raw(),
        warning_soft: Token::WARNING_SOFT.raw(),
        danger_soft: Token::ERROR_SOFT.raw(),
        hero_start: Token::HERO_START.raw(),
        hero_end: Token::HERO_END.raw(),
        on_hero: Token::ON_HERO.raw(),
        selection_fill: Token::SELECTION.raw(),
        divider: Token::DIVIDER.raw(),
        hud_bg: Token::HUD_BG.raw(),
        hud_text: Token::HUD_TEXT.raw(),
        accent_soft: Token::ACCENT_SOFT.raw(),
        grid_dot: Token::GRID_DOT.raw(),
        secondary_soft: Token::SECONDARY_SOFT.raw(),
        nav_text: Token::NAV_TEXT.raw(),
        nav_text_muted: Token::NAV_TEXT_MUTED.raw(),
        hero_scrim: Token::HERO_SCRIM.raw(),
    }
}

/// The gallery scene, ready to be uploaded.
#[derive(Default)]
pub struct Gallery {
    /// The primitives, in draw order.
    pub primitives: Vec<GpuPrimitiveInstance>,
    /// The transform table; the scene only uses the identity.
    pub transforms: Vec<GpuTransform>,
    /// The clip nodes.
    pub clips: Vec<GpuClipNode>,
    /// The payloads for curves and glyphs.
    pub payloads: Vec<GpuPayload>,
    /// The camera.
    pub camera: Camera,
}

impl Gallery {
    /// Start an empty scene.
    pub(crate) fn new() -> Self {
        Self {
            primitives: Vec::new(),
            transforms: vec![GpuTransform::IDENTITY],
            clips: Vec::new(),
            payloads: Vec::new(),
            camera: Camera::ui((1280.0, 720.0), 1.0),
        }
    }

    /// Add a rounded rectangle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        fill: u16,
        border: u16,
        border_width: f32,
        shadow: f32,
        z: u16,
    ) {
        let prim = GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [x, y],
            size: [w, h],
            corner_radii: unbound_math::f16::pack_radii([radius; 4]),
            border_width: unbound_math::f16::f32_to_f16(border_width),
            shadow_blur: unbound_math::f16::f32_to_f16(shadow),
            transform_idx: 0,
            clip_idx: unbound_gpu::data::CLIP_NONE,
            fill_token_or_uv: u32::from(fill),
            border_token_or_tex: u32::from(border),
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                PrimType::RoundedRect,
                BlendMode::Normal,
                SubElement::Background as u16,
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: 0,
        };
        self.primitives.push(prim);
    }

    /// Add a stroke (a hollow rounded rectangle).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stroke(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: u16,
        z: u16,
    ) {
        let prim = GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [x, y],
            size: [w, h],
            corner_radii: unbound_math::f16::pack_radii([radius; 4]),
            border_width: unbound_math::f16::f32_to_f16(2.0),
            shadow_blur: 0,
            transform_idx: 0,
            clip_idx: unbound_gpu::data::CLIP_NONE,
            fill_token_or_uv: 0,
            border_token_or_tex: u32::from(color),
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                PrimType::Stroke,
                BlendMode::Normal,
                SubElement::Border as u16,
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: 0,
        };
        self.primitives.push(prim);
    }

    /// Add a glyph quad.
    pub(crate) fn glyph(&mut self, quad: &GlyphQuad, color: u16, z: u16) {
        let payload = GpuPayload {
            p0: [quad.uv[0], quad.uv[1]],
            p1: [quad.uv[2], quad.uv[3]],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        };
        let payload_idx = self.payloads.len() as u32;
        self.payloads.push(payload);

        let prim = GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [quad.rect.origin.x, quad.rect.origin.y],
            size: [quad.rect.width(), quad.rect.height()],
            corner_radii: [0; 4],
            border_width: 0,
            shadow_blur: 0,
            transform_idx: 0,
            clip_idx: unbound_gpu::data::CLIP_NONE,
            fill_token_or_uv: u32::from(color),
            // The atlas layer rides in the border slot.
            border_token_or_tex: quad.layer,
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                if quad.color {
                    PrimType::GlyphColor
                } else {
                    PrimType::Glyph
                },
                BlendMode::Normal,
                // The shader has to know that the RGBA texel is a distance
                // field; without the flag it treats it as a plain colour
                // bitmap and paints an opaque blob.
                SubElement::Content as u16 | if quad.msdf { PrimFlags::MSDF.bits() } else { 0 },
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: payload_idx,
        };
        self.primitives.push(prim);
    }

    /// Add a quadratic curve segment.
    pub(crate) fn curve(
        &mut self,
        p0: [f32; 2],
        p1: [f32; 2],
        p2: [f32; 2],
        line_width: f32,
        color: u16,
        z: u16,
    ) {
        let segment = unbound_vector::QuadSegment::new(
            Vec2::new(p0[0], p0[1]),
            Vec2::new(p1[0], p1[1]),
            Vec2::new(p2[0], p2[1]),
        );
        let instance = unbound_vector::CurveInstance::from_quad(&segment, line_width);
        let draw = unbound_gpu::CurveDraw::new([0, 0], &instance, u32::from(color), line_width, z);
        let mut prim = draw.primitive;
        prim.extra_payload_idx = self.payloads.len() as u32;
        prim.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
            PrimType::Curve,
            BlendMode::Normal,
            SubElement::Content as u16,
        );
        // The control points live in the oriented box's *own* frame, so the
        // box's rotation has to reach the vertex shader. Without it the curve
        // is drawn unrotated, in the wrong place, and cut off at the edge of
        // an axis-aligned box that was never rotated.
        let transform_idx = if draw.transform == GpuTransform::IDENTITY {
            0
        } else {
            let idx = self.transforms.len() as u32;
            self.transforms.push(draw.transform);
            idx
        };
        prim.transform_idx = transform_idx;
        self.payloads.push(draw.payload);
        self.primitives.push(prim);
    }

    /// The scene as an upload plan.
    ///
    /// `dirty_ranges` are the primitive ranges that changed since the last
    /// upload. An empty slice means "upload everything"; a non-empty one
    /// makes the renderer patch just those slices, which is what turns a
    /// hover into a handful of uploaded bytes.
    #[must_use]
    pub fn upload<'a>(
        &'a self,
        generation: u64,
        dirty_ranges: &'a [(u32, u32)],
    ) -> SceneUpload<'a> {
        SceneUpload {
            primitives: &self.primitives,
            transforms: &self.transforms,
            clips: &self.clips,
            payloads: &self.payloads,
            generation,
            camera: self.camera,
            key_prefix: (unbound_gpu::data::PassLayer::Ui as u64) << 56,
            dirty_ranges,
        }
    }
}

/// The metrics of a laid-out run, in logical units.
///
/// The vertical metrics come from the **font**, not from the run's ink. Two
/// runs of the same size have to share a baseline — otherwise a row of
/// buttons shows `"Comfortable"` a couple of pixels lower than `"Compact"`,
/// because the second string has no descender and the first one has an
/// ascender.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct TextMetrics {
    /// The advance width of the widest line.
    pub width: f32,
    /// The height of the whole text block.
    pub height: f32,
    /// The distance from the top of the line box to the baseline.
    pub ascent: f32,
    /// The distance from the baseline to the bottom of the line box.
    pub descent: f32,
    /// The baseline of the first line, relative to the top of the line box.
    pub first_baseline: f32,
    /// How many lines the run occupies.
    pub line_count: u32,
}

impl TextMetrics {
    /// The height of one line box.
    #[must_use]
    pub fn line_height(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// The text engine that drives the gallery's labels.
///
/// `TextEngine` is both the shaper and the two-level cache owner; the
/// `GlyphCache` on top rasterises glyphs and keeps them in the atlas.
pub struct Labels {
    /// The glyph cache.
    pub cache: GlyphCache,
    /// The shaping engine.
    pub engine: TextEngine,
    /// How many atlas texels one logical unit covers.
    ///
    /// The layout stays in logical units; rasterising at
    /// `font_size × raster_scale` and scaling the quads back keeps every glyph
    /// sharp when the OS scale or the app zoom magnifies the whole UI.
    raster_scale: f32,
    /// The size and raster scale the dynamic button label was prewarmed at.
    prewarmed: Option<(f32, f32)>,
}

impl Labels {
    /// Create the label engine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: GlyphCache::new(),
            engine: TextEngine::new(),
            raster_scale: 1.0,
            prewarmed: None,
        }
    }

    /// Rasterise at `logical_size × scale` and place the quads in logical units.
    pub fn set_raster_scale(&mut self, scale: f32) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        self.raster_scale = scale.clamp(0.5, 4.0);
    }

    /// Whether glyphs are rasterised for the whole-pixel grid.
    pub fn set_snap_phases(&mut self, snap: bool) {
        self.cache.snap_phases = snap;
    }

    /// The scale glyphs are rasterised at.
    #[must_use]
    pub fn raster_scale(&self) -> f32 {
        self.raster_scale
    }

    /// The origin that centres a run inside `rect`.
    ///
    /// The vertical centring uses the font's ascent and descent rather than
    /// the run's ink box, so every label of the same size lands on the same
    /// baseline. The horizontal centring uses the advance width, which is
    /// what the font itself considers the run's width.
    #[must_use]
    pub fn center_in_rect(metrics: &TextMetrics, rect: Rect) -> Vec2 {
        let x = rect.origin.x + (rect.size.x - metrics.width) * 0.5;
        // The optical middle of the line box: the baseline sits half a line
        // box above the centre of the box.
        let middle = (metrics.ascent - metrics.descent) * 0.5;
        let target_baseline = rect.origin.y + rect.size.y * 0.5 + middle;
        Vec2::new(x, target_baseline - metrics.first_baseline)
    }

    /// The metrics of a run at `size`, in logical units.
    pub fn metrics(&mut self, text: &str, size: f32) -> TextMetrics {
        let scale = self.raster_scale;
        let shaped = self.engine.shape(text, &TextStyle::sized(size * scale));
        metrics_of(&shaped, scale)
    }

    /// Rasterise every glyph a dynamic label can produce before it is shown.
    ///
    /// The atlas is uploaded once at startup. If clicking the button changed
    /// `"Clicked: 0"` to `"Clicked: 1"` and `'1'` had never been shaped, the
    /// quad would sample an empty atlas page and the glyph would render as a
    /// blank box. Shaping the whole alphabet up front keeps the click at zero
    /// texture uploads.
    pub fn prewarm(&mut self, text: &str, size: f32) {
        let scale = self.raster_scale;
        if self.prewarmed == Some((size, scale)) {
            return;
        }
        let style = TextStyle::sized(size * scale);
        let shaped = self.engine.shape(text, &style);
        let _ = self
            .cache
            .quads_for(&mut self.engine, &shaped, Vec2::ZERO, 400, false);
        self.prewarmed = Some((size, scale));
    }

    /// Lay out a label and return its glyph quads.
    ///
    /// `origin` is the top-left of the text's **line box** in logical units:
    /// the baseline of the first line sits at `origin.y + ascent` and the
    /// glyphs hang above it.
    ///
    /// The run is deliberately *not* aligned by its ink. Aligning by the ink
    /// box moves every string to a different baseline, because the ink top
    /// depends on the string's own tallest glyph — and that is what made the
    /// labels of a row of buttons sit at slightly different heights.
    ///
    /// The origin is passed to the glyph cache *before* the quads are
    /// produced, so the sub-pixel phase of every glyph is derived from its
    /// real position on screen. A phase derived from a run-relative position
    /// would shift each glyph by up to a pixel.
    pub fn layout(&mut self, text: &str, origin: Vec2, size: f32) -> Vec<GlyphQuad> {
        self.layout_measured(text, origin, size).0
    }

    /// Lay out a label and report its metrics.
    pub fn layout_measured(
        &mut self,
        text: &str,
        origin: Vec2,
        size: f32,
    ) -> (Vec<GlyphQuad>, TextMetrics) {
        let scale = self.raster_scale;
        let style = TextStyle::sized(size * scale);
        let shaped = self.engine.shape(text, &style);
        let metrics = metrics_of(&shaped, scale);
        let mut quads = self
            .cache
            .quads_for(&mut self.engine, &shaped, origin * scale, 400, false);
        if (scale - 1.0).abs() > 1e-6 {
            for quad in &mut quads {
                quad.rect.origin.x /= scale;
                quad.rect.origin.y /= scale;
                quad.rect.size.x /= scale;
                quad.rect.size.y /= scale;
            }
        }
        (quads, metrics)
    }

    /// Measure a single-line run without emitting glyphs.
    #[must_use]
    pub fn measure(&mut self, text: &str, size: f32) -> Vec2 {
        let scale = self.raster_scale;
        self.engine.measure(text, &TextStyle::sized(size * scale)) / scale
    }

    /// Lay out a wrapped paragraph, one shaped line at a time.
    ///
    /// The line breaks come from the engine's wrap cache, so resizing a card
    /// only recomputes the break positions, never the glyph shapes.
    pub fn layout_wrapped(
        &mut self,
        text: &str,
        origin: Vec2,
        size: f32,
        max_width: f32,
    ) -> Vec<GlyphQuad> {
        let scale = self.raster_scale;
        let style = TextStyle::sized(size * scale);
        let shaped = self
            .engine
            .shape_wrapped(text, &style, (max_width * scale).max(1.0));
        let mut quads = self
            .cache
            .quads_for(&mut self.engine, &shaped, origin * scale, 400, false);
        if (scale - 1.0).abs() > 1e-6 {
            for quad in &mut quads {
                quad.rect.origin.x /= scale;
                quad.rect.origin.y /= scale;
                quad.rect.size.x /= scale;
                quad.rect.size.y /= scale;
            }
        }
        quads
    }
}

/// The logical metrics of a shaped run.
fn metrics_of(shaped: &ShapedText, scale: f32) -> TextMetrics {
    TextMetrics {
        width: shaped.width / scale,
        height: shaped.height / scale,
        ascent: shaped.ascent / scale,
        descent: shaped.descent / scale,
        first_baseline: shaped.first_baseline / scale,
        line_count: shaped.line_count,
    }
}

impl Default for Labels {
    fn default() -> Self {
        Self::new()
    }
}

/// The button's rect in canvas coordinates.
#[must_use]
pub fn button_rect() -> Rect {
    Rect::new(120.0, 468.0, 200.0, 44.0)
}

/// The placeholder identity [`build`] gives the button when the caller does
/// not own a `UiWorld` (the offscreen `--shot` path and the visual tests).
pub const DEFAULT_BUTTON_NODE: u64 = 2;

/// The root of the semantic tree the gallery reports.
const A11Y_ROOT: u64 = 1;

/// The characters the dynamic button label can contain. Shaped up front so a
/// click never grows the atlas.
const BUTTON_LABEL_ALPHABET: &str = "Clicked: 0123456789";

/// Build the whole gallery with the button in its idle state.
///
/// Returns the scene and the glyph quads the caller still has to upload to
/// the GPU atlas.
pub fn build(
    labels: &mut Labels,
    palette: &Palette,
    viewport: (f32, f32),
) -> (Gallery, Vec<GlyphQuad>) {
    let button = ButtonDesc::new(button_rect(), "Clicked: 0");
    let (scene, glyphs, _, _) = build_interactive(
        labels,
        palette,
        viewport,
        &button,
        DEFAULT_BUTTON_NODE,
        ButtonVisualState::default(),
        0,
    );
    (scene, glyphs)
}

/// Build the gallery with the button in `visual` state.
///
/// Returns the scene, the button label's glyph quads (already placed in scene
/// space), the hit regions the pointer router should own, and the semantic
/// tree. The signature is the first step towards the `(Scene, HitRegions,
/// Semantics)` triple the platform layer will consume.
pub fn build_interactive(
    labels: &mut Labels,
    palette: &Palette,
    viewport: (f32, f32),
    button: &ButtonDesc,
    node: u64,
    visual: ButtonVisualState,
    click_count: u32,
) -> (Gallery, Vec<GlyphQuad>, Vec<HitRegion>, Semantics) {
    let mut scene = Gallery::new();
    scene.camera = Camera::ui(viewport, 1.0);

    // `local_pos` is the absolute position inside chunk [0, 0], which for a
    // UI primitive *is* the screen position: the shader subtracts the camera
    // offset itself (`chunk_base = -camera_local`).
    let at = |x: f32, y: f32| (x, y);

    // ---- the page background ------------------------------------------
    let (bx, by) = at(0.0, 0.0);
    scene.rect(
        bx,
        by,
        viewport.0,
        viewport.1,
        0.0,
        palette.background,
        palette.background,
        0.0,
        0.0,
        0,
    );

    // ---- the title -----------------------------------------------------
    let title = labels.layout(
        "Unbound UI — 组件展台",
        Vec2::new(at(40.0, 32.0).0, at(40.0, 32.0).1),
        28.0,
    );
    for quad in &title {
        scene.glyph(quad, palette.text, 100);
    }

    // ---- the primitive grid --------------------------------------------
    let swatches = [
        ("Primary", palette.primary),
        ("Secondary", palette.secondary),
        ("Success", palette.success),
        ("Warning", palette.warning),
        ("Danger", palette.danger),
    ];

    for (index, (name, token)) in swatches.iter().enumerate() {
        let x = 40.0 + index as f32 * 150.0;
        // Reverse-Z: a larger depth is nearer and is drawn later, so the
        // swatch sits above the page background and its label above the
        // swatch.
        let z = 200 + index as u16;
        let (rx, ry) = at(x, 100.0);
        // The analytic shadow is a property of the primitive itself: the
        // shader derives it from `shadow_blur`, no extra draw is needed.
        scene.rect(
            rx,
            ry,
            120.0,
            64.0,
            12.0,
            *token,
            palette.border,
            1.0,
            14.0,
            z + 1,
        );
        let label = labels.layout(
            name,
            Vec2::new(at(x + 10.0, 174.0).0, at(x + 10.0, 174.0).1),
            13.0,
        );
        for quad in &label {
            scene.glyph(quad, palette.text_muted, z + 2);
        }
    }

    // ---- a card with a border and a shadow ------------------------------
    let (cx, cy) = at(40.0, 230.0);
    scene.rect(
        cx,
        cy,
        360.0,
        150.0,
        16.0,
        palette.surface,
        palette.border,
        1.0,
        18.0,
        301,
    );
    let card_title = labels.layout(
        "Card / 圆角 + 边框 + 解析阴影",
        Vec2::new(at(60.0, 250.0).0, at(60.0, 250.0).1),
        16.0,
    );
    for quad in &card_title {
        scene.glyph(quad, palette.text, 302);
    }
    let card_body = labels.layout(
        "Analytic shadow (erf), zero offscreen passes.",
        Vec2::new(at(60.0, 278.0).0, at(60.0, 278.0).1),
        13.0,
    );
    for quad in &card_body {
        scene.glyph(quad, palette.text_muted, 302);
    }

    // ---- stroke (focus ring) -------------------------------------------
    let (sx, sy) = at(430.0, 230.0);
    scene.rect(
        sx,
        sy,
        160.0,
        150.0,
        16.0,
        palette.surface,
        palette.surface,
        0.0,
        0.0,
        400,
    );
    scene.stroke(sx, sy, 160.0, 150.0, 16.0, palette.primary, 401);
    let stroke_label = labels.layout(
        "Stroke / 焦点环",
        Vec2::new(at(sx + 12.0, sy + 120.0).0, at(sx + 12.0, sy + 120.0).1),
        13.0,
    );
    for quad in &stroke_label {
        scene.glyph(quad, palette.text_muted, 402);
    }

    // ---- curves ---------------------------------------------------------
    let (gx, gy) = at(620.0, 230.0);
    scene.rect(
        gx,
        gy,
        300.0,
        150.0,
        16.0,
        palette.surface,
        palette.border,
        1.0,
        0.0,
        500,
    );
    for (index, color) in [palette.primary, palette.secondary, palette.success]
        .iter()
        .enumerate()
    {
        let f = index as f32;
        scene.curve(
            [gx + 20.0, gy + 30.0 + f * 30.0],
            [gx + 150.0, gy + 110.0 - f * 20.0],
            [gx + 280.0, gy + 30.0 + f * 30.0],
            3.0,
            *color,
            501 + index as u16,
        );
    }
    let curve_label = labels.layout(
        "Curves / 二次贝塞尔解析 SDF",
        Vec2::new(at(gx + 14.0, gy + 120.0).0, at(gx + 14.0, gy + 120.0).1),
        13.0,
    );
    for quad in &curve_label {
        scene.glyph(quad, palette.text_muted, 505);
    }

    // ---- the interactive card ------------------------------------------
    scene.rect(
        40.0,
        410.0,
        360.0,
        150.0,
        16.0,
        palette.surface,
        palette.border,
        1.0,
        18.0,
        700,
    );
    let interactive_title =
        labels.layout("Button / 第一个可交互控件", Vec2::new(60.0, 430.0), 16.0);
    for quad in &interactive_title {
        scene.glyph(quad, palette.text, 701);
    }
    labels.prewarm(BUTTON_LABEL_ALPHABET, button.font_size);
    let mut glyphs = labels.layout(&button.label, Vec2::ZERO, button.font_size);
    let artifacts = button::emit_button(node, button, visual, &mut glyphs);
    let base = scene.payloads.len() as u32;
    let mut primitives = artifacts.primitives;
    for prim in &mut primitives {
        if matches!(prim.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
            prim.extra_payload_idx += base;
        }
    }
    scene.primitives.extend(primitives);
    scene.payloads.extend(artifacts.payloads);

    let button_hint = labels.layout(
        "hover / press / click · Tab 聚焦 · Space 激活",
        Vec2::new(60.0, 524.0),
        13.0,
    );
    for quad in &button_hint {
        scene.glyph(quad, palette.text_muted, 701);
    }

    // ---- the theme hint -------------------------------------------------
    let hint = labels.layout(
        "T / 切换主题    Space / 主题    滚轮 / 缩放    拖拽 / 平移",
        Vec2::new(at(40.0, viewport.1 - 60.0).0, at(40.0, viewport.1 - 60.0).1),
        14.0,
    );
    for quad in &hint {
        scene.glyph(quad, palette.text_muted, 600);
    }

    // ---- semantics ------------------------------------------------------
    let mut semantics = Semantics::new(A11Y_ROOT);
    semantics.upsert(
        A11Y_ROOT,
        A11yNode::new(
            Role::Window,
            "Unbound UI — 组件展台",
            Rect::new(0.0, 0.0, viewport.0, viewport.1),
        ),
    );
    let mut button_node = artifacts.a11y.unwrap_or_default();
    button_node.value = click_count.to_string();
    semantics.upsert(node, button_node);
    semantics.set_children(A11Y_ROOT, &[node]);

    // The glyphs are already part of the scene; the atlas updates they
    // produced are drained by the caller and uploaded to the GPU.
    (
        scene,
        glyphs,
        artifacts.hit.into_iter().collect(),
        semantics,
    )
}

/// The button's geometry as a headless frame, for the five sanitizers.
///
/// Kept next to the emitter so the rect the pointer hits, the rect the frame
/// validates and the rect the primitives paint can never drift apart.
#[must_use]
pub fn button_frame(viewport: (f32, f32), button: &ButtonDesc, glyphs: &[GlyphQuad]) -> Frame {
    let viewport = Rect::new(0.0, 0.0, viewport.0, viewport.1);
    let mut frame = FrameBuilder::new(viewport);
    frame.open(NodeKind::Container, viewport, "Root");
    frame.open(
        NodeKind::Container,
        Rect::new(40.0, 410.0, 360.0, 150.0),
        "ButtonCard",
    );
    frame.open(
        NodeKind::Interactive {
            min_size: NodeKind::DEFAULT_MIN_SIZE,
            occluded: false,
        },
        button.rect,
        "Button",
    );
    frame.set_clip(button.rect);
    if let Some(ink) = button::ink_bounds(glyphs) {
        frame.open(
            NodeKind::Text {
                ink,
                ellipsis: false,
            },
            ink,
            "Label",
        );
        frame.close();
    }
    frame.close();
    frame.close();
    frame.close();
    frame.build()
}

/// The scene's bounding rect, used by the layout tests.
#[must_use]
pub fn scene_bounds(scene: &Gallery) -> Rect {
    let mut bounds = Rect::ZERO;
    for prim in &scene.primitives {
        let rect = Rect::new(
            prim.local_pos[0],
            prim.local_pos[1],
            prim.size[0],
            prim.size[1],
        );
        bounds = bounds.union(&rect);
    }
    bounds
}
