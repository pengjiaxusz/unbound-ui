//! Stable widget identities for the gallery shell.
//!
//! The renderer never allocates per frame: every interactive control has a
//! deterministic `u64` key derived from its role and page position. The same
//! key travels through `unbound-input` (hit regions), `unbound-a11y`
//! (semantics) and the event reducer, so there is exactly one identity
//! namespace for the whole shell.

/// Everything the gallery can activate.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// Collapse / expand the navigation pane.
    NavToggle,
    /// Switch to light (`0`) or dark (`1`).
    Theme(u8),
    /// Pick a UI zoom step (`0` = 100 %, `3` = 200 %).
    Zoom(u8),
    /// Toggle static pixel snapping.
    Snap,
    /// Toggle the performance HUD.
    Hud,
    /// Focus the global search box.
    Search,
    /// Jump to a search result.
    SearchResult(u8),
    /// Navigate to a page.
    NavPage(u8),
    /// Expand / collapse an example card's code panel.
    Code(u8, u8),
    /// Activate a knob (toggle / checkbox row).
    Knob(u8, u8, u8),
    /// Pick a segmented-control option.
    KnobOption(u8, u8, u8, u8),
    /// Activate a part of an example's live stage.
    Demo(u8, u8, u16),
    /// Footer shortcut: `0` telemetry, `1` settings.
    Footer(u8),
    /// Minimise the native window.
    WindowMinimize,
    /// Toggle the native window's maximised state.
    WindowMaximize,
    /// Close the native window.
    WindowClose,
    /// Drag the page's vertical scrollbar.
    ContentScrollbar,
}

const TAG_NAV_TOGGLE: u64 = 1;
const TAG_THEME: u64 = 2;
const TAG_ZOOM: u64 = 3;
const TAG_SNAP: u64 = 4;
const TAG_HUD: u64 = 5;
const TAG_SEARCH: u64 = 6;
const TAG_SEARCH_RESULT: u64 = 7;
const TAG_NAV_PAGE: u64 = 8;
const TAG_CODE: u64 = 9;
const TAG_KNOB: u64 = 10;
const TAG_KNOB_OPTION: u64 = 11;
const TAG_DEMO: u64 = 12;
const TAG_FOOTER: u64 = 13;
const TAG_WINDOW_MINIMIZE: u64 = 14;
const TAG_WINDOW_MAXIMIZE: u64 = 15;
const TAG_WINDOW_CLOSE: u64 = 16;
const TAG_CONTENT_SCROLLBAR: u64 = 17;

/// The semantic root of the gallery.
pub const ROOT: u64 = 1;

/// Whether an identity belongs to an example's live stage.
///
/// A demo is a bespoke drawing: unlike a standard control it may reveal
/// widgets when hovered (a table row highlight, a dialog shadow), so it is
/// allowed to change the scene's shape.
#[must_use]
pub const fn is_demo(node: u64) -> bool {
    node >> 56 == TAG_DEMO
}

const fn tag(value: u64) -> u64 {
    value << 56
}

/// Encode a target into the shared identity namespace.
#[must_use]
pub const fn id(target: Target) -> u64 {
    match target {
        Target::NavToggle => tag(TAG_NAV_TOGGLE),
        Target::Theme(bit) => tag(TAG_THEME) | bit as u64,
        Target::Zoom(step) => tag(TAG_ZOOM) | step as u64,
        Target::Snap => tag(TAG_SNAP),
        Target::Hud => tag(TAG_HUD),
        Target::Search => tag(TAG_SEARCH),
        Target::SearchResult(page) => tag(TAG_SEARCH_RESULT) | page as u64,
        Target::NavPage(page) => tag(TAG_NAV_PAGE) | page as u64,
        Target::Code(page, card) => tag(TAG_CODE) | ((page as u64) << 8) | card as u64,
        Target::Knob(page, card, knob) => {
            tag(TAG_KNOB) | ((page as u64) << 16) | ((card as u64) << 8) | knob as u64
        }
        Target::KnobOption(page, card, knob, option) => {
            tag(TAG_KNOB_OPTION)
                | ((page as u64) << 24)
                | ((card as u64) << 16)
                | ((knob as u64) << 8)
                | option as u64
        }
        Target::Demo(page, card, part) => {
            tag(TAG_DEMO) | ((page as u64) << 24) | ((card as u64) << 16) | part as u64
        }
        Target::Footer(index) => tag(TAG_FOOTER) | index as u64,
        Target::WindowMinimize => tag(TAG_WINDOW_MINIMIZE),
        Target::WindowMaximize => tag(TAG_WINDOW_MAXIMIZE),
        Target::WindowClose => tag(TAG_WINDOW_CLOSE),
        Target::ContentScrollbar => tag(TAG_CONTENT_SCROLLBAR),
    }
}

/// Decode an identity produced by [`id`].
#[must_use]
pub const fn decode(value: u64) -> Option<Target> {
    let tag = value >> 56;
    let low = value & 0x00ff_ffff_ffff_ffff;
    Some(match tag {
        TAG_NAV_TOGGLE => Target::NavToggle,
        TAG_THEME => Target::Theme((low & 0xff) as u8),
        TAG_ZOOM => Target::Zoom((low & 0xff) as u8),
        TAG_SNAP => Target::Snap,
        TAG_HUD => Target::Hud,
        TAG_SEARCH => Target::Search,
        TAG_SEARCH_RESULT => Target::SearchResult((low & 0xff) as u8),
        TAG_NAV_PAGE => Target::NavPage((low & 0xff) as u8),
        TAG_CODE => Target::Code(((low >> 8) & 0xff) as u8, (low & 0xff) as u8),
        TAG_KNOB => Target::Knob(
            ((low >> 16) & 0xff) as u8,
            ((low >> 8) & 0xff) as u8,
            (low & 0xff) as u8,
        ),
        TAG_KNOB_OPTION => Target::KnobOption(
            ((low >> 24) & 0xff) as u8,
            ((low >> 16) & 0xff) as u8,
            ((low >> 8) & 0xff) as u8,
            (low & 0xff) as u8,
        ),
        TAG_DEMO => Target::Demo(
            ((low >> 24) & 0xff) as u8,
            ((low >> 16) & 0xff) as u8,
            (low & 0xffff) as u16,
        ),
        TAG_FOOTER => Target::Footer((low & 0xff) as u8),
        TAG_WINDOW_MINIMIZE => Target::WindowMinimize,
        TAG_WINDOW_MAXIMIZE => Target::WindowMaximize,
        TAG_WINDOW_CLOSE => Target::WindowClose,
        TAG_CONTENT_SCROLLBAR => Target::ContentScrollbar,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_round_trips_through_its_identity() {
        let targets = [
            Target::NavToggle,
            Target::Theme(1),
            Target::Zoom(3),
            Target::Snap,
            Target::Hud,
            Target::Search,
            Target::SearchResult(7),
            Target::NavPage(12),
            Target::Code(3, 2),
            Target::Knob(4, 1, 2),
            Target::KnobOption(4, 1, 2, 3),
            Target::Demo(5, 2, 500),
            Target::Footer(1),
            Target::WindowMinimize,
            Target::WindowMaximize,
            Target::WindowClose,
            Target::ContentScrollbar,
        ];
        for target in targets {
            assert_eq!(decode(id(target)), Some(target), "{target:?}");
            assert_ne!(id(target), ROOT);
        }
    }

    #[test]
    fn ids_are_unique_across_fields() {
        assert_ne!(id(Target::Code(1, 2)), id(Target::Code(2, 1)));
        assert_ne!(id(Target::NavPage(2)), id(Target::NavPage(3)));
        assert_ne!(
            id(Target::Knob(1, 2, 3)),
            id(Target::KnobOption(1, 2, 3, 0))
        );
    }
}
