//! The gallery's single source of truth.
//!
//! Navigation, environment switches, knob values and the live demo state all
//! live here. Pages are pure functions of this state, which is what makes the
//! whole shell testable without a window.

use std::collections::{HashMap, HashSet};

use unbound_anim::{Spring, SpringPreset};
use unbound_math::Vec2;
use unbound_widgets::progress::ProgressState;

use crate::pages::{ExampleDef, KnobSpec, PageId};

/// The two shipped application themes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    #[must_use]
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}

/// One selectable UI scale step.
pub const ZOOM_STEPS: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

/// Runtime state that is not derived from an example definition.
#[derive(Debug)]
pub struct DemoState {
    /// The text-field demo's buffer.
    pub text: String,
    /// Whether the text field owns the keyboard.
    pub typing: bool,
    /// A dropdown / modal in the Overlays demo.
    pub dropdown_open: bool,
    pub modal_open: bool,
    pub toggle_a: bool,
    pub toggle_b: bool,
    pub toggle_c: bool,
    pub radio_choice: usize,
    pub pill_choice: usize,
    pub sliders: [f32; 3],
    pub drag_order: Vec<usize>,
    pub drag_active: Option<usize>,
    pub drag_position: Option<Vec2>,
    pub list_scroll: [f32; 2],
    pub list_selected: [Option<usize>; 2],
    pub canvas_pan: Vec2,
    pub canvas_zoom: f32,
    pub marquee: Option<(Vec2, Vec2)>,
    pub selected_nodes: [bool; 12],
    pub history: Vec<&'static str>,
    pub history_index: usize,
    pub spring: Spring,
    /// The segmented pill's animated slot position.
    pub pill_spring: Spring,
    pub spring_high: bool,
    pub a11y_focus: usize,
    pub frame_ms: f32,
    pub fps: f32,
    pub upload_bytes: u32,
    pub primitive_count: u32,
    pub draw_calls: u32,
    pub elapsed: f32,
    /// Button demo activation count.
    pub clicks: u32,
    /// The last icon-button selected in the icon variant.
    pub selected_icon: usize,
    /// The progress demo's eased bar display.
    pub progress_bar: ProgressState,
    /// The progress demo's eased ring display.
    pub progress_ring: ProgressState,
}

impl Default for DemoState {
    fn default() -> Self {
        let mut spring = Spring::new(SpringPreset::Snappy, 0.0);
        spring.set_target(0.0);
        // The default pill sits in slot 1 ("All"), so the spring starts there
        // instead of sliding in from the left on the first frame.
        let mut pill_spring = Spring::new(SpringPreset::Snappy, 1.0);
        pill_spring.snap_to(1.0);
        Self {
            text: "Unbound UI".to_string(),
            typing: false,
            dropdown_open: false,
            modal_open: false,
            toggle_a: true,
            toggle_b: false,
            toggle_c: true,
            radio_choice: 0,
            pill_choice: 1,
            sliders: [0.62, 0.35, 0.8],
            drag_order: vec![0, 1, 2, 3],
            drag_active: None,
            drag_position: None,
            list_scroll: [0.0, 0.0],
            list_selected: [Some(2), Some(2)],
            canvas_pan: Vec2::new(18.0, 12.0),
            canvas_zoom: 1.0,
            marquee: None,
            selected_nodes: [false; 12],
            history: vec!["Open canvas", "Add node", "Move node"],
            history_index: 3,
            spring,
            pill_spring,
            spring_high: false,
            a11y_focus: 0,
            frame_ms: 4.2,
            fps: 120.0,
            upload_bytes: 0,
            primitive_count: 0,
            draw_calls: 4,
            elapsed: 0.0,
            clicks: 0,
            selected_icon: 0,
            progress_bar: ProgressState::new(),
            progress_ring: ProgressState::new(),
        }
    }
}

impl DemoState {
    /// Select a segmented slot with the sliding pill.
    pub fn set_pill(&mut self, index: usize) {
        self.pill_choice = index;
        self.pill_spring.set_target(index as f32);
    }
}

/// The whole gallery state.
#[derive(Debug)]
pub struct GalleryState {
    pub active_page: PageId,
    pub nav_expanded: bool,
    pub search_query: String,
    pub search_focused: bool,
    /// The input method's composition string while a text control is focused.
    ///
    /// Kept in the state (instead of in the renderer) so the page and the
    /// search field draw exactly what the IME is composing.
    pub ime_preedit: String,
    pub theme: ThemeMode,
    pub zoom_step: usize,
    /// Whether the pixel grid is snapped.
    pub pixel_snap: bool,
    /// The frame-rate limit in FPS; `0` means unlimited.
    pub fps_limit: u32,
    /// The custom FPS-limit input buffer.
    pub fps_input: String,
    /// Whether the custom FPS input owns the keyboard.
    pub fps_focused: bool,
    pub show_hud: bool,
    pub content_scroll: f32,
    pub nav_scroll: f32,
    pub knobs: HashMap<u32, f32>,
    pub expanded_code: HashSet<(u8, u8)>,
    pub demo: DemoState,
    /// Bumped on every state change, so the window can skip identical frames.
    pub revision: u64,
    /// Accumulator that throttles demo animation rebuilds to ~30 Hz.
    animation_clock: f32,
}

impl Default for GalleryState {
    fn default() -> Self {
        Self {
            active_page: PageId::Home,
            nav_expanded: true,
            search_query: String::new(),
            search_focused: false,
            ime_preedit: String::new(),
            theme: ThemeMode::Dark,
            zoom_step: 0,
            pixel_snap: true,
            fps_limit: 120,
            fps_input: "120".to_string(),
            fps_focused: false,
            // The HUD is a floating popover opened from the top bar; showing
            // it by default covered the hero card at 960×600.
            show_hud: false,
            content_scroll: 0.0,
            nav_scroll: 0.0,
            knobs: HashMap::new(),
            expanded_code: HashSet::new(),
            demo: DemoState::default(),
            revision: 1,
            animation_clock: 0.0,
        }
    }
}

impl GalleryState {
    /// Mark the state dirty.
    pub fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// The current UI scale.
    #[must_use]
    pub fn zoom(&self) -> f32 {
        ZOOM_STEPS[self.zoom_step.min(ZOOM_STEPS.len() - 1)]
    }

    /// Navigate to a page; content scroll resets so the header is visible.
    pub fn navigate(&mut self, page: PageId) {
        if self.active_page != page {
            self.active_page = page;
            self.content_scroll = 0.0;
            self.demo.modal_open = false;
            self.demo.dropdown_open = false;
            self.bump();
        }
    }

    /// Set the frame-rate limit.
    ///
    /// `0` is unlimited; any other value is clamped to the engine's 60 FPS
    /// floor. The text buffer is normalised so the UI always shows the value
    /// that is actually in effect.
    pub fn set_fps_limit(&mut self, fps: u32) {
        self.fps_limit = if fps == 0 {
            0
        } else {
            fps.max(unbound_platform::MIN_FPS_LIMIT)
        };
        self.fps_input = self.fps_limit.to_string();
        self.bump();
    }

    /// The limit the pacer should currently use.
    ///
    /// While the user is typing `"1"` on the way to `"144"`, the non-zero
    /// partial value is raised to the 60 FPS floor instead of stalling the
    /// renderer at 1 FPS.
    #[must_use]
    pub fn effective_fps_limit(&self) -> u32 {
        if !self.fps_focused {
            return self.fps_limit;
        }
        match self.fps_input.trim().parse::<u32>() {
            Ok(0) => 0,
            Ok(fps) => fps.max(unbound_platform::MIN_FPS_LIMIT),
            Err(_) => self.fps_limit,
        }
    }

    /// Apply the custom input: normalise the text and commit the limit.
    pub fn commit_fps_input(&mut self) {
        let parsed = self
            .fps_input
            .trim()
            .parse::<u32>()
            .unwrap_or(self.fps_limit);
        self.set_fps_limit(parsed);
        self.fps_focused = false;
    }

    /// The FPS preset index that matches the current limit, if any.
    #[must_use]
    pub fn fps_preset(&self) -> Option<u8> {
        match self.fps_limit {
            60 => Some(0),
            120 => Some(1),
            144 => Some(2),
            240 => Some(3),
            0 => Some(4),
            _ => None,
        }
    }

    /// The key of one knob.
    #[must_use]
    pub fn knob_key(page: PageId, card: usize, knob: usize) -> u32 {
        ((page.index() as u32) << 16) | ((card as u32) << 8) | knob as u32
    }

    /// A knob's current value, falling back to its declared default.
    #[must_use]
    pub fn knob(&self, page: PageId, card: usize, knob: usize, spec: &KnobSpec) -> f32 {
        self.knobs
            .get(&Self::knob_key(page, card, knob))
            .copied()
            .unwrap_or_else(|| spec.default_value())
    }

    /// All knob values for one example.
    #[must_use]
    pub fn knob_values(&self, page: PageId, card: usize, example: &ExampleDef) -> Vec<f32> {
        example
            .knobs
            .iter()
            .enumerate()
            .map(|(index, spec)| self.knob(page, card, index, spec))
            .collect()
    }

    /// Write a knob value.
    pub fn set_knob(&mut self, page: PageId, card: usize, knob: usize, value: f32) {
        self.knobs.insert(Self::knob_key(page, card, knob), value);
        self.bump();
    }

    /// Toggle a code panel.
    pub fn toggle_code(&mut self, page: PageId, card: u8) {
        let key = (page.index() as u8, card);
        if !self.expanded_code.remove(&key) {
            self.expanded_code.insert(key);
        }
        self.bump();
    }

    /// Whether a code panel is open.
    #[must_use]
    pub fn code_open(&self, page: PageId, card: u8) -> bool {
        self.expanded_code.contains(&(page.index() as u8, card))
    }

    /// Advance time-driven demos. Returns `true` while something moved.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.demo.elapsed = (self.demo.elapsed + dt).rem_euclid(3600.0);
        // The progress demo eases its bar and ring toward the clock-driven
        // target through the real `ProgressState`, so a value jump reads as
        // movement. Pausing freezes both the target and the indeterminate sweep.
        let progress_example = &PageId::Sliders.examples()[1];
        let progress_knobs = self.knob_values(PageId::Sliders, 1, progress_example);
        let paused = progress_knobs.get(1).copied().unwrap_or(0.0) > 0.5;
        let target = if paused {
            0.64
        } else {
            (self.demo.elapsed * 0.18).rem_euclid(1.0)
        };
        self.demo.progress_bar.tick(dt, Some(target), !paused);
        self.demo.progress_ring.tick(dt, Some(target), !paused);
        let mut moving = false;
        self.demo.spring.step(dt);
        if !self.demo.spring.is_settled() {
            moving = true;
        }
        self.demo.pill_spring.step(dt);
        if !self.demo.pill_spring.is_settled() {
            moving = true;
        }
        // Re-shape and repaint animated pages at ~30 Hz. Static pages never
        // rebuild, which is what keeps the gallery idle-light.
        let animated = matches!(
            self.active_page,
            PageId::Assets | PageId::Sliders | PageId::Telemetry | PageId::Springs
        );
        self.animation_clock += dt;
        if animated && self.animation_clock >= 1.0 / 30.0 {
            self.animation_clock = 0.0;
            moving = true;
        }
        if moving {
            self.bump();
        }
        moving
    }

    /// Push a new operation onto the undo timeline.
    pub fn push_history(&mut self, label: &'static str) {
        self.demo.history.truncate(self.demo.history_index);
        self.demo.history.push(label);
        self.demo.history_index = self.demo.history.len();
        self.bump();
    }

    /// Undo one operation.
    pub fn undo(&mut self) {
        if self.demo.history_index > 0 {
            self.demo.history_index -= 1;
            self.bump();
        }
    }

    /// Redo one operation.
    pub fn redo(&mut self) {
        if self.demo.history_index < self.demo.history.len() {
            self.demo.history_index += 1;
            self.bump();
        }
    }

    /// Filter the page catalogue by the search query.
    #[must_use]
    pub fn search_results(&self) -> Vec<PageId> {
        let query = self.search_query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        PageId::ALL
            .iter()
            .copied()
            .filter(|page| page.matches(&query))
            .take(4)
            .collect()
    }
}
