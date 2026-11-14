//! 由 guest 生成、host 消费的扁平 POD 视图树描述符。
//!
//! `docs/architecture/principles.md` 禁止深层泛型 typestate 树：guest
//! 通过 C 函数指针为每个节点发出**扁平具体描述符**，host 再将其
//! 转换为 `LayoutNode` 实体。这也正是 LLM 无需臆测 API 即可生成
//! 的结构——接口表面仅有一个结构体和一次调用。
//!
//!
//! 枚举字段有意存储为 `u8`：在 `Pod` 结构体中使用 `#[repr(u8)]` 枚举
//! 会导致 `bytemuck::from_bytes` 对任意 guest 字节流产生未定义行为。
//! 每个读取端都会经过 `from_u8` 归一化函数，将未知判别值（来自新版本
//! ABI 编译的 guest）映射为安全默认值。

use bytemuck::{Pod, Zeroable};

/// 单次构建过程中的稳定节点标识符。
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Pod, Zeroable)]
pub struct RawNodeId(pub u32);

impl RawNodeId {
    pub const ROOT: Self = Self(0);

    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == NO_NODE
    }
}

/// 表示“无节点”的哨兵值。`u32::MAX` 永不为合法 arena 索引。
pub const NO_NODE: u32 = u32::MAX;

#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum DimUnit {
    #[default]
    Auto = 0,
    Px = 1,
    Percent = 2,
    Fr = 3,
}

impl DimUnit {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Px,
            2 => Self::Percent,
            3 => Self::Fr,
            _ => Self::Auto,
        }
    }
}

/// 单个布局维度：`{unit, value}`。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Dim {
    pub unit: u8,
    pub _pad: [u8; 3],
    pub value: f32,
}

impl Dim {
    pub const AUTO: Self = Self {
        unit: DimUnit::Auto as u8,
        _pad: [0; 3],
        value: 0.0,
    };

    #[inline]
    #[must_use]
    pub const fn px(v: f32) -> Self {
        Self {
            unit: DimUnit::Px as u8,
            _pad: [0; 3],
            value: v,
        }
    }

    #[inline]
    #[must_use]
    pub const fn percent(v: f32) -> Self {
        Self {
            unit: DimUnit::Percent as u8,
            _pad: [0; 3],
            value: v,
        }
    }

    #[inline]
    #[must_use]
    pub const fn fr(v: f32) -> Self {
        Self {
            unit: DimUnit::Fr as u8,
            _pad: [0; 3],
            value: v,
        }
    }

    #[inline]
    #[must_use]
    pub const fn kind(self) -> DimUnit {
        DimUnit::from_u8(self.unit)
    }

    /// 若可解析，则基于确定的容器尺寸进行解析。
    #[must_use]
    pub fn resolve(self, basis: Option<f32>) -> Option<f32> {
        match self.kind() {
            DimUnit::Px => Some(self.value),
            DimUnit::Percent => basis.map(|b| b * self.value / 100.0),
            DimUnit::Fr => match basis {
                Some(b) if self.value > 0.0 => Some(b / self.value),
                _ => None,
            },
            DimUnit::Auto => None,
        }
    }
}

/// 左/上/右/下四元组（内边距、外边距、边框、内嵌偏移）。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Edges {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Edges {
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    #[inline]
    #[must_use]
    pub const fn all(v: f32) -> Self {
        Self {
            left: v,
            top: v,
            right: v,
            bottom: v,
        }
    }

    #[inline]
    #[must_use]
    pub const fn xy(x: f32, y: f32) -> Self {
        Self {
            left: x,
            top: y,
            right: x,
            bottom: y,
        }
    }

    #[inline]
    #[must_use]
    pub const fn trbl(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum NodeTag {
    #[default]
    Container = 0,
    Text = 1,
    Image = 2,
    /// 承载无限画布的 host 视口叶节点。
    Canvas = 3,
    Scroll = 4,
    /// 逃逸所有祖先裁剪并在覆盖层（overlay）绘制（`docs` §6.3）。
    Portal = 5,
    Separator = 6,
}

impl NodeTag {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Text,
            2 => Self::Image,
            3 => Self::Canvas,
            4 => Self::Scroll,
            5 => Self::Portal,
            6 => Self::Separator,
            _ => Self::Container,
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Display {
    #[default]
    Flex = 0,
    Grid = 1,
    Block = 2,
    None = 3,
}

impl Display {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Grid,
            2 => Self::Block,
            3 => Self::None,
            _ => Self::Flex,
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum FlexDirection {
    #[default]
    Row = 0,
    Column = 1,
    RowReverse = 2,
    ColumnReverse = 3,
}

impl FlexDirection {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Column,
            2 => Self::RowReverse,
            3 => Self::ColumnReverse,
            _ => Self::Row,
        }
    }

    #[must_use]
    pub const fn is_row(self) -> bool {
        matches!(self, Self::Row | Self::RowReverse)
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    #[default]
    Stretch = 0,
    FlexStart = 1,
    FlexEnd = 2,
    Center = 3,
    Baseline = 4,
}

impl Align {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::FlexStart,
            2 => Self::FlexEnd,
            3 => Self::Center,
            4 => Self::Baseline,
            _ => Self::Stretch,
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Justify {
    #[default]
    FlexStart = 0,
    FlexEnd = 1,
    Center = 2,
    SpaceBetween = 3,
    SpaceAround = 4,
    SpaceEvenly = 5,
}

impl Justify {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::FlexEnd,
            2 => Self::Center,
            3 => Self::SpaceBetween,
            4 => Self::SpaceAround,
            5 => Self::SpaceEvenly,
            _ => Self::FlexStart,
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Position {
    #[default]
    Relative = 0,
    Absolute = 1,
}

impl Position {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        if v == 1 {
            Self::Absolute
        } else {
            Self::Relative
        }
    }
}

bitflags_like! {
    /// 节点标志位。保留为 `u32` 以维持 POD 特性并降低跨 C-ABI 边界传输开销。
    ///
    pub struct NodeFlags: u32 {
        const CLIP              = 1 << 0;
        const SCROLL_X          = 1 << 1;
        const SCROLL_Y          = 1 << 2;
        const INTERACTIVE       = 1 << 3;
        const FOCUSABLE         = 1 << 4;
        const HIDDEN            = 1 << 5;
        const TEXT_ELLIPSIS     = 1 << 6;
        const WRAP              = 1 << 7;
        const ANIMATED          = 1 << 8;
        /// 永久跳过像素对齐（例如光标覆盖层）。
        const SUBPIXEL          = 1 << 9;
        /// 即使启发式规则未触发，也强制设为 `RelayoutBoundary`。
        const RELAYOUT_BOUNDARY = 1 << 10;
        const MEASURE_TEXT      = 1 << 11;
        const DRAG_HANDLE       = 1 << 12;
        /// 内容由引擎的虚拟化容器提供。
        const VIRTUALIZED       = 1 << 13;
    }
}

/// 单个显式网格轨道：与盒维度采用相同的 `Dim` 编码，因此
/// `px`、`%`、`fr` 和 `auto` 在 flex 与 grid 间行为一致。
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct GridTrack {
    pub unit: u8,
    pub _pad: [u8; 3],
    pub value: f32,
}

impl GridTrack {
    #[must_use]
    pub fn from_dim(dim: Dim) -> Self {
        Self {
            unit: dim.unit,
            _pad: [0; 3],
            value: dim.value,
        }
    }

    #[must_use]
    pub fn unit(&self) -> DimUnit {
        DimUnit::from_u8(self.unit)
    }
}

/// 扁平 POD 节点描述符。
///
/// 字段顺序按访问频率分组：几何字段最前（每次布局都会读取），
/// 其次是绘制字段（仅脏绘制时读取），最后是标识字段。
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct RawNodeDesc {
    // ---- 几何 / 布局（高频热路径） ----
    pub width: Dim,
    pub height: Dim,
    pub min_width: Dim,
    pub min_height: Dim,
    pub max_width: Dim,
    pub max_height: Dim,
    pub padding: Edges,
    pub margin: Edges,
    pub inset: Edges,
    pub gap_row: f32,
    pub gap_column: f32,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Dim,
    pub aspect_ratio: f32,

    // ---- 绘制 ----
    pub fill: u32,
    pub border_color: u32,
    pub text_color: u32,
    pub shadow_color: u32,
    pub radius: [f32; 4],
    pub border_width: f32,
    pub shadow_blur: f32,
    pub shadow_offset: [f32; 2],
    pub opacity: f32,
    pub transform: [f32; 6],
    pub font_size: f32,
    pub line_height: f32,
    pub z_index: i32,

    // ---- 标识 / 行为 ----
    pub tag: u8,
    pub display: u8,
    pub direction: u8,
    pub align_items: u8,
    pub align_self: u8,
    pub justify: u8,
    pub position: u8,
    pub _pad0: u8,
    pub flags: u32,
    /// 跨重建的稳定键——用于 diff 与记忆化（memoization）的句柄。
    pub key: u64,
    /// 不透明的按 tag 负载（虚拟列表项索引、图集槽位等）。
    pub payload: u64,

    // ---- 网格 ----
    /// 显式列轨道尺寸（仅前 `grid_col_count` 项生效）。
    pub grid_cols: [Dim; 8],
    /// 显式行轨道尺寸（仅前 `grid_row_count` 项生效）。
    pub grid_rows: [Dim; 8],
    pub grid_col_count: u8,
    pub grid_row_count: u8,
    pub grid_col_span: u8,
    pub grid_row_span: u8,
    /// 从 1 开始的网格线索引，`0` 表示自动放置。
    pub grid_col_start: i16,
    /// 从 1 开始的网格线索引，`0` 表示自动放置。
    pub grid_row_start: i16,
}

impl Default for RawNodeDesc {
    fn default() -> Self {
        Self {
            width: Dim::AUTO,
            height: Dim::AUTO,
            min_width: Dim::AUTO,
            min_height: Dim::AUTO,
            max_width: Dim::AUTO,
            max_height: Dim::AUTO,
            padding: Edges::ZERO,
            margin: Edges::ZERO,
            inset: Edges::ZERO,
            gap_row: 0.0,
            gap_column: 0.0,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Dim::AUTO,
            aspect_ratio: 0.0,
            fill: 0,
            border_color: 0,
            text_color: 0,
            shadow_color: 0,
            radius: [0.0; 4],
            border_width: 0.0,
            shadow_blur: 0.0,
            shadow_offset: [0.0; 2],
            opacity: 1.0,
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            font_size: 14.0,
            line_height: 0.0,
            z_index: 0,
            tag: NodeTag::Container as u8,
            display: Display::Flex as u8,
            direction: FlexDirection::Column as u8,
            align_items: Align::Stretch as u8,
            align_self: Align::Stretch as u8,
            justify: Justify::FlexStart as u8,
            position: Position::Relative as u8,
            _pad0: 0,
            flags: 0,
            key: 0,
            payload: 0,
            grid_cols: [Dim::AUTO; 8],
            grid_rows: [Dim::AUTO; 8],
            grid_col_count: 0,
            grid_row_count: 0,
            grid_col_start: 0,
            grid_col_span: 0,
            grid_row_start: 0,
            grid_row_span: 0,
        }
    }
}

impl RawNodeDesc {
    /// 无装饰的纵向流式容器。
    #[must_use]
    pub fn container() -> Self {
        Self::default()
    }

    /// 文本节点（内容通过 host 上下文单独附加）。
    #[must_use]
    pub fn text() -> Self {
        Self::default().tag(NodeTag::Text)
    }

    /// 逻辑像素表示的固定尺寸。
    #[must_use]
    pub fn size(mut self, w: f32, h: f32) -> Self {
        self.width = Dim::px(w);
        self.height = Dim::px(h);
        self
    }

    #[must_use]
    pub fn width(mut self, d: Dim) -> Self {
        self.width = d;
        self
    }

    #[must_use]
    pub fn height(mut self, d: Dim) -> Self {
        self.height = d;
        self
    }

    /// 声明最多 8 条显式列轨道。
    ///
    /// 超出的轨道会被丢弃：描述符为固定大小的 POD 结构体。
    #[must_use]
    pub fn grid_columns(mut self, tracks: &[Dim]) -> Self {
        let count = tracks.len().min(self.grid_cols.len());
        self.grid_cols[..count].copy_from_slice(&tracks[..count]);
        self.grid_col_count = u8::try_from(count).unwrap_or(u8::MAX);
        self
    }

    /// 声明最多 8 条显式行轨道。
    #[must_use]
    pub fn grid_rows(mut self, tracks: &[Dim]) -> Self {
        let count = tracks.len().min(self.grid_rows.len());
        self.grid_rows[..count].copy_from_slice(&tracks[..count]);
        self.grid_row_count = u8::try_from(count).unwrap_or(u8::MAX);
        self
    }

    /// 将此项放置在从 1 开始的网格线与跨度上（`0`/`0` = 自动）。
    #[must_use]
    pub fn grid_area(mut self, col: i16, col_span: u8, row: i16, row_span: u8) -> Self {
        self.grid_col_start = col;
        self.grid_col_span = col_span;
        self.grid_row_start = row;
        self.grid_row_span = row_span;
        self
    }

    #[must_use]
    pub fn fill_token(mut self, token: u32) -> Self {
        self.fill = token;
        self
    }

    #[must_use]
    pub fn text_token(mut self, token: u32) -> Self {
        self.text_color = token;
        self
    }

    #[must_use]
    pub fn border(mut self, color: u32, width: f32) -> Self {
        self.border_color = color;
        self.border_width = width;
        self
    }

    #[must_use]
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = [r; 4];
        self
    }

    #[must_use]
    pub fn padding(mut self, p: Edges) -> Self {
        self.padding = p;
        self
    }

    #[must_use]
    pub fn margin(mut self, m: Edges) -> Self {
        self.margin = m;
        self
    }

    #[must_use]
    pub fn gap(mut self, g: f32) -> Self {
        self.gap_row = g;
        self.gap_column = g;
        self
    }

    #[must_use]
    pub fn row(mut self) -> Self {
        self.direction = FlexDirection::Row as u8;
        self
    }

    #[must_use]
    pub fn column(mut self) -> Self {
        self.direction = FlexDirection::Column as u8;
        self
    }

    #[must_use]
    pub fn tag(mut self, tag: NodeTag) -> Self {
        self.tag = tag as u8;
        self
    }

    #[must_use]
    pub fn display(mut self, d: Display) -> Self {
        self.display = d as u8;
        self
    }

    #[must_use]
    pub fn align(mut self, a: Align) -> Self {
        self.align_items = a as u8;
        self
    }

    #[must_use]
    pub fn justify(mut self, j: Justify) -> Self {
        self.justify = j as u8;
        self
    }

    #[must_use]
    pub fn absolute(mut self) -> Self {
        self.position = Position::Absolute as u8;
        self
    }

    #[must_use]
    pub fn key(mut self, key: u64) -> Self {
        self.key = key;
        self
    }

    #[must_use]
    pub fn payload(mut self, payload: u64) -> Self {
        self.payload = payload;
        self
    }

    #[must_use]
    pub fn flags(mut self, flags: NodeFlags) -> Self {
        self.flags |= flags.bits();
        self
    }

    #[must_use]
    pub fn opacity(mut self, o: f32) -> Self {
        self.opacity = o;
        self
    }

    #[must_use]
    pub fn shadow(mut self, color: u32, blur: f32, dy: f32) -> Self {
        self.shadow_color = color;
        self.shadow_blur = blur;
        self.shadow_offset = [0.0, dy];
        self
    }

    #[must_use]
    pub fn transform(mut self, m: [f32; 6]) -> Self {
        self.transform = m;
        self
    }

    #[inline]
    #[must_use]
    pub const fn kind(&self) -> NodeTag {
        NodeTag::from_u8(self.tag)
    }

    #[inline]
    #[must_use]
    pub const fn display_kind(&self) -> Display {
        Display::from_u8(self.display)
    }

    #[inline]
    #[must_use]
    pub const fn direction_kind(&self) -> FlexDirection {
        FlexDirection::from_u8(self.direction)
    }

    #[inline]
    #[must_use]
    pub const fn justify_kind(&self) -> Justify {
        Justify::from_u8(self.justify)
    }

    #[inline]
    #[must_use]
    pub const fn align_items_kind(&self) -> Align {
        Align::from_u8(self.align_items)
    }

    #[inline]
    #[must_use]
    pub const fn align_self_kind(&self) -> Align {
        Align::from_u8(self.align_self)
    }

    #[inline]
    #[must_use]
    pub const fn position_kind(&self) -> Position {
        Position::from_u8(self.position)
    }

    #[inline]
    #[must_use]
    pub fn has(&self, flag: NodeFlags) -> bool {
        self.flags & flag.bits() != 0
    }

    #[inline]
    #[must_use]
    pub fn is_clipping(&self) -> bool {
        self.has(NodeFlags::CLIP)
    }

    #[inline]
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        self.has(NodeFlags::INTERACTIVE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_is_pod_and_8_byte_aligned() {
        let size = core::mem::size_of::<RawNodeDesc>();
        assert_eq!(size % 8, 0, "size {size} must stay 8-byte aligned");
        assert!(
            size <= 512,
            "descriptor grew to {size} bytes; grid tracks live inline"
        );
    }

    #[test]
    fn unknown_enum_bytes_never_break_the_host() {
        assert_eq!(NodeTag::from_u8(200), NodeTag::Container);
        assert_eq!(Display::from_u8(9), Display::Flex);
        assert_eq!(Align::from_u8(77), Align::Stretch);
        assert_eq!(Justify::from_u8(42), Justify::FlexStart);
        assert_eq!(DimUnit::from_u8(9), DimUnit::Auto);
        assert_eq!(Position::from_u8(3), Position::Relative);
        assert_eq!(FlexDirection::from_u8(9), FlexDirection::Row);
    }

    #[test]
    fn dim_resolution() {
        assert_eq!(Dim::px(12.0).resolve(Some(100.0)), Some(12.0));
        assert_eq!(Dim::percent(50.0).resolve(Some(200.0)), Some(100.0));
        assert_eq!(Dim::percent(50.0).resolve(None), None);
        assert_eq!(Dim::AUTO.resolve(Some(10.0)), None);
        assert_eq!(Dim::fr(2.0).resolve(Some(100.0)), Some(50.0));
        assert_eq!(Dim::px(1.0).kind(), DimUnit::Px);
    }

    #[test]
    fn flags_round_trip_through_bits() {
        let f = NodeFlags::CLIP | NodeFlags::SCROLL_Y;
        assert!(f.contains(NodeFlags::CLIP));
        assert!(!f.contains(NodeFlags::SCROLL_X));
        assert_eq!(NodeFlags::from_bits_truncate(f.bits()), f);
        let mut g = NodeFlags::EMPTY;
        g.insert(NodeFlags::INTERACTIVE);
        assert!(g.intersects(NodeFlags::INTERACTIVE));
        g.remove(NodeFlags::INTERACTIVE);
        assert!(g.is_empty());
    }

    #[test]
    fn builder_sets_typed_fields() {
        let d = RawNodeDesc::container()
            .size(120.0, 40.0)
            .row()
            .gap(8.0)
            .radius(6.0)
            .tag(NodeTag::Text)
            .flags(NodeFlags::CLIP | NodeFlags::INTERACTIVE);
        assert_eq!(d.kind(), NodeTag::Text);
        assert!(d.direction_kind().is_row());
        assert!(d.is_clipping());
        assert!(d.is_interactive());
        assert_eq!(d.width.resolve(None), Some(120.0));
        assert_eq!(d.radius, [6.0; 4]);
    }

    #[test]
    fn default_transform_is_identity_and_opaque() {
        let d = RawNodeDesc::default();
        assert_eq!(d.transform, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(d.opacity, 1.0);
        assert_eq!(d.position_kind(), Position::Relative);
        assert!(!d.is_clipping());
    }
}
