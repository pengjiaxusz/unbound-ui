//! 像素一致性差分器：官方 WinUI 参考图 vs unbound-ui 渲染图（基于有效对比区 ROI 与四重门禁）。
//!
//! 输入是 `tools/winui-reference/captures`（`<控件>_<状态>_<主题>[_<后景>].png` 与同名 JSON），
//! 输出是本框架按相同边界、相同画布渲染的 PNG，以及基于**有效对比区（ROI）**的指标、热力图与 JSON 报告。
//!
//! 四重门禁判定口径（ADR 0008）：
//! 1. **内容包围盒对齐（`bbox_matches`）**：内容包围盒逐边相差 `≤ 1px`（含阴影与焦点环）；
//! 2. **固有尺寸对齐（`intrinsic_matches`）**：在未显式指定固定宽高的自由维度上，固有尺寸相差 `≤ 1px`；
//! 3. **ROI 色差均值与分位数**：仅在 `union(reference_bbox, ours_bbox, controlBounds)` 外扩 `2px` 的有效对比区内统计，`mean_de <= 1.5` 且 `p95_de <= 4.0`；
//! 4. **显著色差像素占比（`over_limit_ratio`）**：ROI 内 `ΔE2000 > 5.0` 的像素占比 `≤ 2.0%`（防止圆角、描边或图标漏画被面积稀释）。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use unbound_math::Rect;

use crate::parity::{ParityCase, ParitySession};
use crate::shot::{self, Image};

/// 有效对比区（ROI）内 ΔE2000 均值上限。
pub const MEAN_LIMIT: f64 = 1.5;
/// 有效对比区（ROI）内第 95 百分位 ΔE2000 上限。
pub const P95_LIMIT: f64 = 4.0;
/// 显著色差单像素判定阈值（用于统计 `over_limit_ratio`）。
pub const SIGNIFICANT_DE_THRESHOLD: f64 = 5.0;
/// 有效对比区内超过 [`SIGNIFICANT_DE_THRESHOLD`] 的最大允许像素占比（2%）。
pub const OVER_LIMIT_RATIO_MAX: f64 = 0.02;
/// 兼容旧常量名。
pub const P99_LIMIT: f64 = SIGNIFICANT_DE_THRESHOLD;
/// 有效对比区在联合包围盒四周额外扩展的出血像素数。
pub const ROI_MARGIN_PX: i64 = 2;
/// 有效用例要求的最小内容像素占比（0.1%），低于该值视为采集/渲染无效。
pub const MIN_CONTENT_COVERAGE: f64 = 0.001;
/// 参考宿主钉死的 WinUI 3 默认 Fluent 强调色（浅色 / 深色），与 Catalog.PinnedAccentColor 一致。
pub const REFERENCE_PINNED_ACCENT: &str = "#005FB8/#60CDFF";

/// 检查参考目录里哪些用例缺少「钉死强调色」元数据。
///
/// 旧版采集宿主没有记录 `pinnedAccent`，且没有在指纹里绑定系统强调色，
/// 因此切换系统个性化强调色后会继续复用被污染的旧图（典型症状：.checked 态出现红/紫色填充）。
/// 这里在差分前明确提示重采，避免把「缓存没清干净」误判为渲染器色值错误。
pub fn accent_stale_references(reference_dir: &str) -> Result<Vec<String>, String> {
    let mut stale = Vec::new();
    for entry in std::fs::read_dir(reference_dir)
        .map_err(|error| format!("{}: {error}", reference_dir))?
        .flatten()
    {
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext == "png")
            || path
                .file_stem()
                .is_some_and(|stem| stem.to_string_lossy().ends_with("_sheet"))
        {
            continue;
        }
        let name = path
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let meta = match std::fs::read_to_string(path.with_extension("json")) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let Some(pinned) = json_string(&meta, "pinnedAccent") else {
            stale.push(name);
            continue;
        };
        if pinned != REFERENCE_PINNED_ACCENT {
            stale.push(name);
        }
    }
    stale.sort();
    Ok(stale)
}

/// 单个用例的四态判定结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseStatus {
    /// 四重门禁全部通过且内容有效。
    Pass,
    /// 内容有效但至少一门禁未通过。
    Fail,
    /// 内容或采集本身无效，不参与通过率统计。
    Invalid,
    /// 几何、尺寸与整体色差已对齐，仅剩经视觉复核确认的 1px 边缘抗锯齿差异。
    KnownAa,
}

impl CaseStatus {
    /// 报告与终端展示用的短名称。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Invalid => "invalid",
            Self::KnownAa => "known-aa",
        }
    }
}

/// 经视觉复核、仅剩 1px 边缘抗锯齿差异的用例。
///
/// 豁免条件不是“放宽阈值”，而是同时满足：
/// - 内容包围盒与固有尺寸已对齐；
/// - `mean_de <= 2.0`，即整体颜色与几何已经一致；
/// - 差异仅体现在轮廓像素的覆盖率上。
///
/// 新增条目必须附带参考/我方放大图的复核记录，见 ADR 0010。
pub const KNOWN_AA_CASES: &[&str] = &[
    "InfoBadge_normal_light",
    "InfoBadge_normal_dark",
    "InfoBadge_disabled_light",
    "InfoBadge_disabled_dark",
    "RatingControl_normal_light",
    "RatingControl_normal_dark",
    "RatingControl_disabled_light",
    "RatingControl_disabled_dark",
];

/// `known-aa` 豁免允许的有效对比区 ΔE2000 均值上限。
///
/// 比正常门禁略宽，用于容纳 1px 轮廓覆盖差异带来的整体抬升，但仍明显低于失败区间。
pub const KNOWN_AA_MEAN_LIMIT: f64 = 2.0;

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
    /// 后景合成底板类型（`solid` / `mica` / `mica_alt` / `acrylic`）。
    pub backdrop: String,
    /// 有效对比区（ROI）ΔE2000 均值。
    pub mean_de: f64,
    /// 有效对比区（ROI）ΔE2000 第 95 百分位。
    pub p95_de: f64,
    /// 有效对比区（ROI）ΔE2000 第 99 百分位。
    pub p99_de: f64,
    /// 有效对比区（ROI）ΔE2000 最大值。
    pub max_de: f64,
    /// 有效对比区（ROI）内 ΔE 超过 [`SIGNIFICANT_DE_THRESHOLD`] 的像素占比。
    pub over_limit_ratio: f64,
    /// 参考图内容包围盒 `[min_x, min_y, max_x, max_y]`（含端点）。
    pub reference_bbox: [i64; 4],
    /// 本框架内容包围盒 `[min_x, min_y, max_x, max_y]`（含端点）。
    pub ours_bbox: [i64; 4],
    /// 实际参与色差统计的有效对比区（ROI）`[min_x, min_y, max_x, max_y]`（含端点）。
    pub roi_bbox: [i64; 4],
    /// 本框架固有尺寸 `[width, height]`。
    pub intrinsic: [f32; 2],
    /// 参考 JSON 给出的控件边界尺寸 `[width, height]`。
    pub reference_size: [f32; 2],
    /// 参考用例是否显式指定了固定宽度（若为 true，固有尺寸门禁只校验高度）。
    pub explicit_width: bool,
    /// 本次运行是否命中了本框架源码指纹缓存。
    pub cache_hit: bool,
    /// 参考图非背景像素占比。
    pub reference_content_coverage: f64,
    /// 本框架图非背景像素占比。
    pub ours_content_coverage: f64,
    /// 用例无效原因；`Some` 时判定为 [`CaseStatus::Invalid`]。
    pub invalid_reason: Option<String>,
}

impl CaseMetrics {
    /// 有效用例的内容覆盖率取两侧较小值（任一侧过低即无效）。
    #[must_use]
    pub fn content_coverage(&self) -> f64 {
        self.reference_content_coverage
            .min(self.ours_content_coverage)
    }

    /// 四态判定：无效优先于四门禁，未过门禁但属于已复核的边缘抗锯齿用例判为 [`CaseStatus::KnownAa`]。
    #[must_use]
    pub fn status(&self) -> CaseStatus {
        if self.invalid_reason.is_some() {
            CaseStatus::Invalid
        } else if self.passes() {
            CaseStatus::Pass
        } else if self.is_known_aa() {
            CaseStatus::KnownAa
        } else {
            CaseStatus::Fail
        }
    }

    /// 是否属于经视觉复核的纯边缘抗锯齿用例。
    ///
    /// 只豁免轮廓像素覆盖率：几何与整体颜色必须先对齐，且用例在 [`KNOWN_AA_CASES`] 名单内。
    #[must_use]
    pub fn is_known_aa(&self) -> bool {
        KNOWN_AA_CASES.contains(&self.name.as_str())
            && self.bbox_matches()
            && self.intrinsic_matches()
            && self.mean_de <= KNOWN_AA_MEAN_LIMIT
    }

    /// 门禁 1：内容包围盒是否逐边相等（允许 ±1px 字体亚像素抗锯齿溢出）。
    #[must_use]
    pub fn bbox_matches(&self) -> bool {
        self.reference_bbox
            .iter()
            .zip(self.ours_bbox.iter())
            .all(|(reference, ours)| (reference - ours).abs() <= 1)
    }

    /// 门禁 2：固有尺寸是否与 WinUI 自然尺寸对齐（允许 ±1px；固定宽度的控件只校验固有高度）。
    #[must_use]
    pub fn intrinsic_matches(&self) -> bool {
        let height_ok = (self.intrinsic[1] - self.reference_size[1]).abs() <= 1.0;
        let width_ok =
            self.explicit_width || (self.intrinsic[0] - self.reference_size[0]).abs() <= 1.0;
        width_ok && height_ok
    }

    /// 四重门禁是否全部通过。
    #[must_use]
    pub fn passes(&self) -> bool {
        self.bbox_matches()
            && self.intrinsic_matches()
            && self.mean_de <= MEAN_LIMIT
            && self.p95_de <= P95_LIMIT
            && self.over_limit_ratio <= OVER_LIMIT_RATIO_MAX
    }
}

/// 批量结果。
pub struct BatchSummary {
    /// 全部用例。
    pub cases: Vec<CaseMetrics>,
    /// 报告目录。
    pub out_dir: PathBuf,
    /// 命中源码指纹缓存跳过的渲染次数。
    pub cache_hits: usize,
    /// 被 `--control` / `--theme` / `--only-failed` 等筛选条件跳过的用例数。
    pub filtered_out: usize,
}

impl BatchSummary {
    /// 通过数（仅统计内容有效且四门禁全过的用例）。
    #[must_use]
    pub fn passed(&self) -> usize {
        self.cases
            .iter()
            .filter(|case| case.status() == CaseStatus::Pass)
            .count()
    }

    /// 失败数（内容有效但门禁未过）。
    #[must_use]
    pub fn failed(&self) -> usize {
        self.cases
            .iter()
            .filter(|case| case.status() == CaseStatus::Fail)
            .count()
    }

    /// 无效数（空内容、覆盖率过低或参考边界越界）。
    #[must_use]
    pub fn invalid(&self) -> usize {
        self.cases
            .iter()
            .filter(|case| case.status() == CaseStatus::Invalid)
            .count()
    }

    /// 已复核的边缘抗锯齿豁免数。
    #[must_use]
    pub fn known_aa(&self) -> usize {
        self.cases
            .iter()
            .filter(|case| case.status() == CaseStatus::KnownAa)
            .count()
    }
}

/// 差分筛选：只重渲关心的用例，其余直接跳过（不渲染、不写产物、不计入统计）。
///
/// 空列表表示「不限」，因此 `ParityFilter::default()` 等于全量差分。
/// `names` 用于 `--only-failed`：名字通常来自上一份 `report.json` 的失败清单。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParityFilter {
    /// 用例名（即参考图文件基名）。
    pub names: Vec<String>,
    pub controls: Vec<String>,
    pub states: Vec<String>,
    pub themes: Vec<String>,
    pub backdrops: Vec<String>,
}

impl ParityFilter {
    /// 是否放行了任何限制条件。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
            && self.controls.is_empty()
            && self.states.is_empty()
            && self.themes.is_empty()
            && self.backdrops.is_empty()
    }

    /// 用例是否命中筛选。
    #[must_use]
    pub fn matches(
        &self,
        name: &str,
        control: &str,
        state: &str,
        theme: &str,
        backdrop: &str,
    ) -> bool {
        fn hit(list: &[String], value: &str) -> bool {
            list.is_empty() || list.iter().any(|item| item.eq_ignore_ascii_case(value))
        }
        hit(&self.names, name)
            && hit(&self.controls, control)
            && hit(&self.states, state)
            && hit(&self.themes, theme)
            && hit(&self.backdrops, backdrop)
    }
}

/// 从上一份 `report.json` 里挑出未通过（`fail` / `invalid`）的用例名。
///
/// 报告由本模块写出，每个用例固定占一行，因此逐行读取即可，无需引入 JSON 解析器。
#[must_use]
pub fn failed_case_names(report_json: &str) -> Vec<String> {
    let mut names: Vec<String> = report_json
        .lines()
        .filter(|line| line.contains("\"name\":"))
        .filter(|line| {
            json_string(line, "status").is_none_or(|status| status == "fail" || status == "invalid")
        })
        .filter_map(|line| json_string(line, "name"))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// 对参考目录中的每个用例 PNG 渲染本框架对应控件并计算有效对比区（ROI）指标（默认启用缓存）。
pub fn run_batch(reference_dir: &str, out_dir: &str) -> Result<BatchSummary, String> {
    run_batch_with_options(reference_dir, out_dir, true)
}

/// 带缓存开关的批量差分入口（全量）。
pub fn run_batch_with_options(
    reference_dir: &str,
    out_dir: &str,
    allow_cache: bool,
) -> Result<BatchSummary, String> {
    run_batch_filtered(
        reference_dir,
        out_dir,
        allow_cache,
        &ParityFilter::default(),
    )
}

/// 带缓存开关与筛选条件的批量差分入口。
pub fn run_batch_filtered(
    reference_dir: &str,
    out_dir: &str,
    allow_cache: bool,
    filter: &ParityFilter,
) -> Result<BatchSummary, String> {
    let reference_dir = Path::new(reference_dir);
    let out_root = Path::new(out_dir);
    let ours_dir = out_root.join("ours");
    let heat_dir = out_root.join("heat");
    let reference_out = out_root.join("reference");
    std::fs::create_dir_all(&ours_dir).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&heat_dir).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&reference_out).map_err(|error| error.to_string())?;

    let mut png_files: Vec<PathBuf> = std::fs::read_dir(reference_dir)
        .map_err(|error| format!("{}: {error}", reference_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "png")
                && !path
                    .file_stem()
                    .is_some_and(|stem| stem.to_string_lossy().ends_with("_sheet"))
        })
        .collect();
    png_files.sort();
    if png_files.is_empty() {
        return Err(format!(
            "no reference PNGs found in {}",
            reference_dir.display()
        ));
    }

    // 强调色过期预警：旧参考图没有钉死色元数据，可能是切换系统强调色前的污染产物。
    // 只提示不阻断，方便诊断历史基线；真正修复必须重采。
    match accent_stale_references(&reference_dir.to_string_lossy()) {
        Ok(stale) if !stale.is_empty() => eprintln!(
            "parity: WARNING {} reference(s) missing pinnedAccent={} ({}...). \
             These may be stale captures with leaked system accent color. \
             Re-capture with: just parity-capture --no-cache --theme all",
            stale.len(),
            REFERENCE_PINNED_ACCENT,
            stale.first().map_or("", String::as_str),
        ),
        Err(error) => eprintln!("parity: accent freshness check skipped: {error}"),
        Ok(_) => {}
    }

    // 整个批量差分只创建一次 Vulkan 渲染会话
    let mut session = ParitySession::new(420, 160)?;
    let mut cases = Vec::new();
    let mut cache_hits = 0;
    let mut filtered_out = 0;

    for reference_png in png_files {
        let name = reference_png
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let meta_path = reference_png.with_extension("json");
        let meta = match std::fs::read_to_string(&meta_path) {
            Ok(text) => text,
            Err(error) => {
                session.destroy();
                return Err(format!("{}: {error}", meta_path.display()));
            }
        };
        let reference_copy = reference_out.join(format!("{name}.png"));
        if reference_copy != reference_png {
            if let Err(error) = std::fs::copy(&reference_png, &reference_copy) {
                session.destroy();
                return Err(format!("{}: {error}", reference_copy.display()));
            }
        }

        let control =
            json_string(&meta, "control").ok_or_else(|| format!("{name}: missing control"))?;
        let state = json_string(&meta, "state").ok_or_else(|| format!("{name}: missing state"))?;
        let theme = json_string(&meta, "theme").ok_or_else(|| format!("{name}: missing theme"))?;
        let backdrop = json_string(&meta, "backdrop").unwrap_or_else(|| "solid".to_string());

        // 筛选放在最前：未命中的用例不进渲染器、不写渲染产物。
        if !filter.matches(&name, &control, &state, &theme, &backdrop) {
            filtered_out += 1;
            continue;
        }

        let explicit_width =
            json_bool(&meta, "explicitWidth").unwrap_or_else(|| default_explicit_width(&control));
        let scale = json_number(&meta, "rasterizationScale").unwrap_or(1.0) as f32;
        let width = json_number(&meta, "imagePixelWidth")
            .ok_or_else(|| format!("{name}: missing imagePixelWidth"))? as u32;
        let height = json_number(&meta, "imagePixelHeight")
            .ok_or_else(|| format!("{name}: missing imagePixelHeight"))?
            as u32;
        let bounds = json_bounds(&meta).ok_or_else(|| format!("{name}: missing controlBounds"))?;

        let ours_png = ours_dir.join(format!("{name}.png"));
        let ours_png_text = ours_png.to_string_lossy().into_owned();
        let case = ParityCase::new(&control, &state, &theme).with_backdrop(&backdrop);
        let outcome = match session.render_case_cached(
            &case,
            width,
            height,
            Rect::new(bounds[0], bounds[1], bounds[2], bounds[3]),
            &ours_png_text,
            allow_cache,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                session.destroy();
                return Err(error);
            }
        };
        if outcome.cache_hit {
            cache_hits += 1;
        }

        let reference_image = shot::read_rgba_png(&reference_png)?;
        let ours_image = shot::read_rgba_png(&ours_png)?;
        if reference_image.width != ours_image.width || reference_image.height != ours_image.height
        {
            session.destroy();
            return Err(format!(
                "{name}: size mismatch {}x{} vs {}x{}",
                reference_image.width, reference_image.height, ours_image.width, ours_image.height
            ));
        }

        let reference_stats = content_stats(&reference_image);
        let ours_stats = content_stats(&ours_image);
        let reference_bbox = reference_stats.bbox.unwrap_or([0, 0, 0, 0]);
        let ours_bbox = ours_stats.bbox.unwrap_or([0, 0, 0, 0]);
        let control_px_bbox = [
            (bounds[0] * scale).floor() as i64,
            (bounds[1] * scale).floor() as i64,
            ((bounds[0] + bounds[2]) * scale).ceil() as i64,
            ((bounds[1] + bounds[3]) * scale).ceil() as i64,
        ];
        let invalid_reason = invalid_reason(
            &reference_stats,
            &ours_stats,
            control_px_bbox,
            reference_image.width,
            reference_image.height,
        );
        let roi_bbox = compute_roi_bbox(
            reference_bbox,
            ours_bbox,
            control_px_bbox,
            reference_image.width,
            reference_image.height,
        );

        let metrics = compare_roi(&reference_image, &ours_image, roi_bbox);
        write_heat(
            &heat_dir.join(format!("{name}.png")),
            &reference_image,
            &ours_image,
            roi_bbox,
        )?;

        cases.push(CaseMetrics {
            name,
            control,
            state,
            theme,
            backdrop,
            mean_de: metrics.mean_de,
            p95_de: metrics.p95_de,
            p99_de: metrics.p99_de,
            max_de: metrics.max_de,
            over_limit_ratio: metrics.over_limit_ratio,
            reference_bbox,
            ours_bbox,
            roi_bbox,
            intrinsic: [outcome.intrinsic.x, outcome.intrinsic.y],
            reference_size: [bounds[2], bounds[3]],
            explicit_width,
            cache_hit: outcome.cache_hit,
            reference_content_coverage: reference_stats.coverage,
            ours_content_coverage: ours_stats.coverage,
            invalid_reason,
        });
    }

    session.destroy();

    if cases.is_empty() && !filter.is_empty() {
        return Err(format!(
            "no reference cases matched the filter ({filtered_out} skipped) in {}",
            reference_dir.display()
        ));
    }

    let summary = BatchSummary {
        cases,
        out_dir: out_root.to_path_buf(),
        cache_hits,
        filtered_out,
    };
    write_report(&summary)?;
    Ok(summary)
}

fn default_explicit_width(control: &str) -> bool {
    matches!(control, "Button" | "Slider" | "ComboBox" | "TextBox")
}

/// 计算有效对比区（ROI）：取 `reference_bbox`、`ours_bbox` 与 `control_px_bbox` 的并集，
/// 再向外扩展 [`ROI_MARGIN_PX`] 并钳制在图像像素范围内。
#[must_use]
pub fn compute_roi_bbox(
    reference_bbox: [i64; 4],
    ours_bbox: [i64; 4],
    control_px_bbox: [i64; 4],
    width: u32,
    height: u32,
) -> [i64; 4] {
    let max_w = i64::from(width.saturating_sub(1));
    let max_h = i64::from(height.saturating_sub(1));

    let mut min_x = control_px_bbox[0];
    let mut min_y = control_px_bbox[1];
    let mut max_x = control_px_bbox[2];
    let mut max_y = control_px_bbox[3];

    for bbox in [reference_bbox, ours_bbox] {
        if bbox[2] > bbox[0] || bbox[3] > bbox[1] {
            min_x = min_x.min(bbox[0]);
            min_y = min_y.min(bbox[1]);
            max_x = max_x.max(bbox[2]);
            max_y = max_y.max(bbox[3]);
        }
    }

    [
        (min_x - ROI_MARGIN_PX).clamp(0, max_w),
        (min_y - ROI_MARGIN_PX).clamp(0, max_h),
        (max_x + ROI_MARGIN_PX).clamp(0, max_w),
        (max_y + ROI_MARGIN_PX).clamp(0, max_h),
    ]
}

/// 逐像素指标（仅在有效对比区 ROI 内统计）。
struct PixelMetrics {
    mean_de: f64,
    p95_de: f64,
    p99_de: f64,
    max_de: f64,
    over_limit_ratio: f64,
}

fn compare_roi(reference: &Image, candidate: &Image, roi: [i64; 4]) -> PixelMetrics {
    let min_x = (roi[0].max(0) as u32).min(reference.width.saturating_sub(1));
    let min_y = (roi[1].max(0) as u32).min(reference.height.saturating_sub(1));
    let max_x = (roi[2].max(0) as u32).min(reference.width.saturating_sub(1));
    let max_y = (roi[3].max(0) as u32).min(reference.height.saturating_sub(1));

    let roi_w = max_x.saturating_sub(min_x) + 1;
    let roi_h = max_y.saturating_sub(min_y) + 1;
    let mut values = Vec::with_capacity((roi_w * roi_h) as usize);
    let mut sum = 0.0_f64;
    let mut max_de = 0.0_f64;
    let mut over = 0_usize;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let delta = delta_e2000(
                rgb_to_lab(reference.rgb(x, y)),
                rgb_to_lab(candidate.rgb(x, y)),
            );
            sum += f64::from(delta);
            max_de = max_de.max(f64::from(delta));
            if f64::from(delta) > SIGNIFICANT_DE_THRESHOLD {
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

/// 单张图的内容统计：非背景像素包围盒（空图为 `None`）与内容像素占比。
struct ContentStats {
    bbox: Option<[i64; 4]>,
    coverage: f64,
}

/// 统计与左上角背景色差异超过阈值的像素：包围盒与内容覆盖率。
fn content_stats(image: &Image) -> ContentStats {
    let background = image.rgb(0, 0);
    let mut min_x = i64::MAX;
    let mut min_y = i64::MAX;
    let mut max_x = i64::MIN;
    let mut max_y = i64::MIN;
    let mut content_pixels = 0_u64;
    for y in 0..image.height {
        for x in 0..image.width {
            let pixel = image.rgb(x, y);
            let diff = (0..3)
                .map(|channel| i64::from(pixel[channel]).abs_diff(i64::from(background[channel])))
                .max()
                .unwrap_or(0);
            if diff > 2 {
                content_pixels += 1;
                min_x = min_x.min(i64::from(x));
                min_y = min_y.min(i64::from(y));
                max_x = max_x.max(i64::from(x));
                max_y = max_y.max(i64::from(y));
            }
        }
    }
    let total = u64::from(image.width) * u64::from(image.height);
    let coverage = if total == 0 {
        0.0
    } else {
        content_pixels as f64 / total as f64
    };
    ContentStats {
        bbox: (min_x != i64::MAX).then_some([min_x, min_y, max_x, max_y]),
        coverage,
    }
}

/// 汇总任一无效条件：空内容、覆盖率过低或参考 `controlBounds` 越界。
fn invalid_reason(
    reference: &ContentStats,
    ours: &ContentStats,
    control_px_bbox: [i64; 4],
    width: u32,
    height: u32,
) -> Option<String> {
    let mut reasons = Vec::new();
    for (label, stats) in [("reference", reference), ("ours", ours)] {
        if stats.bbox.is_none() {
            reasons.push(format!("{label} content bbox is empty"));
        } else if stats.coverage < MIN_CONTENT_COVERAGE {
            reasons.push(format!(
                "{label} content coverage {:.6} below {MIN_CONTENT_COVERAGE:.4}",
                stats.coverage
            ));
        }
    }
    if control_px_bbox[0] < 0
        || control_px_bbox[1] < 0
        || control_px_bbox[2] > i64::from(width)
        || control_px_bbox[3] > i64::from(height)
    {
        reasons.push(format!(
            "reference controlBounds {:?} exceed image {}x{}",
            control_px_bbox, width, height
        ));
    }
    (!reasons.is_empty()).then(|| reasons.join("; "))
}

/// 写入 ΔE 热力图：黑→蓝→绿→黄→红，`ΔE = 30` 封顶，并用暗青色 1px 框标出有效对比区（ROI）边界。
fn write_heat(
    path: &Path,
    reference: &Image,
    candidate: &Image,
    roi: [i64; 4],
) -> Result<(), String> {
    let mut rgba = vec![0_u8; (reference.width * reference.height * 4) as usize];
    let [min_x, min_y, max_x, max_y] = roi;

    for y in 0..reference.height {
        for x in 0..reference.width {
            let delta = f64::from(delta_e2000(
                rgb_to_lab(reference.rgb(x, y)),
                rgb_to_lab(candidate.rgb(x, y)),
            ));
            let mut color = heat_color(delta);
            let xi = i64::from(x);
            let yi = i64::from(y);
            let on_roi_border = ((xi == min_x || xi == max_x) && yi >= min_y && yi <= max_y)
                || ((yi == min_y || yi == max_y) && xi >= min_x && xi <= max_x);
            if on_roi_border && delta < 0.5 {
                color = [32, 96, 128];
            }
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

/// 把 `0..30` 的色差映射为黑→蓝→绿→黄→红的五段配色。
///
/// 后景对照热力图复用同一套配色，但色差来源不同；颜色映射本身必须只有一处。
pub(crate) fn heat_color(delta: f64) -> [u8; 3] {
    let t = (delta / 30.0).clamp(0.0, 1.0) as f32;
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

fn write_report(summary: &BatchSummary) -> Result<(), String> {
    let json_path = summary.out_dir.join("report.json");
    let mut json = String::from("{\n  \"cases\": [\n");
    for (index, case) in summary.cases.iter().enumerate() {
        let status = case.status();
        let invalid_reason = case
            .invalid_reason
            .as_deref()
            .map_or_else(|| "null".to_string(), |reason| format!("\"{reason}\""));
        let _ = writeln!(
            json,
            "    {{ \"name\": \"{}\", \"control\": \"{}\", \"state\": \"{}\", \"theme\": \"{}\", \"backdrop\": \"{}\", \
             \"status\": \"{}\", \"invalidReason\": {}, \"contentCoverage\": {:.6}, \
             \"meanDe\": {:.4}, \"p95De\": {:.4}, \"p99De\": {:.4}, \"maxDe\": {:.4}, \
             \"overLimitRatio\": {:.6}, \"referenceBbox\": {:?}, \"oursBbox\": {:?}, \"roiBbox\": {:?}, \
             \"intrinsic\": [{:.2}, {:.2}], \"referenceSize\": [{:.2}, {:.2}], \
             \"intrinsicMatches\": {}, \"cacheHit\": {}, \"pass\": {} }}{}",
            case.name,
            case.control,
            case.state,
            case.theme,
            case.backdrop,
            status.as_str(),
            invalid_reason,
            case.content_coverage(),
            case.mean_de,
            case.p95_de,
            case.p99_de,
            case.max_de,
            case.over_limit_ratio,
            case.reference_bbox,
            case.ours_bbox,
            case.roi_bbox,
            case.intrinsic[0],
            case.intrinsic[1],
            case.reference_size[0],
            case.reference_size[1],
            case.intrinsic_matches(),
            case.cache_hit,
            status == CaseStatus::Pass,
            if index + 1 == summary.cases.len() { "" } else { "," },
        );
    }
    json.push_str("  ]\n}\n");
    std::fs::write(&json_path, json).map_err(|error| error.to_string())
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

fn json_bool(text: &str, key: &str) -> Option<bool> {
    let rest = find_value(text, key)?;
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
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
        let text = r#"{"control":"Button","state":"normal","theme":"light","backdrop":"mica","explicitWidth":true,"imagePixelWidth":152,"controlBounds":{"x":16,"y":16,"width":120,"height":32}}"#;
        assert_eq!(json_string(text, "control").as_deref(), Some("Button"));
        assert_eq!(json_string(text, "backdrop").as_deref(), Some("mica"));
        assert_eq!(json_bool(text, "explicitWidth"), Some(true));
        assert_eq!(json_number(text, "imagePixelWidth"), Some(152.0));
        assert_eq!(json_bounds(text), Some([16.0, 16.0, 120.0, 32.0]));
    }

    #[test]
    fn roi_excludes_background_dilution() {
        // 420x160 大画布，仅中心 20x10 区域有明显色差（蓝 vs 红），其余全为相同浅灰背景。
        // 全画布均值会被稀释到 < 1.0，但有效对比区（ROI）必须抓出高色差。
        let width = 420_u32;
        let height = 160_u32;
        let mut ref_rgba = vec![243_u8; (width * height * 4) as usize];
        let mut our_rgba = vec![243_u8; (width * height * 4) as usize];
        for y in 70..80 {
            for x in 200..220 {
                let idx = ((y * width + x) * 4) as usize;
                ref_rgba[idx..idx + 4].copy_from_slice(&[0, 95, 184, 255]);
                our_rgba[idx..idx + 4].copy_from_slice(&[220, 40, 40, 255]);
            }
        }
        let ref_img = Image {
            width,
            height,
            rgba: ref_rgba,
        };
        let our_img = Image {
            width,
            height,
            rgba: our_rgba,
        };
        let ref_bbox = content_stats(&ref_img).bbox.unwrap();
        let our_bbox = content_stats(&our_img).bbox.unwrap();
        let roi = compute_roi_bbox(ref_bbox, our_bbox, [200, 70, 220, 80], width, height);
        let metrics = compare_roi(&ref_img, &our_img, roi);
        assert!(
            metrics.mean_de > 20.0,
            "ROI mean_de must not be diluted by background: {}",
            metrics.mean_de
        );
        assert!(metrics.over_limit_ratio > 0.4);
    }

    #[test]
    fn four_gates_check_intrinsic_height_and_over_limit_ratio() {
        let mut m = CaseMetrics {
            name: "Button_normal_light".into(),
            control: "Button".into(),
            state: "normal".into(),
            theme: "light".into(),
            backdrop: "solid".into(),
            mean_de: 0.4,
            p95_de: 1.2,
            p99_de: 2.0,
            max_de: 3.0,
            over_limit_ratio: 0.005,
            reference_bbox: [16, 16, 136, 48],
            ours_bbox: [16, 16, 136, 48],
            roi_bbox: [14, 14, 138, 50],
            intrinsic: [80.0, 32.0],
            reference_size: [120.0, 32.0],
            explicit_width: true,
            cache_hit: false,
            reference_content_coverage: 0.2,
            ours_content_coverage: 0.2,
            invalid_reason: None,
        };
        assert!(m.passes());
        assert_eq!(m.status(), CaseStatus::Pass);

        // 若本框架固有高度算成 36px 而 WinUI 标准高度为 32px，固有尺寸门禁必须拦截
        m.intrinsic = [80.0, 36.0];
        assert!(!m.intrinsic_matches());
        assert!(!m.passes());
        assert_eq!(m.status(), CaseStatus::Fail);
    }

    #[test]
    fn known_aa_requires_listed_case_and_aligned_geometry() {
        let mut m = CaseMetrics {
            name: "InfoBadge_normal_light".into(),
            control: "InfoBadge".into(),
            state: "normal".into(),
            theme: "light".into(),
            backdrop: "solid".into(),
            mean_de: 1.8,
            p95_de: 5.2,
            p99_de: 6.0,
            max_de: 8.0,
            over_limit_ratio: 0.031,
            reference_bbox: [16, 16, 31, 31],
            ours_bbox: [16, 16, 31, 31],
            roi_bbox: [14, 14, 33, 33],
            intrinsic: [15.0, 15.0],
            reference_size: [15.0, 15.0],
            explicit_width: false,
            cache_hit: false,
            reference_content_coverage: 0.02,
            ours_content_coverage: 0.02,
            invalid_reason: None,
        };
        assert!(!m.passes());
        assert!(m.is_known_aa());
        assert_eq!(m.status(), CaseStatus::KnownAa);

        // 未列入复核名单的用例，即使指标完全相同也必须判失败。
        m.name = "InfoBadge_hover_light".into();
        assert_eq!(m.status(), CaseStatus::Fail);

        // 名单内但整体色差超出豁免上限，仍判失败。
        m.name = "RatingControl_normal_light".into();
        m.mean_de = 2.4;
        assert!(!m.is_known_aa());
        assert_eq!(m.status(), CaseStatus::Fail);

        // 名单内但几何未对齐，豁免不生效。
        m.mean_de = 1.2;
        m.reference_bbox = [16, 24, 127, 39];
        m.ours_bbox = [16, 24, 127, 41];
        assert!(!m.is_known_aa());
    }

    /// 生成指定背景色的纯色图。
    fn solid_image(width: u32, height: u32, rgb: [u8; 3]) -> Image {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        Image {
            width,
            height,
            rgba,
        }
    }

    #[test]
    fn blank_versus_blank_is_invalid() {
        let blank = solid_image(64, 64, [243, 243, 243]);
        let reference = content_stats(&blank);
        let ours = content_stats(&blank);
        let reason = invalid_reason(&reference, &ours, [8, 8, 40, 40], 64, 64);
        assert!(
            reason
                .as_deref()
                .is_some_and(|text| text.contains("content bbox is empty")),
            "blank vs blank must be invalid: {reason:?}"
        );
    }

    #[test]
    fn one_side_blank_is_invalid() {
        let mut content = solid_image(64, 64, [243, 243, 243]);
        for y in 16..48 {
            for x in 16..48 {
                let index = ((y * 64 + x) * 4) as usize;
                content.rgba[index..index + 4].copy_from_slice(&[0, 95, 184, 255]);
            }
        }
        let reference = content_stats(&content);
        let ours = content_stats(&solid_image(64, 64, [243, 243, 243]));
        let reason = invalid_reason(&reference, &ours, [8, 8, 56, 56], 64, 64);
        assert!(
            reason
                .as_deref()
                .is_some_and(|text| text.contains("ours content bbox is empty")),
            "one blank side must be invalid: {reason:?}"
        );
    }

    #[test]
    fn control_bounds_outside_image_is_invalid() {
        let mut content = solid_image(64, 64, [243, 243, 243]);
        for y in 16..48 {
            for x in 16..48 {
                let index = ((y * 64 + x) * 4) as usize;
                content.rgba[index..index + 4].copy_from_slice(&[0, 95, 184, 255]);
            }
        }
        let stats = content_stats(&content);
        // 参考 controlBounds 向右越出 64px 画布。
        let reason = invalid_reason(&stats, &stats, [8, 8, 96, 56], 64, 64);
        assert!(
            reason
                .as_deref()
                .is_some_and(|text| text.contains("exceed image")),
            "out-of-bounds control bounds must be invalid: {reason:?}"
        );
    }

    #[test]
    fn small_but_valid_images_still_pass() {
        let mut reference = solid_image(64, 64, [243, 243, 243]);
        let mut ours = solid_image(64, 64, [243, 243, 243]);
        for y in 16..48 {
            for x in 16..48 {
                let index = ((y * 64 + x) * 4) as usize;
                reference.rgba[index..index + 4].copy_from_slice(&[0, 95, 184, 255]);
                ours.rgba[index..index + 4].copy_from_slice(&[0, 95, 184, 255]);
            }
        }
        let reference_stats = content_stats(&reference);
        let ours_stats = content_stats(&ours);
        let reason = invalid_reason(&reference_stats, &ours_stats, [16, 16, 48, 48], 64, 64);
        assert!(
            reason.is_none(),
            "valid pair must not be invalid: {reason:?}"
        );
        let metrics = compare_roi(
            &reference,
            &ours,
            compute_roi_bbox(
                reference_stats.bbox.unwrap(),
                ours_stats.bbox.unwrap(),
                [16, 16, 48, 48],
                64,
                64,
            ),
        );
        let case = CaseMetrics {
            name: "small_normal_light".into(),
            control: "small".into(),
            state: "normal".into(),
            theme: "light".into(),
            backdrop: "solid".into(),
            mean_de: metrics.mean_de,
            p95_de: metrics.p95_de,
            p99_de: metrics.p99_de,
            max_de: metrics.max_de,
            over_limit_ratio: metrics.over_limit_ratio,
            reference_bbox: reference_stats.bbox.unwrap(),
            ours_bbox: ours_stats.bbox.unwrap(),
            roi_bbox: [16, 16, 48, 48],
            intrinsic: [32.0, 32.0],
            reference_size: [32.0, 32.0],
            explicit_width: true,
            cache_hit: false,
            reference_content_coverage: reference_stats.coverage,
            ours_content_coverage: ours_stats.coverage,
            invalid_reason: reason,
        };
        assert_eq!(case.status(), CaseStatus::Pass);
    }

    #[test]
    fn accent_stale_detector_flags_missing_and_mismatched_pinned_accent() {
        // 固定临时目录名，避免并行测试互相覆盖。
        let dir = std::env::temp_dir().join("unbound_png_diff_accent_stale_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        for (name, meta) in [
            ("fresh", r##"{"pinnedAccent": "#005FB8/#60CDFF"}"##),
            ("mismatch", r##"{"pinnedAccent": "#FF0000/#FF8080"}"##),
            ("legacy", r##"{"fingerprint": "old"}"##),
        ] {
            std::fs::write(dir.join(format!("{name}.png")), b"dummy").unwrap();
            std::fs::write(dir.join(format!("{name}.json")), meta).unwrap();
        }

        let stale = super::accent_stale_references(&dir.to_string_lossy()).unwrap();
        assert_eq!(stale, vec!["legacy".to_string(), "mismatch".to_string()]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn default_filter_admits_every_case() {
        let filter = super::ParityFilter::default();
        assert!(filter.is_empty());
        assert!(filter.matches("Button_normal_light", "Button", "normal", "light", "solid"));
    }

    #[test]
    fn filter_narrows_by_control_and_state_without_case_sensitivity() {
        let filter = super::ParityFilter {
            controls: vec!["button".to_string()],
            states: vec!["normal".to_string()],
            ..Default::default()
        };
        assert!(!filter.is_empty());
        assert!(filter.matches("Button_normal_light", "Button", "normal", "light", "solid"));
        assert!(!filter.matches(
            "CheckBox_normal_light",
            "CheckBox",
            "normal",
            "light",
            "solid"
        ));
        assert!(!filter.matches(
            "Button_disabled_light",
            "Button",
            "disabled",
            "light",
            "solid"
        ));
    }

    #[test]
    fn only_failed_names_skips_passing_and_known_aa_cases() {
        let report = r#"{
  "cases": [
    { "name": "Button_normal_light", "control": "Button", "status": "pass" },
    { "name": "Button_disabled_dark", "control": "Button", "status": "fail" },
    { "name": "RatingControl_normal_light", "control": "RatingControl", "status": "known-aa" },
    { "name": "Image_normal_light", "control": "Image", "status": "invalid" }
  ]
}"#;
        assert_eq!(
            super::failed_case_names(report),
            vec![
                "Button_disabled_dark".to_string(),
                "Image_normal_light".to_string()
            ]
        );
    }
}
