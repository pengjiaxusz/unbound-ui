//! 像素一致性基准的「我们这边」：把单个试点控件离屏渲染成 PNG。
//!
//! 差分闭环只比较同一份输入：`tools/winui-reference` 采集的官方 WinUI 3 截图
//! 给出画布尺寸、控件边界与主题，本模块把 unbound-ui 的同一个控件按**完全相同的
//! 矩形**渲染到同样大小的画布上，再把两张 PNG 交给 [`crate::png_diff`]。
//!
//! 控件边界取自参考 JSON 而非本框架的固有尺寸：这样对比的是「同样大小的控件
//! 长什么样」，固有尺寸偏差则由单独的 `intrinsicWidth/Height` 字段暴露出来。

use std::sync::Arc;

use unbound_gpu::atlas::{AtlasKind, AtlasSet, AtlasUpload};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::{Camera, Renderer};
use unbound_math::{Rect, Vec2};
use unbound_widgets::{
    ButtonDesc, ButtonVariant, CheckboxDesc, ControlState, PointerPhase, SelectDesc, SliderDesc,
    SwitchDesc, TextCtx, TextInputDesc, UiWidget,
};

use crate::gallery::{palette, Gallery, Labels};
use crate::shot::write_bgra_png;
use crate::theme;

/// 试点控件清单，与 `tools/winui-reference/Catalog.cs` 一一对应。
pub const PILOT_CONTROLS: [&str; 6] = [
    "Button",
    "CheckBox",
    "ToggleSwitch",
    "Slider",
    "ComboBox",
    "TextBox",
];

/// 单个对比用例。
pub struct ParityCase<'a> {
    /// 控件规范名。
    pub control: &'a str,
    /// 状态：`normal` / `disabled` / `checked`。
    pub state: &'a str,
    /// 主题：`light` / `dark`。
    pub theme: &'a str,
}

/// 场景中唯一的控件节点。
const NODE: u64 = 1;

/// 渲染一个用例到 `png_path`，并写出与参考 JSON 同构的元数据。
///
/// 返回本框架该控件的固有尺寸，供差分器报告「自然大小 vs 参考边界」的偏差。
pub fn render_case(
    case: &ParityCase<'_>,
    width: u32,
    height: u32,
    bounds: Rect,
    png_path: &str,
) -> Result<Vec2, String> {
    let dark = case.theme == "dark";
    let tokens = palette();

    let mut scene = Gallery::new();
    scene.camera = Camera::ui((width as f32, height as f32), 1.0);
    // 参考宿主的 `Host` 背景是 `ApplicationPageBackgroundThemeBrush`，
    // 与本框架的 `SURFACE` 是同一个 Win11 基准面（浅色 #F3F3F3 / 深色 #202020）。
    scene.rect(
        0.0,
        0.0,
        width as f32,
        height as f32,
        0.0,
        tokens.background,
        0,
        0.0,
        0.0,
        0,
    );

    let mut labels = Labels::new();
    labels.set_raster_scale(1.0);

    let intrinsic = {
        let mut text_ctx = TextCtx {
            engine: &mut labels.engine,
            cache: &mut labels.cache,
            atlas_clock: 0,
            raster_scale: 1.0,
        };
        emit_case(case, bounds, &mut scene, &mut text_ctx)?
    };

    let gpu = Gpu::new(GpuConfig::default()).map_err(|error| error.diagnostic_report())?;
    let mut renderer =
        Renderer::new(gpu, width, height).map_err(|error| error.diagnostic_report())?;
    let atlases =
        Arc::new(AtlasSet::new(&renderer.gpu, 64).map_err(|error| error.diagnostic_report())?);
    renderer
        .install_atlases(atlases.clone())
        .map_err(|error| error.diagnostic_report())?;

    for update in labels.cache.take_updates() {
        let kind = match update.target {
            unbound_text::AtlasTarget::Coverage => AtlasKind::R8,
            unbound_text::AtlasTarget::Color => AtlasKind::Rgba8,
        };
        atlases
            .upload(
                &renderer.gpu,
                &AtlasUpload {
                    kind,
                    layer: update.layer,
                    x: update.x,
                    y: update.y,
                    width: update.width,
                    height: update.height,
                    data: update.data,
                },
            )
            .map_err(|error| error.to_string())?;
    }

    renderer
        .write_theme_lut(&theme::lut(dark))
        .map_err(|error| error.to_string())?;
    renderer
        .render(&scene.upload(1, &[]))
        .map_err(|error| error.to_string())?;
    let pixels = renderer.read_pixels().map_err(|error| error.to_string())?;
    renderer.destroy();
    write_bgra_png(png_path, width, height, &pixels)?;

    let json_path = format!("{}.json", png_path.trim_end_matches(".png"));
    let meta = format!(
        "{{\n  \"control\": \"{}\",\n  \"state\": \"{}\",\n  \"theme\": \"{}\",\n  \"imagePixelWidth\": {},\n  \"imagePixelHeight\": {},\n  \"controlBounds\": {{ \"x\": {}, \"y\": {}, \"width\": {}, \"height\": {} }},\n  \"intrinsicWidth\": {},\n  \"intrinsicHeight\": {}\n}}\n",
        case.control,
        case.state,
        case.theme,
        width,
        height,
        bounds.origin.x,
        bounds.origin.y,
        bounds.size.x,
        bounds.size.y,
        intrinsic.x,
        intrinsic.y,
    );
    std::fs::write(&json_path, meta).map_err(|error| format!("{json_path}: {error}"))?;

    Ok(intrinsic)
}

/// 按用例名构造描述符并发射图元，返回固有尺寸。
fn emit_case(
    case: &ParityCase<'_>,
    rect: Rect,
    scene: &mut Gallery,
    text_ctx: &mut TextCtx<'_>,
) -> Result<Vec2, String> {
    let control = ControlState {
        phase: PointerPhase::Idle,
        focused: false,
        disabled: case.state == "disabled",
        selected: case.state == "checked",
    };
    let checked = case.state == "checked";

    let intrinsic = match case.control {
        "Button" => {
            let desc = ButtonDesc::label("Button").variant(ButtonVariant::Secondary);
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &(), text_ctx),
            );
            size
        }
        "CheckBox" => {
            let desc = CheckboxDesc::labeled("CheckBox", checked);
            let state = desc.initial_state();
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            size
        }
        "ToggleSwitch" => {
            let desc = SwitchDesc::labeled("ToggleSwitch", checked);
            let state = desc.initial_state();
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            size
        }
        "Slider" => {
            // WinUI Slider 在没有 Header 时不画标签，描述符也不应携带文案。
            let desc = SliderDesc::range(0.0, 100.0, 50.0).label("");
            let state = desc.initial_state();
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            size
        }
        "ComboBox" => {
            let desc = SelectDesc::options(["Item 1", "Item 2", "Item 3"]).selected(0);
            let state = desc.initial_state();
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            size
        }
        "TextBox" => {
            let desc = TextInputDesc::text("Text");
            let state = desc.initial_state();
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            size
        }
        other => return Err(format!("unknown pilot control: {other}")),
    };

    Ok(intrinsic)
}

/// 把一次发射的图元与载荷追加到场景。
fn push(scene: &mut Gallery, output: unbound_widgets::WidgetOutput) {
    scene.primitives.extend(output.primitives);
    scene.payloads.extend(output.payloads);
}
