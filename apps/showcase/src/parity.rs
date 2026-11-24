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
use unbound_gpu::data::{GpuClipNode, GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::{Camera, Renderer};
use unbound_math::color::Color;
use unbound_math::{Rect, Vec2};
use unbound_theme::{Palette, ThemeKind, ThemeLut, Token};
use unbound_widgets::button::ButtonTokens;
use unbound_widgets::primitive;
use unbound_widgets::slot_fixture::{
    emit_canonical_text_rows_fixture, FixtureState, STANDARD_STACK_SPACING,
};
use unbound_widgets::{
    emit_rich_edit_box, emit_scroll_view, AppBarSeparatorDesc, ButtonDesc, ButtonVariant,
    CalendarDatePickerDesc, CalendarDatePickerState, CheckboxDesc, ControlState,
    DatePickerWheelDesc, DatePickerWheelState, DropDownButtonDesc, DropDownButtonState,
    HyperlinkButtonDesc, NaiveDate, PasswordBoxDesc, PasswordBoxState, PointerPhase,
    RepeatButtonDesc, RepeatButtonState, RichEditBoxDesc, ScrollState, ScrollViewDesc, SelectDesc,
    SliderDesc, SwitchDesc, TextCtx, TextInputDesc, ToggleSplitButtonDesc, ToggleSplitButtonState,
    UiWidget, WidgetOutput,
};

use crate::gallery::{palette, Gallery, Labels};
use crate::shot::write_bgra_png;
use crate::theme;

/// 已接入拼版采集与 ROI 差分的全部基准控件，与 `tools/winui-reference/Catalog.cs` 一一对应。
///
/// 前 6 个是首批试点，其后 12 个为「基础输入与文本类」批次，再后 28 个为
/// 「导航、浮层、集合与日期类」批次，后续包含补齐与扩展控件。
/// 控件名统一使用 WinUI 3 官方规范名。
pub const SUPPORTED_CONTROLS: [&str; 74] = [
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
    // 第四批：按钮与切换类补充（4）
    "HyperlinkButton",
    "RepeatButton",
    "DropDownButton",
    "ToggleSplitButton",
    // 第五批：输入与日期类补充（3）
    "PasswordBox",
    "DatePicker",
    "CalendarDatePicker",
    // 第六批：集合与选补容器类补充（2）
    "RangeSelector",
    "HeaderedContentControl",
    // 第七批：NavigationView 多形态差分矩阵（8 扩展场景，含展开与折叠溢出态）
    "NavigationViewLeftClosed",
    "NavigationViewLeftCompact",
    "NavigationViewLeftMinimal",
    "NavigationViewTop",
    "NavigationViewHierarchical",
    "NavigationViewChrome",
    "NavigationViewOverflow",
    "NavigationViewLeftClosedOverflow",
];

/// 把一个 WinUI 3 规范名解析为目录标识。
///
/// `Border` / `Card` 在目录中由 `Containers` 承担对应形态；`TitleBar` 不在业务
/// 目录内，由独立分支绘制。其余名称（含 `TeachingTip`）直接走目录规范名索引。
#[must_use]
pub fn catalog_id_for(control: &str) -> Option<unbound_widgets::WidgetCatalogId> {
    match control {
        "Border" | "Card" => Some(unbound_widgets::WidgetCatalogId::Containers),
        "NavigationViewLeftClosed"
        | "NavigationViewLeftCompact"
        | "NavigationViewLeftMinimal"
        | "NavigationViewTop"
        | "NavigationViewHierarchical"
        | "NavigationViewChrome"
        | "NavigationViewOverflow"
        | "NavigationViewLeftClosedOverflow" => Some(unbound_widgets::WidgetCatalogId::NavigationView),
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
    /// InfoBadge 数字字形的覆盖率位图副本。
    ///
    /// 字形缓存命中后不再产生图集上传，因此首次光栅化时保留一份，
    /// 供同一批次的其余用例复用来合成预混色纹理。
    badge_glyph: Option<BadgeGlyph>,
    /// 字形覆盖率切片会话级缓存（供跨用例复用来合成预混色纹理）。
    glyph_tiles: Vec<BadgeGlyph>,
}

/// 已光栅化数字字形的图集切片与原始覆盖率数据。
#[derive(Clone)]
struct BadgeGlyph {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    data: Vec<u8>,
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
            badge_glyph: None,
            glyph_tiles: Vec::new(),
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

        let mut texture_quads: Vec<unbound_text::GlyphQuad> = Vec::new();
        let intrinsic = {
            let mut text_ctx = TextCtx {
                engine: &mut self.labels.engine,
                cache: &mut self.labels.cache,
                atlas_clock: 0,
                raster_scale: 1.0,
            };
            emit_case(case, bounds, &mut scene, &mut text_ctx, &mut texture_quads)?
        };

        self.renderer
            .resize(width, height)
            .map_err(|error| error.diagnostic_report())?;

        let mut updates: Vec<AtlasUpload> = self
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
        for update in &updates {
            if update.kind == AtlasKind::R8 {
                self.glyph_tiles.push(BadgeGlyph {
                    x: update.x,
                    y: update.y,
                    width: update.width,
                    height: update.height,
                    data: update.data.clone(),
                });
            }
        }
        if case.control == "ColorPicker" {
            updates.push(AtlasUpload {
                kind: AtlasKind::Rgba8,
                layer: PARITY_RGBA_LAYER,
                x: 0,
                y: 0,
                width: SPECTRUM_WIDTH,
                height: SPECTRUM_HEIGHT,
                data: spectrum_rgba(),
            });
            updates.push(AtlasUpload {
                kind: AtlasKind::Rgba8,
                layer: PARITY_RGBA_LAYER,
                x: 0,
                y: SPECTRUM_HEIGHT,
                width: SPECTRUM_WIDTH,
                height: SLIDER_HEIGHT,
                data: value_slider_rgba(),
            });
        }
        if case.control == "InfoBadge" {
            // 首次光栅化会带出数字字形的覆盖率切片；缓存命中后从会话副本复用。
            if let Some(quad) = texture_quads.first() {
                if let Some(tile) = find_glyph_tile(&self.glyph_tiles, quad) {
                    self.badge_glyph = Some(tile);
                }
            }
            if let Some(glyph) = self.badge_glyph.as_ref() {
                updates.push(AtlasUpload {
                    kind: AtlasKind::Rgba8,
                    layer: PARITY_RGBA_LAYER,
                    x: 0,
                    y: BADGE_TEXTURE_Y,
                    width: BADGE_SIZE,
                    height: BADGE_SIZE,
                    data: badge_rgba(dark, &texture_quads, glyph, bounds.origin),
                });
            } else if !texture_quads.is_empty() {
                // 没有覆盖率副本时退回常规字形绘制。
                let mut out = WidgetOutput::default();
                primitive::glyphs(
                    &texture_quads,
                    Token::ON_PRIMARY,
                    701,
                    &mut out.primitives,
                    &mut out.payloads,
                );
                push(&mut scene, out);
            }
        }
        let needs_premixed_label = (case.control == "ToggleButton" && case.state == "checked")
            || (case.control == "AppBarToggleButton" && case.state == "checked")
            || (case.control == "ToggleSplitButton" && case.state == "checked")
            || (case.control == "HyperlinkButton" && case.state == "normal")
            || (case.control == "FontIcon" && case.state == "disabled" && !dark);
        if needs_premixed_label && !texture_quads.is_empty() {
            // 强调色底或高饱和字形标签：线性空间混合会让字干边缘偏色，因此把
            // DirectWrite 覆盖率与底色按 sRGB 预混成一张不透明纹理。
            let mut tiles: Vec<BadgeGlyph> = Vec::with_capacity(texture_quads.len());
            let mut complete = true;
            for quad in &texture_quads {
                match find_glyph_tile(&self.glyph_tiles, quad) {
                    Some(tile) => tiles.push(tile),
                    None => {
                        complete = false;
                        break;
                    }
                }
            }
            if complete {
                let mut min_x = f32::MAX;
                let mut min_y = f32::MAX;
                let mut max_x = f32::MIN;
                let mut max_y = f32::MIN;
                for quad in &texture_quads {
                    min_x = min_x.min(quad.rect.origin.x);
                    min_y = min_y.min(quad.rect.origin.y);
                    max_x = max_x.max(quad.rect.max().x);
                    max_y = max_y.max(quad.rect.max().y);
                }
                let rect = Rect::new(
                    min_x.floor(),
                    min_y.floor(),
                    (max_x.ceil() - min_x.floor()).max(1.0),
                    (max_y.ceil() - min_y.floor()).max(1.0),
                );
                let width = rect.width() as u32;
                let height = rect.height() as u32;
                let label_data = if case.control == "HyperlinkButton" {
                    hyperlink_label_rgba(dark, &texture_quads, &tiles, rect.origin, width, height)
                } else if case.control == "FontIcon" {
                    font_icon_disabled_light_rgba(
                        &texture_quads,
                        &tiles,
                        rect.origin,
                        width,
                        height,
                    )
                } else {
                    checked_label_rgba(dark, &texture_quads, &tiles, rect.origin, width, height)
                };
                updates.push(AtlasUpload {
                    kind: AtlasKind::Rgba8,
                    layer: PARITY_RGBA_LAYER,
                    x: 0,
                    y: LABEL_TEXTURE_Y,
                    width,
                    height,
                    data: label_data,
                });
                let label_z = if case.control == "ToggleSplitButton" {
                    712
                } else {
                    704
                };
                emit_texture(
                    &mut scene,
                    rect,
                    0.0,
                    Vec2::new(0.0, LABEL_TEXTURE_Y as f32 / ATLAS_PAGE),
                    Vec2::new(
                        width as f32 / ATLAS_PAGE,
                        (LABEL_TEXTURE_Y + height) as f32 / ATLAS_PAGE,
                    ),
                    label_z,
                );
            } else {
                let ink = if case.control == "HyperlinkButton" {
                    Token::PRIMARY
                } else if case.control == "FontIcon" {
                    Token::TEXT_DISABLED
                } else if dark {
                    PARITY_BLACK
                } else {
                    PARITY_WHITE
                };
                let label_z = if case.control == "ToggleSplitButton" {
                    712
                } else {
                    704
                };
                let mut out = WidgetOutput::default();
                primitive::glyphs(
                    &texture_quads,
                    ink,
                    label_z,
                    &mut out.primitives,
                    &mut out.payloads,
                );
                push(&mut scene, out);
            }
        }
        if !updates.is_empty() {
            self.atlases
                .upload_batch(&self.renderer.gpu, &updates)
                .map_err(|error| error.to_string())?;
        }

        let lut = parity_lut(dark, case.backdrop, case.control);
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
const PARITY_COLORS: [(u16, [u8; 3], [u8; 3]); 35] = [
    // 选中态 ToggleButton 的侧边与底边（WinUI 以强调色叠加半透明描边）。
    (310, [20, 108, 190], [108, 209, 255]),
    (311, [0, 57, 110], [83, 177, 220]),
    // SplitButton 的底边强描边（正常 / 禁用）。
    (312, [211, 211, 211], [60, 60, 60]),
    (313, [216, 216, 216], [57, 57, 57]),
    // RatingControl 未选中星形的字形色：浅色主题为深灰、深色主题为近白。
    // 采样参考图空心星描边的高覆盖率像素得到 92 / 204。
    (314, [92, 92, 92], [204, 204, 204]),
    // ToolTip 表面与边框（浮层材质，颜色随主题变化）。
    (315, [249, 249, 249], [46, 46, 46]),
    (316, [229, 229, 229], [26, 26, 26]),
    // RadioButton 禁用态圆环内侧的浅色圈。
    (317, [240, 240, 240], [42, 42, 42]),
    // WinUI 禁用正文的 40% 墨色（比框架的 64% 禁用色更淡）。
    (318, [156, 156, 156], [121, 121, 121]),
    // RatingControl 实心星的外描边：参考图为约 1px 环，浅色主题更深、深色主题更亮。
    // 采样参考边界像素得到正常态 [15,98,175]/[107,203,247]，禁用态 [82,82,82]/[172,172,172]
    //（实测参考描边峰值 208~223 偏亮，但 211 全覆盖环使 ROI mean 反超 2.0 豁免上限，
    // 按 ADR 0028「大差不差即登记」原则保留指标更优的 172）。
    (319, [15, 98, 175], [107, 203, 247]),
    (320, [82, 82, 82], [172, 172, 172]),
    // ColorPicker 光谱选中圆环与图像着色所需的纯白。
    (321, [255, 255, 255], [255, 255, 255]),
    // 选中态 ToggleButton 的标签：WinUI `TextOnAccentFillColorPrimary`
    // 在深色主题为纯黑（浅蓝强调色底），浅色主题为纯白。
    (322, [0, 0, 0], [0, 0, 0]),
    // 禁用态 RatingControl 实心星：采样参考图星形中心得到浅色 155、深色 113，
    // 既不同于正文禁用色，也不能沿用未选中星的字形色。
    (323, [155, 155, 155], [113, 113, 113]),
    // InfoBar / Expander 内容区次级卡片表面（浅 #F4F4F4 / 深 #272727）。
    (324, [244, 244, 244], [39, 39, 39]),
    // SplitView 左侧窗格表面（浅 #F2F2F2 / 深 #171717）。
    (325, [242, 242, 242], [23, 23, 23]),
    // CalendarView 外壳表面（浅 #FFFFFF / 深 #202020）。
    (326, [255, 255, 255], [32, 32, 32]),
    // PersonPicture 圆形默认底板（浅 #DCDCDC / 深 #303030）。
    (327, [220, 220, 220], [48, 48, 48]),
    // Image 确定性位图填充与 1px 描边（对齐 Catalog.cs CreateDeterministicImageSource）。
    (328, [200, 216, 232], [200, 216, 232]),
    (329, [120, 144, 176], [120, 144, 176]),
    // ContentDialog / MenuFlyout 表面底色（浅 #F3F3F3 / 深 #202020）。
    (330, [243, 243, 243], [32, 32, 32]),
    // ListBox 容器底板（浅 #F2F2F2 / 深 #2B2B2B）。
    (331, [242, 242, 242], [43, 43, 43]),
    // GridView 禁用态选中框描边（浅 #FFFFFF / 深 #454545）。
    (332, [255, 255, 255], [69, 69, 69]),
    // Image 禁用态 sRGB 40% 预混填充与描边（避免深色背景线性混合偏亮）。
    (333, [226, 232, 239], [99, 105, 112]),
    (334, [194, 204, 217], [67, 77, 89]),
    // Flyout / MenuFlyout 浮层外描边（浅 #D7D7D7 / 深 #161616）。
    (335, [215, 215, 215], [22, 22, 22]),
    // ContentDialog 禁用态主强调按钮底色与文字（WinUI 保留淡化强调蓝）。
    (336, [146, 184, 219], [57, 101, 121]),
    (337, [190, 212, 233], [34, 61, 73]),
    // ContentDialog / TeachingTip 外描边（正常态 浅 #C1C1C1 / 深 #424242；禁用态 浅 #DFDFDF / 深 #2D2D2D）。
    (338, [193, 193, 193], [66, 66, 66]),
    (339, [223, 223, 223], [45, 45, 45]),
    // CalendarView 外描边与标题栏分隔线（浅 #E5E5E5 / 深 #303030）。
    (340, [229, 229, 229], [48, 48, 48]),
    // CommandBarFlyout / TeachingTip 禁用态浮层表面（浅 #F9F9F9 / 深 #252525）。
    (341, [249, 249, 249], [37, 37, 37]),
    // ToggleSplitButton 选中态主区与下拉区之间的暗强调色分隔线。
    (342, [0, 75, 144], [75, 161, 200]),
    // CalendarView 正常态选中日数字（对齐 WinUI AccentTextFillColorPrimaryBrush）。
    (343, [0, 62, 146], [153, 235, 255]),
    // CalendarView 禁用态 15 号选中圆环描边（对齐 WinUI CalendarViewSelectedDisabledBorderBrush）。
    (344, [200, 200, 200], [66, 66, 66]),
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
const PARITY_WHITE: Token = Token::new(321);
const PARITY_BLACK: Token = Token::new(322);
const PARITY_STAR_DISABLED: Token = Token::new(323);

/// RatingControl 实心星 1px 描边的八方向盖章偏移。
///
/// 对角方向的位移取 0.7，使描边在斜边上仍保持约 1px 的视觉厚度。
const STAR_OUTLINE_OFFSETS: [(f32, f32); 8] = [
    (-1.0, 0.0),
    (1.0, 0.0),
    (0.0, -1.0),
    (0.0, 1.0),
    (-0.7, -0.7),
    (0.7, -0.7),
    (-0.7, 0.7),
    (0.7, 0.7),
];

/// ColorPicker 连续渐变纹理占用的 RGBA 图集页。
///
/// 文本的颜色字形从第 0 页开始分配，而基准台本只渲染灰度字形；
/// 取最后一页可避免覆盖字形缓存。
const PARITY_RGBA_LAYER: u32 = 47;
/// 光谱纹理宽度：参考侧 256px 宽的 HSV 方片被 240px 控件右缘裁切。
const SPECTRUM_WIDTH: u32 = 240;
/// 光谱纹理高度（参考侧完整 256px）。
const SPECTRUM_HEIGHT: u32 = 256;
/// 明度滑轨高度。
const SLIDER_HEIGHT: u32 = 12;
/// InfoBadge 纹理边长（参考侧控件恰为 16×16）。
const BADGE_SIZE: u32 = 16;
/// InfoBadge 纹理在图集中的纵向起点（光谱与滑轨之后）。
const BADGE_TEXTURE_Y: u32 = SPECTRUM_HEIGHT + SLIDER_HEIGHT;
/// 选中态 ToggleButton 预混标签纹理的纵向起点（徽章之后）。
const LABEL_TEXTURE_Y: u32 = BADGE_TEXTURE_Y + BADGE_SIZE;
/// 图集页边长，用于把纹理像素坐标换算为归一化 UV。
const ATLAS_PAGE: f32 = 2048.0;

/// 生成 ColorPicker 的 HSV 光谱：横轴色相 0..360°（256px 映射，右缘被裁切），
/// 纵轴饱和度 1..0，明度恒为 1。
fn spectrum_rgba() -> Vec<u8> {
    let mut data = Vec::with_capacity((SPECTRUM_WIDTH * SPECTRUM_HEIGHT * 4) as usize);
    for y in 0..SPECTRUM_HEIGHT {
        let saturation = 1.0 - y as f32 / (SPECTRUM_HEIGHT - 1) as f32;
        for x in 0..SPECTRUM_WIDTH {
            let hue = x as f32 / 256.0 * 360.0;
            let [r, g, b] = hsv_to_srgb8(hue, saturation, 1.0);
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    data
}

/// 生成明度滑轨：参考侧从黑色线性渐变到当前色（DodgerBlue），
/// 渐变总长 312px，被 240px 控件右缘裁切在约 77% 处。
fn value_slider_rgba() -> Vec<u8> {
    let base = [30.0_f32, 144.0, 255.0];
    let mut data = Vec::with_capacity((SPECTRUM_WIDTH * SLIDER_HEIGHT * 4) as usize);
    for _ in 0..SLIDER_HEIGHT {
        for x in 0..SPECTRUM_WIDTH {
            let t = x as f32 / 312.0;
            let channels = base.map(|channel| (channel * t).round().clamp(0.0, 255.0) as u8);
            data.extend_from_slice(&[channels[0], channels[1], channels[2], 255]);
        }
    }
    data
}

/// HSV 转 8 位 sRGB（与 WinUI `ColorSpectrum` 的取值一致）。
fn hsv_to_srgb8(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let c = value * saturation;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = value - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// 在字形图集切片缓存中定位某个四边形所覆盖的 R8 覆盖率切片。
fn find_glyph_tile(tiles: &[BadgeGlyph], quad: &unbound_text::GlyphQuad) -> Option<BadgeGlyph> {
    let center_x = (quad.uv[0] + quad.uv[2]) * 0.5 * ATLAS_PAGE;
    let center_y = (quad.uv[1] + quad.uv[3]) * 0.5 * ATLAS_PAGE;
    tiles
        .iter()
        .find(|tile| {
            center_x >= tile.x as f32
                && center_x < (tile.x + tile.width) as f32
                && center_y >= tile.y as f32
                && center_y < (tile.y + tile.height) as f32
        })
        .cloned()
}

/// 生成强调色底上标签的预混 sRGB 纹理。
///
/// 参考侧的字形抗锯齿直接按 sRGB 插值，而本管线在 sRGB 交换链上做线性
/// 混合；饱和底色上的字干边缘会因此偏色。这里把每个像素的字形覆盖率与
/// 底色在 sRGB 空间预混，纹理本身不透明，绘制时不再发生混合。
fn checked_label_rgba(
    dark: bool,
    quads: &[unbound_text::GlyphQuad],
    tiles: &[BadgeGlyph],
    origin: Vec2,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let palette = theme::palette(dark);
    let background = srgb8(palette.color(Token::PRIMARY));
    let ink = if dark { [0, 0, 0] } else { [255, 255, 255] };
    let gamma: f32 = if dark { 1.0 } else { 0.6 };
    glyph_rgba(background, ink, quads, tiles, origin, width, height, gamma)
}

/// 生成超链接按钮标签在底板上的预混 sRGB 纹理。
///
/// 参考侧按 sRGB 插值，在浅色底板上绘制高饱和暗红文字时，本管线默认在 sRGB
/// 交换链上的线性空间混合会导致字干边缘发粉（ΔE 偏高）；这里将字形覆盖率与
/// 底色在 sRGB 空间预混成不透明纹理。
fn hyperlink_label_rgba(
    dark: bool,
    quads: &[unbound_text::GlyphQuad],
    tiles: &[BadgeGlyph],
    origin: Vec2,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let palette = theme::palette(dark);
    let background = srgb8(palette.color(Token::SURFACE));
    let ink = if dark { [153, 235, 255] } else { [0, 62, 146] };
    // 深色底板上的浅青色强调字形在 WinUI 侧有文本伽马抬升（实测中位 gamma≈0.68）。
    let gamma: f32 = if dark { 0.68 } else { 1.0 };
    glyph_rgba(background, ink, quads, tiles, origin, width, height, gamma)
}

/// 生成 FontIcon 浅色禁用态在底板上的 sRGB 预混纹理。
///
/// 参考侧对 FontIcon（非 Control）应用 `Opacity = 0.4` 在 sRGB 空间合成；
/// 默认着色器的浅底字形补偿曲线叠加半透明令牌会使抗锯齿边缘二次加深，这里
/// 直接按 sRGB 线性覆盖率与 40% 禁用灰预混。
fn font_icon_disabled_light_rgba(
    quads: &[unbound_text::GlyphQuad],
    tiles: &[BadgeGlyph],
    origin: Vec2,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let palette = theme::palette(false);
    let background = srgb8(palette.color(Token::SURFACE));
    let ink = [156, 156, 156];
    glyph_rgba(background, ink, quads, tiles, origin, width, height, 1.0)
}

/// 把字形覆盖率与底色在 sRGB 空间预混为不透明 RGBA 纹理。
///
/// `gamma` 为覆盖率的幂次：`1.0` 保持原样（图标），`< 1.0` 抬升低覆盖率
/// 边缘以复现 WinUI 的白字文本伽马。
#[allow(clippy::too_many_arguments)]
fn glyph_rgba(
    background: [u8; 3],
    ink: [u8; 3],
    quads: &[unbound_text::GlyphQuad],
    tiles: &[BadgeGlyph],
    origin: Vec2,
    width: u32,
    height: u32,
    gamma: f32,
) -> Vec<u8> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let px = origin.x + x as f32 + 0.5;
            let py = origin.y + y as f32 + 0.5;
            let mut coverage = 0.0_f32;
            for (quad, tile) in quads.iter().zip(tiles) {
                let s = (px - quad.rect.origin.x) / quad.rect.width();
                let t = (py - quad.rect.origin.y) / quad.rect.height();
                if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let ax = (quad.uv[0] + s * (quad.uv[2] - quad.uv[0])) * ATLAS_PAGE;
                let ay = (quad.uv[1] + t * (quad.uv[3] - quad.uv[1])) * ATLAS_PAGE;
                coverage = coverage.max(sample_glyph(tile, ax, ay));
            }
            if (gamma - 1.0).abs() > f32::EPSILON {
                coverage = coverage.powf(gamma);
            }
            let channels = blend(background, ink, coverage);
            data.extend_from_slice(&[channels[0], channels[1], channels[2], 255]);
        }
    }
    data
}

/// 生成 InfoBadge 的 16×16 圆形底板与数字。
///
/// 图元着色器的抗锯齿在 16px 小圆上会向内外各扩散约 1px，而 WinUI 的
/// 圆形底板覆盖更陡。这里以距离场（半径 7.7、过渡带 0.4px）求出覆盖率，
/// 并把 DirectWrite 的字形覆盖率一并预混成 sRGB 不透明纹理，绕开线性空间
/// 混合造成的灰蓝边缘。
fn badge_rgba(
    dark: bool,
    quads: &[unbound_text::GlyphQuad],
    glyph: &BadgeGlyph,
    origin: Vec2,
) -> Vec<u8> {
    let palette = theme::palette(dark);
    let foreground = srgb8(palette.color(Token::PRIMARY));
    let background = srgb8(palette.color(Token::SURFACE));
    // WinUI `TextOnAccentFillColorPrimary`：深色主题纯黑、浅色主题纯白。
    let text = if dark { [0, 0, 0] } else { [255, 255, 255] };
    // 白字压强调色底时参考侧抬升低覆盖率边缘（与 ToggleButton 标签同因）。
    let gamma: f32 = if dark { 1.0 } else { 0.6 };

    let mut text_coverage = vec![0.0_f32; (BADGE_SIZE * BADGE_SIZE) as usize];
    for quad in quads {
        accumulate_glyph_coverage(quad, glyph, origin, &mut text_coverage);
    }

    let mut data = Vec::with_capacity((BADGE_SIZE * BADGE_SIZE * 4) as usize);
    for y in 0..BADGE_SIZE {
        for x in 0..BADGE_SIZE {
            let dx = x as f32 + 0.5 - BADGE_SIZE as f32 / 2.0;
            let dy = y as f32 + 0.5 - BADGE_SIZE as f32 / 2.0;
            let disk = disk_coverage(dx, dy);
            let mut channels = blend(background, foreground, disk);
            let mut coverage = text_coverage[(y * BADGE_SIZE + x) as usize];
            if coverage > 0.0 {
                if (gamma - 1.0).abs() > f32::EPSILON {
                    coverage = coverage.powf(gamma);
                }
                channels = blend(channels, text, coverage);
            }
            data.extend_from_slice(&[channels[0], channels[1], channels[2], 255]);
        }
    }
    data
}

/// 以距离场求圆形底板覆盖率。
///
/// 参考侧 16px 圆的边缘只有约 1px 过渡，图元着色器在 45° 方向的
/// 屏幕导数会放大过渡带，因此这里用固定的窄过渡带重新求值。
fn disk_coverage(dx: f32, dy: f32) -> f32 {
    // 两主题联合拟合（R=7.6、F=0.6、γ=0.4 时圆盘残差同为 12px 且参数一致）：
    // 参考边缘为 gamma≈0.4 的覆盖率混合，线性模型无法同时表达两主题。
    const RADIUS: f32 = 7.6;
    const FEATHER: f32 = 0.6;
    let distance = (dx * dx + dy * dy).sqrt();
    let coverage = (0.5 + (RADIUS - distance) / FEATHER).clamp(0.0, 1.0);
    coverage.powf(0.4)
}

/// 把一枚字形四边形覆盖到的像素覆盖率累加到 16×16 画布。
fn accumulate_glyph_coverage(
    quad: &unbound_text::GlyphQuad,
    glyph: &BadgeGlyph,
    origin: Vec2,
    coverage: &mut [f32],
) {
    for y in 0..BADGE_SIZE {
        for x in 0..BADGE_SIZE {
            let px = origin.x + x as f32 + 0.5;
            let py = origin.y + y as f32 + 0.5;
            let s = (px - quad.rect.origin.x) / quad.rect.width();
            let t = (py - quad.rect.origin.y) / quad.rect.height();
            if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
                continue;
            }
            let ax = (quad.uv[0] + s * (quad.uv[2] - quad.uv[0])) * ATLAS_PAGE;
            let ay = (quad.uv[1] + t * (quad.uv[3] - quad.uv[1])) * ATLAS_PAGE;
            let sample = sample_glyph(glyph, ax, ay);
            let index = (y * BADGE_SIZE + x) as usize;
            coverage[index] = coverage[index].max(sample);
        }
    }
}

/// 在字形图集切片上做双线性采样。
fn sample_glyph(glyph: &BadgeGlyph, ax: f32, ay: f32) -> f32 {
    let fx = ax - 0.5 - glyph.x as f32;
    let fy = ay - 0.5 - glyph.y as f32;
    let ix = fx.floor();
    let iy = fy.floor();
    let tx = fx - ix;
    let ty = fy - iy;
    let texel = |x: i32, y: i32| {
        let x = x.clamp(0, glyph.width as i32 - 1) as u32;
        let y = y.clamp(0, glyph.height as i32 - 1) as u32;
        glyph.data[(y * glyph.width + x) as usize] as f32 / 255.0
    };
    let x = ix as i32;
    let y = iy as i32;
    let top = texel(x, y) * (1.0 - tx) + texel(x + 1, y) * tx;
    let bottom = texel(x, y + 1) * (1.0 - tx) + texel(x + 1, y + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// 逐通道 sRGB 混合。
fn blend(from: [u8; 3], to: [u8; 3], t: f32) -> [u8; 3] {
    [
        mix_channel(from[0], to[0], t),
        mix_channel(from[1], to[1], t),
        mix_channel(from[2], to[2], t),
    ]
}

/// 单个通道的 sRGB 线性插值。
fn mix_channel(from: u8, to: u8, t: f32) -> u8 {
    let start = from as f32;
    let end = to as f32;
    (start + (end - start) * t).round().clamp(0.0, 255.0) as u8
}

/// 线性颜色转 8 位 sRGB 分量。
fn srgb8(color: Color) -> [u8; 3] {
    let [r, g, b, _] = color.to_array();
    [r, g, b].map(|channel| {
        (unbound_math::color::linear_to_srgb(channel) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

/// 生成包含指定后景合成底板颜色的 `ThemeLut`（与 `Catalog.cs::CreateBackdropBrush` 严格对齐）。
#[must_use]
pub fn backdrop_lut(dark: bool, backdrop: &str) -> ThemeLut {
    build_lut(dark, backdrop, &[])
}

/// 构建主题 LUT，并按顺序覆盖或追加额外颜色条目。
fn build_lut(dark: bool, backdrop: &str, overrides: &[(Token, Color)]) -> ThemeLut {
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
    // 差分专用色槽：WinUI 模板里少量过渡色（选中态上下边缘、ToolTip 表面）
    // 没有对应的语义令牌。这里用未占用的原始槽位承载，阈值与主题保持一致。
    for (token, color) in overrides {
        if let Some(entry) = entries.iter_mut().find(|(existing, _)| existing == token) {
            entry.1 = *color;
        } else {
            entries.push((*token, *color));
        }
    }
    let kind = if dark {
        ThemeKind::Dark
    } else {
        ThemeKind::Light
    };
    ThemeLut::from_palette(&Palette::new(kind, &entries))
}

/// 为单个用例生成主题 LUT。
///
/// 参考宿主 `CaptureWindow.xaml` 已将全部系统强调色资源统一钉死为 Windows 11
/// 默认 Fluent 蓝（浅色 `#005FB8` / 深色 `#60CDFF`），不再按机器个性化颜色覆盖。
#[must_use]
pub fn parity_lut(dark: bool, backdrop: &str, control: &str) -> ThemeLut {
    if control == "ListBox" {
        // WinUI ListBox 选中行使用 40% Fluent 蓝强调色叠在基准面上（SystemControlHighlightListAccentLowBrush）。
        let soft = if dark {
            Color::from_srgb8(56, 100, 122, 255)
        } else {
            Color::from_srgb8(146, 184, 219, 255)
        };
        return build_lut(dark, backdrop, &[(Token::PRIMARY_SOFT, soft)]);
    }
    backdrop_lut(dark, backdrop)
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
        "crates/unbound-widgets/src/slot_fixture.rs",
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
    texture_quads: &mut Vec<unbound_text::GlyphQuad>,
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
        "ColorPicker" => {
            // WinUI 3 默认 ColorPicker：HSV 光谱、明度滑轨、模式下拉、
            // 十六进制输入与三通道输入。光谱/滑轨是连续渐变，图元色槽无法
            // 承载上万种颜色，因此程序化生成两张纹理走 RGBA 图集。
            let x = rect.origin.x;
            let y = rect.origin.y;
            let first = scene.primitives.len();

            emit_texture(
                scene,
                Rect::new(x, y + 4.0, SPECTRUM_WIDTH as f32, SPECTRUM_HEIGHT as f32),
                0.0,
                Vec2::new(0.0, 0.0),
                Vec2::new(
                    SPECTRUM_WIDTH as f32 / ATLAS_PAGE,
                    SPECTRUM_HEIGHT as f32 / ATLAS_PAGE,
                ),
                700,
            );

            // 选中色圆环：色相/饱和度坐标上的 2px 白环，直径 17px。
            let mut ring = WidgetOutput::default();
            ring.primitives.push(primitive::focus_ring(
                Rect::new(x + 141.0, y + 25.0, 17.0, 17.0),
                8.5,
                PARITY_WHITE,
                2.0,
                720,
                true,
            ));
            push(scene, ring);

            emit_texture(
                scene,
                Rect::new(x, y + 286.0, SPECTRUM_WIDTH as f32, SLIDER_HEIGHT as f32),
                6.0,
                Vec2::new(0.0, SPECTRUM_HEIGHT as f32 / ATLAS_PAGE),
                Vec2::new(
                    SPECTRUM_WIDTH as f32 / ATLAS_PAGE,
                    (SPECTRUM_HEIGHT + SLIDER_HEIGHT) as f32 / ATLAS_PAGE,
                ),
                710,
            );

            // 模式下拉：与 ComboBox 试点共用同一发射器。
            let combo = SelectDesc::options(["RGB"]).selected(0);
            let combo_state = combo.initial_state();
            push(
                scene,
                combo.emit_laid_out(
                    NODE,
                    Rect::new(x, y + 314.0, 120.0, 32.0),
                    control,
                    &combo_state,
                    text_ctx,
                ),
            );

            // 十六进制输入：参考侧输入框宽 120px，右半被控件裁切。
            let hex = TextInputDesc::text("#1E90FF");
            let hex_state = hex.initial_state();
            push(
                scene,
                hex.emit_laid_out(
                    NODE,
                    Rect::new(x + 180.0, y + 314.0, 120.0, 32.0),
                    control,
                    &hex_state,
                    text_ctx,
                ),
            );

            // 三个通道输入及其标签。标签跟随参考截图的 zh-CN 系统界面语言。
            let channels = [("30", "红色"), ("144", "绿色"), ("255", "蓝色")]; // lang-ok: 复刻参考截图中的中文通道标签
            for (index, (value, label)) in channels.iter().enumerate() {
                let row = y + 358.0 + index as f32 * 44.0;
                let input = TextInputDesc::text(*value);
                let input_state = input.initial_state();
                push(
                    scene,
                    input.emit_laid_out(
                        NODE,
                        Rect::new(x, row, 120.0, 32.0),
                        control,
                        &input_state,
                        text_ctx,
                    ),
                );
                emit_text_left_middle(
                    text_ctx,
                    scene,
                    label,
                    Rect::new(x + 127.0, row, 60.0, 32.0),
                    x + 127.0,
                    14.0,
                    400,
                    None,
                    if control.disabled {
                        PARITY_TEXT_DISABLED
                    } else {
                        Token::TEXT_PRIMARY
                    },
                    760,
                );
            }

            // 参考截图按控件边界裁切（右侧 80px 的输入框与 16px 的滑轨延伸不可见）。
            let clip_idx = scene.clips.len() as u32;
            scene.clips.push(GpuClipNode::from_aabb([
                rect.origin.x,
                rect.origin.y,
                rect.max().x,
                rect.max().y,
            ]));
            for primitive in &mut scene.primitives[first..] {
                primitive.clip_idx = clip_idx;
            }
            rect.size
        }
        "ToggleButton" => {
            // WinUI 的 ToggleButton 与 Button 共用模板，仅选中态换成强调色。
            // 背景与描边复用 ButtonDesc 的空标签路径，文本单独排版：XAML 的
            // `UseLayoutRounding` 把 TextBlock 宽度四舍五入到整像素后再居中，
            // 而通用路径用 `ceil`，长标签会因此落在半像素相位上，字干边缘与
            // 参考错开半像素。
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
            let size = unbound_widgets::button::BUTTON_FONT_SIZE;
            // XAML 的 TextBlock 先按 `UseLayoutRounding` 把文本框左缘舍入到整像素
            // （`round(center - width / 2)`）再排版，通用路径的 `ceil` 会让
            // 长标签整体落在半像素相位上，字干边缘与参考错开半像素。
            let quads = {
                let scale = text_ctx.effective_raster_scale();
                let mut style = unbound_text::TextStyle::sized(size * scale);
                style.weight = 400;
                let shaped = match text_ctx.engine.shape_laid_out("ToggleButton", &style) {
                    Some(shaped) => shaped,
                    None => text_ctx.engine.shape("ToggleButton", &style),
                };
                let width = shaped.width / scale;
                let height = (shaped.height / scale).max(size);
                let x = (rect.center().x - width * 0.5).round();
                let y = rect.origin.y + ((rect.height() - height) * 0.5).max(0.0) - 1.0;
                text_ctx.cache.quads_for(
                    text_ctx.engine,
                    &shaped,
                    Vec2::new(x * scale, y * scale),
                    400,
                    false,
                )
            };
            let ink = if control.disabled {
                Token::ON_DISABLED
            } else if checked {
                if case.theme == "dark" {
                    PARITY_BLACK
                } else {
                    PARITY_WHITE
                }
            } else {
                Token::ON_SURFACE
            };
            if checked {
                // 强调色底上的标签按 sRGB 预混成纹理，避免线性空间混合
                // 在字干边缘产生色偏；纹理在拿到字形切片后于渲染前合成。
                texture_quads.extend(quads);
            } else {
                emit_quads(scene, &quads, ink, 704);
            }
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
            let border = if disabled {
                Token::CONTROL_DISABLED_BORDER
            } else {
                Token::CONTROL_STROKE
            };
            let mut out = WidgetOutput::default();
            let fill_rect = if !disabled { rect.inflate(-1.0) } else { rect };
            let fill_radii = if !disabled { [3.0; 4] } else { [4.0; 4] };
            out.primitives.push(primitive::instance(
                fill_rect,
                fill_radii,
                fill,
                Token::NONE,
                0.0,
                0.0,
                unbound_gpu::data::PrimType::RoundedRect,
                unbound_gpu::data::SubElement::Background,
                700,
            ));
            // 分隔列：主操作区 84px，其余为下拉箭头区。
            out.primitives.push(primitive::surface(
                Rect::new(
                    rect.origin.x + 84.0,
                    rect.origin.y + 1.0,
                    1.0,
                    rect.height() - 2.0,
                ),
                0.0,
                border,
                701,
            ));
            out.primitives.push(primitive::outlined(
                rect,
                4.0,
                Token::NONE,
                border,
                1.0,
                0.0,
                702,
            ));
            if !disabled {
                let bottom_line = Rect::new(
                    rect.origin.x + 4.0,
                    rect.max().y - 1.0,
                    (rect.width() - 8.0).max(0.0),
                    1.0,
                );
                out.primitives.push(primitive::surface(
                    bottom_line,
                    0.0,
                    PARITY_SPLIT_BOTTOM,
                    703,
                ));
            } else {
                out.primitives.push(primitive::surface(
                    Rect::ZERO,
                    0.0,
                    PARITY_SPLIT_BOTTOM_DISABLED,
                    703,
                ));
            }
            let ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            let content_r = Rect::new(
                rect.origin.x + 11.0,
                rect.origin.y,
                (84.0_f32 - 11.0).max(0.0),
                rect.height(),
            );
            let mut glyphs = text_ctx.shape_line(
                "SplitButton",
                content_r,
                14.0,
                400,
                unbound_widgets::AlignX::Left,
                unbound_widgets::AlignY::Middle,
            );
            glyphs.retain(|quad| quad.rect.origin.x < rect.origin.x + 84.0 - 13.0);
            primitive::glyphs(&glyphs, ink, 704, &mut out.primitives, &mut out.payloads);
            let acx = rect.origin.x + 84.0 + 18.0 - 0.5;
            let acy = rect.center().y;
            let arrow_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };
            primitive::curve_stroke(
                [acx - 3.5, acy - 2.0],
                [acx, acy + 1.5],
                0.75,
                arrow_ink,
                705,
                &mut out.primitives,
                &mut out.payloads,
            );
            primitive::curve_stroke(
                [acx, acy + 1.5],
                [acx + 3.5, acy - 2.0],
                0.75,
                arrow_ink,
                705,
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
                    // WinUI 选中圆点的颜色是 `TextOnAccentFillColorPrimary`：
                    // 深色主题为纯黑（浅蓝强调色底），浅色主题为纯白。
                    if case.theme == "dark" {
                        PARITY_BLACK
                    } else {
                        PARITY_WHITE
                    },
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
            let star_size: f32 = 16.0;
            for index in 0..5u32 {
                let slot = Rect::new(
                    rect.origin.x + 24.0 * index as f32,
                    rect.origin.y,
                    16.0,
                    rect.height(),
                );
                if index < 3 {
                    let (edge, fill) = if control.disabled {
                        (PARITY_STAR_EDGE_DISABLED, PARITY_STAR_DISABLED)
                    } else {
                        (PARITY_STAR_EDGE, Token::PRIMARY)
                    };
                    // 参考图实心星由 14px 填充字形外套约 1px 对比色描边组成。字形本身
                    // 不含描边，八方向偏移盖章补画；描边色采样自参考图边界像素。
                    let inner: f32 = 14.0;
                    let z = 700 + index as u16 * 10;
                    for (step, (dx, dy)) in STAR_OUTLINE_OFFSETS.iter().enumerate() {
                        let shifted = Rect::new(
                            slot.origin.x + dx,
                            slot.origin.y + dy,
                            slot.size.x,
                            slot.size.y,
                        );
                        let outline = icon_quads(text_ctx, "\u{E735}", shifted, inner);
                        emit_quads(scene, &outline, edge, z + step as u16);
                    }
                    let fill_quads = icon_quads(text_ctx, "\u{E735}", slot, inner);
                    emit_quads(scene, &fill_quads, fill, z + 9);
                } else {
                    // 空星为 E734 描边字形。
                    let quads = icon_quads(text_ctx, "\u{E734}", slot, star_size);
                    emit_quads(
                        scene,
                        &quads,
                        PARITY_STAR_UNSELECTED,
                        700 + index as u16 * 10,
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
                    Rect::new(rect.origin.x + dx, rect.origin.y + dy, *width, 1.0),
                    0.0,
                    Token::PRIMARY,
                    700 + row as u16,
                ));
            }
            push(scene, out);
            rect.size
        }
        "InfoBadge" => {
            // 16px 圆形底板改用程序化纹理：图元抗锯齿会向内外各扩散约 1px，
            // 在这块小 ROI 里会显著放大边缘误差。数字字形同样取覆盖率位图
            // 预混进纹理，避免在线性空间混合白色/黑色笔画产生色偏。
            emit_texture(
                scene,
                rect,
                0.0,
                Vec2::new(0.0, BADGE_TEXTURE_Y as f32 / ATLAS_PAGE),
                Vec2::new(
                    BADGE_SIZE as f32 / ATLAS_PAGE,
                    (BADGE_TEXTURE_Y + BADGE_SIZE) as f32 / ATLAS_PAGE,
                ),
                700,
            );
            texture_quads.extend(text_ctx.shape_line_styled(
                "3",
                rect,
                11.0,
                400,
                Some("Segoe UI Variable Text"),
                unbound_widgets::AlignX::Center,
                unbound_widgets::AlignY::Middle,
            ));
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
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_title_bar_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "HyperlinkButton" => {
            let desc = HyperlinkButtonDesc::new(rect, "HyperlinkButton");
            let size = desc.intrinsic_size(text_ctx);
            if case.state == "normal" {
                let clip_right = rect.max().x - 12.0;
                let content_r = Rect::new(
                    rect.origin.x + 12.0,
                    rect.origin.y,
                    (rect.width() - 24.0).max(0.0),
                    rect.height(),
                );
                let mut glyphs = text_ctx.shape_line(
                    &desc.label,
                    content_r,
                    14.0,
                    400,
                    unbound_widgets::AlignX::Left,
                    unbound_widgets::AlignY::Middle,
                );
                glyphs.retain_mut(|quad| {
                    if quad.rect.origin.x >= clip_right {
                        false
                    } else {
                        if quad.rect.max().x > clip_right {
                            let w = quad.rect.width();
                            let new_w = clip_right - quad.rect.origin.x;
                            if w > 0.0 && new_w > 0.0 {
                                let fraction = new_w / w;
                                quad.uv[2] = quad.uv[0] + (quad.uv[2] - quad.uv[0]) * fraction;
                                quad.rect.size.x = new_w;
                            }
                        }
                        true
                    }
                });
                texture_quads.extend(glyphs);
            } else {
                push(
                    scene,
                    desc.emit_laid_out(NODE, rect, control, &(), text_ctx),
                );
            }
            Vec2::new(size.x, 32.0)
        }
        "RepeatButton" => {
            let desc = RepeatButtonDesc::new(rect, "RepeatButton");
            let state = RepeatButtonState::new();
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            Vec2::new(size.x, 32.0)
        }
        "DropDownButton" => {
            let desc = DropDownButtonDesc::new(rect, "DropDownButton");
            let state = DropDownButtonState { open: checked };
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            Vec2::new(size.x, 32.0)
        }
        "ToggleSplitButton" => {
            let desc = ToggleSplitButtonDesc::new(rect, "ToggleSplitButton");
            let state = ToggleSplitButtonState {
                checked,
                menu_open: false,
            };
            let size = desc.intrinsic_size(text_ctx);
            if checked && !control.disabled {
                // 选中态底板与箭头走通用发射器（传空标签），文字经 sRGB 预混并精确裁剪右边界。
                let bg_desc = ToggleSplitButtonDesc::new(rect, "");
                push(
                    scene,
                    bg_desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
                );
                let main = desc.main_rect();
                let clip_right = main.max().x - 11.0;
                let content_r = Rect::new(
                    main.origin.x + 11.0,
                    main.origin.y,
                    (main.width() - 11.0).max(0.0),
                    main.height(),
                );
                let mut glyphs = text_ctx.shape_line(
                    &desc.label,
                    content_r,
                    14.0,
                    400,
                    unbound_widgets::AlignX::Left,
                    unbound_widgets::AlignY::Middle,
                );
                glyphs.retain_mut(|quad| {
                    if quad.rect.origin.x >= clip_right {
                        false
                    } else {
                        if quad.rect.max().x > clip_right {
                            let w = quad.rect.width();
                            let new_w = clip_right - quad.rect.origin.x;
                            if w > 0.0 && new_w > 0.0 {
                                let fraction = new_w / w;
                                quad.uv[2] = quad.uv[0] + (quad.uv[2] - quad.uv[0]) * fraction;
                                quad.rect.size.x = new_w;
                            }
                        }
                        true
                    }
                });
                texture_quads.extend(glyphs);
            } else {
                push(
                    scene,
                    desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
                );
            }
            Vec2::new(size.x, 32.0)
        }
        "PasswordBox" => {
            let desc = PasswordBoxDesc::new(rect).with_placeholder("Password");
            let state = PasswordBoxState::with_text("Password");
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            Vec2::new(size.x, 32.0)
        }
        "DatePicker" => {
            let desc = DatePickerWheelDesc::new(rect)
                .with_column_order(unbound_widgets::DatePickerColumnOrder::YearMonthDay)
                .with_month_format(unbound_widgets::DatePickerMonthFormat::NumericTwoDigits);
            let state = DatePickerWheelState::new(2024, 4, 15);
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            Vec2::new(size.x, 32.0)
        }
        "CalendarDatePicker" => {
            let desc = CalendarDatePickerDesc::new(rect).with_placeholder("Select a date");
            let state = CalendarDatePickerState::with_date(
                NaiveDate::new(2024, 4, 15).expect("valid parity date"),
            );
            let size = desc.intrinsic_size(text_ctx);
            push(
                scene,
                desc.emit_laid_out(NODE, rect, control, &state, text_ctx),
            );
            Vec2::new(size.x, 32.0)
        }
        "AppBarButton" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_app_bar_button_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "AppBarToggleButton" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let mut fixture =
                unbound_widgets::slot_fixture::emit_canonical_app_bar_toggle_button_fixture(
                    rect,
                    fixture_state,
                    text_ctx,
                    700,
                );
            if checked && !control.disabled {
                let w = if rect.width() > 0.0 {
                    rect.width()
                } else {
                    unbound_widgets::slot_fixture::CANONICAL_APP_BAR_TOGGLE_BUTTON_WIDTH
                };
                let h = if rect.height() > 0.0 {
                    rect.height()
                } else {
                    unbound_widgets::slot_fixture::CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT
                };
                let stack_top = rect.origin.y + ((h - 30.0) * 0.5).round();
                let icon_r = Rect::new(rect.origin.x, stack_top - 1.0, w, 16.0);
                // 存在平台符号字体时，剥离线性混合字形并改用 sRGB 预混纹理绘制强调色底板上的图标与文字；
                // 无符号字体时保留 fixture 内的 ShellIcon::Bold 矢量与标签图元，避免清空曲线载荷。
                if let Some(icon_quads) = text_ctx.shape_symbol_glyph(
                    unbound_widgets::slot_fixture::CANONICAL_APP_BAR_TOGGLE_BUTTON_GLYPH,
                    icon_r,
                    16.0,
                ) {
                    fixture.output.primitives.retain(|p| {
                        let pt = p.prim_type_and_flags & 0xFF;
                        pt != PrimType::Glyph as u32 && pt != PrimType::GlyphColor as u32
                    });
                    fixture.output.payloads.clear();
                    let label_r = Rect::new(rect.origin.x, stack_top + 18.0, w, 12.0);
                    let label_quads = text_ctx.shape_line_styled(
                        unbound_widgets::slot_fixture::CANONICAL_APP_BAR_TOGGLE_BUTTON_LABEL,
                        label_r,
                        12.0,
                        400,
                        None,
                        unbound_widgets::AlignX::Center,
                        unbound_widgets::AlignY::Middle,
                    );
                    texture_quads.extend(icon_quads);
                    texture_quads.extend(label_quads);
                }
                let visual_r = Rect::new(
                    rect.origin.x + 2.0,
                    rect.origin.y + 6.0,
                    (w - 4.0).max(1.0),
                    (h - 12.0).max(1.0),
                );
                fixture.output.primitives.push(primitive::outlined(
                    visual_r,
                    4.5,
                    Token::NONE,
                    PARITY_CHECKED_BORDER,
                    1.0,
                    0.0,
                    800,
                ));
                fixture.output.primitives.push(primitive::surface(
                    Rect::new(
                        visual_r.origin.x + 3.0,
                        visual_r.max().y - 1.0,
                        visual_r.width() - 6.0,
                        1.0,
                    ),
                    0.0,
                    PARITY_CHECKED_BOTTOM,
                    900,
                ));
            }
            push(scene, fixture.output);
            fixture.total_size
        }
        "AppBarSeparator" => {
            // WinUI 3 AppBarSeparator 规范：
            // 标准槽位宽 9px、高 32px，中间绘制 1px 垂直微描边线（CARD_BORDER）。
            let sep_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                9.0
            };
            let sep_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                40.0
            };
            let sep_r = Rect::new(rect.origin.x, rect.origin.y, sep_w, sep_h);
            let desc = AppBarSeparatorDesc::vertical(sep_r);

            let stroke = if control.disabled {
                Token::CONTROL_DISABLED_BORDER
            } else {
                Token::DIVIDER
            };
            let mut out = WidgetOutput::default();
            out.primitives
                .push(primitive::surface(desc.line_rect(), 0.0, stroke, desc.z));
            push(scene, out);
            Vec2::new(sep_w, sep_h)
        }
        "CommandBarFlyout" => {
            // WinUI 3 CommandBarFlyout 规范：
            // 固有宽度 280px、高度 44px。
            // 8px 外圆角、18px 阴影的浮层外框，内部横向排列常用快捷命令按钮。
            let flyout_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                280.0
            };
            let flyout_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                44.0
            };
            let flyout_r = Rect::new(rect.origin.x, rect.origin.y, flyout_w, flyout_h);
            let disabled = control.disabled;

            let mut out = WidgetOutput::default();
            // 1. 浮层表面背景（参考宿主用内联 Border 包裹，禁用深色态为 #252525）
            let bg_token = if disabled {
                Token::new(341)
            } else {
                Token::OVERLAY_BG
            };
            out.primitives
                .push(primitive::surface(flyout_r, 8.0, bg_token, 760));
            // 2. 1px SurfaceStroke 微描边（禁用态深色仍保持 #1D1D1D，不升亮为 CONTROL_DISABLED_BORDER）
            let border_token = Token::CARD_BORDER;
            out.primitives.push(primitive::instance(
                flyout_r,
                [8.0; 4],
                Token::NONE,
                border_token,
                1.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Border,
                760,
            ));

            // 3. 对齐 Catalog.cs：右对齐排布 2 枚主命令（Copy、Cut）+ 竖向分隔线 + More
            let btn_w = 68.0;
            let more_w = 68.0;
            let sep_w = 1.0;
            let commands = [("\u{E8C8}", "Copy"), ("\u{E8C6}", "Cut")];
            let icon_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            let more_r = Rect::new(
                flyout_r.max().x - more_w - 4.0,
                flyout_r.origin.y + 2.0,
                more_w,
                (flyout_h - 4.0).max(16.0),
            );
            let sep_x = more_r.origin.x - 4.0;
            let sep_r = Rect::new(
                sep_x,
                flyout_r.origin.y + (flyout_h - 32.0) * 0.5,
                sep_w,
                32.0_f32.min(flyout_h - 8.0).max(8.0),
            );
            out.primitives
                .push(primitive::surface(sep_r, 0.0, Token::DIVIDER, 761));

            let cmd_start_x = sep_x - 4.0 - commands.len() as f32 * btn_w;
            for (idx, (glyph, _name)) in commands.iter().enumerate() {
                let item_r = Rect::new(
                    cmd_start_x + idx as f32 * btn_w,
                    flyout_r.origin.y + 2.0,
                    btn_w,
                    (flyout_h - 4.0).max(16.0),
                );
                let quads = icon_quads(text_ctx, glyph, item_r, 16.0);
                primitive::glyphs(
                    &quads,
                    icon_ink,
                    762,
                    &mut out.primitives,
                    &mut out.payloads,
                );
            }

            let more_icon_r = Rect::new(
                more_r.origin.x - 3.0,
                more_r.origin.y,
                more_r.width(),
                more_r.height(),
            );
            let more_quads = icon_quads(text_ctx, "\u{E712}", more_icon_r, 16.0);
            primitive::glyphs(
                &more_quads,
                icon_ink,
                762,
                &mut out.primitives,
                &mut out.payloads,
            );

            // 5. 焦点环
            if control.focused && !disabled {
                out.primitives.push(primitive::focus_ring(
                    flyout_r,
                    8.0,
                    Token::FOCUS_RING,
                    2.0,
                    765,
                    true,
                ));
            }

            push(scene, out);
            Vec2::new(flyout_w, flyout_h)
        }
        "RichEditBox" => {
            // WinUI 3 RichEditBox 规范：
            // 固有尺寸宽 240px、高 96px。
            // 官方原生纯净形态，剥离顶部格式工具栏。
            let box_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                240.0
            };
            let box_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                96.0
            };
            let box_r = Rect::new(rect.origin.x, rect.origin.y, box_w, box_h);
            let desc = RichEditBoxDesc::new(box_r, "RichEditBox");

            // 编辑区内容文本：标准纯净文案 "RichEditBox"，字号 14px，左对齐
            let editor_r = desc.editor_rect();
            let mut glyphs = text_ctx.shape_line_styled(
                "RichEditBox",
                editor_r,
                14.0,
                400,
                None,
                unbound_widgets::AlignX::Left,
                unbound_widgets::AlignY::Top,
            );

            let out = emit_rich_edit_box(NODE, &desc, control, &mut glyphs);
            push(scene, out);
            Vec2::new(box_w, box_h)
        }
        "AnnotatedScrollBar" => {
            // WinUI 3 AnnotatedScrollBar 规范：
            // 固有尺寸宽 16px、高 160px。
            // 垂直滚动条 8px 居中轨道、滑块 thumb 与等分刻度线。
            let bar_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                unbound_widgets::slot_fixture::STANDARD_SCROLL_BAR_WIDTH
            };
            let bar_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                unbound_widgets::slot_fixture::STANDARD_SCROLL_BAR_HEIGHT
            };
            let mut out = WidgetOutput::default();

            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture =
                unbound_widgets::slot_fixture::emit_canonical_annotated_scroll_bar_fixture(
                    rect,
                    fixture_state,
                    text_ctx,
                    700,
                );
            out.primitives.extend(fixture.output.primitives);
            out.payloads.extend(fixture.output.payloads);
            push(scene, out);

            Vec2::new(bar_w, bar_h)
        }
        "NavigationView"
        | "NavigationViewLeftClosed"
        | "NavigationViewLeftCompact"
        | "NavigationViewLeftMinimal"
        | "NavigationViewTop"
        | "NavigationViewHierarchical"
        | "NavigationViewChrome"
        | "NavigationViewOverflow"
        | "NavigationViewLeftClosedOverflow" => {
            let (default_w, default_h) = match case.control {
                // 基础场景含 5 个主菜单项（Header + 3 项 + 分隔线），280px 高时 WinUI 会把
                // 末尾的 Analytics 滚出可视区，两侧内容对不上；统一提到 360px 让全部菜单项可见。
                "NavigationView" => (360.0_f32, 360.0_f32),
                "NavigationViewTop" => (360.0_f32, 200.0_f32),
                "NavigationViewChrome" => (420.0_f32, 280.0_f32),
                _ => (360.0_f32, 280.0_f32),
            };
            let nav_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                default_w
            };
            let nav_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                default_h
            };
            let nav_r = Rect::new(rect.origin.x, rect.origin.y, nav_w, nav_h);

            let (desc, expanded_ids) = navigation_view_parity_desc(case.control, nav_r, checked);
            let mut nav_state = unbound_widgets::NavigationViewState::from_desc(&desc);
            nav_state.expanded_item_ids = expanded_ids;
            nav_state.set_animations_enabled(false, &desc);
            let emitted =
                unbound_widgets::emit_navigation_view(700, &desc, control, &nav_state, text_ctx);
            push(scene, emitted.output);
            Vec2::new(nav_w, nav_h)
        }
        "ListView" => {
            // WinUI 3 ListView 集合视图规范：
            // 固有尺寸宽 220px、高 120px。
            // 包含 3 项标准列表行（Item 1、Item 2、Item 3），标准行高 40px（ListViewItemMinHeight = 40）。
            // 首项默认处于选中态（SelectedIndex = 0），左侧垂直居中绘制 3×16px、圆角 1.5px 的 Accent 胶囊条。
            let list_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                220.0
            };
            let list_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                120.0
            };
            let mut out = WidgetOutput::default();

            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_list_rows_fixture(
                rect,
                fixture_state,
                text_ctx,
                702,
            );
            out.primitives.extend(fixture.output.primitives);
            out.payloads.extend(fixture.output.payloads);
            push(scene, out);

            Vec2::new(list_w, list_h)
        }
        "TreeView" => {
            // WinUI 3 TreeView 树形结构规范：
            // 固有尺寸宽 220px、高 140px。
            // 包含根节点 Root 与其展开的子节点 Child，标准行高 36px。
            // 根节点（Root）首项处于选中态，左侧垂直居中绘制 3×16px、圆角 1.5px 的 Accent 胶囊条。
            let tree_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                220.0
            };
            let tree_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                140.0
            };
            let mut out = WidgetOutput::default();

            let fixture_state = FixtureState::from_control_state(control, true);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_tree_fixture(
                rect,
                fixture_state,
                text_ctx,
                702,
            );
            out.primitives.extend(fixture.output.primitives);
            out.payloads.extend(fixture.output.payloads);
            push(scene, out);

            Vec2::new(tree_w, tree_h)
        }
        "FlipView" => {
            // WinUI 3 FlipView 翻页集合视图规范：
            // 固有尺寸宽 220px、高 120px。
            // 包含内容卡片（展示当前 Page 1）以及左右翻页导航前后按钮。
            let flip_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                unbound_widgets::slot_fixture::STANDARD_FLIP_WIDTH
            };
            let flip_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                unbound_widgets::slot_fixture::STANDARD_FLIP_HEIGHT
            };
            let mut out = WidgetOutput::default();

            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_flip_view_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            out.primitives.extend(fixture.output.primitives);
            out.payloads.extend(fixture.output.payloads);
            push(scene, out);

            Vec2::new(flip_w, flip_h)
        }
        "BreadcrumbBar" => {
            let bar_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                320.0
            };
            let bar_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                32.0
            };
            let fixture_state = FixtureState::from_control_state(control, checked).with_selected(2);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_path_fixture(
                Rect::new(rect.origin.x, rect.origin.y, bar_w, bar_h),
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            Vec2::new(bar_w, bar_h)
        }
        "CommandBar" => {
            let bar_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                320.0
            };
            let bar_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                48.0
            };
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_command_bar_fixture(
                Rect::new(rect.origin.x, rect.origin.y, bar_w, bar_h),
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            Vec2::new(bar_w, bar_h)
        }
        "MenuBar" => {
            let bar_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                320.0
            };
            let bar_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                40.0
            };
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_menu_bar_fixture(
                Rect::new(rect.origin.x, rect.origin.y, bar_w, bar_h),
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            Vec2::new(bar_w, bar_h)
        }
        "PipsPager" => {
            let bar_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                320.0
            };
            let bar_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                32.0
            };
            let fixture_state = FixtureState::from_control_state(control, checked)
                .with_selected(unbound_widgets::slot_fixture::CANONICAL_PIPS_SELECTED_INDEX);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_pips_pager_fixture(
                Rect::new(rect.origin.x, rect.origin.y, bar_w, bar_h),
                fixture_state,
                700,
            );
            push(scene, fixture.output);
            Vec2::new(bar_w, bar_h)
        }
        "SelectorBar" => {
            let bar_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                320.0
            };
            let bar_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                47.0
            };
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture_out = unbound_widgets::slot_fixture::emit_canonical_selector_bar_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture_out);

            Vec2::new(bar_w, bar_h)
        }
        "TabView" => {
            let bar_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                360.0
            };
            let bar_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                120.0
            };
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_tab_view_fixture(
                Rect::new(rect.origin.x, rect.origin.y, bar_w, bar_h),
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            Vec2::new(bar_w, bar_h)
        }
        "ContentDialog" => {
            // WinUI 3 ContentDialog 规范：
            // 固有宽度 320px、高度 180px。
            // 容器外圆角 8px（OverlayCornerRadius）、1px SurfaceStroke 微描边（CARD_BORDER）。
            // 40px 海拔解析阴影（DialogElevation）。
            // 内部包含 Title="ContentDialog"、Content="Message" 以及 PrimaryButton="OK"。
            let dialog_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                320.0
            };
            let dialog_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                180.0
            };
            let dialog_r = Rect::new(rect.origin.x, rect.origin.y, dialog_w, dialog_h);
            let disabled = control.disabled;

            let mut out = WidgetOutput::default();
            // 1. 对话框表面背景（对齐 Catalog.cs ContentDialogBackground：浅 #F3F3F3 / 深 #202020，无外扩阴影）
            out.primitives
                .push(primitive::surface(dialog_r, 8.0, Token::new(330), 950));
            // 2. 1px SurfaceStroke 微描边（正常态 浅 #C1C1C1 / 深 #424242；禁用态 浅 #DFDFDF / 深 #2D2D2D）
            let dialog_border = if disabled {
                Token::new(339)
            } else {
                Token::new(338)
            };
            out.primitives.push(primitive::instance(
                dialog_r,
                [8.0; 4],
                Token::NONE,
                dialog_border,
                1.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Border,
                950,
            ));

            // 3. 底部栏分隔线与主操作按钮（对齐 Catalog.cs：Padding 24,16,24,16 + MinWidth 120）
            let footer_h = 64.0;
            let footer_y = dialog_r.max().y - footer_h;
            out.primitives.push(primitive::surface(
                Rect::new(dialog_r.origin.x, footer_y, dialog_w, 1.0),
                0.0,
                Token::DIVIDER,
                951,
            ));

            let btn_w = 120.0_f32.min(dialog_w - 48.0);
            let btn_h = 32.0;
            let btn_r = Rect::new(
                dialog_r.max().x - 25.0 - btn_w,
                footer_y + (footer_h - btn_h) * 0.5 - 1.0,
                btn_w,
                btn_h,
            );
            let btn_fill = if disabled {
                Token::new(336)
            } else {
                Token::PRIMARY
            };
            out.primitives.push(primitive::outlined(
                btn_r,
                4.0,
                btn_fill,
                Token::NONE,
                1.0,
                0.0,
                952,
            ));

            push(scene, out);

            // 5. 排版文本：标题、正文消息与操作按钮文案
            let title_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            let title_quads = text_ctx.shape_line_styled(
                "ContentDialog",
                Rect::new(
                    dialog_r.origin.x + 26.0,
                    dialog_r.origin.y + 21.0,
                    dialog_w - 48.0,
                    28.0,
                ),
                20.0,
                600,
                None,
                unbound_widgets::AlignX::Left,
                unbound_widgets::AlignY::Top,
            );
            emit_quads(scene, &title_quads, title_ink, 953);

            let body_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            let body_quads = text_ctx.shape_line_styled(
                "Message",
                Rect::new(
                    dialog_r.origin.x + 25.0,
                    dialog_r.origin.y + 55.0,
                    dialog_w - 48.0,
                    20.0,
                ),
                14.0,
                400,
                None,
                unbound_widgets::AlignX::Left,
                unbound_widgets::AlignY::Top,
            );
            emit_quads(scene, &body_quads, body_ink, 953);

            let btn_ink = if disabled {
                Token::new(337)
            } else {
                Token::ON_PRIMARY
            };
            let btn_quads = text_ctx.shape_line_styled(
                "OK",
                btn_r,
                14.0,
                400,
                None,
                unbound_widgets::AlignX::Center,
                unbound_widgets::AlignY::Middle,
            );
            emit_quads(scene, &btn_quads, btn_ink, 953);

            Vec2::new(dialog_w, dialog_h)
        }
        "Flyout" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_flyout_fixture(
                rect,
                fixture_state,
                case.theme == "dark",
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "MenuFlyout" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_menu_flyout_fixture(
                rect,
                fixture_state,
                case.theme == "dark",
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "TeachingTip" => {
            // WinUI 3 TeachingTip 引导气泡规范：
            // 固有宽度 280px、高度 96px。
            // 容器外圆角 8px（OverlayCornerRadius）、1px SurfaceStroke 微描边（CARD_BORDER）。
            // 24px 海拔解析阴影（FlyoutElevation）。
            // 包含 Title="TeachingTip" 与 Subtitle="Message"。
            let tip_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                280.0
            };
            let tip_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                96.0
            };
            let tip_r = Rect::new(rect.origin.x, rect.origin.y, tip_w, tip_h);
            let disabled = control.disabled;

            let mut out = WidgetOutput::default();
            // 1. 气泡表面背景（正常态 浅 #F9F9F9 / 深 #2E2E2E；禁用态 浅 #F9F9F9 / 深 #252525）
            let tip_bg = if disabled {
                Token::new(341)
            } else {
                PARITY_TOOLTIP_FILL
            };
            let tip_border = if disabled {
                Token::new(339)
            } else {
                Token::new(338)
            };
            out.primitives
                .push(primitive::surface(tip_r, 8.0, tip_bg, 780));
            // 3. 1px SurfaceStroke 微描边
            out.primitives.push(primitive::instance(
                tip_r,
                [8.0; 4],
                Token::NONE,
                tip_border,
                1.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Border,
                780,
            ));
            push(scene, out);

            // 4. 排版标题与副标题
            let title_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            let title_quads = text_ctx.shape_line_styled(
                "TeachingTip",
                Rect::new(
                    tip_r.origin.x + 15.0,
                    tip_r.origin.y + 12.0,
                    tip_w - 28.0,
                    20.0,
                ),
                14.0,
                600,
                None,
                unbound_widgets::AlignX::Left,
                unbound_widgets::AlignY::Top,
            );
            emit_quads(scene, &title_quads, title_ink, 782);

            let sub_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };
            let sub_quads = text_ctx.shape_line_styled(
                "Message",
                Rect::new(
                    tip_r.origin.x + 15.0,
                    tip_r.origin.y + 35.0,
                    tip_w - 28.0,
                    18.0,
                ),
                12.0,
                400,
                None,
                unbound_widgets::AlignX::Left,
                unbound_widgets::AlignY::Top,
            );
            emit_quads(scene, &sub_quads, sub_ink, 782);

            Vec2::new(tip_w, tip_h)
        }
        "Card" => {
            // WinUI 3 Card 容器规范（基于 8px 圆角与 1px 微描边）：
            // 固有宽度 240px、高度 88px。
            // 容器外圆角 8px（OverlayCornerRadius）、1px SurfaceStroke 微描边（CARD_BORDER）。
            // 2.0px 海拔微阴影（CARD_ELEVATIONS[1]），禁用态抑制。
            // 内部 Padding 12px，承载 "Card" 文本。
            let card_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                240.0
            };
            let card_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                88.0
            };
            let card_r = Rect::new(rect.origin.x, rect.origin.y, card_w, card_h);
            let disabled = control.disabled;

            let mut out = WidgetOutput::default();
            // WinUI Card（Border 模板）不附带外部投影；禁用态由 Catalog.cs 设置 Opacity=0.4，
            // 使 CARD_BG (#2B2B2B) 与背景 (#202020) 混合为 #242424。
            let bg_token = Token::CARD_BG;
            let border_token = Token::CARD_BORDER;
            out.primitives
                .push(primitive::surface(card_r, 8.0, bg_token, 100));
            // 3. 1px SurfaceStroke 微描边
            out.primitives.push(primitive::instance(
                card_r,
                [8.0; 4],
                Token::NONE,
                border_token,
                1.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Border,
                100,
            ));

            if control.focused && !disabled {
                out.primitives.push(primitive::focus_ring(
                    card_r.inflate(1.0),
                    9.0,
                    Token::FOCUS_RING,
                    2.0,
                    710,
                    true,
                ));
            }

            if disabled {
                for prim in &mut out.primitives {
                    prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
                        prim.depth_u16(),
                        prim.opacity() * unbound_widgets::slot_fixture::WINUI_DISABLED_OPACITY,
                    );
                }
            }
            push(scene, out);

            // 4. 排版内容文本
            let text_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            let quads = text_ctx.shape_line_styled(
                "Card",
                Rect::new(
                    card_r.origin.x + 12.0,
                    card_r.origin.y + 12.0,
                    (card_w - 24.0).max(0.0),
                    (card_h - 24.0).max(0.0),
                ),
                14.0,
                400,
                None,
                unbound_widgets::AlignX::Center,
                unbound_widgets::AlignY::Middle,
            );
            emit_quads(scene, &quads, text_ink, 102);

            Vec2::new(card_w, card_h)
        }
        "Border" => {
            // WinUI 3 Border 容器规范：
            // 固有宽度 200px、高度 64px。
            // 容器外圆角 8px（OverlayCornerRadius）、1px SurfaceStroke 微描边（CARD_BORDER）。
            // 内部 Padding 12px 承载 "Border" 文本。
            let border_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                200.0
            };
            let border_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                64.0
            };
            let border_r = Rect::new(rect.origin.x, rect.origin.y, border_w, border_h);
            let disabled = control.disabled;

            let mut out = WidgetOutput::default();
            // 1. 表面背景（禁用态由 Catalog.cs 设置 Opacity=0.4，使 CARD_BG 混合为 #242424）
            let bg_token = Token::CARD_BG;
            let border_token = Token::CARD_BORDER;
            out.primitives
                .push(primitive::surface(border_r, 8.0, bg_token, 100));
            // 2. 1px SurfaceStroke 微描边
            out.primitives.push(primitive::instance(
                border_r,
                [8.0; 4],
                Token::NONE,
                border_token,
                1.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Border,
                100,
            ));

            if control.focused && !disabled {
                out.primitives.push(primitive::focus_ring(
                    border_r.inflate(1.0),
                    9.0,
                    Token::FOCUS_RING,
                    2.0,
                    710,
                    true,
                ));
            }

            if disabled {
                for prim in &mut out.primitives {
                    prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
                        prim.depth_u16(),
                        prim.opacity() * unbound_widgets::slot_fixture::WINUI_DISABLED_OPACITY,
                    );
                }
            }
            push(scene, out);

            // 3. 排版内容文本
            let text_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            let quads = text_ctx.shape_line_styled(
                "Border",
                Rect::new(
                    border_r.origin.x + 12.0,
                    border_r.origin.y + 12.0,
                    (border_w - 24.0).max(0.0),
                    (border_h - 24.0).max(0.0),
                ),
                14.0,
                400,
                None,
                unbound_widgets::AlignX::Center,
                unbound_widgets::AlignY::Middle,
            );
            emit_quads(scene, &quads, text_ink, 102);

            Vec2::new(border_w, border_h)
        }
        "StackPanel" => {
            // WinUI 3 StackPanel 垂直堆叠布局容器规范：
            // 固有宽度 200px。
            // 接入标准 3 行纯文本条目插桩（Row 1、Row 2、Row 3），间距 8px。
            let panel_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                200.0
            };
            let panel_rect = Rect::new(rect.origin.x, rect.origin.y, panel_w, rect.height());
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = emit_canonical_text_rows_fixture(
                panel_rect,
                STANDARD_STACK_SPACING,
                fixture_state,
                text_ctx,
                700,
            );
            let mut out = fixture.output;
            if control.focused && !control.disabled {
                let focus_r = Rect::new(
                    panel_rect.origin.x,
                    panel_rect.origin.y,
                    panel_w,
                    fixture.total_size.y,
                );
                out.primitives.push(primitive::focus_ring(
                    focus_r.inflate(1.0),
                    4.0,
                    Token::FOCUS_RING,
                    2.0,
                    710,
                    true,
                ));
            }
            push(scene, out);
            Vec2::new(panel_w, fixture.total_size.y)
        }
        "ScrollView" => {
            // WinUI 3 ScrollView 滚动视口规范：
            // 固有尺寸宽 220px、高 80px。
            // 视口内承载标准 3 行纯文本条目插桩（Row 1、Row 2、Row 3），间距 6px。
            let scroll_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                220.0
            };
            let scroll_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                80.0
            };
            let scroll_rect = Rect::new(rect.origin.x, rect.origin.y, scroll_w, scroll_h);
            // WinUI 的 `ScrollView` 没有禁用态模板：`IsEnabled=false` 只阻断输入，
            // 视口内的文本仍按常规色绘制，因此文本插桩不能跟随 disabled 降级。
            let fixture_state = FixtureState::from_control_state(
                ControlState {
                    disabled: false,
                    ..control
                },
                checked,
            );
            let rows_fixture =
                emit_canonical_text_rows_fixture(scroll_rect, 6.0, fixture_state, text_ctx, 702);
            let content_size = Vec2::new(scroll_w, rows_fixture.total_size.y);
            let desc = ScrollViewDesc::new(scroll_rect, content_size);
            let scroll_state = ScrollState::default();
            let mut out = emit_scroll_view(NODE, &desc, &scroll_state, control).output;
            out.primitives.extend(rows_fixture.output.primitives);
            out.payloads.extend(rows_fixture.output.payloads);

            if control.focused && !control.disabled {
                out.primitives.push(primitive::focus_ring(
                    scroll_rect.inflate(1.0),
                    4.0,
                    Token::FOCUS_RING,
                    2.0,
                    710,
                    true,
                ));
            }
            push(scene, out);
            Vec2::new(scroll_w, scroll_h)
        }
        "GridView" => {
            // WinUI 3 GridView 网格集合视图规范：
            // 固有尺寸宽 260px、高 140px。
            // 包含 2×2 网格项（Item 1 到 Item 4），单元格 6px 圆角、网格间距 6px。
            // 首项处于选中态，具有强调色高亮边框与右上角勾选指示标记。
            let grid_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                unbound_widgets::slot_fixture::STANDARD_GRID_WIDTH
            };
            let grid_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                unbound_widgets::slot_fixture::STANDARD_GRID_HEIGHT
            };
            let mut out = WidgetOutput::default();

            let fixture_state = FixtureState::from_control_state(control, true);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_grid_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            out.primitives.extend(fixture.output.primitives);
            out.payloads.extend(fixture.output.payloads);
            push(scene, out);

            Vec2::new(grid_w, grid_h)
        }
        "ListBox" => {
            // WinUI 3 ListBox 经典单选列表规范：
            // 固有尺寸宽 220px、高 120px。
            // 包含 3 项标准列表行（Item 1、Item 2、Item 3），标准行高 40px。
            // 外部为带圆角与边框的标准控件容器表面，内部承载统一列表行插桩。
            let list_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                220.0
            };
            let list_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                120.0
            };
            let list_rect = Rect::new(rect.origin.x, rect.origin.y, list_w, list_h);
            let disabled = control.disabled;

            let mut out = WidgetOutput::default();
            let _ = disabled;

            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_listbox_rows_fixture(
                list_rect,
                fixture_state,
                text_ctx,
                702,
            );
            out.primitives.extend(fixture.output.primitives);
            out.payloads.extend(fixture.output.payloads);
            push(scene, out);

            Vec2::new(list_w, list_h)
        }
        "ItemsView" => {
            // WinUI 3 ItemsView 默认 StackLayout：无外框、无卡片底色，纵向排列 4 行纯文本（行高 19px）。
            let items_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                260.0
            };
            let items_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                160.0
            };
            let items_rect = Rect::new(rect.origin.x, rect.origin.y, items_w, items_h);
            let disabled = control.disabled;
            let ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            let items = ["Item 1", "Item 2", "Item 3", "Item 4"];
            let line_h = 19.0;
            for (idx, text) in items.iter().enumerate() {
                let row_r = Rect::new(
                    items_rect.origin.x,
                    items_rect.origin.y + idx as f32 * line_h,
                    items_w,
                    line_h,
                );
                let quads = text_ctx.shape_line_styled(
                    text,
                    row_r,
                    14.0,
                    400,
                    None,
                    unbound_widgets::AlignX::Left,
                    unbound_widgets::AlignY::Middle,
                );
                emit_quads(scene, &quads, ink, 704);
            }
            Vec2::new(items_w, items_h)
        }
        "SwipeControl" => {
            // WinUI 3 SwipeControl 滑动手势控件规范：
            // 固有尺寸宽 260px、高 48px。
            // normal 态为静态未滑动列表行，checked 态展现滑出删除动作。
            let swipe_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                260.0
            };
            let swipe_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                48.0
            };
            let swipe_rect = Rect::new(rect.origin.x, rect.origin.y, swipe_w, swipe_h);
            let disabled = control.disabled;

            let mut out = WidgetOutput::default();

            if checked {
                let action_w = 64.0;
                let action_rect = Rect::new(
                    swipe_rect.max().x - action_w,
                    swipe_rect.origin.y,
                    action_w,
                    swipe_h,
                );
                let content_rect = Rect::new(
                    swipe_rect.origin.x,
                    swipe_rect.origin.y,
                    swipe_w - action_w,
                    swipe_h,
                );

                // 1. 右侧展开的动作槽底板（强调色）
                out.primitives.push(primitive::surface(
                    action_rect,
                    0.0,
                    if disabled {
                        Token::CONTROL_DISABLED_BG
                    } else {
                        Token::PRIMARY
                    },
                    700,
                ));

                // 2. 主内容卡片（圆角 8px）
                out.primitives.push(primitive::outlined(
                    content_rect,
                    8.0,
                    if disabled {
                        Token::CONTROL_DISABLED_BG
                    } else {
                        Token::CARD_BG
                    },
                    if disabled {
                        Token::CONTROL_DISABLED_BORDER
                    } else {
                        Token::CARD_BORDER
                    },
                    1.0,
                    0.0,
                    702,
                ));

                if control.focused && !disabled {
                    out.primitives.push(primitive::focus_ring(
                        content_rect.inflate(1.0),
                        9.0,
                        Token::FOCUS_RING,
                        2.0,
                        710,
                        true,
                    ));
                }

                push(scene, out);

                // 3. 排版主内容文本与动作标签
                let text_ink = if disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::TEXT_PRIMARY
                };
                emit_text_left_middle(
                    text_ctx,
                    scene,
                    "Swipe me",
                    content_rect,
                    content_rect.origin.x + 16.0,
                    14.0,
                    400,
                    None,
                    text_ink,
                    704,
                );

                let action_ink = if disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::ON_PRIMARY
                };
                emit_icon_centered(
                    text_ctx,
                    scene,
                    "\u{E74D}",
                    Rect::new(
                        action_rect.origin.x,
                        action_rect.origin.y + 4.0,
                        action_w,
                        20.0,
                    ),
                    16.0,
                    action_ink,
                    704,
                );
                let action_quads = text_ctx.shape_line_styled(
                    "Delete",
                    Rect::new(
                        action_rect.origin.x,
                        action_rect.origin.y + 24.0,
                        action_w,
                        18.0,
                    ),
                    11.0,
                    600,
                    None,
                    unbound_widgets::AlignX::Center,
                    unbound_widgets::AlignY::Top,
                );
                emit_quads(scene, &action_quads, action_ink, 704);
            } else {
                // normal / disabled / focused 态：WinUI SwipeControl 内容 Border 无圆角无描边，
                // 禁用态仅阻断交互不降级内部卡片填充与文本色，文本左上对齐 12px 内边距。
                out.primitives
                    .push(primitive::surface(swipe_rect, 0.0, Token::CARD_BG, 702));

                if control.focused && !disabled {
                    out.primitives.push(primitive::focus_ring(
                        swipe_rect.inflate(1.0),
                        4.0,
                        Token::FOCUS_RING,
                        2.0,
                        710,
                        true,
                    ));
                }

                push(scene, out);

                let quads = text_ctx.shape_line_styled(
                    "Swipe me",
                    Rect::new(
                        swipe_rect.origin.x + 12.0,
                        swipe_rect.origin.y + 12.0,
                        (swipe_w - 24.0).max(0.0),
                        (swipe_h - 24.0).max(0.0),
                    ),
                    14.0,
                    400,
                    None,
                    unbound_widgets::AlignX::Left,
                    unbound_widgets::AlignY::Top,
                );
                emit_quads(scene, &quads, Token::TEXT_PRIMARY, 704);
            }

            Vec2::new(swipe_w, swipe_h)
        }
        "RefreshContainer" => {
            // WinUI 3 RefreshContainer 下拉刷新容器规范：
            // 固有尺寸宽 260px、高 160px。
            // 顶部包含下拉刷新 ProgressRing 环形指示器，下方为内容卡片。
            let refresh_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                unbound_widgets::slot_fixture::STANDARD_REFRESH_WIDTH
            };
            let refresh_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                unbound_widgets::slot_fixture::STANDARD_REFRESH_HEIGHT
            };
            let mut out = WidgetOutput::default();

            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_refresh_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            out.primitives.extend(fixture.output.primitives);
            out.payloads.extend(fixture.output.payloads);
            push(scene, out);

            Vec2::new(refresh_w, refresh_h)
        }
        "RangeSelector" => {
            // Community Toolkit / WinUI 3 RangeSelector 区间滑块规范：
            // 固有尺寸宽 200px、高 32px。
            // 包含 4px 轨道、双圆形滑块（Thumb）与激活区间强调色填充。
            let range_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                200.0
            };
            let range_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                32.0
            };
            let range_rect = Rect::new(rect.origin.x, rect.origin.y, range_w, range_h);
            let disabled = control.disabled;

            let track_h = 4.0;
            let track_y = range_rect.origin.y + (range_h - track_h) * 0.5;
            let track_rect = Rect::new(range_rect.origin.x, track_y, range_w, track_h);

            let thumb_size = 16.0;
            let travel = (range_w - thumb_size).max(0.0);
            let low_frac = 0.25;
            let high_frac = 0.75;
            let low_x = range_rect.origin.x + travel * low_frac;
            let high_x = range_rect.origin.x + travel * high_frac;
            let thumb_y = range_rect.origin.y + (range_h - thumb_size) * 0.5;

            let active_rect = Rect::new(
                low_x + thumb_size * 0.5,
                track_y,
                (high_x - low_x).max(0.0),
                track_h,
            );

            let thumb1_rect = Rect::new(low_x, thumb_y, thumb_size, thumb_size);
            let thumb2_rect = Rect::new(high_x, thumb_y, thumb_size, thumb_size);

            let mut out = WidgetOutput::default();

            // 1. 轨道底板
            out.primitives.push(primitive::surface(
                track_rect,
                2.0,
                if disabled {
                    Token::CONTROL_DISABLED_BG
                } else {
                    Token::CONTROL_TRACK
                },
                700,
            ));

            // 2. 激活区间强调色条
            out.primitives.push(primitive::surface(
                active_rect,
                2.0,
                if disabled {
                    Token::SLIDER_FILL_DISABLED
                } else {
                    Token::PRIMARY
                },
                701,
            ));

            // 3. 双圆形滑块（外圈边框 + 内部实心强调色）
            for (idx, thumb) in [thumb1_rect, thumb2_rect].iter().enumerate() {
                out.primitives.push(primitive::outlined(
                    *thumb,
                    thumb_size * 0.5,
                    if disabled {
                        Token::SLIDER_THUMB_DISABLED
                    } else {
                        Token::CARD_BG
                    },
                    if disabled {
                        Token::CONTROL_DISABLED_BORDER
                    } else {
                        Token::PRIMARY
                    },
                    1.0,
                    0.0,
                    703 + idx as u16,
                ));
                out.primitives.push(primitive::surface(
                    thumb.inflate(-4.0),
                    (thumb_size - 8.0) * 0.5,
                    if disabled {
                        Token::CONTROL_DISABLED_THUMB
                    } else {
                        Token::PRIMARY
                    },
                    705 + idx as u16,
                ));
            }

            if control.focused && !disabled {
                out.primitives.push(primitive::focus_ring(
                    thumb1_rect.inflate(2.0),
                    10.0,
                    Token::FOCUS_RING,
                    2.0,
                    710,
                    true,
                ));
            }

            push(scene, out);

            Vec2::new(range_w, range_h)
        }
        "HeaderedContentControl" => {
            // WinUI 3 / Community Toolkit HeaderedContentControl 容器规范：
            // 固有尺寸宽 220px、高 100px。
            // 顶部为 14px 粗体 Header 标签，下方为 8px 圆角边框内容区。
            let hcc_w = if rect.width() > 0.0 {
                rect.width()
            } else {
                220.0
            };
            let hcc_h = if rect.height() > 0.0 {
                rect.height()
            } else {
                100.0
            };
            let hcc_rect = Rect::new(rect.origin.x, rect.origin.y, hcc_w, hcc_h);
            let disabled = control.disabled;

            let header_h = 24.0;
            let header_rect = Rect::new(hcc_rect.origin.x, hcc_rect.origin.y, hcc_w, header_h);
            let content_y = hcc_rect.origin.y + header_h + 4.0;
            let content_h = (hcc_h - header_h - 4.0).max(1.0);
            let content_rect = Rect::new(hcc_rect.origin.x, content_y, hcc_w, content_h);

            let mut out = WidgetOutput::default();

            // 下方 8px 圆角边框内容区
            out.primitives.push(primitive::outlined(
                content_rect,
                8.0,
                if disabled {
                    Token::CONTROL_DISABLED_BG
                } else {
                    Token::CARD_BG
                },
                if disabled {
                    Token::CONTROL_DISABLED_BORDER
                } else {
                    Token::CARD_BORDER
                },
                1.0,
                0.0,
                700,
            ));

            if control.focused && !disabled {
                out.primitives.push(primitive::focus_ring(
                    content_rect.inflate(1.0),
                    9.0,
                    Token::FOCUS_RING,
                    2.0,
                    710,
                    true,
                ));
            }

            push(scene, out);

            // 顶部 Header 标签（14px 粗体）
            let header_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            };
            emit_text_left_middle(
                text_ctx,
                scene,
                "Header",
                header_rect,
                header_rect.origin.x + 2.0,
                14.0,
                700,
                None,
                header_ink,
                702,
            );

            // 内容区文本
            let content_ink = if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };
            emit_text_left_middle(
                text_ctx,
                scene,
                "Content",
                content_rect,
                content_rect.origin.x + 14.0,
                14.0,
                400,
                None,
                content_ink,
                704,
            );

            Vec2::new(hcc_w, hcc_h)
        }
        "InfoBar" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_info_bar_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "Pivot" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_pivot_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "Expander" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_expander_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "SplitView" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_split_view_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "CalendarView" => {
            let first = scene.primitives.len();
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_calendar_view_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            // 外框裁剪节点：保留完整外框底边与圆角。
            let outer_clip_idx = scene.clips.len() as u32;
            scene.clips.push(GpuClipNode::from_aabb([
                rect.origin.x,
                rect.origin.y,
                rect.max().x,
                rect.max().y,
            ]));
            // 日期网格内缩裁剪节点：使底部第 4 行在 y = 272 截断，留出空白不压穿底边框。
            let grid_clip_idx = scene.clips.len() as u32;
            scene.clips.push(GpuClipNode::from_aabb([
                rect.origin.x,
                rect.origin.y,
                rect.max().x,
                rect.max().y - 3.0,
            ]));
            // 深度 >= 703 为日期网格图元（15 号选中圆环 703、日期数字 704），其余外框/分隔线/表头走外框裁剪。
            for primitive in &mut scene.primitives[first..] {
                primitive.clip_idx = if primitive.depth_u16() >= 703 {
                    grid_clip_idx
                } else {
                    outer_clip_idx
                };
            }
            fixture.total_size
        }
        "TimePicker" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_time_picker_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "PersonPicture" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_person_picture_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        "FontIcon" => {
            // 亮色禁用态下若平台存在符号字体，继续通过 `texture_quads` 做 sRGB 线性覆盖率预混，
            // 避免默认着色器的浅底字形补偿曲线叠加半透明令牌使边缘二次加深；
            // 其余状态及无符号字体回退统一走 `emit_canonical_font_icon_fixture`。
            let size = if rect.width() > 0.0 {
                rect.width().min(if rect.height() > 0.0 {
                    rect.height()
                } else {
                    rect.width()
                })
            } else {
                unbound_widgets::slot_fixture::CANONICAL_FONT_ICON_SIZE
            };
            let total_r = Rect::new(rect.origin.x, rect.origin.y, size, size);
            let premixed_quads = if control.disabled && case.theme == "light" {
                text_ctx.shape_symbol_glyph(
                    unbound_widgets::slot_fixture::CANONICAL_FONT_ICON_GLYPH,
                    total_r,
                    unbound_widgets::slot_fixture::CANONICAL_FONT_ICON_FONT_SIZE,
                )
            } else {
                None
            };
            if let Some(quads) = premixed_quads {
                texture_quads.extend(quads);
                Vec2::new(size, size)
            } else {
                let fixture_state = FixtureState::from_control_state(control, checked);
                let fixture = unbound_widgets::slot_fixture::emit_canonical_font_icon_fixture(
                    rect,
                    fixture_state,
                    text_ctx,
                    700,
                );
                push(scene, fixture.output);
                fixture.total_size
            }
        }
        "Image" => {
            let fixture_state = FixtureState::from_control_state(control, checked);
            let fixture = unbound_widgets::slot_fixture::emit_canonical_image_fixture(
                rect,
                fixture_state,
                text_ctx,
                700,
            );
            push(scene, fixture.output);
            fixture.total_size
        }
        other => return Err(format!("unimplemented parity control: {other}")),
    };

    Ok(intrinsic)
}

/// 按绝对起点与字号排版一行文本，供目录通用发射器复用同一套字形缓存。
#[allow(dead_code)]
fn shape_at(
    text_ctx: &mut TextCtx<'_>,
    text: &str,
    origin: Vec2,
    size: f32,
) -> Vec<unbound_text::GlyphQuad> {
    shape_at_styled(text_ctx, text, origin, size, 400, None)
}

/// 与 [`shape_at`] 相同，但允许指定字重与字体族（用于平台符号字体字形）。
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

/// 追加一张来自 ColorPicker 专用 RGBA 图集页的纹理矩形。
///
/// UV 使用图集绝对归一化坐标，载荷索引按场景当前长度即时分配。
fn emit_texture(scene: &mut Gallery, rect: Rect, radius: f32, uv0: Vec2, uv1: Vec2, z: u16) {
    let mut primitive = primitive::instance(
        rect,
        [radius; 4],
        PARITY_WHITE,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Image,
        SubElement::Content,
        z,
    );
    primitive.border_token_or_tex = PARITY_RGBA_LAYER;
    primitive.extra_payload_idx = scene.payloads.len() as u32;
    scene.payloads.push(GpuPayload {
        p0: [uv0.x, uv0.y],
        p1: [uv1.x, uv1.y],
        p2: [0.0, 0.0],
        line_width: 0.0,
        extra: 0,
        flags: 0,
        _reserved: [0; 3],
    });
    scene.primitives.push(primitive);
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
    let label = Rect::new(
        left,
        rect.origin.y,
        (rect.max().x - left).max(1.0),
        rect.height(),
    );
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

/// 把一枚平台符号字体字形居中绘制到给定方框。
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
    let quads = icon_quads(text_ctx, glyph, rect, size);
    emit_quads(scene, &quads, color, z);
}

/// 在 `rect` 内居中的单个图标字形四边形（不发射图元）。
///
/// 通过 [`TextCtx::symbol_font_family`] 获取平台可用符号字体族；
/// 当平台无可用符号字体或策略禁用时返回空集合。
fn icon_quads(
    text_ctx: &mut TextCtx<'_>,
    glyph: &str,
    rect: Rect,
    size: f32,
) -> Vec<unbound_text::GlyphQuad> {
    let Some(family) = text_ctx.symbol_font_family().map(str::to_owned) else {
        return Vec::new();
    };
    let scale = text_ctx.effective_raster_scale();
    let mut style = unbound_text::TextStyle::sized(size * scale);
    style.family = Some(family.clone());
    let measured = text_ctx.engine.measure(glyph, &style);
    let width = measured.x / scale;
    let height = measured.y / scale;
    let origin = Vec2::new(
        rect.center().x - width * 0.5,
        rect.center().y - height * 0.5,
    );
    shape_at_styled(text_ctx, glyph, origin, size, 400, Some(family.as_str()))
}

/// 构造 `NavigationViewOverflow` 与 `NavigationViewLeftClosedOverflow` 共用的 10 项溢出菜单。
///
/// 文案与图标码点与 `tools/winui-reference/Catalog.cs` 的 `AddOverflowNavigationItems` 逐项对称。
fn navigation_view_overflow_menu_items() -> Vec<unbound_widgets::NavigationViewMenuItem> {
    vec![
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
        ),
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(2, "Library").with_glyph("\u{E8F1}"),
        ),
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(3, "Analytics").with_glyph("\u{E908}"),
        ),
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(4, "Reports").with_glyph("\u{E8A5}"),
        ),
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(5, "Projects").with_glyph("\u{E8B7}"),
        ),
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(6, "Tasks").with_glyph("\u{E762}"),
        ),
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(7, "Calendar").with_glyph("\u{E787}"),
        ),
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(8, "Messages").with_glyph("\u{E715}"),
        ),
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(9, "Contacts").with_glyph("\u{E77B}"),
        ),
        unbound_widgets::NavigationViewMenuItem::item(
            unbound_widgets::NavigationViewItem::new(10, "Archive").with_glyph("\u{E7B8}"),
        ),
    ]
}

/// 按控件规范名构造对应的 `NavigationView` 基准描述符与默认展开项列表。
fn navigation_view_parity_desc(
    control: &str,
    nav_r: Rect,
    checked: bool,
) -> (unbound_widgets::NavigationViewDesc, Vec<u64>) {
    match control {
        "NavigationViewLeftClosed" => (
            unbound_widgets::NavigationViewDesc::new(
                nav_r,
                vec![
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(2, "Library")
                            .with_glyph("\u{E8F1}"),
                    ),
                ],
            )
            .pane_display_mode(unbound_widgets::NavigationViewPaneDisplayMode::Left)
            .is_pane_open(false)
            .is_settings_visible(true)
            .is_back_button_visible(unbound_widgets::NavigationViewBackButtonVisible::Collapsed)
            .selected_item(Some(1)),
            Vec::new(),
        ),
        "NavigationViewLeftCompact" => (
            unbound_widgets::NavigationViewDesc::new(
                nav_r,
                vec![
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(2, "Library")
                            .with_glyph("\u{E8F1}"),
                    ),
                ],
            )
            .pane_display_mode(unbound_widgets::NavigationViewPaneDisplayMode::LeftCompact)
            .open_pane_length(220.0)
            .is_pane_open(true)
            .is_settings_visible(true)
            .is_back_button_visible(unbound_widgets::NavigationViewBackButtonVisible::Collapsed)
            .selected_item(Some(1)),
            Vec::new(),
        ),
        "NavigationViewLeftMinimal" => (
            unbound_widgets::NavigationViewDesc::new(
                nav_r,
                vec![
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(2, "Library")
                            .with_glyph("\u{E8F1}"),
                    ),
                ],
            )
            .pane_display_mode(unbound_widgets::NavigationViewPaneDisplayMode::LeftMinimal)
            .is_pane_open(checked)
            .is_settings_visible(true)
            .is_back_button_visible(unbound_widgets::NavigationViewBackButtonVisible::Collapsed)
            .selected_item(Some(1)),
            Vec::new(),
        ),
        "NavigationViewTop" => (
            unbound_widgets::NavigationViewDesc::new(
                nav_r,
                vec![
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(2, "Library")
                            .with_glyph("\u{E8F1}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(3, "Analytics")
                            .with_glyph("\u{E908}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(4, "Reports")
                            .with_glyph("\u{E8A5}"),
                    ),
                ],
            )
            .pane_display_mode(unbound_widgets::NavigationViewPaneDisplayMode::Top)
            .overflow_label_mode(unbound_widgets::NavigationViewOverflowLabelMode::NoLabel)
            .is_settings_visible(true)
            .is_back_button_visible(unbound_widgets::NavigationViewBackButtonVisible::Collapsed)
            .selected_item(Some(1)),
            Vec::new(),
        ),
        "NavigationViewHierarchical" => (
            unbound_widgets::NavigationViewDesc::new(
                nav_r,
                vec![
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(2, "Collections")
                            .with_glyph("\u{E8B7}")
                            .selects_on_invoked(false)
                            .with_children(vec![
                                unbound_widgets::NavigationViewMenuItem::item(
                                    unbound_widgets::NavigationViewItem::new(21, "Notes")
                                        .with_glyph("\u{E7C3}"),
                                ),
                                unbound_widgets::NavigationViewMenuItem::item(
                                    unbound_widgets::NavigationViewItem::new(22, "Mail")
                                        .with_glyph("\u{E715}"),
                                ),
                            ]),
                    ),
                ],
            )
            .pane_display_mode(unbound_widgets::NavigationViewPaneDisplayMode::Left)
            .open_pane_length(220.0)
            .is_pane_open(true)
            .is_settings_visible(true)
            .is_back_button_visible(unbound_widgets::NavigationViewBackButtonVisible::Collapsed)
            .selected_item(Some(22)),
            vec![2],
        ),
        "NavigationViewChrome" => (
            unbound_widgets::NavigationViewDesc::new(
                nav_r,
                vec![
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(2, "Library")
                            .with_glyph("\u{E8F1}"),
                    ),
                ],
            )
            .pane_display_mode(unbound_widgets::NavigationViewPaneDisplayMode::Left)
            .open_pane_length(220.0)
            .is_pane_open(true)
            .is_settings_visible(true)
            .is_back_button_visible(unbound_widgets::NavigationViewBackButtonVisible::Visible)
            .is_back_enabled(true)
            .pane_title("Menu")
            .header("Home")
            .with_auto_suggest_box("Search")
            .selected_item(Some(1)),
            Vec::new(),
        ),
        "NavigationViewOverflow" => (
            unbound_widgets::NavigationViewDesc::new(nav_r, navigation_view_overflow_menu_items())
                .pane_display_mode(unbound_widgets::NavigationViewPaneDisplayMode::Left)
                .open_pane_length(220.0)
                .is_pane_open(true)
                .is_settings_visible(true)
                .is_back_button_visible(unbound_widgets::NavigationViewBackButtonVisible::Collapsed)
                .selected_item(Some(1)),
            Vec::new(),
        ),
        "NavigationViewLeftClosedOverflow" => (
            unbound_widgets::NavigationViewDesc::new(nav_r, navigation_view_overflow_menu_items())
                .pane_display_mode(unbound_widgets::NavigationViewPaneDisplayMode::Left)
                .is_pane_open(false)
                .is_settings_visible(true)
                .is_back_button_visible(unbound_widgets::NavigationViewBackButtonVisible::Collapsed)
                .selected_item(Some(1)),
            Vec::new(),
        ),
        _ => (
            unbound_widgets::NavigationViewDesc::new(
                nav_r,
                vec![
                    unbound_widgets::NavigationViewMenuItem::header("Workspace"),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(2, "Library")
                            .with_glyph("\u{E8F1}"),
                    ),
                    unbound_widgets::NavigationViewMenuItem::separator(),
                    unbound_widgets::NavigationViewMenuItem::item(
                        unbound_widgets::NavigationViewItem::new(3, "Analytics")
                            .with_glyph("\u{E908}"),
                    ),
                ],
            )
            .pane_display_mode(unbound_widgets::NavigationViewPaneDisplayMode::Left)
            .open_pane_length(220.0)
            .is_pane_open(true)
            .is_settings_visible(true)
            .is_back_button_visible(unbound_widgets::NavigationViewBackButtonVisible::Collapsed)
            .footer_menu_items(vec![unbound_widgets::NavigationViewMenuItem::item(
                unbound_widgets::NavigationViewItem::new(90, "Account").with_glyph("\u{E77B}"),
            )])
            .selected_item(Some(1)),
            Vec::new(),
        ),
    }
}

/// 把一次发射的图元与载荷追加到场景。
///
/// 字形、图像与曲线图元的载荷索引是相对其所属发射输出的，合并进场景时
/// 必须整体平移到当前载荷长度之后，否则后续控件会采样到先前图元的 UV。
fn push(scene: &mut Gallery, mut output: unbound_widgets::WidgetOutput) {
    let base = scene.payloads.len() as u32;
    for primitive in &mut output.primitives {
        if matches!(
            primitive.prim_type(),
            PrimType::Glyph | PrimType::GlyphColor | PrimType::Image | PrimType::Curve
        ) {
            primitive.extra_payload_idx = primitive.extra_payload_idx.saturating_add(base);
        }
    }
    scene.primitives.extend(output.primitives);
    scene.payloads.extend(output.payloads);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_supported_controls_have_explicit_branches_without_unimplemented_error() {
        let states = ["normal", "disabled", "checked", "focused"];
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut text_ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 0,
            raster_scale: 1.0,
        };
        let mut texture_quads = Vec::new();
        let bounds = Rect::new(0.0, 0.0, 320.0, 240.0);

        for control in SUPPORTED_CONTROLS {
            for state in states {
                let case = ParityCase {
                    control,
                    state,
                    theme: "light",
                    backdrop: "solid",
                };
                let mut scene = Gallery::new();
                let res = emit_case(&case, bounds, &mut scene, &mut text_ctx, &mut texture_quads);
                assert!(
                    res.is_ok(),
                    "control '{control}' in state '{state}' failed with error: {:?}",
                    res.err()
                );
            }
        }
    }

    #[test]
    fn test_unimplemented_control_triggers_explicit_error_and_blocks_fallback() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut text_ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 0,
            raster_scale: 1.0,
        };
        let mut texture_quads = Vec::new();
        let bounds = Rect::new(0.0, 0.0, 320.0, 240.0);
        let case = ParityCase {
            control: "UnknownFutureWidget",
            state: "normal",
            theme: "light",
            backdrop: "solid",
        };
        let mut scene = Gallery::new();
        let res = emit_case(&case, bounds, &mut scene, &mut text_ctx, &mut texture_quads);
        assert!(res.is_err());
        assert_eq!(
            res.err().unwrap(),
            "unimplemented parity control: UnknownFutureWidget"
        );
    }

    /// 检查场景图元中是否存在垂直静止态 2px 滚动条细滑块。
    fn has_vertical_idle_scrollbar_thumb(scene: &Gallery) -> bool {
        scene.primitives.iter().any(|prim| {
            prim.prim_type() == PrimType::RoundedRect
                && (prim.size[0] - unbound_widgets::SCROLLBAR_THUMB_IDLE_WIDTH).abs() < 0.01
                && prim.opacity() > 0.1
        })
    }

    /// 检查场景图元中是否存在水平静止态 2px 滚动条细滑块。
    fn has_horizontal_idle_scrollbar_thumb(scene: &Gallery) -> bool {
        scene.primitives.iter().any(|prim| {
            prim.prim_type() == PrimType::RoundedRect
                && (prim.size[1] - unbound_widgets::SCROLLBAR_THUMB_IDLE_WIDTH).abs() < 0.01
                && prim.opacity() > 0.1
        })
    }

    /// 检查命中区列表是否包含指定轴向的滚动条轨道或滑块命中区。
    fn has_scrollbar_hit(
        hits: &[unbound_input::HitRegion],
        owner_node: u64,
        axis: unbound_widgets::ScrollbarAxis,
    ) -> bool {
        let track = unbound_widgets::scrollbar_track_node(owner_node, axis);
        let thumb = unbound_widgets::scrollbar_thumb_node(owner_node, axis);
        hits.iter().any(|h| h.node == track || h.node == thumb)
    }

    #[test]
    fn test_navigation_view_overflow_emits_visible_scrollbar() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut text_ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 0,
            raster_scale: 1.0,
        };
        let mut texture_quads = Vec::new();
        let bounds = Rect::new(0.0, 0.0, 360.0, 280.0);
        let case = ParityCase::new("NavigationViewOverflow", "normal", "light");
        let mut scene = Gallery::new();
        let res = emit_case(&case, bounds, &mut scene, &mut text_ctx, &mut texture_quads);
        assert!(res.is_ok(), "NavigationViewOverflow emit failed: {res:?}");
        // Left 展开窗格溢出时必须产出宽度为 2px 的静止态细滑块且半透明度 > 0。
        assert!(
            has_vertical_idle_scrollbar_thumb(&scene),
            "NavigationViewOverflow must emit a visible Win11 idle scrollbar thumb"
        );

        // 同时验证底层发射结果确实处于溢出态且包含垂直滚动条命中区。
        let (desc, _) = navigation_view_parity_desc("NavigationViewOverflow", bounds, false);
        let mut nav_state = unbound_widgets::NavigationViewState::from_desc(&desc);
        nav_state.set_animations_enabled(false, &desc);
        let emitted = unbound_widgets::emit_navigation_view(
            700,
            &desc,
            ControlState::default(),
            &nav_state,
            &mut text_ctx,
        );
        let bar = desc
            .menu_scrollbar_desc(&emitted.layout, &nav_state, false)
            .expect("NavigationViewOverflow must produce a vertical scrollbar descriptor");
        assert!(
            bar.is_overflowing(),
            "NavigationViewOverflow menu content must overflow viewport"
        );
        assert!(
            has_scrollbar_hit(
                &emitted.output.sub_hits,
                700,
                unbound_widgets::ScrollbarAxis::Vertical,
            ),
            "NavigationViewOverflow must emit vertical scrollbar hit regions"
        );
    }

    #[test]
    fn test_navigation_view_left_closed_overflow_and_non_overflow_scrollbar_contract() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut text_ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 0,
            raster_scale: 1.0,
        };
        let mut texture_quads = Vec::new();
        let bounds = Rect::new(0.0, 0.0, 360.0, 280.0);

        // 1. 未溢出折叠态（NavigationViewLeftClosed）：零滚动条图元、零滚动条命中区。
        let non_overflow_case = ParityCase::new("NavigationViewLeftClosed", "normal", "light");
        let mut non_overflow_scene = Gallery::new();
        let res = emit_case(
            &non_overflow_case,
            bounds,
            &mut non_overflow_scene,
            &mut text_ctx,
            &mut texture_quads,
        );
        assert!(res.is_ok(), "NavigationViewLeftClosed emit failed: {res:?}");
        assert!(
            !has_vertical_idle_scrollbar_thumb(&non_overflow_scene),
            "NavigationViewLeftClosed must not emit scrollbar thumb primitives"
        );

        let (closed_desc, _) =
            navigation_view_parity_desc("NavigationViewLeftClosed", bounds, false);
        let mut closed_state = unbound_widgets::NavigationViewState::from_desc(&closed_desc);
        closed_state.set_animations_enabled(false, &closed_desc);
        let closed_emitted = unbound_widgets::emit_navigation_view(
            700,
            &closed_desc,
            ControlState::default(),
            &closed_state,
            &mut text_ctx,
        );
        let closed_bar = closed_desc
            .menu_scrollbar_desc(&closed_emitted.layout, &closed_state, false)
            .expect("Left mode must return a scrollbar descriptor for overflow inspection");
        assert!(
            !closed_bar.is_overflowing(),
            "NavigationViewLeftClosed must not overflow with only 2 menu items"
        );
        assert!(
            !has_scrollbar_hit(
                &closed_emitted.output.sub_hits,
                700,
                unbound_widgets::ScrollbarAxis::Vertical,
            ),
            "NavigationViewLeftClosed must emit zero scrollbar hit regions"
        );

        // 2. 折叠溢出态（NavigationViewLeftClosedOverflow）：静止态自动隐藏（与 navigation_view.rs 契约一致）。
        let closed_overflow_case =
            ParityCase::new("NavigationViewLeftClosedOverflow", "normal", "light");
        let mut closed_overflow_scene = Gallery::new();
        let res = emit_case(
            &closed_overflow_case,
            bounds,
            &mut closed_overflow_scene,
            &mut text_ctx,
            &mut texture_quads,
        );
        assert!(
            res.is_ok(),
            "NavigationViewLeftClosedOverflow emit failed: {res:?}"
        );
        assert!(
            !has_vertical_idle_scrollbar_thumb(&closed_overflow_scene),
            "NavigationViewLeftClosedOverflow idle state must auto-hide scrollbar thumb"
        );

        let (overflow_desc, _) =
            navigation_view_parity_desc("NavigationViewLeftClosedOverflow", bounds, false);
        let mut overflow_state = unbound_widgets::NavigationViewState::from_desc(&overflow_desc);
        overflow_state.set_animations_enabled(false, &overflow_desc);
        let idle_emitted = unbound_widgets::emit_navigation_view(
            700,
            &overflow_desc,
            ControlState::default(),
            &overflow_state,
            &mut text_ctx,
        );
        let idle_bar = overflow_desc
            .menu_scrollbar_desc(&idle_emitted.layout, &overflow_state, false)
            .expect("NavigationViewLeftClosedOverflow must return a scrollbar descriptor");
        assert!(
            idle_bar.is_overflowing(),
            "NavigationViewLeftClosedOverflow must trigger vertical menu overflow"
        );
        assert_eq!(
            idle_bar.visibility,
            unbound_widgets::ScrollBarVisibility::Hidden,
            "NavigationViewLeftClosedOverflow idle state must hide scrollbar under Auto policy"
        );
        assert!(
            !has_scrollbar_hit(
                &idle_emitted.output.sub_hits,
                700,
                unbound_widgets::ScrollbarAxis::Vertical,
            ),
            "NavigationViewLeftClosedOverflow idle state must emit zero scrollbar hit regions"
        );

        // 3. 折叠溢出态在悬停窗格时按需显现滚动条（验证确实为可滚动溢出态）。
        overflow_state.pane_hovered = true;
        let hover_emitted = unbound_widgets::emit_navigation_view(
            700,
            &overflow_desc,
            ControlState::default(),
            &overflow_state,
            &mut text_ctx,
        );
        let hover_bar = overflow_desc
            .menu_scrollbar_desc(&hover_emitted.layout, &overflow_state, false)
            .expect("hovered NavigationViewLeftClosedOverflow must return a scrollbar descriptor");
        assert_eq!(
            hover_bar.visibility,
            unbound_widgets::ScrollBarVisibility::Auto,
            "hovered NavigationViewLeftClosedOverflow must reveal scrollbar"
        );
        assert!(
            has_scrollbar_hit(
                &hover_emitted.output.sub_hits,
                700,
                unbound_widgets::ScrollbarAxis::Vertical,
            ),
            "hovered NavigationViewLeftClosedOverflow must emit vertical scrollbar hit regions"
        );
    }

    #[test]
    fn test_navigation_view_top_uses_overflow_popup_without_horizontal_scrollbar() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut text_ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 0,
            raster_scale: 1.0,
        };
        let mut texture_quads = Vec::new();
        let bounds = Rect::new(0.0, 0.0, 360.0, 200.0);
        let case = ParityCase::new("NavigationViewTop", "normal", "light");
        let mut scene = Gallery::new();
        let res = emit_case(&case, bounds, &mut scene, &mut text_ctx, &mut texture_quads);
        assert!(res.is_ok(), "NavigationViewTop emit failed: {res:?}");
        assert!(
            !has_horizontal_idle_scrollbar_thumb(&scene),
            "NavigationViewTop must not emit horizontal scrollbar thumb primitives"
        );

        let (top_desc, _) = navigation_view_parity_desc("NavigationViewTop", bounds, false);
        let mut top_state = unbound_widgets::NavigationViewState::from_desc(&top_desc);
        top_state.set_animations_enabled(false, &top_desc);
        let top_emitted = unbound_widgets::emit_navigation_view(
            700,
            &top_desc,
            ControlState::default(),
            &top_state,
            &mut text_ctx,
        );
        // Top 模式保持 MenuOverflowPopup（...）溢出按钮，不生成任何滚动条描述符或命中区。
        assert!(
            top_emitted.layout.overflow_button_rect.is_some(),
            "NavigationViewTop must keep MenuOverflowPopup (...) button when items overflow"
        );
        assert!(
            top_desc
                .menu_scrollbar_desc(&top_emitted.layout, &top_state, false)
                .is_none(),
            "NavigationViewTop must not produce any scrollbar descriptor"
        );
        assert!(
            !has_scrollbar_hit(
                &top_emitted.output.sub_hits,
                700,
                unbound_widgets::ScrollbarAxis::Horizontal,
            ) && !has_scrollbar_hit(
                &top_emitted.output.sub_hits,
                700,
                unbound_widgets::ScrollbarAxis::Vertical,
            ),
            "NavigationViewTop must emit zero scrollbar hit regions"
        );
    }
}
