//! 下拉菜单与上下文菜单（Menu）。
//!
//! 菜单即数据：包含菜单项、禁用状态、危险操作标记与快捷键。状态机（[`MenuState`]）仅持有当前高亮索引；
//! `Up`/`Down` 会跳过分隔符与禁用项，`Enter` 上报选中的菜单项。
//! 发射器绘制背景阴影、表面、每个菜单项的高亮胶囊、图标/标签/快捷键或子菜单箭头以及每条分隔符横线——
//! 同样为固定描述符保持恒定的图元数量。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, PORTAL_LAYER};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::image::IconDesc;
use crate::overlay::OverlayEntry;
use crate::primitive::{glyphs, instance, surface};
use crate::scroll::{
    emit_scrollbar, ScrollBarDesc, ScrollPointerEvent, ScrollState, ScrollViewDesc,
};
use crate::shell::{
    emit_shell_icon_primitives, emit_widget_icon_primitives, ShellIcon, WidgetIcon,
};
use crate::{ControlState, PointerPhase, WidgetDesc, WidgetOutput};

/// 菜单的命中测试层级。
pub const MENU_LAYER: u8 = PORTAL_LAYER;
/// 默认行高。
///
/// WinUI `MenuFlyoutItemThemePadding = 11,8,11,9` 叠加 20px 行高即 37px，
/// 与下拉框选中项行高 [`crate::select::SELECT_ROW_HEIGHT`] 保持一致。
pub const MENU_ITEM_HEIGHT: f32 = 37.0;
/// Win11 上下文菜单默认项高（`32px`）。
pub const CONTEXT_MENU_ITEM_HEIGHT: f32 = 32.0;
/// 菜单面板四边内边距（`4px`）。
pub const MENU_PANEL_PADDING: f32 = 4.0;
/// 菜单项高亮胶囊圆角（`4px`）。
pub const MENU_ITEM_RADIUS: f32 = 4.0;
/// 菜单项前置图标槽尺寸（`16×16px`）。
pub const MENU_ICON_SIZE: f32 = 16.0;
/// 菜单分隔线行高度（`9px`，居中绘制 `1px` `Token::DIVIDER`）。
pub const MENU_SEPARATOR_HEIGHT: f32 = 9.0;
/// 菜单浮层海拔阴影模糊半径。对齐 WinUI 3 `FlyoutElevation` / `MenuFlyout`（18px 解析阴影）。
pub const MENU_SHADOW: f32 = 18.0;
/// 菜单浮层圆角半径。对齐 WinUI `OverlayCornerRadius = 8`（MenuFlyoutPresenter 标准圆角）。
pub const MENU_RADIUS: f32 = 8.0;
/// 菜单浮层外描边厚度。对齐 WinUI `SurfaceStrokeColorFlyout`（1px 微描边）。
pub const MENU_BORDER_WIDTH: f32 = 1.0;
/// Win11 菜单亚克力毛玻璃模糊半径（逻辑像素，对齐 Fluent `AcrylicInAppFillColorDefault`）。
pub const MENU_BACKDROP_BLUR: f32 = 24.0;
/// Win11 菜单亚克力底板半透明叠色不透明度（约 82%，使底层 Dual-Kawase 虚化自然透出）。
pub const MENU_ACRYLIC_SURFACE_OPACITY: f32 = 0.82;
/// 下拉框选中项左侧强调条的宽度。
const MENU_ACCENT_BAR_WIDTH: f32 = 3.0;
/// 子菜单指示箭头尺寸（`12×12px`）。
const MENU_SUBMENU_ARROW_SIZE: f32 = 12.0;

/// 为菜单项生成确定性子热区节点键。
#[must_use]
pub const fn menu_item_node(menu_key: u64, index: usize) -> u64 {
    menu_key ^ 0x4D45_4E55_0000_0000u64 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15u64) ^ 3
}

/// 单个菜单项的预塑形字形与交互状态。
#[derive(Clone, Debug, Default)]
pub struct MenuItemRenderRow<'a> {
    /// 左对齐主标签字形。
    pub label_glyphs: &'a [GlyphQuad],
    /// 右对齐快捷键提示字形。
    pub shortcut_glyphs: &'a [GlyphQuad],
    /// 该菜单项独立的指针交互阶段。
    pub phase: PointerPhase,
}

/// 菜单中的单行条目。
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    /// 可见标签。
    pub label: String,
    /// 绘制在右侧的快捷键提示（若有）。
    pub shortcut: String,
    /// 该行是否不可选择。
    pub disabled: bool,
    /// 该行是分隔符而非操作项。
    pub separator: bool,
    /// 该行是否为「当前值」。
    ///
    /// 与键盘/指针高亮不同：下拉框重新打开时，选中项即使不再被高亮也要保留
    /// 左侧 3px 强调条，表明「当前选的是这一项」。
    pub selected: bool,
    /// 该行是否为危险操作项（使用 `Token::ERROR` 前景色）。
    pub danger: bool,
    /// 前置图标（若有）。
    pub icon: Option<IconDesc>,
    /// 统一前置图标槽（支持内置矢量与外部彩色纹理）。
    pub widget_icon: WidgetIcon,
    /// 可勾选行的勾选状态（`Some` 表示可勾选）。
    pub checked: Option<bool>,
    /// 按 `ArrowRight` 打开的嵌套子菜单（若有）。
    pub submenu: Option<Vec<MenuItem>>,
}

impl MenuItem {
    /// 创建可选择的操作项。
    #[must_use]
    pub fn action(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            shortcut: String::new(),
            disabled: false,
            separator: false,
            selected: false,
            danger: false,
            icon: None,
            widget_icon: WidgetIcon::None,
            checked: None,
            submenu: None,
        }
    }

    /// 创建水平分隔符。
    #[must_use]
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            shortcut: String::new(),
            disabled: true,
            separator: true,
            selected: false,
            danger: false,
            icon: None,
            widget_icon: WidgetIcon::None,
            checked: None,
            submenu: None,
        }
    }

    /// 标记该行为当前值（下拉框选中项）。
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 标记该菜单项为危险操作项（使用 `Token::ERROR` 前景色）。
    #[must_use]
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    /// 附加快捷键提示。
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = shortcut.into();
        self
    }

    /// 附加前置图标。
    #[must_use]
    pub fn with_icon(mut self, icon: IconDesc) -> Self {
        self.icon = Some(icon);
        self
    }

    /// 附加统一前置图标（支持内置矢量或外部彩色纹理）。
    #[must_use]
    pub fn with_widget_icon(mut self, icon: WidgetIcon) -> Self {
        self.widget_icon = icon;
        self
    }

    /// 设为可勾选行（已勾选或未勾选）。
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// 附加嵌套子菜单。
    #[must_use]
    pub fn with_submenu(mut self, items: Vec<MenuItem>) -> Self {
        self.submenu = Some(items);
        self
    }

    /// 将菜单项设为禁用。
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// 判断菜单项是否可被选中。
    #[must_use]
    pub fn selectable(&self) -> bool {
        !self.disabled && !self.separator
    }

    /// 判断菜单项是否会打开嵌套子菜单。
    #[must_use]
    pub fn has_submenu(&self) -> bool {
        self.submenu.as_ref().is_some_and(|items| !items.is_empty())
    }
}

/// 菜单的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct MenuDesc {
    /// 菜单矩形。
    pub rect: Rect,
    /// 从上到下排列的菜单行列表。
    pub items: Vec<MenuItem>,
    /// 行高。
    pub item_height: f32,
    /// 圆角半径。
    pub radius: f32,
    /// 绘制顺序键。
    pub z: u16,
    /// 已知时菜单必须保持在内的视口。
    pub viewport: Option<Rect>,
    /// 面板上下内边距（`MenuDesc::new` 保持 `0.0` 向后兼容，`MenuDesc::context_menu` 默认 `MENU_PANEL_PADDING`）。
    panel_padding: f32,
    /// 是否为分隔线行启用紧凑高度（`MENU_SEPARATOR_HEIGHT`）。
    compact_separators: bool,
    /// 菜单亚克力毛玻璃模糊半径（`MenuDesc::context_menu` 默认启用 [`MENU_BACKDROP_BLUR`]）。
    backdrop_blur: f32,
}

impl MenuDesc {
    #[must_use]
    pub fn new(rect: Rect, items: Vec<MenuItem>) -> Self {
        Self {
            rect,
            items,
            item_height: MENU_ITEM_HEIGHT,
            // `MenuFlyoutPresenter` 的 `CornerRadius = OverlayCornerRadius = 8`。
            radius: MENU_RADIUS,
            z: 905,
            viewport: None,
            panel_padding: 0.0,
            compact_separators: false,
            backdrop_blur: 0.0,
        }
    }

    /// 创建符合 Win11 上下文菜单密度的描述符（无标题头、`item_height = CONTEXT_MENU_ITEM_HEIGHT (32.0)`、
    /// 上下 `MENU_PANEL_PADDING (4.0)` 内边距、`MENU_SEPARATOR_HEIGHT (9.0)` 分隔线与 Win11 亚克力毛玻璃底板）。
    #[must_use]
    pub fn context_menu(rect: Rect, items: Vec<MenuItem>) -> Self {
        Self {
            rect,
            items,
            item_height: CONTEXT_MENU_ITEM_HEIGHT,
            radius: MENU_RADIUS,
            z: 905,
            viewport: None,
            panel_padding: MENU_PANEL_PADDING,
            compact_separators: true,
            backdrop_blur: MENU_BACKDROP_BLUR,
        }
    }

    /// 覆写菜单项行高。
    #[must_use]
    pub fn with_item_height(mut self, height: f32) -> Self {
        self.item_height = height.max(1.0);
        self
    }

    /// 返回菜单亚克力毛玻璃模糊半径（`0.0` 表示不透明常规表面）。
    #[must_use]
    pub const fn backdrop_blur(&self) -> f32 {
        self.backdrop_blur
    }

    /// 覆写菜单亚克力毛玻璃模糊半径（`0.0` 关闭亚克力虚化）。
    #[must_use]
    pub fn with_backdrop_blur(mut self, blur: f32) -> Self {
        self.backdrop_blur = blur.max(0.0);
        self
    }

    /// 构造带 DWM 标准圆角与浮层 Z 切分阈值的亚克力毛玻璃背板请求。
    #[must_use]
    pub fn backdrop_request(&self, node: u64) -> Option<crate::overlay::BackdropRequest> {
        crate::overlay::BackdropRequest::new(node, self.rect, self.backdrop_blur)
            .map(|req| req.with_corner_radius(self.radius).with_z(self.z))
    }

    /// 返回单行的实际高度（分隔线在上下文菜单模式下采用 `MENU_SEPARATOR_HEIGHT`）。
    #[must_use]
    fn row_height(&self, item: &MenuItem) -> f32 {
        if item.separator && self.compact_separators {
            MENU_SEPARATOR_HEIGHT
        } else {
            self.item_height
        }
    }

    /// 判断当前菜单是否需要为左侧图标列保留统一对齐宽度。
    #[must_use]
    pub fn has_icon_column(&self) -> bool {
        self.items.iter().any(|item| {
            !item.separator
                && (item.widget_icon != WidgetIcon::None
                    || item.icon.is_some()
                    || item.checked.is_some())
        })
    }

    /// 所有行所需的总高度（含上下内边距与分隔线行高）。
    #[must_use]
    pub fn content_height(&self) -> f32 {
        let rows_h: f32 = self.items.iter().map(|item| self.row_height(item)).sum();
        self.panel_padding * 2.0 + rows_h
    }

    /// 将菜单高度约束到视口范围内。
    ///
    /// 返回行是否溢出（即是否需要滚动）。
    /// 菜单保持顶边不变并向下收缩，符合锚定在输入框下方的下拉列表行为。
    pub fn clamp_height(&mut self) -> bool {
        let overflow = self.content_height() > self.rect.height() + f32::EPSILON;
        let Some(viewport) = self.viewport else {
            return overflow;
        };
        let available = (viewport.max().y - self.rect.origin.y)
            .max(self.item_height.min(self.content_height()));
        if self.content_height() > available + f32::EPSILON {
            self.rect = Rect::new(
                self.rect.origin.x,
                self.rect.origin.y,
                self.rect.width(),
                available,
            );
            true
        } else {
            false
        }
    }

    /// 沿着从根开始的 `path` 获取嵌套层级的行切片。
    #[must_use]
    pub fn items_at(&self, path: &[usize]) -> Option<&[MenuItem]> {
        let mut level: &[MenuItem] = &self.items;
        for &index in path {
            level = level.get(index)?.submenu.as_deref()?;
        }
        Some(level)
    }

    /// 单行的矩形。
    #[must_use]
    pub fn item_rect(&self, index: usize) -> Rect {
        let mut y = self.rect.origin.y + self.panel_padding;
        for item in self.items.iter().take(index) {
            y += self.row_height(item);
        }
        let h = self
            .items
            .get(index)
            .map_or(self.item_height, |item| self.row_height(item));
        Rect::new(self.rect.origin.x, y, self.rect.width(), h)
    }

    /// 获取指定坐标点下的行（若有）。
    #[must_use]
    pub fn item_at(&self, point: unbound_math::Vec2) -> Option<usize> {
        self.item_at_offset(point, unbound_math::Vec2::ZERO)
    }

    /// 考虑滚动内容偏移量后，获取指定坐标点下的行。
    #[must_use]
    pub fn item_at_offset(
        &self,
        point: unbound_math::Vec2,
        offset: unbound_math::Vec2,
    ) -> Option<usize> {
        if !self.rect.contains_point(point) {
            return None;
        }
        let local_y = point.y - self.rect.origin.y + offset.y - self.panel_padding;
        if local_y < 0.0 {
            return None;
        }
        let mut cursor_y = 0.0;
        for (index, item) in self.items.iter().enumerate() {
            let h = self.row_height(item);
            if local_y < cursor_y + h {
                return Some(index);
            }
            cursor_y += h;
        }
        None
    }

    /// 为持有 `Semantics` 的调用方构造单行的无障碍节点。
    #[must_use]
    pub fn item_a11y(&self, index: usize) -> Option<A11yNode> {
        let item = self.items.get(index)?;
        if item.separator {
            return Some(A11yNode::new(Role::Unknown, "", self.item_rect(index)));
        }
        Some(
            A11yNode::new(Role::MenuItem, item.label.clone(), self.item_rect(index))
                .clickable()
                .disabled(item.disabled),
        )
    }
}

/// 菜单的高亮行状态。
#[derive(Clone, Debug, Default)]
pub struct MenuState {
    /// 当前高亮索引。
    pub highlighted: Option<usize>,
    /// 已展开的子菜单索引路径，最外层在前。
    pub open_path: Vec<usize>,
    /// 当菜单高于视口时的滚动状态。
    ///
    /// 行未溢出时为 `None`。由 [`Self::ensure_scroll`] 安装；滚动偏移量刻意不参与 `PartialEq`，
    /// 因为它是实时指针状态，而非菜单选区的一部分。
    pub scroll: Option<ScrollState>,
    /// 指针是否悬停在按需叠加滚动条热区上（不参与 `PartialEq`）。
    pub scrollbar_hovered: bool,
    /// 滚动条滑块拖拽起始快照 `(start_pointer_y, start_offset_y)`（不参与 `PartialEq`）。
    pub scrollbar_drag: Option<(f32, f32)>,
}

impl PartialEq for MenuState {
    fn eq(&self, other: &Self) -> bool {
        self.highlighted == other.highlighted && self.open_path == other.open_path
    }
}

impl Eq for MenuState {}

impl MenuState {
    /// 创建无高亮行的菜单状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 获取当前打开层级的行切片。
    #[must_use]
    pub fn open_items<'a>(&self, desc: &'a MenuDesc) -> &'a [MenuItem] {
        desc.items_at(&self.open_path).unwrap_or(&desc.items)
    }

    /// 移动高亮，自动跳过分隔符与禁用行。
    ///
    /// 返回新的高亮索引。
    pub fn move_highlight(&mut self, desc: &MenuDesc, forward: bool) -> Option<usize> {
        let items = self.open_items(desc);
        if items.is_empty() {
            return None;
        }
        let count = items.len();
        let start = self
            .highlighted
            .unwrap_or(if forward { count - 1 } else { 0 });
        for step in 1..=count {
            let index = if forward {
                (start + step) % count
            } else {
                (start + count - step) % count
            };
            if items[index].selectable() {
                self.highlighted = Some(index);
                return Some(index);
            }
        }
        None
    }

    /// 高亮指针下方的行，并在需要时展开子菜单。
    pub fn hover(&mut self, desc: &MenuDesc, point: unbound_math::Vec2) -> Option<usize> {
        let index = desc.item_at_offset(point, self.scroll_offset())?;
        let item = desc.items.get(index)?;
        if !item.selectable() {
            return self.highlighted;
        }
        if item.has_submenu() {
            self.open_path = vec![index];
            self.highlighted = None;
            return Some(index);
        } else if self.open_path.is_empty() {
            self.highlighted = Some(index);
        } else {
            // 在根层级悬停叶子项时会关闭已打开的子菜单。
            self.open_path.clear();
            self.highlighted = Some(index);
        }
        self.highlighted
    }

    /// `ArrowRight`：展开高亮行的子菜单。
    ///
    /// 返回是否打开了子菜单。
    pub fn expand(&mut self, desc: &MenuDesc) -> bool {
        let Some(index) = self.highlighted else {
            return false;
        };
        let opens = self
            .open_items(desc)
            .get(index)
            .is_some_and(MenuItem::has_submenu);
        if opens {
            self.open_path.push(index);
            self.highlighted = None;
        }
        opens
    }

    /// `ArrowLeft`：折叠最内层已打开的子菜单。
    ///
    /// 返回是否关闭了子菜单。
    pub fn collapse(&mut self) -> bool {
        let Some(parent) = self.open_path.pop() else {
            return false;
        };
        self.highlighted = Some(parent);
        true
    }

    /// 高亮下一个标签以 `ch` 开头的行。
    ///
    /// 搜索大小写不敏感，且从当前高亮之后开始，因此重复输入同一字母会在匹配项间循环。
    /// 返回新的高亮索引。
    pub fn typeahead(&mut self, desc: &MenuDesc, ch: char) -> Option<usize> {
        let items = self.open_items(desc);
        if items.is_empty() || ch.is_control() {
            return self.highlighted;
        }
        let needle = ch.to_lowercase().next()?;
        let start = self.highlighted.map_or(0, |index| index + 1);
        for step in 0..items.len() {
            let index = (start + step) % items.len();
            let item = &items[index];
            if item.selectable()
                && item
                    .label
                    .chars()
                    .next()
                    .is_some_and(|first| first.to_lowercase().next() == Some(needle))
            {
                self.highlighted = Some(index);
                return Some(index);
            }
        }
        self.highlighted
    }

    /// 提交当前高亮行。
    pub fn activate(&self, desc: &MenuDesc) -> Option<usize> {
        let index = self.highlighted?;
        self.open_items(desc)
            .get(index)
            .filter(|item| item.selectable())
            .map(|_| index)
    }

    /// 安装（或移除）内嵌滚动状态。
    ///
    /// 返回行是否超出裁剪后的菜单高度。
    pub fn ensure_scroll(&mut self, desc: &MenuDesc) -> bool {
        let overflow = desc.content_height() > desc.rect.height() + f32::EPSILON;
        if overflow {
            self.scroll.get_or_insert_with(ScrollState::new);
        } else {
            self.scroll = None;
        }
        overflow
    }

    /// 当前滚动偏移量。
    #[must_use]
    pub fn scroll_offset(&self) -> unbound_math::Vec2 {
        self.scroll
            .as_ref()
            .map_or(unbound_math::Vec2::ZERO, ScrollState::offset)
    }

    /// 按 `delta` 滚动打开的菜单，并限制在内容范围内。
    pub fn scroll_by(&mut self, desc: &MenuDesc, delta: unbound_math::Vec2) {
        self.ensure_scroll(desc);
        let view = ScrollViewDesc::new(
            desc.rect,
            unbound_math::Vec2::new(desc.rect.width(), desc.content_height()),
        );
        if let Some(scroll) = self.scroll.as_mut() {
            scroll.scroll_by(&view, delta);
            scroll.jump_to(&view, scroll.target());
        }
    }

    /// 更新垂直叠加滚动条热区悬停状态。
    pub fn set_scrollbar_hovered(&mut self, hovered: bool) {
        self.scrollbar_hovered = hovered;
    }

    /// 当前是否正在拖拽垂直叠加滚动条滑块。
    #[must_use]
    pub fn is_dragging_scrollbar(&self) -> bool {
        self.scrollbar_drag.is_some()
    }

    /// 构造菜单面板右缘对应的垂直按需叠加滚动条描述符。
    #[must_use]
    pub fn scrollbar_desc(&self, desc: &MenuDesc) -> ScrollBarDesc {
        let mut bar =
            ScrollBarDesc::vertical(desc.rect, desc.content_height(), self.scroll_offset().y);
        bar.z = desc.z.saturating_add(2);
        bar.layer = MENU_LAYER;
        bar.hovered = self.scrollbar_hovered;
        bar.dragging = self.scrollbar_drag.is_some();
        bar
    }

    /// 归约作用于溢出菜单垂直叠加滚动条的指针手势（滑块拖拽与点击轨道翻页）。
    pub fn reduce_scrollbar_pointer(&mut self, desc: &MenuDesc, event: ScrollPointerEvent) -> bool {
        let overflow = desc.content_height() > desc.rect.height() + f32::EPSILON;
        if !overflow {
            self.scrollbar_drag = None;
            return false;
        }
        self.ensure_scroll(desc);
        let view = ScrollViewDesc::new(
            desc.rect,
            unbound_math::Vec2::new(desc.rect.width(), desc.content_height()),
        );
        match event {
            ScrollPointerEvent::Press(pos) => {
                let bar = self.scrollbar_desc(desc);
                if bar.thumb_rect().contains_point(pos) {
                    self.scrollbar_drag = Some((pos.y, self.scroll_offset().y));
                    if let Some(scroll) = self.scroll.as_mut() {
                        scroll.reveal_scrollbar();
                    }
                    return true;
                }
                if let Some(next_y) = bar.page_offset_for_track_click(pos) {
                    if let Some(scroll) = self.scroll.as_mut() {
                        scroll.jump_to(&view, unbound_math::Vec2::new(0.0, next_y));
                    }
                    return true;
                }
                false
            }
            ScrollPointerEvent::Drag(pos) => {
                let Some((start_pointer, start_offset)) = self.scrollbar_drag else {
                    return false;
                };
                let bar = self.scrollbar_desc(desc);
                let next_y = bar.offset_for_drag(start_offset, start_pointer, pos.y);
                if let Some(scroll) = self.scroll.as_mut() {
                    scroll.jump_to(&view, unbound_math::Vec2::new(0.0, next_y));
                }
                true
            }
            ScrollPointerEvent::Release => self.scrollbar_drag.take().is_some(),
        }
    }
}

/// 绑定了身份标识的菜单描述符。
#[derive(Clone, Debug)]
pub struct MenuWidget {
    /// 菜单节点。
    pub node: u64,
    /// 触发器节点。
    pub anchor_node: u64,
    /// 菜单描述符。
    pub desc: MenuDesc,
    /// 当前高亮行。
    pub state: MenuState,
}

impl WidgetDesc for MenuWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_menu(self.node, self.anchor_node, &self.desc, &self.state, state).0
    }
}

/// 发射菜单、其命中区域与浮层栈条目（兼容入口，委托至 [`emit_menu_with_rows`]）。
#[must_use]
pub fn emit_menu(
    node: u64,
    anchor_node: u64,
    desc: &MenuDesc,
    state: &MenuState,
    control: ControlState,
) -> (WidgetOutput, OverlayEntry) {
    emit_menu_with_rows(node, anchor_node, desc, state, control, &[])
}

/// 发射完整的 Win11 `MenuFlyout` / `ContextMenu`（含图标、标签、快捷键、子菜单箭头与独立子热区）。
#[must_use]
pub fn emit_menu_with_rows(
    node: u64,
    anchor_node: u64,
    desc: &MenuDesc,
    state: &MenuState,
    control: ControlState,
    rows: &[MenuItemRenderRow<'_>],
) -> (WidgetOutput, OverlayEntry) {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    let offset = state.scroll_offset();
    let mut primitives: Vec<GpuPrimitiveInstance> = Vec::with_capacity(3 + desc.items.len() * 3);
    let mut payloads = Vec::new();
    let mut out = WidgetOutput::default();

    primitives.push(instance(
        rect,
        radii,
        Token::NONE,
        Token::NONE,
        0.0,
        MENU_SHADOW,
        PrimType::Shadow,
        SubElement::Shadow,
        desc.z,
    ));
    let mut panel_bg = surface(
        rect,
        desc.radius,
        if control.disabled {
            Token::SURFACE_SUNKEN
        } else {
            Token::OVERLAY_BG
        },
        desc.z,
    );
    if desc.backdrop_blur > 0.0 && !control.disabled {
        // 启用 Win11 亚克力底板时降低菜单表面不透明度，让底层模糊纹理透过半透明叠色显现。
        panel_bg.z_and_opacity =
            GpuPrimitiveInstance::pack_z_opacity(desc.z, MENU_ACRYLIC_SURFACE_OPACITY);
    }
    primitives.push(panel_bg);

    // 菜单行元素统一抬升至 `row_z = desc.z + 1`，与菜单底板严格分层：
    // 面板阴影/底板(desc.z) < 行胶囊(row_z, Background) < 图标/字形/分隔线(row_z, Content) < 强调条/外框(row_z, Border)。
    let row_z = desc.z.saturating_add(1);

    for (index, item) in desc.items.iter().enumerate() {
        let row = desc
            .item_rect(index)
            .translate(unbound_math::Vec2::new(0.0, -offset.y));
        if item.separator {
            let inset_x = MENU_PANEL_PADDING + 8.0;
            let line = Rect::new(
                row.origin.x + inset_x,
                row.center().y - 0.5,
                (row.width() - inset_x * 2.0).max(0.0),
                1.0,
            );
            primitives.push(instance(
                line,
                [0.0; 4],
                Token::DIVIDER,
                Token::NONE,
                0.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Content,
                row_z,
            ));
            // 保持行数恒定：为高亮保留一个不可见槽位。
            let mut placeholder = surface(Rect::ZERO, 0.0, Token::NONE, row_z);
            placeholder.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(row_z, 0.0);
            primitives.push(placeholder);
            continue;
        }

        let render_row = rows.get(index);
        let row_phase = render_row.map_or(PointerPhase::Idle, |r| r.phase);
        let item_disabled = control.disabled || item.disabled;
        let selectable = !item_disabled && item.selectable();

        let active_row = if state.open_path.is_empty() {
            state.highlighted == Some(index)
        } else {
            state.open_path.first() == Some(&index)
        };
        let (pill_fill, pill_visible) = if !selectable {
            (Token::NONE, false)
        } else if row_phase == PointerPhase::Pressed {
            (Token::PRESSED, true)
        } else if row_phase == PointerPhase::Hovered || active_row {
            if item.danger {
                (Token::ERROR_SOFT, true)
            } else {
                (Token::HOVER, true)
            }
        } else {
            (Token::NONE, false)
        };

        // `MenuFlyoutItemMargin = 4,2,4,2`：高亮胶囊与菜单边缘留 4 / 2。
        let pill = Rect::new(
            row.origin.x + MENU_PANEL_PADDING,
            row.origin.y + 2.0,
            (row.size.x - MENU_PANEL_PADDING * 2.0).max(0.0),
            (row.size.y - 4.0).max(0.0),
        );
        let mut slot = surface(pill, MENU_ITEM_RADIUS, pill_fill, row_z);
        if !pill_visible {
            slot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(row_z, 0.0);
        }
        primitives.push(slot);

        let fg_color = if item_disabled {
            Token::TEXT_DISABLED
        } else if item.danger {
            Token::ERROR
        } else {
            Token::TEXT_PRIMARY
        };
        let row_bg = if pill_visible {
            pill_fill
        } else {
            Token::OVERLAY_BG
        };

        let label_glyphs = render_row.map_or(&[][..], |r| r.label_glyphs);
        let shortcut_glyphs = render_row.map_or(&[][..], |r| r.shortcut_glyphs);

        if label_glyphs.is_empty() {
            // 兼容无字形调用方（如旧版 `emit_menu`）：保留固定内容槽占位图元。
            primitives.push(instance(
                row,
                [0.0; 4],
                Token::NONE,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Content,
                row_z,
            ));
        } else {
            glyphs(
                label_glyphs,
                fg_color,
                row_z,
                &mut primitives,
                &mut payloads,
            );
        }

        // 统一前置图标（`16×16`，位于胶囊左侧内缩 10px 处垂直居中）；
        // 若未设自定义图标且已勾选，则绘制 `ShellIcon::Check`。
        let effective_icon = if item.widget_icon != WidgetIcon::None {
            item.widget_icon
        } else if item.checked == Some(true) {
            WidgetIcon::Vector(ShellIcon::Check)
        } else {
            WidgetIcon::None
        };
        if effective_icon != WidgetIcon::None {
            let icon_rect = Rect::new(
                pill.origin.x + 10.0,
                row.origin.y + (row.size.y - MENU_ICON_SIZE) * 0.5,
                MENU_ICON_SIZE,
                MENU_ICON_SIZE,
            );
            emit_widget_icon_primitives(
                effective_icon,
                icon_rect,
                fg_color,
                row_bg,
                item_disabled,
                row_z,
                &mut primitives,
                &mut payloads,
            );
        }

        // 右侧列：有子菜单时绘制 `›` 箭头，否则发射右对齐次级色快捷键字形。
        if item.has_submenu() {
            let chevron_rect = Rect::new(
                pill.max().x - 10.0 - MENU_SUBMENU_ARROW_SIZE,
                row.origin.y + (row.size.y - MENU_SUBMENU_ARROW_SIZE) * 0.5,
                MENU_SUBMENU_ARROW_SIZE,
                MENU_SUBMENU_ARROW_SIZE,
            );
            let arrow_color = if item_disabled {
                Token::TEXT_DISABLED
            } else {
                Token::SUBMENU_ARROW
            };
            emit_shell_icon_primitives(
                ShellIcon::ChevronRight,
                chevron_rect,
                arrow_color,
                row_bg,
                row_z,
                &mut primitives,
                &mut payloads,
            );
        } else if !shortcut_glyphs.is_empty() {
            let shortcut_color = if item_disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };
            glyphs(
                shortcut_glyphs,
                shortcut_color,
                row_z,
                &mut primitives,
                &mut payloads,
            );
        }

        // 「当前值」标记：与高亮分开，键盘移到别的行时仍指示已选中的项。
        if item.selected {
            let bar_height = (row.size.y * 0.52).clamp(10.0, 16.0);
            primitives.push(instance(
                Rect::new(
                    pill.origin.x + 2.0,
                    row.origin.y + (row.size.y - bar_height) * 0.5,
                    MENU_ACCENT_BAR_WIDTH,
                    bar_height,
                ),
                [MENU_ACCENT_BAR_WIDTH * 0.5; 4],
                Token::PRIMARY,
                Token::NONE,
                0.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Border,
                row_z,
            ));
        }

        // 为每个可用非分隔线项注册独立子热区与无障碍节点。
        if selectable {
            let item_key = menu_item_node(node, index);
            let item_hit = HitRegion::new(
                item_key,
                MENU_LAYER,
                u32::from(desc.z).saturating_add(1),
                pill,
            )
            .with_cursor(CursorKind::Pointer);
            let item_a11y = A11yNode::new(Role::MenuItem, item.label.clone(), pill)
                .clickable()
                .disabled(false);
            out.with_sub_control(item_key, item_hit, item_a11y, true);
        }
    }

    primitives.push(instance(
        rect,
        radii,
        Token::NONE,
        // `MenuFlyoutPresenterBorderBrush` = SurfaceStrokeColorFlyout。
        Token::CARD_BORDER,
        MENU_BORDER_WIDTH,
        0.0,
        PrimType::RoundedRect,
        SubElement::Border,
        row_z,
    ));

    // 仅当菜单内容高度超出视口可用高度时发射按需叠加滚动条；常规短菜单保持 0 滚动条图元。
    let overflow = desc.content_height() > desc.rect.height() + f32::EPSILON;
    if overflow {
        let mut bar = state.scrollbar_desc(desc);
        bar.hovered = bar.hovered
            || (!control.disabled
                && matches!(control.phase, PointerPhase::Hovered | PointerPhase::Pressed));
        bar.disabled = control.disabled;
        let bar_out = emit_scrollbar(node, &bar);
        primitives.extend(bar_out.output.primitives);
        if !control.disabled {
            for hit in bar_out.hits {
                out.sub_hits.push(hit);
            }
        }
    }

    out.primitives = primitives;
    out.payloads = payloads;
    out.hit = Some(HitRegion::new(node, MENU_LAYER, u32::from(desc.z), rect));
    out.a11y = Some(A11yNode::new(Role::Menu, "", rect).disabled(control.disabled));
    out.focusable = false;

    (out, OverlayEntry::menu(node, anchor_node, rect))
}

/// 菜单中的分隔符行。
#[must_use]
pub fn separator_indices(desc: &MenuDesc) -> Vec<usize> {
    desc.items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.separator.then_some(index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> MenuDesc {
        MenuDesc::new(
            Rect::new(100.0, 100.0, 220.0, 150.0),
            vec![
                MenuItem::action("Open").with_shortcut("Ctrl+O"),
                MenuItem::separator(),
                MenuItem::action("Disabled").disabled(),
                MenuItem::action("Quit"),
            ],
        )
    }

    fn dummy_quad(x: f32, y: f32) -> GlyphQuad {
        GlyphQuad {
            rect: Rect::new(x, y, 8.0, 12.0),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        }
    }

    #[test]
    fn arrow_navigation_skips_separators_and_disabled_rows() {
        let desc = desc();
        let mut state = MenuState::new();
        assert_eq!(state.move_highlight(&desc, true), Some(0));
        assert_eq!(state.move_highlight(&desc, true), Some(3));
        assert_eq!(state.move_highlight(&desc, true), Some(0));
        assert_eq!(state.move_highlight(&desc, false), Some(3));
    }

    #[test]
    fn enter_returns_the_highlighted_action_only() {
        let desc = desc();
        let mut state = MenuState::new();
        assert_eq!(state.activate(&desc), None);
        state.highlighted = Some(2);
        assert_eq!(
            state.activate(&desc),
            None,
            "a disabled row cannot activate"
        );
        state.highlighted = Some(3);
        assert_eq!(state.activate(&desc), Some(3));
    }

    #[test]
    fn hovering_a_disabled_row_does_not_highlight_it() {
        let desc = desc();
        let mut state = MenuState::new();
        assert_eq!(
            state.hover(&desc, unbound_math::Vec2::new(120.0, 175.0)),
            None
        );
        assert_eq!(
            state.hover(&desc, unbound_math::Vec2::new(120.0, 115.0)),
            Some(0)
        );
    }

    #[test]
    fn the_primitive_count_is_constant_for_a_fixed_menu() {
        let desc = desc();
        let state = MenuState::new();
        let control = ControlState::default();
        let a = emit_menu(1, 2, &desc, &state, control).0.primitives.len();
        let mut highlighted = MenuState::new();
        highlighted.highlighted = Some(0);
        let b = emit_menu(1, 2, &desc, &highlighted, control)
            .0
            .primitives
            .len();
        assert_eq!(a, b);
    }

    #[test]
    fn a_menu_exposes_menu_item_semantics() {
        let desc = desc();
        let item = desc.item_a11y(0).unwrap();
        assert_eq!(item.role, Role::MenuItem);
        assert_eq!(item.label, "Open");
        assert!(desc.item_a11y(1).unwrap().label.is_empty());
        assert_eq!(separator_indices(&desc), vec![1]);
    }

    #[test]
    fn menu_item_widget_icon_emits_external_texture_and_preserves_constant_count() {
        let desc = MenuDesc::new(
            Rect::new(100.0, 100.0, 220.0, 120.0),
            vec![
                MenuItem::action("Open").with_widget_icon(WidgetIcon::ExternalTexture(61)),
                MenuItem::action("Disabled")
                    .disabled()
                    .with_widget_icon(WidgetIcon::ExternalTexture(62)),
            ],
        );
        let idle = MenuState::new();
        let mut hovered = MenuState::new();
        hovered.highlighted = Some(0);

        let idle_out = emit_menu(1, 2, &desc, &idle, ControlState::default()).0;
        let hovered_out = emit_menu(1, 2, &desc, &hovered, ControlState::default()).0;
        assert_eq!(idle_out.primitives.len(), hovered_out.primitives.len());

        let img_enabled = idle_out
            .primitives
            .iter()
            .find(|p| p.prim_type() == PrimType::Image && p.border_token_or_tex == 61)
            .expect("enabled menu item external icon");
        assert_eq!(img_enabled.fill_token_or_uv, u32::from(Token::NONE.raw()));
        assert!((img_enabled.opacity() - 1.0).abs() < 0.01);

        let img_disabled = idle_out
            .primitives
            .iter()
            .find(|p| p.prim_type() == PrimType::Image && p.border_token_or_tex == 62)
            .expect("disabled menu item external icon");
        assert_eq!(img_disabled.fill_token_or_uv, u32::from(Token::NONE.raw()));
        assert!((img_disabled.opacity() - crate::WINUI_DISABLED_OPACITY).abs() < 0.02);
    }

    #[test]
    fn context_menu_desc_uses_win11_geometry_padding_and_separator_height() {
        let items = vec![
            MenuItem::action("Open").with_shortcut("Ctrl+O"),
            MenuItem::separator(),
            MenuItem::action("Delete").danger(true),
        ];
        let desc = MenuDesc::context_menu(Rect::new(20.0, 30.0, 220.0, 0.0), items);
        assert_eq!(desc.item_height, CONTEXT_MENU_ITEM_HEIGHT);
        assert_eq!(desc.radius, MENU_RADIUS);
        let expected_h =
            MENU_PANEL_PADDING * 2.0 + CONTEXT_MENU_ITEM_HEIGHT * 2.0 + MENU_SEPARATOR_HEIGHT;
        assert_eq!(desc.content_height(), expected_h);

        let r0 = desc.item_rect(0);
        let r1 = desc.item_rect(1);
        let r2 = desc.item_rect(2);
        assert_eq!(r0.origin.y, 30.0 + MENU_PANEL_PADDING);
        assert_eq!(r0.height(), CONTEXT_MENU_ITEM_HEIGHT);
        assert_eq!(r1.origin.y, r0.max().y);
        assert_eq!(r1.height(), MENU_SEPARATOR_HEIGHT);
        assert_eq!(r2.origin.y, r1.max().y);
        assert_eq!(r2.height(), CONTEXT_MENU_ITEM_HEIGHT);

        let sized =
            MenuDesc::context_menu(Rect::new(20.0, 30.0, 220.0, expected_h), desc.items.clone());
        assert_eq!(
            sized.item_at(unbound_math::Vec2::new(40.0, r0.center().y)),
            Some(0)
        );
        assert_eq!(
            sized.item_at(unbound_math::Vec2::new(40.0, r1.center().y)),
            Some(1)
        );
        assert_eq!(
            sized.item_at(unbound_math::Vec2::new(40.0, r2.center().y)),
            Some(2)
        );
        // 顶部内边距区不命中任何菜单项。
        assert_eq!(sized.item_at(unbound_math::Vec2::new(40.0, 31.0)), None);

        let custom = sized.with_item_height(36.0);
        assert_eq!(custom.item_height, 36.0);
        assert_eq!(
            custom.content_height(),
            MENU_PANEL_PADDING * 2.0 + 36.0 * 2.0 + MENU_SEPARATOR_HEIGHT
        );
    }

    #[test]
    fn emit_menu_with_rows_renders_win11_elements_sub_hits_and_constant_count() {
        let items = vec![
            MenuItem::action("Open")
                .with_widget_icon(WidgetIcon::ExternalTexture(71))
                .with_shortcut("Ctrl+O"),
            MenuItem::separator(),
            MenuItem::action("Disabled")
                .disabled()
                .with_shortcut("Ctrl+D"),
            MenuItem::action("More").with_submenu(vec![MenuItem::action("Sub")]),
            MenuItem::action("Delete").danger(true),
        ];
        let mut desc = MenuDesc::context_menu(Rect::new(40.0, 50.0, 240.0, 0.0), items);
        desc.rect = Rect::new(40.0, 50.0, 240.0, desc.content_height());

        let q_open = [dummy_quad(80.0, 62.0)];
        let q_open_sc = [dummy_quad(220.0, 62.0)];
        let q_dis = [dummy_quad(80.0, 103.0)];
        let q_dis_sc = [dummy_quad(220.0, 103.0)];
        let q_more = [dummy_quad(80.0, 135.0)];
        let q_del = [dummy_quad(80.0, 167.0)];

        let make_rows = |p0: PointerPhase, p4: PointerPhase| {
            vec![
                MenuItemRenderRow {
                    label_glyphs: &q_open,
                    shortcut_glyphs: &q_open_sc,
                    phase: p0,
                },
                MenuItemRenderRow::default(),
                MenuItemRenderRow {
                    label_glyphs: &q_dis,
                    shortcut_glyphs: &q_dis_sc,
                    phase: PointerPhase::Hovered,
                },
                MenuItemRenderRow {
                    label_glyphs: &q_more,
                    shortcut_glyphs: &[],
                    phase: PointerPhase::Idle,
                },
                MenuItemRenderRow {
                    label_glyphs: &q_del,
                    shortcut_glyphs: &[],
                    phase: p4,
                },
            ]
        };

        let menu_key = 9001u64;
        let state = MenuState::new();
        let idle_rows = make_rows(PointerPhase::Idle, PointerPhase::Idle);
        let hover_rows = make_rows(PointerPhase::Hovered, PointerPhase::Hovered);
        let press_rows = make_rows(PointerPhase::Pressed, PointerPhase::Pressed);

        let idle_out = emit_menu_with_rows(
            menu_key,
            1,
            &desc,
            &state,
            ControlState::default(),
            &idle_rows,
        )
        .0;
        let hover_out = emit_menu_with_rows(
            menu_key,
            1,
            &desc,
            &state,
            ControlState::default(),
            &hover_rows,
        )
        .0;
        let press_out = emit_menu_with_rows(
            menu_key,
            1,
            &desc,
            &state,
            ControlState::default(),
            &press_rows,
        )
        .0;

        assert_eq!(idle_out.primitives.len(), hover_out.primitives.len());
        assert_eq!(idle_out.primitives.len(), press_out.primitives.len());

        // 仅可选非分隔线项（0、3、4）注册子热区，分隔线（1）与禁用项（2）不得注册。
        let sub_nodes: Vec<u64> = idle_out.sub_hits.iter().map(|h| h.node).collect();
        assert_eq!(
            sub_nodes,
            vec![
                menu_item_node(menu_key, 0),
                menu_item_node(menu_key, 3),
                menu_item_node(menu_key, 4),
            ]
        );
        assert!(idle_out.sub_hits.iter().all(|h| h.layer == MENU_LAYER));

        // 悬停与按压胶囊填充令牌正确切换。
        assert!(hover_out
            .primitives
            .iter()
            .any(|p| p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv == u32::from(Token::HOVER.raw())
                && p.opacity() > 0.9));
        assert!(hover_out
            .primitives
            .iter()
            .any(|p| p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv == u32::from(Token::ERROR_SOFT.raw())
                && p.opacity() > 0.9));
        assert!(press_out
            .primitives
            .iter()
            .any(|p| p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv == u32::from(Token::PRESSED.raw())
                && p.opacity() > 0.9));

        // 危险项字形使用 Token::ERROR，禁用项字形使用 Token::TEXT_DISABLED，快捷键使用 Token::TEXT_SECONDARY。
        assert!(idle_out.primitives.iter().any(|p| {
            p.prim_type() == PrimType::Glyph && p.fill_token_or_uv == u32::from(Token::ERROR.raw())
        }));
        assert!(idle_out.primitives.iter().any(|p| {
            p.prim_type() == PrimType::Glyph
                && p.fill_token_or_uv == u32::from(Token::TEXT_DISABLED.raw())
        }));
        assert!(idle_out.primitives.iter().any(|p| {
            p.prim_type() == PrimType::Glyph
                && p.fill_token_or_uv == u32::from(Token::TEXT_SECONDARY.raw())
        }));
        // 分隔线使用 Token::DIVIDER，外部彩色图标保持 Token::NONE 填充。
        assert!(idle_out.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv == u32::from(Token::DIVIDER.raw())
        }));
        let ext_img = idle_out
            .primitives
            .iter()
            .find(|p| p.prim_type() == PrimType::Image && p.border_token_or_tex == 71)
            .expect("external icon image primitive");
        assert_eq!(ext_img.fill_token_or_uv, u32::from(Token::NONE.raw()));
    }

    #[test]
    fn context_menu_emits_acrylic_backdrop_and_strict_layer_ordering() {
        let items = vec![
            MenuItem::action("Refresh").with_widget_icon(WidgetIcon::Vector(ShellIcon::Refresh)),
            MenuItem::action("Settings")
                .disabled()
                .with_widget_icon(WidgetIcon::Vector(ShellIcon::Settings)),
        ];
        let mut desc = MenuDesc::context_menu(Rect::new(40.0, 60.0, 220.0, 0.0), items);
        desc.rect = Rect::new(40.0, 60.0, 220.0, desc.content_height());
        desc.z = 1100;

        let req = desc
            .backdrop_request(42)
            .expect("context menu must emit an acrylic backdrop request");
        assert_eq!(req.blur, MENU_BACKDROP_BLUR);
        assert_eq!(req.corner_radii, [MENU_RADIUS; 4]);
        assert_eq!(req.z, 1100);

        let (out, _) = emit_menu(42, 1, &desc, &MenuState::new(), ControlState::default());
        // 索引 1 为菜单面板背景：应处于 desc.z 且为半透明亚克力叠色。
        let panel_bg = out.primitives[1];
        assert_eq!(panel_bg.depth_u16(), 1100);
        assert!((panel_bg.opacity() - MENU_ACRYLIC_SURFACE_OPACITY).abs() < 0.02);

        // 索引 2 为首行胶囊：应严格位于面板背景之上（desc.z + 1）。
        let row_pill = out.primitives[2];
        assert_eq!(row_pill.depth_u16(), 1101);
        assert_eq!(
            (row_pill.prim_type_and_flags >> 16) & 0x7,
            u32::from(SubElement::Background as u8)
        );

        // 所有向量图标图元均处于 SubElement::Content 且 z >= row_pill.depth_u16()。
        let icon_prims: Vec<_> = out
            .primitives
            .iter()
            .filter(|p| {
                p.fill_token_or_uv == u32::from(Token::TEXT_DISABLED.raw())
                    || p.border_token_or_tex == u32::from(Token::TEXT_DISABLED.raw())
            })
            .collect();
        assert!(!icon_prims.is_empty());
        for prim in icon_prims {
            assert!(prim.depth_u16() >= row_pill.depth_u16());
            assert_eq!(
                (prim.prim_type_and_flags >> 16) & 0x7,
                u32::from(SubElement::Content as u8)
            );
        }
    }

    #[test]
    fn menu_overlay_scrollbar_emits_only_when_overflowing_and_supports_drag_and_paging() {
        use crate::scroll::{
            scrollbar_thumb_node, scrollbar_track_node, ScrollbarAxis, SCROLLBAR_THUMB_IDLE_WIDTH,
            SCROLLBAR_THUMB_WIDTH,
        };
        use unbound_math::Vec2;

        let node = 88u64;
        let thumb_node = scrollbar_thumb_node(node, ScrollbarAxis::Vertical);
        let track_node = scrollbar_track_node(node, ScrollbarAxis::Vertical);

        // 常规短菜单（不溢出）：0 滚动条图元与 0 滚动条命中区。
        let short_desc = desc();
        let short_state = MenuState::new();
        let short_out = emit_menu(node, 1, &short_desc, &short_state, ControlState::default()).0;
        assert!(!short_out
            .sub_hits
            .iter()
            .any(|h| h.node == thumb_node || h.node == track_node));

        // 溢出菜单：20 项（740px > 200px）在面板右缘发射 2 个稳定槽位与轨道/滑块命中区。
        let items: Vec<MenuItem> = (0..20)
            .map(|i| MenuItem::action(format!("Item {i}")))
            .collect();
        let tall_desc = MenuDesc::new(Rect::new(40.0, 40.0, 220.0, 200.0), items);
        let mut tall_state = MenuState::new();
        let idle_out = emit_menu(node, 1, &tall_desc, &tall_state, ControlState::default()).0;
        // 2 个外框图元 + 20 行 * 2 + 1 边框 + 2 滚动条槽位 = 45。
        assert_eq!(idle_out.primitives.len(), 45);
        let track_hit = idle_out
            .sub_hits
            .iter()
            .find(|h| h.node == track_node)
            .expect("overflowing menu track hit");
        let thumb_hit = idle_out
            .sub_hits
            .iter()
            .find(|h| h.node == thumb_node)
            .expect("overflowing menu thumb hit");
        assert_eq!(track_hit.layer, MENU_LAYER);
        assert_eq!(thumb_hit.layer, MENU_LAYER);
        assert!(tall_desc.rect.contains_rect(&track_hit.bounds));
        assert!(tall_desc.rect.contains_rect(&thumb_hit.bounds));
        assert_eq!(idle_out.primitives[43].opacity(), 0.0);
        assert!((idle_out.primitives[44].size[0] - SCROLLBAR_THUMB_IDLE_WIDTH).abs() < 0.01);

        // 悬停展开为 6px 粗滑块与半透明轨道胶囊，图元总数保持恒定。
        tall_state.set_scrollbar_hovered(true);
        let hovered_out = emit_menu(node, 1, &tall_desc, &tall_state, ControlState::default()).0;
        assert_eq!(hovered_out.primitives.len(), 45);
        assert!(hovered_out.primitives[43].opacity() > 0.0);
        assert!((hovered_out.primitives[44].size[0] - SCROLLBAR_THUMB_WIDTH).abs() < 0.01);

        // 点击轨道翻页与拖拽滑块。
        let track_click = Vec2::new(track_hit.bounds.center().x, track_hit.bounds.max().y - 4.0);
        assert!(
            tall_state.reduce_scrollbar_pointer(&tall_desc, ScrollPointerEvent::Press(track_click))
        );
        assert!(tall_state.scroll_offset().y > 0.0);

        let cur_thumb = tall_state.scrollbar_desc(&tall_desc).thumb_rect();
        let drag_start = cur_thumb.center();
        assert!(
            tall_state.reduce_scrollbar_pointer(&tall_desc, ScrollPointerEvent::Press(drag_start))
        );
        assert!(tall_state.is_dragging_scrollbar());
        assert!(tall_state.reduce_scrollbar_pointer(
            &tall_desc,
            ScrollPointerEvent::Drag(Vec2::new(drag_start.x, drag_start.y + 30.0))
        ));
        assert!(tall_state.reduce_scrollbar_pointer(&tall_desc, ScrollPointerEvent::Release));
        assert!(!tall_state.is_dragging_scrollbar());
    }
}
