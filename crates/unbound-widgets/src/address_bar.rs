//! 双模路径导航地址栏（`AddressBar`）：
//! 外部 `← → ↑ ↻` 导航按钮簇 + 一体化面包屑/原位文本编辑双模地址框 + 右侧可折叠内联搜索框。

use std::ops::Deref;

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, KeyChord, KeyName};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::shell::{emit_shell_icon_primitives, ShellIcon};
use crate::text_input::{
    floor_boundary, focus_underline, reduce_text_input, TextEditState, TextInputDesc,
    TextInputEvent, TextInputOutcome, TextInputVisual,
};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 地址栏标准高度（`32px`）。
pub const ADDRESS_BAR_HEIGHT: f32 = 32.0;
/// 外部导航按钮尺寸（`28×28px`）。
const NAV_BUTTON_SIZE: f32 = 28.0;
/// 外部导航按钮间距（`2px`）。
const NAV_BUTTON_GAP: f32 = 2.0;
/// 三段区域之间的水平间距（`6px`）。
const SECTION_GAP: f32 = 6.0;
/// 地址框与搜索框圆角（`4px`）。
const BAR_RADIUS: f32 = 4.0;
/// 内部按钮与分段胶囊圆角（`4px`）。
const INNER_RADIUS: f32 = 4.0;
/// 地址框左侧根图标尺寸（`16×16px`）。
const ROOT_ICON_SIZE: f32 = 16.0;
/// 面包屑末段右侧保留的最小空白点击编辑触发区宽度（`36px`）。
const MIN_EDIT_TRIGGER_WIDTH: f32 = 36.0;
/// 溢出折叠按钮（`«`）宽度（`24px`）。
const OVERFLOW_BUTTON_WIDTH: f32 = 24.0;
/// 根图标右侧分隔箭头（`›`）宽度（`14px`）。
const ROOT_CHEVRON_WIDTH: f32 = 14.0;
/// 分段右侧子目录展开箭头（`›`）宽度（`18px`）。
const CHEVRON_WIDTH: f32 = 18.0;
/// 地址栏默认深度键（与 `TreeViewDesc` 对齐，确保绘制在容器 `BoxSurface` 背景之上）。
const DEFAULT_ADDRESS_BAR_Z: u16 = 450;
/// 启用搜索框时，低于该总宽则自动折叠为图标按钮。
const SEARCH_COLLAPSE_THRESHOLD: f32 = 420.0;

/// 面包屑路径分段描述。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathSegment {
    /// 展示名称（如本地化名称或目录名）。
    pub label: String,
    /// 该段对应的完整规范路径。
    pub path: String,
    /// 可选图标（首段常用于标识根/驱动器）。
    pub icon: Option<ShellIcon>,
    /// 是否存在子目录（决定是否渲染可点击的 `›` 箭头）。
    pub has_subfolders: bool,
}

impl PathSegment {
    /// 创建带标签与规范路径的面包屑分段（默认 `has_subfolders = true`）。
    #[must_use]
    pub fn new(label: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
            icon: None,
            has_subfolders: true,
        }
    }

    /// 设置该分段的可选图标。
    #[must_use]
    pub fn with_icon(mut self, icon: ShellIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// 设置该分段是否存在子目录（控制 `›` 箭头显隐）。
    #[must_use]
    pub fn with_subfolders(mut self, has_subfolders: bool) -> Self {
        self.has_subfolders = has_subfolders;
        self
    }
}

impl Deref for PathSegment {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.label
    }
}

impl AsRef<str> for PathSegment {
    fn as_ref(&self) -> &str {
        &self.label
    }
}

/// 双模地址栏离散交互事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddressBarEvent {
    /// 点击外部后退按钮（`←`）。
    NavigateBack,
    /// 点击外部前进按钮（`→`）。
    NavigateForward,
    /// 点击外部向上一级按钮（`↑`）。
    NavigateUp,
    /// 点击外部刷新按钮（`↻`）。
    Refresh,
    /// 点击指定面包屑分段，跳转到对应路径。
    SegmentClicked { index: usize, path: String },
    /// 点击面包屑 `›` 箭头或 `«` 溢出按钮，请求在指定锚点矩形弹出菜单。
    SubfolderMenuRequested {
        index: usize,
        path: String,
        anchor_rect: [i32; 4],
    },
    /// 在原位编辑态按 `Enter` 提交路径文本。
    PathSubmitted(String),
    /// 取消原位路径编辑并恢复面包屑态。
    EditCancelled,
    /// 右侧内联搜索框内容发生变化。
    SearchChanged(String),
    /// 在右侧内联搜索框中按 `Enter` 提交搜索。
    SearchSubmitted(String),
}

/// 地址栏精确命中区域类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddressBarHitKind {
    /// 外部后退按钮（`←`）。
    NavBack,
    /// 外部前进按钮（`→`）。
    NavForward,
    /// 外部向上一级按钮（`↑`）。
    NavUp,
    /// 外部刷新按钮（`↻`）。
    Refresh,
    /// 面包屑前段溢出折叠按钮（`«`）。
    OverflowButton,
    /// 第 `index` 个面包屑分段标签热区。
    Segment(usize),
    /// 第 `index` 个面包屑分段右侧 `›` 子目录箭头热区。
    SegmentChevron(usize),
    /// 末段右侧空白编辑触发区（或编辑态文本框主体）。
    EditTrigger,
    /// 右侧内联搜索框主体。
    SearchBox,
    /// 右侧内联搜索框清除按钮（`×`）。
    SearchClear,
}

/// 单个可见面包屑分段的排版矩形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentLayout {
    /// 在 `desc.segments` 中的原始索引。
    pub index: usize,
    /// 段名按钮矩形。
    pub label_rect: Rect,
    /// 可选 `›` 子目录展开箭头矩形（仅 `has_subfolders == true` 时为 `Some`）。
    pub chevron_rect: Option<Rect>,
}

/// 地址栏三段式确定性几何布局（供绘制器与命中测试共用）。
#[derive(Clone, Debug, PartialEq)]
pub struct AddressBarLayout {
    /// 后退按钮矩形。
    pub back_rect: Option<Rect>,
    /// 前进按钮矩形。
    pub forward_rect: Option<Rect>,
    /// 向上按钮矩形。
    pub up_rect: Option<Rect>,
    /// 刷新按钮矩形。
    pub refresh_rect: Option<Rect>,
    /// 一体化双模主地址框矩形。
    pub address_rect: Rect,
    /// 左侧 `16×16` 根图标矩形。
    pub root_icon_rect: Rect,
    /// 根图标右侧与首个分段之间的分隔箭头（`›`）矩形。
    pub root_chevron_rect: Option<Rect>,
    /// 被折叠的前段数量（大于 0 时渲染 `«` 按钮）。
    pub overflow_count: usize,
    /// `«` 溢出按钮矩形。
    pub overflow_rect: Option<Rect>,
    /// 可见面包屑分段列表。
    pub segments: Vec<SegmentLayout>,
    /// 末段右侧空白编辑触发区矩形。
    pub edit_trigger_rect: Rect,
    /// 可选右侧内联搜索框矩形。
    pub search_rect: Option<Rect>,
    /// 搜索框是否因窄窗折叠为图标按钮。
    pub search_collapsed: bool,
    /// 搜索框放大镜图标矩形。
    pub search_icon_rect: Option<Rect>,
    /// 搜索框清除 `×` 按钮矩形。
    pub search_clear_rect: Option<Rect>,
}

/// 双模地址栏描述符（平时显示分段路径面包屑，点击空白或按下 `Ctrl+L` / `Alt+D` 切换为完整路径编辑框）。
#[derive(Clone, Debug, PartialEq)]
pub struct AddressBarDesc {
    pub rect: Rect,
    pub full_path: String,
    pub segments: Vec<PathSegment>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub can_go_up: bool,
    pub show_nav_buttons: bool,
    pub show_refresh: bool,
    pub show_search: bool,
    pub search_placeholder: String,
    pub z: u16,
}

impl AddressBarDesc {
    /// 从字符串分段列表构建地址栏描述符（保持既有调用点向后兼容）。
    #[must_use]
    pub fn new(rect: Rect, segments: Vec<String>) -> Self {
        let mut path_segments = Vec::with_capacity(segments.len());
        let mut acc = String::new();
        for (idx, label) in segments.into_iter().enumerate() {
            if idx > 0 {
                acc.push('/');
            }
            acc.push_str(&label);
            let mut seg = PathSegment::new(label, acc.clone());
            if idx == 0 {
                seg.icon = Some(ShellIcon::Folder);
            }
            path_segments.push(seg);
        }
        let full_path = if acc.is_empty() {
            "/".to_owned()
        } else {
            acc
        };
        let can_go_up = path_segments.len() > 1;
        Self {
            rect,
            full_path,
            segments: path_segments,
            can_go_back: true,
            can_go_forward: false,
            can_go_up,
            show_nav_buttons: true,
            show_refresh: true,
            show_search: false,
            search_placeholder: String::new(),
            z: DEFAULT_ADDRESS_BAR_Z,
        }
    }

    /// 使用结构化 [`PathSegment`] 列表与完整路径构造地址栏描述符。
    #[must_use]
    pub fn with_path_segments(
        rect: Rect,
        full_path: impl Into<String>,
        segments: Vec<PathSegment>,
    ) -> Self {
        let can_go_up = segments.len() > 1;
        Self {
            rect,
            full_path: full_path.into(),
            segments,
            can_go_back: false,
            can_go_forward: false,
            can_go_up,
            show_nav_buttons: true,
            show_refresh: true,
            show_search: false,
            search_placeholder: String::new(),
            z: DEFAULT_ADDRESS_BAR_Z,
        }
    }

    /// 配置后退、前进与向上导航按钮的可用状态。
    #[must_use]
    pub fn with_nav_state(
        mut self,
        can_go_back: bool,
        can_go_forward: bool,
        can_go_up: bool,
    ) -> Self {
        self.can_go_back = can_go_back;
        self.can_go_forward = can_go_forward;
        self.can_go_up = can_go_up;
        self
    }

    /// 控制是否显示外部导航按钮簇（后退/前进/向上）。
    #[must_use]
    pub fn with_nav_buttons(mut self, show: bool) -> Self {
        self.show_nav_buttons = show;
        self
    }

    /// 控制是否显示外部刷新按钮。
    #[must_use]
    pub fn with_refresh(mut self, show: bool) -> Self {
        self.show_refresh = show;
        self
    }

    /// 启用右侧内联搜索框并设置动态占位文本。
    #[must_use]
    pub fn with_search(mut self, placeholder: impl Into<String>) -> Self {
        self.show_search = true;
        self.search_placeholder = placeholder.into();
        self
    }

    /// 隐藏右侧内联搜索框。
    #[must_use]
    pub fn without_search(mut self) -> Self {
        self.show_search = false;
        self
    }

    /// 设置绘制深度键。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 完整路径字符串（优先返回显式 `full_path`，否则以 `/` 连接分段）。
    #[must_use]
    pub fn full_path(&self) -> String {
        if !self.full_path.is_empty() {
            return self.full_path.clone();
        }
        if self.segments.is_empty() {
            "/".to_owned()
        } else {
            self.segments
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>()
                .join("/")
        }
    }
}

/// 估算面包屑分段标签的逻辑像素宽度（非 ASCII / CJK 宽字符按 `13.8px`、ASCII 按 `7.5px` 估算）。
#[must_use]
fn estimate_segment_label_width(label: &str) -> f32 {
    let text_w: f32 = label
        .chars()
        .map(|ch| if ch.is_ascii() { 7.5 } else { 13.8 })
        .sum();
    (text_w + 16.0).clamp(32.0, 220.0)
}

/// 计算地址栏三段式确定性几何布局。
#[must_use]
pub fn compute_layout(desc: &AddressBarDesc) -> AddressBarLayout {
    let r = desc.rect;
    let btn_size = NAV_BUTTON_SIZE.min(r.size.y.max(16.0));
    let btn_y = r.origin.y + (r.size.y - btn_size) * 0.5;
    let mut left_x = r.origin.x;

    let (back_rect, forward_rect, up_rect) = if desc.show_nav_buttons {
        let back = Rect::new(left_x, btn_y, btn_size, btn_size);
        left_x += btn_size + NAV_BUTTON_GAP;
        let forward = Rect::new(left_x, btn_y, btn_size, btn_size);
        left_x += btn_size + NAV_BUTTON_GAP;
        let up = Rect::new(left_x, btn_y, btn_size, btn_size);
        left_x += btn_size + NAV_BUTTON_GAP;
        (Some(back), Some(forward), Some(up))
    } else {
        (None, None, None)
    };

    let refresh_rect = if desc.show_refresh {
        let refresh = Rect::new(left_x, btn_y, btn_size, btn_size);
        left_x += btn_size + NAV_BUTTON_GAP;
        Some(refresh)
    } else {
        None
    };

    if desc.show_nav_buttons || desc.show_refresh {
        left_x += SECTION_GAP - NAV_BUTTON_GAP;
    }

    let (search_rect, search_collapsed, search_icon_rect, search_clear_rect, right_limit) =
        if desc.show_search {
            let collapsed = r.size.x < SEARCH_COLLAPSE_THRESHOLD;
            let sw = if collapsed {
                r.size.y.clamp(28.0, 32.0)
            } else {
                (r.size.x * 0.28).clamp(200.0, 260.0)
            };
            let sx = (r.max().x - sw).max(left_x);
            let sr = Rect::new(sx, r.origin.y, (r.max().x - sx).max(0.0), r.size.y);
            let icon_r = if collapsed {
                let icon_s = 24.0f32.min(sr.size.y).min(sr.size.x);
                let icon_y = sr.origin.y + (sr.size.y - icon_s) * 0.5;
                Rect::new(
                    sr.origin.x + (sr.size.x - icon_s) * 0.5,
                    icon_y,
                    icon_s,
                    icon_s,
                )
            } else {
                // Windows 11 资源管理器标准：非折叠搜索框放大镜图标位于左侧
                let icon_s = 16.0f32.min(sr.size.y).min(sr.size.x);
                let icon_y = sr.origin.y + (sr.size.y - icon_s) * 0.5;
                let icon_x = (sr.origin.x + 8.0).min((sr.max().x - icon_s).max(sr.origin.x));
                Rect::new(icon_x, icon_y, icon_s, icon_s)
            };
            let clear_r = if collapsed {
                None
            } else {
                // 清除按钮固定位于搜索框最右侧，确保文本区严格落在图标与清除按钮之间
                let clear_s = 22.0f32.min(sr.size.y).min(sr.size.x);
                let clear_y = sr.origin.y + (sr.size.y - clear_s) * 0.5;
                let clear_x = (sr.max().x - clear_s - 4.0)
                    .max(icon_r.max().x + 4.0)
                    .min((sr.max().x - clear_s).max(sr.origin.x));
                Some(Rect::new(clear_x, clear_y, clear_s, clear_s))
            };
            (
                Some(sr),
                collapsed,
                Some(icon_r),
                clear_r,
                (sx - SECTION_GAP).max(left_x),
            )
        } else {
            (None, false, None, None, r.max().x)
        };

    let address_rect = Rect::new(
        left_x,
        r.origin.y,
        (right_limit - left_x).max(0.0),
        r.size.y,
    );
    let root_icon_rect = Rect::new(
        address_rect.origin.x + 8.0,
        address_rect.origin.y + (address_rect.size.y - ROOT_ICON_SIZE) * 0.5,
        ROOT_ICON_SIZE,
        ROOT_ICON_SIZE,
    );

    let inner_y = address_rect.origin.y + 3.0;
    let inner_h = (address_rect.size.y - 6.0).max(16.0);
    // 根图标后置 `›` 分隔符，使面包屑呈现 `[图标] > 分段1 > 分段2 >`
    let root_chevron_rect = if !desc.segments.is_empty()
        && root_icon_rect.max().x + 2.0 + ROOT_CHEVRON_WIDTH <= address_rect.max().x
    {
        Some(Rect::new(
            root_icon_rect.max().x + 2.0,
            inner_y,
            ROOT_CHEVRON_WIDTH,
            inner_h,
        ))
    } else {
        None
    };
    let content_start_x = root_chevron_rect.map_or(root_icon_rect.max().x + 4.0, |rc| rc.max().x + 2.0);
    let available_total =
        (address_rect.max().x - content_start_x - MIN_EDIT_TRIGGER_WIDTH).max(0.0);

    // 预估每段所需宽度（标签 + 可选 `›` 箭头），对 CJK 宽字符按双倍宽度系数估算
    let seg_widths: Vec<(f32, f32)> = desc
        .segments
        .iter()
        .map(|seg| {
            let label_w = estimate_segment_label_width(&seg.label);
            let chev_w = if seg.has_subfolders {
                CHEVRON_WIDTH
            } else {
                0.0
            };
            (label_w, chev_w)
        })
        .collect();

    let total_segs_w: f32 = seg_widths
        .iter()
        .map(|(lw, cw)| lw + cw + 2.0)
        .sum();

    let mut overflow_count = 0usize;
    if total_segs_w > available_total && desc.segments.len() > 1 {
        let budget_with_overflow = (available_total - OVERFLOW_BUTTON_WIDTH - 2.0).max(0.0);
        let mut remaining_w = total_segs_w;
        while overflow_count + 1 < desc.segments.len() && remaining_w > budget_with_overflow {
            let (lw, cw) = seg_widths[overflow_count];
            remaining_w -= lw + cw + 2.0;
            overflow_count += 1;
        }
    }

    let mut cursor_x = content_start_x;
    let overflow_rect = if overflow_count > 0 {
        let ov = Rect::new(cursor_x, inner_y, OVERFLOW_BUTTON_WIDTH, inner_h);
        cursor_x = ov.max().x + 2.0;
        Some(ov)
    } else {
        None
    };

    let mut segments = Vec::with_capacity(desc.segments.len().saturating_sub(overflow_count));
    let seg_right_limit = (address_rect.max().x - MIN_EDIT_TRIGGER_WIDTH).max(cursor_x);
    for (idx, &(label_w, chev_w)) in seg_widths.iter().enumerate().skip(overflow_count) {
        let max_w = (seg_right_limit - cursor_x).max(0.0);
        if max_w < 16.0 && !segments.is_empty() {
            break;
        }
        let actual_label_w = label_w.min((max_w - chev_w).max(16.0));
        let label_rect = Rect::new(cursor_x, inner_y, actual_label_w, inner_h);
        cursor_x = label_rect.max().x;
        let chevron_rect = if chev_w > 0.0 && cursor_x + chev_w <= address_rect.max().x {
            let cr = Rect::new(cursor_x, inner_y, chev_w, inner_h);
            cursor_x = cr.max().x;
            Some(cr)
        } else {
            None
        };
        cursor_x += 2.0;
        segments.push(SegmentLayout {
            index: idx,
            label_rect,
            chevron_rect,
        });
    }

    let edit_trigger_rect = Rect::new(
        cursor_x.min(address_rect.max().x),
        address_rect.origin.y,
        (address_rect.max().x - cursor_x).max(0.0),
        address_rect.size.y,
    );

    AddressBarLayout {
        back_rect,
        forward_rect,
        up_rect,
        refresh_rect,
        address_rect,
        root_icon_rect,
        root_chevron_rect,
        overflow_count,
        overflow_rect,
        segments,
        edit_trigger_rect,
        search_rect,
        search_collapsed,
        search_icon_rect,
        search_clear_rect,
    }
}

/// 双模地址栏交互状态（内嵌复用 [`TextEditState`] 编辑核心）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddressBarState {
    /// 是否处于全路径文本编辑模式。
    pub editing: bool,
    /// 编辑缓冲区内容（与 `edit_core.text()` 同步，保持向后兼容）。
    pub edit_buffer: String,
    /// 历史后退栈。
    pub back_stack: Vec<Vec<String>>,
    /// 历史前进栈。
    pub forward_stack: Vec<Vec<String>>,
    /// 全路径原位编辑核心（支持光标、选区、撤销重做、剪贴板与 IME）。
    pub edit_core: TextEditState,
    /// 右侧内联搜索框编辑核心。
    pub search_core: TextEditState,
    /// 右侧内联搜索框是否处于聚焦态。
    pub search_focused: bool,
    /// 待宿主消费的离散交互事件队列。
    pub pending_events: Vec<AddressBarEvent>,
    /// 缓存的后退可用状态。
    pub can_go_back: bool,
    /// 缓存的前进可用状态。
    pub can_go_forward: bool,
    /// 缓存的向上一级可用状态。
    pub can_go_up: bool,
    /// 当前指针悬停的子热区（用于高亮对应按钮或分段）。
    pub hovered_hit: Option<AddressBarHitKind>,
}

impl AddressBarState {
    /// 创建默认地址栏状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 取出并清空待处理的地址栏交互事件。
    pub fn drain_events(&mut self) -> Vec<AddressBarEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// 切换进入全路径编辑态并全选路径文本。
    pub fn begin_edit(&mut self, current_path: &str) {
        self.editing = true;
        self.search_focused = false;
        self.edit_buffer = current_path.to_owned();
        self.edit_core.set_text(current_path);
        self.edit_core.select_all();
    }

    /// 提交编辑路径、发射 [`AddressBarEvent::PathSubmitted`] 并解析为分段列表。
    pub fn commit_edit(&mut self) -> Vec<String> {
        self.editing = false;
        let text = if self.edit_core.text().is_empty() && !self.edit_buffer.is_empty() {
            self.edit_buffer.clone()
        } else {
            self.edit_core.text().to_owned()
        };
        self.edit_buffer = text.clone();
        self.pending_events
            .push(AddressBarEvent::PathSubmitted(text.clone()));
        text.split(['/', '\\'])
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// 取消全路径编辑并恢复面包屑态，发射 [`AddressBarEvent::EditCancelled`]。
    pub fn cancel_edit(&mut self) {
        if self.editing {
            self.editing = false;
            self.pending_events.push(AddressBarEvent::EditCancelled);
        }
    }

    /// 失焦处理：若正在编辑路径则取消编辑；同时取消搜索框聚焦。
    pub fn handle_blur(&mut self) {
        self.cancel_edit();
        self.search_focused = false;
    }

    /// 清空右侧内联搜索框内容并发射 `SearchChanged("")`。
    pub fn clear_search(&mut self) -> bool {
        if self.search_core.text().is_empty() {
            return false;
        }
        self.search_core.set_text("");
        self.pending_events
            .push(AddressBarEvent::SearchChanged(String::new()));
        true
    }

    /// 对指定坐标执行精确命中测试。
    #[must_use]
    pub fn hit_test(&self, desc: &AddressBarDesc, point: Vec2) -> Option<AddressBarHitKind> {
        let layout = compute_layout(desc);
        if layout.back_rect.is_some_and(|r| r.contains_point(point)) {
            return Some(AddressBarHitKind::NavBack);
        }
        if layout.forward_rect.is_some_and(|r| r.contains_point(point)) {
            return Some(AddressBarHitKind::NavForward);
        }
        if layout.up_rect.is_some_and(|r| r.contains_point(point)) {
            return Some(AddressBarHitKind::NavUp);
        }
        if layout.refresh_rect.is_some_and(|r| r.contains_point(point)) {
            return Some(AddressBarHitKind::Refresh);
        }
        if let Some(sr) = layout.search_rect {
            if sr.contains_point(point) {
                if !layout.search_collapsed
                    && !self.search_core.text().is_empty()
                    && layout
                        .search_clear_rect
                        .is_some_and(|cr| cr.contains_point(point))
                {
                    return Some(AddressBarHitKind::SearchClear);
                }
                return Some(AddressBarHitKind::SearchBox);
            }
        }
        if layout.address_rect.contains_point(point) {
            if self.editing {
                return Some(AddressBarHitKind::EditTrigger);
            }
            if layout.overflow_rect.is_some_and(|r| r.contains_point(point)) {
                return Some(AddressBarHitKind::OverflowButton);
            }
            for seg in &layout.segments {
                if seg.chevron_rect.is_some_and(|r| r.contains_point(point)) {
                    return Some(AddressBarHitKind::SegmentChevron(seg.index));
                }
                if seg.label_rect.contains_point(point) {
                    return Some(AddressBarHitKind::Segment(seg.index));
                }
            }
            return Some(AddressBarHitKind::EditTrigger);
        }
        None
    }

    /// 处理指针左键点击事件（纯状态机，返回是否消费该点击）。
    pub fn handle_click(&mut self, desc: &AddressBarDesc, point: Vec2) -> bool {
        self.can_go_back = desc.can_go_back;
        self.can_go_forward = desc.can_go_forward;
        self.can_go_up = desc.can_go_up;

        let layout = compute_layout(desc);
        let hit = self.hit_test(desc, point);
        match hit {
            Some(AddressBarHitKind::NavBack) => {
                if !desc.can_go_back {
                    return false;
                }
                self.pending_events.push(AddressBarEvent::NavigateBack);
                true
            }
            Some(AddressBarHitKind::NavForward) => {
                if !desc.can_go_forward {
                    return false;
                }
                self.pending_events.push(AddressBarEvent::NavigateForward);
                true
            }
            Some(AddressBarHitKind::NavUp) => {
                if !desc.can_go_up {
                    return false;
                }
                self.pending_events.push(AddressBarEvent::NavigateUp);
                true
            }
            Some(AddressBarHitKind::Refresh) => {
                self.pending_events.push(AddressBarEvent::Refresh);
                true
            }
            Some(AddressBarHitKind::OverflowButton) => {
                if let (Some(ov_r), Some(first)) = (layout.overflow_rect, desc.segments.first()) {
                    self.pending_events
                        .push(AddressBarEvent::SubfolderMenuRequested {
                            index: 0,
                            path: first.path.clone(),
                            anchor_rect: rect_to_i32(ov_r),
                        });
                    return true;
                }
                false
            }
            Some(AddressBarHitKind::Segment(index)) => {
                if let Some(seg) = desc.segments.get(index) {
                    self.pending_events.push(AddressBarEvent::SegmentClicked {
                        index,
                        path: seg.path.clone(),
                    });
                    return true;
                }
                false
            }
            Some(AddressBarHitKind::SegmentChevron(index)) => {
                if let (Some(seg), Some(seg_layout)) = (
                    desc.segments.get(index),
                    layout.segments.iter().find(|s| s.index == index),
                ) {
                    if let Some(chev_r) = seg_layout.chevron_rect {
                        self.pending_events
                            .push(AddressBarEvent::SubfolderMenuRequested {
                                index,
                                path: seg.path.clone(),
                                anchor_rect: rect_to_i32(chev_r),
                            });
                        return true;
                    }
                }
                false
            }
            Some(AddressBarHitKind::EditTrigger) => {
                if !self.editing {
                    self.begin_edit(&desc.full_path());
                }
                true
            }
            Some(AddressBarHitKind::SearchClear) => self.clear_search(),
            Some(AddressBarHitKind::SearchBox) => {
                self.cancel_edit();
                self.search_focused = true;
                true
            }
            None => {
                let was_active = self.editing || self.search_focused;
                self.handle_blur();
                was_active
            }
        }
    }

    /// 处理键盘快捷键与文本编辑按键（含 `Ctrl+L`、`Alt+D`、`Enter`、`Esc`）。
    pub fn handle_key(&mut self, desc: &AddressBarDesc, chord: KeyChord) -> bool {
        // `Ctrl+L` 或 `Alt+D`：随时切入全路径原位编辑态并全选
        if (chord.ctrl && matches!(chord.key, KeyName::Char('l' | 'L')))
            || (chord.alt && matches!(chord.key, KeyName::Char('d' | 'D')))
        {
            self.begin_edit(&desc.full_path());
            return true;
        }

        if self.editing {
            match chord.key {
                KeyName::Named("Enter") => {
                    self.commit_edit();
                    return true;
                }
                KeyName::Named("Escape") => {
                    self.cancel_edit();
                    return true;
                }
                _ => {
                    if let Some(ev) = chord_to_text_event(chord) {
                        let outcome = self.handle_text_event(desc, &ev);
                        return outcome.changed
                            || outcome.copied.is_some()
                            || outcome.cut.is_some()
                            || outcome.paste_requested;
                    }
                }
            }
            return false;
        }

        if self.search_focused {
            match chord.key {
                KeyName::Named("Enter") => {
                    self.pending_events.push(AddressBarEvent::SearchSubmitted(
                        self.search_core.text().to_owned(),
                    ));
                    return true;
                }
                KeyName::Named("Escape") => {
                    self.search_focused = false;
                    return true;
                }
                _ => {
                    if let Some(ev) = chord_to_text_event(chord) {
                        let outcome = self.handle_text_event(desc, &ev);
                        return outcome.changed
                            || outcome.copied.is_some()
                            || outcome.cut.is_some()
                            || outcome.paste_requested;
                    }
                }
            }
        }

        false
    }

    /// 将完整文本输入事件（含 IME 预编辑/提交/取消、剪贴板粘贴/复制）分发至当前激活的 `TextEditState`。
    pub fn handle_text_event(
        &mut self,
        desc: &AddressBarDesc,
        event: &TextInputEvent,
    ) -> TextInputOutcome {
        if self.editing {
            let editor_desc = TextInputDesc::new(desc.rect);
            let outcome = reduce_text_input(&editor_desc, &mut self.edit_core, event);
            if outcome.changed {
                self.edit_buffer = self.edit_core.text().to_owned();
            }
            return outcome;
        }
        if self.search_focused {
            let prev = self.search_core.text().to_owned();
            let editor_desc = TextInputDesc::new(desc.rect);
            let outcome = reduce_text_input(&editor_desc, &mut self.search_core, event);
            if self.search_core.text() != prev {
                self.pending_events.push(AddressBarEvent::SearchChanged(
                    self.search_core.text().to_owned(),
                ));
            }
            return outcome;
        }
        TextInputOutcome::default()
    }
}

/// 为地址栏子热区生成确定性子节点键。
#[must_use]
pub const fn address_bar_sub_node(node: u64, kind: AddressBarHitKind) -> u64 {
    let salt: u64 = match kind {
        AddressBarHitKind::NavBack => 0x4142_0001,
        AddressBarHitKind::NavForward => 0x4142_0002,
        AddressBarHitKind::NavUp => 0x4142_0003,
        AddressBarHitKind::Refresh => 0x4142_0004,
        AddressBarHitKind::OverflowButton => 0x4142_0005,
        AddressBarHitKind::EditTrigger => 0x4142_0006,
        AddressBarHitKind::SearchBox => 0x4142_0007,
        AddressBarHitKind::SearchClear => 0x4142_0008,
        AddressBarHitKind::Segment(idx) => 0x4142_1000 + idx as u64,
        AddressBarHitKind::SegmentChevron(idx) => 0x4142_2000 + idx as u64,
    };
    node ^ salt
}

fn rect_to_i32(r: Rect) -> [i32; 4] {
    [
        r.origin.x.round() as i32,
        r.origin.y.round() as i32,
        r.size.x.round() as i32,
        r.size.y.round() as i32,
    ]
}

fn chord_to_text_event(chord: KeyChord) -> Option<TextInputEvent> {
    if chord.ctrl && !chord.alt && !chord.meta {
        return match chord.key {
            KeyName::Char('a' | 'A') => Some(TextInputEvent::SelectAll),
            KeyName::Char('c' | 'C') => Some(TextInputEvent::Copy),
            KeyName::Char('x' | 'X') => Some(TextInputEvent::Cut),
            KeyName::Char('z' | 'Z') if chord.shift => Some(TextInputEvent::Redo),
            KeyName::Char('z' | 'Z') => Some(TextInputEvent::Undo),
            KeyName::Char('y' | 'Y') => Some(TextInputEvent::Redo),
            KeyName::Named("ArrowLeft") => Some(TextInputEvent::MoveWordLeft {
                extend: chord.shift,
            }),
            KeyName::Named("ArrowRight") => Some(TextInputEvent::MoveWordRight {
                extend: chord.shift,
            }),
            KeyName::Named("Backspace") => Some(TextInputEvent::DeleteWordBack),
            KeyName::Named("Delete") => Some(TextInputEvent::DeleteWordForward),
            _ => None,
        };
    }
    if chord.alt || chord.meta {
        return None;
    }
    match chord.key {
        KeyName::Named("Backspace") => Some(TextInputEvent::Backspace),
        KeyName::Named("Delete") => Some(TextInputEvent::Delete),
        KeyName::Named("ArrowLeft") => Some(TextInputEvent::MoveLeft {
            extend: chord.shift,
        }),
        KeyName::Named("ArrowRight") => Some(TextInputEvent::MoveRight {
            extend: chord.shift,
        }),
        KeyName::Named("Home") => Some(TextInputEvent::Home {
            extend: chord.shift,
        }),
        KeyName::Named("End") => Some(TextInputEvent::End {
            extend: chord.shift,
        }),
        KeyName::Char(ch) if !ch.is_control() => Some(TextInputEvent::Insert(ch.to_string())),
        _ => None,
    }
}

/// 绘制左箭头矢量几何（恒定 3 条线段）。
fn emit_arrow_left_icon(
    rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let c = rect.center();
    curve_stroke([c.x + 5.0, c.y], [c.x - 5.0, c.y], 1.6, color, z, primitives, payloads);
    curve_stroke(
        [c.x - 1.0, c.y - 4.0],
        [c.x - 5.0, c.y],
        1.6,
        color,
        z,
        primitives,
        payloads,
    );
    curve_stroke(
        [c.x - 1.0, c.y + 4.0],
        [c.x - 5.0, c.y],
        1.6,
        color,
        z,
        primitives,
        payloads,
    );
}

/// 绘制刷新环箭头矢量几何（恒定 1 个圆环 + 2 条箭头线段）。
fn emit_refresh_icon(
    rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let c = rect.center();
    primitives.push(outlined(
        Rect::new(c.x - 5.5, c.y - 5.5, 11.0, 11.0),
        5.5,
        Token::NONE,
        color,
        1.5,
        0.0,
        z,
    ));
    curve_stroke(
        [c.x + 2.0, c.y - 5.5],
        [c.x + 5.8, c.y - 5.5],
        1.5,
        color,
        z + 1,
        primitives,
        payloads,
    );
    curve_stroke(
        [c.x + 5.8, c.y - 5.5],
        [c.x + 5.8, c.y - 1.7],
        1.5,
        color,
        z + 1,
        primitives,
        payloads,
    );
}

/// 绘制双左折线溢出图标 `«`（恒定 4 条线段）。
fn emit_overflow_chevrons(
    rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let c = rect.center();
    for dx in [-2.8f32, 2.2] {
        let cx = c.x + dx;
        curve_stroke(
            [cx + 2.0, c.y - 3.2],
            [cx - 1.8, c.y],
            1.4,
            color,
            z,
            primitives,
            payloads,
        );
        curve_stroke(
            [cx - 1.8, c.y],
            [cx + 2.0, c.y + 3.2],
            1.4,
            color,
            z,
            primitives,
            payloads,
        );
    }
}

/// 绘制单个分段右侧子目录展开箭头 `›`（恒定 2 条线段）。
fn emit_segment_chevron(
    rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let c = rect.center();
    curve_stroke(
        [c.x - 1.8, c.y - 3.2],
        [c.x + 2.0, c.y],
        1.4,
        color,
        z,
        primitives,
        payloads,
    );
    curve_stroke(
        [c.x + 2.0, c.y],
        [c.x - 1.8, c.y + 3.2],
        1.4,
        color,
        z,
        primitives,
        payloads,
    );
}

/// 地址栏编辑态与内联搜索框的已测量文本视觉信息（光标、选区与 IME 预编辑下划线）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AddressBarTextVisual {
    /// 主地址栏编辑态光标、选区与 IME 几何信息（仅 `state.editing == true` 时生效）。
    pub edit_visual: TextInputVisual,
    /// 右侧内联搜索框光标、选区与 IME 几何信息（仅 `state.search_focused == true` 且非折叠时生效）。
    pub search_visual: TextInputVisual,
}

impl Default for AddressBarTextVisual {
    fn default() -> Self {
        let hidden = TextInputVisual {
            caret: Rect::ZERO,
            selection: None,
            preedit: None,
            caret_visible: false,
        };
        Self {
            edit_visual: hidden,
            search_visual: hidden,
        }
    }
}

/// 根据编辑核心状态与文本测量回调计算光标竖线、选区高亮矩形与 IME 预编辑下划线矩形。
#[must_use]
pub fn compute_text_edit_visual<F>(
    edit: &TextEditState,
    text_rect: Rect,
    active: bool,
    mut measure_width: F,
) -> TextInputVisual
where
    F: FnMut(&str) -> f32,
{
    let text = edit.text();
    let scroll_x = edit.scroll_x();
    let x_at = |byte_offset: usize, measure: &mut F| -> f32 {
        let clamped = floor_boundary(text, byte_offset.min(text.len()));
        let prefix_w = if clamped == 0 {
            0.0
        } else {
            measure(&text[..clamped])
        };
        (text_rect.origin.x + prefix_w - scroll_x).clamp(text_rect.origin.x, text_rect.max().x)
    };

    let caret_x = x_at(edit.cursor(), &mut measure_width);
    let caret_h = (text_rect.size.y - 12.0).max(12.0).min(text_rect.size.y);
    let caret_y = text_rect.origin.y + (text_rect.size.y - caret_h) * 0.5;
    let caret = Rect::new(caret_x, caret_y, 1.5, caret_h);

    let selection = edit.selection().and_then(|(start, end)| {
        let x0 = x_at(start, &mut measure_width);
        let x1 = x_at(end, &mut measure_width);
        let w = (x1 - x0).max(0.0);
        (w > 0.5).then_some(Rect::new(x0, caret_y, w, caret_h))
    });

    let preedit = edit.preedit().and_then(|(start, end)| {
        let x0 = x_at(start, &mut measure_width);
        let x1 = x_at(end, &mut measure_width);
        let w = (x1 - x0).max(0.0);
        (w > 0.5).then_some(Rect::new(x0, caret_y + caret_h, w, 1.5))
    });

    TextInputVisual {
        caret,
        selection,
        preedit,
        caret_visible: active,
    }
}

/// 发射编辑态选区高亮矩形（槽位恒定，通过不透明度切换显隐）。
fn push_selection_overlay(
    visual: &TextInputVisual,
    active: bool,
    disabled: bool,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
) {
    let mut selection = surface(
        visual.selection.unwrap_or(Rect::ZERO),
        2.0,
        Token::SELECTION,
        z,
    );
    selection.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::InnerShadow as u16,
    );
    selection.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z,
        if active && !disabled && visual.selection.is_some() {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(selection);
}

/// 发射编辑态光标竖线与 IME 预编辑下划线（槽位恒定，通过不透明度切换显隐）。
fn push_caret_and_preedit_overlays(
    visual: &TextInputVisual,
    active: bool,
    has_preedit: bool,
    disabled: bool,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
) {
    let mut caret = surface(visual.caret, 0.0, Token::TEXT_PRIMARY, z);
    caret.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Content as u16,
    );
    caret.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z,
        if active && !disabled && visual.caret_visible {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(caret);

    let mut preedit = surface(
        visual.preedit.unwrap_or(Rect::ZERO),
        0.0,
        Token::PREEDIT_UNDERLINE,
        z,
    );
    preedit.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Content as u16,
    );
    preedit.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z,
        if active && !disabled && has_preedit && visual.preedit.is_some() {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(preedit);
}

/// 发射地址栏单个文本裁剪层（用于路径区或搜索区在独立 `push_clip` 作用域内绘制主文本、次级占位文本与光标/选区/IME）。
#[must_use]
pub fn emit_address_bar_text_layer(
    primary_glyphs: &[GlyphQuad],
    placeholder_glyphs: &[GlyphQuad],
    visual: &TextInputVisual,
    active: bool,
    has_preedit: bool,
    disabled: bool,
    z: u16,
) -> WidgetOutput {
    let mut primitives =
        Vec::with_capacity(3 + primary_glyphs.len() + placeholder_glyphs.len());
    let mut payloads = Vec::with_capacity(primary_glyphs.len() + placeholder_glyphs.len());
    let text_z = z.saturating_add(2);

    push_selection_overlay(visual, active, disabled, text_z, &mut primitives);
    emit_glyphs(
        primary_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        text_z,
        &mut primitives,
        &mut payloads,
    );
    emit_glyphs(
        placeholder_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_SECONDARY
        },
        text_z,
        &mut primitives,
        &mut payloads,
    );
    push_caret_and_preedit_overlays(visual, active, has_preedit, disabled, text_z, &mut primitives);

    WidgetOutput {
        primitives,
        payloads,
        ..Default::default()
    }
}

/// 发射双模复合地址栏（外部导航按钮簇 + 一体化双模地址框 + 可选内联搜索框，严格保持 24 态恒定图元数）。
#[must_use]
pub fn emit_address_bar(
    node: u64,
    desc: &AddressBarDesc,
    state: &AddressBarState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    emit_address_bar_with_visual(
        node,
        desc,
        state,
        control,
        glyphs,
        &[],
        &AddressBarTextVisual::default(),
    )
}

/// 发射双模复合地址栏，支持区分主文本与次级色占位文本字形，以及编辑态/搜索框的光标、选区与 IME 预编辑下划线。
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn emit_address_bar_with_visual(
    node: u64,
    desc: &AddressBarDesc,
    state: &AddressBarState,
    control: ControlState,
    primary_glyphs: &[GlyphQuad],
    placeholder_glyphs: &[GlyphQuad],
    visual: &AddressBarTextVisual,
) -> WidgetOutput {
    let layout = compute_layout(desc);
    let z = desc.z;
    let total_glyphs = primary_glyphs.len() + placeholder_glyphs.len();
    let mut primitives = Vec::with_capacity(36 + layout.segments.len() * 4 + total_glyphs);
    let mut payloads = Vec::with_capacity(20 + layout.segments.len() * 2 + total_glyphs);

    // 1. 左段：外部 4 枚导航按钮槽位（后退/前进/向上/刷新，位于地址框之外）
    let nav_slots: [(Option<Rect>, AddressBarHitKind, bool); 4] = [
        (layout.back_rect, AddressBarHitKind::NavBack, desc.can_go_back),
        (
            layout.forward_rect,
            AddressBarHitKind::NavForward,
            desc.can_go_forward,
        ),
        (layout.up_rect, AddressBarHitKind::NavUp, desc.can_go_up),
        (layout.refresh_rect, AddressBarHitKind::Refresh, true),
    ];
    for (slot_rect, kind, enabled) in nav_slots {
        let Some(btn_r) = slot_rect else {
            continue;
        };
        let active = !control.disabled && enabled;
        // 仅在 `hovered_hit` 精确命中对应按钮时才高亮，避免空白悬停误高亮后退按钮
        let is_hovered = active && state.hovered_hit == Some(kind);
        let fill = if is_hovered {
            if control.phase == PointerPhase::Pressed {
                Token::PRESSED
            } else {
                Token::HOVER
            }
        } else {
            Token::NONE
        };
        let icon_color = if active {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_DISABLED
        };
        primitives.push(surface(btn_r, INNER_RADIUS, fill, z + 1));
        match kind {
            AddressBarHitKind::NavBack => {
                emit_arrow_left_icon(btn_r, icon_color, z + 2, &mut primitives, &mut payloads);
            }
            AddressBarHitKind::NavForward => {
                emit_shell_icon_primitives(
                    ShellIcon::ArrowRight,
                    btn_r,
                    icon_color,
                    fill,
                    z + 2,
                    &mut primitives,
                    &mut payloads,
                );
            }
            AddressBarHitKind::NavUp => {
                emit_shell_icon_primitives(
                    ShellIcon::ArrowUp,
                    btn_r,
                    icon_color,
                    fill,
                    z + 2,
                    &mut primitives,
                    &mut payloads,
                );
            }
            AddressBarHitKind::Refresh => {
                emit_refresh_icon(btn_r, icon_color, z + 2, &mut primitives, &mut payloads);
            }
            _ => {}
        }
    }

    // 2. 中段：一体化双模地址框
    let addr_r = layout.address_rect;
    let bg = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else {
        Token::CONTROL_FILL
    };
    let border = if control.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if control.phase == PointerPhase::Hovered || state.editing {
        Token::OUTLINE_STRONG
    } else {
        Token::CONTROL_STROKE
    };
    primitives.push(outlined(addr_r, BAR_RADIUS, bg, border, 1.0, 0.0, z));

    // 聚焦/编辑态底部 2px 强调线（图元始终存在，通过透明度切换保证 24 态图元数恒定）
    primitives.push(focus_underline(
        addr_r,
        BAR_RADIUS,
        z,
        if !control.disabled && (state.editing || control.focused) {
            1.0
        } else {
            0.0
        },
    ));

    // 左侧固定 16×16 根目录/驱动器图标
    let root_icon = desc
        .segments
        .first()
        .and_then(|s| s.icon)
        .unwrap_or(ShellIcon::Folder);
    let root_icon_color = if control.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    emit_shell_icon_primitives(
        root_icon,
        layout.root_icon_rect,
        root_icon_color,
        bg,
        z + 1,
        &mut primitives,
        &mut payloads,
    );

    // 根图标后置 `›` 分隔箭头（编辑态置透明以保持图元数恒定）
    if let Some(rc_r) = layout.root_chevron_rect {
        let rc_color = if state.editing {
            Token::NONE
        } else if control.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_SECONDARY
        };
        emit_segment_chevron(rc_r, rc_color, z + 2, &mut primitives, &mut payloads);
    }

    // 溢出折叠 `«` 按钮（编辑态置透明以保持图元数恒定）
    if let Some(ov_r) = layout.overflow_rect {
        let ov_fill = if state.editing || control.disabled {
            Token::NONE
        } else if state.hovered_hit == Some(AddressBarHitKind::OverflowButton) {
            if control.phase == PointerPhase::Pressed {
                Token::PRESSED
            } else {
                Token::HOVER
            }
        } else {
            Token::NONE
        };
        let ov_color = if state.editing {
            Token::NONE
        } else if control.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_SECONDARY
        };
        primitives.push(surface(ov_r, INNER_RADIUS, ov_fill, z + 1));
        emit_overflow_chevrons(ov_r, ov_color, z + 2, &mut primitives, &mut payloads);
    }

    // 面包屑分段胶囊与条件性 `›` 展开箭头（常态透明，悬停/按下呈现圆角高亮；编辑态隐藏但保持图元数恒定）
    for seg in &layout.segments {
        let seg_fill = if state.editing || control.disabled {
            Token::NONE
        } else if state.hovered_hit == Some(AddressBarHitKind::Segment(seg.index)) {
            if control.phase == PointerPhase::Pressed {
                Token::PRESSED
            } else {
                Token::HOVER
            }
        } else {
            Token::NONE
        };
        primitives.push(surface(seg.label_rect, INNER_RADIUS, seg_fill, z + 1));

        if let Some(chev_r) = seg.chevron_rect {
            let chev_fill = if state.editing || control.disabled {
                Token::NONE
            } else if state.hovered_hit == Some(AddressBarHitKind::SegmentChevron(seg.index)) {
                if control.phase == PointerPhase::Pressed {
                    Token::PRESSED
                } else {
                    Token::HOVER
                }
            } else {
                Token::NONE
            };
            let chev_color = if state.editing {
                Token::NONE
            } else if control.disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };
            primitives.push(surface(chev_r, INNER_RADIUS, chev_fill, z + 1));
            emit_segment_chevron(chev_r, chev_color, z + 2, &mut primitives, &mut payloads);
        }
    }

    // 3. 右段：可选可折叠内联搜索框
    if let Some(sr) = layout.search_rect {
        let search_border = if control.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if state.search_focused
            || state.hovered_hit == Some(AddressBarHitKind::SearchBox)
        {
            Token::OUTLINE_STRONG
        } else {
            Token::CONTROL_STROKE
        };
        primitives.push(outlined(sr, BAR_RADIUS, bg, search_border, 1.0, 0.0, z));
        primitives.push(focus_underline(
            sr,
            BAR_RADIUS,
            z,
            if !control.disabled && state.search_focused {
                1.0
            } else {
                0.0
            },
        ));
        if let Some(icon_r) = layout.search_icon_rect {
            emit_shell_icon_primitives(
                ShellIcon::Search,
                icon_r,
                if control.disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::TEXT_SECONDARY
                },
                bg,
                z + 2,
                &mut primitives,
                &mut payloads,
            );
        }
        if let Some(clear_r) = layout.search_clear_rect {
            let show_clear = !control.disabled && !state.search_core.text().is_empty();
            let clear_fill = if show_clear
                && state.hovered_hit == Some(AddressBarHitKind::SearchClear)
            {
                Token::HOVER
            } else {
                Token::NONE
            };
            let clear_color = if show_clear {
                Token::TEXT_SECONDARY
            } else {
                Token::NONE
            };
            primitives.push(surface(clear_r, INNER_RADIUS, clear_fill, z + 1));
            emit_shell_icon_primitives(
                ShellIcon::Close,
                clear_r,
                clear_color,
                clear_fill,
                z + 2,
                &mut primitives,
                &mut payloads,
            );
        }
    }

    // 4. 编辑态/搜索框选区高亮、主文本与次级占位文本字形、光标与 IME 预编辑下划线
    let edit_active = state.editing && !control.disabled;
    let search_active =
        layout.search_rect.is_some() && !layout.search_collapsed && state.search_focused && !control.disabled;

    push_selection_overlay(&visual.edit_visual, edit_active, control.disabled, z + 2, &mut primitives);
    if layout.search_rect.is_some() {
        push_selection_overlay(
            &visual.search_visual,
            search_active,
            control.disabled,
            z + 2,
            &mut primitives,
        );
    }

    emit_glyphs(
        primary_glyphs,
        if control.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    emit_glyphs(
        placeholder_glyphs,
        if control.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_SECONDARY
        },
        z + 2,
        &mut primitives,
        &mut payloads,
    );

    push_caret_and_preedit_overlays(
        &visual.edit_visual,
        edit_active,
        state.edit_core.preedit().is_some(),
        control.disabled,
        z + 2,
        &mut primitives,
    );
    if layout.search_rect.is_some() {
        push_caret_and_preedit_overlays(
            &visual.search_visual,
            search_active,
            state.search_core.preedit().is_some(),
            control.disabled,
            z + 2,
            &mut primitives,
        );
    }
    primitives.push(focus_ring(
        addr_r.inflate(2.0),
        BAR_RADIUS + 2.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled).then(|| {
            HitRegion::new(node, LAYER, u32::from(z), desc.rect).with_cursor(if state.editing {
                CursorKind::Text
            } else {
                CursorKind::Pointer
            })
        }),
        a11y: Some(
            A11yNode::new(Role::Toolbar, desc.full_path(), desc.rect).disabled(control.disabled),
        ),
        focusable: !control.disabled,
        ..Default::default()
    };

    // 注册独立子热区（便于输入路由器精确命中各按钮、分段与搜索框）
    if !control.disabled {
        for (slot_rect, kind, enabled, label) in [
            (
                layout.back_rect,
                AddressBarHitKind::NavBack,
                desc.can_go_back,
                "Back",
            ),
            (
                layout.forward_rect,
                AddressBarHitKind::NavForward,
                desc.can_go_forward,
                "Forward",
            ),
            (
                layout.up_rect,
                AddressBarHitKind::NavUp,
                desc.can_go_up,
                "Up",
            ),
            (
                layout.refresh_rect,
                AddressBarHitKind::Refresh,
                true,
                "Refresh",
            ),
        ] {
            if let Some(btn_r) = slot_rect {
                let sub = address_bar_sub_node(node, kind);
                out.with_sub_control(
                    sub,
                    HitRegion::new(sub, LAYER, u32::from(z) + 1, btn_r).with_cursor(if enabled {
                        CursorKind::Pointer
                    } else {
                        CursorKind::Default
                    }),
                    A11yNode::new(Role::Button, label, btn_r)
                        .clickable()
                        .disabled(!enabled),
                    enabled,
                );
            }
        }

        if !state.editing {
            if let Some(ov_r) = layout.overflow_rect {
                let sub = address_bar_sub_node(node, AddressBarHitKind::OverflowButton);
                out.with_sub_control(
                    sub,
                    HitRegion::new(sub, LAYER, u32::from(z) + 2, ov_r)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::Button, "Overflow path segments", ov_r).clickable(),
                    true,
                );
            }
            for seg in &layout.segments {
                let seg_label = desc
                    .segments
                    .get(seg.index)
                    .map_or("", |s| s.label.as_str());
                let seg_sub = address_bar_sub_node(node, AddressBarHitKind::Segment(seg.index));
                out.with_sub_control(
                    seg_sub,
                    HitRegion::new(seg_sub, LAYER, u32::from(z) + 2, seg.label_rect)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::Button, seg_label, seg.label_rect).clickable(),
                    true,
                );
                if let Some(chev_r) = seg.chevron_rect {
                    let chev_sub =
                        address_bar_sub_node(node, AddressBarHitKind::SegmentChevron(seg.index));
                    out.with_sub_control(
                        chev_sub,
                        HitRegion::new(chev_sub, LAYER, u32::from(z) + 2, chev_r)
                            .with_cursor(CursorKind::Pointer),
                        A11yNode::new(Role::Button, "Subfolders", chev_r).clickable(),
                        true,
                    );
                }
            }
        }

        let edit_sub = address_bar_sub_node(node, AddressBarHitKind::EditTrigger);
        let edit_r = if state.editing {
            layout.address_rect
        } else {
            layout.edit_trigger_rect
        };
        out.with_sub_control(
            edit_sub,
            HitRegion::new(edit_sub, LAYER, u32::from(z) + 1, edit_r)
                .with_cursor(CursorKind::Text),
            A11yNode::new(Role::TextInput, desc.full_path(), edit_r),
            true,
        );

        if let Some(sr) = layout.search_rect {
            let search_sub = address_bar_sub_node(node, AddressBarHitKind::SearchBox);
            let search_label = if desc.search_placeholder.is_empty() {
                "Search"
            } else {
                desc.search_placeholder.as_str()
            };
            out.with_sub_control(
                search_sub,
                HitRegion::new(search_sub, LAYER, u32::from(z) + 1, sr)
                    .with_cursor(if layout.search_collapsed {
                        CursorKind::Pointer
                    } else {
                        CursorKind::Text
                    }),
                A11yNode::new(Role::SearchInput, search_label, sr),
                true,
            );
            if let Some(clear_r) = layout.search_clear_rect {
                if !state.search_core.text().is_empty() {
                    let clear_sub = address_bar_sub_node(node, AddressBarHitKind::SearchClear);
                    out.with_sub_control(
                        clear_sub,
                        HitRegion::new(clear_sub, LAYER, u32::from(z) + 2, clear_r)
                            .with_cursor(CursorKind::Pointer),
                        A11yNode::new(Role::Button, "Clear search", clear_r).clickable(),
                        true,
                    );
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_desc() -> AddressBarDesc {
        AddressBarDesc::with_path_segments(
            Rect::new(0.0, 0.0, 760.0, ADDRESS_BAR_HEIGHT),
            "C:/Workspace/Assets/Shaders",
            vec![
                PathSegment::new("C:", "C:/").with_icon(ShellIcon::Folder),
                PathSegment::new("Workspace", "C:/Workspace"),
                PathSegment::new("Assets", "C:/Workspace/Assets"),
                PathSegment::new("Shaders", "C:/Workspace/Assets/Shaders").with_subfolders(false),
            ],
        )
        .with_nav_state(true, false, true)
        .with_search("Search Shaders")
    }

    #[test]
    fn address_bar_keeps_constant_primitives_across_24_states() {
        let desc = sample_desc();
        for editing in [false, true] {
            let mut ab_state = AddressBarState::new();
            if editing {
                ab_state.begin_edit(&desc.full_path());
            }
            let base = emit_address_bar(1, &desc, &ab_state, ControlState::default(), &[])
                .primitives
                .len();
            for phase in [
                PointerPhase::Idle,
                PointerPhase::Hovered,
                PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let count = emit_address_bar(1, &desc, &ab_state, control, &[])
                                .primitives
                                .len();
                            assert_eq!(
                                count, base,
                                "primitive count drifted in editing={editing} state={control:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn nav_buttons_are_outside_address_box_and_respect_disabled_state() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let back_r = layout.back_rect.expect("back button");
        let fwd_r = layout.forward_rect.expect("forward button");
        let up_r = layout.up_rect.expect("up button");
        let ref_r = layout.refresh_rect.expect("refresh button");

        assert!(ref_r.max().x < layout.address_rect.origin.x);

        let mut state = AddressBarState::new();
        assert!(state.handle_click(&desc, back_r.center()));
        assert!(
            !state.handle_click(&desc, fwd_r.center()),
            "disabled forward button must not emit event"
        );
        assert!(state.handle_click(&desc, up_r.center()));
        assert!(state.handle_click(&desc, ref_r.center()));

        assert_eq!(
            state.drain_events(),
            vec![
                AddressBarEvent::NavigateBack,
                AddressBarEvent::NavigateUp,
                AddressBarEvent::Refresh,
            ]
        );
    }

    #[test]
    fn segments_click_and_chevrons_emit_anchor_rect_and_overflow_folds() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        assert_eq!(layout.overflow_count, 0);
        assert_eq!(layout.segments.len(), 4);
        assert!(layout.segments[3].chevron_rect.is_none());

        let mut state = AddressBarState::new();
        assert!(state.handle_click(&desc, layout.segments[1].label_rect.center()));
        let chev_r = layout.segments[1].chevron_rect.expect("chevron");
        assert!(state.handle_click(&desc, chev_r.center()));

        assert_eq!(
            state.drain_events(),
            vec![
                AddressBarEvent::SegmentClicked {
                    index: 1,
                    path: "C:/Workspace".to_owned(),
                },
                AddressBarEvent::SubfolderMenuRequested {
                    index: 1,
                    path: "C:/Workspace".to_owned(),
                    anchor_rect: rect_to_i32(chev_r),
                },
            ]
        );

        // 窄宽下自动将前段折叠为 `«` 溢出按钮
        let mut narrow = desc.clone();
        narrow.rect = Rect::new(0.0, 0.0, 320.0, ADDRESS_BAR_HEIGHT);
        narrow.show_search = false;
        let narrow_layout = compute_layout(&narrow);
        assert!(narrow_layout.overflow_count > 0);
        let ov_r = narrow_layout.overflow_rect.expect("overflow button");
        assert!(state.handle_click(&narrow, ov_r.center()));
        assert!(matches!(
            state.drain_events().as_slice(),
            [AddressBarEvent::SubfolderMenuRequested { index: 0, .. }]
        ));
    }

    #[test]
    fn blank_click_and_ctrl_l_enter_edit_mode_with_enter_and_esc() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let mut state = AddressBarState::new();

        // 点击末段右侧空白区切入编辑态并全选完整路径
        assert!(state.handle_click(&desc, layout.edit_trigger_rect.center()));
        assert!(state.editing);
        assert_eq!(state.edit_core.text(), "C:/Workspace/Assets/Shaders");
        assert_eq!(
            state.edit_core.selected_text(),
            Some("C:/Workspace/Assets/Shaders")
        );

        // Esc 取消编辑
        assert!(state.handle_key(&desc, KeyChord::named("Escape")));
        assert!(!state.editing);
        assert_eq!(state.drain_events(), vec![AddressBarEvent::EditCancelled]);

        // Ctrl+L 切入编辑态，输入新路径后按 Enter 提交
        assert!(state.handle_key(&desc, KeyChord::character('l').with_ctrl()));
        assert!(state.editing);
        state.handle_text_event(&desc, &TextInputEvent::Insert("D:/Projects".to_owned()));
        assert!(state.handle_key(&desc, KeyChord::named("Enter")));
        assert!(!state.editing);
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::PathSubmitted("D:/Projects".to_owned())]
        );
    }

    #[test]
    fn inline_search_box_typing_submit_and_clear() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let sr = layout.search_rect.expect("search box");
        let clear_r = layout.search_clear_rect.expect("clear button");
        let mut state = AddressBarState::new();

        assert!(state.handle_click(&desc, sr.center()));
        assert!(state.search_focused);

        assert!(state.handle_key(&desc, KeyChord::character('p')));
        assert!(state.handle_key(&desc, KeyChord::character('b')));
        assert!(state.handle_key(&desc, KeyChord::character('r')));
        assert!(state.handle_key(&desc, KeyChord::named("Enter")));

        assert_eq!(
            state.drain_events(),
            vec![
                AddressBarEvent::SearchChanged("p".to_owned()),
                AddressBarEvent::SearchChanged("pb".to_owned()),
                AddressBarEvent::SearchChanged("pbr".to_owned()),
                AddressBarEvent::SearchSubmitted("pbr".to_owned()),
            ]
        );

        assert!(state.handle_click(&desc, clear_r.center()));
        assert!(state.search_core.text().is_empty());
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::SearchChanged(String::new())]
        );
    }

    #[test]
    fn legacy_address_bar_api_remains_compatible() {
        let desc = AddressBarDesc::new(
            Rect::new(0.0, 0.0, 360.0, 34.0),
            vec!["Workspace".to_owned(), "Assets".to_owned()],
        );
        assert_eq!(desc.full_path(), "Workspace/Assets");
        let mut state = AddressBarState::new();
        state.begin_edit(&desc.full_path());
        assert_eq!(state.commit_edit(), vec!["Workspace", "Assets"]);
    }

    #[test]
    fn default_z_is_above_container_surface_layer() {
        let legacy = AddressBarDesc::new(
            Rect::new(0.0, 0.0, 600.0, ADDRESS_BAR_HEIGHT),
            vec!["C:".to_owned(), "Workspace".to_owned()],
        );
        let structured = sample_desc();
        assert!(legacy.z >= 400, "legacy default z must be >= 400, got {}", legacy.z);
        assert!(
            structured.z >= 400,
            "structured default z must be >= 400, got {}",
            structured.z
        );
        assert_eq!(legacy.z, 450);
        assert_eq!(structured.z, 450);
    }

    #[test]
    fn search_icon_is_on_left_and_text_region_stays_inside_search_box() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let sr = layout.search_rect.expect("search rect");
        let icon_r = layout.search_icon_rect.expect("search icon rect");
        let clear_r = layout.search_clear_rect.expect("search clear rect");

        assert!(!layout.search_collapsed);
        assert!(
            icon_r.origin.x >= sr.origin.x && icon_r.max().x < sr.center().x,
            "search icon must be positioned on the left side of search box"
        );
        assert!(
            clear_r.origin.x > sr.center().x && clear_r.max().x <= sr.max().x,
            "clear button must be positioned on the right side of search box"
        );
        let text_left = icon_r.max().x + 6.0;
        let text_right = clear_r.origin.x - 4.0;
        assert!(
            text_left < text_right && text_right <= sr.max().x,
            "search text region [{text_left}, {text_right}] must fit inside search_rect"
        );

        // 折叠态下图标居中且不分配清除按钮
        let mut narrow = desc.clone();
        narrow.rect = Rect::new(0.0, 0.0, 360.0, ADDRESS_BAR_HEIGHT);
        let narrow_layout = compute_layout(&narrow);
        assert!(narrow_layout.search_collapsed);
        assert!(narrow_layout.search_clear_rect.is_none());
        let collapsed_sr = narrow_layout.search_rect.expect("collapsed search rect");
        let collapsed_icon = narrow_layout.search_icon_rect.expect("collapsed icon");
        assert!((collapsed_icon.center().x - collapsed_sr.center().x).abs() < 0.5);
    }

    #[test]
    fn root_chevron_rect_sits_between_root_icon_and_first_segment() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let rc = layout.root_chevron_rect.expect("root chevron rect");
        let first_seg = layout.segments.first().expect("first segment");
        assert!(
            rc.origin.x >= layout.root_icon_rect.max().x,
            "root chevron must follow root icon"
        );
        assert!(
            rc.max().x <= first_seg.label_rect.origin.x,
            "root chevron must precede first segment"
        );
    }

    #[test]
    fn cjk_segment_width_exceeds_same_char_count_ascii_and_clamps_upper_bound() {
        // 使用 Unicode 转义构造 CJK 目录名，满足代码语言守卫同时覆盖宽字符估算
        let cjk_short = "\u{6B64}\u{7535}\u{8111}";
        let ascii_short = "abc";
        let cjk_long = "\u{672C}\u{5730}\u{78C1}\u{76D8}\u{5DE5}\u{4F5C}\u{533A}\u{8D44}\u{6E90}\u{7BA1}\u{7406}\u{5668}\u{957F}\u{76EE}\u{5F55}\u{540D}\u{79F0}\u{6D4B}\u{8BD5}\u{7528}\u{4F8B}";

        let desc_cjk = AddressBarDesc::with_path_segments(
            Rect::new(0.0, 0.0, 900.0, ADDRESS_BAR_HEIGHT),
            cjk_short,
            vec![
                PathSegment::new(cjk_short, cjk_short),
                PathSegment::new(cjk_long, cjk_long).with_subfolders(false),
            ],
        )
        .without_search();
        let desc_ascii = AddressBarDesc::with_path_segments(
            Rect::new(0.0, 0.0, 900.0, ADDRESS_BAR_HEIGHT),
            ascii_short,
            vec![PathSegment::new(ascii_short, ascii_short)],
        )
        .without_search();

        let layout_cjk = compute_layout(&desc_cjk);
        let layout_ascii = compute_layout(&desc_ascii);
        assert!(
            layout_cjk.segments[0].label_rect.width()
                > layout_ascii.segments[0].label_rect.width() + 10.0,
            "CJK segment width must exceed same-char-count ASCII width"
        );
        assert!(
            layout_cjk.segments[1].label_rect.width() <= 220.0 + 0.01,
            "long CJK segment width must clamp to 220px"
        );
    }

    #[test]
    fn idle_segments_are_transparent_and_none_hover_does_not_highlight_nav_back() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let mut state = AddressBarState::new();
        let hovered_control = ControlState {
            phase: PointerPhase::Hovered,
            ..ControlState::default()
        };

        // hovered_hit == None 且控件整体处于 Hovered 时：NavBack 不得被误高亮，且所有分段常态透明
        let out = emit_address_bar(1, &desc, &state, hovered_control, &[]);
        let back_r = layout.back_rect.expect("back rect");
        let back_bg = out
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - back_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - back_r.origin.y).abs() < 0.5
                    && (p.size[0] - back_r.size.x).abs() < 0.5
            })
            .expect("back button surface");
        assert_eq!(
            back_bg.fill_token_or_uv,
            u32::from(Token::NONE.raw()),
            "NavBack must remain transparent when hovered_hit is None"
        );

        let seg0_r = layout.segments[0].label_rect;
        let seg0_bg = out
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - seg0_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - seg0_r.origin.y).abs() < 0.5
                    && (p.size[0] - seg0_r.size.x).abs() < 0.5
            })
            .expect("segment 0 surface");
        assert_eq!(
            seg0_bg.fill_token_or_uv,
            u32::from(Token::NONE.raw()),
            "unhovered breadcrumb segment must be transparent"
        );

        // 精确悬停在 NavBack 或 Segment(0) 时应呈现 Token::HOVER，按下时呈现 Token::PRESSED
        state.hovered_hit = Some(AddressBarHitKind::NavBack);
        let out_back_hover = emit_address_bar(1, &desc, &state, hovered_control, &[]);
        let back_bg_hover = out_back_hover
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - back_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - back_r.origin.y).abs() < 0.5
                    && (p.size[0] - back_r.size.x).abs() < 0.5
            })
            .expect("hovered back button surface");
        assert_eq!(back_bg_hover.fill_token_or_uv, u32::from(Token::HOVER.raw()));

        state.hovered_hit = Some(AddressBarHitKind::Segment(0));
        let pressed_control = ControlState {
            phase: PointerPhase::Pressed,
            ..ControlState::default()
        };
        let out_seg_pressed = emit_address_bar(1, &desc, &state, pressed_control, &[]);
        let seg0_bg_pressed = out_seg_pressed
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - seg0_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - seg0_r.origin.y).abs() < 0.5
                    && (p.size[0] - seg0_r.size.x).abs() < 0.5
            })
            .expect("pressed segment 0 surface");
        assert_eq!(
            seg0_bg_pressed.fill_token_or_uv,
            u32::from(Token::PRESSED.raw())
        );
    }

    #[test]
    fn edit_and_search_overlays_emit_selection_caret_ime_and_secondary_placeholder() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let mut state = AddressBarState::new();
        state.begin_edit(&desc.full_path());
        let edit_r = Rect::new(
            layout.root_icon_rect.max().x + 6.0,
            layout.address_rect.origin.y,
            240.0,
            layout.address_rect.size.y,
        );
        // 刚进入编辑态时全选路径，验证选区高亮矩形发射
        let sel_visual = compute_text_edit_visual(&state.edit_core, edit_r, true, |s| {
            s.chars().count() as f32 * 8.0
        });
        assert!(sel_visual.selection.is_some());
        let out_sel = emit_address_bar_with_visual(
            1,
            &desc,
            &state,
            ControlState::default(),
            &[],
            &[],
            &AddressBarTextVisual {
                edit_visual: sel_visual,
                ..AddressBarTextVisual::default()
            },
        );
        assert!(
            out_sel
                .primitives
                .iter()
                .any(|p| p.fill_token_or_uv == u32::from(Token::SELECTION.raw())
                    && p.opacity() > 0.5),
            "active edit selection must emit visible Token::SELECTION overlay"
        );

        // 触发 IME 预编辑态以验证预编辑下划线发射
        let _ = state.handle_text_event(
            &desc,
            &TextInputEvent::ImePreedit {
                text: "src".to_owned(),
                cursor: Some(3),
            },
        );
        let edit_visual = compute_text_edit_visual(&state.edit_core, edit_r, true, |s| {
            s.chars().count() as f32 * 8.0
        });
        assert!(edit_visual.preedit.is_some());
        assert!(edit_visual.caret_visible);

        let placeholder_quad = GlyphQuad {
            rect: Rect::new(580.0, 8.0, 8.0, 14.0),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        };
        let visual = AddressBarTextVisual {
            edit_visual,
            ..AddressBarTextVisual::default()
        };
        let out = emit_address_bar_with_visual(
            1,
            &desc,
            &state,
            ControlState::default(),
            &[],
            &[placeholder_quad],
            &visual,
        );

        assert!(
            out.primitives
                .iter()
                .any(|p| p.fill_token_or_uv == u32::from(Token::TEXT_SECONDARY.raw())
                    && (p.local_pos[0] - 580.0).abs() < 0.5),
            "placeholder glyph must use Token::TEXT_SECONDARY"
        );
        assert!(
            out.primitives
                .iter()
                .any(|p| p.fill_token_or_uv == u32::from(Token::PREEDIT_UNDERLINE.raw())
                    && p.opacity() > 0.5),
            "active IME preedit must emit visible underline"
        );
        assert!(
            out.primitives
                .iter()
                .any(|p| p.fill_token_or_uv == u32::from(Token::TEXT_PRIMARY.raw())
                    && (p.size[0] - 1.5).abs() < 0.01
                    && p.opacity() > 0.5),
            "active edit state must emit visible caret"
        );
    }
}
