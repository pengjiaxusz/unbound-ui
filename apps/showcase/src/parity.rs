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
use unbound_widgets::button::ButtonTokens;
use unbound_widgets::catalog::{emit_widget_with_shaper, CatalogParams};
use unbound_widgets::primitive;
use unbound_widgets::title_bar::{TitleBarChrome, WindowCaptionButton, WindowCaptionKind};
use unbound_widgets::{
    ButtonDesc, ButtonVariant, CheckboxDesc, ControlState, PointerPhase, SelectDesc, SliderDesc,
    SwitchDesc, TextCtx, TextInputDesc, UiWidget, WidgetCatalogId, WidgetOutput,
};

use crate::gallery::{palette, Gallery, Labels};
use crate::shot::write_bgra_png;
use crate::theme;

/// 已接入拼版采集与 ROI 差分的全部基准控件，与 `tools/winui-reference/Catalog.cs` 一一对应。
///
/// 前 6 个是首批试点，其后 12 个为「基础输入与文本类」批次，再后 28 个为
/// 「导航、浮层、集合与日期类」批次，最后 11 个为后续补齐的 WinUI 3 控件。
/// 控件名统一使用 WinUI 3 官方规范名。
pub const SUPPORTED_CONTROLS: [&str; 57] = [
    // 首批试点（6）
    "Button",
    "CheckBox",
    "ToggleSwitch",
    "Slider",
    "ComboBox",
    "TextBox",
    // 第一批：基础输入与文本类（12）
    "ToggleButton",
    "SplitButton",
    "RadioButton",
    "RatingControl",
    "ColorPicker",
    "NumberBox",
    "AutoSuggestBox",
    "RichTextBlock",
    "ProgressBar",
    "ProgressRing",
    "InfoBadge",
    "ToolTip",
    // 第二批：导航、浮层、集合与日期类（28）
    "InfoBar",
    "TeachingTip",
    "ContentDialog",
    "Flyout",
    "MenuFlyout",
    "MenuBar",
    "CommandBar",
    "NavigationView",
    "TabView",
    "Pivot",
    "BreadcrumbBar",
    "SelectorBar",
    "PipsPager",
    "Expander",
    "SplitView",
    "Border",
    "Card",
    "StackPanel",
    "ScrollView",
    "TreeView",
    "ListView",
    "FlipView",
    "CalendarView",
    "TimePicker",
    "PersonPicture",
    "Image",
    "FontIcon",
    "TitleBar",
    // 第三批：后续补齐的 WinUI 3 控件（11）
    "RichEditBox",
    "GridView",
    "ListBox",
    "CommandBarFlyout",
    "AppBarButton",
    "AppBarToggleButton",
    "AppBarSeparator",
    "SwipeControl",
    "RefreshContainer",
    "AnnotatedScrollBar",
    "ItemsView",
];

/// 把一个 WinUI 3 规范名解析为目录标识。
///
/// `Border` / `Card` 在目录中由 `Containers` 承担对应形态；`TitleBar` 不在业务
/// 目录内，由独立分支绘制。其余名称（含 `TeachingTip`）直接走目录规范名索引。
#[must_use]
pub fn catalog_id_for(control: &str) -> Option<unbound_widgets::WidgetCatalogId> {
    match control {
        "Border" | "Card" => Some(unbound_widgets::WidgetCatalogId::Containers),
        "TitleBar" => None,
        other => unbound_widgets::WidgetCatalogId::from_winui3_name(other),
    }
}

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

/// 差分专用原始色槽（避开 [`unbound_theme::TOKENS`] 已占用的语义槽位）。
///
/// 每项为 `(原始槽位, 浅色 sRGB, 深色 sRGB)`；颜色取自 WinUI 3 参考截图里
/// 那些没有语义令牌可表达的模板过渡色。
const PARITY_COLORS: [(u16, [u8; 3], [u8; 3]); 11] = [
    // 选中态 ToggleButton 的侧边与底边（WinUI 以强调色叠加半透明描边）。
    (310, [20, 108, 190], [108, 209, 255]),
    (311, [0, 57, 110], [83, 177, 220]),
    // SplitButton 的底边强描边（正常 / 禁用）。
    (312, [211, 211, 211], [60, 60, 60]),
    (313, [216, 216, 216], [57, 57, 57]),
    // RatingControl 未选中星形的实心色。
    (314, [117, 117, 117], [125, 125, 125]),
    // ToolTip 表面与边框（浮层材质，颜色随主题变化）。
    (315, [249, 249, 249], [46, 46, 46]),
    (316, [229, 229, 229], [26, 26, 26]),
    // RadioButton 禁用态圆环内侧的浅色圈。
    (317, [240, 240, 240], [42, 42, 42]),
    // WinUI 禁用正文的 40% 墨色（比框架的 64% 禁用色更淡）。
    (318, [156, 156, 156], [121, 121, 121]),
    // RatingControl 实心星的外描边（浅色更深、深色更亮）。
    (319, [15, 98, 175], [107, 203, 247]),
    (320, [82, 82, 82], [172, 172, 172]),
];

const PARITY_CHECKED_BORDER: Token = Token::new(310);
const PARITY_CHECKED_BOTTOM: Token = Token::new(311);
const PARITY_SPLIT_BOTTOM: Token = Token::new(312);
const PARITY_SPLIT_BOTTOM_DISABLED: Token = Token::new(313);
const PARITY_STAR_UNSELECTED: Token = Token::new(314);
const PARITY_TOOLTIP_FILL: Token = Token::new(315);
const PARITY_TOOLTIP_BORDER: Token = Token::new(316);
const PARITY_RADIO_DISABLED_RIM: Token = Token::new(317);
const PARITY_TEXT_DISABLED: Token = Token::new(318);
const PARITY_STAR_EDGE: Token = Token::new(319);
const PARITY_STAR_EDGE_DISABLED: Token = Token::new(320);

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
    // 差分专用色槽：WinUI 模板里少量过渡色（选中态上下边缘、ToolTip 表面）
    // 没有对应的语义令牌。这里用未占用的原始槽位承载，阈值与主题保持一致。
    for (token, light, dark_color) in PARITY_COLORS {
        entries.push((
            Token::new(token),
            Color::from_srgb8(
                if dark { dark_color[0] } else { light[0] },
                if dark { dark_color[1] } else { light[1] },
                if dark { dark_color[2] } else { light[2] },
                255,
            ),
        ));
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

    // 缓存版本号：字形缓存修复与三态有效性门禁落地后必须让旧缓存整体失效。
    feed(&mut hash, b"v3-unbound-parity-fingerprint|");
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

    let widget_file = if case.control == "TitleBar" {
        "crates/unbound-widgets/src/title_bar.rs".to_owned()
    } else {
        catalog_id_for(case.control)
            .map(|id| format!("crates/unbound-widgets/src/{}.rs", id.module_name()))
            .unwrap_or_default()
    };

    for path in [
        widget_file.as_str(),
        "crates/unbound-widgets/src/primitive.rs",
        "crates/unbound-theme/src/win11.rs",
        // 字形位图由 DirectWrite/塑形层决定，改动后必须让差分缓存失效。
        "crates/unbound-text/src/directwrite.rs",
        "crates/unbound-text/src/glyph.rs",
        "crates/unbound-text/src/shaping.rs",
        "crates/unbound-text/src/atlas.rs",
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
        "ToggleButton" => {
            // WinUI 的 ToggleButton 与 Button 共用模板，仅选中态换成强调色。
            let mut desc = ButtonDesc::label("")
                .variant(ButtonVariant::Secondary)
                .with_radius(unbound_widgets::button::BUTTON_RADIUS)
                .with_font_size(unbound_widgets::button::BUTTON_FONT_SIZE)
                .with_shadow(0.0);
            if checked {
                desc = desc.with_tokens(ButtonTokens {
                    fill: Token::PRIMARY,
                    fill_hover: Token::PRIMARY_HOVER,
                    fill_pressed: Token::PRIMARY_PRESSED,
                    fill_disabled: Token::PRIMARY_SOFT,
                    border: PARITY_CHECKED_BORDER,
                    label: Token::ON_PRIMARY,
                    label_disabled: Token::ON_DISABLED,
                    focus_ring: Token::PRIMARY,
                });
            }
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &(), text_ctx),
            );
            emit_text_centered(
                text_ctx,
                scene,
                "ToggleButton",
                rect,
                14.0,
                500,
                None,
                if checked {
                    Token::ON_PRIMARY
                } else if control.disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::TEXT_PRIMARY
                },
                901,
            );
            if checked {
                // 模板底边叠了一层更暗的强调色描边。
                let mut out = WidgetOutput::default();
                out.primitives.push(primitive::surface(
                    Rect::new(
                        rect.origin.x + 3.0,
                        rect.max().y - 1.0,
                        rect.width() - 6.0,
                        1.0,
                    ),
                    0.0,
                    PARITY_CHECKED_BOTTOM,
                    900,
                ));
                push(scene, out);
            }
            rect.size
        }
        "SplitButton" => {
            let disabled = control.disabled;
            let fill = if disabled {
                Token::CONTROL_DISABLED_BG
            } else {
                Token::CONTROL_FILL
            };
            let mut out = WidgetOutput::default();
            out.primitives.push(primitive::outlined(
                rect,
                4.0,
                fill,
                Token::CONTROL_STROKE,
                1.0,
                0.0,
                700,
            ));
            // 分隔列：主操作区 84px，其余为下拉箭头区。
            out.primitives.push(primitive::surface(
                Rect::new(rect.origin.x + 84.0, rect.origin.y + 1.0, 1.0, rect.height() - 2.0),
                0.0,
                Token::CONTROL_STROKE,
                701,
            ));
            out.primitives.push(primitive::surface(
                Rect::new(
                    rect.origin.x + 3.0,
                    rect.max().y - 1.0,
                    rect.width() - 6.0,
                    1.0,
                ),
                0.0,
                if disabled {
                    PARITY_SPLIT_BOTTOM_DISABLED
                } else {
                    PARITY_SPLIT_BOTTOM
                },
                702,
            ));
            let ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            emit_text_left_middle(
                text_ctx,
                scene,
                "SplitButton",
                rect,
                rect.origin.x + 11.0,
                unbound_widgets::button::BUTTON_FONT_SIZE,
                600,
                None,
                ink,
                703,
            );
            let cx = rect.max().x - 18.5;
            let cy = rect.center().y;
            let arrow_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };
            primitive::curve_stroke(
                [cx - 4.0, cy - 2.5],
                [cx - 0.5, cy + 1.5],
                1.0,
                arrow_ink,
                704,
                &mut out.primitives,
                &mut out.payloads,
            );
            primitive::curve_stroke(
                [cx - 0.5, cy + 1.5],
                [cx + 3.5, cy - 2.5],
                1.0,
                arrow_ink,
                704,
                &mut out.primitives,
                &mut out.payloads,
            );
            push(scene, out);
            rect.size
        }
        "RadioButton" => {
            let center = Vec2::new(rect.origin.x + 10.0, rect.center().y);
            let circle = Rect::new(center.x - 10.0, center.y - 10.0, 20.0, 20.0);
            let mut out = WidgetOutput::default();
            if checked {
                out.primitives
                    .push(primitive::surface(circle, 10.0, Token::PRIMARY, 700));
                out.primitives.push(primitive::surface(
                    Rect::new(center.x - 5.0, center.y - 5.0, 10.0, 10.0),
                    5.0,
                    Token::ON_PRIMARY,
                    701,
                ));
            } else if control.disabled {
                out.primitives.push(primitive::outlined(
                    circle,
                    10.0,
                    Token::NONE,
                    Token::CONTROL_STRONG_STROKE_DISABLED,
                    1.0,
                    0.0,
                    700,
                ));
                out.primitives.push(primitive::outlined(
                    Rect::new(center.x - 8.0, center.y - 8.0, 16.0, 16.0),
                    8.0,
                    Token::NONE,
                    PARITY_RADIO_DISABLED_RIM,
                    1.0,
                    0.0,
                    701,
                ));
            } else {
                out.primitives.push(primitive::outlined(
                    circle,
                    10.0,
                    Token::CONTROL_ALT_FILL_SECONDARY,
                    Token::CONTROL_TRACK,
                    1.0,
                    0.0,
                    700,
                ));
            }
            push(scene, out);
            emit_text_left_middle(
                text_ctx,
                scene,
                "RadioButton",
                rect,
                rect.origin.x + 28.0,
                14.0,
                400,
                Some("Segoe UI Variable Text"),
                if control.disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::TEXT_PRIMARY
                },
                702,
            );
            rect.size
        }
        "RatingControl" => {
            for index in 0..5u32 {
                let slot = Rect::new(
                    rect.origin.x + 24.0 * index as f32,
                    rect.origin.y,
                    16.0,
                    rect.height(),
                );
                let filled = index < 3;
                if filled {
                    emit_icon_centered(
                        text_ctx,
                        scene,
                        "\u{E735}",
                        slot,
                        16.0,
                        if control.disabled {
                            PARITY_STAR_EDGE_DISABLED
                        } else {
                            PARITY_STAR_EDGE
                        },
                        700 + index as u16,
                    );
                    emit_icon_centered(
                        text_ctx,
                        scene,
                        "\u{E735}",
                        slot,
                        14.0,
                        if control.disabled {
                            PARITY_TEXT_DISABLED
                        } else {
                            Token::PRIMARY
                        },
                        800 + index as u16,
                    );
                } else {
                    // 空心星：先铺实心星，再用背景色的小一号实心星挖出星形内孔，
                    // 得到与 WinUI 空心星一致的 1px 轮廓。
                    emit_icon_centered(
                        text_ctx,
                        scene,
                        "\u{E735}",
                        slot,
                        16.0,
                        PARITY_STAR_UNSELECTED,
                        700 + index as u16,
                    );
                    emit_icon_centered(
                        text_ctx,
                        scene,
                        "\u{E735}",
                        slot,
                        11.0,
                        Token::SURFACE,
                        800 + index as u16,
                    );
                }
            }
            rect.size
        }
        "NumberBox" => {
            let desc = TextInputDesc::text("50");
            let state = desc.initial_state();
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            let cx = rect.max().x - 14.5;
            let cy = rect.center().y;
            let ink = if control.disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };
            let mut out = WidgetOutput::default();
            primitive::curve_stroke(
                [cx - 4.0, cy - 2.0],
                [cx, cy - 6.0],
                1.5,
                ink,
                900,
                &mut out.primitives,
                &mut out.payloads,
            );
            primitive::curve_stroke(
                [cx, cy - 6.0],
                [cx + 4.0, cy - 2.0],
                1.5,
                ink,
                900,
                &mut out.primitives,
                &mut out.payloads,
            );
            primitive::curve_stroke(
                [cx - 4.0, cy + 1.0],
                [cx, cy + 5.0],
                1.5,
                ink,
                900,
                &mut out.primitives,
                &mut out.payloads,
            );
            primitive::curve_stroke(
                [cx, cy + 5.0],
                [cx + 4.0, cy + 1.0],
                1.5,
                ink,
                900,
                &mut out.primitives,
                &mut out.payloads,
            );
            push(scene, out);
            rect.size
        }
        "AutoSuggestBox" => {
            let desc = TextInputDesc::text("Text");
            let state = desc.initial_state();
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            let center = Vec2::new(rect.max().x - 19.5, rect.center().y);
            emit_icon_centered(
                text_ctx,
                scene,
                "\u{E721}",
                Rect::new(center.x - 8.0, center.y - 8.0, 16.0, 16.0),
                14.0,
                if control.disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::TEXT_SECONDARY
                },
                900,
            );
            rect.size
        }
        "RichTextBlock" => {
            emit_text_left_middle(
                text_ctx,
                scene,
                "RichTextBlock",
                rect,
                rect.origin.x,
                14.0,
                400,
                Some("Segoe UI Variable Text"),
                if control.disabled {
                    PARITY_TEXT_DISABLED
                } else {
                    Token::TEXT_PRIMARY
                },
                700,
            );
            rect.size
        }
        "ProgressBar" => {
            let mut out = WidgetOutput::default();
            out.primitives.push(primitive::surface(
                Rect::new(rect.origin.x, rect.origin.y + 1.0, rect.width(), 1.0),
                0.5,
                Token::CONTROL_TRACK,
                700,
            ));
            out.primitives.push(primitive::surface(
                Rect::new(rect.origin.x, rect.origin.y, rect.width() * 0.4, 3.0),
                1.5,
                Token::PRIMARY,
                701,
            ));
            push(scene, out);
            rect.size
        }
        "ProgressRing" => {
            // WinUI 3 的不确定圆环由合成动画驱动；参考采集把它固定在动画进度
            // 0.4 上（见 CaptureWindow.FreezeAnimations），得到一条确定性的
            // 32px 圆环粗弧。这里按行复刻该静止帧，避免任何动画相位漂移。
            const ARC_ROWS: [(f32, f32, f32); 28] = [
                (1.0, 15.0, 6.0),
                (2.0, 15.0, 8.0),
                (3.0, 16.0, 9.0),
                (4.0, 21.0, 5.0),
                (5.0, 23.0, 4.0),
                (6.0, 24.0, 4.0),
                (7.0, 25.0, 4.0),
                (8.0, 26.0, 3.0),
                (9.0, 27.0, 3.0),
                (10.0, 27.0, 3.0),
                (11.0, 27.0, 3.0),
                (12.0, 27.0, 3.0),
                (13.0, 27.0, 3.0),
                (14.0, 27.0, 3.0),
                (15.0, 27.0, 3.0),
                (16.0, 27.0, 3.0),
                (17.0, 27.0, 3.0),
                (18.0, 27.0, 3.0),
                (19.0, 27.0, 3.0),
                (20.0, 27.0, 3.0),
                (21.0, 27.0, 3.0),
                (22.0, 27.0, 3.0),
                (23.0, 26.0, 4.0),
                (24.0, 25.0, 4.0),
                (25.0, 24.0, 4.0),
                (26.0, 23.0, 4.0),
                (27.0, 23.0, 3.0),
                (28.0, 24.0, 1.0),
            ];
            let mut out = WidgetOutput::default();
            for (row, (dy, dx, width)) in ARC_ROWS.iter().enumerate() {
                out.primitives.push(primitive::surface(
                    Rect::new(
                        rect.origin.x + dx,
                        rect.origin.y + dy,
                        *width,
                        1.0,
                    ),
                    0.0,
                    Token::PRIMARY,
                    700 + row as u16,
                ));
            }
            push(scene, out);
            rect.size
        }
        "InfoBadge" => {
            let mut out = WidgetOutput::default();
            out.primitives
                .push(primitive::surface(rect, 7.5, Token::PRIMARY, 700));
            push(scene, out);
            emit_text_centered(
                text_ctx,
                scene,
                "3",
                rect,
                11.0,
                400,
                Some("Segoe UI Variable Text"),
                Token::ON_PRIMARY,
                701,
            );
            rect.size
        }
        "ToolTip" => {
            let mut out = WidgetOutput::default();
            out.primitives.push(primitive::outlined(
                rect,
                4.0,
                PARITY_TOOLTIP_FILL,
                PARITY_TOOLTIP_BORDER,
                1.0,
                0.0,
                700,
            ));
            push(scene, out);
            // ToolTip 内容在浮层左上角，12px 正文。
            let quads = text_ctx.shape_line_styled(
                "ToolTip",
                Rect::new(rect.origin.x + 9.0, rect.origin.y + 6.0, 200.0, 16.0),
                12.0,
                400,
                Some("Segoe UI Variable Text"),
                unbound_widgets::AlignX::Left,
                unbound_widgets::AlignY::Top,
            );
            emit_quads(scene, &quads, Token::TEXT_PRIMARY, 701);
            rect.size
        }
        "TitleBar" => {
            // 标题栏由 chrome 与三枚窗口控制按钮组成；三者都实现 `UiWidget`，
            // 因此固有高度直接取自真实描述符（32px）。
            let chrome = TitleBarChrome::new(!control.disabled);
            let chrome_size = chrome.intrinsic_size(text_ctx);
            push(
                scene,
                chrome.emit_laid_out(NODE, rect, control, &(), text_ctx),
            );
            let caption_w = unbound_widgets::title_bar::TITLE_BAR_CAPTION_WIDTH;
            let kinds = [
                WindowCaptionKind::Minimize,
                WindowCaptionKind::Maximize { maximized: false },
                WindowCaptionKind::Close,
            ];
            let count = kinds.len();
            for (index, kind) in kinds.into_iter().enumerate() {
                let left = rect.max().x - caption_w * (count - index) as f32;
                let button_rect = Rect::new(left, rect.origin.y, caption_w, rect.height());
                let button =
                    WindowCaptionButton::new(kind, rect.height()).focused(!control.disabled);
                push(
                    scene,
                    button.emit_laid_out(NODE, button_rect, control, &(), text_ctx),
                );
            }
            Vec2::new(rect.width(), chrome_size.y.max(rect.height()))
        }
        other => {
            let id: WidgetCatalogId =
                catalog_id_for(other).ok_or_else(|| format!("unknown parity control: {other}"))?;
            let params = CatalogParams {
                value_norm: 0.5,
                variant_idx: usize::from(checked),
                show_icon: true,
                bordered: true,
                active: true,
                compact: false,
                read_only: false,
                toggled: checked,
                click_count: 0,
            };
            let output =
                emit_widget_with_shaper(id, NODE, control, rect, params, |text, origin, size| {
                    shape_at(text_ctx, text, origin, size)
                });
            push(scene, output);
            // 没有实现 `UiWidget` 的组件暂时以参考边界作为固有尺寸；一旦其描述符
            // 补齐 `intrinsic_size`，这里会自动被替换为真实测量值。
            rect.size
        }
    };

    Ok(intrinsic)
}

/// 按绝对起点与字号排版一行文本，供目录通用发射器复用同一套字形缓存。
fn shape_at(
    text_ctx: &mut TextCtx<'_>,
    text: &str,
    origin: Vec2,
    size: f32,
) -> Vec<unbound_text::GlyphQuad> {
    shape_at_styled(text_ctx, text, origin, size, 400, None)
}

/// 与 [`shape_at`] 相同，但允许指定字重与字体族（用于 Segoe Fluent Icons 字形）。
fn shape_at_styled(
    text_ctx: &mut TextCtx<'_>,
    text: &str,
    origin: Vec2,
    size: f32,
    weight: u16,
    family: Option<&str>,
) -> Vec<unbound_text::GlyphQuad> {
    if text.is_empty() {
        return Vec::new();
    }
    let scale = text_ctx.effective_raster_scale();
    let mut style = unbound_text::TextStyle::sized(size * scale);
    style.weight = weight;
    style.family = family.map(str::to_owned);
    // 与控件发射器一致：优先用 DirectWrite 校正后的簇原点，避免
    // cosmic-text 的累计前进宽度在长标签上逐字漂移。
    let shaped = match text_ctx.engine.shape_laid_out(text, &style) {
        Some(shaped) => shaped,
        None => text_ctx.engine.shape(text, &style),
    };
    let mut quads = text_ctx.cache.quads_for(
        text_ctx.engine,
        &shaped,
        Vec2::new(origin.x * scale, origin.y * scale),
        400,
        false,
    );
    if (scale - 1.0).abs() > 1e-6 {
        for quad in &mut quads {
            quad.rect.origin.x /= scale;
            quad.rect.origin.y /= scale;
            quad.rect.size.x /= scale;
            quad.rect.size.y /= scale;
        }
    }
    quads
}

/// 把一组字形作为独立场景节点追加，沿用统一的场景发射路径。
fn emit_quads(scene: &mut Gallery, quads: &[unbound_text::GlyphQuad], color: Token, z: u16) {
    let mut out = WidgetOutput::default();
    primitive::glyphs(quads, color, z, &mut out.primitives, &mut out.payloads);
    push(scene, out);
}

/// 在矩形内左对齐、垂直居中排版一行文本。
#[allow(clippy::too_many_arguments)]
fn emit_text_left_middle(
    text_ctx: &mut TextCtx<'_>,
    scene: &mut Gallery,
    text: &str,
    rect: Rect,
    left: f32,
    size: f32,
    weight: u16,
    family: Option<&str>,
    color: Token,
    z: u16,
) {
    let label = Rect::new(left, rect.origin.y, (rect.max().x - left).max(1.0), rect.height());
    let quads = text_ctx.shape_line_styled(
        text,
        label,
        size,
        weight,
        family,
        unbound_widgets::AlignX::Left,
        unbound_widgets::AlignY::Middle,
    );
    emit_quads(scene, &quads, color, z);
}

/// 把一枚 Segoe Fluent Icons 字形居中绘制到给定方框。
#[allow(clippy::too_many_arguments)]
fn emit_icon_centered(
    text_ctx: &mut TextCtx<'_>,
    scene: &mut Gallery,
    glyph: &str,
    rect: Rect,
    size: f32,
    color: Token,
    z: u16,
) {
    let scale = text_ctx.effective_raster_scale();
    let mut style = unbound_text::TextStyle::sized(size * scale);
    style.family = Some("Segoe Fluent Icons".to_owned());
    let measured = text_ctx.engine.measure(glyph, &style);
    let width = measured.x / scale;
    let height = measured.y / scale;
    let origin = Vec2::new(
        rect.center().x - width * 0.5,
        rect.center().y - height * 0.5,
    );
    let quads = shape_at_styled(
        text_ctx,
        glyph,
        origin,
        size,
        400,
        Some("Segoe Fluent Icons"),
    );
    emit_quads(scene, &quads, color, z);
}

/// 在控件矩形内居中排版一行文本。
#[allow(clippy::too_many_arguments)]
fn emit_text_centered(
    text_ctx: &mut TextCtx<'_>,
    scene: &mut Gallery,
    text: &str,
    rect: Rect,
    size: f32,
    weight: u16,
    family: Option<&str>,
    color: Token,
    z: u16,
) {
    let quads = text_ctx.shape_line_styled(
        text,
        rect,
        size,
        weight,
        family,
        unbound_widgets::AlignX::Center,
        unbound_widgets::AlignY::Middle,
    );
    emit_quads(scene, &quads, color, z);
}

/// 把一次发射的图元与载荷追加到场景。
fn push(scene: &mut Gallery, output: unbound_widgets::WidgetOutput) {
    scene.primitives.extend(output.primitives);
    scene.payloads.extend(output.payloads);
}
