//! 像素一致性基准的「我们这边」：把试点控件离屏渲染成 PNG，支持单实例渲染器复用、
//! 源码哈希指纹缓存与确定性后景合成底板。
//!
//! 差分闭环只比较同一份输入：`tools/winui-reference` 采集的官方 WinUI 3 截图
//! 给出画布尺寸、控件边界、主题与后景类型，本模块把 unbound-ui 的同一个控件按
//! **完全相同的矩形**渲染到同样大小的画布上，再把两张 PNG 交给 [`crate::png_diff`]。
//!
//! 控件边界取自参考 JSON 而非本框架的固有尺寸：这样对比的是「同样大小的控件
//! 长什么样」，固有尺寸偏差则由单独的 `intrinsicWidth/Height` 字段与四重门禁暴露出来。

use std::path::Path;
use std::sync::Arc;

use unbound_gpu::atlas::{AtlasKind, AtlasSet, AtlasUpload};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::{Camera, Renderer};
use unbound_math::color::Color;
use unbound_math::{Rect, Vec2};
use unbound_theme::{Palette, ThemeKind, ThemeLut, Token};
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

/// 支持的确定性后景合成底板类型。
pub const BACKDROP_KINDS: [&str; 4] = ["solid", "mica", "mica_alt", "acrylic"];

/// 单个对比用例（四维坐标：控件 × 状态 × 主题 × 后景底板）。
pub struct ParityCase<'a> {
    /// 控件规范名。
    pub control: &'a str,
    /// 状态：`normal` / `disabled` / `checked` / `focused`。
    pub state: &'a str,
    /// 主题：`light` / `dark`。
    pub theme: &'a str,
    /// 后景合成底板：`solid` / `mica` / `mica_alt` / `acrylic`。
    pub backdrop: &'a str,
}

impl<'a> ParityCase<'a> {
    /// 以默认 `solid` 纯色基准面构造用例。
    #[must_use]
    pub const fn new(control: &'a str, state: &'a str, theme: &'a str) -> Self {
        Self {
            control,
            state,
            theme,
            backdrop: "solid",
        }
    }

    /// 指定后景合成底板类型。
    #[must_use]
    pub const fn with_backdrop(mut self, backdrop: &'a str) -> Self {
        self.backdrop = backdrop;
        self
    }
}

/// 单次渲染或缓存命中的结果。
#[derive(Clone, Copy, Debug)]
pub struct RenderOutcome {
    /// 本框架该控件的固有尺寸。
    pub intrinsic: Vec2,
    /// 本次是否命中源码指纹缓存并跳过了 GPU 渲染。
    pub cache_hit: bool,
}

/// 批量差分复用的离屏渲染会话：在整个 `--parity-run` 期间只初始化一次 Vulkan 设备、
/// 渲染器、字形图集与文本引擎，彻底消除逐用例重建 GPU 资源的开销。
pub struct ParitySession {
    renderer: Renderer,
    atlases: Arc<AtlasSet>,
    labels: Labels,
    generation: u64,
}

impl ParitySession {
    /// 创建可复用的批量离屏渲染会话。
    pub fn new(initial_width: u32, initial_height: u32) -> Result<Self, String> {
        let width = initial_width.max(64);
        let height = initial_height.max(64);
        let gpu = Gpu::new(GpuConfig::default()).map_err(|error| error.diagnostic_report())?;
        let mut renderer =
            Renderer::new(gpu, width, height).map_err(|error| error.diagnostic_report())?;
        let atlases =
            Arc::new(AtlasSet::new(&renderer.gpu, 64).map_err(|error| error.diagnostic_report())?);
        renderer
            .install_atlases(atlases.clone())
            .map_err(|error| error.diagnostic_report())?;
        let mut labels = Labels::new();
        labels.set_raster_scale(1.0);
        Ok(Self {
            renderer,
            atlases,
            labels,
            generation: 1,
        })
    }

    /// 渲染单个用例（若源码与输入指纹命中缓存则直接跳过 GPU 绘制）。
    pub fn render_case_cached(
        &mut self,
        case: &ParityCase<'_>,
        width: u32,
        height: u32,
        bounds: Rect,
        png_path: &str,
        allow_cache: bool,
    ) -> Result<RenderOutcome, String> {
        let json_path = format!("{}.json", png_path.trim_end_matches(".png"));
        let fingerprint = compute_source_fingerprint(case, width, height, bounds);

        if allow_cache && Path::new(png_path).is_file() && Path::new(&json_path).is_file() {
            if let Ok(existing_meta) = std::fs::read_to_string(&json_path) {
                let needle = format!("\"fingerprint\": \"{fingerprint}\"");
                if existing_meta.contains(&needle) {
                    if let (Some(ix), Some(iy)) = (
                        parse_meta_f32(&existing_meta, "intrinsicWidth"),
                        parse_meta_f32(&existing_meta, "intrinsicHeight"),
                    ) {
                        return Ok(RenderOutcome {
                            intrinsic: Vec2::new(ix, iy),
                            cache_hit: true,
                        });
                    }
                }
            }
        }

        let dark = case.theme == "dark";
        let tokens = palette();

        let mut scene = Gallery::new();
        scene.camera = Camera::ui((width as f32, height as f32), 1.0);
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

        let intrinsic = {
            let mut text_ctx = TextCtx {
                engine: &mut self.labels.engine,
                cache: &mut self.labels.cache,
                atlas_clock: 0,
                raster_scale: 1.0,
            };
            emit_case(case, bounds, &mut scene, &mut text_ctx)?
        };

        self.renderer
            .resize(width, height)
            .map_err(|error| error.diagnostic_report())?;

        let updates: Vec<AtlasUpload> = self
            .labels
            .cache
            .take_updates()
            .into_iter()
            .map(|update| {
                let kind = match update.target {
                    unbound_text::AtlasTarget::Coverage => AtlasKind::R8,
                    unbound_text::AtlasTarget::Color => AtlasKind::Rgba8,
                };
                AtlasUpload {
                    kind,
                    layer: update.layer,
                    x: update.x,
                    y: update.y,
                    width: update.width,
                    height: update.height,
                    data: update.data,
                }
            })
            .collect();
        if !updates.is_empty() {
            self.atlases
                .upload_batch(&self.renderer.gpu, &updates)
                .map_err(|error| error.to_string())?;
        }

        let lut = backdrop_lut(dark, case.backdrop);
        self.renderer
            .write_theme_lut(&lut)
            .map_err(|error| error.to_string())?;
        self.generation = self.generation.wrapping_add(1);
        self.renderer
            .render(&scene.upload(self.generation, &[]))
            .map_err(|error| error.to_string())?;
        let pixels = self
            .renderer
            .read_pixels()
            .map_err(|error| error.to_string())?;
        write_bgra_png(png_path, width, height, &pixels)?;

        let meta = format!(
            "{{\n  \"control\": \"{}\",\n  \"state\": \"{}\",\n  \"theme\": \"{}\",\n  \"backdrop\": \"{}\",\n  \"fingerprint\": \"{}\",\n  \"imagePixelWidth\": {},\n  \"imagePixelHeight\": {},\n  \"controlBounds\": {{ \"x\": {}, \"y\": {}, \"width\": {}, \"height\": {} }},\n  \"intrinsicWidth\": {},\n  \"intrinsicHeight\": {}\n}}\n",
            case.control,
            case.state,
            case.theme,
            case.backdrop,
            fingerprint,
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

        Ok(RenderOutcome {
            intrinsic,
            cache_hit: false,
        })
    }

    /// 显式释放底层 Vulkan 渲染器资源。
    pub fn destroy(self) {
        self.renderer.destroy();
    }
}

/// 场景中唯一的控件节点。
const NODE: u64 = 1;

/// 渲染单个用例到 `png_path`（兼容单次调用入口，内部委托给 [`ParitySession`]）。
pub fn render_case(
    case: &ParityCase<'_>,
    width: u32,
    height: u32,
    bounds: Rect,
    png_path: &str,
) -> Result<Vec2, String> {
    let mut session = ParitySession::new(width, height)?;
    let outcome = session.render_case_cached(case, width, height, bounds, png_path, false)?;
    session.destroy();
    Ok(outcome.intrinsic)
}

/// 生成包含指定后景合成底板颜色的 `ThemeLut`（与 `Catalog.cs::CreateBackdropBrush` 严格对齐）。
#[must_use]
pub fn backdrop_lut(dark: bool, backdrop: &str) -> ThemeLut {
    let [r, g, b] = backdrop_surface_srgb(dark, backdrop);
    let base_palette = theme::palette(dark);
    let mut entries: Vec<(Token, Color)> = unbound_theme::TOKENS
        .iter()
        .map(|&token| (token, base_palette.color(token)))
        .collect();
    for (token, color) in &mut entries {
        if *token == Token::SURFACE {
            *color = Color::from_srgb8(r, g, b, 255);
        }
    }
    let kind = if dark {
        ThemeKind::Dark
    } else {
        ThemeKind::Light
    };
    ThemeLut::from_palette(&Palette::new(kind, &entries))
}

/// 确定性合成底板的 sRGB 背景基准色（与 `tools/winui-reference/Catalog.cs` 一一对应）。
#[must_use]
pub fn backdrop_surface_srgb(dark: bool, backdrop: &str) -> [u8; 3] {
    match (backdrop, dark) {
        ("mica", false) => [238, 242, 249],
        ("mica", true) => [29, 32, 39],
        ("mica_alt", false) => [229, 235, 245],
        ("mica_alt", true) => [22, 25, 32],
        ("acrylic", false) => [234, 240, 250],
        ("acrylic", true) => [36, 40, 48],
        (_, false) => [243, 243, 243],
        (_, true) => [32, 32, 32],
    }
}

/// 计算本框架侧的用例输入与源码哈希指纹（FNV-1a 64-bit）：
/// 绑定对应控件源码、基础图元源码、Win11 调色板源码与用例几何参数。
#[must_use]
pub fn compute_source_fingerprint(
    case: &ParityCase<'_>,
    width: u32,
    height: u32,
    bounds: Rect,
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    fn feed(hash: &mut u64, bytes: &[u8]) {
        for &b in bytes {
            *hash ^= u64::from(b);
            *hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }

    feed(&mut hash, b"v2-unbound-parity-fingerprint|");
    feed(&mut hash, case.control.as_bytes());
    feed(&mut hash, b"|");
    feed(&mut hash, case.state.as_bytes());
    feed(&mut hash, b"|");
    feed(&mut hash, case.theme.as_bytes());
    feed(&mut hash, b"|");
    feed(&mut hash, case.backdrop.as_bytes());
    feed(
        &mut hash,
        format!(
            "|{}x{}|{:.2},{:.2},{:.2},{:.2}|",
            width, height, bounds.origin.x, bounds.origin.y, bounds.size.x, bounds.size.y
        )
        .as_bytes(),
    );

    let widget_file = match case.control {
        "Button" => "crates/unbound-widgets/src/button.rs",
        "CheckBox" => "crates/unbound-widgets/src/checkbox.rs",
        "ToggleSwitch" => "crates/unbound-widgets/src/switch.rs",
        "Slider" => "crates/unbound-widgets/src/slider.rs",
        "ComboBox" => "crates/unbound-widgets/src/select.rs",
        "TextBox" => "crates/unbound-widgets/src/text_input.rs",
        _ => "",
    };

    for path in [
        widget_file,
        "crates/unbound-widgets/src/primitive.rs",
        "crates/unbound-theme/src/win11.rs",
        "apps/showcase/src/parity.rs",
    ] {
        if !path.is_empty() {
            if let Ok(bytes) = std::fs::read(path) {
                feed(&mut hash, &bytes);
            }
        }
    }

    format!("{hash:016x}")
}

fn parse_meta_f32(text: &str, key: &str) -> Option<f32> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = text[start..].trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
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
        focused: case.state == "focused",
        disabled: case.state == "disabled",
        selected: case.state == "checked",
    };
    let checked = case.state == "checked";

    let intrinsic = match case.control {
        "Button" => {
            // 参考截图按 WinUI 模板度量：4px 圆角、14px 字号、无外部投影。
            // 展台的便捷构造器 `label()` 为网页风格预览保留 6px 圆角，差分必须显式校正。
            let desc = ButtonDesc::label("Button")
                .variant(ButtonVariant::Secondary)
                .with_radius(unbound_widgets::button::BUTTON_RADIUS)
                .with_font_size(unbound_widgets::button::BUTTON_FONT_SIZE)
                .with_shadow(0.0);
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
            let desc = SwitchDesc::labeled("ToggleSwitch", checked)
                .with_layout(unbound_widgets::SwitchLayout::Header);
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
