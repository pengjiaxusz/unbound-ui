//! 滚动容器、独立按需叠加滚动条内核与虚拟列表回放。
//!
//! [`ScrollBarDesc`] 与 [`emit_scrollbar`] 提供不依赖 [`ScrollViewDesc`] 裁剪与变换的
//! 独立 Win11 按需叠加滚动条内核，可供自带虚拟化或自定义裁剪区的复合组件直接挂载。
//!
//! [`ScrollViewDesc`] 描述视口；[`ScrollState`] 持有滚动偏移量及其弹簧状态。
//! 两者均不持有子控件：子元素由调用方输出，并由 [`ScrollViewOutput::clip`] 裁剪、
//! 由 [`ScrollViewOutput::transform`] 平移。这正是亚像素滚动零开销的原因——
//! 滚动仅修改单个 `GpuTransform`，而不会修改任何子图元。
//!
//! 虚拟列表通过 [`update_virtual_list`] 接入：每帧重算一次可见窗口，
//! 当仅亚行偏移发生变化时返回 `WindowUpdate::offset_only`，实现**零**重新绑定。

use unbound_anim::{Spring2, SpringPreset};
use unbound_gpu::data::{GpuClipNode, GpuPrimitiveInstance, GpuTransform, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;
use unbound_world::virtualize::VirtualList;

use crate::primitive::surface;
use crate::{ControlState, PointerPhase, SceneCollector, WidgetDesc, WidgetOutput};

/// 滚动视图所在的界面层。
pub const SCROLL_LAYER: u8 = 1;
/// 滚动条占据的带宽（逻辑像素）。
///
/// WinUI 3 的 `ScrollBarSize` = 12：它既是滚动条的命中带宽度，
/// 也是悬停展开后滑块矩形（含透明描边）与轨道胶囊的宽度。
pub const SCROLLBAR_WIDTH: f32 = 12.0;
/// 滚动条滑块的最小长度（逻辑像素）。
///
/// WinUI `ScrollBarVerticalThumbMinHeight` / `ScrollBarHorizontalThumbMinWidth` = 30。
pub const MIN_THUMB: f32 = 30.0;
/// 悬停 / 拖拽时滑块实际可见的宽度（逻辑像素）。
///
/// WinUI：`ScrollBarSize`(12) − `ScrollBarThumbStrokeThickness`(6) = 6；
/// 配合 `ScrollBarCornerRadius` 3，圆角正好是宽度的一半。
pub const SCROLLBAR_THUMB_WIDTH: f32 = 6.0;
/// 静止（收起）态滑块实际可见的宽度（逻辑像素）。
///
/// WinUI：`ScrollBarVerticalThumbMinWidth`(8) − `ScrollBarThumbStrokeThickness`(6) = 2，
/// 即 Windows 11 那条 2px 细线；轨道透明、无箭头按钮。
pub const SCROLLBAR_THUMB_IDLE_WIDTH: f32 = 2.0;
/// 轨道距视口边缘的内缩距离（逻辑像素，对应 WinUI `ScrollBarThumbOffset` = 2）。
pub const SCROLLBAR_EDGE_INSET: f32 = 2.0;
/// 静止态 2px 细指示条的半透明度（溢出时一眼可见，不依赖非零滚动历史）。
pub const SCROLLBAR_IDLE_OPACITY: f32 = 0.55;
/// 展开态（悬停 / 拖拽 / `AlwaysExpanded`）滑块不透明度。
pub const SCROLLBAR_EXPANDED_OPACITY: f32 = 1.0;
/// 展开态半透明轨道胶囊不透明度。
pub const SCROLLBAR_TRACK_CAPSULE_OPACITY: f32 = 0.78;
/// 垂直滚动条滑块子命中位掩码（与 `ScrollView` 历史位域兼容）。
pub const SCROLLBAR_VERTICAL_THUMB_HIT_MASK: u64 = 1 << 63;
/// 水平滚动条滑块子命中位掩码（与 `ScrollView` 历史位域兼容）。
pub const SCROLLBAR_HORIZONTAL_THUMB_HIT_MASK: u64 = 1 << 62;
/// 垂直滚动条轨道子命中位掩码（用于点击轨道翻页与悬停热区识别）。
pub const SCROLLBAR_VERTICAL_TRACK_HIT_MASK: u64 = 1 << 61;
/// 水平滚动条轨道子命中位掩码（用于点击轨道翻页与悬停热区识别）。
pub const SCROLLBAR_HORIZONTAL_TRACK_HIT_MASK: u64 = 1 << 60;
/// 最后一次滚动后滚动条保持可见的时长（秒）。
pub const SCROLLBAR_HOLD: f32 = 1.0;
/// 保持时间到期后的淡出时长。
pub const SCROLLBAR_FADE: f32 = 0.25;
/// 惯性甩动将释放速度投射到弹簧目标的时长系数。
pub const FLING_PROJECTION: f32 = 0.25;

/// 滚动视图允许滚动的坐标轴。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ScrollAxis {
    #[default]
    Vertical,
    Horizontal,
    Both,
}

impl ScrollAxis {
    fn vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }
}

/// 正在交互或发射的单轴滚动条方向。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollbarAxis {
    Horizontal,
    #[default]
    Vertical,
}

impl ScrollbarAxis {
    /// 当前滚动轴是否为水平方向。
    #[must_use]
    pub const fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal)
    }
}

/// 生成滚动条滑块子命中节点标识。
#[must_use]
pub const fn scrollbar_thumb_node(owner: u64, axis: ScrollbarAxis) -> u64 {
    match axis {
        ScrollbarAxis::Vertical => owner ^ SCROLLBAR_VERTICAL_THUMB_HIT_MASK,
        ScrollbarAxis::Horizontal => owner ^ SCROLLBAR_HORIZONTAL_THUMB_HIT_MASK,
    }
}

/// 生成滚动条轨道子命中节点标识。
#[must_use]
pub const fn scrollbar_track_node(owner: u64, axis: ScrollbarAxis) -> u64 {
    match axis {
        ScrollbarAxis::Vertical => owner ^ SCROLLBAR_VERTICAL_TRACK_HIT_MASK,
        ScrollbarAxis::Horizontal => owner ^ SCROLLBAR_HORIZONTAL_TRACK_HIT_MASK,
    }
}

/// 按需叠加滚动条的可见性策略。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollBarVisibility {
    /// 默认 Win11 按需策略：未溢出不发射任何图元与命中区；溢出静止态渲染 2px 半透明细指示条，
    /// 悬停滚动条热区或拖拽滑块时展开为 6px 粗滑块 + 半透明轨道胶囊。
    #[default]
    Auto,
    /// 溢出时始终以 6px 粗滑块 + 半透明轨道胶囊的展开态呈现；未溢出仍不发射。
    AlwaysExpanded,
    /// 始终隐藏：无论是否溢出均不发射任何图元与命中区。
    Hidden,
}

/// 轨道上的滑块长度；由绘制与拖拽共用，确保两条路径下的滑块行程计算完全一致。
fn thumb_length(track_length: f32, viewport: f32, content: f32) -> f32 {
    let ratio = (viewport / content).clamp(0.0, 1.0);
    (track_length * ratio).max(MIN_THUMB).min(track_length)
}

/// 把命中带内的滑块收窄成 WinUI 的可见宽度。
///
/// 命中带保持 [`SCROLLBAR_WIDTH`] 全宽：
/// - 展开态（6px 粗滑块）：沿垂直于滚动方向在轨道胶囊内居中；
/// - 静止态（2px 细指示条）：贴紧轨道外右缘（垂直）或外底缘（水平），留 [`SCROLLBAR_EDGE_INSET`]（2px），
///   避免在窄栏（如 48px 紧凑导航轨）内居中偏左靠近图标。
fn visible_thumb_rect(thumb: Rect, width: f32, horizontal: bool, expanded: bool) -> Rect {
    if horizontal {
        let height = width.min(thumb.size.y);
        let y = if expanded {
            thumb.origin.y + (thumb.size.y - height) * 0.5
        } else {
            (thumb.max().y - SCROLLBAR_EDGE_INSET - height).max(thumb.origin.y)
        };
        Rect::new(thumb.origin.x, y, thumb.size.x, height)
    } else {
        let visible = width.min(thumb.size.x);
        let x = if expanded {
            thumb.origin.x + (thumb.size.x - visible) * 0.5
        } else {
            (thumb.max().x - SCROLLBAR_EDGE_INSET - visible).max(thumb.origin.x)
        };
        Rect::new(x, thumb.origin.y, visible, thumb.size.y)
    }
}

/// 独立于 [`ScrollViewDesc`] 的 Win11 按需叠加滚动条描述符。
///
/// 该描述符不接管宿主的 `clip` 与 `transform`，供自带虚拟化或自定义裁剪区的复合组件一键复用。
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollBarDesc {
    /// 宿主可滚动内容视口矩形（逻辑像素）。
    pub viewport_rect: Rect,
    /// 沿滚动轴的完整内容长度（逻辑像素）。
    pub content_extent: f32,
    /// 沿滚动轴的当前滚动偏移量（逻辑像素）。
    pub scroll_offset: f32,
    /// 滚动轴方向。
    pub axis: ScrollbarAxis,
    /// 可见性策略（默认 [`ScrollBarVisibility::Auto`]）。
    pub visibility: ScrollBarVisibility,
    /// 绘制顺序键（展开态轨道胶囊位于 `z`，滑块位于 `z + 1`）。
    pub z: u16,
    /// 命中层（默认 [`SCROLL_LAYER`]）。
    pub layer: u8,
    /// 滚动条占据的命中带宽（逻辑像素，默认 [`SCROLLBAR_WIDTH`] = 12）。
    pub width: f32,
    /// 沿滚动轴末端预留的避让长度（双轴并存时避免角落交叠；默认 `0.0`）。
    pub end_inset: f32,
    /// 指针是否悬停在滚动条热区（轨道或滑块）上。
    pub hovered: bool,
    /// 滑块是否正在被拖拽或按压。
    pub dragging: bool,
    /// 滚动条是否处于禁用态。
    pub disabled: bool,
    /// 可选的外部不透明度覆盖；为 `None` 时按 Win11 策略自动取静止 0.55 / 展开 1.0。
    pub opacity_override: Option<f32>,
}

impl ScrollBarDesc {
    /// 构造指定坐标轴的按需叠加滚动条描述符。
    #[must_use]
    pub fn new(
        viewport_rect: Rect,
        content_extent: f32,
        scroll_offset: f32,
        axis: ScrollbarAxis,
    ) -> Self {
        Self {
            viewport_rect,
            content_extent,
            scroll_offset,
            axis,
            visibility: ScrollBarVisibility::Auto,
            z: 700,
            layer: SCROLL_LAYER,
            width: SCROLLBAR_WIDTH,
            end_inset: 0.0,
            hovered: false,
            dragging: false,
            disabled: false,
            opacity_override: None,
        }
    }

    /// 构造垂直按需叠加滚动条描述符。
    #[must_use]
    pub fn vertical(viewport_rect: Rect, content_extent: f32, scroll_offset: f32) -> Self {
        Self::new(
            viewport_rect,
            content_extent,
            scroll_offset,
            ScrollbarAxis::Vertical,
        )
    }

    /// 构造水平按需叠加滚动条描述符。
    #[must_use]
    pub fn horizontal(viewport_rect: Rect, content_extent: f32, scroll_offset: f32) -> Self {
        Self::new(
            viewport_rect,
            content_extent,
            scroll_offset,
            ScrollbarAxis::Horizontal,
        )
    }

    /// 沿当前滚动轴的视口长度。
    #[must_use]
    pub fn viewport_extent(&self) -> f32 {
        if self.axis.horizontal() {
            self.viewport_rect.width()
        } else {
            self.viewport_rect.height()
        }
    }

    /// 沿当前滚动轴允许的最大滚动偏移量。
    #[must_use]
    pub fn max_offset(&self) -> f32 {
        (self.content_extent - self.viewport_extent()).max(0.0)
    }

    /// 内容是否溢出视口。
    #[must_use]
    pub fn is_overflowing(&self) -> bool {
        let viewport = self.viewport_extent();
        viewport > 0.0 && self.content_extent > viewport
    }

    /// 在当前可见性策略与溢出状态下，是否应当发射滚动条图元与命中区。
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.visibility != ScrollBarVisibility::Hidden && self.is_overflowing()
    }

    /// 当前是否处于展开态（6px 粗滑块 + 半透明轨道胶囊）。
    #[must_use]
    pub fn is_expanded(&self) -> bool {
        self.visibility == ScrollBarVisibility::AlwaysExpanded || self.hovered || self.dragging
    }

    /// 滚动条轨道矩形（命中带与展开态轨道胶囊共用）。
    ///
    /// 轨道距视口右沿 / 下沿 [`SCROLLBAR_EDGE_INSET`]（2px），对应 WinUI `ScrollBarThumbOffset`。
    #[must_use]
    pub fn track_rect(&self) -> Rect {
        match self.axis {
            ScrollbarAxis::Horizontal => {
                let len = (self.viewport_rect.width() - self.end_inset).max(0.0);
                Rect::new(
                    self.viewport_rect.origin.x,
                    self.viewport_rect.max().y - self.width - SCROLLBAR_EDGE_INSET,
                    len,
                    self.width,
                )
            }
            ScrollbarAxis::Vertical => {
                let len = (self.viewport_rect.height() - self.end_inset).max(0.0);
                Rect::new(
                    self.viewport_rect.max().x - self.width - SCROLLBAR_EDGE_INSET,
                    self.viewport_rect.origin.y,
                    self.width,
                    len,
                )
            }
        }
    }

    /// 当前滚动偏移量对应的滑块命中矩形（保持 [`Self::width`] 全宽）。
    #[must_use]
    pub fn thumb_rect(&self) -> Rect {
        let track = self.track_rect();
        let horizontal = self.axis.horizontal();
        let viewport = self.viewport_extent();
        let track_length = if horizontal {
            track.width()
        } else {
            track.height()
        };
        if self.content_extent <= viewport || track_length <= 0.0 {
            return track;
        }
        let thumb = thumb_length(track_length, viewport, self.content_extent);
        let max_scroll = (self.content_extent - viewport).max(0.0);
        let progress = if max_scroll > 0.0 {
            (self.scroll_offset / max_scroll).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let travel = track_length - thumb;
        if horizontal {
            Rect::new(
                track.origin.x + travel * progress,
                track.origin.y,
                thumb,
                track.height(),
            )
        } else {
            Rect::new(
                track.origin.x,
                track.origin.y + travel * progress,
                track.width(),
                thumb,
            )
        }
    }

    /// 由拖拽滑块的新指针坐标计算夹紧后的滚动偏移量。
    ///
    /// 与 [`Self::thumb_rect`] 共享同一滑块行程公式，确保绘制与拖拽路径完全一致。
    #[must_use]
    pub fn offset_for_drag(&self, start_offset: f32, start_pointer: f32, pointer: f32) -> f32 {
        let max_scroll = self.max_offset();
        let track = self.track_rect();
        let track_length = if self.axis.horizontal() {
            track.width()
        } else {
            track.height()
        };
        if track_length <= 0.0 || max_scroll <= 0.0 {
            return start_offset.clamp(0.0, max_scroll);
        }
        // 滑块实际行程仅为 `track_length - thumb_length` 而非整条轨道：
        // 若除以整条轨道长度会导致滑块滞后于指针且无法拉到底部。
        let thumb_len = thumb_length(track_length, self.viewport_extent(), self.content_extent);
        let travel = (track_length - thumb_len).max(1.0);
        let ratio = max_scroll / travel;
        let delta = (pointer - start_pointer) * ratio;
        (start_offset + delta).clamp(0.0, max_scroll)
    }

    /// 点击滑块之外的轨道区域时，按一页视口长度计算翻页后的目标偏移量。
    ///
    /// 若未溢出、点击落在滑块内或轨道外则返回 `None`。
    #[must_use]
    pub fn page_offset_for_track_click(&self, click_pos: Vec2) -> Option<f32> {
        if !self.is_overflowing() {
            return None;
        }
        let track = self.track_rect();
        if !track.contains_point(click_pos) {
            return None;
        }
        let thumb = self.thumb_rect();
        if thumb.contains_point(click_pos) {
            return None;
        }
        let direction = if self.axis.horizontal() {
            if click_pos.x < thumb.origin.x {
                -1.0
            } else {
                1.0
            }
        } else if click_pos.y < thumb.origin.y {
            -1.0
        } else {
            1.0
        };
        Some(
            (self.scroll_offset + direction * self.viewport_extent()).clamp(0.0, self.max_offset()),
        )
    }
}

/// 独立按需叠加滚动条的输出结果。
///
/// 不包含裁剪与变换；调用方可通过 [`Self::push_into`] 直接推入 [`SceneCollector`]，
/// 或通过 [`Self::append_to`] 将图元与独立命中区合并至宿主输出。
#[derive(Clone, Debug, Default)]
pub struct ScrollBarOutput {
    /// 滚动条图元（未溢出或 `Hidden` 时为空；溢出时固定 2 个图元槽位：
    /// 索引 0 为轨道胶囊（静止透明、展开半透明），索引 1 为滑块（静止 2px、展开 6px））。
    pub output: WidgetOutput,
    /// 滚动条轨道矩形（未发射时为 `None`）。
    pub track_rect: Option<Rect>,
    /// 滚动条滑块命中矩形（全带宽，未发射时为 `None`）。
    pub thumb_rect: Option<Rect>,
    /// 滚动条可见滑块矩形（静止 2px / 展开 6px，未发射时为 `None`）。
    pub visible_thumb_rect: Option<Rect>,
    /// 滚动条独立命中区：包含滑块命中区（高 z，支持拖拽）与轨道命中区（低 z，支持点击翻页）。
    pub hits: Vec<HitRegion>,
    /// 轨道命中节点标识（未发射时为 `None`）。
    pub track_node: Option<u64>,
    /// 滑块命中节点标识（未发射时为 `None`）。
    pub thumb_node: Option<u64>,
}

impl ScrollBarOutput {
    /// 是否未发射任何图元与命中区。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.output.primitives.is_empty() && self.hits.is_empty()
    }

    /// 将所有滚动条命中区注册到 [`SceneCollector`]。
    pub fn register_hits(&self, collector: &mut SceneCollector) {
        for hit in &self.hits {
            collector.push_hit(hit.clone());
        }
    }

    /// 一次性向 [`SceneCollector`] 推入滚动条图元与全部独立命中区。
    pub fn push_into(self, collector: &mut SceneCollector, owner_key: u64) {
        if self.is_empty() {
            return;
        }
        for hit in &self.hits {
            collector.hit_owners.push((hit.node, owner_key));
        }
        self.register_hits(collector);
        collector.push(owner_key, self.output);
    }

    /// 将图元与独立命中区追加到宿主缓冲区（便于复合组件或 `ScrollView` 内联复用）。
    pub fn append_to(&self, primitives: &mut Vec<GpuPrimitiveInstance>, hits: &mut Vec<HitRegion>) {
        primitives.extend_from_slice(&self.output.primitives);
        hits.extend(self.hits.iter().cloned());
    }
}

/// 发射独立的 Win11 按需叠加滚动条图元与独立命中区（不接管宿主 `clip` / `transform`）。
#[must_use]
pub fn emit_scrollbar(owner_node: u64, desc: &ScrollBarDesc) -> ScrollBarOutput {
    if !desc.is_visible() {
        return ScrollBarOutput::default();
    }

    let track = desc.track_rect();
    if track.width() <= 0.0 || track.height() <= 0.0 {
        return ScrollBarOutput::default();
    }
    let thumb = desc.thumb_rect();
    let horizontal = desc.axis.horizontal();
    let expanded = desc.is_expanded();
    let visible_width = if expanded {
        SCROLLBAR_THUMB_WIDTH
    } else {
        SCROLLBAR_THUMB_IDLE_WIDTH
    };
    let visible_thumb = visible_thumb_rect(thumb, visible_width, horizontal, expanded);

    let thumb_token = if desc.disabled {
        // WinUI `ScrollBarThumbFillDisabled` = `ControlStrongFillColorDisabledBrush`。
        Token::CONTROL_DISABLED_THUMB
    } else {
        // WinUI `ScrollBarThumbFill` = `ControlStrongFillColorDefaultBrush`，
        // 悬停与静止态同色，视觉变化由宽度（2 → 6）、不透明度与轨道胶囊体现。
        Token::CONTROL_TRACK_STROKE
    };
    let thumb_opacity = desc
        .opacity_override
        .unwrap_or(if expanded {
            SCROLLBAR_EXPANDED_OPACITY
        } else {
            SCROLLBAR_IDLE_OPACITY
        })
        .clamp(0.0, 1.0);

    // 保持固定 2 个图元槽位（索引 0 轨道胶囊、索引 1 滑块），确保悬停/按压态切换时
    // 不改变图元计数，满足 `stable_shape` 零重建差分契约：静止态轨道胶囊不透明度为 0.0，
    // 展开态显现为半透明轨道胶囊。
    let mut primitives = Vec::with_capacity(2);
    let track_radius = desc.width * 0.5;
    let mut track_primitive = surface(track, track_radius, Token::OVERLAY_BG, desc.z);
    let track_opacity = if expanded {
        (SCROLLBAR_TRACK_CAPSULE_OPACITY * thumb_opacity).clamp(0.0, 1.0)
    } else {
        0.0
    };
    set_opacity(&mut track_primitive, track_opacity);
    primitives.push(track_primitive);

    let thumb_z = desc.z.saturating_add(1);
    let mut thumb_primitive = surface(visible_thumb, visible_width * 0.5, thumb_token, thumb_z);
    set_opacity(&mut thumb_primitive, thumb_opacity);
    primitives.push(thumb_primitive);

    let thumb_node = scrollbar_thumb_node(owner_node, desc.axis);
    let track_node = scrollbar_track_node(owner_node, desc.axis);
    // 滑块命中区位于更高 z，确保点击滑块优先触发拖拽；轨道命中区覆盖整条轨道，支持点击翻页与热区悬停。
    let hits = vec![
        HitRegion::new(thumb_node, desc.layer, u32::from(desc.z) + 2, thumb)
            .with_cursor(CursorKind::Pointer),
        HitRegion::new(track_node, desc.layer, u32::from(desc.z) + 1, track)
            .with_cursor(CursorKind::Pointer),
    ];

    ScrollBarOutput {
        output: WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit: None,
            a11y: None,
            focusable: false,
            ..Default::default()
        },
        track_rect: Some(track),
        thumb_rect: Some(thumb),
        visible_thumb_rect: Some(visible_thumb),
        hits,
        track_node: Some(track_node),
        thumb_node: Some(thumb_node),
    }
}

/// 滚动视口的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollViewDesc {
    /// 视口矩形。
    pub rect: Rect,
    /// 完整内容尺寸。
    pub content_size: Vec2,
    /// 允许滚动的坐标轴。
    pub axis: ScrollAxis,
    /// 绘制顺序键。
    pub z: u16,
    /// 视口裁剪的圆角半径。
    pub radius: f32,
    /// 滚动条占据的带宽（逻辑像素，WinUI `ScrollBarSize` = 12）。
    pub scrollbar_width: f32,
    /// 滚动条可见性策略（默认 [`ScrollBarVisibility::Auto`]）。
    pub scrollbar_visibility: ScrollBarVisibility,
}

impl ScrollViewDesc {
    #[must_use]
    pub fn new(rect: Rect, content_size: Vec2) -> Self {
        Self {
            rect,
            content_size,
            axis: ScrollAxis::Vertical,
            z: 700,
            radius: 0.0,
            scrollbar_width: SCROLLBAR_WIDTH,
            scrollbar_visibility: ScrollBarVisibility::Auto,
        }
    }

    /// 仍能使内容覆盖视口的最大偏移量。
    #[must_use]
    pub fn max_offset(&self) -> Vec2 {
        Vec2::new(
            (self.content_size.x - self.rect.width()).max(0.0),
            (self.content_size.y - self.rect.height()).max(0.0),
        )
    }

    /// 将提议值夹紧后的合法偏移量。
    #[must_use]
    pub fn clamp_offset(&self, offset: Vec2) -> Vec2 {
        let max = self.max_offset();
        Vec2::new(
            if self.axis.horizontal() {
                offset.x.clamp(0.0, max.x)
            } else {
                0.0
            },
            if self.axis.vertical() {
                offset.y.clamp(0.0, max.y)
            } else {
                0.0
            },
        )
    }

    /// 构造指定坐标轴对应的独立叠加滚动条描述符。
    #[must_use]
    pub fn scrollbar_desc(&self, offset: Vec2, axis: ScrollbarAxis) -> ScrollBarDesc {
        let (content_extent, scroll_offset) = match axis {
            ScrollbarAxis::Horizontal => (self.content_size.x, offset.x),
            ScrollbarAxis::Vertical => (self.content_size.y, offset.y),
        };
        ScrollBarDesc {
            viewport_rect: self.rect,
            content_extent,
            scroll_offset,
            axis,
            visibility: self.scrollbar_visibility,
            z: self.z,
            layer: SCROLL_LAYER,
            width: self.scrollbar_width,
            end_inset: self.scrollbar_width + 4.0,
            hovered: false,
            dragging: false,
            disabled: false,
            opacity_override: None,
        }
    }

    /// 单轴上的滚动条轨道矩形。
    ///
    /// 轨道距视口右沿 / 下沿 2px，对应 WinUI 的 `ScrollBarThumbOffset` = 2。
    #[must_use]
    pub fn track_rect(&self, horizontal: bool) -> Rect {
        let axis = if horizontal {
            ScrollbarAxis::Horizontal
        } else {
            ScrollbarAxis::Vertical
        };
        self.scrollbar_desc(Vec2::ZERO, axis).track_rect()
    }

    /// 给定偏移量对应的滚动条滑块矩形。
    #[must_use]
    pub fn thumb_rect(&self, offset: Vec2, horizontal: bool) -> Rect {
        let axis = if horizontal {
            ScrollbarAxis::Horizontal
        } else {
            ScrollbarAxis::Vertical
        };
        self.scrollbar_desc(offset, axis).thumb_rect()
    }
}

/// 路由至滚动视图滚动条的指针手势。
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ScrollPointerEvent {
    /// 在屏幕空间点处按下。
    Press(Vec2),
    /// 捕获滑块后指针移动。
    Drag(Vec2),
    /// 指针释放。
    Release,
}

/// 单次滚动条拖拽状态。
#[derive(Copy, Clone, Debug)]
struct Drag {
    axis: ScrollbarAxis,
    start_pointer: f32,
    start_offset: Vec2,
}

/// 实时滚动偏移量及其弹簧状态。
#[derive(Clone, Debug)]
pub struct ScrollState {
    spring: Spring2,
    offset: Vec2,
    drag: Option<Drag>,
    bar_opacity: f32,
    idle: f32,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            spring: Spring2::new(SpringPreset::Precise, Vec2::ZERO),
            offset: Vec2::ZERO,
            drag: None,
            bar_opacity: 0.0,
            idle: 0.0,
        }
    }

    /// 当前视觉偏移量。
    #[must_use]
    pub fn offset(&self) -> Vec2 {
        self.offset
    }

    /// 弹簧正在趋向的目标偏移量。
    #[must_use]
    pub fn target(&self) -> Vec2 {
        Vec2::new(self.spring.x.target, self.spring.y.target)
    }

    /// 弹簧是否仍在运动。
    #[must_use]
    pub fn is_animating(&self) -> bool {
        !self.spring.is_settled()
    }

    /// 无动画直接跳转到指定偏移量。
    pub fn jump_to(&mut self, desc: &ScrollViewDesc, offset: Vec2) {
        let offset = desc.clamp_offset(offset);
        self.spring.snap_to(offset);
        self.offset = offset;
        self.reveal_scrollbar();
    }

    /// 累加增量（滚轮格数或拖拽）并夹紧。
    pub fn scroll_by(&mut self, desc: &ScrollViewDesc, delta: Vec2) {
        let target = desc.clamp_offset(self.target() + delta);
        self.spring.set_target(target);
        self.reveal_scrollbar();
    }

    /// 滚动到绝对偏移量并夹紧。
    pub fn scroll_to(&mut self, desc: &ScrollViewDesc, offset: Vec2) {
        let target = desc.clamp_offset(offset);
        self.spring.set_target(target);
        self.reveal_scrollbar();
    }

    /// 滚动条不透明度（`0..=1`）。
    #[must_use]
    pub fn scrollbar_opacity(&self) -> f32 {
        self.bar_opacity
    }

    /// 立即显示滚动条（用于滚轮、悬停或拖拽事件）。
    pub fn reveal_scrollbar(&mut self) {
        self.bar_opacity = 1.0;
        self.idle = 0.0;
    }

    /// 推进滚动条淡出动画。
    ///
    /// 当指针位于视图上方、正在拖拽或弹簧仍在运动时，滚动条保持完全不透明。
    /// 之后保持 [`SCROLLBAR_HOLD`] 时长并在 [`SCROLLBAR_FADE`] 内淡出。
    ///
    /// 返回不透明度是否发生变化。
    pub fn tick_scrollbar(&mut self, dt: f32, hovering: bool) -> bool {
        let before = self.bar_opacity;
        let active = hovering || self.drag.is_some() || self.is_animating();
        if active {
            self.reveal_scrollbar();
        } else {
            self.idle += dt.max(0.0);
            if self.idle > SCROLLBAR_HOLD {
                self.bar_opacity = (self.bar_opacity - dt.max(0.0) / SCROLLBAR_FADE).max(0.0);
            }
        }
        (self.bar_opacity - before).abs() > f32::EPSILON
    }

    /// 推进弹簧物理步进。仍在运动时返回 `true`。
    ///
    /// 每步都会夹紧视觉偏移：猛烈甩动时弹簧可能过冲一两像素，
    /// 而内容绝不应在首行上方露出一道空隙。
    pub fn step(&mut self, desc: &ScrollViewDesc, dt: f32) -> bool {
        let moving = self.spring.step(dt);
        self.offset = desc.clamp_offset(self.spring.value());
        moving
    }

    /// 开始拖拽滚动条滑块。
    pub fn begin_drag(&mut self, axis: ScrollbarAxis, pointer: f32) {
        self.drag = Some(Drag {
            axis,
            start_pointer: pointer,
            start_offset: self.offset,
        });
        self.reveal_scrollbar();
    }

    /// 更新滚动条拖拽。
    ///
    /// 滑块在轨道空间移动，内容在内容空间按相同比例移动，使滑块始终紧跟指针。
    pub fn drag_to(&mut self, desc: &ScrollViewDesc, pointer: f32) {
        let Some(drag) = self.drag else {
            return;
        };
        let bar = desc.scrollbar_desc(drag.start_offset, drag.axis);
        if !bar.is_overflowing() {
            return;
        }
        let track = bar.track_rect();
        let track_length = if drag.axis.horizontal() {
            track.width()
        } else {
            track.height()
        };
        if track_length <= 0.0 {
            return;
        }
        let start = if drag.axis.horizontal() {
            drag.start_offset.x
        } else {
            drag.start_offset.y
        };
        let next = bar.offset_for_drag(start, drag.start_pointer, pointer);
        let mut offset = drag.start_offset;
        if drag.axis.horizontal() {
            offset.x = next;
        } else {
            offset.y = next;
        }
        self.jump_to(desc, offset);
    }

    /// 结束滚动条拖拽。
    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    /// 是否正在拖拽滚动条。
    #[must_use]
    pub fn dragging(&self) -> Option<ScrollbarAxis> {
        self.drag.map(|drag| drag.axis)
    }

    /// 滚动条按下点隐含的目标偏移量（点击轨道将跳转）。
    pub fn jump_from_track(&mut self, desc: &ScrollViewDesc, pointer: Vec2) {
        let horizontal = desc.axis.horizontal()
            && pointer.x >= desc.rect.origin.x
            && pointer.x <= desc.rect.max().x;
        let vertical = desc.axis.vertical()
            && pointer.y >= desc.rect.origin.y
            && pointer.y <= desc.rect.max().y;
        if horizontal {
            let track = desc.track_rect(true);
            if track.contains_point(pointer) {
                let max_scroll = (desc.content_size.x - desc.rect.width()).max(0.0);
                let progress =
                    ((pointer.x - track.origin.x) / track.width().max(1.0)).clamp(0.0, 1.0);
                self.scroll_to(desc, Vec2::new(max_scroll * progress, self.target().y));
                return;
            }
        }
        if vertical {
            let track = desc.track_rect(false);
            if track.contains_point(pointer) {
                let max_scroll = (desc.content_size.y - desc.rect.height()).max(0.0);
                let progress =
                    ((pointer.y - track.origin.y) / track.height().max(1.0)).clamp(0.0, 1.0);
                self.scroll_to(desc, Vec2::new(self.target().x, max_scroll * progress));
            }
        }
    }

    /// 当点击滑块外部的滚动条**轨道**时，按一页视口大小翻页。
    ///
    /// 点击滑块上方/左侧向前翻页，点击下方/右侧向后翻页。返回是否触发了翻页。
    pub fn page_at_track_click(&mut self, desc: &ScrollViewDesc, click_pos: Vec2) -> bool {
        let current = self.offset();
        let mut target = self.target();
        if desc.axis.horizontal() {
            let bar = desc.scrollbar_desc(current, ScrollbarAxis::Horizontal);
            if let Some(next_x) = bar.page_offset_for_track_click(click_pos) {
                let delta = next_x - current.x;
                target.x += delta;
                self.scroll_to(desc, target);
                return true;
            }
        }
        if desc.axis.vertical() {
            let bar = desc.scrollbar_desc(current, ScrollbarAxis::Vertical);
            if let Some(next_y) = bar.page_offset_for_track_click(click_pos) {
                let delta = next_y - current.y;
                target.y += delta;
                self.scroll_to(desc, target);
                return true;
            }
        }
        false
    }

    /// 滚动最小距离以使 `child_rect` 可见。
    ///
    /// `child_rect` 处于内容坐标系：原点在内容左上角，偏移 `o` 对应显示窗口 `[o, o + viewport]`。
    /// 此处调整弹簧目标而非直接修改视觉偏移，使视口外获焦的子元素平滑动画滚入。
    /// 返回目标是否发生移动。
    pub fn scroll_into_view(&mut self, desc: &ScrollViewDesc, child_rect: Rect) -> bool {
        let before = self.target();
        let mut target = before;
        if desc.axis.vertical() {
            let height = desc.rect.height();
            if child_rect.origin.y < target.y {
                target.y = child_rect.origin.y;
            } else if child_rect.max().y > target.y + height {
                target.y = child_rect.max().y - height;
            }
        }
        if desc.axis.horizontal() {
            let width = desc.rect.width();
            if child_rect.origin.x < target.x {
                target.x = child_rect.origin.x;
            } else if child_rect.max().x > target.x + width {
                target.x = child_rect.max().x - width;
            }
        }
        if target == before {
            return false;
        }
        self.scroll_to(desc, target);
        true
    }

    /// 将释放时的惯性速度投射到弹簧目标。
    ///
    /// `velocity` 单位为逻辑像素/秒；按 [`FLING_PROJECTION`] 秒外推并夹紧到内容边界，
    /// 使快速轻扫先滑行一段距离，随后精确停在内容末尾而不会越界。
    pub fn fling(&mut self, desc: &ScrollViewDesc, velocity: Vec2) {
        self.scroll_to(desc, self.offset() + velocity * FLING_PROJECTION);
    }

    /// 将指针手势归约为滚动条状态变化。
    ///
    /// 按下滑块将其就地捕获；按下轨道翻动一页；拖拽移动已捕获的滑块；释放结束拖拽。
    /// 返回偏移量或捕获状态是否变化。这是宿主接入指针事件的唯一入口，
    /// 确保滑块抓取、轨道点击与释放清理不会各自漂移。
    pub fn reduce_pointer(&mut self, desc: &ScrollViewDesc, event: ScrollPointerEvent) -> bool {
        match event {
            ScrollPointerEvent::Press(pos) => {
                if desc.axis.vertical() && desc.content_size.y > desc.rect.height() {
                    let thumb = desc.thumb_rect(self.offset(), false);
                    if thumb.contains_point(pos) {
                        self.begin_drag(ScrollbarAxis::Vertical, pos.y);
                        return true;
                    }
                }
                if desc.axis.horizontal() && desc.content_size.x > desc.rect.width() {
                    let thumb = desc.thumb_rect(self.offset(), true);
                    if thumb.contains_point(pos) {
                        self.begin_drag(ScrollbarAxis::Horizontal, pos.x);
                        return true;
                    }
                }
                self.page_at_track_click(desc, pos)
            }
            ScrollPointerEvent::Drag(pos) => {
                let Some(axis) = self.dragging() else {
                    return false;
                };
                let pointer = if axis.horizontal() { pos.x } else { pos.y };
                self.drag_to(desc, pointer);
                true
            }
            ScrollPointerEvent::Release => {
                let was_dragging = self.drag.is_some();
                self.end_drag();
                was_dragging
            }
        }
    }
}

impl ScrollViewOutput {
    /// 在视口裁剪与滚动平移作用域内收集子元素。
    ///
    /// 这是 [`emit_scroll_view`] 与 [`SceneCollector::push_clip`] / [`SceneCollector::push_transform`] 的桥梁：
    /// 闭包推入的每个图元都会继承这两者，且即使闭包未输出任何图元也会正确弹出作用域。
    pub fn collect_children<F>(&self, collector: &mut SceneCollector, viewport: Rect, children: F)
    where
        F: FnOnce(&mut SceneCollector),
    {
        collector.push_clip(self.clip, viewport);
        collector.push_transform(self.transform);
        children(collector);
        collector.pop_transform();
        collector.pop_clip();
    }

    /// 向收集器注册所有滚动条滑块与轨道的命中区域。
    ///
    /// [`SceneCollector::push`] 仅收集控件自身的单个 `hit`，
    /// 因此位于 [`WidgetOutput`] 之外的滚动条命中区若不注册，每帧都会被静默丢弃而无法交互。
    pub fn register_hits(&self, collector: &mut SceneCollector) {
        for hit in &self.hits {
            collector.push_hit(hit.clone());
        }
    }

    /// 一次性推入滚动视图自身输出**以及**滚动条命中区域，防止调用方遗漏。
    pub fn push_into(self, collector: &mut SceneCollector, key: u64) {
        for hit in &self.hits {
            collector.hit_owners.push((hit.node, key));
        }
        self.register_hits(collector);
        collector.push(key, self.output);
    }
}

/// 滚动视图的输出结果。
///
/// 裁剪与变换与 [`WidgetOutput`] 分离，因为它们作用于**子元素**；
/// 滚动视图自身仅绘制滚动条。
#[derive(Clone, Debug)]
pub struct ScrollViewOutput {
    /// 滚动条及滚轮命中区域。
    pub output: WidgetOutput,
    /// 所有子图元必须引用的裁剪区域。
    pub clip: GpuClipNode,
    /// 所有子图元必须引用的平移变换。
    pub transform: GpuTransform,
    /// 滚动条命中区域（先垂直后水平，每个可见滚动条包含滑块与轨道两个命中区）。
    /// 请通过 [`Self::register_hits`] 注册，以便路由器拾取滑块拖拽与轨道翻页。
    pub hits: Vec<HitRegion>,
}

/// 绑定了节点标识的滚动视图描述符。
#[derive(Clone, Debug)]
pub struct ScrollViewWidget {
    /// 控件标识。
    pub node: u64,
    /// 描述符。
    pub desc: ScrollViewDesc,
    /// 实时偏移量。
    pub state: ScrollState,
}

impl WidgetDesc for ScrollViewWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_scroll_view(self.node, &self.desc, &self.state, state).output
    }
}

/// 输出滚动条、裁剪区域与子元素变换。
///
/// 内部复用独立叠加滚动条内核 [`emit_scrollbar`]，同时严格保持未溢出时不发射任何滚动条图元与命中区。
#[must_use]
pub fn emit_scroll_view(
    node: u64,
    desc: &ScrollViewDesc,
    scroll: &ScrollState,
    state: ControlState,
) -> ScrollViewOutput {
    let rect = desc.rect;
    let offset = scroll.offset();
    let mut primitives: Vec<GpuPrimitiveInstance> = Vec::with_capacity(4);
    let mut hits: Vec<HitRegion> = Vec::with_capacity(4);

    let pointer_hot = matches!(state.phase, PointerPhase::Hovered | PointerPhase::Pressed);
    let idle_opacity = scroll.scrollbar_opacity().max(SCROLLBAR_IDLE_OPACITY);

    let mut emit_axis = |axis: ScrollbarAxis| {
        let mut bar = desc.scrollbar_desc(offset, axis);
        bar.hovered = pointer_hot;
        bar.dragging = scroll.dragging() == Some(axis);
        bar.disabled = state.disabled;
        if !bar.is_expanded() {
            bar.opacity_override = Some(idle_opacity);
        }
        let bar_out = emit_scrollbar(node, &bar);
        bar_out.append_to(&mut primitives, &mut hits);
    };

    if desc.axis.vertical() {
        emit_axis(ScrollbarAxis::Vertical);
    }
    if desc.axis.horizontal() {
        emit_axis(ScrollbarAxis::Horizontal);
    }

    let viewport_z = desc.z.saturating_sub(740);
    let hit = Some(HitRegion::new(
        node,
        SCROLL_LAYER,
        u32::from(viewport_z),
        rect,
    ));
    let mut a11y =
        unbound_a11y::A11yNode::new(accesskit::Role::ScrollView, "", rect).disabled(state.disabled);
    a11y.actions
        .insert(unbound_a11y::A11yAction::ScrollIntoView);
    a11y.actions.insert(unbound_a11y::A11yAction::Focus);

    let clip = GpuClipNode::from_aabb([rect.min().x, rect.min().y, rect.max().x, rect.max().y]);
    let mut transform = GpuTransform::IDENTITY;
    transform.translation = [-offset.x, -offset.y];
    if offset != Vec2::ZERO {
        transform.flags = unbound_gpu::data::TransformFlags::ANIMATED.bits();
    }

    ScrollViewOutput {
        output: WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        clip,
        transform,
        hits,
    }
}

/// 单帧之后虚拟列表窗口的状态。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct VirtualWindow {
    /// 来自 [`VirtualList::update`] 的槽位/绑定统计。
    pub update: unbound_world::virtualize::WindowUpdate,
    /// **整个**列表的变换（含亚行偏移）。
    pub transform: GpuTransform,
    /// 当前已实例化的首个条目索引。
    pub first_item: usize,
    /// 当前存活的槽位数量。
    pub slots: usize,
}

/// 根据滚动视图更新虚拟列表。
///
/// 亚行滚动仅修改 `transform`：`update.rebound == 0` 且不重新绑定数据，
/// 这正是实现“10 万行 @ 120 FPS”的关键路径。
#[must_use]
pub fn update_virtual_list(
    list: &mut VirtualList,
    desc: &ScrollViewDesc,
    scroll: &ScrollState,
) -> VirtualWindow {
    let offset = scroll.offset();
    let update = list.update(offset.y, desc.rect.height(), list_total(list, desc));
    let mut transform = GpuTransform::IDENTITY;
    // `VirtualList` 将亚行余量报告为 `offset`，但调用方定位槽位时还需要整行偏移。
    transform.translation = [-offset.x, -offset.y];
    VirtualWindow {
        update,
        transform,
        first_item: list.first_item(),
        slots: list.slot_count(),
    }
}

/// 列表当前报告的总行数。
///
/// `VirtualList` 维护自身的 `total`，而滚动描述符持有内容总高度。
/// 通过行高反向换算可保持单一数据源（描述符），无需重复维护条目数。
fn list_total(list: &VirtualList, desc: &ScrollViewDesc) -> usize {
    let rows = (desc.content_size.y / list.row_height()).round();
    if rows.is_finite() && rows >= 0.0 {
        rows as usize
    } else {
        0
    }
}

fn set_opacity(primitive: &mut GpuPrimitiveInstance, opacity: f32) {
    primitive.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(primitive.depth_u16(), opacity);
}

/// 虚拟列表中单行在视口空间的位置。
#[must_use]
pub fn virtual_row_rect(
    desc: &ScrollViewDesc,
    scroll: &ScrollState,
    row_index: usize,
    row_height: f32,
) -> Rect {
    Rect::new(
        desc.rect.origin.x,
        desc.rect.origin.y + row_index as f32 * row_height - scroll.offset().y,
        desc.rect.width(),
        row_height,
    )
}

/// 将滚动条滑块绘制为独立图元（供测试与工具使用）。
///
/// 与 [`emit_scroll_view`] 一致：按展开态宽度 [`SCROLLBAR_THUMB_WIDTH`] 绘制，
/// 颜色取 WinUI `ScrollBarThumbFill`（`ControlStrongFillColorDefault`）。
#[must_use]
pub fn scrollbar_thumb_primitive(
    desc: &ScrollViewDesc,
    offset: Vec2,
    horizontal: bool,
) -> GpuPrimitiveInstance {
    let thumb = visible_thumb_rect(
        desc.thumb_rect(offset, horizontal),
        SCROLLBAR_THUMB_WIDTH,
        horizontal,
        true,
    );
    let mut primitive = surface(
        thumb,
        SCROLLBAR_THUMB_WIDTH * 0.5,
        Token::CONTROL_TRACK_STROKE,
        desc.z,
    );
    primitive.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Background as u16,
    );
    primitive
}

/// 临界阻尼平滑滚轮插值器。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct SmoothWheelHandler {
    pub current: f32,
    pub target: f32,
    pub velocity: f32,
}

impl SmoothWheelHandler {
    #[must_use]
    pub const fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            velocity: 0.0,
        }
    }

    /// 注入滚轮脉冲并钳制在 `[min, max]` 范围内。
    pub fn push_wheel_delta(&mut self, delta: f32, min: f32, max: f32) {
        self.target = (self.target + delta).clamp(min, max);
    }

    /// 推进单帧阻尼弹簧步进（返回是否仍在运动）。
    pub fn tick(&mut self, dt: f32) -> bool {
        let diff = self.target - self.current;
        if diff.abs() < 0.05 && self.velocity.abs() < 0.05 {
            self.current = self.target;
            self.velocity = 0.0;
            return false;
        }
        let omega = 22.0_f32;
        let step = dt.clamp(0.001, 0.05);
        let accel = omega * omega * diff - 2.0 * omega * self.velocity;
        self.velocity += accel * step;
        self.current += self.velocity * step;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ScrollViewDesc {
        ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(400.0, 3000.0))
    }

    #[test]
    fn wheel_scrolling_is_clamped_to_the_content() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.scroll_by(&desc, Vec2::new(0.0, 120.0));
        state.step(&desc, 1.0 / 60.0);
        assert!(state.offset().y > 0.0);

        state.scroll_by(&desc, Vec2::new(0.0, 1_000_000.0));
        assert_eq!(state.target().y, 2700.0);
        for _ in 0..240 {
            if !state.step(&desc, 1.0 / 60.0) {
                break;
            }
        }
        assert!(
            (state.offset().y - 2700.0).abs() < 0.5,
            "{}",
            state.offset().y
        );

        state.scroll_by(&desc, Vec2::new(0.0, -1_000_000.0));
        for _ in 0..240 {
            if !state.step(&desc, 1.0 / 60.0) {
                break;
            }
        }
        assert_eq!(state.offset().y, 0.0);
    }

    #[test]
    fn a_vertical_only_view_never_scrolls_horizontally() {
        let desc = ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(800.0, 3000.0));
        let mut state = ScrollState::new();
        state.scroll_by(&desc, Vec2::new(200.0, 100.0));
        state.step(&desc, 1.0 / 60.0);
        assert_eq!(state.offset().x, 0.0);
        assert!(state.offset().y > 0.0);
    }

    #[test]
    fn the_thumb_ratio_tracks_the_viewport() {
        let desc = ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(400.0, 600.0));
        let track = desc.track_rect(false);
        let thumb = desc.thumb_rect(Vec2::ZERO, false);
        assert!((thumb.height() - track.height() * 0.5).abs() < 1e-3);

        assert_eq!(
            desc.thumb_rect(Vec2::new(0.0, 300.0), false).max().y,
            track.max().y
        );
        assert_eq!(
            desc.thumb_rect(Vec2::new(0.0, 300.0), false).height(),
            thumb.height()
        );
    }

    #[test]
    fn the_thumb_is_never_smaller_than_the_minimum() {
        let desc = ScrollViewDesc::new(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            Vec2::new(400.0, 100_000.0),
        );
        assert!(desc.thumb_rect(Vec2::ZERO, false).height() >= MIN_THUMB);
    }

    #[test]
    fn dragging_the_thumb_moves_the_content_by_the_track_ratio() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.begin_drag(ScrollbarAxis::Vertical, 10.0);
        assert_eq!(state.dragging(), Some(ScrollbarAxis::Vertical));
        state.drag_to(&desc, 110.0);
        assert!(state.offset().y > 0.0);
        state.end_drag();
        assert!(state.dragging().is_none());
    }

    #[test]
    fn a_thumb_drag_is_clamped_at_both_ends() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.begin_drag(ScrollbarAxis::Vertical, 100.0);
        state.drag_to(&desc, 100_000.0);
        assert_eq!(state.offset().y, 2700.0);
        state.drag_to(&desc, -100_000.0);
        assert_eq!(state.offset().y, 0.0);
    }

    #[test]
    fn clicking_the_track_jumps_towards_the_click() {
        let desc = desc();
        let mut state = ScrollState::new();
        let track = desc.track_rect(false);
        state.jump_from_track(
            &desc,
            Vec2::new(track.center().x, track.origin.y + track.height() * 0.5),
        );
        assert!(state.target().y > 1000.0);
    }

    #[test]
    fn the_scrollbar_fades_after_the_hold_expires() {
        let desc = desc();
        let mut state = ScrollState::new();
        assert_eq!(state.scrollbar_opacity(), 0.0);

        state.scroll_by(&desc, Vec2::new(0.0, 60.0));
        assert_eq!(state.scrollbar_opacity(), 1.0);

        // 弹簧仍在运动，因此滚动条保持可见。
        state.tick_scrollbar(0.016, false);
        assert_eq!(state.scrollbar_opacity(), 1.0);

        // 立即跳转稳定，随后保持一秒。
        state.jump_to(&desc, Vec2::new(0.0, 120.0));
        assert_eq!(state.scrollbar_opacity(), 1.0);
        state.tick_scrollbar(SCROLLBAR_HOLD + 0.01, false);
        assert!(state.scrollbar_opacity() < 1.0);
        state.tick_scrollbar(SCROLLBAR_FADE, false);
        assert_eq!(state.scrollbar_opacity(), 0.0);
    }

    #[test]
    fn hovering_reveals_the_scrollbar_without_scrolling() {
        let mut state = ScrollState::new();
        assert_eq!(state.scrollbar_opacity(), 0.0);
        assert!(state.tick_scrollbar(0.016, true));
        assert_eq!(state.scrollbar_opacity(), 1.0);
    }

    #[test]
    fn scrolling_reveals_the_bar_and_the_emitter_writes_its_opacity() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.jump_to(&desc, Vec2::new(0.0, 60.0));
        let output = emit_scroll_view(9, &desc, &state, ControlState::default());
        // 静止态保持稳定双槽位：索引 0 轨道胶囊全透明，索引 1 为 2px 细指示条。
        assert_eq!(
            output.output.primitives.len(),
            2,
            "stable two-slot output: track capsule + thumb"
        );
        assert_eq!(output.output.primitives[0].opacity(), 0.0);
        assert_eq!(
            output.output.primitives[1].size[0],
            SCROLLBAR_THUMB_IDLE_WIDTH
        );
        assert!(output.output.primitives[1].opacity() > 0.99);

        // 悬停展开为半透明轨道胶囊 + 6px 粗滑块。
        let hovered = emit_scroll_view(
            9,
            &desc,
            &state,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
        );
        assert_eq!(
            hovered.output.primitives.len(),
            2,
            "expanded state paints track capsule and thumb"
        );
        assert_eq!(hovered.output.primitives[0].size[0], SCROLLBAR_WIDTH);
        assert_eq!(hovered.output.primitives[1].size[0], SCROLLBAR_THUMB_WIDTH);
    }

    #[test]
    fn the_clip_covers_the_viewport_and_the_transform_owns_the_offset() {
        let desc = desc();
        let mut state = ScrollState::new();
        state.jump_to(&desc, Vec2::new(0.0, 120.0));
        let output = emit_scroll_view(7, &desc, &state, ControlState::default());
        assert_eq!(output.clip.aabb_clip, [0.0, 0.0, 400.0, 300.0]);
        assert_eq!(output.transform.translation, [0.0, -120.0]);
        assert!(output
            .output
            .hit
            .as_ref()
            .is_some_and(|hit| hit.bounds.contains_rect(&desc.rect)));
    }

    #[test]
    fn a_sub_row_scroll_rebinds_nothing() {
        let desc = desc();
        let mut state = ScrollState::new();
        let mut list = VirtualList::new(28.0, 1.0);
        let first = update_virtual_list(&mut list, &desc, &state);
        assert!(first.slots > 0);

        state.jump_to(&desc, Vec2::new(0.0, 7.5));
        let second = update_virtual_list(&mut list, &desc, &state);
        assert!(second.update.offset_only, "{:?}", second.update);
        assert_eq!(second.update.rebound, 0);
        assert_eq!(second.update.offset, -7.5);
        assert_eq!(second.transform.translation, [0.0, -7.5]);
    }

    #[test]
    fn a_virtual_list_keeps_its_entity_budget_across_a_long_scroll() {
        let desc = ScrollViewDesc::new(
            Rect::new(0.0, 0.0, 400.0, 800.0),
            Vec2::new(400.0, 28.0 * 100_000.0),
        );
        let mut state = ScrollState::new();
        let mut list = VirtualList::new(28.0, 1.0);
        let _ = update_virtual_list(&mut list, &desc, &state);
        let budget = list.slot_count();
        for offset in [28.0, 2_800.0, 280_000.0, 2_800_000.0] {
            state.jump_to(&desc, Vec2::new(0.0, offset));
            let _ = update_virtual_list(&mut list, &desc, &state);
            assert_eq!(list.slot_count(), budget);
        }
        assert!((28..=120).contains(&budget), "{budget}");
    }

    #[test]
    fn dragging_thumb_across_travel_reaches_exact_max_scroll() {
        let desc = desc();
        let track = desc.track_rect(false);
        let thumb = desc.thumb_rect(Vec2::ZERO, false);
        let travel = track.height() - thumb.height();
        let mut state = ScrollState::new();
        let start = track.origin.y + thumb.height() * 0.5;
        state.begin_drag(ScrollbarAxis::Vertical, start);
        state.drag_to(&desc, start + travel);
        assert!(
            (state.offset().y - desc.max_offset().y).abs() < 1e-3,
            "{} should equal {}",
            state.offset().y,
            desc.max_offset().y
        );
    }

    #[test]
    fn both_scrollbar_thumbs_and_tracks_are_pickable_and_registered() {
        let mut desc =
            ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(800.0, 3000.0));
        desc.axis = ScrollAxis::Both;
        let state = ScrollState::new();
        let output = emit_scroll_view(90, &desc, &state, ControlState::default());
        assert_eq!(
            output.hits.len(),
            4,
            "vertical and horizontal thumb + track hits"
        );

        let mut collector = SceneCollector::new(1);
        output.push_into(&mut collector, 90);
        assert_eq!(
            collector.regions.len(),
            5,
            "viewport hit plus two thumbs and two tracks"
        );
        assert!(collector
            .regions
            .iter()
            .any(|r| r.node == scrollbar_thumb_node(90, ScrollbarAxis::Vertical)));
        assert!(collector
            .regions
            .iter()
            .any(|r| r.node == scrollbar_track_node(90, ScrollbarAxis::Vertical)));
        assert!(collector
            .regions
            .iter()
            .any(|r| r.node == scrollbar_thumb_node(90, ScrollbarAxis::Horizontal)));
        assert!(collector
            .regions
            .iter()
            .any(|r| r.node == scrollbar_track_node(90, ScrollbarAxis::Horizontal)));
    }

    #[test]
    fn reduce_pointer_grabs_the_thumb_and_drags_to_the_end() {
        let desc = desc();
        let mut state = ScrollState::new();
        let thumb = desc.thumb_rect(Vec2::ZERO, false);
        let press = thumb.center();
        assert!(state.reduce_pointer(&desc, ScrollPointerEvent::Press(press)));
        assert_eq!(state.dragging(), Some(ScrollbarAxis::Vertical));

        let track = desc.track_rect(false);
        let travel = track.height() - thumb.height();
        assert!(state.reduce_pointer(
            &desc,
            ScrollPointerEvent::Drag(Vec2::new(press.x, press.y + travel))
        ));
        assert!((state.offset().y - desc.max_offset().y).abs() < 1e-3);
        assert!(state.reduce_pointer(&desc, ScrollPointerEvent::Release));
        assert!(state.dragging().is_none());
    }

    #[test]
    fn reduce_pointer_pages_on_a_track_click() {
        let desc = desc();
        let mut state = ScrollState::new();
        let track = desc.track_rect(false);
        let below = Vec2::new(track.center().x, track.max().y - 2.0);
        assert!(state.reduce_pointer(&desc, ScrollPointerEvent::Press(below)));
        assert_eq!(state.target().y, desc.rect.height());
        assert!(state.dragging().is_none(), "a track click does not capture");
    }

    #[test]
    fn overlay_scrollbar_emits_nothing_when_content_fits_viewport() {
        let viewport = Rect::new(10.0, 20.0, 320.0, 240.0);
        let exact = ScrollBarDesc::vertical(viewport, 240.0, 0.0);
        let shorter = ScrollBarDesc::vertical(viewport, 120.0, 0.0);

        for bar in [exact, shorter] {
            let out = emit_scrollbar(42, &bar);
            assert!(out.is_empty());
            assert!(out.output.primitives.is_empty());
            assert!(out.hits.is_empty());
            assert!(out.track_rect.is_none());
            assert!(out.thumb_rect.is_none());
            assert!(out.visible_thumb_rect.is_none());
            assert!(out.track_node.is_none());
            assert!(out.thumb_node.is_none());
        }
    }

    #[test]
    fn overlay_scrollbar_auto_idle_emits_2px_translucent_indicator_and_both_hits() {
        let viewport = Rect::new(0.0, 0.0, 320.0, 240.0);
        let bar = ScrollBarDesc::vertical(viewport, 960.0, 0.0);
        let out = emit_scrollbar(42, &bar);

        assert!(!out.is_empty());
        assert_eq!(out.output.primitives.len(), 2);
        assert_eq!(out.output.primitives[0].opacity(), 0.0);
        let track = out.track_rect.expect("overflowing track rect");
        let thumb_prim = &out.output.primitives[1];
        assert_eq!(thumb_prim.size[0], SCROLLBAR_THUMB_IDLE_WIDTH);
        let expected_idle_x = track.max().x - SCROLLBAR_EDGE_INSET - SCROLLBAR_THUMB_IDLE_WIDTH;
        assert!(
            (thumb_prim.local_pos[0] - expected_idle_x).abs() < 1e-3,
            "idle 2px vertical thumb must hug outer right edge: got {}, expected {expected_idle_x}",
            thumb_prim.local_pos[0]
        );
        assert!((thumb_prim.opacity() - SCROLLBAR_IDLE_OPACITY).abs() < 0.02);

        assert_eq!(out.hits.len(), 2);
        assert_eq!(
            out.hits[0].node,
            scrollbar_thumb_node(42, ScrollbarAxis::Vertical)
        );
        assert_eq!(
            out.hits[1].node,
            scrollbar_track_node(42, ScrollbarAxis::Vertical)
        );
        assert!(
            out.hits[0].z > out.hits[1].z,
            "thumb hit must sit above track hit"
        );
        assert_eq!(Some(out.hits[0].bounds), out.thumb_rect);
        assert_eq!(Some(out.hits[1].bounds), out.track_rect);
    }

    #[test]
    fn overlay_scrollbar_hovered_and_dragging_expand_to_6px_thumb_and_track_capsule() {
        let viewport = Rect::new(0.0, 0.0, 320.0, 240.0);

        let mut hovered = ScrollBarDesc::vertical(viewport, 960.0, 120.0);
        hovered.hovered = true;
        let mut dragging = ScrollBarDesc::vertical(viewport, 960.0, 120.0);
        dragging.dragging = true;

        for bar in [hovered, dragging] {
            let out = emit_scrollbar(77, &bar);
            assert_eq!(out.output.primitives.len(), 2);
            let track_capsule = &out.output.primitives[0];
            let thumb_prim = &out.output.primitives[1];

            let track = out.track_rect.expect("expanded track rect");
            assert_eq!(track_capsule.size[0], SCROLLBAR_WIDTH);
            assert!((track_capsule.opacity() - SCROLLBAR_TRACK_CAPSULE_OPACITY).abs() < 0.02);
            assert_eq!(thumb_prim.size[0], SCROLLBAR_THUMB_WIDTH);
            let expected_expanded_x =
                track.origin.x + (track.width() - SCROLLBAR_THUMB_WIDTH) * 0.5;
            assert!(
                (thumb_prim.local_pos[0] - expected_expanded_x).abs() < 1e-3,
                "expanded 6px thumb must stay centered in 12px track capsule"
            );
            assert!(thumb_prim.opacity() > 0.99);
            assert!(
                thumb_prim.depth_u16() > track_capsule.depth_u16(),
                "thumb must render above track capsule"
            );
            assert_eq!(out.hits.len(), 2);
        }
    }

    #[test]
    fn overlay_scrollbar_visibility_modes_hidden_and_always_expanded() {
        let viewport = Rect::new(0.0, 0.0, 320.0, 240.0);

        // Hidden 模式即使溢出也不发射任何图元与命中区。
        let mut hidden = ScrollBarDesc::vertical(viewport, 1200.0, 100.0);
        hidden.visibility = ScrollBarVisibility::Hidden;
        hidden.hovered = true;
        let hidden_out = emit_scrollbar(10, &hidden);
        assert!(hidden_out.is_empty());

        // AlwaysExpanded 在溢出时无需悬停即展开为 6px 滑块 + 半透明轨道胶囊。
        let mut always = ScrollBarDesc::vertical(viewport, 1200.0, 100.0);
        always.visibility = ScrollBarVisibility::AlwaysExpanded;
        let always_out = emit_scrollbar(10, &always);
        assert_eq!(always_out.output.primitives.len(), 2);
        assert_eq!(always_out.output.primitives[0].size[0], SCROLLBAR_WIDTH);
        assert_eq!(
            always_out.output.primitives[1].size[0],
            SCROLLBAR_THUMB_WIDTH
        );

        // AlwaysExpanded 未溢出时仍保持 0 图元、0 命中区。
        let mut always_fit = ScrollBarDesc::vertical(viewport, 200.0, 0.0);
        always_fit.visibility = ScrollBarVisibility::AlwaysExpanded;
        assert!(emit_scrollbar(10, &always_fit).is_empty());
    }

    #[test]
    fn overlay_scrollbar_horizontal_geometry_and_sub_hit_ids() {
        let viewport = Rect::new(0.0, 0.0, 400.0, 200.0);
        let mut bar = ScrollBarDesc::horizontal(viewport, 1200.0, 200.0);
        let idle = emit_scrollbar(55, &bar);
        assert_eq!(idle.output.primitives.len(), 2);
        assert_eq!(idle.output.primitives[0].opacity(), 0.0);
        let h_track = idle.track_rect.expect("horizontal track rect");
        assert_eq!(
            idle.output.primitives[1].size[1],
            SCROLLBAR_THUMB_IDLE_WIDTH
        );
        let expected_idle_y = h_track.max().y - SCROLLBAR_EDGE_INSET - SCROLLBAR_THUMB_IDLE_WIDTH;
        assert!(
            (idle.output.primitives[1].local_pos[1] - expected_idle_y).abs() < 1e-3,
            "idle 2px horizontal thumb must hug outer bottom edge: got {}, expected {expected_idle_y}",
            idle.output.primitives[1].local_pos[1]
        );
        assert_eq!(
            idle.thumb_node,
            Some(scrollbar_thumb_node(55, ScrollbarAxis::Horizontal))
        );
        assert_eq!(
            idle.track_node,
            Some(scrollbar_track_node(55, ScrollbarAxis::Horizontal))
        );

        bar.hovered = true;
        let expanded = emit_scrollbar(55, &bar);
        assert_eq!(expanded.output.primitives.len(), 2);
        assert_eq!(expanded.output.primitives[0].size[1], SCROLLBAR_WIDTH);
        assert_eq!(expanded.output.primitives[1].size[1], SCROLLBAR_THUMB_WIDTH);
    }

    #[test]
    fn overlay_scrollbar_drag_and_track_page_math_match_thumb_geometry() {
        let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
        let bar = ScrollBarDesc::vertical(viewport, 3000.0, 0.0);
        let track = bar.track_rect();
        let thumb = bar.thumb_rect();
        let travel = track.height() - thumb.height();

        let end_offset = bar.offset_for_drag(0.0, thumb.center().y, thumb.center().y + travel);
        assert!((end_offset - bar.max_offset()).abs() < 1e-3);

        // 点击滑块下方轨道向后翻一页。
        let below = Vec2::new(track.center().x, track.max().y - 4.0);
        assert_eq!(bar.page_offset_for_track_click(below), Some(300.0));

        // 位于中间偏移时，点击滑块上方轨道向前翻一页；点击滑块本身不翻页。
        let mid_bar = ScrollBarDesc::vertical(viewport, 3000.0, 600.0);
        let mid_thumb = mid_bar.thumb_rect();
        let above = Vec2::new(track.center().x, track.origin.y + 2.0);
        assert_eq!(mid_bar.page_offset_for_track_click(above), Some(300.0));
        assert_eq!(
            mid_bar.page_offset_for_track_click(mid_thumb.center()),
            None
        );
        assert_eq!(
            mid_bar.page_offset_for_track_click(Vec2::new(10.0, 10.0)),
            None
        );
    }

    #[test]
    fn scroll_view_idle_overflow_shows_2px_indicator_and_hidden_hides_bar() {
        let mut desc = desc();
        let fresh = ScrollState::new();
        // 即使尚未滚动（bar_opacity == 0.0），溢出时仍显示克制的 2px 半透明指示条。
        let idle = emit_scroll_view(90, &desc, &fresh, ControlState::default());
        assert_eq!(idle.output.primitives.len(), 2);
        assert_eq!(idle.output.primitives[0].opacity(), 0.0);
        assert_eq!(
            idle.output.primitives[1].size[0],
            SCROLLBAR_THUMB_IDLE_WIDTH
        );
        assert!((idle.output.primitives[1].opacity() - SCROLLBAR_IDLE_OPACITY).abs() < 0.02);
        assert_eq!(idle.hits.len(), 2);

        // Hidden 模式不发射滚动条图元与命中区，仅保留视口滚轮命中区。
        desc.scrollbar_visibility = ScrollBarVisibility::Hidden;
        let hidden = emit_scroll_view(90, &desc, &fresh, ControlState::default());
        assert!(hidden.output.primitives.is_empty());
        assert!(hidden.hits.is_empty());
        assert!(hidden.output.hit.is_some());
    }
}
