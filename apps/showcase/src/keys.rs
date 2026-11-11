//! 键盘到演示动作的映射。
//!
//! 窗口与无头测试共用这一张表：窗口只负责把 [`KeyAction`] 派发到具体方法，
//! 测试则直接断言映射结果并驱动 [`crate::testing::ButtonHarness`]。如果映射
//! 逻辑留在 `main.rs` 里，无头测试就只能复制一份，测到的永远是副本。

use unbound_platform::PlatformKey;

/// 一次按键要求演示执行的动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// `T`：切换主题。
    ToggleTheme,
    /// `Tab`：把焦点给按钮。
    FocusButton,
    /// `Escape`：清除焦点。
    ClearFocus,
    /// `Enter` / `Space`：激活当前聚焦的按钮。
    Activate,
}

/// 按当前焦点解析一次按键。
///
/// `T` 在任何时候都切换主题；`Space` 在按钮聚焦时激活按钮，否则保持原来的
/// 切换主题语义；`Enter` 只在按钮聚焦时激活。
#[must_use]
pub fn resolve(key: &PlatformKey, button_focused: bool) -> Option<KeyAction> {
    match key {
        PlatformKey::Character(text) if text.eq_ignore_ascii_case("t") => {
            Some(KeyAction::ToggleTheme)
        }
        PlatformKey::Named(name) if name.eq_ignore_ascii_case("tab") => {
            Some(KeyAction::FocusButton)
        }
        PlatformKey::Named(name) if name.eq_ignore_ascii_case("escape") => {
            Some(KeyAction::ClearFocus)
        }
        PlatformKey::Named(name) if name.eq_ignore_ascii_case("enter") => {
            button_focused.then_some(KeyAction::Activate)
        }
        PlatformKey::Named(name) if name.eq_ignore_ascii_case("space") => Some(if button_focused {
            KeyAction::Activate
        } else {
            KeyAction::ToggleTheme
        }),
        _ => None,
    }
}
