//! Standard controls: the vocabulary every widget speaks.
//!
//! The crate sits between the engine's capability crates and the application
//! layer. It owns the *control* semantics — a pure descriptor, a visual state
//! derived from the input router, and a scene emission — but no windowing, no
//! layout solving and no application state.
//!
//! Three rules hold for every control in this crate:
//!
//! 1. **No interaction booleans.** A widget never stores "am I hovered".
//!    [`ControlState::resolve`] derives the phase from the
//!    [`unbound_input::Router`] on every frame, so a pointer that leaves the
//!    widget while captured still reads as *pressed*.
//! 2. **No closures, no type erasure.** A control is a plain data struct plus
//!    a pure function that turns `(descriptor, state)` into primitives, a hit
//!    region and a semantic node.
//! 3. **Colours are tokens, never literals.** A widget names
//!    [`unbound_theme::Token`] slots; the theme decides what they resolve to.
//!    A theme switch therefore never re-uploads a single primitive.
//!
//! [`app-guest`](../../app_guest) still depends on [`unbound_abi`] alone, so a
//! guest application may host these controls without pulling in the engine.

pub mod accordion;
pub mod breadcrumb;
pub mod button;
pub mod checkbox;
pub mod command_palette;
pub mod containers;
pub mod context_menu;
pub mod diff;
pub mod divider;
pub mod dock;
pub mod image;
pub mod menu;
pub mod menubar;
pub mod modal;
pub mod overlay;
pub mod popover;
pub mod primitive;
pub mod radio;
pub mod scroll;
pub mod segmented;
pub mod select;
pub mod slider;
pub mod split_pane;
pub mod switch;
pub mod tabs;
pub mod text_input;
pub mod toolbar;
pub mod tooltip;
pub mod tree_view;

use rustc_hash::FxHashMap;
use unbound_a11y::{A11yNode, Semantics};
use unbound_gpu::data::{GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform, CLIP_NONE};
use unbound_headless::driver::{ClickReport, TestApp};
use unbound_input::{ClipRect, HitRegion, PointerEvent, Router};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;

pub use accordion::{
    emit_accordion_body, emit_accordion_header, AccordionDesc, AccordionSection, AccordionState,
    AccordionTokens,
};
pub use breadcrumb::{
    emit_crumb, emit_crumb_ellipsis, emit_crumb_separator, BreadcrumbDesc, BreadcrumbState,
    BreadcrumbTokens, Crumb, CrumbSlot,
};
pub use button::{ButtonDesc, ButtonTokens, ButtonVariant, ButtonWidget};
pub use checkbox::{CheckboxDesc, CheckboxTokens};
pub use command_palette::{
    emit_command_palette, emit_command_row, score_command, Command, CommandPaletteDesc,
    CommandPaletteState, CommandPaletteTokens, MatchKind,
};
pub use containers::{
    emit_card, emit_stack, CardDesc, CardTokens, StackDesc, WrapDesc, CARD_ELEVATIONS, WRAP_FLAG,
};
pub use context_menu::{ContextClose, ContextKey, ContextMenuState};
pub use divider::{emit_divider, DividerDesc, DividerOrientation, DividerTokens};
pub use dock::{
    emit_dock_pane, emit_dock_preview, emit_dock_tab, DockDrag, DockHover, DockNode, DockPane,
    DockState, DockTab, DockTokens, DockTree, DockZone,
};
pub use image::{
    emit_icon, emit_image, emit_image_with_assets, fit_image, IconDesc, ImageAssetResolver,
    ImageDesc, ImageFit, ImageSource, ImageWidget, ObjectFit, ResolvedImage,
};
pub use menu::{emit_menu, MenuDesc, MenuItem, MenuState, MenuWidget};
pub use menubar::{
    emit_menubar, emit_menubar_item, reduce_menubar_key, MenuBarDesc, MenuBarItem, MenuBarKey,
    MenuBarState, MenuBarTokens,
};
pub use modal::{emit_modal, ModalDesc, ModalOutput, ModalWidget};
pub use overlay::{
    place_anchored_popup, BackdropRequest, OverlayEntry, OverlayKind, OverlayStack, Placement,
};
pub use popover::{emit_popover, PopoverDesc, PopoverOutput, PopoverWidget};
pub use radio::{RadioDesc, RadioTokens};
pub use scroll::{
    emit_scroll_view, update_virtual_list, ScrollAxis, ScrollState, ScrollViewDesc,
    ScrollViewOutput, ScrollViewWidget, ScrollbarAxis, VirtualWindow,
};
pub use segmented::{SegmentedDesc, SegmentedTokens};
pub use select::{emit_select, SelectDesc, SelectState, SelectWidget};
pub use slider::{SliderDesc, SliderTokens};
pub use split_pane::{
    emit_split_pane, emit_split_pane_panes, reduce_split_key, SplitKey, SplitLayout, SplitPaneDesc,
    SplitPaneState, SplitPaneTokens, SplitSide,
};
pub use switch::{SwitchDesc, SwitchTokens};
pub use tabs::{
    emit_tab, emit_tab_close, emit_tabs_add, emit_tabs_strip, reduce_tabs_key, TabItem, TabsDesc,
    TabsKey, TabsState, TabsTokens,
};
pub use text_input::{
    emit_text_input, reduce_text_input, TextEditState, TextInputDesc, TextInputEvent,
    TextInputOutcome, TextInputVisual, TextInputWidget,
};
pub use toolbar::{
    emit_status_bar, emit_status_segment, emit_toolbar, emit_toolbar_item, emit_toolbar_overflow,
    StatusAlign, StatusBarDesc, StatusSegment, ToolbarDesc, ToolbarItem, ToolbarItemKind,
    ToolbarLayout, ToolbarState, ToolbarTokens,
};
pub use tooltip::{emit_tooltip, TooltipDesc, TooltipState, TooltipWidget};
pub use tree_view::{
    emit_tree_background, emit_tree_row, TreeViewDesc, TreeViewRow, TreeViewState, TreeViewTokens,
};

/// The default viewport a headless widget session renders into.
pub const DEFAULT_VIEWPORT: (f32, f32) = (1280.0, 720.0);

/// Where the pointer is relative to a widget.
///
/// This is the *only* pointer dimension a control needs. Focus is a separate,
/// orthogonal dimension ([`ControlState::focused`]): a button reached with the
/// keyboard must show its focus ring even while the mouse hovers elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerPhase {
    /// The pointer is not over the widget.
    #[default]
    Idle,
    /// The pointer is over the widget.
    Hovered,
    /// The pointer is over the widget and the button is down.
    Pressed,
}

/// The visual state of a control in one frame.
///
/// Every field is derived, never stored: [`ControlState::resolve`] is the only
/// constructor the engine uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlState {
    /// Where the pointer is relative to the widget.
    pub phase: PointerPhase,
    /// Whether the widget holds keyboard focus.
    pub focused: bool,
    /// Whether the widget refuses interaction.
    pub disabled: bool,
    /// Whether the widget is toggled on.
    pub selected: bool,
}

impl ControlState {
    /// Derive the state from the pointer router and the focus slot.
    ///
    /// A disabled widget never shows a pointer phase: it is not a target, so
    /// the pointer cannot be over it.
    #[must_use]
    pub fn resolve(
        node_key: u64,
        router: &Router,
        focused_key: Option<u64>,
        disabled: bool,
        selected: bool,
    ) -> Self {
        let phase = if disabled {
            PointerPhase::Idle
        } else if router.pointer().pressed() == Some(node_key) {
            PointerPhase::Pressed
        } else if router.hover() == Some(node_key) {
            PointerPhase::Hovered
        } else {
            PointerPhase::Idle
        };
        Self {
            phase,
            focused: focused_key == Some(node_key),
            disabled,
            selected,
        }
    }
}

/// What one control emits in one frame.
///
/// The three parts are deliberately independent: a purely decorative widget
/// emits no hit region, a widget that is not on screen emits no primitives,
/// and a widget that is not reachable by assistive technology emits no
/// semantic node.
#[derive(Clone, Debug, Default)]
pub struct WidgetOutput {
    /// The primitives, in draw order.
    pub primitives: Vec<GpuPrimitiveInstance>,
    /// The payloads the primitives refer to.
    pub payloads: Vec<GpuPayload>,
    /// The pickable surface, if the widget accepts input.
    pub hit: Option<HitRegion>,
    /// The semantic node, if the widget is reachable.
    pub a11y: Option<A11yNode>,
    /// Whether the widget takes part in the keyboard tab order.
    pub focusable: bool,
}

/// Collects the output of many widgets into one frame.
///
/// The collector also records, for every widget, the half-open range of
/// primitives it owns. That map is what makes a *local* repaint possible: a
/// hover that changes five primitives can rewrite
/// `primitives[start..end]` and upload just those bytes instead of the whole
/// scene.
#[derive(Default)]
pub struct SceneCollector {
    /// The primitives, in draw order.
    pub primitives: Vec<GpuPrimitiveInstance>,
    /// The transform table.
    pub transforms: Vec<GpuTransform>,
    /// The clip nodes.
    pub clips: Vec<GpuClipNode>,
    /// The payloads the primitives refer to.
    pub payloads: Vec<GpuPayload>,
    /// The pickable surfaces.
    pub regions: Vec<HitRegion>,
    /// The semantic tree.
    pub semantics: Semantics,
    /// The keyboard tab order.
    pub focus_order: Vec<u64>,
    /// The primitive range each widget owns, keyed by widget identity.
    pub node_spans: FxHashMap<u64, (u32, u32)>,
    /// Backdrop-blur requests produced by overlays this frame.
    pub backdrops: Vec<crate::overlay::BackdropRequest>,
    /// Active clip scopes: `(clip index, screen-space bounds)`.
    clip_stack: Vec<(u32, Rect)>,
    /// Active transform scopes: `(transform index, cumulative translation)`.
    transform_stack: Vec<(u32, Vec2)>,
}

impl SceneCollector {
    /// An empty collector whose semantic tree is rooted at `root`.
    #[must_use]
    pub fn new(root: u64) -> Self {
        Self {
            semantics: Semantics::new(root),
            ..Self::default()
        }
    }

    /// Reset for a new frame, reusing every allocation.
    pub fn reset(&mut self, root: u64) {
        self.primitives.clear();
        self.transforms.clear();
        self.clips.clear();
        self.payloads.clear();
        self.regions.clear();
        self.focus_order.clear();
        self.node_spans.clear();
        self.backdrops.clear();
        self.clip_stack.clear();
        self.transform_stack.clear();
        self.semantics.reset(root);
    }

    /// Open a clip scope for every widget pushed until [`Self::pop_clip`].
    ///
    /// `node` describes the new clip; when a parent scope is active its
    /// rounded slot is folded into `rounded_clip_1` and the effective bounds
    /// become the intersection of both scopes. The returned index is what the
    /// children's primitives reference.
    pub fn push_clip(&mut self, mut node: GpuClipNode, world_bounds: Rect) -> u32 {
        let mut bounds = world_bounds;
        if let Some(&(parent_idx, parent_bounds)) = self.clip_stack.last() {
            bounds = bounds.intersect(&parent_bounds);
            let [min_x, min_y, max_x, max_y] = node.aabb_clip;
            let own = Rect::new(
                min_x,
                min_y,
                (max_x - min_x).max(0.0),
                (max_y - min_y).max(0.0),
            );
            let folded = own.intersect(&bounds);
            node.aabb_clip = [
                folded.min().x,
                folded.min().y,
                folded.max().x,
                folded.max().y,
            ];
            if let Some(parent) = self.clips.get(parent_idx as usize) {
                node.rounded_clip_1 = parent.rounded_clip_0;
                node.radii_1 = parent.radii_0;
            }
        }
        let index = u32::try_from(self.clips.len()).unwrap_or(CLIP_NONE - 1);
        self.clips.push(node);
        self.clip_stack.push((index, bounds));
        index
    }

    /// Close the innermost clip scope.
    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    /// Open a transform scope for every widget pushed until
    /// [`Self::pop_transform`].
    ///
    /// The scope translation is the sum of every active scope, so children
    /// only ever reference the innermost transform index. Slot `0` of the
    /// table is always the identity.
    pub fn push_transform(&mut self, mut transform: GpuTransform) -> u32 {
        if self.transforms.is_empty() {
            self.transforms.push(GpuTransform::IDENTITY);
        }
        if let Some(&(_, parent_offset)) = self.transform_stack.last() {
            transform.translation[0] += parent_offset.x;
            transform.translation[1] += parent_offset.y;
        }
        let offset = Vec2::new(transform.translation[0], transform.translation[1]);
        let index = u32::try_from(self.transforms.len()).unwrap_or(CLIP_NONE - 1);
        self.transforms.push(transform);
        self.transform_stack.push((index, offset));
        index
    }

    /// Close the innermost transform scope.
    pub fn pop_transform(&mut self) {
        self.transform_stack.pop();
    }

    /// Append one widget's output, reserving a fixed number of primitive
    /// slots so a dynamic glyph run keeps every following widget's span
    /// stable.
    ///
    /// Unused slots are padded with a fully transparent instance, so adding
    /// a character changes only the slots it actually paints instead of
    /// shifting the whole scene. When the output outgrows the reservation the
    /// span is rounded up to the next power of two.
    pub fn push_with_capacity(
        &mut self,
        key: u64,
        mut output: WidgetOutput,
        reserved_slots: usize,
    ) {
        let target = if output.primitives.len() > reserved_slots {
            output.primitives.len().next_power_of_two()
        } else {
            reserved_slots
        };
        output
            .primitives
            .resize_with(target, GpuPrimitiveInstance::default);
        self.push(key, output);
    }

    /// Append one widget's output and record the primitives it owns.
    pub fn push(&mut self, key: u64, mut output: WidgetOutput) {
        let active_transform = self.transform_stack.last().copied();
        if let Some((transform_idx, offset)) = active_transform {
            for primitive in &mut output.primitives {
                if primitive.transform_idx == 0 {
                    primitive.transform_idx = transform_idx;
                }
            }
            if let Some(hit) = output.hit.as_mut() {
                hit.bounds = hit.bounds.translate(offset);
                if let Some(clip) = hit.clip.as_mut() {
                    clip.rect = clip.rect.translate(offset);
                }
            }
            if let Some(node) = output.a11y.as_mut() {
                node.bounds = node.bounds.translate(offset);
            }
        }

        if let Some((clip_idx, clip_rect)) = self.clip_stack.last().copied() {
            for primitive in &mut output.primitives {
                if primitive.clip_idx == CLIP_NONE {
                    primitive.clip_idx = clip_idx;
                }
            }
            if let Some(hit) = output.hit.as_mut() {
                hit.bounds = hit.bounds.intersect(&clip_rect);
                hit.clip = Some(match hit.clip.take() {
                    Some(existing) => ClipRect {
                        rect: existing.rect.intersect(&clip_rect),
                        radii: existing.radii,
                    },
                    None => ClipRect::new(clip_rect),
                });
            }
            if let Some(node) = output.a11y.as_mut() {
                node.bounds = node.bounds.intersect(&clip_rect);
            }
        }

        let start = u32::try_from(self.primitives.len()).unwrap_or(u32::MAX);
        self.primitives.extend(output.primitives);
        let end = u32::try_from(self.primitives.len()).unwrap_or(u32::MAX);
        self.node_spans.insert(key, (start, end));
        self.payloads.extend(output.payloads);
        if let Some(hit) = output.hit {
            self.regions.push(hit);
        }
        if let Some(node) = output.a11y {
            self.semantics.upsert(key, node);
        }
        if output.focusable {
            self.focus_order.push(key);
        }
    }

    /// The primitives one widget owns.
    #[must_use]
    pub fn span(&self, key: u64) -> Option<&[GpuPrimitiveInstance]> {
        let (start, end) = *self.node_spans.get(&key)?;
        let start = usize::try_from(start).ok()?;
        let end = usize::try_from(end).ok()?;
        self.primitives.get(start..end)
    }
}

/// A widget descriptor: the pure data that determines a widget's scene.
///
/// The trait exists so that one harness ([`WidgetHarness`]) can drive any
/// control: the descriptor answers "where am I" and "what do I look like in
/// this state", and nothing else.
pub trait WidgetDesc {
    /// The interactive rectangle of the widget.
    fn rect(&self) -> Rect;

    /// The stable identity of the widget instance.
    ///
    /// The identity must survive a rebuild: it is what the router captures,
    /// what the focus slot points at and what the semantic tree is keyed by.
    fn key(&self) -> u64;

    /// Whether the widget refuses interaction.
    fn disabled(&self) -> bool {
        false
    }

    /// Whether the widget is toggled on.
    fn selected(&self) -> bool {
        false
    }

    /// Emit the widget's scene output for a state.
    ///
    /// `glyphs` carries the shaped text the widget draws; the emitter may
    /// position it but never re-shapes it.
    fn emit(&self, state: ControlState, glyphs: &mut [GlyphQuad]) -> WidgetOutput;
}

/// A headless session around one widget.
///
/// The harness owns the interaction session, re-derives the visual state from
/// the router after every input event and re-emits the widget only when that
/// state actually changed — the same redraw policy the window applies.
pub struct WidgetHarness<D: WidgetDesc> {
    /// The shared interaction session.
    pub app: TestApp,
    /// The descriptor under test.
    pub desc: D,
    /// The shaped glyphs the widget draws.
    pub glyphs: Vec<GlyphQuad>,
    /// The most recent emission.
    pub output: WidgetOutput,
    /// The state currently baked into `output`.
    baked: ControlState,
}

impl<D: WidgetDesc> WidgetHarness<D> {
    /// A session for the default viewport.
    #[must_use]
    pub fn new(desc: D, glyphs: Vec<GlyphQuad>) -> Self {
        Self::with_viewport(desc, glyphs, DEFAULT_VIEWPORT)
    }

    /// A session for an explicit viewport.
    #[must_use]
    pub fn with_viewport(desc: D, glyphs: Vec<GlyphQuad>, viewport: (f32, f32)) -> Self {
        let mut harness = Self {
            app: TestApp::new(viewport),
            desc,
            glyphs,
            output: WidgetOutput::default(),
            baked: ControlState::default(),
        };
        harness.rebuild();
        harness
    }

    /// The widget's identity.
    #[must_use]
    pub fn node_key(&self) -> u64 {
        self.desc.key()
    }

    /// The state derived from the router and the focus slot.
    #[must_use]
    pub fn state(&self) -> ControlState {
        ControlState::resolve(
            self.desc.key(),
            &self.app.router,
            self.app.focused_key(),
            self.desc.disabled(),
            self.desc.selected(),
        )
    }

    /// Emit the widget and install the result into the session.
    pub fn rebuild(&mut self) {
        let state = self.state();
        self.output = self.desc.emit(state, &mut self.glyphs);
        self.install();
        self.baked = state;
    }

    /// Re-emit the widget when the visual state changed.
    ///
    /// Returns whether the scene was rebuilt.
    pub fn sync(&mut self) -> bool {
        let state = self.state();
        if state == self.baked {
            return false;
        }
        self.rebuild();
        true
    }

    fn install(&mut self) {
        let key = self.desc.key();
        let mut semantics = Semantics::new(key);
        if let Some(node) = self.output.a11y.clone() {
            semantics.upsert(key, node);
        }
        let regions = self.output.hit.clone().into_iter().collect();
        self.app.install(regions, semantics);
    }

    /// The widget centre in window pixels.
    #[must_use]
    pub fn screen_center(&self) -> Vec2 {
        self.app.canvas_to_screen(self.desc.rect().center())
    }

    /// Move the pointer onto the widget.
    pub fn hover(&mut self) {
        self.hover_at(self.screen_center());
    }

    /// Move the pointer to a window position.
    pub fn hover_at(&mut self, screen: Vec2) {
        self.app.move_to(screen);
        self.sync();
    }

    /// Press the left button at the current pointer position.
    pub fn press(&mut self) {
        self.app.pointer_down();
        self.sync();
    }

    /// Release the left button at the current pointer position.
    pub fn release(&mut self) -> Vec<PointerEvent> {
        let events = self.app.pointer_up();
        self.sync();
        events
    }

    /// Hover, press and release the widget in one step.
    pub fn click(&mut self) -> ClickReport {
        let report = self.app.click(self.screen_center());
        self.sync();
        report
    }

    /// Give the widget keyboard focus.
    pub fn focus(&mut self) {
        self.app.set_focus_key(self.desc.key());
        self.sync();
    }

    /// Take keyboard focus away from the widget.
    pub fn blur(&mut self) {
        self.app.clear_focus();
        self.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_input::PointerConfig;

    /// A router with one square hit region owned by `node`.
    fn router_with(node: u64) -> Router {
        let mut router = Router::new(PointerConfig::default());
        router.set_regions(vec![HitRegion::new(
            node,
            1,
            0,
            Rect::new(0.0, 0.0, 100.0, 100.0),
        )]);
        router
    }

    #[test]
    fn the_pointer_phase_follows_the_router() {
        let mut router = router_with(7);
        router.pointer_move(Vec2::new(10.0, 10.0));
        let state = ControlState::resolve(7, &router, None, false, false);
        assert_eq!(state.phase, PointerPhase::Hovered);

        router.pointer_move(Vec2::new(500.0, 500.0));
        let state = ControlState::resolve(7, &router, None, false, false);
        assert_eq!(state.phase, PointerPhase::Idle);
    }

    #[test]
    fn a_captured_pointer_stays_pressed_after_it_leaves() {
        let mut router = router_with(7);
        router.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        // The pointer is captured, so moving away must not end the press.
        router.pointer_move(Vec2::new(500.0, 500.0));
        let state = ControlState::resolve(7, &router, None, false, false);
        assert_eq!(state.phase, PointerPhase::Pressed);
    }

    #[test]
    fn a_disabled_control_never_shows_a_pointer_phase() {
        let mut router = router_with(7);
        router.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        let state = ControlState::resolve(7, &router, None, true, false);
        assert_eq!(state.phase, PointerPhase::Idle);
        assert!(state.disabled);
    }

    #[test]
    fn the_focus_slot_is_orthogonal_to_the_pointer_phase() {
        let mut router = router_with(7);
        router.pointer_move(Vec2::new(10.0, 10.0));
        let state = ControlState::resolve(7, &router, Some(7), false, false);
        assert_eq!(state.phase, PointerPhase::Hovered);
        assert!(state.focused);

        // Focus on another widget leaves the phase alone.
        let state = ControlState::resolve(7, &router, Some(9), false, false);
        assert_eq!(state.phase, PointerPhase::Hovered);
        assert!(!state.focused);
    }

    #[test]
    fn the_collector_records_a_span_per_widget() {
        let mut collector = SceneCollector::new(1);
        let first = WidgetOutput {
            primitives: vec![GpuPrimitiveInstance::default(); 3],
            ..WidgetOutput::default()
        };
        let second = WidgetOutput {
            primitives: vec![GpuPrimitiveInstance::default(); 2],
            ..WidgetOutput::default()
        };
        collector.push(10, first);
        collector.push(20, second);

        assert_eq!(collector.node_spans.get(&10), Some(&(0, 3)));
        assert_eq!(collector.node_spans.get(&20), Some(&(3, 5)));
        assert_eq!(
            collector.span(20).map(<[GpuPrimitiveInstance]>::len),
            Some(2)
        );
        assert_eq!(collector.span(30), None);
    }

    #[test]
    fn a_focusable_widget_joins_the_tab_order() {
        let mut collector = SceneCollector::new(1);
        let output = WidgetOutput {
            focusable: true,
            ..WidgetOutput::default()
        };
        collector.push(10, output);
        assert_eq!(collector.focus_order, vec![10]);
    }
}
