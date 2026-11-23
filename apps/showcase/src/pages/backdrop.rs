//! 后景两轴对照页（Backdrop）。
//! lang-allow: bilingual showcase page copy; Chinese is the zh-CN half
//!
//! 页面同时暴露后景的两个正交维度：**材质种类**（无、跟随系统、标准云母、云母变体、
//! 亚克力、沉浸式、亚克力自定义色调）与**来源**（自动、系统原生、引擎自绘）。两个
//! 维度各自成行，互不影响：改材质不会动来源，改来源也不会动材质。
//!
//! 读出面板同时显示五组事实：用户的选择、宿主解析出的实际生效组合、平台探测到的
//! Windows 构建号、所选材质的原生可用性、当前亚克力自定义色调；回退原因与机器能力
//! 不足的原因各占一整行，没有发生时显式写「None」。
//!
//! 两类不可用在界面上走两条通道：机器承载不了的组合进入禁用态；显式自绘来源撞上
//! 没有自绘变体的材质属于**非法组合**，仍然可以选中，选中的按钮带告警色顶条、
//! 告警色边框与 «INVALID COMBO» 角标，避免用户把它误读成禁用。

use accesskit::Role;
use unbound_backdrop::{
    BackdropCombination, BackdropSelection, BackdropSource, FallbackReason, MaterialKind,
    UnavailableReason,
};
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, PageInfo};
use crate::canvas::{AlignX, AlignY};

/// 材质行的起始部件编号。
///
/// 后景页的部件编号分两段，互不重叠：**材质行**从 [`MATERIAL_PART_BASE`] 起，按材质
/// 种类顺序占 7 个编号；**来源行**从 [`SOURCE_PART_BASE`] 起，按来源顺序占 3 个编号。
/// 外壳按同一套编号派发点击，因此这里定义一次、两处使用。
pub const MATERIAL_PART_BASE: u16 = 1;

/// 来源行的起始部件编号：紧接材质行之后。
pub const SOURCE_PART_BASE: u16 = MATERIAL_PART_BASE + MaterialKind::ALL.len() as u16;

/// 按钮标签的字号上限与下限。
const LABEL_SIZE: f32 = 11.5;
const LABEL_MIN_SIZE: f32 = 9.0;
/// 按钮左右内边距，也是分配按钮宽度时给标签预留的底线。
const CELL_PADDING: f32 = 20.0;
/// 读出面板里原因行的字段名宽度与原因短名宽度。
const FIELD_LABEL_W: f32 = 88.0;
const REASON_NAME_W: f32 = 132.0;
/// 读出面板的高度。
///
/// 五组事实按两列排成三行，下面再接两行原因，因此比四组事实的面板更高。
const READOUT_H: f32 = 180.0;

pub const BACKDROP_INFO: PageInfo = PageInfo {
    title: "Backdrop Material & Source Axes",
    short: "Backdrop",
    description: "Compare the DWM-native material path, the self-drawn wallpaper blur and the disabled backdrop; pick a material kind and a source independently, then read back the effective combination resolved by the host.",
    crates: &[
        "unbound-backdrop",
        "unbound-app",
        "unbound-platform",
        "unbound-gpu",
    ],
    keywords: "backdrop mica dwm native self-drawn wallpaper blur transparency swapchain material source auto windows build",
};

pub const BACKDROP_EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "Backdrop Material & Source Picker",
    "Pick a material kind on the first row and a source on the second. The host resolves both axes on the next frame and reports the effective combination, the fallback reason and the reason the machine cannot host the material.",
    &["unbound-backdrop", "unbound-app"],
    r#"let combination = resolve_combination(selection, &capabilities);
if combination != state.effective {
    state.effective = combination;
    request_rebuild();
}"#,
    "Every material and source button publishes the Button role, an accessible name and the selected state; the two rows are told apart by their accessible names.",
    &[],
    DemoKind::Backdrop,
)
.with_height(380.0)];

/// 渲染后景两轴对照舞台。
pub fn backdrop(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let selection = ctx.state.backdrop_selection;
    let combination = ctx.state.backdrop_combination;
    let windows_build = ctx.state.windows_build;
    let native_available = ctx.state.backdrop_native_available();
    let accent_tint = ctx.state.backdrop_accent_tint;
    // 当前选择是否落在「组合非法但允许选中」上：它与机器能力不足是两回事，
    // 前者允许选中，只在按钮上做可见标记，不禁用。
    let illegal_combination = ctx.state.backdrop_combination_illegal();

    draw_readout(
        ctx,
        selection,
        combination,
        windows_build,
        native_available,
        accent_tint,
    );

    // ---------- 两行选项组（复用 `unbound-widgets` 分段控件） ----------
    let hint_h = 30.0;
    let hint_top = rect.max().y - hint_h - 4.0;
    let row_header_h = 13.0;
    let row_gap = 8.0;
    let rows_top = rect.origin.y + 6.0 + READOUT_H + 8.0;
    // 两行按钮等高：剩下的高度平分给它们，窄视口下也不会互相压住。
    let button_h =
        ((hint_top - rows_top - 2.0 * (row_header_h + 4.0) - row_gap) / 2.0).clamp(44.0, 76.0);

    let material_top = rows_top;
    let locale = ctx.state.locale;
    draw_row_header(
        ctx,
        material_top,
        locale.pick("MATERIAL", "材质类型 (MATERIAL)"),
    );
    let material_row_rect = Rect::new(
        rect.origin.x,
        material_top + row_header_h + 4.0,
        rect.size.x,
        button_h,
    );
    let material_labels: Vec<&str> = MaterialKind::ALL.iter().map(|kind| kind.label()).collect();
    let (material_widths, material_label_sizes) =
        weighted_slot_widths(ctx, &material_labels, material_row_rect);
    let material_selected_idx = MaterialKind::ALL
        .iter()
        .position(|kind| *kind == selection.material)
        .unwrap_or(0);
    let mut material_a11y = Vec::with_capacity(MaterialKind::ALL.len());
    let mut material_meta = Vec::with_capacity(MaterialKind::ALL.len());
    for kind in MaterialKind::ALL {
        let selected = kind == selection.material;
        let illegal = selected && illegal_combination;
        let disabled = !ctx.state.backdrop_material_selectable(kind);
        let reason = if disabled {
            unbound_backdrop::unavailable_reason(kind, &ctx.state.backdrop_capabilities())
        } else {
            None
        };
        let mut accessible_name = format!(
            "{}: {} — {}",
            locale.pick("Material", "材质"),
            kind.label(),
            kind.description()
        );
        if illegal {
            let fallback = FallbackReason::NoSelfDrawnVariant { material: kind };
            accessible_name.push_str(&format!(
                " — {}: {}",
                locale.pick("Invalid combination", "非法组合"),
                fallback.label()
            ));
        }
        material_a11y.push(accessible_name);
        material_meta.push((kind, selected, disabled, illegal, reason));
    }
    let mut cursor_x = material_row_rect.origin.x;
    let material_slots: Vec<crate::widgets::SegmentedSlot<'_>> = material_meta
        .iter()
        .enumerate()
        .map(|(idx, (kind, _selected, disabled, _illegal, _reason))| {
            let w = material_widths[idx];
            let seg_x = cursor_x + unbound_widgets::segmented::SEGMENT_INSET;
            let seg_w = (w - unbound_widgets::segmented::SEGMENT_INSET * 2.0).max(8.0);
            cursor_x += w;
            let part = MATERIAL_PART_BASE + kind.index() as u16;
            let mut slot = crate::widgets::SegmentedSlot::new(ctx.part(part), kind.label());
            slot.disabled = *disabled;
            slot.a11y_label = Some(material_a11y[idx].as_str());
            slot.a11y_role = Some(Role::Button);
            slot.font_size = Some(material_label_sizes[idx]);
            slot.label_box = Some((
                Rect::new(
                    seg_x + 8.0,
                    material_row_rect.origin.y + 6.0,
                    (seg_w - 16.0).max(16.0),
                    16.0,
                ),
                AlignX::Start,
            ));
            slot
        })
        .collect();
    let material_rects = crate::widgets::segmented_slots(
        ctx.canvas,
        material_row_rect,
        &material_slots,
        material_selected_idx,
        LABEL_SIZE,
        Some(material_widths),
    );
    for (seg_rect, (kind, selected, disabled, illegal, reason)) in
        material_rects.into_iter().zip(material_meta.iter())
    {
        draw_segment_annotations(
            ctx,
            seg_rect,
            SegmentAnnotations {
                description: kind.description(),
                selected: *selected,
                disabled: *disabled,
                illegal: *illegal,
                experimental: kind.is_experimental(),
                reason: reason.as_ref(),
            },
        );
    }

    let source_top = material_top + row_header_h + 4.0 + button_h + row_gap;
    draw_row_header(ctx, source_top, locale.pick("SOURCE", "渲染来源 (SOURCE)"));
    let source_row_rect = Rect::new(
        rect.origin.x,
        source_top + row_header_h + 4.0,
        rect.size.x,
        button_h,
    );
    let source_labels: Vec<&str> = BackdropSource::ALL
        .iter()
        .map(|source| source.label())
        .collect();
    let (source_widths, source_label_sizes) =
        weighted_slot_widths(ctx, &source_labels, source_row_rect);
    let source_selected_idx = BackdropSource::ALL
        .iter()
        .position(|source| *source == selection.source)
        .unwrap_or(0);
    let source_a11y: Vec<String> = BackdropSource::ALL
        .iter()
        .map(|source| {
            format!(
                "{}: {} — {}",
                locale.pick("Source", "来源"),
                source.label(),
                source.description()
            )
        })
        .collect();
    let mut source_cursor_x = source_row_rect.origin.x;
    let source_slots: Vec<crate::widgets::SegmentedSlot<'_>> = BackdropSource::ALL
        .iter()
        .enumerate()
        .map(|(idx, source)| {
            let w = source_widths[idx];
            let seg_x = source_cursor_x + unbound_widgets::segmented::SEGMENT_INSET;
            let seg_w = (w - unbound_widgets::segmented::SEGMENT_INSET * 2.0).max(8.0);
            source_cursor_x += w;
            let part = SOURCE_PART_BASE + source.index() as u16;
            let mut slot = crate::widgets::SegmentedSlot::new(ctx.part(part), source.label());
            slot.a11y_label = Some(source_a11y[idx].as_str());
            slot.a11y_role = Some(Role::Button);
            slot.font_size = Some(source_label_sizes[idx]);
            slot.label_box = Some((
                Rect::new(
                    seg_x + 8.0,
                    source_row_rect.origin.y + 6.0,
                    (seg_w - 16.0).max(16.0),
                    16.0,
                ),
                AlignX::Start,
            ));
            slot
        })
        .collect();
    let source_rects = crate::widgets::segmented_slots(
        ctx.canvas,
        source_row_rect,
        &source_slots,
        source_selected_idx,
        LABEL_SIZE,
        Some(source_widths),
    );
    for (seg_rect, source) in source_rects.into_iter().zip(BackdropSource::ALL.iter()) {
        draw_segment_annotations(
            ctx,
            seg_rect,
            SegmentAnnotations {
                description: source.description(),
                selected: *source == selection.source,
                disabled: false,
                illegal: false,
                experimental: false,
                reason: None,
            },
        );
    }

    // ---------- 提示行 ----------
    ctx.canvas.text(
        &hint_text(combination, locale),
        Rect::new(
            rect.origin.x + 10.0,
            hint_top,
            (rect.size.x - 20.0).max(40.0),
            hint_h - 4.0,
        ),
        9.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
}

/// 绘制读出面板：五组事实加两行原因。
///
/// 下半部两行整宽，几十个字的原因说明才读得清。
fn draw_readout(
    ctx: &mut DemoCtx<'_, '_>,
    selection: BackdropSelection,
    combination: BackdropCombination,
    windows_build: Option<u32>,
    native_available: bool,
    accent_tint: u32,
) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let info = Rect::new(
        rect.origin.x + 6.0,
        rect.origin.y + 6.0,
        rect.size.x - 12.0,
        READOUT_H,
    );
    ctx.canvas
        .rect(info, 10.0, palette.card_bg, palette.card_border, 1.0, 0.0);

    let locale = ctx.state.locale;
    let build_text = windows_build
        .map(|build| format!("Build {build}"))
        .unwrap_or_else(|| locale.pick("Unknown", "未知").to_string());
    let availability_text = if native_available {
        locale.pick("Available", "可用").to_string()
    } else {
        locale.pick("Unavailable", "不可用").to_string()
    };
    let fields = [
        (
            locale.pick("USER CHOICE", "用户选择"),
            format!(
                "{} · {}",
                selection.material.label(),
                selection.source.label()
            ),
            true,
        ),
        (
            locale.pick("EFFECTIVE", "实际生效"),
            format!(
                "{} · {}",
                combination.material.label(),
                combination.source.label()
            ),
            false,
        ),
        (
            locale.pick("WINDOWS BUILD", "Windows 构建号"),
            build_text,
            false,
        ),
        (
            locale.pick("NATIVE AVAILABLE", "原生可用性"),
            availability_text,
            false,
        ),
        // 色调是我们自己的选择，十六进制展示即可（位序为 AABBGGRR）。
        (
            locale.pick("ACCENT TINT", "亚克力强调色调"),
            format!("#{accent_tint:08X}"),
            false,
        ),
    ];
    let column_gap = 14.0;
    let column_w = ((info.size.x - 24.0 - column_gap) / 2.0).max(120.0);
    let field_h = 34.0;
    for (index, (label, value, highlight)) in fields.iter().enumerate() {
        let column = index % 2;
        let row = index / 2;
        let cell = Rect::new(
            info.origin.x + 12.0 + column as f32 * (column_w + column_gap),
            info.origin.y + 6.0 + row as f32 * field_h,
            column_w,
            field_h - 4.0,
        );
        ctx.canvas.text(
            label,
            Rect::new(cell.origin.x, cell.origin.y, cell.size.x, 12.0),
            8.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            value,
            Rect::new(
                cell.origin.x,
                cell.origin.y + 13.0,
                cell.size.x,
                cell.size.y - 13.0,
            ),
            12.0,
            if *highlight {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
    }

    // 回退原因与不可用原因各占一整行；五组事实占三行，因此原因行从第三行下方开始。
    let reason_top = info.origin.y + 6.0 + 3.0 * field_h + 4.0;
    let reason_h = 30.0;
    draw_reason_row(
        ctx,
        Rect::new(
            info.origin.x + 12.0,
            reason_top,
            info.size.x - 24.0,
            reason_h,
        ),
        locale.pick("FALLBACK", "回退原因"),
        combination
            .fallback
            .as_ref()
            .map(|reason| (reason.label(), reason.description())),
    );
    draw_reason_row(
        ctx,
        Rect::new(
            info.origin.x + 12.0,
            reason_top + reason_h,
            info.size.x - 24.0,
            reason_h,
        ),
        locale.pick("UNAVAILABLE", "不可用原因"),
        combination
            .unavailable
            .as_ref()
            .map(|reason| (reason.label(), reason.description())),
    );
}

/// 绘制一行原因读出：字段名、原因短名、原因说明。
fn draw_reason_row(
    ctx: &mut DemoCtx<'_, '_>,
    rect: Rect,
    field: &str,
    reason: Option<(&'static str, String)>,
) {
    let palette = *ctx.canvas.palette;
    ctx.canvas.text(
        field,
        Rect::new(rect.origin.x, rect.origin.y, FIELD_LABEL_W, rect.size.y),
        8.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
    let name = Rect::new(
        rect.origin.x + FIELD_LABEL_W,
        rect.origin.y,
        REASON_NAME_W,
        rect.size.y,
    );
    match reason {
        Some((reason_label, description)) => {
            ctx.canvas.text(
                reason_label,
                name,
                10.0,
                palette.warning,
                AlignX::Start,
                AlignY::Center,
            );
            ctx.canvas.text_wrapped(
                &description,
                Rect::new(
                    name.max().x,
                    rect.origin.y,
                    (rect.max().x - name.max().x).max(40.0),
                    rect.size.y,
                ),
                9.0,
                palette.text_muted,
            );
        }
        None => {
            ctx.canvas.text(
                ctx.state.locale.pick("None", "无"),
                name,
                10.0,
                palette.text_muted,
                AlignX::Start,
                AlignY::Center,
            );
        }
    }
}

/// 绘制一行按钮上方的小标题。
fn draw_row_header(ctx: &mut DemoCtx<'_, '_>, y: f32, title: &str) {
    let palette = *ctx.canvas.palette;
    ctx.canvas.text(
        title,
        Rect::new(
            ctx.rect.origin.x + 10.0,
            y,
            (ctx.rect.size.x - 20.0).max(40.0),
            13.0,
        ),
        8.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
}

/// 分段槽位上的附加展示信息（说明文案、实验标记、非法组合告警与不可用原因）。
struct SegmentAnnotations<'a> {
    description: &'a str,
    selected: bool,
    disabled: bool,
    /// 选中的组合非法（该材质没有自绘变体）：允许选中，但需要可见告警标记。
    illegal: bool,
    experimental: bool,
    reason: Option<&'a UnavailableReason>,
}

/// 在已由 `unbound-widgets` 分段控件绘制好的槽位上叠加说明文案、实验标记与非法组合告警。
fn draw_segment_annotations(ctx: &mut DemoCtx<'_, '_>, cell: Rect, info: SegmentAnnotations<'_>) {
    let palette = *ctx.canvas.palette;
    // 非法组合：在槽位顶部叠加告警色细条与边框，避免用户将其与禁用态混淆。
    if info.illegal {
        ctx.canvas.rect(
            Rect::new(
                cell.origin.x + 8.0,
                cell.origin.y + 1.5,
                (cell.size.x - 16.0).max(8.0),
                3.0,
            ),
            1.5,
            palette.warning,
            palette.warning,
            0.0,
            0.0,
        );
    }
    let inner = (cell.size.x - 16.0).max(20.0);
    // 实验档在右上角加一枚小标记，让它在材质行里与常规档显式区分。
    let locale = ctx.state.locale;
    if info.experimental && cell.size.x >= 150.0 {
        let badge = locale.pick("EXPERIMENTAL", "实验特性");
        let badge_glyph_width = ctx.canvas.labels.metrics(badge, 8.0).width;
        ctx.canvas.text(
            badge,
            Rect::new(
                cell.max().x - 8.0 - badge_glyph_width,
                cell.origin.y + 7.0,
                badge_glyph_width,
                14.0,
            ),
            8.0,
            if info.selected && !info.disabled {
                palette.on_primary
            } else {
                palette.warning
            },
            AlignX::Start,
            AlignY::Center,
        );
    }
    let title_bottom = cell.origin.y + 24.0;
    let selected_strip = if info.selected { 14.0 } else { 0.0 };
    let detail = info.reason.map_or_else(
        || info.description.to_string(),
        |reason| reason.description(),
    );
    let detail_height = (cell.max().y - title_bottom - 4.0 - selected_strip).max(0.0);
    if cell.size.x >= 104.0 && detail_height >= 12.0 {
        ctx.canvas.text_wrapped(
            &detail,
            Rect::new(cell.origin.x + 8.0, title_bottom, inner, detail_height),
            9.0,
            if info.reason.is_some() {
                palette.warning
            } else if info.selected && !info.disabled {
                palette.on_primary
            } else {
                palette.text_muted
            },
        );
    }
    if info.selected && !info.disabled {
        ctx.canvas.text(
            locale.pick("SELECTED", "已选中"),
            Rect::new(cell.origin.x + 8.0, cell.max().y - 16.0, inner, 12.0),
            8.5,
            palette.on_primary,
            AlignX::Start,
            AlignY::Center,
        );
    }
    // 非法组合的角标：与「SELECTED」同一行、贴右边，宽度够时始终可见。
    if info.illegal && cell.size.x >= 160.0 {
        let badge = locale.pick("INVALID COMBO", "非法组合");
        let badge_width = ctx.canvas.labels.metrics(badge, 8.0).width;
        ctx.canvas.text(
            badge,
            Rect::new(
                cell.max().x - 8.0 - badge_width,
                cell.max().y - 16.0,
                badge_width,
                12.0,
            ),
            8.0,
            palette.warning,
            AlignX::Start,
            AlignY::Center,
        );
    }
}

/// 按标签宽度加权计算分段控件每列宽度及自适应字号，防止长标签在窄视口越界。
fn weighted_slot_widths(
    ctx: &mut DemoCtx<'_, '_>,
    labels: &[&str],
    origin: Rect,
) -> (Vec<f32>, Vec<f32>) {
    let measured: Vec<f32> = labels
        .iter()
        .map(|label| ctx.canvas.labels.metrics(label, LABEL_SIZE).width)
        .collect();
    let weights: Vec<f32> = measured.iter().map(|width| width + CELL_PADDING).collect();
    let total: f32 = weights.iter().sum();
    let usable = origin.size.x.max(0.0);
    let mut widths = Vec::with_capacity(labels.len());
    let mut sizes = Vec::with_capacity(labels.len());
    for (index, width_weight) in weights.iter().enumerate() {
        let width = if total > 0.0 {
            usable * width_weight / total
        } else {
            usable / labels.len().max(1) as f32
        };
        let inner = (width - CELL_PADDING).max(8.0);
        let size = if measured[index] > 0.0 {
            (LABEL_SIZE * inner / measured[index]).clamp(LABEL_MIN_SIZE, LABEL_SIZE)
        } else {
            LABEL_SIZE
        };
        widths.push(width);
        sizes.push(size);
    }
    (widths, sizes)
}

/// 底部的提示行：说明当前实际生效的是哪条路径、为什么。
fn hint_text(combination: BackdropCombination, locale: unbound_app::Locale) -> String {
    let mut hint = String::new();
    if let Some(unavailable) = &combination.unavailable {
        hint.push_str(&format!(
            "{}{}.",
            locale.pick("Unavailable on this machine: ", "当前机器不支持："),
            unavailable.description()
        ));
    }
    if let Some(fallback) = &combination.fallback {
        hint.push(' ');
        hint.push_str(&fallback.description());
        hint.push('.');
    }
    if !hint.is_empty() {
        return hint;
    }
    match (combination.material, combination.source) {
        (MaterialKind::None, _) => locale
            .pick(
                "Backdrop disabled: the root surface uses the opaque theme colour.",
                "后景已关闭：根表面使用不透明主题背景色。",
            )
            .to_string(),
        (MaterialKind::AcrylicTint, BackdropSource::Native) => locale
            .pick(
                "Accent policy path active: the system compositor draws the tinted acrylic behind a transparent swapchain; this bypasses the DWM system material channel.",
                "Accent Policy 路径生效：系统合成器在透明交换链后方绘制自定义色调 Acrylic，绕过 DWM 系统材质通道。",
            )
            .to_string(),
        (_, BackdropSource::Native) => {
            if locale.is_zh() {
                format!(
                    "原生路径生效：系统合成器在透明交换链后方绘制 {} 材质。",
                    combination.material.label()
                )
            } else {
                format!(
                    "Native path active: the system compositor draws the {} material behind a transparent swapchain.",
                    combination.material.label()
                )
            }
        }
        (_, BackdropSource::SelfDrawn) => locale
            .pick(
                "Self-drawn path active: an approximation built from the primary wallpaper (cover fit, blurred, sampled at this window's screen rect); it cannot capture content behind the window.",
                "自绘路径生效：基于主壁纸等比铺满模糊并按窗口屏幕区域采样近似（不采集窗口后方其他应用内容）。",
            )
            .to_string(),
        (_, BackdropSource::Auto) => locale
            .pick(
                "Automatic source: the platform capability decides between the native and self-drawn paths.",
                "自动来源：根据平台能力自动在原生与自绘路径之间择优选择。",
            )
            .to_string(),
    }
}
