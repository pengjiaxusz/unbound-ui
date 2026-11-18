//! 首批 6 个试点基准控件的几何门禁。
//!
//! 这组断言锁定 WinUI 3 参考截图中可被「自由维度固有尺寸」门禁直接观测的常数：
//! 单行控件必须严格落在 `32px` 标准交互高度上，`ToggleSwitch` 的 Header 排版
//! 必须保持「20px 标签行 + 13px 间距 + 20px 轨道」的三段式高度。
//! 若未来有人把输入框改回 34px 或把下拉框改回 36px，本测试会先于像素差分失败。

use unbound_math::Rect;
use unbound_text::{GlyphCache, TextEngine};
use unbound_widgets::{
    AutoCompleteDesc, ButtonDesc, ButtonVariant, CheckboxDesc, NumberInputDesc, SelectDesc,
    SliderDesc, SwitchDesc, SwitchLayout, TextCtx, TextInputDesc, UiWidget,
};

/// 以 32px 标准高度构造试点控件并测量其固有尺寸。
fn measure<T: UiWidget>(desc: T) -> unbound_math::Vec2 {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut text_ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };
    desc.intrinsic_size(&mut text_ctx)
}

fn row(width: f32) -> Rect {
    Rect::new(0.0, 0.0, width, 32.0)
}

#[test]
fn the_button_stays_on_the_32px_control_row() {
    let size = measure(ButtonDesc::label("Button").variant(ButtonVariant::Secondary));
    assert_eq!(size.y, 32.0, "WinUI standard Button height is 32px");
    assert!(size.x >= 56.0, "the button keeps its minimum hit width");
}

#[test]
fn the_checkbox_stays_on_the_32px_control_row() {
    let mut desc = CheckboxDesc::labeled("CheckBox", false);
    desc.rect = row(120.0);
    let size = measure(desc);
    assert_eq!(size.y, 32.0, "WinUI CheckBox height is 32px");
    assert!(size.x >= 120.0, "WinUI CheckBox MinWidth is 120px");
}

#[test]
fn the_toggle_switch_header_keeps_its_three_row_stack() {
    let mut desc = SwitchDesc::labeled("ToggleSwitch", false).with_layout(SwitchLayout::Header);
    desc.rect = row(200.0);
    let size = measure(desc);
    // 20px 标签行 + 13px 间距 + 20px 轨道：Header 排版不能被压回 32px 单行。
    assert_eq!(size.y, 53.0, "ToggleSwitch(Header) stacks label and track");
}

#[test]
fn the_slider_keeps_the_32px_interaction_row() {
    let desc = SliderDesc::range(0.0, 100.0, 50.0).label("");
    let size = measure(desc);
    assert_eq!(size.y, 32.0, "Slider without Header is 32px tall");
}

#[test]
fn the_combo_box_trigger_is_32px_tall() {
    let desc = SelectDesc::options(["Item 1", "Item 2", "Item 3"]).selected(0);
    let size = measure(desc);
    assert_eq!(size.y, 32.0, "ComboBox trigger height is 32px");
}

#[test]
fn the_number_box_and_auto_suggest_box_share_the_32px_row() {
    let number = measure(NumberInputDesc::new(Rect::ZERO, 0.0, 100.0));
    assert_eq!(number.y, 32.0, "NumberBox height is 32px");
    let suggest = measure(AutoCompleteDesc::new(Rect::ZERO, "Text", 3));
    assert_eq!(
        suggest.y, 32.0,
        "AutoSuggestBox must not fall back to the old 34px height"
    );
}

#[test]
fn the_text_box_is_32px_tall_not_34() {
    let desc = TextInputDesc::text("Text");
    let size = measure(desc);
    assert_eq!(
        size.y, 32.0,
        "TextBox shares the ComboBox 32px row, not the old 34px fallback"
    );
}
