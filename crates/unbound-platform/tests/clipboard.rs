//! Clipboard contract through the public API.

use unbound_platform::{
    ClipboardError, ClipboardProvider, MemoryClipboard, SystemClipboard, MAX_CLIPBOARD_BYTES,
};

#[test]
fn the_memory_clipboard_round_trips_and_clears() {
    let mut clipboard = MemoryClipboard::new();
    assert_eq!(clipboard.read_text(), Ok(None));
    clipboard.write_text("unbound").unwrap();
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("unbound"));
    clipboard.clear().unwrap();
    assert_eq!(clipboard.read_text(), Ok(None));
}

#[test]
fn the_byte_limit_is_enforced_before_storing() {
    let mut clipboard = MemoryClipboard::with_limit(4);
    let error = clipboard.write_text("12345").unwrap_err();
    assert_eq!(error, ClipboardError::TooLarge { bytes: 5, limit: 4 });
    assert_eq!(clipboard.peek(), None);
}

#[test]
fn the_system_clipboard_never_exceeds_the_limit() {
    let mut clipboard = SystemClipboard::new();
    match clipboard.read_text() {
        Ok(Some(text)) => assert!(text.len() <= MAX_CLIPBOARD_BYTES),
        Ok(None) | Err(ClipboardError::Unavailable) | Err(ClipboardError::Os) => {}
        Err(ClipboardError::TooLarge { limit, .. }) => assert_eq!(limit, MAX_CLIPBOARD_BYTES),
    }
}
