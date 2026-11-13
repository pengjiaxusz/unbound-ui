//! The dogfooded application shell.
//!
//! `GalleryApp` owns navigation, environment switches and demo state. It draws
//! the custom title bar, the collapsible NavigationView, the page viewport and
//! the `ExampleCard` composition, so the showcase exercises the same
//! primitives a real application would use.

use accesskit::Role;
use unbound_anim::{Spring, SpringPreset};
use unbound_headless::{Frame, FrameBuilder, NodeKind};
use unbound_input::CursorKind;
use unbound_layout::LayoutStats;
use unbound_math::{Rect, Vec2};
use unbound_platform::{Modifiers, PlatformKey};
use unbound_widgets::diff::{diff_primitives, SceneDiff};

use crate::canvas::{AlignX, AlignY, Canvas, RangeZone};
use crate::gallery::{self, Labels, Palette};
use crate::layout_tree::{CardLayout, KnobRow, PageLayout, ShellLayout, ShellTree};
use crate::pages::{self, CategoryId, DemoCtx, KnobSpec, PageId};
use crate::state::{GalleryState, ThemeMode, ZOOM_STEPS};
use crate::target::{self, Target, ROOT};
use crate::widgets::{self, Icon};

const NAV_EXPANDED: f32 = 248.0;
const NAV_COLLAPSED: f32 = 68.0;

/// A window action requested by the custom title bar.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WindowCommand {
    /// Minimise the window.
    Minimize,
    /// Toggle the maximised state.
    ToggleMaximize,
    /// Close the window.
    Close,
}

/// The geometry of the page's vertical scrollbar.
///
/// The thumb is a real control: it can be dragged, and the track can be
/// clicked to jump. A scrollbar that only exists as a painted rectangle is
/// decoration, not a control.
#[derive(Copy, Clone, Debug)]
struct Scrollbar {
    track: Rect,
    thumb_h: f32,
    max_scroll: f32,
}

impl Scrollbar {
    /// The thumb's top edge for a scroll position.
    fn thumb_top(&self, scroll: f32) -> f32 {
        let t = (scroll / self.max_scroll).clamp(0.0, 1.0);
        self.track.origin.y + (self.track.size.y - self.thumb_h) * t
    }

    /// The scroll position for a thumb top edge.
    fn scroll_for(&self, thumb_top: f32) -> f32 {
        let span = (self.track.size.y - self.thumb_h).max(1.0);
        let t = ((thumb_top - self.track.origin.y) / span).clamp(0.0, 1.0);
        t * self.max_scroll
    }
}

/// The page's scrollbar, or `None` when the page fits.
fn content_scrollbar(layout: &ShellLayout, page: &PageLayout) -> Option<Scrollbar> {
    let max_scroll = (page.content_height - layout.content.size.y + 48.0).max(0.0);
    if max_scroll <= 1.0 {
        return None;
    }
    let track = Rect::new(
        layout.content.max().x - 10.0,
        layout.content.origin.y + 8.0,
        6.0,
        (layout.content.size.y - 16.0).max(1.0),
    );
    let thumb_h = (track.size.y * layout.content.size.y / page.content_height.max(1.0))
        .clamp(28.0, track.size.y);
    Some(Scrollbar {
        track,
        thumb_h,
        max_scroll,
    })
}

/// The navigation pane's scrollbar, or `None` when the tree fits.
fn nav_scrollbar(layout: &ShellLayout) -> Option<Scrollbar> {
    let tree = nav_tree_rect(layout);
    let max_scroll = (nav_height(layout) - tree.size.y).max(0.0);
    if max_scroll <= 1.0 || tree.size.y <= 1.0 {
        return None;
    }
    let track = Rect::new(layout.nav.max().x - 10.0, tree.origin.y, 6.0, tree.size.y);
    let thumb_h =
        (track.size.y * tree.size.y / (tree.size.y + max_scroll)).clamp(28.0, track.size.y);
    Some(Scrollbar {
        track,
        thumb_h,
        max_scroll,
    })
}

/// The application.
pub struct GalleryApp {
    pub state: GalleryState,
    pub scene: gallery::Gallery,
    pub labels: Labels,
    pub layout: ShellLayout,
    /// The focus order the last rebuild produced.
    ///
    /// The runtime adopts it into its `FocusManager` right after `view`, so
    /// this is an output, never a second source of truth.
    pub focus_order: Vec<u64>,
    /// The hit regions the last rebuild produced.
    ///
    /// The runtime adopts them into its `Router` right after `view`; the
    /// shell no longer keeps a router of its own.
    pub regions: Vec<unbound_input::HitRegion>,
    pub ranges: Vec<RangeZone>,
    pub scroll_zones: Vec<(Rect, u8)>,
    pub semantics: unbound_a11y::Semantics,
    /// The layout tree the shell's geometry comes from.
    ///
    /// The tree is the single source of truth for the shell's rectangles:
    /// the title bar, the navigation pane and the page are nodes that
    /// `LayoutEngine` solves, not arithmetic in this file.
    tree: ShellTree,
    page_layout: Option<PageLayout>,
    nav_spring: Spring,
    /// The upload generation of the current scene.
    ///
    /// The generation is bumped by every rebuild, and *only* by a rebuild:
    /// the GPU skips the upload when the generation did not move, so a
    /// host that forgets to bump it shows a stale frame. Tying the counter
    /// to the rebuild itself makes that mistake impossible.
    generation: u64,
    /// The clock the glyph atlas evicts on.
    ///
    /// The atlas drops the least recently *used* glyph, and "used" is measured
    /// in frames. The clock therefore has to move on every rebuild — otherwise
    /// every slot looks equally old and the atlas evicts the same glyphs
    /// forever. It is deliberately separate from the upload generation: the
    /// generation only moves when the scene's *shape* changed, and the host
    /// owns it in the external-diff mode.
    atlas_clock: u64,
    last_revision: u64,
    last_hover: Option<u64>,
    last_pressed: Option<u64>,
    last_focus: Option<u64>,
    last_viewport: (f32, f32),
    nav_max_scroll: f32,
    pointer: Vec2,
    panning: Option<Vec2>,
    dragging_range: Option<u64>,
    effective_scale: f32,
    pending_window: Option<WindowCommand>,
    /// Mirrors the native window's maximised state for the title-bar icon.
    pub window_maximized: bool,
    /// The page's scrollbar, when the page is taller than the viewport.
    scrollbar: Option<Scrollbar>,
    /// The grab offset inside the scrollbar thumb, while it is dragged.
    scroll_drag: Option<f32>,
    /// The navigation pane's scrollbar, when the tree is taller than the pane.
    nav_scrollbar: Option<Scrollbar>,
    /// The grab offset inside the navigation scrollbar thumb, while dragged.
    nav_scroll_drag: Option<f32>,
    /// The virtual-list scrollbar drag: `(card, grab offset)`.
    list_scroll_drag: Option<(usize, f32)>,
    /// The primitive ranges that changed in the last rebuild.
    ///
    /// An empty list means "the whole scene changed" and the renderer has to
    /// upload it. A non-empty list means "only these slices changed", which
    /// is what turns a hover into a handful of uploaded bytes.
    dirty_ranges: Vec<(u32, u32)>,
    /// The inputs the layout depends on.
    ///
    /// A hover, a press or a focus change moves nothing, so the layout is
    /// not re-solved for them: the frame's layout cost stays at zero.
    layout_key: LayoutKey,
    /// Whether a host runtime owns the frame diff.
    ///
    /// The showcase is driven by `unbound_app::AppRuntime`, which diffs the
    /// primitive list once per frame. Re-diffing here as well would scan the
    /// whole scene twice for the same answer.
    external_diff: bool,
    /// The layout statistics of the current frame.
    frame_layout_stats: LayoutStats,
    /// The primitive range each control owns.
    ///
    /// The ranges are what tie a dirty slice back to the control that owns
    /// it: a hover patch has to stay inside one control's span.
    node_spans: rustc_hash::FxHashMap<u64, (u32, u32)>,
    /// The real widget emitters that ran this frame, for the coverage gate.
    emitted_widgets: rustc_hash::FxHashSet<unbound_widgets::WidgetCatalogId>,
    /// Where the input method should put its candidate window.
    ///
    /// The platform only needs an approximate caret; the click position is
    /// accurate enough and avoids threading a caret rectangle through every
    /// page.
    ime_caret: Vec2,
    /// A focus request raised by a click, a navigation or an a11y action.
    ///
    /// The runtime owns the focus manager, so the shell only records the
    /// request and hands it over after the reduction.
    pending_focus: Option<u64>,
    /// A blur request raised by an accessibility action.
    pending_blur: bool,
}

/// The interaction state the runtime resolved for this frame.
///
/// The shell no longer routes the pointer itself: the runtime's `Router`
/// picks the hovered / pressed node, and its `FocusManager` owns the focused
/// one. Painting a frame therefore takes a read-only snapshot instead of a
/// second router.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Interaction {
    /// The node under the pointer.
    pub hovered: Option<u64>,
    /// The node the pointer is pressing.
    pub pressed: Option<u64>,
    /// The focused node.
    pub focused: Option<u64>,
}

/// The inputs the shell's layout depends on.
///
/// Two frames with the same key have the same geometry, so the second one
/// does not have to be solved again.
#[derive(Copy, Clone, PartialEq, Eq)]
struct LayoutKey {
    width: u32,
    height: u32,
    nav_width: u32,
    page: u8,
    /// Which code panels are open.
    code_mask: u64,
}

impl Default for GalleryApp {
    fn default() -> Self {
        Self::new()
    }
}

impl GalleryApp {
    /// Create the gallery at its Home page.
    #[must_use]
    pub fn new() -> Self {
        let mut tree = ShellTree::new();
        let layout = tree.solve(
            &GalleryState::default(),
            Rect::new(0.0, 0.0, 1280.0, 800.0),
            NAV_EXPANDED,
        );
        let mut nav_spring = Spring::new(SpringPreset::Snappy, NAV_EXPANDED);
        nav_spring.set_target(NAV_EXPANDED);
        Self {
            state: GalleryState::default(),
            scene: gallery::Gallery::new(),
            labels: Labels::new(),
            layout,
            focus_order: Vec::new(),
            regions: Vec::new(),
            ranges: Vec::new(),
            scroll_zones: Vec::new(),
            semantics: unbound_a11y::Semantics::new(ROOT),
            tree,
            page_layout: None,
            nav_spring,
            generation: 0,
            atlas_clock: 0,
            last_revision: 0,
            last_hover: None,
            last_pressed: None,
            last_focus: None,
            last_viewport: (0.0, 0.0),
            nav_max_scroll: 0.0,
            pointer: Vec2::ZERO,
            panning: None,
            dragging_range: None,
            effective_scale: 1.0,
            pending_window: None,
            window_maximized: false,
            scrollbar: None,
            scroll_drag: None,
            nav_scrollbar: None,
            nav_scroll_drag: None,
            list_scroll_drag: None,
            dirty_ranges: Vec::new(),
            layout_key: LayoutKey {
                width: 0,
                height: 0,
                nav_width: 0,
                page: 0,
                code_mask: 0,
            },
            external_diff: false,
            frame_layout_stats: LayoutStats::default(),
            node_spans: rustc_hash::FxHashMap::default(),
            emitted_widgets: rustc_hash::FxHashSet::default(),
            ime_caret: Vec2::new(24.0, 60.0),
            pending_focus: None,
            pending_blur: false,
        }
    }

    /// Mirror the native window's maximised state for the title-bar icon.
    pub fn set_window_maximized(&mut self, maximized: bool) {
        self.window_maximized = maximized;
    }

    /// Set the physical pixels per logical unit (`OS scale × app zoom`).
    ///
    /// Everything painted afterwards lays out in logical units but rasterises
    /// glyphs and snaps rectangles for this scale.
    pub fn set_effective_scale(&mut self, scale: f32) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        self.effective_scale = scale.clamp(0.5, 4.0);
    }

    /// The physical pixels per logical unit currently in use.
    #[must_use]
    pub fn effective_scale(&self) -> f32 {
        self.effective_scale
    }

    /// Take the window action requested by the last press, if any.
    pub fn take_window_command(&mut self) -> Option<WindowCommand> {
        self.pending_window.take()
    }

    /// Let a host runtime own the frame diff.
    ///
    /// [`crate::runtime_app::GalleryRuntime`] installs this so the scene is
    /// diffed exactly once, by the runtime, instead of once here and once
    /// there.
    pub fn set_external_diff(&mut self, external: bool) {
        self.external_diff = external;
    }

    /// Whether `point` is draggable title-bar background.
    ///
    /// The runtime resolves the hover, so the caller passes it in: a point
    /// over a window button is a click, not a drag.
    #[must_use]
    pub fn is_titlebar_drag(&self, point: Vec2, hovered: Option<u64>) -> bool {
        self.layout.top_bar.contains_point(point) && hovered.is_none()
    }

    /// The palette currently used by the shell.
    #[must_use]
    pub fn palette(&self) -> Palette {
        gallery::palette()
    }

    /// Navigate and close transient surfaces.
    pub fn navigate(&mut self, page: PageId) {
        self.state.navigate(page);
        self.state.search_focused = false;
        self.state.search_query.clear();
        self.state.demo.modal_open = false;
        self.state.demo.dropdown_open = false;
        self.pending_focus = Some(target::id(Target::NavPage(page.index() as u8)));
    }

    /// Apply an accessibility action exactly like the equivalent pointer or
    /// keyboard gesture.
    ///
    /// The OS adapter hands actions back in the engine's vocabulary, so the
    /// reducer path is the same one a click takes — an assistive-technology
    /// user cannot reach a code path a mouse user cannot.
    pub fn apply_a11y(
        &mut self,
        node: u64,
        action: unbound_a11y::A11yAction,
        value: Option<String>,
    ) {
        use unbound_a11y::A11yAction;
        match action {
            A11yAction::Click => {
                if let Some(target) = target::decode(node) {
                    let position = self.pointer;
                    self.click_target(target, position);
                }
            }
            A11yAction::Focus => {
                if target::decode(node).is_some() {
                    self.pending_focus = Some(node);
                    self.state.bump();
                }
            }
            A11yAction::Blur => {
                self.pending_blur = true;
                self.state.bump();
            }
            A11yAction::SetValue => {
                if let Some(value) = value {
                    if self.state.search_focused {
                        self.state.search_query = value;
                        self.state.bump();
                    } else if self.state.demo.typing {
                        self.state.demo.text = value;
                        self.state.bump();
                    }
                }
            }
            // Increment / decrement need the control's range, which lives in
            // the page description; the demo has no such descriptor yet.
            _ => {}
        }
    }

    /// Whether the scene has to be rebuilt for this interaction snapshot.
    ///
    /// A hover, a press or a focus change repaints a control but moves
    /// nothing, so a frame whose snapshot is unchanged reuses the previous
    /// scene instead of rebuilding it. `force` comes from the runtime: an
    /// explicit `AppRuntime::invalidate` rebuilds even when nothing moved.
    #[must_use]
    pub fn needs_rebuild(&self, viewport: (f32, f32), inter: &Interaction, force: bool) -> bool {
        force
            || self.last_revision != self.state.revision
            || self.last_hover != inter.hovered
            || self.last_pressed != inter.pressed
            || self.last_focus != inter.focused
            || self.last_viewport != viewport
            || !self.nav_spring.is_settled()
    }

    /// The primitive ranges that changed in the last rebuild.
    ///
    /// An empty list means "the whole scene changed". A non-empty list names
    /// the slices the renderer has to patch into the pool.
    #[must_use]
    pub fn dirty_ranges(&self) -> &[(u32, u32)] {
        &self.dirty_ranges
    }

    /// The host uploaded the pending slices: drop them.
    ///
    /// `dirty_ranges` accumulates across every rebuild between two uploads, so
    /// only the host knows when the data actually reached the GPU.
    pub fn mark_uploaded(&mut self) {
        self.dirty_ranges.clear();
    }

    /// The layout statistics of the current frame.
    ///
    /// `nodes_visited` is zero when the frame did not have to be laid out.
    #[must_use]
    pub fn layout_stats(&self) -> LayoutStats {
        self.frame_layout_stats
    }

    /// The primitive range a control owns, `[start, end)`.
    ///
    /// Hand-drawn shell controls and emitted widgets both register one, so a
    /// dirty slice can always be attributed to the control it belongs to.
    #[must_use]
    pub fn span_of(&self, node: u64) -> Option<(u32, u32)> {
        self.node_spans.get(&node).copied()
    }

    /// Every control's primitive range, keyed by node identity.
    #[must_use]
    pub fn node_spans(&self) -> &rustc_hash::FxHashMap<u64, (u32, u32)> {
        &self.node_spans
    }

    /// The real widget emitters that ran this frame.
    ///
    /// The Showcase coverage gate reads this to prove every catalog component
    /// is hosted by a genuine demo, not a hand-drawn imitation.
    #[must_use]
    pub fn emitted_widgets(&self) -> &rustc_hash::FxHashSet<unbound_widgets::WidgetCatalogId> {
        &self.emitted_widgets
    }

    /// The geometry of the page as the layout tree solved it.
    #[must_use]
    pub fn page_layout(&self) -> Option<&PageLayout> {
        self.page_layout.as_ref()
    }

    /// The number of cards that are relayout boundaries.
    ///
    /// A card's size is fixed, so nothing inside it can change it: the page
    /// can be re-solved without touching the card.
    #[must_use]
    pub fn relayout_boundary_count(&self) -> usize {
        self.tree.card_boundary_count()
    }

    /// The upload generation of the current scene.
    ///
    /// The GPU skips the upload when the generation did not move, so the
    /// host has to advance it exactly when the scene was rebuilt. Tying it
    /// to the rebuild is what makes a click show up on screen: the host
    /// cannot "forget" to bump it.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The inputs the layout depends on.
    fn layout_key(&self, viewport: (f32, f32), nav_width: f32) -> LayoutKey {
        let page = self.state.active_page;
        let mut code_mask = 0u64;
        for (index, _) in page.examples().iter().enumerate() {
            if self.state.code_open(page, index as u8) {
                code_mask |= 1u64 << index;
            }
        }
        LayoutKey {
            width: viewport.0.to_bits(),
            height: viewport.1.to_bits(),
            nav_width: nav_width.to_bits(),
            page: page.index() as u8,
            code_mask,
        }
    }

    /// Rebuild unconditionally.
    pub fn rebuild(&mut self, viewport: (f32, f32), inter: Interaction) {
        let (width, height) = viewport;
        let viewport_rect = Rect::new(0.0, 0.0, width.max(320.0), height.max(240.0));
        let nav_width = self.nav_spring.value.clamp(NAV_COLLAPSED, NAV_EXPANDED);
        let palette = gallery::palette();
        let hover = inter.hovered;
        let pressed = inter.pressed;
        // The tree owns the geometry: the title bar, the navigation pane and
        // the page are solved, not measured by hand.
        //
        // The layout is only solved when something that *moves* changed. A
        // hover, a press or a focus change repaints a control but moves
        // nothing, so it must not pay for a layout pass.
        let key = self.layout_key(viewport, nav_width);
        if key != self.layout_key {
            self.layout_key = key;
            self.layout = self.tree.solve(&self.state, viewport_rect, nav_width);
            self.frame_layout_stats = self.tree.stats();
        } else {
            self.frame_layout_stats = LayoutStats::default();
        }
        let layout = self.layout;
        let nav_max_scroll = nav_height(&layout) - nav_tree_rect(&layout).size.y;
        self.state.nav_scroll = self.state.nav_scroll.clamp(0.0, nav_max_scroll.max(0.0));
        let content_height = self.tree.content_height();
        self.state.content_scroll = self.state.content_scroll.clamp(
            0.0,
            (content_height - layout.content.size.y + 48.0).max(0.0),
        );
        let page_layout = self
            .tree
            .page_layout(&self.state, self.state.content_scroll);

        // The previous scene is kept for one frame: the diff against the
        // new one tells the renderer which primitives actually changed.
        let previous = std::mem::replace(&mut self.scene, gallery::Gallery::new());
        let focused = inter.focused;
        let scale = self.effective_scale;
        let pixel_snap = self.state.pixel_snap;
        // The atlas clock moves on every rebuild, independent of the upload
        // generation: the generation only moves when the scene's *shape*
        // changed, while the eviction order has to advance on every frame.
        self.atlas_clock = self.atlas_clock.wrapping_add(1);
        let atlas_clock = self.atlas_clock;
        let GalleryApp {
            scene,
            labels,
            state,
            ..
        } = self;
        labels.set_raster_scale(scale);
        // Glyph slots are evicted by least-recently-used frame. Without this
        // the atlas clock never advanced and every slot looked equally old,
        // so a full atlas evicted the same top-left glyphs forever.
        labels.cache.begin_frame(atlas_clock);
        // The shell keeps the four sub-pixel phases: a glyph's raster is
        // generated for the phase implied by its *absolute* position, and the
        // quad is placed on the pixel grid around it. Forcing phase 0 would
        // put every glyph up to a pixel away from where it belongs, which is
        // what made the letter spacing jitter.
        labels.set_snap_phases(false);
        let mut canvas = Canvas::new(
            scene,
            labels,
            &palette,
            (layout.viewport.size.x, layout.viewport.size.y),
            focused,
            hover,
            pressed,
            scale,
            pixel_snap,
        );
        draw_background(&mut canvas, &layout);
        draw_top_bar(&mut canvas, state, &layout, self.window_maximized);
        draw_nav(&mut canvas, state, &layout);
        draw_page(&mut canvas, state, &layout, &page_layout);
        draw_hud(&mut canvas, state, &layout, page_layout.cards.len());
        draw_search_overlay(&mut canvas, state, &layout);
        let output = canvas.finish();
        let scrollbar = content_scrollbar(&layout, &page_layout);
        let scene_len = scene.primitives.len() as u32;
        let draw_calls = 4 + u32::from(!scene.clips.is_empty());
        let _ = output.glyph_count;

        // How much of the scene has to travel decides how the frame is
        // uploaded: a paint-only change patches its slices under the *same*
        // generation, and the renderer leaves the rest of the pool alone.
        //
        // A frame can rebuild twice (the event handler, then `tick`), and the
        // second diff only knows what changed since the *first* rebuild. The
        // pending slices therefore accumulate until the host has actually
        // uploaded them (`mark_uploaded`); overwriting them here lost every
        // paint change made before the frame's animation tick.
        let diff = diff_scenes(&previous, scene);
        if self.external_diff {
            // The host runtime diffs the frame itself; drop the slices the
            // shell would otherwise have accumulated for its own uploader.
            self.dirty_ranges.clear();
        } else {
            let bumps_generation = diff.bumps_generation();
            match diff {
                // Nothing changed: the slices still pending from an earlier
                // rebuild in this frame must survive.
                SceneDiff::Identical => {}
                SceneDiff::Ranges(ranges) => merge_ranges(&mut self.dirty_ranges, ranges),
                // The shape changed, so the generation has to move and the whole
                // scene goes up again.
                SceneDiff::Whole => {
                    self.dirty_ranges.clear();
                    self.generation = self.generation.wrapping_add(1);
                }
            }
            let stride = core::mem::size_of::<unbound_gpu::data::GpuPrimitiveInstance>() as u32;
            self.state.demo.upload_bytes = if bumps_generation {
                (scene.primitives.len() as u32).saturating_mul(stride)
            } else {
                self.dirty_ranges
                    .iter()
                    .map(|(start, end)| (end - start) * stride)
                    .sum()
            };
        }
        self.state.demo.primitive_count = scene_len;
        self.state.demo.draw_calls = draw_calls;
        self.regions = output.regions;
        self.focus_order = output.focus_order;
        self.ranges = output.ranges;
        self.scroll_zones = output.scroll_zones;
        self.semantics = output.semantics;
        self.node_spans = output.node_spans;
        self.emitted_widgets = output.emitted_widgets;
        self.layout = layout;
        self.page_layout = Some(page_layout);
        self.scrollbar = scrollbar;
        self.nav_scrollbar = nav_scrollbar(&layout);
        self.nav_max_scroll = nav_max_scroll.max(0.0);
        self.last_revision = self.state.revision;
        self.last_hover = hover;
        self.last_pressed = pressed;
        self.last_focus = focused;
        self.last_viewport = viewport;
    }

    /// Advance time-driven demos.
    pub fn tick(&mut self, dt: f32) -> bool {
        let target = if self.state.nav_expanded {
            NAV_EXPANDED
        } else {
            NAV_COLLAPSED
        };
        self.nav_spring.set_target(target);
        let nav_moving = self.nav_spring.step(dt);
        let demo_moving = self.state.tick(dt);
        if nav_moving || demo_moving {
            self.state.bump();
        }
        nav_moving || demo_moving
    }

    /// Move the pointer (logical pixels).
    ///
    /// Only the drags the shell is already running follow the pointer; the
    /// hover itself is the runtime's, so nothing here has to route.
    pub fn pointer_move(&mut self, position: Vec2) {
        self.pointer = position;
        if let Some(grab) = self.scroll_drag {
            if let Some(bar) = self.scrollbar {
                let scroll = bar.scroll_for(position.y - grab);
                if (scroll - self.state.content_scroll).abs() > 0.01 {
                    self.state.content_scroll = scroll;
                    self.state.bump();
                }
            }
        }
        if let Some(grab) = self.nav_scroll_drag {
            if let Some(bar) = self.nav_scrollbar {
                let scroll = bar.scroll_for(position.y - grab);
                if (scroll - self.state.nav_scroll).abs() > 0.01 {
                    self.state.nav_scroll = scroll;
                    self.state.bump();
                }
            }
        }
        if let Some((card, grab)) = self.list_scroll_drag {
            if let Some(bar) = self.list_scrollbar(card) {
                let scroll = bar.scroll_for(position.y - grab);
                if (scroll - self.state.demo.list_scroll[card]).abs() > 0.01 {
                    self.state.demo.list_scroll[card] = scroll;
                    self.state.bump();
                }
            }
        }
        // A drag only exists while the shell started one on the press, so the
        // snapshot does not have to be consulted: the `Option` *is* the
        // capture.
        if let Some(node) = self.dragging_range {
            self.update_range(node, position);
        }
        if self.panning.is_some() {
            let zoom = self.state.demo.canvas_zoom.max(0.01);
            let delta = position - self.panning.unwrap_or(position);
            self.state.demo.canvas_pan -= delta / zoom;
            self.panning = Some(position);
            self.state.bump();
        }
        if let Some((start, _)) = self.state.demo.marquee {
            self.state.demo.marquee = Some((start, position));
            self.state.bump();
        }
        if self.state.demo.drag_active.is_some() {
            self.state.demo.drag_position = Some(position);
            self.state.bump();
        }
    }

    /// Press the pointer.
    ///
    /// Only the controls that *capture* the pointer act here: the
    /// scrollbar, the sliders and the draggable demo stages. Every
    /// discrete control waits for the release, so a press that is dragged
    /// off the control never activates it.
    pub fn pointer_down(&mut self, position: Vec2, target: Option<u64>) {
        self.pointer = position;
        // Clicking anywhere but the FPS input commits the pending value, the
        // same way a native spin box does on blur.
        if self.state.fps_focused && target != Some(target::id(Target::FpsInput)) {
            self.state.commit_fps_input();
        }
        if let Some(node) = target {
            self.pending_focus = Some(node);
            if let Some(target) = target::decode(node) {
                self.begin_press_target(target, position);
            }
        }
    }

    /// Release the pointer.
    ///
    /// The release itself never activates a control: the runtime reports a
    /// separate [`AppEvent::Click`](unbound_app::AppEvent::Click) only when
    /// the press landed on the same control and never became a drag.
    pub fn pointer_up(&mut self, position: Vec2) {
        self.pointer = position;
        self.finish_drag(position);
        self.dragging_range = None;
        self.panning = None;
    }

    /// Abort every in-flight interaction.
    ///
    /// The window lost the focus: the pointer capture, the scrollbar grab
    /// and the demo drags have to go, or the shell keeps a ghost drag
    /// alive until the user happens to click somewhere else.
    pub fn cancel_interactions(&mut self) {
        self.scroll_drag = None;
        self.nav_scroll_drag = None;
        self.list_scroll_drag = None;
        self.dragging_range = None;
        self.panning = None;
        // Both drags have to be taken, not just the first one: `||` would
        // short-circuit and leave the second drag alive.
        let marquee = self.state.demo.marquee.take();
        let drag = self.state.demo.drag_active.take();
        if marquee.is_some() || drag.is_some() {
            self.state.demo.drag_position = None;
            self.state.bump();
        }
    }

    /// Wheel over the shell.
    pub fn wheel(&mut self, delta_y: f32) {
        let point = self.pointer;
        if self.layout.nav.contains_point(point) {
            self.state.nav_scroll =
                (self.state.nav_scroll - delta_y * 44.0).clamp(0.0, self.nav_max_scroll);
            self.state.bump();
            return;
        }
        if !self.layout.content.contains_point(point) {
            return;
        }
        if let Some((zone_rect, tag)) = self
            .scroll_zones
            .iter()
            .find(|(rect, _)| rect.contains_point(point))
        {
            let zone_rect = *zone_rect;
            let tag = *tag;
            match tag {
                // Tag 1 is the first list card, tag 5 the second; each owns its
                // own offset and row metrics. The zone is the viewport, so its
                // height is the exact clamp bound (the header is excluded).
                1 | 5 => {
                    let card = if tag == 1 { 0 } else { 1 };
                    let row = self.list_row_height(card);
                    let total = self.list_total(card);
                    let stage_h = zone_rect.size.y;
                    let scroll = self.state.demo.list_scroll[card];
                    self.state.demo.list_scroll[card] = (scroll - delta_y * row * 2.0)
                        .clamp(0.0, (total as f32 * row - stage_h).max(0.0));
                    self.state.bump();
                }
                2 => {
                    let factor = (1.0 + delta_y * 0.08).clamp(0.85, 1.2);
                    self.state.demo.canvas_zoom =
                        (self.state.demo.canvas_zoom * factor).clamp(0.4, 2.5);
                    self.state.bump();
                }
                3 | 4 => {}
                _ => {}
            }
            return;
        }
        let max = self.page_layout.as_ref().map_or(0.0, |page| {
            (page.content_height - self.layout.content.size.y + 48.0).max(0.0)
        });
        self.state.content_scroll = (self.state.content_scroll - delta_y * 52.0).clamp(0.0, max);
        self.state.bump();
    }

    /// Whether a text control owns the keyboard.
    ///
    /// The window enables the IME only while this is true; leaving it on for
    /// the whole application would pop the candidate strip over every page.
    #[must_use]
    pub fn ime_active(&self) -> bool {
        self.state.search_focused || self.state.demo.typing
    }

    /// Where the candidate window should be anchored, in logical pixels.
    #[must_use]
    pub fn ime_caret(&self) -> Vec2 {
        self.ime_caret
    }

    /// Insert committed IME text into the focused control.
    ///
    /// Control characters are dropped: a text field is single-line, and a
    /// stray line feed from a commit would corrupt the buffer.
    pub fn type_text(&mut self, text: &str) {
        let filtered: String = text.chars().filter(|c| !c.is_control()).collect();
        if filtered.is_empty() {
            return;
        }
        if self.state.search_focused {
            self.state.search_query.push_str(&filtered);
        } else if self.state.demo.typing {
            self.state.demo.text.push_str(&filtered);
        } else {
            return;
        }
        self.state.bump();
    }

    /// Handle a key that the OS is repeating while it is held down.
    ///
    /// Text is excluded: a held `a` is not a stream of `a`s, and `winit` also
    /// reports the composed characters again. Only the editing keys repeat.
    pub fn key_repeat(&mut self, key: &PlatformKey) {
        let PlatformKey::Named(name) = key else {
            return;
        };
        if name != "Backspace" {
            return;
        }
        if self.state.search_focused {
            self.state.search_query.pop();
        } else if self.state.demo.typing {
            self.state.demo.text.pop();
        } else {
            return;
        }
        self.state.bump();
    }

    /// Handle a key press.
    ///
    /// `focused` is the runtime's focused node: the shell does not keep one.
    /// `Tab` never reaches this method — the runtime's `FocusManager` steps
    /// the order before dispatching.
    pub fn key(&mut self, key: &PlatformKey, modifiers: Modifiers, focused: Option<u64>) {
        if self.state.fps_focused {
            match key {
                PlatformKey::Named(name) if name == "Escape" => {
                    self.state.fps_focused = false;
                    self.state.fps_input = self.state.fps_limit.to_string();
                    self.state.bump();
                }
                PlatformKey::Named(name) if name == "Enter" => {
                    self.state.commit_fps_input();
                }
                PlatformKey::Named(name) if name == "Backspace" => {
                    self.state.fps_input.pop();
                    self.state.bump();
                }
                PlatformKey::Character(text) if !modifiers.control => {
                    // Only digits: the limit is an FPS count.
                    for c in text.chars().filter(char::is_ascii_digit) {
                        self.state.fps_input.push(c);
                    }
                    self.state.bump();
                }
                _ => {}
            }
            return;
        }
        if self.state.search_focused {
            match key {
                PlatformKey::Named(name) if name == "Escape" => {
                    self.state.search_focused = false;
                    self.state.search_query.clear();
                    self.state.bump();
                }
                PlatformKey::Named(name) if name == "Backspace" => {
                    self.state.search_query.pop();
                    self.state.bump();
                }
                PlatformKey::Named(name) if name == "Enter" => {
                    if let Some(page) = self.state.search_results().first().copied() {
                        self.navigate(page);
                    }
                }
                PlatformKey::Character(text) if !modifiers.control => {
                    self.state.search_query.push_str(text);
                    self.state.bump();
                }
                _ => {}
            }
            return;
        }
        if self.state.demo.typing {
            match key {
                PlatformKey::Named(name) if name == "Escape" || name == "Enter" => {
                    self.state.demo.typing = false;
                    self.state.bump();
                }
                PlatformKey::Named(name) if name == "Backspace" => {
                    self.state.demo.text.pop();
                    self.state.bump();
                }
                PlatformKey::Character(text) if !modifiers.control => {
                    self.state.demo.text.push_str(text);
                    self.state.bump();
                }
                _ => {}
            }
            return;
        }
        if modifiers.control {
            if let PlatformKey::Character(text) = key {
                if text.eq_ignore_ascii_case("k") {
                    self.state.search_focused = true;
                    self.state.bump();
                }
            }
            return;
        }
        match key {
            PlatformKey::Named(name) if name == "Enter" => {
                if let Some(node) = focused {
                    if let Some(target) = target::decode(node) {
                        self.activate_target(target, self.pointer);
                    }
                }
            }
            PlatformKey::Character(text) if text == " " => {
                if let Some(node) = focused {
                    if let Some(target) = target::decode(node) {
                        self.activate_target(target, self.pointer);
                    }
                }
            }
            PlatformKey::Named(name) if name == "Escape" => {
                self.state.demo.modal_open = false;
                self.state.demo.dropdown_open = false;
                self.state.search_focused = false;
                self.state.search_query.clear();
                self.state.bump();
            }
            PlatformKey::Named(name) if name == "F12" => {
                self.state.show_hud = !self.state.show_hud;
                self.state.bump();
            }
            PlatformKey::Named(name)
                if matches!(
                    name.as_str(),
                    "ArrowDown" | "ArrowUp" | "PageDown" | "PageUp" | "Home" | "End"
                ) =>
            {
                self.scroll_page(name);
            }
            PlatformKey::Character(text) if text.eq_ignore_ascii_case("t") => {
                self.state.theme = self.state.theme.toggled();
                self.state.bump();
            }
            _ => {}
        }
    }

    /// Scroll the page with the keyboard.
    ///
    /// A wheel alone is not enough: the page has to be reachable without a
    /// pointer, and `Home` / `End` are the conventional way to jump to the
    /// top and the bottom.
    fn scroll_page(&mut self, key: &str) {
        let viewport = self.layout.content.size.y;
        let step = match key {
            "ArrowDown" => 40.0,
            "ArrowUp" => -40.0,
            "PageDown" => viewport * 0.8,
            "PageUp" => -viewport * 0.8,
            "Home" => f32::NEG_INFINITY,
            "End" => f32::INFINITY,
            _ => return,
        };
        let max = self.page_layout.as_ref().map_or(0.0, |page| {
            (page.content_height - self.layout.content.size.y + 48.0).max(0.0)
        });
        let next = if step.is_infinite() {
            if step > 0.0 {
                max
            } else {
                0.0
            }
        } else {
            (self.state.content_scroll + step).clamp(0.0, max)
        };
        if (next - self.state.content_scroll).abs() > 0.01 {
            self.state.content_scroll = next;
            self.state.bump();
        }
    }

    /// Activate the control a click landed on.
    ///
    /// The runtime only reports a click when the release landed on the
    /// pressed control and the press never became a drag, so this is the one
    /// activation entry point.
    pub fn click(&mut self, node: u64, position: Vec2) {
        if let Some(target) = target::decode(node) {
            self.click_target(target, position);
        }
    }

    /// The focus request the last reduction raised, if any.
    pub fn take_focus_request(&mut self) -> Option<u64> {
        self.pending_focus.take()
    }

    /// Whether the last reduction asked to blur.
    pub fn take_blur_request(&mut self) -> bool {
        core::mem::take(&mut self.pending_blur)
    }

    /// Whether a hovered control captures the pointer, so the window's resize
    /// border must yield to it instead of stealing the press or the hover.
    #[must_use]
    pub fn hover_captures_pointer(&self, node: Option<u64>) -> bool {
        let Some(node) = node else {
            return false;
        };
        matches!(
            target::decode(node),
            Some(
                Target::ContentScrollbar
                    | Target::NavScrollbar
                    | Target::Knob(..)
                    | Target::Demo(..)
            )
        )
    }
    /// Start dragging the page's scrollbar thumb.
    ///
    /// A press on the thumb grabs it where it was; a press on the track
    /// jumps the thumb to the pointer.
    fn begin_scrollbar_drag(&mut self, position: Vec2) {
        let Some(bar) = self.scrollbar else {
            return;
        };
        let thumb_top = bar.thumb_top(self.state.content_scroll);
        let grab = position.y - thumb_top;
        self.scroll_drag = Some(grab);
        if !(0.0..=bar.thumb_h).contains(&grab) {
            // A press on the track — above *or* below the thumb — centres the
            // thumb on the pointer and keeps dragging from there. Testing only
            // `grab > thumb_h` left above-thumb clicks storing a negative grab,
            // which jumped the page on the next move.
            let centred = position.y - bar.thumb_h * 0.5;
            let scroll = bar.scroll_for(centred);
            if (scroll - self.state.content_scroll).abs() > 0.01 {
                self.state.content_scroll = scroll;
                self.state.bump();
            }
            self.scroll_drag = Some(bar.thumb_h * 0.5);
        }
    }

    /// Start dragging the navigation pane's scrollbar thumb.
    fn begin_nav_scrollbar_drag(&mut self, position: Vec2) {
        let Some(bar) = self.nav_scrollbar else {
            return;
        };
        let thumb_top = bar.thumb_top(self.state.nav_scroll);
        let grab = position.y - thumb_top;
        self.nav_scroll_drag = Some(grab);
        if !(0.0..=bar.thumb_h).contains(&grab) {
            let centred = position.y - bar.thumb_h * 0.5;
            let scroll = bar.scroll_for(centred);
            if (scroll - self.state.nav_scroll).abs() > 0.01 {
                self.state.nav_scroll = scroll;
                self.state.bump();
            }
            self.nav_scroll_drag = Some(bar.thumb_h * 0.5);
        }
    }

    /// Start dragging a virtual-list card's scrollbar thumb.
    fn begin_list_scrollbar_drag(&mut self, card: usize, position: Vec2) {
        let Some(bar) = self.list_scrollbar(card) else {
            return;
        };
        let thumb_top = bar.thumb_top(self.state.demo.list_scroll[card]);
        let grab = position.y - thumb_top;
        self.list_scroll_drag = Some((card, grab));
        if !(0.0..=bar.thumb_h).contains(&grab) {
            let centred = position.y - bar.thumb_h * 0.5;
            let scroll = bar.scroll_for(centred);
            if (scroll - self.state.demo.list_scroll[card]).abs() > 0.01 {
                self.state.demo.list_scroll[card] = scroll;
                self.state.bump();
            }
            self.list_scroll_drag = Some((card, bar.thumb_h * 0.5));
        }
    }

    /// Begin a press-captured interaction.
    ///
    /// Only the controls that need the pointer *captured* act on the
    /// press: the content scrollbar, the sliders and the draggable demo
    /// stages. Every discrete control waits for the release, so pressing a
    /// button and dragging off it never activates it.
    fn begin_press_target(&mut self, target: Target, position: Vec2) {
        match target {
            Target::ContentScrollbar => {
                self.begin_scrollbar_drag(position);
            }
            Target::NavScrollbar => {
                self.begin_nav_scrollbar_drag(position);
            }
            Target::Knob(page, card, knob) => {
                // Only a slider captures the pointer. A toggle or a checkbox
                // is a discrete control and waits for the release.
                if knob_spec(page, card, knob)
                    .and_then(|spec| spec.range())
                    .is_some()
                {
                    self.dragging_range = Some(target::id(target));
                    self.update_range(target::id(target), position);
                }
            }
            Target::Demo(page, card, part) => {
                self.press_demo(page, card, part, position);
            }
            _ => {}
        }
    }

    /// Activate a control.
    ///
    /// Every discrete action of the shell happens here: the navigation, the
    /// theme and zoom switches, the toggles, the segmented options, the
    /// code panels and the window buttons. The router only reports a click
    /// when the release landed on the pressed control and the press never
    /// became a drag, so a press that was dragged away does nothing.
    fn click_target(&mut self, target: Target, position: Vec2) {
        match target {
            Target::NavPage(page) => self.navigate(PageId::from_index(page as usize)),
            Target::NavToggle => {
                self.state.nav_expanded = !self.state.nav_expanded;
                self.state.bump();
            }
            Target::Theme(bit) => {
                self.state.theme = if bit == 0 {
                    ThemeMode::Light
                } else {
                    ThemeMode::Dark
                };
                self.state.bump();
            }
            Target::Zoom(step) => {
                self.state.zoom_step = (step as usize).min(ZOOM_STEPS.len() - 1);
                self.state.bump();
            }
            Target::Snap => {
                self.state.pixel_snap = !self.state.pixel_snap;
                self.state.bump();
            }
            Target::Hud => {
                self.state.show_hud = !self.state.show_hud;
                self.state.bump();
            }
            Target::Search => {
                self.state.search_focused = true;
                self.state.demo.typing = false;
                self.ime_caret = self.pointer;
                self.state.bump();
            }
            Target::FpsPreset(index) => {
                let fps = [60, 120, 144, 240, 0][usize::from(index).min(4)];
                self.state.set_fps_limit(fps);
                self.state.fps_focused = false;
            }
            Target::FpsInput => {
                self.state.fps_focused = true;
                self.state.fps_input = self.state.fps_limit.to_string();
                self.ime_caret = self.pointer;
                self.state.bump();
            }
            Target::Footer(index) => self.navigate(if index == 0 {
                PageId::Telemetry
            } else {
                PageId::Settings
            }),
            Target::SearchResult(page) => self.navigate(PageId::from_index(page as usize)),
            Target::Knob(page, card, knob) => {
                // A slider was already moved by the press and the drag; a
                // toggle or a checkbox flips here.
                if knob_spec(page, card, knob)
                    .and_then(|spec| spec.range())
                    .is_none()
                {
                    let page_id = PageId::from_index(page as usize);
                    if let Some(spec) = knob_spec(page, card, knob) {
                        let current = self
                            .state
                            .knob(page_id, card as usize, knob as usize, &spec);
                        self.state.set_knob(
                            page_id,
                            card as usize,
                            knob as usize,
                            if current > 0.5 { 0.0 } else { 1.0 },
                        );
                    }
                }
            }
            Target::KnobOption(page, card, knob, option) => {
                self.state.set_knob(
                    PageId::from_index(page as usize),
                    card as usize,
                    knob as usize,
                    option as f32,
                );
            }
            Target::Code(page, card) => {
                self.state
                    .toggle_code(PageId::from_index(page as usize), card);
            }
            Target::Demo(page, card, part) => {
                self.click_demo(page, card, part, position);
            }
            Target::WindowMinimize => {
                self.pending_window = Some(WindowCommand::Minimize);
            }
            Target::WindowMaximize => {
                self.pending_window = Some(WindowCommand::ToggleMaximize);
            }
            Target::WindowClose => {
                self.pending_window = Some(WindowCommand::Close);
            }
            Target::ContentScrollbar => {
                // The scrollbar acts on the press, not on the release.
            }
            Target::NavScrollbar => {
                // The nav scrollbar acts on the press, not on the release.
            }
        }
    }

    /// Activate a control from the keyboard.
    ///
    /// The keyboard cannot drag, so a slider jumps to the pointer instead
    /// of following a drag.
    fn activate_target(&mut self, target: Target, position: Vec2) {
        match target {
            Target::Knob(page, card, knob)
                if knob_spec(page, card, knob)
                    .and_then(|spec| spec.range())
                    .is_some() =>
            {
                self.update_range(target::id(target), position);
            }
            _ => self.click_target(target, position),
        }
    }

    fn press_demo(&mut self, page: u8, card: u8, part: u16, position: Vec2) {
        let page = PageId::from_index(page as usize);
        if page == PageId::Viewport && part == 1 {
            self.panning = Some(position);
        }
        if page == PageId::Marquee && part == 1 {
            self.state.demo.marquee = Some((position, position));
            self.state.bump();
        }
        if page == PageId::DragDrop && card == 0 && (1..=4).contains(&part) {
            self.state.demo.drag_active = Some(part as usize - 1);
            self.state.demo.drag_position = Some(position);
            self.state.bump();
        }
        if page == PageId::ListView && part == 200 {
            self.begin_list_scrollbar_drag(card as usize, position);
        }
        if page == PageId::Sliders {
            if let Some(zone) = self
                .ranges
                .iter()
                .find(|zone| zone.node == target::id(Target::Demo(page.index() as u8, card, part)))
            {
                self.dragging_range = Some(zone.node);
                self.update_range(zone.node, position);
            }
        }
    }

    fn click_demo(&mut self, page: u8, card: u8, part: u16, position: Vec2) {
        let page = PageId::from_index(page as usize);
        // The frame-rate bar is shared by Telemetry and Settings and uses the
        // same part ids on both pages.
        if matches!(page, PageId::Telemetry | PageId::Settings) {
            match part {
                20..=24 => {
                    let fps = [60, 120, 144, 240, 0][(part - 20) as usize];
                    self.state.set_fps_limit(fps);
                    self.state.fps_focused = false;
                    return;
                }
                30 => {
                    self.state.fps_focused = true;
                    self.state.fps_input = self.state.fps_limit.to_string();
                    self.ime_caret = position;
                    self.state.bump();
                    return;
                }
                _ => {}
            }
        }
        match page {
            PageId::Home => match card {
                0 => {
                    if part == 1 {
                        self.navigate(PageId::Buttons);
                    } else if part == 2 {
                        self.navigate(PageId::Viewport);
                    }
                }
                1 => {
                    if let Some((_, _, target, _)) =
                        pages::home::CAPABILITIES.get(part as usize - 1)
                    {
                        self.navigate(*target);
                    }
                }
                2 => {
                    if let Some(target) = pages::home::INDEX_PAGES.get(part as usize - 1) {
                        self.navigate(*target);
                    }
                }
                _ => {}
            },
            PageId::Buttons => {
                if card == 0 {
                    self.state.demo.clicks = self.state.demo.clicks.wrapping_add(1);
                    self.state.bump();
                } else if (10..18).contains(&part) {
                    self.state.demo.selected_icon = part as usize - 10;
                    self.state.bump();
                }
            }
            PageId::Toggles => match part {
                1 => {
                    self.state.demo.toggle_a = !self.state.demo.toggle_a;
                    self.state.bump();
                }
                2 => {
                    self.state.demo.toggle_b = !self.state.demo.toggle_b;
                    self.state.bump();
                }
                10..=12 => {
                    self.state.demo.radio_choice = (part - 10) as usize;
                    self.state.bump();
                }
                20..=22 => {
                    self.state.demo.set_pill((part - 20) as usize);
                    self.state.bump();
                }
                30..=34 => {
                    self.state.demo.set_pill((part - 30) as usize);
                    self.state.bump();
                }
                _ => {}
            },
            PageId::TextField => {
                if part == 1 {
                    self.state.demo.typing = true;
                    self.state.search_focused = false;
                    self.ime_caret = self.pointer;
                    self.state.bump();
                }
            }
            PageId::Fonts => {
                // The BiDi sandbox's custom preset is a real text input: the
                // IME has to compose into `demo.text` exactly like the text
                // field page, otherwise a CJK/Arabic input method would only
                // ever deliver ASCII.
                if card == 2 && part == 1 {
                    self.state.demo.typing = true;
                    self.state.search_focused = false;
                    self.ime_caret = self.pointer;
                    self.state.bump();
                }
            }
            PageId::Overlays => match part {
                1 => {
                    self.state.demo.dropdown_open = !self.state.demo.dropdown_open;
                    self.state.demo.modal_open = false;
                    self.state.bump();
                }
                2 => {
                    self.state.demo.modal_open = !self.state.demo.modal_open;
                    self.state.demo.dropdown_open = false;
                    self.state.bump();
                }
                10..=12 => {
                    self.state.demo.dropdown_open = false;
                    self.state.bump();
                }
                20 | 21 => {
                    self.state.demo.modal_open = false;
                    self.state.bump();
                }
                _ => {}
            },
            PageId::ListView => {
                if (100..112).contains(&part) {
                    let card = card as usize;
                    let row = self.list_row_height(card);
                    let first = (self.state.demo.list_scroll[card] / row).floor() as usize;
                    self.state.demo.list_selected[card] = Some(first + (part - 100) as usize);
                    self.state.bump();
                }
            }
            PageId::Marquee => {
                if (10..22).contains(&part) {
                    let index = (part - 10) as usize;
                    self.state.demo.selected_nodes[index] = !self.state.demo.selected_nodes[index];
                    self.state.bump();
                }
            }
            PageId::Undo => match part {
                1 => self.state.undo(),
                2 => self.state.redo(),
                3 => self.state.push_history("Move node"),
                _ => {}
            },
            PageId::Springs => match part {
                1 => {
                    self.state.demo.spring_high = !self.state.demo.spring_high;
                    self.state
                        .demo
                        .spring
                        .set_target(if self.state.demo.spring_high {
                            1.0
                        } else {
                            0.0
                        });
                    self.state.bump();
                }
                2 => {
                    self.state.demo.spring.snap_to(0.0);
                    self.state.demo.spring_high = true;
                    self.state.demo.spring.set_target(1.0);
                    self.state.bump();
                }
                _ => {}
            },
            PageId::A11y => {
                if (10..=12).contains(&part) {
                    self.state.demo.a11y_focus = (part - 10) as usize;
                    self.state.bump();
                }
            }
            _ => {
                let _ = position;
            }
        }
    }

    fn finish_drag(&mut self, position: Vec2) {
        self.scroll_drag = None;
        self.nav_scroll_drag = None;
        self.list_scroll_drag = None;
        if let Some(start) = self.state.demo.marquee.map(|(start, _)| start) {
            let marquee = Rect::from_points(start, position);
            for index in 0..12u16 {
                if let Some(candidate) = demo_rect(self, PageId::Marquee, 0, 10 + index) {
                    if marquee.contains_point(candidate.center()) {
                        self.state.demo.selected_nodes[index as usize] = true;
                    }
                }
            }
            self.state.demo.marquee = None;
            self.state.bump();
        }
        if let Some(index) = self.state.demo.drag_active {
            let target_slot = (1..=4u16)
                .filter_map(|part| {
                    demo_rect(self, PageId::DragDrop, 0, part).map(|rect| (part, rect))
                })
                .min_by(|(_, a), (_, b)| {
                    let da = (a.center() - position).length_squared();
                    let db = (b.center() - position).length_squared();
                    da.total_cmp(&db)
                })
                .map_or(index, |(part, _)| part as usize - 1);
            let mut order = self.state.demo.drag_order.clone();
            if let Some(from) = order.iter().position(|value| *value == index) {
                let card = order.remove(from);
                let slot = target_slot.min(order.len());
                order.insert(slot, card);
                self.state.demo.drag_order = order;
            }
            self.state.demo.drag_active = None;
            self.state.demo.drag_position = None;
            self.state.bump();
        }
    }

    fn update_range(&mut self, node: u64, position: Vec2) {
        let Some(zone) = self.ranges.iter().find(|zone| zone.node == node).copied() else {
            return;
        };
        // The mapping from a pointer position to a value belongs to the
        // control, not to the shell: the same rule paints the slider and
        // moves it.
        let desc = unbound_widgets::slider::SliderDesc::new(
            zone.rect, "Slider", zone.min, zone.min, zone.max,
        )
        .with_step(zone.step);
        let value = unbound_widgets::slider::value_from_pointer(position, &desc);
        match target::decode(node) {
            Some(Target::Knob(page, card, knob)) => {
                let page_id = PageId::from_index(page as usize);
                self.state
                    .set_knob(page_id, card as usize, knob as usize, value);
                if page_id == PageId::Springs && card == 0 && knob <= 1 {
                    if knob == 0 {
                        self.state.demo.spring.stiffness = value.max(1.0);
                    } else {
                        self.state.demo.spring.damping = value.max(0.1);
                    }
                }
            }
            Some(Target::Demo(page, _, part))
                if page as usize == PageId::Sliders.index() && (1..=3).contains(&part) =>
            {
                self.state.demo.sliders[part as usize - 1] = value;
                self.state.bump();
            }
            _ => {}
        }
    }

    fn list_total(&self, card: usize) -> usize {
        let spec = PageId::ListView.examples()[card].knobs[0];
        match self.state.knob(PageId::ListView, card, 0, &spec) as usize {
            0 => 1_000,
            1 => 10_000,
            _ => 100_000,
        }
    }

    fn list_row_height(&self, card: usize) -> f32 {
        let spec = PageId::ListView.examples()[card].knobs[1];
        self.state
            .knob(PageId::ListView, card, 1, &spec)
            .clamp(24.0, 48.0)
    }

    /// The virtual-list scrollbar geometry for a card, rebuilt from the
    /// registered track hot zone so a drag stays in sync with what is painted.
    fn list_scrollbar(&self, card: usize) -> Option<Scrollbar> {
        let track = demo_rect(self, PageId::ListView, card as u8, 200)?;
        let viewport_h = track.size.y + 8.0;
        let row = self.list_row_height(card);
        let content_h = self.list_total(card) as f32 * row;
        let max_scroll = (content_h - viewport_h).max(0.0);
        if max_scroll <= 1.0 {
            return None;
        }
        let thumb_h = (viewport_h * viewport_h / content_h)
            .max(18.0)
            .min(track.size.y);
        Some(Scrollbar {
            track,
            thumb_h,
            max_scroll,
        })
    }

    /// A layout-only frame for the headless verifier.
    #[must_use]
    pub fn frame(&self) -> Frame {
        let mut builder = FrameBuilder::new(self.layout.viewport);
        builder.open(
            NodeKind::Container,
            self.layout.viewport,
            "Unbound UI Gallery",
        );
        // Title bar.
        builder.open(NodeKind::Container, self.layout.top_bar, "TitleBar");
        builder.open(
            interactive_rect(self.layout.nav_toggle),
            self.layout.nav_toggle,
            "Button \"Toggle navigation\"",
        );
        builder.close();
        text_node(
            &mut builder,
            Rect::new(52.0, 14.0, 190.0, 28.0),
            "Unbound UI Gallery",
        );
        if self.layout.search.size.x > 100.0 {
            builder.open(
                interactive_rect(self.layout.search),
                self.layout.search,
                "SearchBox \"Search controls\"",
            );
            builder.close();
        }
        bundle_controls(&mut builder, &self.layout);
        builder.close();

        // Navigation pane.
        builder.open(NodeKind::Container, self.layout.nav, "NavigationView");
        builder.set_clip(self.layout.nav);
        builder.set_scrollable(true);
        let nav_rect = self.layout.nav;
        let nav_rows = nav_rows(&self.state, &self.layout);
        for (rect, label) in nav_rows {
            // A row scrolled out of the pane paints nothing; validating its
            // clipped-to-zero target would only report the scroll position as
            // a layout bug.
            let visible = rect.intersect(&nav_rect);
            if visible.size.y >= 24.0 && visible.size.x >= 24.0 {
                builder.open(interactive_rect(rect), rect, &label);
                builder.close();
            }
        }
        builder.close();

        // Page content.
        builder.open(NodeKind::Container, self.layout.content, "Page");
        builder.set_clip(self.layout.content);
        builder.set_scrollable(true);
        if let Some(page) = &self.page_layout {
            let info = page.page.info();
            text_node(
                &mut builder,
                Rect::new(
                    page.header.origin.x,
                    page.header.origin.y,
                    page.header.size.x.min(600.0),
                    34.0,
                ),
                info.title,
            );
            text_node(
                &mut builder,
                Rect::new(
                    page.header.origin.x,
                    page.header.origin.y + 38.0,
                    page.header.size.x.min(720.0),
                    40.0,
                ),
                info.description,
            );
            let content_rect = self.layout.content;
            let visible = |rect: Rect| {
                let clipped = rect.intersect(&content_rect);
                clipped.size.x >= 24.0 && clipped.size.y >= 24.0
            };
            for card in &page.cards {
                if !card.rect.intersects(&content_rect) {
                    continue;
                }
                builder.open(NodeKind::Container, card.rect, "ExampleCard");
                builder.set_clip(card.rect);
                if visible(card.stage) {
                    builder.open(NodeKind::Container, card.stage, "LiveStage");
                    builder.set_clip(card.stage);
                    builder.open(interactive_rect(card.stage), card.stage, "Live preview");
                    builder.close();
                    builder.close();
                }
                if !card.knob_rows.is_empty() {
                    builder.open(NodeKind::Container, card.knobs, "Options");
                    for row in &card.knob_rows {
                        if let Some(label) = row.label {
                            if visible(label) {
                                text_node(&mut builder, label, "Property label");
                            }
                        }
                        if visible(row.control) {
                            builder.open(
                                interactive_rect(row.control),
                                row.control,
                                "Property control",
                            );
                            builder.close();
                        }
                    }
                    builder.close();
                }
                if visible(card.code_bar) {
                    builder.open(
                        interactive_rect(card.code_bar),
                        card.code_bar,
                        "Button \"Code and accessibility\"",
                    );
                    builder.close();
                }
                if let Some(panel) = card.code_panel {
                    if visible(panel) {
                        text_node(&mut builder, panel, "Source code");
                    }
                }
                builder.close();
            }
        }
        builder.close();

        // Portals.
        if self.state.search_focused {
            let overlay = search_overlay_rect(&self.layout);
            builder.open(NodeKind::Portal, overlay, "Command palette");
            builder.set_overlays_siblings(true);
            let results = self.state.search_results();
            let count = results.len().max(1);
            for index in 0..count {
                let row = Rect::new(
                    overlay.origin.x + 6.0,
                    overlay.origin.y + 34.0 + index as f32 * 36.0,
                    overlay.size.x - 12.0,
                    34.0,
                );
                builder.open(interactive_rect(row), row, "Search result");
                builder.close();
            }
            builder.close();
        }
        if self.state.demo.dropdown_open || self.state.demo.modal_open {
            let overlay = self
                .page_layout
                .as_ref()
                .and_then(|page| page.cards.first())
                .map_or(self.layout.content, |card| card.stage);
            builder.open(NodeKind::Portal, overlay, "Overlay");
            builder.set_overlays_siblings(true);
            builder.open(
                interactive_rect(overlay.inflate(-24.0)),
                overlay.inflate(-24.0),
                "Overlay action",
            );
            builder.close();
            builder.close();
        }
        if self.state.show_hud {
            if let Some(rect) = hud_rect(&self.layout) {
                builder.open(NodeKind::Portal, rect, "Stats HUD");
                builder.set_overlays_siblings(true);
                let (foreground, background) = hud_colors(self.state.theme.is_dark());
                let hud_text = |builder: &mut FrameBuilder, rect: Rect, text: &str| {
                    text_node(builder, rect, text);
                    builder.set_text_colors(foreground, background);
                };
                hud_text(
                    &mut builder,
                    Rect::new(
                        rect.origin.x + 12.0,
                        rect.origin.y + 8.0,
                        rect.size.x - 24.0,
                        14.0,
                    ),
                    "LIVE FRAME",
                );
                for index in 0..5 {
                    hud_text(
                        &mut builder,
                        Rect::new(
                            rect.origin.x + 12.0,
                            rect.origin.y + 26.0 + index as f32 * 15.0,
                            rect.size.x - 24.0,
                            14.0,
                        ),
                        "HUD metric",
                    );
                }
                builder.close();
            }
        }
        builder.close();
        builder.build()
    }
}

fn knob_spec(page: u8, card: u8, knob: u8) -> Option<KnobSpec> {
    PageId::from_index(page as usize)
        .examples()
        .get(card as usize)
        .and_then(|example| example.knobs.get(knob as usize))
        .copied()
}

fn interactive_rect(_rect: Rect) -> NodeKind {
    NodeKind::Interactive {
        min_size: NodeKind::DEFAULT_MIN_SIZE,
        occluded: false,
    }
}

fn text_node(builder: &mut FrameBuilder, rect: Rect, text: &str) {
    builder.open(
        NodeKind::Text {
            ink: rect,
            ellipsis: true,
        },
        rect,
        &format!("Text \"{text}\""),
    );
    builder.close();
}

fn bundle_controls(builder: &mut FrameBuilder, layout: &ShellLayout) {
    for (rect, label) in [
        (layout.theme, "Theme selector"),
        (layout.zoom, "Zoom selector"),
        (layout.snap, "Button \"Pixel snapping\""),
        (layout.hud, "Button \"Performance HUD\""),
        (layout.window_minimize, "Button \"Minimise window\""),
        (layout.window_maximize, "Button \"Maximise window\""),
        (layout.window_close, "Button \"Close window\""),
    ] {
        if rect.size.x >= 24.0 {
            builder.open(interactive_rect(rect), rect, label);
            builder.close();
        }
    }
}

fn nav_rows(state: &GalleryState, layout: &ShellLayout) -> Vec<(Rect, String)> {
    let mut rows = Vec::new();
    if layout.nav_width < 120.0 {
        let mut y = layout.nav.origin.y + 10.0;
        for page in [
            PageId::Home,
            PageId::Colors,
            PageId::Buttons,
            PageId::Flex,
            PageId::ListView,
            PageId::Viewport,
            PageId::Springs,
            PageId::Telemetry,
            PageId::Settings,
        ] {
            rows.push((
                Rect::new(layout.nav.origin.x + 12.0, y, layout.nav_width - 24.0, 36.0),
                format!("Navigation \"{}\"", page.info().title),
            ));
            y += 42.0;
        }
        let _ = state;
        return rows;
    }
    let mut y = layout.nav.origin.y + 10.0;
    rows.push((
        Rect::new(layout.nav.origin.x + 10.0, y, layout.nav_width - 20.0, 36.0),
        "Navigation \"Home\"".to_string(),
    ));
    // The tree starts below the Home row and its divider — the same offset
    // `nav_tree_rect` uses.
    y = layout.nav.origin.y + 56.0;
    for category in CategoryId::ALL {
        y += 24.0;
        for page in category.pages() {
            rows.push((
                Rect::new(layout.nav.origin.x + 10.0, y, layout.nav_width - 20.0, 30.0),
                format!("Navigation \"{}\"", page.info().short),
            ));
            y += 30.0;
        }
        y += 8.0;
    }
    let _ = state;
    rows
}

fn demo_rect(app: &GalleryApp, page: PageId, card: u8, part: u16) -> Option<Rect> {
    let node = target::id(Target::Demo(page.index() as u8, card, part));
    node_rect_from_regions(app, node)
}

fn node_rect_from_regions(app: &GalleryApp, node: u64) -> Option<Rect> {
    app.regions
        .iter()
        .find(|region| region.node == node)
        .map(|region| region.bounds)
}

// ---------------------------------------------------------------------------
// layout
// ---------------------------------------------------------------------------

fn nav_tree_rect(layout: &ShellLayout) -> Rect {
    let top = layout.nav.origin.y + 56.0;
    let bottom = layout.viewport.max().y - 10.0 - 82.0;
    Rect::new(
        layout.nav.origin.x,
        top,
        layout.nav.width(),
        (bottom - top).max(0.0),
    )
}

fn nav_height(layout: &ShellLayout) -> f32 {
    if layout.nav_width < 140.0 {
        return 9.0 * 42.0 + 10.0;
    }
    let mut height = 10.0 + 38.0 + 8.0;
    for category in CategoryId::ALL {
        height += 24.0 + category.pages().len() as f32 * 30.0 + 8.0;
    }
    height + 90.0
}

fn search_overlay_rect(layout: &ShellLayout) -> Rect {
    let width = layout.search.size.x.max(380.0);
    let x = layout
        .search
        .origin
        .x
        .min(layout.viewport.max().x - width - 14.0)
        .max(14.0);
    let results = 4.0;
    Rect::new(
        x,
        layout.search.max().y + 6.0,
        width,
        34.0 + results * 36.0 + 8.0,
    )
}

// ---------------------------------------------------------------------------
// drawing
// ---------------------------------------------------------------------------

fn draw_background(canvas: &mut Canvas<'_>, layout: &ShellLayout) {
    let palette = *canvas.palette;
    canvas.rect(layout.viewport, 0.0, palette.background, 0, 0.0, 0.0);
}

fn draw_top_bar(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    layout: &ShellLayout,
    window_maximized: bool,
) {
    let palette = *canvas.palette;
    canvas.rect(layout.top_bar, 0.0, palette.header_bg, 0, 0.0, 0.0);
    canvas.divider(0.0, layout.top_bar.max().y - 1.0, layout.top_bar.size.x);

    widgets::icon_button(
        canvas,
        target::id(Target::NavToggle),
        layout.nav_toggle,
        Icon::Menu,
        "Toggle navigation",
        !state.nav_expanded,
    );

    let logo = Rect::new(52.0, 14.0, 28.0, 28.0);
    canvas.rect(logo, 8.0, palette.primary, 0, 0.0, 4.0);
    canvas.text(
        "U",
        logo,
        15.0,
        palette.on_primary,
        AlignX::Center,
        AlignY::Center,
    );
    // The title gives way before it can collide with the theme control: at
    // 200 % zoom the logical viewport is barely 512 px wide, and a title
    // that runs into the segmented control is worse than no title.
    if layout.theme.origin.x > 292.0 {
        canvas.text(
            "Unbound UI Gallery",
            Rect::new(88.0, 12.0, 190.0, 32.0),
            13.5,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
    }

    if layout.wide {
        let info = state.active_page.info();
        let category = state.active_page.category().label();
        let crumb = if category == info.short {
            info.short.to_string()
        } else {
            format!("{category}  ›  {}", info.short)
        };
        canvas.text(
            &crumb,
            Rect::new(220.0, 12.0, layout.theme.origin.x - 240.0, 32.0),
            10.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
    }

    let search = layout.search;
    if search.size.x >= 100.0 {
        canvas.rect(
            search,
            8.0,
            palette.card_bg,
            if state.search_focused {
                palette.primary
            } else {
                palette.card_border
            },
            if state.search_focused { 2.0 } else { 1.0 },
            0.0,
        );
        widgets::icon(
            canvas,
            Icon::Search,
            Rect::new(search.origin.x + 4.0, search.origin.y, 28.0, search.size.y),
            palette.text_muted,
            palette.card_bg,
        );
        let preedit = if state.search_focused {
            state.ime_preedit.as_str()
        } else {
            ""
        };
        let composed;
        let placeholder = if state.search_query.is_empty() {
            if search.size.x < 280.0 {
                "Search…"
            } else {
                "Search controls, crates, features…"
            }
        } else if preedit.is_empty() {
            state.search_query.as_str()
        } else {
            composed = format!("{}{}", state.search_query, preedit);
            composed.as_str()
        };
        let ink = canvas.text(
            placeholder,
            Rect::new(
                search.origin.x + 32.0,
                search.origin.y,
                search.size.x - 84.0,
                search.size.y,
            ),
            11.0,
            if state.search_query.is_empty() {
                palette.text_muted
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
        // An IME composition is underlined, the way every native text field
        // shows one.
        if !preedit.is_empty() {
            let committed = canvas.labels.measure(&state.search_query, 11.0).x;
            let width = canvas.labels.measure(preedit, 11.0).x;
            canvas.rect(
                Rect::new(ink.origin.x + committed, ink.max().y - 1.5, width, 1.0),
                0.5,
                palette.primary,
                0,
                0.0,
                0.0,
            );
        }
        if layout.search.size.x > 280.0 {
            widgets::badge(
                canvas,
                Rect::new(search.max().x - 52.0, search.origin.y + 7.0, 44.0, 18.0),
                "Ctrl K",
                palette.text_muted,
                palette.stage_bg,
            );
        }
        canvas.control(
            target::id(Target::Search),
            search,
            Role::Search,
            "Search controls, crates and features",
            CursorKind::Text,
            false,
            state.search_focused,
        );
    } else if search.size.x > 0.0 {
        // Narrow windows keep the magnifier as a button that opens the same
        // command palette. Dropping search entirely left 150–200 % zoom with
        // no pointer path to it at all.
        widgets::icon_button(
            canvas,
            target::id(Target::Search),
            search,
            Icon::Search,
            "Search controls, crates and features",
            state.search_focused,
        );
    }

    // Theme.
    if layout.compact {
        widgets::icon_button(
            canvas,
            target::id(Target::Theme(if state.theme.is_dark() { 0 } else { 1 })),
            layout.theme,
            if state.theme.is_dark() {
                Icon::Sun
            } else {
                Icon::Moon
            },
            "Toggle theme",
            false,
        );
    } else {
        let ids = [target::id(Target::Theme(0)), target::id(Target::Theme(1))];
        widgets::segmented(
            canvas,
            &ids,
            layout.theme,
            &["Light", "Dark"],
            if state.theme.is_dark() { 1 } else { 0 },
            false,
            None,
        );
    }

    // Zoom.
    let labels: [&str; 4] = if layout.zoom.size.x >= 170.0 {
        ["100%", "125%", "150%", "200%"]
    } else {
        ["1×", "1.25×", "1.5×", "2×"]
    };
    let zoom_ids = [
        target::id(Target::Zoom(0)),
        target::id(Target::Zoom(1)),
        target::id(Target::Zoom(2)),
        target::id(Target::Zoom(3)),
    ];
    widgets::segmented(
        canvas,
        &zoom_ids,
        layout.zoom,
        &labels,
        state.zoom_step,
        false,
        None,
    );

    widgets::icon_button(
        canvas,
        target::id(Target::Snap),
        layout.snap,
        Icon::Snap,
        "Pixel snapping",
        state.pixel_snap,
    );
    widgets::icon_button(
        canvas,
        target::id(Target::Hud),
        layout.hud,
        Icon::Chart,
        "Performance HUD",
        state.show_hud,
    );

    // Client-side window buttons.
    draw_window_controls(canvas, layout, window_maximized);
}

fn draw_window_controls(canvas: &mut Canvas<'_>, layout: &ShellLayout, canvas_maximized: bool) {
    let palette = *canvas.palette;
    let buttons = [
        (
            Target::WindowMinimize,
            layout.window_minimize,
            "Minimise window",
            false,
        ),
        (
            Target::WindowMaximize,
            layout.window_maximize,
            "Maximise window",
            false,
        ),
        (
            Target::WindowClose,
            layout.window_close,
            "Close window",
            true,
        ),
    ];
    for (target, rect, label, destructive) in buttons {
        let node = target::id(target);
        let start = canvas.span_start();
        let hovered = canvas.hovered(node);
        let pressed = canvas.pressed(node);
        // The background always exists and only fades in on hover, so
        // pointing at a window button repaints one primitive instead of
        // inserting one.
        let fill = if destructive {
            palette.danger
        } else {
            palette.control_hover
        };
        canvas.rect_faded(rect, 0.0, fill, if hovered || pressed { 1.0 } else { 0.0 });
        let color = if destructive && hovered {
            palette.on_hero
        } else {
            palette.text
        };
        let middle = rect.center();
        match target {
            Target::WindowMinimize => {
                // A 10 × 1 logical pixel bar, vertically centred. The stroke
                // is rounded to whole physical pixels and aligned to the pixel
                // grid, so it stays one pixel thick at 100 %, 125 %, 150 % and
                // 200 % alike (the old `middle.y + 4.0` sat on the icon box's
                // bottom edge and doubled in weight at fractional scales).
                let scale = canvas.scale().max(0.5);
                let stroke = scale.round().max(1.0) / scale;
                let top = (middle.y * scale - stroke * scale * 0.5).round() / scale;
                canvas.rect(
                    Rect::new(middle.x - 5.0, top, 10.0, stroke),
                    0.0,
                    color,
                    0,
                    0.0,
                    0.0,
                );
            }
            Target::WindowMaximize => {
                let middle = rect.center();
                // Both states draw exactly **two** primitives, so the toggle
                // is a patch of two slices instead of a change of the
                // scene's shape. A shape change would move the upload
                // generation and re-upload the whole scene — and, worse, it
                // would let a dropped frame slip through with the pool
                // behind the diff base.
                if canvas_maximized {
                    // Restore: two offset squares. The front square is filled
                    // with whatever is behind the icon, otherwise the back
                    // square's edges run through its interior and the pair
                    // reads as a tic-tac-toe grid.
                    let occluder = if hovered || pressed {
                        palette.control_hover
                    } else {
                        palette.header_bg
                    };
                    canvas.rect(
                        Rect::new(middle.x - 3.0, middle.y - 5.0, 8.0, 8.0),
                        0.0,
                        0,
                        color,
                        1.0,
                        0.0,
                    );
                    canvas.rect(
                        Rect::new(middle.x - 5.0, middle.y - 3.0, 8.0, 8.0),
                        0.0,
                        occluder,
                        color,
                        1.0,
                        0.0,
                    );
                } else {
                    // Maximise: one stroked square, plus a transparent fill
                    // that stands in for the restore icon's second square.
                    // The fill carries no colour and no opacity, so it draws
                    // nothing, but it keeps the primitive count at two.
                    let square = Rect::new(middle.x - 5.0, middle.y - 5.0, 10.0, 10.0);
                    canvas.rect_faded(square, 0.0, 0, 0.0);
                    canvas.rect(square, 0.0, 0, color, 1.0, 0.0);
                }
            }
            Target::WindowClose => {
                // Two analytic strokes rather than the "×" glyph (U+00D7).
                // The glyph sits on the font's maths axis and its ink is a
                // fraction of the em box, so it was 40 % too small and 4 px
                // low at 200 %. The strokes intersect exactly at the button
                // centre and are antialiased by the SDF.
                let half = 4.5;
                let mid = [middle.x, middle.y];
                canvas.curve(
                    [middle.x - half, middle.y - half],
                    mid,
                    [middle.x + half, middle.y + half],
                    1.25,
                    color,
                );
                canvas.curve(
                    [middle.x + half, middle.y - half],
                    mid,
                    [middle.x - half, middle.y + half],
                    1.25,
                    color,
                );
            }
            _ => {}
        }
        canvas.control(
            node,
            rect,
            Role::Button,
            label,
            CursorKind::Default,
            false,
            false,
        );
        canvas.record_span(node, start);
    }
}

fn draw_nav(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout) {
    let palette = *canvas.palette;
    canvas.rect(layout.nav, 0.0, palette.nav_bg, 0, 0.0, 0.0);
    canvas.divider(layout.nav.max().x - 1.0, layout.nav.origin.y, 1.0);

    // Swap content while the spring is still moving: expanded labels would be
    // clipped by the shrinking pane, so fall back to the icon rail early.
    if layout.nav_width < 140.0 {
        draw_collapsed_nav(canvas, state, layout);
        return;
    }

    let mut y = layout.nav.origin.y + 10.0;
    nav_item(
        canvas,
        layout,
        Rect::new(layout.nav.origin.x + 10.0, y, layout.nav_width - 20.0, 36.0),
        "Home",
        Icon::Grid,
        state.active_page == PageId::Home,
        Target::NavPage(PageId::Home.index() as u8),
    );
    y += 38.0;
    canvas.divider(layout.nav.origin.x + 14.0, y + 6.0, layout.nav_width - 28.0);

    let tree = nav_tree_rect(layout);
    canvas.begin_clip(tree);
    let mut tree_y = tree.origin.y - state.nav_scroll;
    for category in CategoryId::ALL {
        let active = state.active_page.category() == category;
        canvas.text(
            &category.label().to_uppercase(),
            Rect::new(
                layout.nav.origin.x + 18.0,
                tree_y,
                layout.nav_width - 36.0,
                22.0,
            ),
            9.5,
            if active {
                palette.primary
            } else {
                palette.nav_text_muted
            },
            AlignX::Start,
            AlignY::Center,
        );
        tree_y += 24.0;
        for page in category.pages() {
            nav_item(
                canvas,
                layout,
                Rect::new(
                    layout.nav.origin.x + 10.0,
                    tree_y,
                    layout.nav_width - 20.0,
                    30.0,
                ),
                page.info().short,
                Icon::ChevronRight,
                state.active_page == *page,
                Target::NavPage(page.index() as u8),
            );
            tree_y += 30.0;
        }
        tree_y += 8.0;
    }
    canvas.end_clip();

    let footer_y = layout.viewport.max().y - 90.0;
    canvas.divider(
        layout.nav.origin.x + 14.0,
        footer_y,
        layout.nav_width - 28.0,
    );
    nav_item(
        canvas,
        layout,
        Rect::new(
            layout.nav.origin.x + 10.0,
            footer_y + 8.0,
            layout.nav_width - 20.0,
            34.0,
        ),
        "Engine Telemetry",
        Icon::Chart,
        state.active_page == PageId::Telemetry,
        // The footer shortcut needs its own identity: the tree row for the
        // same page already owns `NavPage(Telemetry)`, and sharing it made
        // hovering either row light up both.
        Target::Footer(0),
    );
    nav_item(
        canvas,
        layout,
        Rect::new(
            layout.nav.origin.x + 10.0,
            footer_y + 44.0,
            layout.nav_width - 20.0,
            34.0,
        ),
        "Settings & About",
        Icon::Layers,
        state.active_page == PageId::Settings,
        Target::Footer(1),
    );

    // The tree scrolls when the window is short or the app zoom is high, and
    // it used to do so silently: only the content pane carried an indicator,
    // so a clipped "Layout & portals" section looked like a missing page.
    draw_nav_scrollbar(canvas, state, layout);
}

/// Paint the navigation tree's scroll indicator.
fn draw_nav_scrollbar(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout) {
    let Some(bar) = nav_scrollbar(layout) else {
        return;
    };
    let palette = *canvas.palette;
    canvas.rect(bar.track, 3.0, palette.control_track, 0, 0.0, 0.0);
    let thumb = Rect::new(
        bar.track.origin.x,
        bar.thumb_top(state.nav_scroll),
        6.0,
        bar.thumb_h,
    );
    canvas.rect(thumb, 3.0, palette.primary, 0, 0.0, 0.0);
    // The nav scrollbar is a real control, not decoration: a 14px hot zone
    // lets the thumb be dragged and the track be clicked, exactly like the
    // page scrollbar.
    canvas.control(
        target::id(Target::NavScrollbar),
        Rect::new(
            bar.track.origin.x - 4.0,
            bar.track.origin.y,
            14.0,
            bar.track.size.y,
        ),
        Role::ScrollBar,
        "Scroll navigation",
        CursorKind::Pointer,
        false,
        false,
    );
}

fn nav_item(
    canvas: &mut Canvas<'_>,
    layout: &ShellLayout,
    rect: Rect,
    label: &str,
    icon: Icon,
    selected: bool,
    target: Target,
) {
    let palette = *canvas.palette;
    let node = target::id(target);
    let start = canvas.span_start();
    let hover = canvas.visual(node, false);
    // The row always paints two primitives — a pill and the selection
    // indicator — whatever its state. The pill takes the pane's own colour
    // when idle and the indicator fades out when unselected, so hovering a
    // row repaints two instances instead of renumbering the pool.
    let background = if selected {
        palette.nav_selected
    } else if hover.is_hovered() {
        palette.nav_hover
    } else {
        palette.nav_bg
    };
    canvas.rect(rect, 8.0, background, 0, 0.0, 0.0);
    // The selection indicator lives *inside* the pill. Flush against the
    // window edge it looked like a rendering artefact rather than part of
    // the row.
    canvas.rect_faded(
        Rect::new(
            rect.origin.x + 4.0,
            rect.origin.y + (rect.size.y - 16.0) * 0.5,
            3.0,
            16.0,
        ),
        1.5,
        palette.primary,
        if selected { 1.0 } else { 0.0 },
    );
    widgets::icon(
        canvas,
        icon,
        Rect::new(rect.origin.x + 14.0, rect.origin.y, 20.0, rect.size.y),
        if selected {
            palette.primary
        } else {
            palette.nav_text_muted
        },
        palette.nav_bg,
    );
    canvas.text(
        label,
        Rect::new(
            rect.origin.x + 40.0,
            rect.origin.y,
            rect.size.x - 46.0,
            rect.size.y,
        ),
        12.0,
        if selected {
            palette.nav_text
        } else {
            palette.nav_text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
    let _ = layout;
    canvas.control(
        node,
        rect,
        Role::Link,
        label,
        CursorKind::Pointer,
        false,
        selected,
    );
    canvas.record_span(node, start);
}

fn draw_collapsed_nav(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout) {
    let _ = state;
    let items = [
        (PageId::Home, Icon::Grid),
        (PageId::Colors, Icon::Layers),
        (PageId::Buttons, Icon::Snap),
        (PageId::Flex, Icon::Grid),
        (PageId::ListView, Icon::Chart),
        (PageId::Viewport, Icon::Snap),
        (PageId::Springs, Icon::Layers),
        (PageId::Telemetry, Icon::Chart),
        (PageId::Settings, Icon::Menu),
    ];
    let mut y = layout.nav.origin.y + 10.0;
    for (page, icon) in items {
        let rect = Rect::new(layout.nav.origin.x + 14.0, y, layout.nav_width - 28.0, 36.0);
        let selected = state.active_page == page
            || (page == PageId::Colors && state.active_page.category() == CategoryId::Design);
        widgets::icon_button(
            canvas,
            target::id(Target::NavPage(page.index() as u8)),
            rect,
            icon,
            page.info().short,
            selected,
        );
        y += 42.0;
    }
    // A hairline above the collapse toggle keeps the icon rail visually
    // separated from the page content.
    canvas.divider(
        layout.nav.origin.x + 10.0,
        layout.nav.max().y - 12.0,
        layout.nav_width - 20.0,
    );
}

fn draw_page(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    layout: &ShellLayout,
    page: &PageLayout,
) {
    let palette = *canvas.palette;
    let info = page.page.info();
    canvas.begin_clip(layout.content);
    canvas.semantic(
        Role::Heading,
        info.title,
        Rect::new(
            page.header.origin.x,
            page.header.origin.y,
            page.header.size.x,
            34.0,
        ),
    );
    canvas.text(
        info.title,
        Rect::new(
            page.header.origin.x,
            page.header.origin.y,
            page.header.size.x,
            38.0,
        ),
        30.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    canvas.text_wrapped(
        info.description,
        Rect::new(
            page.header.origin.x,
            page.header.origin.y + 42.0,
            (page.header.size.x * 0.74).min(820.0),
            46.0,
        ),
        12.5,
        palette.text_muted,
    );
    let mut badge_x = page.header.origin.x;
    for krate in info.crates {
        let width = canvas.labels.measure(krate, 11.0).x + 16.0;
        widgets::badge(
            canvas,
            Rect::new(badge_x, page.header.origin.y + 92.0, width, 20.0),
            krate,
            palette.primary,
            palette.primary_soft,
        );
        badge_x += width + 6.0;
    }
    canvas.text(
        &format!("docs › {}", info.docs),
        Rect::new(
            page.header.origin.x,
            page.header.origin.y + 124.0,
            page.header.size.x,
            14.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );

    for card in &page.cards {
        if card.rect.max().y < layout.content.origin.y
            || card.rect.origin.y > layout.content.max().y
        {
            continue;
        }
        draw_example(canvas, state, page, card);
    }

    if let Some(bar) = content_scrollbar(layout, page) {
        canvas.rect(bar.track, 3.0, palette.control_track, 0, 0.0, 0.0);
        let thumb_h = bar.thumb_h;
        let t = (state.content_scroll / bar.max_scroll).clamp(0.0, 1.0);
        let thumb = Rect::new(
            bar.track.origin.x,
            bar.track.origin.y + (bar.track.size.y - thumb_h) * t,
            6.0,
            thumb_h,
        );
        canvas.rect(thumb, 3.0, palette.primary, 0, 0.0, 0.0);
        // The whole track is the control: the thumb can be dragged, and a
        // click on the track jumps the thumb there. The 14px hot zone is wider
        // than the 6px bar so it is easy to grab, and stays inside the window
        // so the host's resize border never steals the press.
        canvas.control(
            target::id(Target::ContentScrollbar),
            Rect::new(
                bar.track.origin.x - 4.0,
                bar.track.origin.y,
                14.0,
                bar.track.size.y,
            ),
            Role::ScrollBar,
            "Scroll page",
            CursorKind::Pointer,
            false,
            false,
        );
    }
    canvas.end_clip();
}

fn draw_example(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    page: &PageLayout,
    card: &CardLayout,
) {
    let palette = *canvas.palette;
    let example = &page.page.examples()[card.index];
    canvas.rect(
        card.rect,
        14.0,
        palette.card_bg,
        palette.card_border,
        1.0,
        4.0,
    );
    canvas.begin_clip(card.rect);
    canvas.text(
        example.title,
        Rect::new(
            card.rect.origin.x + 18.0,
            card.rect.origin.y + 12.0,
            card.rect.size.x - 36.0,
            24.0,
        ),
        15.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    canvas.text_wrapped(
        example.description,
        Rect::new(
            card.rect.origin.x + 18.0,
            card.rect.origin.y + 38.0,
            card.rect.size.x - 36.0,
            30.0,
        ),
        10.5,
        palette.text_muted,
    );

    let mut badge_x = card.rect.max().x - 18.0;
    let badge_limit = card.rect.origin.x + 170.0;
    for krate in example.crates.iter().rev() {
        let width = canvas.labels.measure(krate, 9.5).x + 16.0;
        if badge_x - width < badge_limit {
            break;
        }
        badge_x -= width;
        let badge = widgets::badge_sized(
            canvas,
            Rect::new(badge_x, card.rect.origin.y + 14.0, width, 18.0),
            krate,
            palette.text_muted,
            palette.stage_bg,
            9.5,
        );
        badge_x = badge.origin.x - 6.0;
    }

    // Live stage.
    canvas.rect(
        card.stage,
        10.0,
        palette.stage_bg,
        palette.stage_grid,
        1.0,
        0.0,
    );
    canvas.begin_clip_rounded(card.stage, 10.0);
    let demo_rect = card.stage.inflate(-10.0);
    let knobs = state.knob_values(page.page, card.index, example);
    {
        let mut ctx = DemoCtx {
            canvas: &mut *canvas,
            state,
            page: page.page,
            card: card.index,
            rect: demo_rect,
            knobs: &knobs,
        };
        pages::render_demo(&mut ctx, example.demo);
    }
    canvas.end_clip();

    if !example.knobs.is_empty() {
        canvas.divider(card.knobs.origin.x - 9.0, card.knobs.origin.y, 1.0);
        canvas.text(
            "OPTIONS",
            Rect::new(
                card.knobs.origin.x,
                card.knobs.origin.y,
                card.knobs.size.x,
                18.0,
            ),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
        for row in &card.knob_rows {
            draw_knob_row(canvas, state, page.page, card.index, example, row, palette);
        }
    }

    draw_code_bar(canvas, state, page, card, example);
    canvas.end_clip();
}

fn draw_knob_row(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    page: PageId,
    card: usize,
    example: &crate::pages::ExampleDef,
    row: &KnobRow,
    palette: Palette,
) {
    let spec = example.knobs[row.index];
    let value = state.knob(page, card, row.index, &spec);
    let node = target::id(Target::Knob(
        page.index() as u8,
        card as u8,
        row.index as u8,
    ));
    match spec {
        KnobSpec::Toggle { label, .. } => {
            widgets::toggle_switch(canvas, node, row.control, label, value > 0.5, false);
        }
        KnobSpec::CheckBox { label, .. } => {
            widgets::checkbox(canvas, node, row.control, label, value > 0.5, false);
        }
        KnobSpec::Slider {
            label,
            min,
            max,
            step,
            unit,
            ..
        } => {
            canvas.text(
                label,
                row.label.unwrap_or(row.row),
                9.5,
                palette.text_muted,
                AlignX::Start,
                AlignY::Top,
            );
            let shown = if step < 1.0 {
                format!("{value:.1}{unit}")
            } else {
                format!("{value:.0}{unit}")
            };
            canvas.text(
                &shown,
                Rect::new(row.row.origin.x, row.row.origin.y, row.row.size.x, 16.0),
                9.5,
                palette.primary,
                AlignX::End,
                AlignY::Top,
            );
            widgets::slider(canvas, node, row.control, value, min, max);
            canvas.range(node, row.control, min, max, step);
        }
        KnobSpec::Segmented {
            label,
            options,
            default_index,
        } => {
            canvas.text(
                label,
                row.label.unwrap_or(row.row),
                9.5,
                palette.text_muted,
                AlignX::Start,
                AlignY::Top,
            );
            let ids: Vec<u64> = (0..options.len())
                .map(|option| {
                    target::id(Target::KnobOption(
                        page.index() as u8,
                        card as u8,
                        row.index as u8,
                        option as u8,
                    ))
                })
                .collect();
            let selected = (value as usize).min(options.len().saturating_sub(1));
            let _ = default_index;
            widgets::segmented(canvas, &ids, row.control, options, selected, false, None);
        }
    }
}

fn draw_code_bar(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    page: &PageLayout,
    card: &CardLayout,
    example: &crate::pages::ExampleDef,
) {
    let palette = *canvas.palette;
    let node = target::id(Target::Code(page.page.index() as u8, card.index as u8));
    let start = canvas.span_start();
    let hover = canvas.visual(node, false);
    let open = state.code_open(page.page, card.index as u8);
    // The hover wash always exists and only fades in, so pointing at the
    // code bar repaints one primitive instead of inserting one.
    canvas.rect_faded(
        card.code_bar,
        6.0,
        palette.control_hover,
        if hover.is_hovered() { 1.0 } else { 0.0 },
    );
    canvas.divider(
        card.code_bar.origin.x,
        card.code_bar.origin.y,
        card.code_bar.size.x,
    );
    canvas.text(
        "</>",
        Rect::new(
            card.code_bar.origin.x + 6.0,
            card.code_bar.origin.y,
            28.0,
            card.code_bar.size.y,
        ),
        10.0,
        palette.primary,
        AlignX::Start,
        AlignY::Center,
    );
    canvas.text(
        "Rust · ABI · accessibility",
        Rect::new(
            card.code_bar.origin.x + 40.0,
            card.code_bar.origin.y,
            card.code_bar.size.x - 80.0,
            card.code_bar.size.y,
        ),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
    widgets::icon(
        canvas,
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        Rect::new(
            card.code_bar.max().x - 28.0,
            card.code_bar.origin.y,
            20.0,
            card.code_bar.size.y,
        ),
        palette.text_muted,
        palette.card_bg,
    );
    canvas.control(
        node,
        card.code_bar,
        Role::Button,
        "Code and accessibility",
        CursorKind::Pointer,
        false,
        open,
    );
    // The span ends at the bar: the panel below it belongs to the card, and
    // it only appears when the card's shape changes anyway.
    canvas.record_span(node, start);

    if let Some(panel) = card.code_panel {
        canvas.rect(panel, 10.0, palette.code_bg, 0, 0.0, 0.0);
        canvas.begin_clip(panel);
        canvas.text(
            "SOURCE",
            Rect::new(
                panel.origin.x + 12.0,
                panel.origin.y + 5.0,
                panel.size.x - 24.0,
                14.0,
            ),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
        let mut y = panel.origin.y + 20.0;
        for (line_index, line) in example.code.lines().take(12).enumerate() {
            canvas.text(
                &format!("{:>2}", line_index + 1),
                Rect::new(panel.origin.x + 12.0, y, 20.0, 14.0),
                9.0,
                palette.text_muted,
                AlignX::End,
                AlignY::Top,
            );
            canvas.text(
                line,
                Rect::new(panel.origin.x + 38.0, y, panel.size.x - 50.0, 14.0),
                9.5,
                palette.code_text,
                AlignX::Start,
                AlignY::Top,
            );
            y += 15.0;
        }
        canvas.text(
            &format!("a11y · {}", example.a11y),
            Rect::new(
                panel.origin.x + 12.0,
                panel.max().y - 20.0,
                panel.size.x - 24.0,
                14.0,
            ),
            9.0,
            palette.primary,
            AlignX::Start,
            AlignY::Top,
        );
        canvas.end_clip();
    }
}

fn hud_rect(layout: &ShellLayout) -> Option<Rect> {
    if layout.content.size.x < 560.0 {
        return None;
    }
    Some(Rect::new(
        layout.content.max().x - 232.0,
        layout.content.max().y - 168.0,
        218.0,
        148.0,
    ))
}

fn hud_colors(dark: bool) -> ([u8; 3], [u8; 3]) {
    if dark {
        ([220, 227, 242], [8, 10, 14])
    } else {
        ([28, 30, 38], [255, 255, 255])
    }
}

fn draw_hud(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout, cards: usize) {
    if !state.show_hud {
        return;
    }
    let Some(rect) = hud_rect(layout) else {
        return;
    };
    let palette = *canvas.palette;
    // Float the telemetry in the bottom-right corner: the top-right area is
    // owned by the page title and its crate pills, and a HUD there covered
    // them at every window size.
    canvas.rect(rect, 10.0, palette.hud_bg, palette.card_border, 1.0, 18.0);
    canvas.text(
        "LIVE FRAME",
        Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y + 8.0,
            rect.size.x - 24.0,
            14.0,
        ),
        9.5,
        palette.hud_text,
        AlignX::Start,
        AlignY::Top,
    );
    let stats = &state.demo;
    let prims = canvas.scene.primitives.len();
    let glyphs = canvas.glyph_count();
    let rows = [
        ("FRAME", format!("{:.2} ms", stats.frame_ms)),
        ("FPS", format!("{:.0}", stats.fps)),
        ("PRIMS", format!("{prims}")),
        ("GLYPHS", format!("{glyphs}")),
        ("CARDS", format!("{cards}")),
        (
            "LIMIT",
            if state.fps_limit == 0 {
                "uncapped".to_string()
            } else {
                format!("{} fps", state.fps_limit)
            },
        ),
    ];
    for (index, (label, value)) in rows.iter().enumerate() {
        let y = rect.origin.y + 26.0 + index as f32 * 15.0;
        canvas.text(
            label,
            Rect::new(rect.origin.x + 12.0, y, 70.0, 14.0),
            9.5,
            palette.hud_text,
            AlignX::Start,
            AlignY::Center,
        );
        canvas.text(
            value,
            Rect::new(rect.max().x - 100.0, y, 88.0, 14.0),
            9.5,
            palette.hud_text,
            AlignX::End,
            AlignY::Center,
        );
    }
    // Sparkline.
    let chart = Rect::new(
        rect.origin.x + 12.0,
        rect.max().y - 26.0,
        rect.size.x - 24.0,
        16.0,
    );
    for index in 0..28 {
        let phase = index as f32 / 28.0;
        let value = 0.35 + (phase * 15.0 + stats.elapsed * 3.0).sin() * 0.22;
        let height = chart.size.y * value.clamp(0.08, 0.95);
        canvas.rect(
            Rect::new(
                chart.origin.x + phase * chart.size.x,
                chart.max().y - height,
                3.0,
                height,
            ),
            1.0,
            if index > 24 {
                palette.warning
            } else {
                palette.primary
            },
            0,
            0.0,
            0.0,
        );
    }
}

fn draw_search_overlay(canvas: &mut Canvas<'_>, state: &GalleryState, layout: &ShellLayout) {
    if !state.search_focused {
        return;
    }
    let palette = *canvas.palette;
    let results = state.search_results();
    let panel = search_overlay_rect(layout);
    canvas.rect(
        panel,
        12.0,
        palette.overlay_bg,
        palette.card_border,
        1.0,
        26.0,
    );
    canvas.text(
        if state.search_query.is_empty() {
            "COMMAND PALETTE · CTRL K"
        } else {
            "MATCHES"
        },
        Rect::new(
            panel.origin.x + 14.0,
            panel.origin.y + 8.0,
            panel.size.x - 28.0,
            14.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    if state.search_query.is_empty() {
        canvas.text(
            "Try “Button”, “MTSDF”, “Quadtree”, “DnD”…",
            Rect::new(
                panel.origin.x + 14.0,
                panel.origin.y + 30.0,
                panel.size.x - 28.0,
                32.0,
            ),
            11.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        return;
    }
    if results.is_empty() {
        canvas.text(
            "No controls match that query.",
            Rect::new(
                panel.origin.x + 14.0,
                panel.origin.y + 30.0,
                panel.size.x - 28.0,
                32.0,
            ),
            11.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        return;
    }
    for (index, page) in results.iter().enumerate() {
        let row = Rect::new(
            panel.origin.x + 6.0,
            panel.origin.y + 30.0 + index as f32 * 36.0,
            panel.size.x - 12.0,
            34.0,
        );
        let node = target::id(Target::SearchResult(page.index() as u8));
        let hover = canvas.visual(node, false);
        if hover.is_hovered() || *page == state.active_page {
            canvas.rect(
                row,
                7.0,
                if *page == state.active_page {
                    palette.primary_soft
                } else {
                    palette.control_hover
                },
                0,
                0.0,
                0.0,
            );
        }
        let info = page.info();
        canvas.text(
            info.short,
            Rect::new(
                row.origin.x + 12.0,
                row.origin.y,
                row.size.x * 0.52,
                row.size.y,
            ),
            11.0,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        canvas.text(
            &format!(
                "{} · {}",
                page.category().label(),
                info.crates.first().copied().unwrap_or("unbound")
            ),
            Rect::new(
                row.origin.x + row.size.x * 0.54,
                row.origin.y,
                row.size.x * 0.44 - 12.0,
                row.size.y,
            ),
            9.5,
            palette.text_muted,
            AlignX::End,
            AlignY::Center,
        );
        canvas.control_on_layer(
            node,
            row,
            Role::Link,
            info.title,
            CursorKind::Pointer,
            false,
            *page == state.active_page,
            2,
        );
    }
}

// ---------------------------------------------------------------------------
// upload
// ---------------------------------------------------------------------------

/// The diff between two scenes.
///
/// The widget layer owns the algorithm; the shell only knows what the
/// *shape* of its scene is, which is the tables a primitive indexes into.
fn diff_scenes(previous: &gallery::Gallery, current: &gallery::Gallery) -> SceneDiff {
    let structure_changed = previous.transforms != current.transforms
        || previous.clips != current.clips
        || previous.payloads != current.payloads;
    diff_primitives(&previous.primitives, &current.primitives, structure_changed)
}

/// Merge `incoming` into `pending`, coalescing overlapping or adjacent runs.
///
/// The upload path stages one contiguous slice per range, so a union of
/// per-rebuild diffs would otherwise stage the same primitive twice.
fn merge_ranges(pending: &mut Vec<(u32, u32)>, incoming: Vec<(u32, u32)>) {
    pending.extend(incoming);
    pending.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(pending.len());
    for (start, end) in pending.drain(..) {
        if start >= end {
            continue;
        }
        match merged.last_mut() {
            Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
            _ => merged.push((start, end)),
        }
    }
    *pending = merged;
}
