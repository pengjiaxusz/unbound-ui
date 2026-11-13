//! Painting surface for the gallery shell.
//!
//! [`Canvas`] is the only place that knows about primitive emission, clipping,
//! hit regions and semantics. Pages call its high-level methods and never touch
//! `GpuPrimitiveInstance` directly, which keeps the shell code readable and the
//! event/geometry wiring in one place.

use accesskit::Role;
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_a11y::{A11yNode, Semantics};
use unbound_gpu::data::{GpuClipNode, GpuPrimitiveInstance, CLIP_NONE};
use unbound_input::{ClipRect, CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphQuad, TextStyle};
use unbound_widgets::{ControlState, PointerPhase, WidgetCatalogId, WidgetOutput};

use crate::gallery::{Gallery, Labels, Palette};
use crate::target::{self, Target, ROOT};

/// Horizontal alignment inside a box.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlignX {
    Start,
    Center,
    End,
}

/// Vertical alignment inside a box.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlignY {
    Top,
    Center,
    Bottom,
}

/// The hover / press state of one identity.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Visual {
    Idle,
    Hover,
    Pressed,
    Disabled,
}

impl Visual {
    #[must_use]
    pub fn is_hovered(self) -> bool {
        matches!(self, Self::Hover)
    }
    #[must_use]
    pub fn is_pressed(self) -> bool {
        matches!(self, Self::Pressed)
    }
    #[must_use]
    pub fn is_disabled(self) -> bool {
        matches!(self, Self::Disabled)
    }
}

/// What a page build produced.
pub struct CanvasOutput {
    pub regions: Vec<HitRegion>,
    pub semantics: Semantics,
    pub focus_order: Vec<u64>,
    pub scroll_zones: Vec<(Rect, u8)>,
    pub ranges: Vec<RangeZone>,
    pub glyph_count: usize,
    /// The primitive range each control owns, `[start, end)`.
    ///
    /// A control's primitives are emitted consecutively, so the span is what
    /// lets the upload path say *which control* a dirty slice belongs to —
    /// and what makes a hover patch provably local.
    pub node_spans: FxHashMap<u64, (u32, u32)>,
    /// The real widget emitters that ran this frame (coverage gate).
    pub emitted_widgets: FxHashSet<WidgetCatalogId>,
}

/// A slider-like region whose value is derived from the pointer's x position.
#[derive(Copy, Clone, Debug)]
pub struct RangeZone {
    pub node: u64,
    pub rect: Rect,
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

/// A rounded clip shape: the rectangle and its corner radii.
///
/// This is the CPU-side twin of the two rounded slots a `GpuClipNode`
/// carries.
#[derive(Copy, Clone, Debug)]
struct RoundedSlot {
    rect: Rect,
    radii: [f32; 4],
}

/// The scene painter.
pub struct Canvas<'a> {
    pub scene: &'a mut Gallery,
    pub labels: &'a mut Labels,
    pub palette: &'a Palette,
    pub viewport: Rect,
    regions: Vec<HitRegion>,
    semantics: Semantics,
    focus_order: Vec<u64>,
    a11y_nodes: Vec<u64>,
    scroll_zones: Vec<(Rect, u8)>,
    ranges: Vec<RangeZone>,
    glyphs_emitted: usize,
    focused: Option<u64>,
    hover: Option<u64>,
    pressed: Option<u64>,
    z: u16,
    clips: Vec<u32>,
    clip_rects: Vec<ClipRect>,
    /// The rounded clip slots inherited by the current level.
    ///
    /// Slot 0 is the innermost rounded ancestor, slot 1 the one above it —
    /// the same two-slot model the shader evaluates. Keeping them in a stack
    /// (instead of squeezing them into the single `ClipRect` of the CPU
    /// mirror) means a plain axis-aligned clip in the middle no longer wipes
    /// the rounded ancestor out.
    rounded_stack: Vec<[Option<RoundedSlot>; 2]>,
    scale: f32,
    snap: bool,
    static_ids: u64,
    /// The primitive range each control owns.
    node_spans: FxHashMap<u64, (u32, u32)>,
    /// The real `unbound-widgets` emitters this frame actually ran.
    ///
    /// The compile-time coverage gate maps every [`WidgetCatalogId`] to a
    /// demo; this set is the runtime half of the proof — a demo that merely
    /// hand-draws a rectangle never inserts its id, so the coverage test can
    /// tell a genuine emission from a fake.
    emitted_widgets: FxHashSet<WidgetCatalogId>,
}

impl<'a> Canvas<'a> {
    /// Create a painter over an empty scene.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scene: &'a mut Gallery,
        labels: &'a mut Labels,
        palette: &'a Palette,
        viewport: (f32, f32),
        focused: Option<u64>,
        hover: Option<u64>,
        pressed: Option<u64>,
        scale: f32,
        snap: bool,
    ) -> Self {
        let viewport = Rect::new(0.0, 0.0, viewport.0, viewport.1);
        let mut semantics = Semantics::new(ROOT);
        semantics.upsert(
            ROOT,
            A11yNode::new(Role::Window, "Unbound UI Gallery", viewport),
        );
        Self {
            scene,
            labels,
            palette,
            viewport,
            regions: Vec::new(),
            semantics,
            focus_order: Vec::new(),
            a11y_nodes: Vec::new(),
            scroll_zones: Vec::new(),
            ranges: Vec::new(),
            glyphs_emitted: 0,
            focused,
            hover,
            pressed,
            z: 1,
            clips: Vec::new(),
            clip_rects: Vec::new(),
            rounded_stack: Vec::new(),
            scale: scale.max(0.25),
            snap,
            static_ids: 0,
            node_spans: FxHashMap::default(),
            emitted_widgets: FxHashSet::default(),
        }
    }

    /// The index the next primitive will occupy.
    ///
    /// A hand-drawn control reads this before its first primitive and hands
    /// it to [`record_span`](Self::record_span) after its last, which is how
    /// it joins the span map the same way an emitted control does.
    #[must_use]
    pub fn span_start(&self) -> usize {
        self.scene.primitives.len()
    }

    /// Record the primitives emitted since `start` as belonging to `node`.
    pub fn record_span(&mut self, node: u64, start: usize) {
        let end = self.scene.primitives.len();
        if end > start {
            self.node_spans.insert(node, (start as u32, end as u32));
        }
    }

    /// Whether `node` currently owns the hover highlight.
    #[must_use]
    pub fn hovered(&self, node: u64) -> bool {
        self.hover == Some(node)
    }

    /// Whether `node` currently owns the pressed highlight.
    #[must_use]
    pub fn pressed(&self, node: u64) -> bool {
        self.pressed == Some(node)
    }

    /// The current paint depth.
    #[must_use]
    pub fn z(&self) -> u16 {
        self.z
    }

    /// Reserve the next paint depth.
    pub fn next_z(&mut self) -> u16 {
        let z = self.z;
        self.z = self.z.saturating_add(1);
        z
    }

    /// Snap a logical coordinate to the physical pixel grid.
    #[must_use]
    pub fn snap_value(&self, value: f32) -> f32 {
        if self.snap {
            (value * self.scale).round() / self.scale
        } else {
            value
        }
    }

    /// Physical pixels per logical unit.
    ///
    /// Geometry that must stay exactly one physical pixel thick (the window
    /// control strokes) needs this to convert its line width.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The current clip rectangle, if any.
    #[must_use]
    pub fn clip_rect(&self) -> Option<Rect> {
        self.clip_rects.last().map(|clip| clip.rect)
    }

    /// The current clip shape, including its rounded corners.
    #[must_use]
    pub fn clip_shape(&self) -> Option<ClipRect> {
        self.clip_rects.last().copied()
    }

    /// Start a clipped region. Clips intersect with the current one.
    pub fn begin_clip(&mut self, rect: Rect) {
        self.push_clip(rect, None);
    }

    /// Start a clip bounded by a rounded rectangle.
    ///
    /// The uber shader evaluates two rounded parent slots, so a nested
    /// `begin_clip` (an axis-aligned card) and `begin_clip_rounded` (a hero
    /// surface inside it) keep both shapes instead of degenerating to a
    /// scissor rectangle.
    pub fn begin_clip_rounded(&mut self, rect: Rect, radius: f32) {
        self.push_clip(rect, Some(radius));
    }

    fn push_clip(&mut self, rect: Rect, radius: Option<f32>) {
        let inherited = self.clip_rects.last().copied();
        let combined = match inherited {
            Some(existing) => existing.rect.intersect(&rect),
            None => rect,
        };
        let mut node = GpuClipNode::from_aabb([
            combined.origin.x,
            combined.origin.y,
            combined.max().x,
            combined.max().y,
        ]);

        // The rounded ancestors are kept in their own stack, not squeezed
        // into the single CPU mirror rectangle.
        //
        // The old code inherited the parent's radii through `ClipRect`, and a
        // plain axis-aligned clip in the middle reset them to zero — so a
        // third level lost the rounded shape of the first one entirely. The
        // GPU then fell back to a scissor rectangle and the rounded card grew
        // square corners.
        //
        // Slot 0 is the innermost rounded ancestor, slot 1 the one above it:
        // exactly what the shader's two-slot mask evaluates, and the same
        // order `unbound-world` uses.
        let mut slots = self.rounded_stack.last().copied().unwrap_or([None, None]);
        if let Some(value) = radius {
            if value > 0.0 {
                slots = [
                    Some(RoundedSlot {
                        rect,
                        radii: [value.max(0.0); 4],
                    }),
                    slots[0],
                ];
            }
        }
        if let Some(slot) = slots[0] {
            Self::write_rounded_slot(
                &mut node.rounded_clip_0,
                &mut node.radii_0,
                slot.rect,
                slot.radii,
            );
        }
        if let Some(slot) = slots[1] {
            Self::write_rounded_slot(
                &mut node.rounded_clip_1,
                &mut node.radii_1,
                slot.rect,
                slot.radii,
            );
        }

        let radii = match radius {
            Some(value) => [value.max(0.0); 4],
            None => slots[0]
                .filter(|slot| slot.rect == combined)
                .map_or([0.0; 4], |slot| slot.radii),
        };
        let index = self.scene.clips.len() as u32;
        self.scene.clips.push(node);
        self.clips.push(index);
        self.clip_rects.push(ClipRect {
            rect: combined,
            radii,
        });
        self.rounded_stack.push(slots);
    }

    fn write_rounded_slot(
        center: &mut [f32; 4],
        packed: &mut [u16; 4],
        rect: Rect,
        radii: [f32; 4],
    ) {
        let middle = rect.center();
        *center = [middle.x, middle.y, rect.size.x * 0.5, rect.size.y * 0.5];
        *packed = unbound_math::f16::pack_radii(radii);
    }

    /// End the innermost clip.
    pub fn end_clip(&mut self) {
        self.clips.pop();
        self.clip_rects.pop();
        self.rounded_stack.pop();
    }

    fn clip_index(&self) -> u32 {
        self.clips.last().copied().unwrap_or(CLIP_NONE)
    }

    fn apply_clip(&mut self, from: usize) {
        let index = self.clip_index();
        if index == CLIP_NONE {
            return;
        }
        for primitive in &mut self.scene.primitives[from..] {
            primitive.clip_idx = index;
        }
    }

    /// Resolve the hover / press state of an identity.
    #[must_use]
    pub fn visual(&self, node: u64, disabled: bool) -> Visual {
        if disabled {
            return Visual::Disabled;
        }
        if self.pressed == Some(node) {
            Visual::Pressed
        } else if self.hover == Some(node) {
            Visual::Hover
        } else {
            Visual::Idle
        }
    }

    /// Whether an identity has keyboard focus.
    #[must_use]
    pub fn focused(&self, node: u64) -> bool {
        self.focused == Some(node)
    }

    /// Paint a rounded rectangle and return its depth.
    #[allow(clippy::too_many_arguments)]
    pub fn rect(
        &mut self,
        rect: Rect,
        radius: f32,
        fill: u16,
        border: u16,
        border_width: f32,
        shadow: f32,
    ) -> u16 {
        let z = self.next_z();
        // Snap both edges, never the origin and the size independently: the
        // latter can round a 1.0-wide hairline to zero.
        let x0 = self.snap_value(rect.origin.x);
        let y0 = self.snap_value(rect.origin.y);
        let x1 = self.snap_value(rect.max().x);
        let y1 = self.snap_value(rect.max().y);
        let start = self.scene.primitives.len();
        self.scene.rect(
            x0,
            y0,
            x1 - x0,
            y1 - y0,
            radius,
            fill,
            border,
            border_width,
            shadow,
            z,
        );
        self.apply_clip(start);
        z
    }

    /// Paint a rounded rectangle at a given opacity.
    ///
    /// The primitive exists in every state; only its opacity changes. That
    /// keeps a control's primitive count constant, which is what lets a
    /// state change be repaired by patching one slice of the pool instead of
    /// re-uploading the whole scene.
    pub fn rect_faded(&mut self, rect: Rect, radius: f32, fill: u16, opacity: f32) -> u16 {
        let start = self.scene.primitives.len();
        let z = self.rect(rect, radius, fill, 0, 0.0, 0.0);
        self.fade(start, opacity);
        z
    }

    /// Paint a stroke at a given opacity.
    ///
    /// The companion of [`rect_faded`](Self::rect_faded) for the keyboard
    /// focus ring: the ring is always emitted, and losing focus fades it out
    /// instead of removing a primitive.
    pub fn stroke_faded(
        &mut self,
        rect: Rect,
        radius: f32,
        color: u16,
        width: f32,
        opacity: f32,
    ) -> u16 {
        let start = self.scene.primitives.len();
        let z = self.stroke(rect, radius, color, width);
        self.fade(start, opacity);
        z
    }

    /// Set the opacity of the primitive at `start`.
    fn fade(&mut self, start: usize, opacity: f32) {
        if let Some(primitive) = self.scene.primitives.get_mut(start) {
            primitive.z_and_opacity = unbound_gpu::data::GpuPrimitiveInstance::pack_z_opacity(
                primitive.depth_u16(),
                opacity,
            );
        }
    }

    /// Paint a shadow-only rounded rectangle.
    pub fn shadow(&mut self, rect: Rect, radius: f32, blur: f32) -> u16 {
        let z = self.next_z();
        let start = self.scene.primitives.len();
        let x0 = self.snap_value(rect.origin.x);
        let y0 = self.snap_value(rect.origin.y);
        let x1 = self.snap_value(rect.max().x);
        let y1 = self.snap_value(rect.max().y);
        self.scene
            .rect(x0, y0, x1 - x0, y1 - y0, radius, 0, 0, 0.0, blur, z);
        if let Some(prim) = self.scene.primitives.get_mut(start) {
            prim.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
                unbound_gpu::data::PrimType::Shadow,
                unbound_gpu::data::BlendMode::Normal,
                unbound_gpu::data::SubElement::Shadow as u16,
            );
        }
        self.apply_clip(start);
        z
    }

    /// Paint a hollow rounded rectangle.
    pub fn stroke(&mut self, rect: Rect, radius: f32, color: u16, width: f32) -> u16 {
        let z = self.next_z();
        let start = self.scene.primitives.len();
        let x0 = self.snap_value(rect.origin.x);
        let y0 = self.snap_value(rect.origin.y);
        let x1 = self.snap_value(rect.max().x);
        let y1 = self.snap_value(rect.max().y);
        self.scene
            .stroke(x0, y0, x1 - x0, y1 - y0, radius, color, z);
        if let Some(prim) = self.scene.primitives.get_mut(start) {
            prim.border_width = unbound_math::f16::f32_to_f16(width);
        }
        self.apply_clip(start);
        z
    }

    /// Paint a single-line label inside `rect`.
    ///
    /// The vertical placement follows the **baseline**, not the run's ink
    /// box: two labels of the same size have to share a baseline, otherwise a
    /// row of buttons shows `"Light"` a couple of pixels higher than
    /// `"Dark"` — the ink box depends on the string's own tallest glyph.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        text: &str,
        rect: Rect,
        size: f32,
        color: u16,
        align_x: AlignX,
        align_y: AlignY,
    ) -> Rect {
        let (quads, ink) = self.layout_text(text, rect, size, align_x, align_y);
        let z = self.next_z();
        let start = self.scene.primitives.len();
        for quad in &quads {
            self.scene.glyph(quad, color, z);
        }
        self.glyphs_emitted += quads.len();
        self.apply_clip(start);
        ink
    }

    /// Shape a run of text at its aligned position **without** painting it.
    ///
    /// The standard controls paint their own labels, so the shell needs the
    /// shaped glyphs at the exact position the painter would have used — the
    /// baseline, not the ink box, decides where a line sits.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_text(
        &mut self,
        text: &str,
        rect: Rect,
        size: f32,
        align_x: AlignX,
        align_y: AlignY,
    ) -> (Vec<GlyphQuad>, Rect) {
        let metrics = self.labels.metrics(text, size);
        let x = match align_x {
            AlignX::Start => rect.origin.x,
            AlignX::Center => rect.origin.x + (rect.size.x - metrics.width) * 0.5,
            AlignX::End => rect.max().x - metrics.width,
        };
        // The line box is what gets aligned: its top is the top of the box,
        // its bottom the bottom of the box, and its middle the middle.
        let y = match align_y {
            AlignY::Top => rect.origin.y,
            AlignY::Center => rect.origin.y + (rect.size.y - metrics.line_height()) * 0.5,
            AlignY::Bottom => rect.max().y - metrics.line_height(),
        };
        // Snap the run origin, not every glyph: the pen advances are integral
        // at phase 0, so one snapped origin keeps the whole line on the grid
        // without changing letter spacing.
        let x = self.snap_value(x);
        let y = self.snap_value(y);
        let quads = self.labels.layout(text, Vec2::new(x, y), size);
        let ink = quads_bounds(&quads).unwrap_or(Rect::new(x, y, metrics.width, size));
        (quads, ink)
    }

    /// Shape a styled run at its aligned position without painting it.
    ///
    /// Same alignment rules as [`Self::layout_text`], but with an explicit
    /// weight, slant and family: the font inspector's whole point is to show
    /// the difference between them.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_text_styled(
        &mut self,
        text: &str,
        rect: Rect,
        style: &TextStyle,
        align_x: AlignX,
        align_y: AlignY,
    ) -> (Vec<GlyphQuad>, Rect) {
        let metrics = self.labels.metrics_styled(text, style);
        let x = match align_x {
            AlignX::Start => rect.origin.x,
            AlignX::Center => rect.origin.x + (rect.size.x - metrics.width) * 0.5,
            AlignX::End => rect.max().x - metrics.width,
        };
        let y = match align_y {
            AlignY::Top => rect.origin.y,
            AlignY::Center => rect.origin.y + (rect.size.y - metrics.line_height()) * 0.5,
            AlignY::Bottom => rect.max().y - metrics.line_height(),
        };
        let x = self.snap_value(x);
        let y = self.snap_value(y);
        let quads = self.labels.layout_styled(text, Vec2::new(x, y), style);
        let ink = quads_bounds(&quads).unwrap_or(Rect::new(x, y, metrics.width, style.font_size));
        (quads, ink)
    }

    /// Paint a styled single-line label inside `rect`.
    #[allow(clippy::too_many_arguments)]
    pub fn text_styled(
        &mut self,
        text: &str,
        rect: Rect,
        style: &TextStyle,
        color: u16,
        align_x: AlignX,
        align_y: AlignY,
    ) -> Rect {
        let (quads, ink) = self.layout_text_styled(text, rect, style, align_x, align_y);
        self.glyphs(&quads, color);
        ink
    }

    /// The visual state of a standard control.
    ///
    /// The shell already resolved hover, press and focus from the router;
    /// the control layer only needs them in the shape it understands.
    #[must_use]
    pub fn control_state(&self, node: u64, disabled: bool, selected: bool) -> ControlState {
        let phase = if disabled {
            PointerPhase::Idle
        } else if self.pressed == Some(node) {
            PointerPhase::Pressed
        } else if self.hover == Some(node) {
            PointerPhase::Hovered
        } else {
            PointerPhase::Idle
        };
        ControlState {
            phase,
            focused: self.focused == Some(node),
            disabled,
            selected,
        }
    }

    /// Paint the output of a standard control.
    ///
    /// The control's primitives are rebased onto the canvas paint depth —
    /// keeping their relative order — and clipped like every other
    /// primitive. Its hit region and semantic node are registered with the
    /// canvas bookkeeping, so the shell keeps a single source of truth for
    /// the focus order.
    pub fn control_output(&mut self, node: u64, output: &WidgetOutput) {
        let start = self.scene.primitives.len();
        if node != 0 {
            self.record_span(node, start);
        }
        let payload_base = self.scene.payloads.len() as u32;
        for mut primitive in output.primitives.iter().copied() {
            primitive.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(self.next_z(), primitive.opacity());
            if matches!(
                primitive.prim_type(),
                unbound_gpu::data::PrimType::Glyph | unbound_gpu::data::PrimType::GlyphColor
            ) {
                primitive.extra_payload_idx += payload_base;
            }
            self.scene.primitives.push(primitive);
        }
        self.scene.payloads.extend(output.payloads.iter().copied());
        self.glyphs_emitted += output.payloads.len();
        self.apply_clip(start);

        if let Some(mut region) = output.hit.clone() {
            region.z = u32::from(self.z);
            if let Some(clip) = self.clip_shape() {
                region = region.with_clip(clip);
            }
            self.regions.push(region);
        }

        if let Some(a11y) = output.a11y.clone() {
            if output.focusable {
                self.focus_order.push(node);
            }
            self.semantics.upsert(node, a11y);
            self.a11y_nodes.push(node);
        }
    }

    /// Paint a real component's output and record that its emitter ran.
    ///
    /// This is the verified counterpart of [`control_output`](Self::control_output):
    /// the demo names the [`WidgetCatalogId`] it is emitting, so the frame's
    /// `emitted_widgets` set proves the genuine `unbound-widgets` emitter ran
    /// rather than a hand-drawn stand-in.
    pub fn widget_output(&mut self, id: WidgetCatalogId, node: u64, output: &WidgetOutput) {
        self.emitted_widgets.insert(id);
        self.control_output(node, output);
    }

    /// The real component emitters that ran this frame.
    #[must_use]
    pub fn emitted_widgets(&self) -> &FxHashSet<WidgetCatalogId> {
        &self.emitted_widgets
    }

    /// Read-only access to the live semantic tree built so far this frame.
    #[must_use]
    pub fn semantics(&self) -> &Semantics {
        &self.semantics
    }

    /// Shape a single-line label into glyph quads for a component emitter.
    pub fn shape(&mut self, text: &str, origin: Vec2, size: f32) -> Vec<GlyphQuad> {
        self.labels.layout(text, origin, size)
    }

    /// Paint a wrapped paragraph inside `rect`.
    pub fn text_wrapped(&mut self, text: &str, rect: Rect, size: f32, color: u16) -> Rect {
        let x = self.snap_value(rect.origin.x);
        let y = self.snap_value(rect.origin.y);
        let quads = self
            .labels
            .layout_wrapped(text, Vec2::new(x, y), size, rect.size.x.max(1.0));
        let ink = quads_bounds(&quads).unwrap_or(Rect::new(x, y, 0.0, 0.0));
        let z = self.next_z();
        let start = self.scene.primitives.len();
        for quad in &quads {
            self.scene.glyph(quad, color, z);
        }
        self.glyphs_emitted += quads.len();
        self.apply_clip(start);
        ink
    }

    /// Paint glyphs that were laid out elsewhere in scene space.
    pub fn glyphs(&mut self, quads: &[GlyphQuad], color: u16) {
        let z = self.next_z();
        let start = self.scene.primitives.len();
        for quad in quads {
            self.scene.glyph(quad, color, z);
        }
        self.glyphs_emitted += quads.len();
        self.apply_clip(start);
    }

    /// Paint a quadratic Bézier segment.
    pub fn curve(&mut self, p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], width: f32, color: u16) {
        let z = self.next_z();
        let start = self.scene.primitives.len();
        self.scene.curve(p0, p1, p2, width, color, z);
        self.apply_clip(start);
    }

    /// A hairline divider.
    pub fn divider(&mut self, x: f32, y: f32, width: f32) {
        self.rect(
            Rect::new(x, y, width, 1.0),
            0.0,
            self.palette.divider,
            0,
            0.0,
            0.0,
        );
    }

    /// Register a clickable region plus its semantic node.
    #[allow(clippy::too_many_arguments)]
    pub fn control(
        &mut self,
        node: u64,
        rect: Rect,
        role: Role,
        label: impl Into<String>,
        cursor: CursorKind,
        disabled: bool,
        selected: bool,
    ) {
        self.control_on_layer(node, rect, role, label, cursor, disabled, selected, 1);
    }

    /// Register a clickable region on an explicit portal layer.
    #[allow(clippy::too_many_arguments)]
    pub fn control_on_layer(
        &mut self,
        node: u64,
        rect: Rect,
        role: Role,
        label: impl Into<String>,
        cursor: CursorKind,
        disabled: bool,
        selected: bool,
        layer: u8,
    ) {
        let mut region = HitRegion::new(node, layer, u32::from(self.z), rect).with_cursor(cursor);
        if let Some(clip) = self.clip_shape() {
            region = region.with_clip(clip);
        }
        self.regions.push(region);

        let mut a11y = A11yNode::new(role, label, rect);
        if !disabled {
            a11y = a11y.clickable();
            self.focus_order.push(node);
        }
        a11y.disabled = disabled;
        a11y.selected = selected;
        self.semantics.upsert(node, a11y);
        self.a11y_nodes.push(node);
    }

    /// Register a non-interactive semantic node.
    pub fn semantic(&mut self, role: Role, label: impl Into<String>, rect: Rect) -> u64 {
        self.static_ids += 1;
        let node = 0x00f0_0000_0000_0000 | self.static_ids;
        let mut a11y = A11yNode::new(role, label, rect);
        a11y.focusable = false;
        self.semantics.upsert(node, a11y);
        self.a11y_nodes.push(node);
        node
    }

    /// Register a heading semantic node.
    pub fn heading(&mut self, label: impl Into<String>, rect: Rect, level: u32) -> u64 {
        self.static_ids += 1;
        let node = 0x00f0_0000_0000_0000 | self.static_ids;
        let mut a11y = A11yNode::new(Role::Heading, label, rect);
        a11y.level = level;
        self.semantics.upsert(node, a11y);
        self.a11y_nodes.push(node);
        node
    }

    /// Register a demo part that is not a standard widget.
    #[allow(clippy::too_many_arguments)]
    pub fn demo_control(
        &mut self,
        page: u8,
        card: u8,
        part: u16,
        rect: Rect,
        role: Role,
        label: impl Into<String>,
        selected: bool,
    ) -> u64 {
        let node = target::id(Target::Demo(page, card, part));
        self.control(
            node,
            rect,
            role,
            label,
            CursorKind::Pointer,
            false,
            selected,
        );
        node
    }

    /// Glyphs emitted so far.
    #[must_use]
    pub fn glyph_count(&self) -> usize {
        self.glyphs_emitted
    }

    /// Declare a region whose wheel gestures are handled by a demo.
    pub fn scroll_zone(&mut self, rect: Rect, tag: u8) {
        self.scroll_zones.push((rect, tag));
    }

    /// Declare a horizontal range whose value follows the pointer.
    pub fn range(&mut self, node: u64, rect: Rect, min: f32, max: f32, step: f32) {
        self.ranges.push(RangeZone {
            node,
            rect,
            min,
            max,
            step,
        });
    }

    /// Finish painting and hand the interaction data back.
    #[must_use]
    pub fn finish(mut self) -> CanvasOutput {
        self.semantics.set_children(ROOT, &self.a11y_nodes);
        self.semantics.set_focus(self.focused.unwrap_or(ROOT));
        CanvasOutput {
            regions: self.regions,
            semantics: self.semantics,
            focus_order: self.focus_order,
            scroll_zones: self.scroll_zones,
            ranges: self.ranges,
            glyph_count: self.glyphs_emitted,
            node_spans: self.node_spans,
            emitted_widgets: self.emitted_widgets,
        }
    }
}

/// Ink bounds of a glyph run.
#[must_use]
pub fn quads_bounds(quads: &[GlyphQuad]) -> Option<Rect> {
    let first = quads.first()?;
    let mut bounds = first.rect;
    for quad in &quads[1..] {
        bounds = bounds.union(&quad.rect);
    }
    Some(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gallery::{palette, Gallery, Labels};

    /// A canvas over an empty scene.
    fn canvas<'a>(
        scene: &'a mut Gallery,
        labels: &'a mut Labels,
        palette: &'a Palette,
    ) -> Canvas<'a> {
        Canvas::new(
            scene,
            labels,
            palette,
            (400.0, 400.0),
            None,
            None,
            None,
            1.0,
            false,
        )
    }

    /// The corner radii of the clip node at `index`.
    fn radii_of(node: &GpuClipNode, slot: usize) -> [f32; 4] {
        let packed = if slot == 0 {
            node.radii_0
        } else {
            node.radii_1
        };
        unbound_math::f16::unpack_radii(packed)
    }

    /// The rounded rectangle of the clip node at `index`.
    fn rounded_of(node: &GpuClipNode, slot: usize) -> [f32; 4] {
        if slot == 0 {
            node.rounded_clip_0
        } else {
            node.rounded_clip_1
        }
    }

    #[test]
    fn a_third_level_plain_clip_keeps_the_rounded_ancestor() {
        let mut scene = Gallery::new();
        let mut labels = Labels::new();
        let palette = palette();
        let mut canvas = canvas(&mut scene, &mut labels, &palette);

        let outer = Rect::new(10.0, 10.0, 120.0, 120.0);
        canvas.begin_clip_rounded(outer, 12.0);
        canvas.begin_clip(Rect::new(20.0, 20.0, 80.0, 80.0));
        canvas.begin_clip(Rect::new(30.0, 30.0, 40.0, 40.0));

        let node = canvas.scene.clips.last().expect("three clip nodes");
        assert_eq!(
            radii_of(node, 0),
            [12.0; 4],
            "the innermost clip lost the rounded ancestor"
        );
        assert_eq!(
            rounded_of(node, 0),
            [70.0, 70.0, 60.0, 60.0],
            "the rounded slot must describe the outer rounded rectangle"
        );
    }

    #[test]
    fn a_nested_rounded_clip_keeps_both_shapes() {
        let mut scene = Gallery::new();
        let mut labels = Labels::new();
        let palette = palette();
        let mut canvas = canvas(&mut scene, &mut labels, &palette);

        canvas.begin_clip_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), 12.0);
        canvas.begin_clip_rounded(Rect::new(10.0, 10.0, 40.0, 40.0), 4.0);

        let node = canvas.scene.clips.last().expect("two clip nodes");
        assert_eq!(
            radii_of(node, 0),
            [4.0; 4],
            "the inner shape is the first slot"
        );
        assert_eq!(
            radii_of(node, 1),
            [12.0; 4],
            "the outer shape is the second slot"
        );
    }
}
