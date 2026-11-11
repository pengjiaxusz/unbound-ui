//! Builds the component gallery scene.
//!
//! Every entry here corresponds to a primitive type the uber shader knows
//! about, so the window shows the whole rendering vocabulary at once: rounded
//! rectangles, borders, analytic shadows, curves, images and glyphs.
//!
//! The scene is rebuilt whenever the theme changes or the canvas camera
//! moves, which is exactly the "0 byte upload" claim the engine makes for
//! those operations — only the affected primitive data is re-staged.

use unbound_gpu::data::{
    BlendMode, GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform, PrimType, SubElement,
};
use unbound_gpu::{Camera, SceneUpload};
use unbound_math::{Color, Rect, Vec2};
use unbound_text::{GlyphCache, GlyphQuad, TextEngine, TextStyle};

/// The colour tokens the gallery uses.
///
/// The theme LUT is written once per theme; the primitives only ever store
/// these indices.
///
/// Both themes use the **same** slot layout — only the colours differ — which
/// is what makes a theme switch a single 4 KiB overwrite of the table. Slot 0
/// is reserved for "no colour" and slot 1 is the engine's shadow slot
/// (`SHADOW_TOKEN` in `common.glsl`).
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
const TOKEN_SHADOW: u16 = 1;
const TOKEN_BACKGROUND: u16 = 2;
const TOKEN_SURFACE: u16 = 3;
const TOKEN_BORDER: u16 = 4;
const TOKEN_PRIMARY: u16 = 5;
const TOKEN_SECONDARY: u16 = 6;
const TOKEN_SUCCESS: u16 = 7;
const TOKEN_WARNING: u16 = 8;
const TOKEN_DANGER: u16 = 9;
const TOKEN_TEXT: u16 = 10;
const TOKEN_TEXT_MUTED: u16 = 11;

/// The tokens the gallery uses.
///
/// Both themes share the same slots — only the colours in the table differ —
/// so switching themes never touches the scene data.
#[must_use]
pub const fn palette() -> Palette {
    Palette {
        background: TOKEN_BACKGROUND,
        surface: TOKEN_SURFACE,
        border: TOKEN_BORDER,
        primary: TOKEN_PRIMARY,
        secondary: TOKEN_SECONDARY,
        success: TOKEN_SUCCESS,
        warning: TOKEN_WARNING,
        danger: TOKEN_DANGER,
        text: TOKEN_TEXT,
        text_muted: TOKEN_TEXT_MUTED,
        shadow: TOKEN_SHADOW,
    }
}

/// The colour table the GPU samples.
///
/// The first twelve slots carry the semantic tokens; the rest stay black.
/// The table is uploaded as a single 4 KiB write, which is what makes a theme
/// switch free.
pub fn theme_table(dark: bool) -> Vec<[u16; 4]> {
    let mut colors = vec![[0u16; 4]; 512];
    let mut set = |index: u16, color: Color| {
        colors[index as usize] = color.to_rgba16f();
    };

    if dark {
        set(TOKEN_SHADOW, Color::from_srgb8(0, 0, 0, 160));
        set(TOKEN_BACKGROUND, Color::from_srgb8(18, 18, 22, 255));
        set(TOKEN_SURFACE, Color::from_srgb8(32, 33, 40, 255));
        set(TOKEN_BORDER, Color::from_srgb8(58, 60, 72, 255));
        set(TOKEN_PRIMARY, Color::from_srgb8(88, 140, 255, 255));
        set(TOKEN_SECONDARY, Color::from_srgb8(150, 110, 240, 255));
        set(TOKEN_SUCCESS, Color::from_srgb8(70, 190, 130, 255));
        set(TOKEN_WARNING, Color::from_srgb8(235, 175, 70, 255));
        set(TOKEN_DANGER, Color::from_srgb8(235, 90, 95, 255));
        set(TOKEN_TEXT, Color::from_srgb8(238, 239, 245, 255));
        set(TOKEN_TEXT_MUTED, Color::from_srgb8(140, 144, 160, 255));
    } else {
        set(TOKEN_SHADOW, Color::from_srgb8(20, 22, 30, 60));
        set(TOKEN_BACKGROUND, Color::from_srgb8(245, 246, 250, 255));
        set(TOKEN_SURFACE, Color::from_srgb8(255, 255, 255, 255));
        set(TOKEN_BORDER, Color::from_srgb8(222, 224, 232, 255));
        set(TOKEN_PRIMARY, Color::from_srgb8(52, 100, 220, 255));
        set(TOKEN_SECONDARY, Color::from_srgb8(120, 80, 210, 255));
        set(TOKEN_SUCCESS, Color::from_srgb8(30, 160, 100, 255));
        set(TOKEN_WARNING, Color::from_srgb8(220, 155, 40, 255));
        set(TOKEN_DANGER, Color::from_srgb8(220, 60, 65, 255));
        set(TOKEN_TEXT, Color::from_srgb8(28, 30, 38, 255));
        set(TOKEN_TEXT_MUTED, Color::from_srgb8(120, 124, 140, 255));
    }
    colors
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
    fn new() -> Self {
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
    fn rect(
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
    fn stroke(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u16, z: u16) {
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
    fn glyph(&mut self, quad: &GlyphQuad, color: u16, z: u16) {
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
                SubElement::Content as u16,
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: payload_idx,
        };
        self.primitives.push(prim);
    }

    /// Add a quadratic curve segment.
    fn curve(
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
        self.payloads.push(draw.payload);
        self.primitives.push(prim);
    }

    /// The scene as an upload plan.
    #[must_use]
    pub fn upload(&self, generation: u64) -> SceneUpload<'_> {
        SceneUpload {
            primitives: &self.primitives,
            transforms: &self.transforms,
            clips: &self.clips,
            payloads: &self.payloads,
            generation,
            camera: self.camera,
            key_prefix: (unbound_gpu::data::PassLayer::Ui as u64) << 56,
        }
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
}

impl Labels {
    /// Create the label engine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: GlyphCache::new(),
            engine: TextEngine::new(),
        }
    }

    /// Lay out a label and return its glyph quads.
    ///
    /// `origin` is the *top-left* of the text block in screen space. The
    /// glyph quads come back in baseline-relative coordinates (a glyph's
    /// rectangle hangs above its baseline), so the whole run is shifted down
    /// by the line's ascent to put the first line's top at `origin`.
    pub fn layout(&mut self, text: &str, origin: Vec2, size: f32) -> Vec<GlyphQuad> {
        let style = TextStyle::sized(size);
        let shaped = self.engine.shape(text, &style);
        let mut quads = self
            .cache
            .quads_for(&mut self.engine, &shaped, Vec2::ZERO, 400, false);
        if quads.is_empty() {
            return quads;
        }
        // The ascent is the distance from the baseline to the top of the
        // tallest glyph; the glyph rectangles extend that far upwards.
        let top = quads
            .iter()
            .map(|q| q.rect.origin.y)
            .fold(f32::INFINITY, f32::min);
        let left = quads
            .iter()
            .map(|q| q.rect.origin.x)
            .fold(f32::INFINITY, f32::min);
        for quad in &mut quads {
            quad.rect.origin.x += origin.x - left;
            quad.rect.origin.y += origin.y - top;
        }
        quads
    }
}

impl Default for Labels {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the whole gallery.
///
/// Returns the scene and the glyph quads the caller still has to upload to
/// the GPU atlas.
pub fn build(
    labels: &mut Labels,
    palette: &Palette,
    viewport: (f32, f32),
) -> (Gallery, Vec<GlyphQuad>) {
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

    // ---- the theme hint -------------------------------------------------
    let hint = labels.layout(
        "T / 切换主题    Space / 主题    滚轮 / 缩放    拖拽 / 平移",
        Vec2::new(at(40.0, viewport.1 - 60.0).0, at(40.0, viewport.1 - 60.0).1),
        14.0,
    );
    for quad in &hint {
        scene.glyph(quad, palette.text_muted, 600);
    }

    // The glyphs are already part of the scene; the atlas updates they
    // produced are drained by the caller and uploaded to the GPU.
    (scene, Vec::new())
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
