//! 像素一致性差分器：官方 WinUI 参考图 vs unbound-ui 渲染图。
//!
//! 输入是 `tools/winui-reference/captures`（`<控件>_<状态>_<主题>.png` 与同名 JSON），
//! 输出是本框架按相同边界、相同画布渲染的 PNG，以及分区指标、热力图与 HTML 报告。
//!
//! 判定口径（与项目约定一致）：
//! * 几何——内容包围盒必须与参考图完全一致（阴影会一并计入包围盒）；
//! * 颜色——控制区之外以整体 ΔE2000 均值与 p99 衡量，文字抗锯齿允许少量离群像素。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use unbound_math::Rect;

use crate::parity::{self, ParityCase};
use crate::shot::{self, Image};

/// 整体 ΔE2000 均值上限。
pub const MEAN_LIMIT: f64 = 1.0;
/// 第 99 百分位 ΔE2000 上限（文字抗锯齿允许少量离群）。
pub const P99_LIMIT: f64 = 3.0;

/// 单个用例的对比结果。
pub struct CaseMetrics {
    /// 用例名（不含扩展名）。
    pub name: String,
    /// 控件名。
    pub control: String,
    /// 状态。
    pub state: String,
    /// 主题。
    pub theme: String,
    /// 全图 ΔE2000 均值。
    pub mean_de: f64,
    /// ΔE2000 第 95 百分位。
    pub p95_de: f64,
    /// ΔE2000 第 99 百分位。
    pub p99_de: f64,
    /// ΔE2000 最大值。
    pub max_de: f64,
    /// ΔE 超过 [`P99_LIMIT`] 的像素占比。
    pub over_limit_ratio: f64,
    /// 参考图内容包围盒 `[min_x, min_y, max_x, max_y]`（含端点）。
    pub reference_bbox: [i64; 4],
    /// 本框架内容包围盒。
    pub ours_bbox: [i64; 4],
    /// 本框架固有尺寸。
    pub intrinsic: [f32; 2],
    /// 参考 JSON 给出的控件边界尺寸。
    pub reference_size: [f32; 2],
}

impl CaseMetrics {
    /// 包围盒是否逐边相等。
    #[must_use]
    pub fn bbox_matches(&self) -> bool {
        self.reference_bbox == self.ours_bbox
    }

    /// 是否达到验收阈值。
    #[must_use]
    pub fn passes(&self) -> bool {
        self.bbox_matches() && self.mean_de <= MEAN_LIMIT && self.p99_de <= P99_LIMIT
    }
}

/// 批量结果。
pub struct BatchSummary {
    /// 全部用例。
    pub cases: Vec<CaseMetrics>,
    /// 报告目录。
    pub out_dir: PathBuf,
}

impl BatchSummary {
    /// 通过数。
    #[must_use]
    pub fn passed(&self) -> usize {
        self.cases.iter().filter(|case| case.passes()).count()
    }
}

/// 对参考目录中的每个 PNG 渲染本框架对应控件并计算指标。
pub fn run_batch(reference_dir: &str, out_dir: &str) -> Result<BatchSummary, String> {
    let reference_dir = Path::new(reference_dir);
    let out_root = Path::new(out_dir);
    let ours_dir = out_root.join("ours");
    let heat_dir = out_root.join("heat");
    // 参考截图一并复制进产物目录：框架内报告页只依赖 `target/parity`，
    // 不再需要回头读取采集目录，HTML 报告也能独立分发。
    let reference_out = out_root.join("reference");
    std::fs::create_dir_all(&ours_dir).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&heat_dir).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&reference_out).map_err(|error| error.to_string())?;

    let mut png_files: Vec<PathBuf> = std::fs::read_dir(reference_dir)
        .map_err(|error| format!("{}: {error}", reference_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    png_files.sort();
    if png_files.is_empty() {
        return Err(format!(
            "no reference PNGs found in {}",
            reference_dir.display()
        ));
    }

    let mut cases = Vec::new();
    for reference_png in png_files {
        let name = reference_png
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let meta_path = reference_png.with_extension("json");
        let meta = std::fs::read_to_string(&meta_path)
            .map_err(|error| format!("{}: {error}", meta_path.display()))?;
        let reference_copy = reference_out.join(format!("{name}.png"));
        if reference_copy != reference_png {
            std::fs::copy(&reference_png, &reference_copy)
                .map_err(|error| format!("{}: {error}", reference_copy.display()))?;
        }

        let control =
            json_string(&meta, "control").ok_or_else(|| format!("{name}: missing control"))?;
        let state = json_string(&meta, "state").ok_or_else(|| format!("{name}: missing state"))?;
        let theme = json_string(&meta, "theme").ok_or_else(|| format!("{name}: missing theme"))?;
        let width = json_number(&meta, "imagePixelWidth")
            .ok_or_else(|| format!("{name}: missing imagePixelWidth"))? as u32;
        let height = json_number(&meta, "imagePixelHeight")
            .ok_or_else(|| format!("{name}: missing imagePixelHeight"))?
            as u32;
        let bounds = json_bounds(&meta).ok_or_else(|| format!("{name}: missing controlBounds"))?;

        let ours_png = ours_dir.join(format!("{name}.png"));
        let ours_png_text = ours_png.to_string_lossy().into_owned();
        let case = ParityCase {
            control: &control,
            state: &state,
            theme: &theme,
        };
        let intrinsic = parity::render_case(
            &case,
            width,
            height,
            Rect::new(bounds[0], bounds[1], bounds[2], bounds[3]),
            &ours_png_text,
        )?;

        let reference_image = shot::read_rgba_png(&reference_png)?;
        let ours_image = shot::read_rgba_png(&ours_png)?;
        if reference_image.width != ours_image.width || reference_image.height != ours_image.height
        {
            return Err(format!(
                "{name}: size mismatch {}x{} vs {}x{}",
                reference_image.width, reference_image.height, ours_image.width, ours_image.height
            ));
        }

        let metrics = compare(&reference_image, &ours_image);
        write_heat(
            &heat_dir.join(format!("{name}.png")),
            &reference_image,
            &ours_image,
        )?;

        cases.push(CaseMetrics {
            name,
            control,
            state,
            theme,
            mean_de: metrics.mean_de,
            p95_de: metrics.p95_de,
            p99_de: metrics.p99_de,
            max_de: metrics.max_de,
            over_limit_ratio: metrics.over_limit_ratio,
            reference_bbox: content_bbox(&reference_image),
            ours_bbox: content_bbox(&ours_image),
            intrinsic: [intrinsic.x, intrinsic.y],
            reference_size: [bounds[2], bounds[3]],
        });
    }

    let summary = BatchSummary {
        cases,
        out_dir: out_root.to_path_buf(),
    };
    write_reports(&summary, &ours_dir, &heat_dir, &reference_out)?;
    Ok(summary)
}

/// 逐像素指标。
struct PixelMetrics {
    mean_de: f64,
    p95_de: f64,
    p99_de: f64,
    max_de: f64,
    over_limit_ratio: f64,
}

fn compare(reference: &Image, candidate: &Image) -> PixelMetrics {
    let mut values = Vec::with_capacity((reference.width * reference.height) as usize);
    let mut sum = 0.0_f64;
    let mut max_de = 0.0_f64;
    let mut over = 0_usize;
    for y in 0..reference.height {
        for x in 0..reference.width {
            let delta = delta_e2000(
                rgb_to_lab(reference.rgb(x, y)),
                rgb_to_lab(candidate.rgb(x, y)),
            );
            sum += f64::from(delta);
            max_de = max_de.max(f64::from(delta));
            if f64::from(delta) > P99_LIMIT {
                over += 1;
            }
            values.push(delta);
        }
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let total = values.len().max(1);
    PixelMetrics {
        mean_de: sum / total as f64,
        p95_de: f64::from(values[(total * 95 / 100).min(total - 1)]),
        p99_de: f64::from(values[(total * 99 / 100).min(total - 1)]),
        max_de,
        over_limit_ratio: over as f64 / total as f64,
    }
}

/// 内容包围盒：与左上角背景色差异超过阈值的最外像素。
fn content_bbox(image: &Image) -> [i64; 4] {
    let background = image.rgb(0, 0);
    let mut min_x = i64::MAX;
    let mut min_y = i64::MAX;
    let mut max_x = i64::MIN;
    let mut max_y = i64::MIN;
    for y in 0..image.height {
        for x in 0..image.width {
            let pixel = image.rgb(x, y);
            let diff = (0..3)
                .map(|channel| i64::from(pixel[channel]).abs_diff(i64::from(background[channel])))
                .max()
                .unwrap_or(0);
            if diff > 2 {
                min_x = min_x.min(i64::from(x));
                min_y = min_y.min(i64::from(y));
                max_x = max_x.max(i64::from(x));
                max_y = max_y.max(i64::from(y));
            }
        }
    }
    if min_x == i64::MAX {
        [0, 0, 0, 0]
    } else {
        [min_x, min_y, max_x, max_y]
    }
}

/// 写入 ΔE 热力图：黑→蓝→绿→黄→红，`ΔE = 30` 封顶。
fn write_heat(path: &Path, reference: &Image, candidate: &Image) -> Result<(), String> {
    let mut rgba = vec![0_u8; (reference.width * reference.height * 4) as usize];
    for y in 0..reference.height {
        for x in 0..reference.width {
            let delta = f64::from(delta_e2000(
                rgb_to_lab(reference.rgb(x, y)),
                rgb_to_lab(candidate.rgb(x, y)),
            ));
            let color = heat_color(delta);
            let index = ((y * reference.width + x) * 4) as usize;
            rgba[index..index + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
        }
    }
    shot::write_rgba_png(
        &path.to_string_lossy(),
        reference.width,
        reference.height,
        &rgba,
    )
}

fn heat_color(delta: f64) -> [u8; 3] {
    let t = (delta / 30.0).clamp(0.0, 1.0) as f32;
    // 五段线性插值：黑 → 蓝 → 绿 → 黄 → 红。
    const STOPS: [[f32; 3]; 5] = [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let scaled = t * (STOPS.len() - 1) as f32;
    let index = (scaled.floor() as usize).min(STOPS.len() - 2);
    let local = scaled - index as f32;
    let mut color = [0_u8; 3];
    for channel in 0..3 {
        let value =
            STOPS[index][channel] + (STOPS[index + 1][channel] - STOPS[index][channel]) * local;
        color[channel] = (value * 255.0).round() as u8;
    }
    color
}

fn write_reports(
    summary: &BatchSummary,
    ours_dir: &Path,
    heat_dir: &Path,
    reference_dir: &Path,
) -> Result<(), String> {
    let json_path = summary.out_dir.join("report.json");
    let mut json = String::from("{\n  \"cases\": [\n");
    for (index, case) in summary.cases.iter().enumerate() {
        let _ =
            write!(
            json,
            "    {{ \"name\": \"{}\", \"control\": \"{}\", \"state\": \"{}\", \"theme\": \"{}\", \
             \"meanDe\": {:.4}, \"p95De\": {:.4}, \"p99De\": {:.4}, \"maxDe\": {:.4}, \
             \"overLimitRatio\": {:.6}, \"referenceBbox\": {:?}, \"oursBbox\": {:?}, \
             \"intrinsic\": [{:.2}, {:.2}], \"referenceSize\": [{:.2}, {:.2}], \"pass\": {} }}{}\n",
            case.name,
            case.control,
            case.state,
            case.theme,
            case.mean_de,
            case.p95_de,
            case.p99_de,
            case.max_de,
            case.over_limit_ratio,
            case.reference_bbox,
            case.ours_bbox,
            case.intrinsic[0],
            case.intrinsic[1],
            case.reference_size[0],
            case.reference_size[1],
            case.passes(),
            if index + 1 == summary.cases.len() { "" } else { "," },
        );
    }
    json.push_str("  ]\n}\n");
    std::fs::write(&json_path, json).map_err(|error| error.to_string())?;

    let html_path = summary.out_dir.join("report.html");
    std::fs::write(
        &html_path,
        render_html(summary, ours_dir, heat_dir, reference_dir),
    )
    .map_err(|error| error.to_string())
}

fn render_html(
    summary: &BatchSummary,
    ours_dir: &Path,
    heat_dir: &Path,
    reference_dir: &Path,
) -> String {
    let mut html = String::from(
        "<!doctype html><meta charset=\"utf-8\"><title>unbound-ui parity</title>\n\
         <style>\n\
         body{font:13px/1.5 'Segoe UI',sans-serif;margin:24px;background:#fafafa;color:#1a1a1a}\n\
         h1{font-size:20px;margin:0 0 4px} .meta{color:#5c5c5c;margin-bottom:20px}\n\
         table{border-collapse:collapse;margin-bottom:28px} th,td{padding:4px 10px;border-bottom:1px solid #e3e3e3;text-align:right}\n\
         th:first-child,td:first-child{text-align:left} .pass{color:#0f7b0f;font-weight:600} .fail{color:#c42b1c;font-weight:600}\n\
         .case{margin:18px 0;padding:12px;background:#fff;border:1px solid #e3e3e3;border-radius:6px}\n\
         .case h2{font-size:14px;margin:0 0 8px} .shots{display:flex;gap:12px;align-items:flex-start;flex-wrap:wrap}\n\
         figure{margin:0} figcaption{color:#5c5c5c;font-size:11px;margin-top:2px}\n\
         img{image-rendering:pixelated;border:1px solid #d6d6d6;background:#fff}\n\
         </style>\n",
    );
    let _ = write!(
        html,
        "<h1>unbound-ui parity report</h1><div class=\"meta\">{} / {} cases pass · mean ΔE2000 ≤ {:.1}, p99 ≤ {:.1}, bbox exact</div>\n",
        summary.passed(),
        summary.cases.len(),
        MEAN_LIMIT,
        P99_LIMIT
    );

    html.push_str("<table><tr><th>case</th><th>mean ΔE</th><th>p95</th><th>p99</th><th>max</th><th>&gt;3 ratio</th><th>bbox Δ</th><th>intrinsic</th><th>status</th></tr>\n");
    for case in &summary.cases {
        let delta: Vec<i64> = (0..4)
            .map(|index| case.ours_bbox[index] - case.reference_bbox[index])
            .collect();
        let status = if case.passes() {
            "<span class=\"pass\">pass</span>"
        } else {
            "<span class=\"fail\">fail</span>"
        };
        let _ = write!(
            html,
            "<tr><td>{}</td><td>{:.2}</td><td>{:.2}</td><td>{:.2}</td><td>{:.1}</td><td>{:.2}%</td><td>{:?}</td><td>{:.0}×{:.0} (ref {:.0}×{:.0})</td><td>{}</td></tr>\n",
            case.name,
            case.mean_de,
            case.p95_de,
            case.p99_de,
            case.max_de,
            case.over_limit_ratio * 100.0,
            delta,
            case.intrinsic[0],
            case.intrinsic[1],
            case.reference_size[0],
            case.reference_size[1],
            status,
        );
    }
    html.push_str("</table>\n");

    for case in &summary.cases {
        let reference_png = reference_dir.join(format!("{}.png", case.name));
        let ours_png = ours_dir.join(format!("{}.png", case.name));
        let heat_png = heat_dir.join(format!("{}.png", case.name));
        let _ = write!(
            html,
            "<div class=\"case\"><h2>{name} · mean {mean:.2} · p99 {p99:.2} · bbox {bbox:?}</h2><div class=\"shots\">\
             <figure><img src=\"{ref_url}\" width=\"420\" height=\"160\"><figcaption>WinUI 3 reference</figcaption></figure>\
             <figure><img src=\"{ours_url}\" width=\"420\" height=\"160\"><figcaption>unbound-ui</figcaption></figure>\
             <figure><img src=\"{heat_url}\" width=\"420\" height=\"160\"><figcaption>ΔE heatmap</figcaption></figure>\
             </div></div>\n",
            name = case.name,
            mean = case.mean_de,
            p99 = case.p99_de,
            bbox = case.ours_bbox,
            ref_url = png_data_url(&reference_png),
            ours_url = png_data_url(&ours_png),
            heat_url = png_data_url(&heat_png),
        );
    }

    html
}

/// 把 PNG 内联成 data URI：报告单文件可移动，浏览器不会拦截本地子资源。
fn png_data_url(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => format!("data:image/png;base64,{}", base64(&bytes)),
        Err(error) => format!("missing: {error}"),
    }
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(char::from(ALPHABET[(first >> 2) as usize]));
        encoded.push(char::from(
            ALPHABET[(((first & 0b11) << 4) | (second >> 4)) as usize],
        ));
        encoded.push(if chunk.len() > 1 {
            char::from(ALPHABET[(((second & 0b1111) << 2) | (third >> 6)) as usize])
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            char::from(ALPHABET[(third & 0b11_1111) as usize])
        } else {
            '='
        });
    }
    encoded
}

// ============================================================================
// 宽松 JSON 读取：参考元数据由本项目生成，字段固定，无需引入完整解析器。
// ============================================================================

fn json_string(text: &str, key: &str) -> Option<String> {
    let rest = find_value(text, key)?;
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn json_number(text: &str, key: &str) -> Option<f64> {
    let rest = find_value(text, key)?;
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

fn json_bounds(text: &str) -> Option<[f32; 4]> {
    let rest = find_value(text, "controlBounds")?;
    Some([
        json_number(rest, "x")? as f32,
        json_number(rest, "y")? as f32,
        json_number(rest, "width")? as f32,
        json_number(rest, "height")? as f32,
    ])
}

fn find_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    Some(text[start..].trim_start())
}

// ============================================================================
// 色差：sRGB → CIE Lab (D65) → CIEDE2000
// ============================================================================

fn rgb_to_lab(rgb: [u8; 3]) -> [f32; 3] {
    let linear = rgb.map(|channel| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    let x = 0.412_456_4 * linear[0] + 0.357_576_1 * linear[1] + 0.180_437_5 * linear[2];
    let y = 0.212_672_9 * linear[0] + 0.715_152_2 * linear[1] + 0.072_175 * linear[2];
    let z = 0.019_333_9 * linear[0] + 0.119_192 * linear[1] + 0.950_304_1 * linear[2];
    let f = |value: f32| -> f32 {
        const EPSILON: f32 = 216.0 / 24_389.0;
        if value > EPSILON {
            value.cbrt()
        } else {
            (841.0 / 108.0) * value + 4.0 / 29.0
        }
    };
    let fx = f(x / 0.950_47);
    let fy = f(y);
    let fz = f(z / 1.088_83);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn delta_e2000(first: [f32; 3], second: [f32; 3]) -> f32 {
    let [l1, a1, b1] = first;
    let [l2, a2, b2] = second;
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let c_bar = (c1 + c2) * 0.5;
    let g = 0.5 * (1.0 - (c_bar.powi(7) / (c_bar.powi(7) + 25_f32.powi(7))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let h1p = if a1p == 0.0 && b1 == 0.0 {
        0.0
    } else {
        b1.atan2(a1p).to_degrees().rem_euclid(360.0)
    };
    let h2p = if a2p == 0.0 && b2 == 0.0 {
        0.0
    } else {
        b2.atan2(a2p).to_degrees().rem_euclid(360.0)
    };
    let delta_l = l2 - l1;
    let delta_c = c2p - c1p;
    let delta_angle = if c1p * c2p == 0.0 {
        0.0
    } else {
        let raw = h2p - h1p;
        if raw.abs() <= 180.0 {
            raw
        } else if raw > 180.0 {
            raw - 360.0
        } else {
            raw + 360.0
        }
    };
    let delta_h = 2.0 * (c1p * c2p).sqrt() * (delta_angle.to_radians() * 0.5).sin();
    let l_bar = (l1 + l2) * 0.5;
    let c_bar_p = (c1p + c2p) * 0.5;
    let h_bar = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) * 0.5
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) * 0.5
    } else {
        (h1p + h2p - 360.0) * 0.5
    };
    let t = 1.0 - 0.17 * (h_bar - 30.0).to_radians().cos()
        + 0.24 * (2.0 * h_bar).to_radians().cos()
        + 0.32 * (3.0 * h_bar + 6.0).to_radians().cos()
        - 0.20 * (4.0 * h_bar - 63.0).to_radians().cos();
    let delta_theta = 30.0 * (-((h_bar - 275.0) / 25.0).powi(2)).exp();
    let r_c = 2.0 * (c_bar_p.powi(7) / (c_bar_p.powi(7) + 25_f32.powi(7))).sqrt();
    let s_l = 1.0 + 0.015 * (l_bar - 50.0).powi(2) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let s_c = 1.0 + 0.045 * c_bar_p;
    let s_h = 1.0 + 0.015 * c_bar_p * t;
    let r_t = -(2.0 * delta_theta).to_radians().sin() * r_c;
    let l_term = delta_l / s_l;
    let c_term = delta_c / s_c;
    let h_term = delta_h / s_h;
    (l_term * l_term + c_term * c_term + h_term * h_term + r_t * c_term * h_term).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_colors_have_zero_delta() {
        let accent = rgb_to_lab([0, 95, 184]);
        assert!(delta_e2000(accent, accent) < 1e-4);
    }

    #[test]
    fn black_and_white_are_one_hundred_delta_e() {
        let delta = delta_e2000(rgb_to_lab([0, 0, 0]), rgb_to_lab([255, 255, 255]));
        assert!((delta - 100.0).abs() < 0.5, "unexpected delta: {delta}");
    }

    #[test]
    fn reference_metadata_is_parsed() {
        let text = r#"{"control":"Button","state":"normal","theme":"light","imagePixelWidth":420,"controlBounds":{"x":150,"y":64,"width":120,"height":32}}"#;
        assert_eq!(json_string(text, "control").as_deref(), Some("Button"));
        assert_eq!(json_number(text, "imagePixelWidth"), Some(420.0));
        assert_eq!(json_bounds(text), Some([150.0, 64.0, 120.0, 32.0]));
    }
}
