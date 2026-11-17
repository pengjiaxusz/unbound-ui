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
    description: "Every pilot control compared against its WinUI 3 capture: the reference, this framework's render at the same bounds, and the CIEDE2000 heatmap, judged by mean, p99 and exact content bounds.",
    crates: &["tools/winui-reference", "apps/showcase/src/png_diff.rs"],
    keywords: "winui parity delta e screenshot pixel comparison heatmap reference",
};

pub const PARITY_EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "Reference / unbound-ui / ΔE",
    "Each row holds three captures of the same control: the WinUI 3 reference, the unbound-ui render at identical bounds, and the CIEDE2000 heatmap. A row fails when mean ΔE, p99 or the content box drifts.",
    &["tools/winui-reference", "apps/showcase/src/png_diff.rs"],
    r#"showcase --parity-run --reference tools/winui-reference/captures --out target/parity"#,
    "A scrollable parity report rendered by the gallery itself; every case exposes its name, pass state, colour metrics and three captures.",
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
/// 参考截图的固有宽高比（420×160）。
const THUMB_ASPECT: f32 = 420.0 / 160.0;
/// 三张缩略图之间的间距。
const THUMB_GAP: f32 = 10.0;

/// 单个对照用例。
#[derive(Debug)]
pub struct ParityCase {
    pub name: String,
    pub control: String,
    pub state: String,
    pub theme: String,
    pub mean_de: f32,
    pub p99_de: f32,
    /// 本框架包围盒相对参考包围盒的逐边差值。
    pub bbox_delta: [i64; 4],
    pub pass: bool,
    /// 参考 / 本框架 / 热力图三张纹理。
    pub images: [Option<AssetId>; 3],
}

/// 已载入的对照报告。
#[derive(Debug)]
pub struct ParityReport {
    pub cases: Vec<ParityCase>,
    pub passed: usize,
    /// 载入失败原因；为 `Some` 时页面显示空态说明。
    pub error: Option<String>,
}

impl ParityReport {
    /// 读取 `report.json` 并提交全部截图进行异步解码。
    #[must_use]
    pub fn load(hub: &mut AssetHub) -> Self {
        let root = Path::new(REPORT_DIR);
        let json = match std::fs::read_to_string(root.join("report.json")) {
            Ok(text) => text,
            Err(error) => {
                return Self {
                    cases: Vec::new(),
                    passed: 0,
                    error: Some(format!("{REPORT_DIR}/report.json: {error}")),
                }
            }
        };
        let reference = reference_dir(root);
        let ours = root.join("ours");
        let heat = root.join("heat");
        let mut cases = Vec::new();
        let mut passed = 0;
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
            if pass {
                passed += 1;
            }
            let images = std::array::from_fn(|which| {
                let dir: &Path = match which {
                    0 => &reference,
                    1 => &ours,
                    _ => &heat,
                };
                std::fs::read(dir.join(format!("{name}.png")))
                    .ok()
                    .map(|bytes| hub.request_png(bytes))
            });
            cases.push(ParityCase {
                name,
                control: string_value(line, "control").unwrap_or_default(),
                state: string_value(line, "state").unwrap_or_default(),
                theme: string_value(line, "theme").unwrap_or_default(),
                mean_de: number_value(line, "meanDe").unwrap_or(0.0) as f32,
                p99_de: number_value(line, "p99De").unwrap_or(0.0) as f32,
                bbox_delta,
                pass,
                images,
            });
        }
        if cases.is_empty() {
            return Self {
                cases,
                passed,
                error: Some(format!("{REPORT_DIR}/report.json 中没有用例")),
            };
        }
        Self {
            cases,
            passed,
            error: None,
        }
    }
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
                "未找到可用的差分产物：{error}\n先运行 `showcase --parity-run --reference tools\\winui-reference\\captures --out target\\parity`，再回到本页。"
            )
        } else {
            format!(
                "No parity artifacts were found: {error}\nRun `showcase --parity-run --reference tools\\winui-reference\\captures --out target/parity` first, then reopen this page."
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
    }

    let palette = *ctx.canvas.palette;
    let visible_top = ctx.canvas.viewport.origin.y - 48.0;
    let visible_bottom = ctx.canvas.viewport.max().y + 48.0;

    // 概要横幅：通过数与判定阈值。
    let summary = Rect::new(rect.origin.x, rect.origin.y, rect.size.x, 64.0);
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
            "像素对照 · WinUI 3 vs unbound-ui"
        } else {
            "Pixel parity · WinUI 3 vs unbound-ui"
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
            "{} / {} {} · mean ΔE ≤ 1.0 · p99 ≤ 3.0 · {}",
            report.passed,
            report.cases.len(),
            if zh { "通过" } else { "pass" },
            if zh {
                "包围盒逐边一致"
            } else {
                "exact content bounds"
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

    let inner_width = (rect.size.x - 24.0).max(1.0);
    let scale = (inner_width / (3.0 * THUMB_MAX_WIDTH + 2.0 * THUMB_GAP)).clamp(0.25, 1.0);
    let image_height = THUMB_MAX_WIDTH * scale / THUMB_ASPECT;
    let row_height = 64.0 + image_height;
    let mut y = summary.max().y + 12.0;
    for (index, case) in report.cases.iter().enumerate() {
        let row = Rect::new(rect.origin.x, y, rect.size.x, row_height);
        y += row_height + 8.0;
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
    let (status_text, status_color, status_bg) = if case.pass {
        (
            if zh { "通过" } else { "pass" },
            palette.success,
            palette.success_soft,
        )
    } else {
        (
            if zh { "未通过" } else { "fail" },
            palette.danger,
            palette.danger_soft,
        )
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
        format!("{} · {} · {}", case.control, case.state, case.theme)
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
            "mean ΔE {:.2} · p99 {:.2} · {} [{}, {}, {}, {}]",
            case.mean_de,
            case.p99_de,
            if zh { "包围盒Δ" } else { "bboxΔ" },
            case.bbox_delta[0],
            case.bbox_delta[1],
            case.bbox_delta[2],
            case.bbox_delta[3]
        ),
        Rect::new(
            row.origin.x + row.size.x * 0.35,
            row.origin.y + 9.0,
            row.size.x * 0.65 - badge_width - 40.0,
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
    for which in 0..3 {
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
        let Some(id) = case.images[which] else {
            continue;
        };
        let part = 100 + (index as u16) * 4 + which as u16;
        let node = ctx.part(part);
        let mut desc = ImageDesc::new(thumb, Vec2::new(420.0, 160.0), 0).with_asset(id);
        desc.fit = ObjectFit::Fill;
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
                    "ΔE2000 热力图"
                } else {
                    "ΔE2000 heatmap"
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
