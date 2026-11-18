//! 后景两轴对照页（Backdrop）。
//!
//! 页面同时暴露后景的两个正交维度：**材质种类**（无、跟随系统、标准云母、云母变体、
//! 亚克力、沉浸式）与**来源**（自动、系统原生、引擎自绘）。两个维度各自成行，
//! 互不影响：改材质不会动来源，改来源也不会动材质。
//!
//! 读出面板同时显示四组事实：用户的选择、宿主解析出的实际生效组合、平台探测到的
//! Windows 构建号、所选材质的原生可用性；回退原因与机器能力不足的原因各占一整行，
//! 没有发生时显式写「None」。

use accesskit::Role;
use unbound_backdrop::{BackdropCombination, BackdropSelection, BackdropSource, MaterialKind};
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, PageInfo};
use crate::canvas::{AlignX, AlignY};

/// 材质行的起始部件编号。
///
/// 后景页的部件编号分两段，互不重叠：**材质行**从 [`MATERIAL_PART_BASE`] 起，按材质
/// 种类顺序占 6 个编号；**来源行**从 [`SOURCE_PART_BASE`] 起，按来源顺序占 3 个编号。
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
const READOUT_H: f32 = 146.0;

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

    draw_readout(ctx, selection, combination, windows_build, native_available);

    // ---------- 两行按钮 ----------
    let hint_h = 30.0;
    let hint_top = rect.max().y - hint_h - 4.0;
    let row_header_h = 13.0;
    let row_gap = 8.0;
    let gap = 8.0;
    let rows_top = rect.origin.y + 6.0 + READOUT_H + 8.0;
    // 两行按钮等高：剩下的高度平分给它们，窄视口下也不会互相压住。
    let button_h =
        ((hint_top - rows_top - 2.0 * (row_header_h + 4.0) - row_gap) / 2.0).clamp(44.0, 76.0);

    let material_top = rows_top;
    draw_row_header(ctx, material_top, "MATERIAL");
    let material_labels: Vec<&str> = MaterialKind::ALL.iter().map(|kind| kind.label()).collect();
    let material_cells = button_cells(
        ctx,
        &material_labels,
        Rect::new(
            rect.origin.x,
            material_top + row_header_h + 4.0,
            rect.size.x,
            button_h,
        ),
        gap,
    );
    for (kind, (cell, label_size)) in MaterialKind::ALL.iter().zip(material_cells) {
        let part = MATERIAL_PART_BASE + kind.index() as u16;
        let selected = *kind == selection.material;
        draw_choice_button(
            ctx,
            part,
            cell,
            label_size,
            kind.label(),
            kind.description(),
            selected,
        );
        let accessible_name = format!("Material: {} — {}", kind.label(), kind.description());
        ctx.control(part, cell, Role::Button, &accessible_name, selected);
    }

    let source_top = material_top + row_header_h + 4.0 + button_h + row_gap;
    draw_row_header(ctx, source_top, "SOURCE");
    let source_labels: Vec<&str> = BackdropSource::ALL
        .iter()
        .map(|source| source.label())
        .collect();
    let source_cells = button_cells(
        ctx,
        &source_labels,
        Rect::new(
            rect.origin.x,
            source_top + row_header_h + 4.0,
            rect.size.x,
            button_h,
        ),
        gap,
    );
    for (source, (cell, label_size)) in BackdropSource::ALL.iter().zip(source_cells) {
        let part = SOURCE_PART_BASE + source.index() as u16;
        let selected = *source == selection.source;
        draw_choice_button(
            ctx,
            part,
            cell,
            label_size,
            source.label(),
            source.description(),
            selected,
        );
        let accessible_name = format!("Source: {} — {}", source.label(), source.description());
        ctx.control(part, cell, Role::Button, &accessible_name, selected);
    }

    // ---------- 提示行 ----------
    ctx.canvas.text(
        &hint_text(combination),
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

/// 绘制读出面板：四组事实加两行原因。
///
/// 下半部两行整宽，几十个字的原因说明才读得清。
fn draw_readout(
    ctx: &mut DemoCtx<'_, '_>,
    selection: BackdropSelection,
    combination: BackdropCombination,
    windows_build: Option<u32>,
    native_available: bool,
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

    let build_text = windows_build
        .map(|build| format!("Build {build}"))
        .unwrap_or_else(|| "Unknown".to_string());
    let availability_text = if native_available {
        "Available".to_string()
    } else {
        "Unavailable".to_string()
    };
    let fields = [
        (
            "USER CHOICE",
            format!(
                "{} · {}",
                selection.material.label(),
                selection.source.label()
            ),
            true,
        ),
        (
            "EFFECTIVE",
            format!(
                "{} · {}",
                combination.material.label(),
                combination.source.label()
            ),
            false,
        ),
        ("WINDOWS BUILD", build_text, false),
        ("NATIVE AVAILABLE", availability_text, false),
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

    // 回退原因与不可用原因各占一整行。
    let reason_top = info.origin.y + 6.0 + 2.0 * field_h + 4.0;
    let reason_h = 30.0;
    draw_reason_row(
        ctx,
        Rect::new(
            info.origin.x + 12.0,
            reason_top,
            info.size.x - 24.0,
            reason_h,
        ),
        "FALLBACK",
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
        "UNAVAILABLE",
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
                "None",
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

/// 画一个选择按钮：标题、说明与选中标记。
fn draw_choice_button(
    ctx: &mut DemoCtx<'_, '_>,
    part: u16,
    cell: Rect,
    label_size: f32,
    label: &str,
    description: &str,
    selected: bool,
) {
    let palette = *ctx.canvas.palette;
    let hover = ctx.canvas.visual(ctx.part(part), selected);
    let background = if selected {
        palette.primary_soft
    } else if hover.is_hovered() {
        palette.control_hover
    } else {
        palette.card_bg
    };
    let border = if selected {
        palette.primary
    } else {
        palette.card_border
    };
    ctx.canvas.rect(
        cell,
        8.0,
        background,
        border,
        if selected { 1.5 } else { 1.0 },
        0.0,
    );
    let inner = (cell.size.x - CELL_PADDING).max(20.0);
    ctx.canvas.text(
        label,
        Rect::new(cell.origin.x + 10.0, cell.origin.y + 8.0, inner, 16.0),
        label_size,
        if selected {
            palette.primary
        } else {
            palette.text
        },
        AlignX::Start,
        AlignY::Center,
    );
    // 说明只在按钮够宽时出现：窄视口下让它换行会压到「SELECTED」标记上。
    if cell.size.x >= 104.0 {
        ctx.canvas.text_wrapped(
            description,
            Rect::new(
                cell.origin.x + 10.0,
                cell.origin.y + 28.0,
                inner,
                (cell.size.y - 50.0).max(18.0),
            ),
            9.0,
            palette.text_muted,
        );
    }
    if selected {
        ctx.canvas.text(
            "SELECTED",
            Rect::new(cell.origin.x + 10.0, cell.max().y - 24.0, inner, 14.0),
            8.5,
            palette.primary,
            AlignX::Start,
            AlignY::Center,
        );
    }
}

/// 按标签宽度分配一行按钮，并给出每个按钮的矩形与标签字号。
///
/// 材质行有六档、标签长度相差很大：等分宽度会让长标签挤出按钮，因此按
/// 「标签宽度 + 内边距」加权分配；实在放不下时按比例缩字号，宁可字小也不越界。
fn button_cells(
    ctx: &mut DemoCtx<'_, '_>,
    labels: &[&str],
    origin: Rect,
    gap: f32,
) -> Vec<(Rect, f32)> {
    let measured: Vec<f32> = labels
        .iter()
        .map(|label| ctx.canvas.labels.metrics(label, LABEL_SIZE).width)
        .collect();
    let weights: Vec<f32> = measured.iter().map(|width| width + CELL_PADDING).collect();
    let total: f32 = weights.iter().sum();
    let usable = (origin.size.x - gap * (labels.len() as f32 - 1.0)).max(0.0);
    let mut cells = Vec::with_capacity(labels.len());
    let mut x = origin.origin.x;
    for (index, width_weight) in weights.iter().enumerate() {
        let width = if total > 0.0 {
            usable * width_weight / total
        } else {
            usable / labels.len() as f32
        };
        let inner = (width - CELL_PADDING).max(8.0);
        let size = if measured[index] > 0.0 {
            (LABEL_SIZE * inner / measured[index]).clamp(LABEL_MIN_SIZE, LABEL_SIZE)
        } else {
            LABEL_SIZE
        };
        cells.push((Rect::new(x, origin.origin.y, width, origin.size.y), size));
        x += width + gap;
    }
    cells
}

/// 底部的提示行：说明当前实际生效的是哪条路径、为什么。
fn hint_text(combination: BackdropCombination) -> String {
    let mut hint = String::new();
    if let Some(unavailable) = &combination.unavailable {
        hint.push_str(&format!(
            "Unavailable on this machine: {}.",
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
        (MaterialKind::None, _) => {
            "Backdrop disabled: the root surface uses the opaque theme colour.".to_string()
        }
        (_, BackdropSource::Native) => format!(
            "Native path active: the system compositor draws the {} material behind a transparent swapchain.",
            combination.material.label()
        ),
        (_, BackdropSource::SelfDrawn) => "Self-drawn path active: an approximation built from the primary wallpaper (cover fit, blurred, sampled at this window's screen rect); it cannot capture content behind the window.".to_string(),
        (_, BackdropSource::Auto) => "Automatic source: the platform capability decides between the native and self-drawn paths.".to_string(),
    }
}
