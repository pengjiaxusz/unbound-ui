//! WinUI 3 像素对照检视台（Parity Inspector，ADR 0035）：
//! lang-allow: bilingual page metadata; Chinese is the zh-CN half
//!
//! 采用左列表 + 右舞台的主从式（Master-Detail）布局：
//! - 顶部汇总条 + 状态筛选胶囊（存在失败/无效项时默认聚焦失败项）+ Theme/Backdrop/排序过滤；
//! - 左侧紧凑用例列表（支持滚轮与 `↑`/`↓` 键盘切换）；
//! - 右侧大尺寸检视舞台：支持一键切换并排（Side-by-Side）、卷帘（Curtain，鼠标悬停跟随 +
//!   点击锁定分界线）与叠加（Overlay，本框架/热力图切换与比例调节），并支持 `1x / 2x / 4x` 缩放。

use std::path::{Path, PathBuf};

use accesskit::Role;
use unbound_app::AssetHub;
use unbound_asset::AssetId;
use unbound_input::CursorKind;
use unbound_math::{Rect, Vec2};
use unbound_widgets::image::{emit_image_with_assets, ImageDesc, ObjectFit};
use unbound_widgets::{ControlState, WidgetCatalogId};

use super::{DemoCtx, DemoKind, ExampleDef, PageId, PageInfo};
use crate::canvas::{AlignX, AlignY, Canvas};
use crate::state::GalleryState;
use crate::target::{self, Target};
use crate::widgets::{self, ButtonVariant};

pub const PARITY_INFO: PageInfo = PageInfo {
    title: "WinUI 3 pixel parity",
    short: "Pixel parity",
    description: "Master-detail parity inspector with status/theme/backdrop filtering, ΔE sorting, and Side-by-Side / Curtain / Overlay comparison views.",
    crates: &["tools/winui-reference", "apps/showcase/src/png_diff.rs"],
    keywords: "winui parity delta e screenshot pixel comparison heatmap reference roi curtain overlay filter",
};

pub const PARITY_EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "Reference / unbound-ui / ROI ΔE",
    "Interactive master-detail parity inspector rendered without ExampleCard chrome.",
    &["tools/winui-reference", "apps/showcase/src/png_diff.rs"],
    r#"just parity diff --only-failed"#,
    "Master-detail parity report with status filter, sorting, Side-by-Side, Curtain wipe, and Overlay comparison views.",
    &[],
    DemoKind::ParityReport,
)
.with_height(640.0)];

/// 差分产物目录（与 `--parity-run` 的默认输出一致）。
const REPORT_DIR: &str = "target/parity";
/// 旧版参考截图的回退目录（报告尚未打包参考图时使用）。
const REFERENCE_FALLBACK: &str = "tools/winui-reference/captures";

// ============================================================================
// 交互部件 ID 空间（统一走 Target::Demo(PageId::Parity, 0, part)）
// ============================================================================

/// 刷新报告按钮。
pub const REFRESH_PART: u16 = 9_000;
/// 状态筛选胶囊基址（`0..=5`：All / Issues / Fail / Invalid / Exempt / Pass）。
pub const STATUS_FILTER_BASE: u16 = 9_010;
/// 循环切换 Theme 筛选（All -> Light -> Dark）。
pub const THEME_FILTER_PART: u16 = 9_020;
/// 循环切换 Backdrop 筛选（All -> Solid -> Mica -> Acrylic）。
pub const BACKDROP_FILTER_PART: u16 = 9_021;
/// 切换排序模式（Severity+ΔE ↓ / Name ↑）。
pub const SORT_MODE_PART: u16 = 9_022;
/// 重置全部筛选条件。
pub const RESET_FILTER_PART: u16 = 9_023;
/// 视图模式切换基址（`0` = SideBySide、`1` = Curtain、`2` = Overlay）。
pub const VIEW_MODE_BASE: u16 = 9_030;
/// 缩放档位切换基址（`0` = 1x、`1` = 2x、`2` = 4x）。
pub const ZOOM_BASE: u16 = 9_040;
/// 卷帘对比画布热区（悬停跟随分界线，点击锁定/解锁）。
pub const CURTAIN_STAGE_PART: u16 = 9_050;
/// 卷帘锁定开关按钮。
pub const CURTAIN_LOCK_PART: u16 = 9_051;
/// 叠加模式源切换（`0` = unbound-ui、`1` = ROI ΔE 热力图）。
pub const OVERLAY_SOURCE_BASE: u16 = 9_060;
/// 叠加模式混合比例基址（`0` = 25%、`1` = 50%、`2` = 75%、`3` = 100%）。
pub const OVERLAY_SPLIT_BASE: u16 = 9_065;
/// 左侧用例列表行点击基址（`CASE_ROW_BASE + filtered_index`）。
pub const CASE_ROW_BASE: u16 = 1_000;

/// 顶部摘要与筛选工具条总高度。
const TOOLBAR_HEIGHT: f32 = 96.0;
const TOOLBAR_GAP: f32 = 12.0;
const SPLIT_GAP: f32 = 12.0;
const LIST_ROW_H: f32 = 52.0;
const LIST_ROW_GAP: f32 = 6.0;

/// 兼容布局树调用：返回检视台在给定容器下的最小推荐高度。
#[must_use]
pub fn stage_height(_case_count: usize, _stage_width: f32) -> f32 {
    640.0
}

/// 单个用例的四态结论，与差分器的 `pass` / `exempt` / `fail` / `invalid` 一一对应。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParityStatus {
    /// 四重门禁全部通过。
    Pass,
    /// 内容有效但至少一门禁未通过。
    Fail,
    /// 采集或渲染内容无效，不计入通过率。
    Invalid,
    /// 几何与整体颜色已对齐，仅剩已登记豁免理由的已知差异（不限于抗锯齿）。
    Exempt,
}

impl ParityStatus {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "pass" => Some(Self::Pass),
            "fail" => Some(Self::Fail),
            "invalid" => Some(Self::Invalid),
            // "known-aa" 为 ADR 0028 之前的旧称，读旧报告时兼容。
            "exempt" | "known-aa" => Some(Self::Exempt),
            _ => None,
        }
    }

    /// 严重度排序权重：未通过最靠前，其次无效、豁免、通过。
    #[must_use]
    pub const fn severity_rank(self) -> u8 {
        match self {
            Self::Fail => 0,
            Self::Invalid => 1,
            Self::Exempt => 2,
            Self::Pass => 3,
        }
    }
}

/// 状态筛选模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParityStatusFilter {
    #[default]
    All,
    /// 失败与无效合集（报告存在问题项时默认激活，零滚动直达需修复项）。
    Issues,
    Fail,
    Invalid,
    Exempt,
    Pass,
}

impl ParityStatusFilter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Issues,
        Self::Fail,
        Self::Invalid,
        Self::Exempt,
        Self::Pass,
    ];

    #[must_use]
    pub const fn from_index(index: u16) -> Option<Self> {
        match index {
            0 => Some(Self::All),
            1 => Some(Self::Issues),
            2 => Some(Self::Fail),
            3 => Some(Self::Invalid),
            4 => Some(Self::Exempt),
            5 => Some(Self::Pass),
            _ => None,
        }
    }

    #[must_use]
    pub const fn matches(self, status: ParityStatus) -> bool {
        match self {
            Self::All => true,
            Self::Issues => matches!(status, ParityStatus::Fail | ParityStatus::Invalid),
            Self::Fail => matches!(status, ParityStatus::Fail),
            Self::Invalid => matches!(status, ParityStatus::Invalid),
            Self::Exempt => matches!(status, ParityStatus::Exempt),
            Self::Pass => matches!(status, ParityStatus::Pass),
        }
    }
}

/// 主题维度筛选。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParityThemeFilter {
    #[default]
    All,
    Light,
    Dark,
}

impl ParityThemeFilter {
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::All,
        }
    }

    #[must_use]
    pub fn matches(self, theme: &str) -> bool {
        match self {
            Self::All => true,
            Self::Light => theme.eq_ignore_ascii_case("light"),
            Self::Dark => theme.eq_ignore_ascii_case("dark"),
        }
    }
}

/// 后景维度筛选。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParityBackdropFilter {
    #[default]
    All,
    Solid,
    Mica,
    Acrylic,
}

impl ParityBackdropFilter {
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Solid,
            Self::Solid => Self::Mica,
            Self::Mica => Self::Acrylic,
            Self::Acrylic => Self::All,
        }
    }

    #[must_use]
    pub fn matches(self, backdrop: &str) -> bool {
        match self {
            Self::All => true,
            Self::Solid => backdrop.eq_ignore_ascii_case("solid"),
            Self::Mica => backdrop.eq_ignore_ascii_case("mica"),
            Self::Acrylic => backdrop.eq_ignore_ascii_case("acrylic"),
        }
    }
}

/// 用例排序方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParitySortMode {
    /// 先按状态严重度，再按 `mean ΔE` 降序。
    #[default]
    SeverityThenDeDesc,
    /// 按用例名升序。
    NameAsc,
}

impl ParitySortMode {
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::SeverityThenDeDesc => Self::NameAsc,
            Self::NameAsc => Self::SeverityThenDeDesc,
        }
    }
}

/// 检视舞台对比视图模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParityViewMode {
    /// 三栏并排（WinUI 3 参考 / unbound-ui / ROI ΔE 热力图）。
    SideBySide,
    /// 卷帘滑块对比（鼠标移动实时跟随分界线，点击锁定/解锁）。
    #[default]
    Curtain,
    /// 叠层对比（WinUI 3 参考与 unbound-ui 或 ΔE 热力图同轴比对）。
    Overlay,
}

impl ParityViewMode {
    #[must_use]
    pub const fn from_index(index: u16) -> Option<Self> {
        match index {
            0 => Some(Self::SideBySide),
            1 => Some(Self::Curtain),
            2 => Some(Self::Overlay),
            _ => None,
        }
    }
}

/// 像素对照检视台的交互状态。
#[derive(Clone, Debug)]
pub struct ParityInspectorState {
    pub status_filter: ParityStatusFilter,
    pub theme_filter: ParityThemeFilter,
    pub backdrop_filter: ParityBackdropFilter,
    pub sort_mode: ParitySortMode,
    /// 当前选中的用例名（在筛选或刷新时尽量维持同一用例）。
    pub selected_case: Option<String>,
    pub view_mode: ParityViewMode,
    /// 卷帘分界线归一化位置（`0.02..=0.98`）。
    pub curtain_ratio: f32,
    /// 为 `true` 时冻结卷帘分界线，不再随鼠标移动变化。
    pub curtain_locked: bool,
    /// 叠加模式下为 `true` 时叠加热力图，为 `false` 时叠加本框架渲染。
    pub overlay_show_heatmap: bool,
    /// 叠加模式下的对比剖分/显现比例（0.25 / 0.50 / 0.75 / 1.0）。
    pub overlay_split: f32,
    /// 检视缩放档位（1 / 2 / 4）。
    pub zoom: u8,
    /// 左侧列表独立垂直滚动偏移。
    pub list_scroll: f32,
    /// 是否已根据当前报告完成首次自动聚焦（有失败项默认切到 `Issues`）。
    pub initialized_for_report: bool,
    /// 最近一帧卷帘画布的逻辑矩形，用于 `pointer_move` 实时跟随。
    pub curtain_stage_rect: core::cell::Cell<Option<Rect>>,
    /// 最近一帧左侧用例列表视口矩形，用于滚轮独立滚动。
    pub list_pane_rect: core::cell::Cell<Option<Rect>>,
}

impl Default for ParityInspectorState {
    fn default() -> Self {
        Self {
            status_filter: ParityStatusFilter::All,
            theme_filter: ParityThemeFilter::All,
            backdrop_filter: ParityBackdropFilter::All,
            sort_mode: ParitySortMode::SeverityThenDeDesc,
            selected_case: None,
            view_mode: ParityViewMode::Curtain,
            curtain_ratio: 0.5,
            curtain_locked: false,
            overlay_show_heatmap: true,
            overlay_split: 0.5,
            zoom: 1,
            list_scroll: 0.0,
            initialized_for_report: false,
            curtain_stage_rect: core::cell::Cell::new(None),
            list_pane_rect: core::cell::Cell::new(None),
        }
    }
}

impl ParityInspectorState {
    /// 首次载入报告或刷新后自动聚焦：存在失败或无效用例时默认切到 `Issues`。
    pub fn auto_focus_for_report(&mut self, report: &ParityReport) {
        if !self.initialized_for_report {
            self.status_filter = if report.failed + report.invalid > 0 {
                ParityStatusFilter::Issues
            } else {
                ParityStatusFilter::All
            };
            self.initialized_for_report = true;
        }
    }

    /// 重置全部过滤条件为默认态。
    pub fn reset_filters(&mut self) {
        self.status_filter = ParityStatusFilter::All;
        self.theme_filter = ParityThemeFilter::All;
        self.backdrop_filter = ParityBackdropFilter::All;
        self.list_scroll = 0.0;
    }

    /// 当前是否激活了任意非默认过滤条件。
    #[must_use]
    pub fn has_active_filters(&self) -> bool {
        self.status_filter != ParityStatusFilter::All
            || self.theme_filter != ParityThemeFilter::All
            || self.backdrop_filter != ParityBackdropFilter::All
    }

    /// 根据指针 X 坐标更新卷帘分界线（仅在卷帘模式且未锁定时生效）。
    pub fn scrub_curtain(&mut self, pointer: Vec2) -> bool {
        if self.view_mode != ParityViewMode::Curtain || self.curtain_locked {
            return false;
        }
        let Some(rect) = self.curtain_stage_rect.get() else {
            return false;
        };
        if !rect.contains_point(pointer) || rect.size.x <= 1.0 {
            return false;
        }
        let ratio = ((pointer.x - rect.origin.x) / rect.size.x).clamp(0.02, 0.98);
        if (ratio - self.curtain_ratio).abs() > 0.002 {
            self.curtain_ratio = ratio;
            return true;
        }
        false
    }

    /// 键盘上下移动当前选中用例。
    pub fn step_selection(&mut self, report: &ParityReport, query: &str, delta: isize) -> bool {
        let filtered = filtered_case_indices(report, self, query);
        if filtered.is_empty() {
            return false;
        }
        let current_pos = self
            .selected_case
            .as_deref()
            .and_then(|name| filtered.iter().position(|&idx| report.cases[idx].name == name))
            .unwrap_or(0);
        let next_pos = (current_pos as isize + delta).clamp(0, filtered.len() as isize - 1) as usize;
        let next_name = report.cases[filtered[next_pos]].name.clone();
        if self.selected_case.as_deref() != Some(next_name.as_str()) {
            self.selected_case = Some(next_name);
            let row_top = next_pos as f32 * (LIST_ROW_H + LIST_ROW_GAP);
            if row_top < self.list_scroll {
                self.list_scroll = row_top;
            } else if let Some(pane) = self.list_pane_rect.get() {
                let row_bottom = row_top + LIST_ROW_H;
                if row_bottom > self.list_scroll + pane.size.y {
                    self.list_scroll = (row_bottom - pane.size.y).max(0.0);
                }
            }
            return true;
        }
        false
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
    /// 三张 PNG 的内容指纹；刷新时逐张比对，未变的直接复用旧纹理，避免图集持续膨胀。
    pub image_keys: [u64; 3],
}

/// 已载入的对照报告。
#[derive(Debug)]
pub struct ParityReport {
    pub cases: Vec<ParityCase>,
    pub passed: usize,
    pub failed: usize,
    pub invalid: usize,
    /// 已登记豁免的用例数。
    pub exempt: usize,
    pub cache_hits: usize,
    /// 载入失败原因；为 `Some` 时页面显示空态说明。
    pub error: Option<String>,
    /// 空态里给出的下一步命令，随运行形态（仓库内 / 独立分发）切换。
    pub hint: String,
}

impl ParityReport {
    /// 读取 `report.json` 并提交全部截图进行异步解码。
    #[must_use]
    pub fn load(hub: &mut AssetHub, previous: Option<&Self>) -> Self {
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
                    exempt: 0,
                    cache_hits: 0,
                    error: Some(format!("{REPORT_DIR}/report.json: {error}")),
                    hint,
                }
            }
        };
        let reference = reference_dir(root);
        let ours = root.join("ours");
        let heat = root.join("heat");
        let previous_images: std::collections::HashMap<&str, ([u64; 3], [Option<AssetId>; 3])> =
            previous
                .map(|report| {
                    report
                        .cases
                        .iter()
                        .map(|case| (case.name.as_str(), (case.image_keys, case.images)))
                        .collect()
                })
                .unwrap_or_default();
        let mut cases = parse_cases(&json);
        for case in &mut cases {
            for which in 0..3 {
                let dir: &Path = match which {
                    0 => &reference,
                    1 => &ours,
                    _ => &heat,
                };
                let Ok(bytes) = std::fs::read(dir.join(format!("{}.png", case.name))) else {
                    continue;
                };
                let key = content_key(&bytes);
                case.image_keys[which] = key;
                let reused = previous_images
                    .get(case.name.as_str())
                    .filter(|entry| entry.0[which] == key)
                    .map(|entry| entry.1[which]);
                if let Some(image) = reused {
                    case.images[which] = image;
                    continue;
                }
                case.images[which] = Some(hub.request_png(bytes));
            }
        }
        let passed = cases.iter().filter(|c| c.status == ParityStatus::Pass).count();
        let failed = cases.iter().filter(|c| c.status == ParityStatus::Fail).count();
        let invalid = cases.iter().filter(|c| c.status == ParityStatus::Invalid).count();
        let exempt = cases.iter().filter(|c| c.status == ParityStatus::Exempt).count();
        let cache_hits = cases.iter().filter(|c| c.cache_hit).count();
        if cases.is_empty() {
            return Self {
                cases,
                passed,
                failed,
                invalid,
                exempt,
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
            exempt,
            cache_hits,
            error: None,
            hint,
        }
    }
}

/// 按当前筛选条件与排序规则返回命中的用例索引列表。
#[must_use]
pub fn filtered_case_indices(
    report: &ParityReport,
    inspector: &ParityInspectorState,
    query: &str,
) -> Vec<usize> {
    let needle = query.trim().to_ascii_lowercase();
    let mut indices: Vec<usize> = report
        .cases
        .iter()
        .enumerate()
        .filter(|(_, case)| {
            if !inspector.status_filter.matches(case.status) {
                return false;
            }
            if !inspector.theme_filter.matches(&case.theme) {
                return false;
            }
            if !inspector.backdrop_filter.matches(&case.backdrop) {
                return false;
            }
            if !needle.is_empty() {
                let hay = format!("{} {} {} {} {}", case.name, case.control, case.state, case.theme, case.backdrop)
                    .to_ascii_lowercase();
                if !hay.contains(&needle) {
                    return false;
                }
            }
            true
        })
        .map(|(idx, _)| idx)
        .collect();

    match inspector.sort_mode {
        ParitySortMode::SeverityThenDeDesc => {
            indices.sort_by(|&a, &b| {
                let ca = &report.cases[a];
                let cb = &report.cases[b];
                ca.status
                    .severity_rank()
                    .cmp(&cb.status.severity_rank())
                    .then_with(|| cb.mean_de.total_cmp(&ca.mean_de))
                    .then_with(|| ca.name.cmp(&cb.name))
            });
        }
        ParitySortMode::NameAsc => {
            indices.sort_by(|&a, &b| report.cases[a].name.cmp(&report.cases[b].name));
        }
    }
    indices
}

/// 空态里给出的下一步命令。
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

fn content_key(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

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
            image_keys: [0; 3],
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

fn parity_node(part: u16) -> u64 {
    target::id(Target::Demo(PageId::Parity.index() as u8, 0, part))
}

/// 兼容 `DemoKind::ParityReport` 分派入口。
pub fn report(ctx: &mut DemoCtx<'_, '_>) {
    draw_inspector(ctx.canvas, ctx.state, ctx.rect);
}

/// 渲染主从式像素对照检视台（脱离 `ExampleCard` 外框）。
pub fn draw_inspector(canvas: &mut Canvas<'_>, state: &GalleryState, rect: Rect) {
    let zh = state.locale.is_zh();
    state.parity_inspector.curtain_stage_rect.set(None);
    state.parity_inspector.list_pane_rect.set(None);

    let Some(report) = state.demo.parity_report.as_ref() else {
        draw_empty_card(
            canvas,
            rect,
            if zh { "正在载入差分报告…" } else { "Loading the parity report…" },
            "",
            zh,
            false,
        );
        return;
    };
    if let Some(error) = &report.error {
        let body = if zh {
            format!("未找到可用的差分产物：{error}\n先运行 `{}`，再回到本页。", report.hint)
        } else {
            format!("No parity artifacts were found: {error}\nRun `{}` first, then reopen this page.", report.hint)
        };
        draw_empty_card(
            canvas,
            rect,
            if zh { "差分报告不可用" } else { "Parity report unavailable" },
            &body,
            zh,
            true,
        );
        return;
    }

    let inspector = &state.parity_inspector;
    let filtered = filtered_case_indices(report, inspector, state.search.text());

    // 1. 顶部摘要与筛选工具条
    let toolbar = Rect::new(rect.origin.x, rect.origin.y, rect.size.x, TOOLBAR_HEIGHT);
    draw_toolbar(canvas, state, report, toolbar, filtered.len(), zh);

    // 2. 主从分栏主体（左侧紧凑用例列表 + 右侧大尺寸检视舞台）
    let body_top = toolbar.max().y + TOOLBAR_GAP;
    let body_h = (rect.max().y - body_top).max(380.0);
    let left_w = (rect.size.x * 0.30).clamp(228.0, 330.0).min((rect.size.x - 280.0).max(180.0));
    let left_rect = Rect::new(rect.origin.x, body_top, left_w, body_h);
    let right_rect = Rect::new(
        left_rect.max().x + SPLIT_GAP,
        body_top,
        (rect.size.x - left_w - SPLIT_GAP).max(260.0),
        body_h,
    );

    inspector.list_pane_rect.set(Some(left_rect));
    let selected_idx = draw_master_list(canvas, state, report, &filtered, left_rect, zh);
    draw_detail_stage(canvas, state, report, selected_idx, right_rect, zh);
}

fn draw_toolbar(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    report: &ParityReport,
    rect: Rect,
    matched_count: usize,
    zh: bool,
) {
    let palette = *canvas.palette;
    let inspector = &state.parity_inspector;
    canvas.rect(rect, 12.0, palette.card_bg, palette.card_border, 1.0, 0.0);

    // 第一行：汇总进度 + 刷新按钮
    let ratio = (report.passed + report.exempt) as f32 / report.cases.len().max(1) as f32;
    let summary_text = if zh {
        format!(
            "像素对照检视台 · 已收口 {:.0}%（{} / {}）· 显示 {} 条",
            ratio * 100.0,
            report.passed + report.exempt,
            report.cases.len(),
            matched_count
        )
    } else {
        format!(
            "Parity Inspector · {:.0}% settled ({} / {}) · showing {}",
            ratio * 100.0,
            report.passed + report.exempt,
            report.cases.len(),
            matched_count
        )
    };
    canvas.text(
        &summary_text,
        Rect::new(rect.origin.x + 14.0, rect.origin.y + 10.0, (rect.size.x - 260.0).max(180.0), 22.0),
        13.5,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );

    let bar = Rect::new(rect.max().x - 236.0, rect.origin.y + 17.0, 116.0, 8.0);
    canvas.rect(bar, 4.0, palette.control_track, 0, 0.0, 0.0);
    canvas.rect(
        Rect::new(bar.origin.x, bar.origin.y, bar.size.x * ratio, bar.size.y),
        4.0,
        if report.failed + report.invalid > 0 {
            palette.warning
        } else {
            palette.success
        },
        0,
        0.0,
        0.0,
    );

    widgets::button(
        canvas,
        parity_node(REFRESH_PART),
        Rect::new(rect.max().x - 106.0, rect.origin.y + 8.0, 92.0, 26.0),
        if zh { "刷新" } else { "Refresh" },
        ButtonVariant::Secondary,
        true,
    );

    // 第二行：状态筛选胶囊 + Theme / Backdrop / Sort / Reset
    let row_y = rect.origin.y + 52.0;
    let mut x = rect.origin.x + 14.0;
    let status_pills = [
        (0u16, ParityStatusFilter::All, if zh { format!("全部 ({})", report.cases.len()) } else { format!("All ({})", report.cases.len()) }),
        (1u16, ParityStatusFilter::Issues, if zh { format!("问题 ({})", report.failed + report.invalid) } else { format!("Issues ({})", report.failed + report.invalid) }),
        (2u16, ParityStatusFilter::Fail, if zh { format!("失败 ({})", report.failed) } else { format!("Fail ({})", report.failed) }),
        (3u16, ParityStatusFilter::Invalid, if zh { format!("无效 ({})", report.invalid) } else { format!("Invalid ({})", report.invalid) }),
        (4u16, ParityStatusFilter::Exempt, if zh { format!("豁免 ({})", report.exempt) } else { format!("Exempt ({})", report.exempt) }),
        (5u16, ParityStatusFilter::Pass, if zh { format!("通过 ({})", report.passed) } else { format!("Pass ({})", report.passed) }),
    ];
    for (idx, filter, label) in status_pills {
        let w = canvas.labels.measure(&label, 10.5).x + 18.0;
        if x + w > rect.max().x - 240.0 {
            break;
        }
        let pill = Rect::new(x, row_y, w, 26.0);
        let active = inspector.status_filter == filter;
        draw_filter_pill(canvas, pill, parity_node(STATUS_FILTER_BASE + idx), &label, active);
        x += w + 6.0;
    }

    // 右侧多维筛选与排序胶囊
    let theme_label = match inspector.theme_filter {
        ParityThemeFilter::All => if zh { "主题: 全部" } else { "Theme: All" },
        ParityThemeFilter::Light => if zh { "主题: 浅色" } else { "Theme: Light" },
        ParityThemeFilter::Dark => if zh { "主题: 深色" } else { "Theme: Dark" },
    };
    let backdrop_label = match inspector.backdrop_filter {
        ParityBackdropFilter::All => if zh { "后景: 全部" } else { "Backdrop: All" },
        ParityBackdropFilter::Solid => if zh { "后景: Solid" } else { "Backdrop: Solid" },
        ParityBackdropFilter::Mica => if zh { "后景: Mica" } else { "Backdrop: Mica" },
        ParityBackdropFilter::Acrylic => if zh { "后景: Acrylic" } else { "Backdrop: Acrylic" },
    };
    let sort_label = match inspector.sort_mode {
        ParitySortMode::SeverityThenDeDesc => if zh { "排序: ΔE ↓" } else { "Sort: ΔE ↓" },
        ParitySortMode::NameAsc => if zh { "排序: A-Z" } else { "Sort: A-Z" },
    };

    let mut rx = rect.max().x - 14.0;
    if inspector.has_active_filters() {
        let reset_label = if zh { "重置" } else { "Reset" };
        let w = canvas.labels.measure(reset_label, 10.5).x + 16.0;
        rx -= w;
        draw_filter_pill(canvas, Rect::new(rx, row_y, w, 26.0), parity_node(RESET_FILTER_PART), reset_label, false);
        rx -= 6.0;
    }
    for (part, label, active) in [
        (SORT_MODE_PART, sort_label, inspector.sort_mode == ParitySortMode::NameAsc),
        (BACKDROP_FILTER_PART, backdrop_label, inspector.backdrop_filter != ParityBackdropFilter::All),
        (THEME_FILTER_PART, theme_label, inspector.theme_filter != ParityThemeFilter::All),
    ] {
        let w = canvas.labels.measure(label, 10.5).x + 16.0;
        if rx - w < x + 8.0 {
            break;
        }
        rx -= w;
        draw_filter_pill(canvas, Rect::new(rx, row_y, w, 26.0), parity_node(part), label, active);
        rx -= 6.0;
    }
}

fn draw_filter_pill(canvas: &mut Canvas<'_>, rect: Rect, node: u64, label: &str, active: bool) {
    let palette = *canvas.palette;
    let visual = canvas.visual(node, active);
    let fill = if active {
        palette.primary
    } else if visual.is_hovered() {
        palette.primary_soft
    } else {
        palette.surface
    };
    let fg = if active { palette.on_primary } else { palette.text };
    canvas.rect(rect, 13.0, fill, palette.card_border, if active { 0.0 } else { 1.0 }, 0.0);
    canvas.text(label, rect, 10.5, fg, AlignX::Center, AlignY::Center);
    canvas.control(node, rect, Role::Button, label, CursorKind::Pointer, active, false);
}

fn draw_master_list(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    report: &ParityReport,
    filtered: &[usize],
    rect: Rect,
    zh: bool,
) -> Option<usize> {
    let palette = *canvas.palette;
    let inspector = &state.parity_inspector;
    canvas.rect(rect, 12.0, palette.card_bg, palette.card_border, 1.0, 0.0);

    if filtered.is_empty() {
        canvas.text_wrapped(
            if zh {
                "当前筛选条件下没有匹配的用例。"
            } else {
                "No parity cases match the current filters."
            },
            Rect::new(rect.origin.x + 16.0, rect.origin.y + 24.0, rect.size.x - 32.0, 44.0),
            11.5,
            palette.text_muted,
        );
        widgets::button(
            canvas,
            parity_node(RESET_FILTER_PART),
            Rect::new(rect.origin.x + 16.0, rect.origin.y + 76.0, 120.0, 30.0),
            if zh { "重置筛选" } else { "Reset filters" },
            ButtonVariant::Primary,
            true,
        );
        return None;
    }

    let selected_idx = inspector
        .selected_case
        .as_deref()
        .and_then(|name| filtered.iter().copied().find(|&idx| report.cases[idx].name == name))
        .unwrap_or(filtered[0]);

    let inner = Rect::new(rect.origin.x + 8.0, rect.origin.y + 8.0, rect.size.x - 16.0, rect.size.y - 16.0);
    canvas.begin_clip(inner);
    let mut y = inner.origin.y - inspector.list_scroll;
    for (pos, &case_idx) in filtered.iter().enumerate() {
        let row = Rect::new(inner.origin.x, y, inner.size.x, LIST_ROW_H);
        y += LIST_ROW_H + LIST_ROW_GAP;
        if row.max().y < inner.origin.y - 4.0 || row.origin.y > inner.max().y + 4.0 {
            continue;
        }
        let case = &report.cases[case_idx];
        let is_selected = case_idx == selected_idx;
        let node = parity_node(CASE_ROW_BASE + pos as u16);
        let visual = canvas.visual(node, is_selected);
        let bg = if is_selected {
            palette.primary_soft
        } else if visual.is_hovered() {
            palette.surface
        } else {
            palette.card_bg
        };
        let border = if is_selected {
            palette.primary
        } else {
            palette.card_border
        };
        canvas.rect(row, 8.0, bg, border, 1.0, 0.0);

        let (status_text, status_fg, status_bg) = status_style(palette, case.status, zh);
        let badge_w = canvas.labels.measure(status_text, 9.0).x + 12.0;
        let badge = Rect::new(row.max().x - badge_w - 8.0, row.origin.y + 8.0, badge_w, 16.0);
        canvas.rect(badge, 8.0, status_bg, 0, 0.0, 0.0);
        canvas.text(status_text, badge, 9.0, status_fg, AlignX::Center, AlignY::Center);

        let title = if case.control.is_empty() {
            case.name.clone()
        } else {
            format!("{} · {}", case.control, case.state)
        };
        canvas.text(
            &title,
            Rect::new(row.origin.x + 10.0, row.origin.y + 7.0, (row.size.x - badge_w - 24.0).max(40.0), 18.0),
            11.0,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        let meta = format!("{} · {} · ΔE {:.2}", case.theme, case.backdrop, case.mean_de);
        canvas.text(
            &meta,
            Rect::new(row.origin.x + 10.0, row.origin.y + 28.0, row.size.x - 20.0, 16.0),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        canvas.control(node, row, Role::ListItem, &title, CursorKind::Pointer, is_selected, false);
    }
    canvas.end_clip();

    Some(selected_idx)
}

fn draw_detail_stage(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    report: &ParityReport,
    selected_idx: Option<usize>,
    rect: Rect,
    zh: bool,
) {
    let palette = *canvas.palette;
    let inspector = &state.parity_inspector;
    canvas.rect(rect, 12.0, palette.card_bg, palette.card_border, 1.0, 0.0);

    let Some(case_idx) = selected_idx else {
        canvas.text(
            if zh { "请选择左侧任意用例进行像素对比" } else { "Select a case on the left to inspect" },
            rect,
            13.0,
            palette.text_muted,
            AlignX::Center,
            AlignY::Center,
        );
        return;
    };
    let case = &report.cases[case_idx];

    // 头部：用例标题 + 状态徽章 + 四门禁读数
    let title = if case.control.is_empty() {
        case.name.clone()
    } else {
        format!("{} · {} · {} · {}", case.control, case.state, case.theme, case.backdrop)
    };
    let (status_text, status_fg, status_bg) = status_style(palette, case.status, zh);
    let badge_w = canvas.labels.measure(status_text, 10.0).x + 18.0;
    let badge = Rect::new(rect.max().x - badge_w - 14.0, rect.origin.y + 12.0, badge_w, 20.0);
    canvas.rect(badge, 10.0, status_bg, 0, 0.0, 0.0);
    canvas.text(status_text, badge, 10.0, status_fg, AlignX::Center, AlignY::Center);

    canvas.text(
        &title,
        Rect::new(rect.origin.x + 16.0, rect.origin.y + 10.0, (rect.size.x - badge_w - 40.0).max(120.0), 22.0),
        14.0,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
    let gates = format!(
        "ROI mean {:.2} · p95 {:.2} · >5ΔE {:.1}% · {} [{}, {}, {}, {}] · intrinsic {}",
        case.mean_de,
        case.p95_de,
        case.over_limit_ratio * 100.0,
        if zh { "包围盒Δ" } else { "bboxΔ" },
        case.bbox_delta[0],
        case.bbox_delta[1],
        case.bbox_delta[2],
        case.bbox_delta[3],
        if case.intrinsic_matches { "ok" } else { "drift" }
    );
    canvas.text(
        &gates,
        Rect::new(rect.origin.x + 16.0, rect.origin.y + 36.0, rect.size.x - 32.0, 18.0),
        10.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    // 视图控制条：并排 / 卷帘 / 叠加 + 缩放 + 模式专属快捷开关
    let ctrl_y = rect.origin.y + 62.0;
    let mut cx = rect.origin.x + 16.0;
    let modes = [
        (0u16, ParityViewMode::SideBySide, if zh { "并排 (1)" } else { "Side-by-Side (1)" }),
        (1u16, ParityViewMode::Curtain, if zh { "卷帘 (2)" } else { "Curtain (2)" }),
        (2u16, ParityViewMode::Overlay, if zh { "叠加 (3)" } else { "Overlay (3)" }),
    ];
    for (idx, mode, label) in modes {
        let w = canvas.labels.measure(label, 10.5).x + 18.0;
        draw_filter_pill(canvas, Rect::new(cx, ctrl_y, w, 26.0), parity_node(VIEW_MODE_BASE + idx), label, inspector.view_mode == mode);
        cx += w + 6.0;
    }

    // 右侧缩放档位 1x / 2x / 4x
    let mut rx = rect.max().x - 16.0;
    for &(idx, z, label) in [(2u16, 4u8, "4x"), (1u16, 2u8, "2x"), (0u16, 1u8, "1x")].iter() {
        let w = 34.0;
        rx -= w;
        draw_filter_pill(canvas, Rect::new(rx, ctrl_y, w, 26.0), parity_node(ZOOM_BASE + idx), label, inspector.zoom == z);
        rx -= 4.0;
    }

    // 模式专属操作胶囊（卷帘锁定 / 叠加源与比例）
    match inspector.view_mode {
        ParityViewMode::Curtain => {
            let lock_label = if inspector.curtain_locked {
                if zh { format!("已锁定 {:.0}%", inspector.curtain_ratio * 100.0) } else { format!("Locked {:.0}%", inspector.curtain_ratio * 100.0) }
            } else if zh {
                format!("鼠标跟随 {:.0}%", inspector.curtain_ratio * 100.0)
            } else {
                format!("Hover Scrub {:.0}%", inspector.curtain_ratio * 100.0)
            };
            let w = canvas.labels.measure(&lock_label, 10.0).x + 18.0;
            if cx + w + 8.0 <= rx {
                draw_filter_pill(canvas, Rect::new(cx + 8.0, ctrl_y, w, 26.0), parity_node(CURTAIN_LOCK_PART), &lock_label, inspector.curtain_locked);
            }
        }
        ParityViewMode::Overlay => {
            let src_ours = if zh { "叠加本框架" } else { "Ours" };
            let src_heat = if zh { "叠加热力图" } else { "Heatmap" };
            let mut ox = cx + 8.0;
            for (idx, label, active) in [
                (0u16, src_ours, !inspector.overlay_show_heatmap),
                (1u16, src_heat, inspector.overlay_show_heatmap),
            ] {
                let w = canvas.labels.measure(label, 10.0).x + 14.0;
                if ox + w <= rx {
                    draw_filter_pill(canvas, Rect::new(ox, ctrl_y, w, 26.0), parity_node(OVERLAY_SOURCE_BASE + idx), label, active);
                    ox += w + 4.0;
                }
            }
            for (idx, pct, label) in [(0u16, 0.25f32, "25%"), (1u16, 0.50f32, "50%"), (2u16, 0.75f32, "75%"), (3u16, 1.0f32, "100%")] {
                let w = 36.0;
                if ox + w <= rx {
                    draw_filter_pill(
                        canvas,
                        Rect::new(ox, ctrl_y, w, 26.0),
                        parity_node(OVERLAY_SPLIT_BASE + idx),
                        label,
                        (inspector.overlay_split - pct).abs() < 0.05,
                    );
                    ox += w + 4.0;
                }
            }
        }
        ParityViewMode::SideBySide => {}
    }

    // 主对比舞台区域
    let stage = Rect::new(
        rect.origin.x + 16.0,
        ctrl_y + 38.0,
        rect.size.x - 32.0,
        (rect.max().y - (ctrl_y + 38.0) - 16.0).max(200.0),
    );
    match inspector.view_mode {
        ParityViewMode::SideBySide => draw_side_by_side(canvas, state, case, stage, zh),
        ParityViewMode::Curtain => draw_curtain_view(canvas, state, case, stage, zh),
        ParityViewMode::Overlay => draw_overlay_view(canvas, state, case, stage, zh),
    }
}

fn draw_side_by_side(canvas: &mut Canvas<'_>, state: &GalleryState, case: &ParityCase, stage: Rect, zh: bool) {
    let palette = *canvas.palette;
    let gap = 10.0;
    let col_w = (stage.size.x - gap * 2.0) / 3.0;
    let img_h = (stage.size.y - 24.0).max(120.0);
    let zoom = f32::from(state.parity_inspector.zoom.max(1));
    let captions = [
        if zh { "WinUI 3 参考" } else { "WinUI 3 reference" },
        "unbound-ui",
        if zh { "ROI ΔE2000 热力图" } else { "ROI ΔE2000 heatmap" },
    ];
    for (which, image) in case.images.iter().enumerate() {
        let cell = Rect::new(stage.origin.x + which as f32 * (col_w + gap), stage.origin.y, col_w, img_h);
        canvas.rect(cell, 8.0, palette.stage_bg, palette.stage_grid, 1.0, 0.0);
        draw_zoomed_asset(canvas, state, cell, *image, zoom, 200 + which as u16);
        canvas.text(
            captions[which],
            Rect::new(cell.origin.x, cell.max().y + 4.0, cell.size.x, 16.0),
            10.0,
            palette.text_muted,
            AlignX::Center,
            AlignY::Top,
        );
    }
}

fn draw_curtain_view(canvas: &mut Canvas<'_>, state: &GalleryState, case: &ParityCase, stage: Rect, zh: bool) {
    let palette = *canvas.palette;
    let inspector = &state.parity_inspector;
    let frame = Rect::new(stage.origin.x, stage.origin.y, stage.size.x, (stage.size.y - 24.0).max(160.0));
    inspector.curtain_stage_rect.set(Some(frame));

    canvas.rect(frame, 10.0, palette.stage_bg, palette.stage_grid, 1.0, 0.0);
    let zoom = f32::from(inspector.zoom.max(1));
    let split_x = frame.origin.x + frame.size.x * inspector.curtain_ratio.clamp(0.02, 0.98);

    // 左侧裁剪区：WinUI 3 参考图
    let left_clip = Rect::new(frame.origin.x, frame.origin.y, (split_x - frame.origin.x).max(0.0), frame.size.y);
    canvas.begin_clip(left_clip);
    draw_zoomed_asset(canvas, state, frame, case.images[0], zoom, 300);
    canvas.end_clip();

    // 右侧裁剪区：unbound-ui 本框架渲染
    let right_clip = Rect::new(split_x, frame.origin.y, (frame.max().x - split_x).max(0.0), frame.size.y);
    canvas.begin_clip(right_clip);
    draw_zoomed_asset(canvas, state, frame, case.images[1], zoom, 301);
    canvas.end_clip();

    // 分界线与中央抓手胶囊
    canvas.rect(Rect::new(split_x - 1.0, frame.origin.y, 2.0, frame.size.y), 1.0, palette.primary, 0, 0.0, 0.0);
    let handle = Rect::new(split_x - 14.0, frame.origin.y + frame.size.y * 0.5 - 14.0, 28.0, 28.0);
    canvas.rect(handle, 14.0, palette.primary, palette.on_primary, 1.5, 4.0);
    canvas.text("↔", handle, 12.0, palette.on_primary, AlignX::Center, AlignY::Center);

    // 左右角标
    widgets::badge(
        canvas,
        Rect::new(frame.origin.x + 10.0, frame.origin.y + 10.0, 96.0, 20.0),
        if zh { "WinUI 3 参考" } else { "WinUI 3 ref" },
        palette.primary,
        palette.card_bg,
    );
    widgets::badge(
        canvas,
        Rect::new(frame.max().x - 94.0, frame.origin.y + 10.0, 84.0, 20.0),
        "unbound-ui",
        palette.success,
        palette.card_bg,
    );

    canvas.text(
        if zh {
            "在画布上移动鼠标即可实时对比，点击画布锁定/解锁分界线；按 ↑/↓ 切换用例"
        } else {
            "Move pointer across the stage to scrub; click to lock/unlock split; press ↑/↓ to switch cases"
        },
        Rect::new(frame.origin.x, frame.max().y + 5.0, frame.size.x, 16.0),
        10.0,
        palette.text_muted,
        AlignX::Center,
        AlignY::Top,
    );

    canvas.control(
        parity_node(CURTAIN_STAGE_PART),
        frame,
        Role::Button,
        if zh { "卷帘对比画布（点击锁定分界线）" } else { "Curtain comparison stage (click to lock split)" },
        CursorKind::Pointer,
        inspector.curtain_locked,
        false,
    );
}

fn draw_overlay_view(canvas: &mut Canvas<'_>, state: &GalleryState, case: &ParityCase, stage: Rect, zh: bool) {
    let palette = *canvas.palette;
    let inspector = &state.parity_inspector;
    let frame = Rect::new(stage.origin.x, stage.origin.y, stage.size.x, (stage.size.y - 24.0).max(160.0));
    canvas.rect(frame, 10.0, palette.stage_bg, palette.stage_grid, 1.0, 0.0);

    let zoom = f32::from(inspector.zoom.max(1));
    // 底图：WinUI 3 参考图
    draw_zoomed_asset(canvas, state, frame, case.images[0], zoom, 400);

    // 叠层：按 overlay_split 控制可见区域，支持本框架图或 ROI ΔE2000 热力图同轴比对
    let overlay_asset = if inspector.overlay_show_heatmap {
        case.images[2]
    } else {
        case.images[1]
    };
    let ratio = inspector.overlay_split.clamp(0.0, 1.0);
    if ratio > 0.01 {
        let clip = Rect::new(frame.origin.x, frame.origin.y, frame.size.x * ratio, frame.size.y);
        canvas.begin_clip(clip);
        draw_zoomed_asset(canvas, state, frame, overlay_asset, zoom, 401);
        canvas.end_clip();
        if ratio < 0.99 {
            canvas.rect(
                Rect::new(clip.max().x - 1.0, frame.origin.y, 2.0, frame.size.y),
                1.0,
                palette.warning,
                0,
                0.0,
                0.0,
            );
        }
    }

    let overlay_label = if inspector.overlay_show_heatmap {
        if zh { "ROI ΔE2000 热力图叠层" } else { "ROI ΔE2000 heatmap layer" }
    } else {
        "unbound-ui"
    };
    widgets::badge(
        canvas,
        Rect::new(frame.origin.x + 10.0, frame.origin.y + 10.0, 148.0, 20.0),
        overlay_label,
        palette.warning,
        palette.card_bg,
    );
    canvas.text(
        if zh {
            "上方可一键切换叠层来源（本框架 / 热力图）与覆盖比例，搭配 2x/4x 排查 1px 偏移"
        } else {
            "Toggle overlay layer (Ours / Heatmap) and coverage ratio above; pair with 2x/4x to inspect 1px drift"
        },
        Rect::new(frame.origin.x, frame.max().y + 5.0, frame.size.x, 16.0),
        10.0,
        palette.text_muted,
        AlignX::Center,
        AlignY::Top,
    );
}

fn draw_zoomed_asset(
    canvas: &mut Canvas<'_>,
    state: &GalleryState,
    container: Rect,
    asset: Option<AssetId>,
    zoom: f32,
    part_offset: u16,
) {
    let Some(hub) = state.demo.asset_hub.as_ref() else {
        return;
    };
    let Some(id) = asset else {
        return;
    };
    let zoomed = if zoom > 1.01 {
        let w = container.size.x * zoom;
        let h = container.size.y * zoom;
        Rect::new(
            container.origin.x - (w - container.size.x) * 0.5,
            container.origin.y - (h - container.size.y) * 0.5,
            w,
            h,
        )
    } else {
        container
    };
    canvas.begin_clip(container);
    let node = parity_node(part_offset);
    let mut desc = ImageDesc::new(zoomed, Vec2::new(240.0, 96.0), 0).with_asset(id);
    desc.fit = ObjectFit::Contain;
    desc.radius = [4.0; 4];
    let output = emit_image_with_assets(node, &desc, ControlState::default(), hub);
    canvas.widget_output(WidgetCatalogId::Image, node, &output);
    canvas.end_clip();
}

fn status_style(palette: crate::gallery::Palette, status: ParityStatus, zh: bool) -> (&'static str, u16, u16) {
    match status {
        ParityStatus::Pass => (if zh { "通过" } else { "pass" }, palette.success, palette.success_soft),
        ParityStatus::Fail => (if zh { "失败" } else { "fail" }, palette.danger, palette.danger_soft),
        ParityStatus::Invalid => (if zh { "无效" } else { "invalid" }, palette.warning, palette.warning_soft),
        ParityStatus::Exempt => (if zh { "豁免" } else { "exempt" }, palette.primary, palette.primary_soft),
    }
}

fn draw_empty_card(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    title: &str,
    body: &str,
    zh: bool,
    show_refresh: bool,
) {
    let palette = *canvas.palette;
    let height = if show_refresh { 208.0 } else { 180.0 };
    let card = Rect::new(rect.origin.x, rect.origin.y, rect.size.x, height);
    canvas.rect(card, 12.0, palette.card_bg, palette.card_border, 1.0, 0.0);
    canvas.text(
        title,
        Rect::new(card.origin.x + 20.0, card.origin.y + 20.0, card.size.x - 40.0, 26.0),
        15.0,
        palette.text,
        AlignX::Start,
        AlignY::Top,
    );
    if !body.is_empty() {
        canvas.text_wrapped(
            body,
            Rect::new(card.origin.x + 20.0, card.origin.y + 54.0, card.size.x - 40.0, 92.0),
            10.5,
            palette.text_muted,
        );
    }
    if show_refresh {
        widgets::button(
            canvas,
            parity_node(REFRESH_PART),
            Rect::new(card.origin.x + 20.0, card.origin.y + 154.0, 112.0, 32.0),
            if zh { "刷新" } else { "Refresh" },
            ButtonVariant::Primary,
            true,
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

    const MULTI_STATE_REPORT: &str = r#"{
  "cases": [
    { "name": "Button_normal_light", "control": "Button", "state": "normal", "theme": "light", "backdrop": "mica", "status": "pass", "meanDe": 0.4200, "p95De": 1.7500, "p99De": 3.2000, "overLimitRatio": 0.005000, "referenceBbox": [16, 16, 136, 48], "oursBbox": [16, 16, 136, 48], "intrinsicMatches": true, "cacheHit": true, "pass": true },
    { "name": "TextBox_normal_light", "control": "TextBox", "state": "normal", "theme": "light", "backdrop": "solid", "status": "invalid", "meanDe": 0.0000, "p95De": 0.0000, "p99De": 0.0000, "overLimitRatio": 0.000000, "referenceBbox": [0, 0, 0, 0], "oursBbox": [0, 0, 0, 0], "intrinsicMatches": false, "cacheHit": false, "pass": false },
    { "name": "Slider_hover_dark", "control": "Slider", "state": "hover", "theme": "dark", "backdrop": "acrylic", "status": "fail", "meanDe": 4.8500, "p95De": 9.1000, "p99De": 12.0000, "overLimitRatio": 0.140000, "referenceBbox": [10, 10, 190, 42], "oursBbox": [10, 10, 190, 42], "intrinsicMatches": true, "cacheHit": false, "pass": false }
  ]
}"#;

    fn sample_report() -> ParityReport {
        let cases = parse_cases(MULTI_STATE_REPORT);
        ParityReport {
            passed: cases.iter().filter(|c| c.status == ParityStatus::Pass).count(),
            failed: cases.iter().filter(|c| c.status == ParityStatus::Fail).count(),
            invalid: cases.iter().filter(|c| c.status == ParityStatus::Invalid).count(),
            exempt: cases.iter().filter(|c| c.status == ParityStatus::Exempt).count(),
            cache_hits: cases.iter().filter(|c| c.cache_hit).count(),
            cases,
            error: None,
            hint: String::new(),
        }
    }

    #[test]
    fn report_lines_expose_four_gates_backdrop_and_three_states() {
        let cases = parse_cases(MULTI_STATE_REPORT);
        assert_eq!(cases.len(), 3);
        let passed = &cases[0];
        assert_eq!(passed.status, ParityStatus::Pass);
        assert_eq!(passed.backdrop, "mica");
        assert!(passed.intrinsic_matches);
        assert!(passed.cache_hit);
        assert!((passed.mean_de - 0.42).abs() < 1e-6);
        assert_eq!(passed.bbox_delta, [0, 0, 0, 0]);
    }

    #[test]
    fn auto_focus_prioritizes_issues_and_sorts_failures_first() {
        let report = sample_report();
        let mut inspector = ParityInspectorState::default();
        inspector.auto_focus_for_report(&report);
        assert_eq!(inspector.status_filter, ParityStatusFilter::Issues);
        let filtered = filtered_case_indices(&report, &inspector, "");
        assert_eq!(filtered.len(), 2);
        assert_eq!(report.cases[filtered[0]].name, "Slider_hover_dark");
        assert_eq!(report.cases[filtered[1]].name, "TextBox_normal_light");
    }

    #[test]
    fn curtain_hover_scrub_respects_lock_state() {
        let mut inspector = ParityInspectorState {
            view_mode: ParityViewMode::Curtain,
            ..Default::default()
        };
        inspector.curtain_stage_rect.set(Some(Rect::new(100.0, 100.0, 400.0, 200.0)));
        assert!(inspector.scrub_curtain(Vec2::new(400.0, 150.0)));
        assert!((inspector.curtain_ratio - 0.75).abs() < 1e-3);
        inspector.curtain_locked = true;
        assert!(!inspector.scrub_curtain(Vec2::new(200.0, 150.0)));
        assert!((inspector.curtain_ratio - 0.75).abs() < 1e-3);
    }

    #[test]
    fn legacy_report_lines_without_status_fall_back_to_pass_flag() {
        let json = r#"    { "name": "Button_normal_light", "control": "Button", "state": "normal", "theme": "light", "backdrop": "solid", "meanDe": 0.1000, "p95De": 0.2000, "overLimitRatio": 0.000000, "intrinsicMatches": true, "cacheHit": false, "pass": true }"#;
        let cases = parse_cases(json);
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].status, ParityStatus::Pass);
    }
}
