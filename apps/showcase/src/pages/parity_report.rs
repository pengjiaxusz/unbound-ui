//! WinUI 3 像素对照报告页：在框架内部直接浏览参考截图、本框架渲染与 ΔE 热力图。
//! lang-allow: bilingual page metadata; Chinese is the zh-CN half
//!
//! 数据来自 `target/parity/report.json` 与三组同尺寸 PNG。页面本身没有任何手绘仿制品：
//! 截图经 `AssetHub` 异步解码后由真实的 `emit_image_with_assets` 发射器绘制，
//! 指标文本则由框架自己的文本引擎排版。

use std::path::{Path, PathBuf};

use unbound_app::AssetHub;
use unbound_asset::AssetId;
use unbound_math::{Rect, Vec2};
use unbound_widgets::image::{emit_image_with_assets, ImageDesc, ObjectFit};
use unbound_widgets::{ControlState, WidgetCatalogId};

use super::{DemoCtx, DemoKind, ExampleDef, PageInfo};
use crate::canvas::{AlignX, AlignY};

pub const PARITY_INFO: PageInfo = PageInfo {
    title: "WinUI 3 pixel parity",
    short: "Pixel parity",
    description: "Every pilot control compared against its WinUI 3 capture inside the tight Region of Interest (ROI): the reference, this framework's render at the same bounds, and the CIEDE2000 heatmap, judged by 4 gates (ROI mean/p95, outlier ratio, content bbox, and intrinsic size).",
    crates: &["tools/winui-reference", "apps/showcase/src/png_diff.rs"],
    keywords: "winui parity delta e screenshot pixel comparison heatmap reference roi cache",
};

pub const PARITY_EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "Reference / unbound-ui / ROI ΔE",
    "Each row holds three captures of the same control: the WinUI 3 reference, the unbound-ui render at identical bounds, and the CIEDE2000 heatmap with ROI bounds. A row fails when ROI mean ΔE, p95, outlier ratio, content bbox, or intrinsic size drifts.",
    &["tools/winui-reference", "apps/showcase/src/png_diff.rs"],
    r#"showcase --parity-run --reference tools/winui-reference/captures --out target/parity"#,
    "A scrollable parity report rendered by the gallery itself; every case exposes its name, pass state, ROI colour metrics, intrinsic size check, and three captures.",
    &[],
    DemoKind::ParityReport,
)
.with_height(5600.0)];

/// 差分产物目录（与 `--parity-run` 的默认输出一致）。
const REPORT_DIR: &str = "target/parity";
/// 旧版参考截图的回退目录（报告尚未打包参考图时使用）。
const REFERENCE_FALLBACK: &str = "tools/winui-reference/captures";

/// 缩略图最大宽度；更窄的窗口按比例缩小。
const THUMB_MAX_WIDTH: f32 = 320.0;
/// 参考缩略图的显示宽高比。
const THUMB_ASPECT: f32 = 420.0 / 160.0;
/// 三张缩略图之间的间距。
const THUMB_GAP: f32 = 10.0;
/// 概要横幅的高度。
const SUMMARY_HEIGHT: f32 = 64.0;
/// 概要横幅与首行对照卡片之间的间距。
const SUMMARY_GAP: f32 = 12.0;
/// 相邻对照卡片之间的间距。
const ROW_GAP: f32 = 8.0;
/// 列表底部的留白。
const LIST_PAD_BOTTOM: f32 = 8.0;
/// 行标题与指标文本占用的高度（缩略图从该高度之下开始）。
const ROW_HEAD: f32 = 64.0;

/// 三列缩略图随舞台宽度收缩的比例系数。
fn thumb_scale(stage_width: f32) -> f32 {
    let inner_width = (stage_width - 24.0).max(1.0);
    (inner_width / (3.0 * THUMB_MAX_WIDTH + 2.0 * THUMB_GAP)).clamp(0.25, 1.0)
}

/// 单行对照卡片的高度。
fn row_height(stage_width: f32) -> f32 {
    ROW_HEAD + THUMB_MAX_WIDTH * thumb_scale(stage_width) / THUMB_ASPECT
}

/// 报告舞台容纳全部用例所需的高度。
///
/// 布局树必须按实际用例数预留舞台高度：若沿用固定高度，页面滚动范围会停在列表中途，
/// 剩余用例被卡片裁剪，滚动条再怎么拖也看不到完整的对比列表。
#[must_use]
pub fn stage_height(case_count: usize, stage_width: f32) -> f32 {
    SUMMARY_HEIGHT
        + SUMMARY_GAP
        + case_count as f32 * (row_height(stage_width) + ROW_GAP)
        + LIST_PAD_BOTTOM
}

/// 单个用例的四态结论，与差分器的 `pass` / `known-aa` / `fail` / `invalid` 一一对应。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParityStatus {
    /// 四重门禁全部通过。
    Pass,
    /// 内容有效但至少一门禁未通过。
    Fail,
    /// 采集或渲染内容无效，不计入通过率。
    Invalid,
    /// 几何与整体颜色已对齐，仅剩经视觉复核的轮廓抗锯齿差异。
    KnownAa,
}

impl ParityStatus {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "pass" => Some(Self::Pass),
            "fail" => Some(Self::Fail),
            "invalid" => Some(Self::Invalid),
            "known-aa" => Some(Self::KnownAa),
            _ => None,
        }
    }
}

/// 单个对照用例。
#[derive(Debug)]
pub struct ParityCase {
    pub name: String,
    pub control: String,
    pub state: String,
    pub theme: String,
    pub backdrop: String,
    pub mean_de: f32,
    pub p95_de: f32,
    pub p99_de: f32,
    pub over_limit_ratio: f32,
    pub intrinsic_matches: bool,
    pub cache_hit: bool,
    /// 本框架包围盒相对参考包围盒的逐边差值。
    pub bbox_delta: [i64; 4],
    pub status: ParityStatus,
    /// 参考 / 本框架 / 热力图三张纹理。
    pub images: [Option<AssetId>; 3],
}

/// 已载入的对照报告。
#[derive(Debug)]
pub struct ParityReport {
    pub cases: Vec<ParityCase>,
    pub passed: usize,
    pub failed: usize,
    pub invalid: usize,
    /// 经视觉复核的纯轮廓抗锯齿豁免数。
    pub known_aa: usize,
    pub cache_hits: usize,
    /// 载入失败原因；为 `Some` 时页面显示空态说明。
    pub error: Option<String>,
    /// 空态里给出的下一步命令，随运行形态（仓库内 / 独立分发）切换。
    pub hint: String,
}

impl ParityReport {
    /// 读取 `report.json` 并提交全部截图进行异步解码。
    #[must_use]
    pub fn load(hub: &mut AssetHub) -> Self {
        let hint = next_step_hint();
        let root = Path::new(REPORT_DIR);
        let json = match std::fs::read_to_string(root.join("report.json")) {
            Ok(text) => text,
            Err(error) => {
                return Self {
                    cases: Vec::new(),
                    passed: 0,
                    failed: 0,
                    invalid: 0,
                    known_aa: 0,
                    cache_hits: 0,
                    error: Some(format!("{REPORT_DIR}/report.json: {error}")),
                    hint,
                }
            }
        };
        let reference = reference_dir(root);
        let ours = root.join("ours");
        let heat = root.join("heat");
        let mut cases = parse_cases(&json);
        for case in &mut cases {
            case.images = std::array::from_fn(|which| {
                let dir: &Path = match which {
                    0 => &reference,
                    1 => &ours,
                    _ => &heat,
                };
                std::fs::read(dir.join(format!("{}.png", case.name)))
                    .ok()
                    .map(|bytes| hub.request_png(bytes))
            });
        }
        let passed = cases
            .iter()
            .filter(|case| case.status == ParityStatus::Pass)
            .count();
        let failed = cases
            .iter()
            .filter(|case| case.status == ParityStatus::Fail)
            .count();
        let invalid = cases
            .iter()
            .filter(|case| case.status == ParityStatus::Invalid)
            .count();
        let known_aa = cases
            .iter()
            .filter(|case| case.status == ParityStatus::KnownAa)
            .count();
        let cache_hits = cases.iter().filter(|case| case.cache_hit).count();
        if cases.is_empty() {
            return Self {
                cases,
                passed,
                failed,
                invalid,
                known_aa,
                cache_hits,
                error: Some(format!("{REPORT_DIR}/report.json 中没有用例")),
                hint,
            };
        }
        Self {
            cases,
            passed,
            failed,
            invalid,
            known_aa,
            cache_hits,
            error: None,
            hint,
        }
    }
}

/// 空态里给出的下一步命令。
///
/// 展厅既会在仓库里跑（有 justfile，配方封装了参数），也会被独立分发（没有 just，
/// 只能给完整 CLI）。这里按运行形态选一条真正可执行的命令：参考截图缺失时补上采集步，
/// 否则只提示差分。
fn next_step_hint() -> String {
    if Path::new("justfile").is_file() {
        if Path::new(REFERENCE_FALLBACK).is_dir() {
            "just parity".to_string()
        } else {
            "just parity-capture && just parity".to_string()
        }
    } else {
        r"showcase --parity-run --reference tools\winui-reference\captures --out target\parity"
            .to_string()
    }
}

/// 逐行解析报告中的用例（图像纹理由 [`ParityReport::load`] 另行提交解码）。
fn parse_cases(json: &str) -> Vec<ParityCase> {
    let mut cases = Vec::new();
    for line in json.lines() {
        if !line.contains("\"name\"") {
            continue;
        }
        let Some(name) = string_value(line, "name") else {
            continue;
        };
        let reference_bbox = bbox_value(line, "referenceBbox");
        let ours_bbox = bbox_value(line, "oursBbox");
        let bbox_delta = std::array::from_fn(|index| ours_bbox[index] - reference_bbox[index]);
        let pass = bool_value(line, "pass").unwrap_or(false);
        // 旧报告没有 `status` 字段，按 `pass` 回退成两态。
        let status = string_value(line, "status")
            .and_then(|text| ParityStatus::parse(&text))
            .unwrap_or(if pass {
                ParityStatus::Pass
            } else {
                ParityStatus::Fail
            });
        cases.push(ParityCase {
            name,
            control: string_value(line, "control").unwrap_or_default(),
            state: string_value(line, "state").unwrap_or_default(),
            theme: string_value(line, "theme").unwrap_or_default(),
            backdrop: string_value(line, "backdrop").unwrap_or_else(|| "solid".into()),
            mean_de: number_value(line, "meanDe").unwrap_or(0.0) as f32,
            p95_de: number_value(line, "p95De").unwrap_or(0.0) as f32,
            p99_de: number_value(line, "p99De").unwrap_or(0.0) as f32,
            over_limit_ratio: number_value(line, "overLimitRatio").unwrap_or(0.0) as f32,
            intrinsic_matches: bool_value(line, "intrinsicMatches").unwrap_or(true),
            cache_hit: bool_value(line, "cacheHit").unwrap_or(false),
            bbox_delta,
            status,
            images: std::array::from_fn(|_| None),
        });
    }
    cases
}

fn reference_dir(root: &Path) -> PathBuf {
    let packaged = root.join("reference");
    if packaged.is_dir() {
        packaged
    } else {
        PathBuf::from(REFERENCE_FALLBACK)
    }
}

/// 报告页面舞台。
pub fn report(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let zh = ctx.state.locale.is_zh();
    let Some(report) = ctx.state.demo.parity_report.as_ref() else {
        empty_state(
            ctx,
            if zh {
                "正在载入差分报告…"
            } else {
                "Loading the parity report…"
            },
            "",
        );
        return;
    };
    if let Some(error) = &report.error {
        let body = if zh {
            format!(
                "未找到可用的差分产物：{error}\n先运行 `{}`，再回到本页。",
                report.hint
            )
        } else {
            format!(
                "No parity artifacts were found: {error}\nRun `{}` first, then reopen this page.",
                report.hint
            )
        };
        empty_state(
            ctx,
            if zh {
                "差分报告不可用"
            } else {
                "Parity report unavailable"
            },
            &body,
        );
        return;
    };

    let palette = *ctx.canvas.palette;
    let visible_top = ctx.canvas.viewport.origin.y - 48.0;
    let visible_bottom = ctx.canvas.viewport.max().y + 48.0;

    // 概要横幅：通过数、ROI 四重门禁与缓存命中统计。
    let summary = Rect::new(rect.origin.x, rect.origin.y, rect.size.x, SUMMARY_HEIGHT);
    ctx.canvas.rect(
        summary,
        12.0,
        palette.card_bg,
        palette.card_border,
        1.0,
        0.0,
    );
    ctx.canvas.text(
        if zh {
            "像素对照（ROI 四重门禁）· WinUI 3 vs unbound-ui"
        } else {
            "Pixel parity (ROI 4-Gate) · WinUI 3 vs unbound-ui"
        },
        Rect::new(
            summary.origin.x + 16.0,
            summary.origin.y + 8.0,
            summary.size.x - 300.0,
            26.0,
        ),
        15.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    ctx.canvas.text(
        &format!(
            "{} / {} · {} {} · {} {} · {} {} · {} {} · {} {} · ROI mean ≤ 1.5 · p95 ≤ 4.0 · outlier ≤ 2% · {}",
            report.passed,
            report.cases.len(),
            if zh { "通过" } else { "pass" },
            report.passed,
            if zh { "抗锯齿豁免" } else { "known-aa" },
            report.known_aa,
            if zh { "未通过" } else { "fail" },
            report.failed,
            if zh { "无效" } else { "invalid" },
            report.invalid,
            if zh { "缓存" } else { "cached" },
            report.cache_hits,
            if zh {
                "包围盒与固有尺寸 ±1px"
            } else {
                "bbox & intrinsic ±1 px"
            }
        ),
        Rect::new(
            summary.origin.x + 16.0,
            summary.origin.y + 36.0,
            summary.size.x - 300.0,
            18.0,
        ),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
    let bar = Rect::new(summary.max().x - 236.0, summary.origin.y + 26.0, 220.0, 8.0);
    ctx.canvas
        .rect(bar, 4.0, palette.control_track, 0, 0.0, 0.0);
    let ratio = report.passed as f32 / report.cases.len().max(1) as f32;
    ctx.canvas.rect(
        Rect::new(bar.origin.x, bar.origin.y, bar.size.x * ratio, bar.size.y),
        4.0,
        palette.success,
        0,
        0.0,
        0.0,
    );
    ctx.canvas.text(
        &format!("{:.0}%", ratio * 100.0),
        Rect::new(bar.origin.x, bar.origin.y - 18.0, bar.size.x, 16.0),
        10.0,
        palette.text_muted,
        AlignX::End,
        AlignY::Top,
    );

    let scale = thumb_scale(rect.size.x);
    let image_height = THUMB_MAX_WIDTH * scale / THUMB_ASPECT;
    let row_h = row_height(rect.size.x);
    let mut y = summary.max().y + SUMMARY_GAP;
    for (index, case) in report.cases.iter().enumerate() {
        let row = Rect::new(rect.origin.x, y, rect.size.x, row_h);
        y += row_h + ROW_GAP;
        if row.max().y < visible_top || row.origin.y > visible_bottom {
            continue;
        }
        draw_case(ctx, row, index, case, scale, image_height, zh);
    }
}

fn draw_case(
    ctx: &mut DemoCtx<'_, '_>,
    row: Rect,
    index: usize,
    case: &ParityCase,
    scale: f32,
    image_height: f32,
    zh: bool,
) {
    let palette = *ctx.canvas.palette;
    ctx.canvas
        .rect(row, 10.0, palette.card_bg, palette.card_border, 1.0, 0.0);
    let (status_text, status_color, status_bg) = match case.status {
        ParityStatus::Pass => (
            if zh { "通过" } else { "pass" },
            palette.success,
            palette.success_soft,
        ),
        ParityStatus::Fail => (
            if zh { "未通过" } else { "fail" },
            palette.danger,
            palette.danger_soft,
        ),
        ParityStatus::Invalid => (
            if zh { "无效" } else { "invalid" },
            palette.warning,
            palette.warning_soft,
        ),
        ParityStatus::KnownAa => (
            if zh { "抗锯齿豁免" } else { "known-aa" },
            palette.primary,
            palette.primary_soft,
        ),
    };
    let badge_width = ctx.canvas.labels.measure(status_text, 9.5).x + 18.0;
    let badge = Rect::new(
        row.max().x - badge_width - 12.0,
        row.origin.y + 10.0,
        badge_width,
        18.0,
    );
    ctx.canvas.rect(badge, 9.0, status_bg, 0, 0.0, 0.0);
    ctx.canvas.text(
        status_text,
        badge,
        9.5,
        status_color,
        AlignX::Center,
        AlignY::Center,
    );
    let title = if case.control.is_empty() {
        case.name.clone()
    } else {
        format!(
            "{} · {} · {} · {}",
            case.control, case.state, case.theme, case.backdrop
        )
    };
    ctx.canvas.text(
        &title,
        Rect::new(
            row.origin.x + 14.0,
            row.origin.y + 9.0,
            row.size.x - badge_width - 190.0,
            20.0,
        ),
        11.5,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
    ctx.canvas.text(
        &format!(
            "ROI mean {:.2} · p95 {:.2} · >5ΔE {:.1}% · {} [{}, {}, {}, {}] · intrinsic {}",
            case.mean_de,
            case.p95_de,
            case.over_limit_ratio * 100.0,
            if zh { "包围盒Δ" } else { "bboxΔ" },
            case.bbox_delta[0],
            case.bbox_delta[1],
            case.bbox_delta[2],
            case.bbox_delta[3],
            if case.intrinsic_matches {
                "ok"
            } else {
                "drift"
            }
        ),
        Rect::new(
            row.origin.x + row.size.x * 0.32,
            row.origin.y + 9.0,
            row.size.x * 0.68 - badge_width - 40.0,
            20.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::End,
        AlignY::Center,
    );

    let image_width = THUMB_MAX_WIDTH * scale;
    let total = 3.0 * image_width + 2.0 * THUMB_GAP;
    let left = row.origin.x + (row.size.x - total) * 0.5;
    let top = row.origin.y + 36.0;
    let hub = ctx.state.demo.asset_hub.as_ref();
    for (which, image) in case.images.iter().enumerate() {
        let thumb = Rect::new(
            left + which as f32 * (image_width + THUMB_GAP),
            top,
            image_width,
            image_height,
        );
        ctx.canvas
            .rect(thumb, 4.0, palette.stage_bg, palette.stage_grid, 1.0, 0.0);
        let Some(hub) = hub else {
            continue;
        };
        let Some(id) = *image else {
            continue;
        };
        let part = 100 + (index as u16) * 4 + which as u16;
        let node = ctx.part(part);
        let mut desc = ImageDesc::new(thumb, Vec2::new(240.0, 96.0), 0).with_asset(id);
        desc.fit = ObjectFit::Contain;
        desc.radius = [4.0; 4];
        let output = emit_image_with_assets(node, &desc, ControlState::default(), hub);
        ctx.canvas
            .widget_output(WidgetCatalogId::Image, node, &output);
        let caption = match which {
            0 => {
                if zh {
                    "WinUI 3 参考"
                } else {
                    "WinUI 3 reference"
                }
            }
            1 => "unbound-ui",
            _ => {
                if zh {
                    "ROI ΔE2000 热力图"
                } else {
                    "ROI ΔE2000 heatmap"
                }
            }
        };
        ctx.canvas.text(
            caption,
            Rect::new(thumb.origin.x, thumb.max().y + 2.0, thumb.size.x, 14.0),
            9.0,
            palette.text_muted,
            AlignX::Center,
            AlignY::Top,
        );
    }
}

fn empty_state(ctx: &mut DemoCtx<'_, '_>, title: &str, body: &str) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let card = Rect::new(rect.origin.x, rect.origin.y, rect.size.x, 180.0);
    ctx.canvas
        .rect(card, 12.0, palette.card_bg, palette.card_border, 1.0, 0.0);
    ctx.canvas.text(
        title,
        Rect::new(
            card.origin.x + 20.0,
            card.origin.y + 20.0,
            card.size.x - 40.0,
            26.0,
        ),
        15.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    if !body.is_empty() {
        ctx.canvas.text_wrapped(
            body,
            Rect::new(
                card.origin.x + 20.0,
                card.origin.y + 54.0,
                card.size.x - 40.0,
                100.0,
            ),
            10.5,
            palette.text_muted,
        );
    }
}

// ============================================================================
// 宽松 JSON 读取：报告由本项目的 `png_diff` 生成，字段固定，无需完整解析器。
// ============================================================================

fn find_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":");
    let start = line.find(&needle)? + needle.len();
    Some(line[start..].trim_start())
}

fn string_value(line: &str, key: &str) -> Option<String> {
    let rest = find_value(line, key)?.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn number_value(line: &str, key: &str) -> Option<f64> {
    let rest = find_value(line, key)?;
    let end = rest
        .find(|character: char| {
            !(character.is_ascii_digit()
                || character == '.'
                || character == '-'
                || character == '+'
                || character == 'e'
                || character == 'E')
        })
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn bool_value(line: &str, key: &str) -> Option<bool> {
    let rest = find_value(line, key)?;
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn bbox_value(line: &str, key: &str) -> [i64; 4] {
    let Some(rest) = find_value(line, key).and_then(|rest| rest.strip_prefix('[')) else {
        return [0; 4];
    };
    let Some(end) = rest.find(']') else {
        return [0; 4];
    };
    let mut values = rest[..end]
        .split(',')
        .filter_map(|value| value.trim().parse::<i64>().ok());
    std::array::from_fn(|_| values.next().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREE_STATE_REPORT: &str = r#"{
  "cases": [
    { "name": "Button_normal_light", "control": "Button", "state": "normal", "theme": "light", "backdrop": "mica", "status": "pass", "invalidReason": null, "contentCoverage": 0.210000, "meanDe": 0.4200, "p95De": 1.7500, "p99De": 3.2000, "maxDe": 6.1000, "overLimitRatio": 0.005000, "referenceBbox": [16, 16, 136, 48], "oursBbox": [16, 16, 136, 48], "roiBbox": [14, 14, 138, 50], "intrinsic": [64.40, 32.00], "referenceSize": [120.00, 32.00], "intrinsicMatches": true, "cacheHit": true, "pass": true },
    { "name": "TextBox_normal_light", "control": "TextBox", "state": "normal", "theme": "light", "backdrop": "solid", "status": "invalid", "invalidReason": "reference content bbox is empty", "contentCoverage": 0.000000, "meanDe": 0.0000, "p95De": 0.0000, "p99De": 0.0000, "maxDe": 0.0000, "overLimitRatio": 0.000000, "referenceBbox": [0, 0, 0, 0], "oursBbox": [0, 0, 0, 0], "roiBbox": [0, 0, 1, 1], "intrinsic": [200.00, 36.00], "referenceSize": [200.00, 32.00], "intrinsicMatches": false, "cacheHit": false, "pass": false }
  ]
}"#;

    #[test]
    fn report_lines_expose_four_gates_backdrop_and_three_states() {
        let cases = parse_cases(THREE_STATE_REPORT);
        assert_eq!(cases.len(), 2);
        let passed = &cases[0];
        assert_eq!(passed.status, ParityStatus::Pass);
        assert_eq!(passed.backdrop, "mica");
        assert!(passed.intrinsic_matches);
        assert!(passed.cache_hit);
        assert!((passed.mean_de - 0.42).abs() < 1e-6);
        assert!((passed.p95_de - 1.75).abs() < 1e-6);
        assert!((passed.over_limit_ratio - 0.005).abs() < 1e-6);
        assert_eq!(passed.bbox_delta, [0, 0, 0, 0]);

        let invalid = &cases[1];
        assert_eq!(invalid.status, ParityStatus::Invalid);
        assert!(!invalid.intrinsic_matches);
        assert!(!invalid.cache_hit);
    }

    #[test]
    fn legacy_report_lines_without_status_fall_back_to_pass_flag() {
        let json = r#"    { "name": "Button_normal_light", "control": "Button", "state": "normal", "theme": "light", "backdrop": "solid", "meanDe": 0.1000, "p95De": 0.2000, "overLimitRatio": 0.000000, "intrinsicMatches": true, "cacheHit": false, "pass": true }"#;
        let cases = parse_cases(json);
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].status, ParityStatus::Pass);
    }
}
