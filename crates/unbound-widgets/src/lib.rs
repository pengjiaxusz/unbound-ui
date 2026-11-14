//! 标准控件库：所有组件共同遵循的语义词汇表。
//!
//! 本 crate 位于引擎能力层与应用程序层之间。它负责*控件*语义——纯描述符、
//! 派生自输入路由器的视觉状态与场景发射——但不涉及窗口管理、布局求解或应用状态。
//!
//! 本 crate 中的所有控件均遵守三条原则：
//!
//! 1. **禁止交互布尔标志。** 控件绝不存储“我是否悬停”。[`ControlState::resolve`]
//!    每帧从 [`unbound_input::Router`] 派生交互阶段，因此指针在捕获状态下离开控件时仍保持*按下*。
//! 2. **无闭包、无类型擦除。** 控件是纯数据结构体加纯函数，将 `(descriptor, state)`
//!    转换为图元、命中区域与语义节点。
//! 3. **颜色皆为令牌，绝非字面量。** 控件仅引用 [`unbound_theme::Token`] 槽位；
//!    具体颜色由主题决定。因此切换主题绝不会重新上传任何图元。
//!
//! [`app-guest`](../../app_guest) 仍仅依赖 [`unbound_abi`]，因此客体应用无需引入引擎即可宿主这些控件。

// 基础设施模块：场景差分与图元打包辅助工具。它们不属于业务组件，因此不纳入展示目录。
pub mod catalog;
pub mod diff;
pub mod ergonomics;
pub mod primitive;

/// 声明所有业务组件模块并派生穷举目录。
///
/// 该宏是组件词汇表的单一事实来源：它同时生成 `pub mod` 声明*和* [`WidgetCatalogId`] 枚举。
/// 由于该枚举未标记 `#[non_exhaustive]`，下游每次对它的 `match`（稳定形状探针、展示覆盖率门禁）
/// 都必须列出所有变体——在此添加模块而未接入这些匹配分支将触发编译错误（`E0004`），而非静默遗漏。
macro_rules! define_widget_catalog {
    (
        modules: [ $( $module:ident ),+ $(,)? ],
        catalog: [ $( $variant:ident => $cat_module:ident ),+ $(,)? ] $(,)?
    ) => {
        $( pub mod $module; )+

        /// 每个业务组件或子组件表面对应一个变体。
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
        pub enum WidgetCatalogId {
            $( $variant, )+
        }

        impl WidgetCatalogId {
            /// 按声明顺序列出的所有组件。
            pub const ALL: &'static [Self] = &[ $( Self::$variant ),+ ];
            /// 业务组件总数。
            pub const COUNT: usize = Self::ALL.len();

            /// 实现该组件的源码模块名。
            #[must_use]
            pub const fn module_name(self) -> &'static str {
                match self { $( Self::$variant => stringify!($cat_module), )+ }
            }

            /// [`Self::ALL`] 中位于 `index` 处的组件。
            #[must_use]
            pub const fn from_index(index: usize) -> Self {
                Self::ALL[index]
            }
        }
    };
}

define_widget_catalog! {
    modules: [
        accordion,
        alert,
        alert_dialog,
        autocomplete,
        avatar,
        badge,
        banner,
        breadcrumb,
        button,
        callout,
        canvas_toolbar,
        carousel,
        cascader,
        chart,
        checkbox,
        code_block,
        code_editor,
        collapsible,
        color_picker,
        combobox,
        command_palette,
        containers,
        context_menu,
        curve_editor,
        data_grid,
        date_picker,
        date_range_picker,
        descriptions,
        divider,
        dock,
        drawer,
        file_drop,
        form,
        gizmo,
        hotkey_input,
        hover_card,
        image,
        masonry,
        menu,
        menubar,
        minimap,
        modal,
        multi_split,
        navigation_rail,
        node_graph,
        notification_center,
        number_input,
        overlay,
        pagination,
        pin_input,
        popconfirm,
        popover,
        progress,
        property_grid,
        radio,
        rating,
        rich_text,
        ruler,
        scroll,
        segmented,
        select,
        skeleton,
        slider,
        spinner,
        split_button,
        split_pane,
        stepper,
        switch,
        table,
        tabs,
        tag_input,
        terminal_view,
        text_area,
        text_input,
        time_picker,
        timeline,
        timeline_track,
        toast,
        toggle_button,
        toolbar,
        tooltip,
        tour,
        transfer,
        tree_table,
        tree_view,
        virtual_masonry,
        virtual_views,
        watermark,
    ],
    catalog: [
        Accordion => accordion,
        Alert => alert,
        AlertDialog => alert_dialog,
        AspectRatioBox => masonry,
        AutoComplete => autocomplete,
        Avatar => avatar,
        AvatarGroup => avatar,
        Badge => badge,
        Banner => banner,
        Breadcrumb => breadcrumb,
        Button => button,
        Callout => callout,
        CanvasToolbar => canvas_toolbar,
        Carousel => carousel,
        Cascader => cascader,
        Chart => chart,
        Checkbox => checkbox,
        Chip => badge,
        CodeBlock => code_block,
        CodeEditor => code_editor,
        Collapsible => collapsible,
        ColorPicker => color_picker,
        Combobox => combobox,
        CommandPalette => command_palette,
        Containers => containers,
        ContextMenu => context_menu,
        CurveEditor => curve_editor,
        DataGrid => data_grid,
        DatePicker => date_picker,
        DateRangePicker => date_range_picker,
        Descriptions => descriptions,
        Divider => divider,
        Dock => dock,
        Drawer => drawer,
        EmptyState => hover_card,
        FileDrop => file_drop,
        Form => form,
        Gizmo => gizmo,
        GradientEditor => curve_editor,
        HotkeyInput => hotkey_input,
        HoverCard => hover_card,
        Icon => image,
        Image => image,
        LoadingOverlay => spinner,
        Masonry => masonry,
        Menu => menu,
        Menubar => menubar,
        Minimap => minimap,
        Modal => modal,
        MultiSplit => multi_split,
        NavigationRail => navigation_rail,
        NinePatch => image,
        NodeGraph => node_graph,
        NotificationCenter => notification_center,
        NumberInput => number_input,
        Overlay => overlay,
        Pagination => pagination,
        PinInput => pin_input,
        Popconfirm => popconfirm,
        Popover => popover,
        Progress => progress,
        PropertyGrid => property_grid,
        Radio => radio,
        Rating => rating,
        RichText => rich_text,
        Ruler => ruler,
        Scroll => scroll,
        Segmented => segmented,
        Select => select,
        Skeleton => skeleton,
        Slider => slider,
        Spinner => spinner,
        SplitButton => split_button,
        SplitPane => split_pane,
        Stack => containers,
        StatusBar => toolbar,
        Stepper => stepper,
        Switch => switch,
        Table => table,
        Tabs => tabs,
        TagInput => tag_input,
        TerminalView => terminal_view,
        TextArea => text_area,
        TextInput => text_input,
        TimePicker => time_picker,
        Timeline => timeline,
        TimelineTrack => timeline_track,
        Toast => toast,
        ToggleButton => toggle_button,
        Toolbar => toolbar,
        Tooltip => tooltip,
        Tour => tour,
        Transfer => transfer,
        TreeTable => tree_table,
        TreeView => tree_view,
        VirtualMasonry => virtual_masonry,
        VirtualViews => virtual_views,
        Watermark => watermark,
        Wrap => containers,
    ],
}

/// 证明组件确实通过真实发射器生成，而非手绘矩形伪造。
///
/// 构造函数为 `pub(crate)`：只有 `unbound-widgets` 自身能通过目录探针运行真实发射器并铸造见证。
/// 应用程序无法伪造该见证，因此返回见证的展示分支可确定已调用真实组件。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WidgetShowcaseWitness {
    id: WidgetCatalogId,
}

impl WidgetShowcaseWitness {
    pub(crate) const fn new(id: WidgetCatalogId) -> Self {
        Self { id }
    }

    /// 此见证所证明已发射的组件。
    #[must_use]
    pub const fn id(self) -> WidgetCatalogId {
        self.id
    }
}

use rustc_hash::FxHashMap;
use unbound_a11y::{A11yNode, Semantics};
use unbound_gpu::data::{GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform, CLIP_NONE};
use unbound_headless::driver::{ClickReport, TestApp};
use unbound_input::{ClipRect, HitRegion, PointerEvent, Router};
use unbound_math::{f16_to_f32, f32_to_f16, project_rounded_radii, Rect, Vec2};
use unbound_text::GlyphQuad;

pub use accordion::{
    emit_accordion_body, emit_accordion_header, AccordionDesc, AccordionSection, AccordionState,
    AccordionTokens,
};
pub use alert::{
    emit_alert, AlertDesc, Severity, ALERT_ACCENT_WIDTH, ALERT_CLOSE_SIZE, ALERT_ICON_SIZE,
    ALERT_PADDING, ALERT_RADIUS,
};
pub use alert_dialog::{
    emit_alert_dialog, AlertDialogDesc, AlertDialogKey, AlertDialogLayout, AlertDialogOutcome,
    AlertDialogOutput, AlertDialogState, AlertFocus, ConfirmTone, DIALOG_BUTTON_HEIGHT,
    DIALOG_FOOTER, DIALOG_ICON, DIALOG_PADDING, DIALOG_RADIUS,
};
pub use autocomplete::{emit_autocomplete, AutoCompleteDesc, AutoCompleteState};
pub use avatar::{emit_avatar, emit_avatar_group, initials, AvatarDesc, AvatarGroupDesc, Presence};
pub use badge::{emit_badge, emit_chip, BadgeDesc, BadgeVariant, ChipDesc};
pub use banner::{emit_banner, BannerDesc};
pub use breadcrumb::{
    emit_crumb, emit_crumb_ellipsis, emit_crumb_separator, BreadcrumbDesc, BreadcrumbState,
    BreadcrumbTokens, Crumb, CrumbSlot,
};
pub use button::{ButtonDesc, ButtonSize, ButtonTokens, ButtonVariant, ButtonWidget};
pub use callout::{emit_callout, CalloutDesc, CalloutState};
pub use canvas_toolbar::{
    emit_canvas_toolbar, CanvasTool, CanvasToolbarDesc, CanvasToolbarOutput, CanvasToolbarState,
    TOOLBAR_BLUR, TOOLBAR_HEIGHT, TOOL_BUTTON, TOOL_GAP, ZOOM_RANGE, ZOOM_WIDTH,
};
pub use carousel::{emit_carousel, CarouselDesc};
pub use cascader::{emit_cascader, CascaderDesc, CascaderOption};
pub use chart::{
    emit_chart, ChartDesc, ChartOutput, ChartSeries, ChartState, SeriesKind, CHART_INSET,
    CHART_STROKE, HOVER_RADIUS, TOOLTIP_SIZE,
};
pub use checkbox::{CheckState, CheckboxDesc, CheckboxTokens};
pub use code_block::{
    emit_code_block, highlight_line, CodeBlockDesc, CodeBlockState, CodeSpan, SyntaxToken,
    CODE_GUTTER, CODE_LINE_HEIGHT, COPY_CONFIRM_SECONDS,
};
pub use code_editor::{emit_code_editor, CodeEditorDesc, CodeEditorState};
pub use collapsible::{emit_collapsible, CollapsibleDesc, CollapsibleState};
pub use color_picker::{
    emit_color_picker, parse_hex, parse_oklch, parse_rgba, ColorFormat, ColorPart, ColorPickerDesc,
    ColorPickerLayout, ColorPickerState, Hsva,
};
pub use combobox::{
    emit_combobox_popup, emit_combobox_trigger, ComboItem, ComboboxDesc, ComboboxKey,
    ComboboxOutcome, ComboboxState,
};
pub use command_palette::{
    emit_command_palette, emit_command_row, score_command, Command, CommandPaletteDesc,
    CommandPaletteState, CommandPaletteTokens, MatchKind,
};
pub use containers::{
    emit_card, emit_stack, CardDesc, CardTokens, StackDesc, WrapDesc, CARD_ELEVATIONS, WRAP_FLAG,
};
pub use context_menu::{ContextClose, ContextKey, ContextMenuState};
pub use curve_editor::{
    emit_curve_editor, emit_gradient_editor, CurveEditorDesc, CurveEditorState, CurveHandle,
    GradientEditorState, GradientStop,
};
pub use data_grid::{emit_data_grid, DataGridDesc, DataGridState};
pub use date_picker::{
    days_in_month, emit_calendar, is_leap_year, CalendarCell, CalendarState, DatePickerDesc,
    NaiveDate, SelectionMode, YearMonth,
};
pub use date_range_picker::{
    emit_date_range_picker, DateRange, DateRangePickerDesc, DateRangePickerState,
};
pub use descriptions::{emit_descriptions, DescriptionItem, DescriptionsDesc, DescriptionsState};
pub use divider::{emit_divider, DividerDesc, DividerOrientation, DividerTokens};
pub use dock::{
    emit_dock_pane, emit_dock_preview, emit_dock_tab, DockDrag, DockHover, DockNode, DockPane,
    DockState, DockTab, DockTokens, DockTree, DockZone,
};
pub use drawer::{
    drawer_rect, emit_drawer, DrawerDesc, DrawerOutput, DrawerSide, DrawerState, DRAWER_BLUR,
    DRAWER_RADIUS,
};
pub use ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
pub use file_drop::{drop_point, emit_file_drop, FileDropDesc, FileDropState};
pub use form::{
    emit_form_label, emit_form_message, emit_form_root, layout_form, required_marker, reveal_field,
    FormDesc, FormField, FormFieldDesc, FormFieldLayout, FormLayout, FormState, FORM_FIELD_GAP,
    FORM_INPUT_HEIGHT, FORM_LABEL_GAP, FORM_LINE, FORM_MESSAGE_GAP,
};
pub use gizmo::{
    emit_gizmo, grip_rect_for, GizmoDesc, GizmoOutput, GRIP_SIZE, ROTATE_GRIP, ROTATE_OFFSET,
    SIZE_BUBBLE,
};
pub use hotkey_input::{emit_hotkey_input, HotkeyInputDesc, HotkeyInputState};
pub use hover_card::{
    emit_empty_state, emit_hover_card, EmptyStateDesc, EmptyStateOutput, HoverCardDesc,
    HoverCardState, EMPTY_ACTION_SIZE, EMPTY_ICON, HOVER_CARD_DELAY_MS, HOVER_CARD_GRACE_MS,
    HOVER_CARD_RADIUS,
};
pub use image::{
    emit_icon, emit_image, emit_image_with_assets, emit_nine_patch, fit_image, IconDesc,
    ImageAssetResolver, ImageDesc, ImageFit, ImageSource, ImageWidget, NinePatchDesc, ObjectFit,
    ResolvedImage,
};
pub use masonry::{emit_masonry, AspectRatioBoxDesc, MasonryDesc, MasonryItem};
pub use menu::{emit_menu, MenuDesc, MenuItem, MenuState, MenuWidget};
pub use menubar::{
    emit_menubar, emit_menubar_item, reduce_menubar_key, MenuBarDesc, MenuBarItem, MenuBarKey,
    MenuBarState, MenuBarTokens,
};
pub use minimap::{emit_minimap, MinimapDesc, MinimapItem, MinimapState, MINIMAP_INSET};
pub use modal::{emit_modal, ModalDesc, ModalOutput, ModalWidget};
pub use multi_split::{emit_multi_split, MultiSplitDesc, MultiSplitState, MULTI_SPLIT_GUTTER_SIZE};
pub use navigation_rail::{
    emit_navigation_rail, NavRailItem, NavigationRailDesc, NavigationRailState,
};
pub use node_graph::{
    emit_node_graph, nearest_input, ActiveWire, GraphNodeDesc, GraphWire, NodeGraphOutput,
    NodeGraphState, PinHit, PinKind, PinSpec, NODE_HEADER, PIN_RADIUS, PIN_SNAP, PIN_SPACING,
    WIRE_WIDTH,
};
pub use notification_center::{
    emit_notification_center, NotificationCenterDesc, NotificationCenterState, NotificationItem,
};
pub use number_input::{
    emit_number_input, evaluate_expression, format_number, number_text_rect, round_to,
    spin_down_rect, spin_up_rect, NumberInputDesc, NumberInputState, NumberMode, NUMBER_PADDING,
    NUMBER_RADIUS, SPIN_WIDTH,
};
pub use overlay::{
    place_anchored_popup, BackdropRequest, OverlayEntry, OverlayKind, OverlayStack, Placement,
};
pub use pagination::{emit_pagination, PaginationDesc, PaginationState};
pub use pin_input::{emit_pin_input, PinInputDesc, PinInputState};
pub use popconfirm::{emit_popconfirm, PopconfirmDesc};
pub use popover::{emit_popover, PopoverDesc, PopoverOutput, PopoverWidget};
pub use progress::{
    emit_progress, ProgressDesc, ProgressKind, ProgressState, ProgressTokens, PROGRESS_RATE,
    RING_SEGMENTS,
};
pub use property_grid::{
    emit_property_grid, PropertyDesc, PropertyGridDesc, PropertyGridState, PropertyGridTokens,
    PropertyKind, PropertyRowLayout, PropertySection, PropertyValue,
};
pub use radio::{
    reduce_radio_group, RadioDesc, RadioGroupDesc, RadioGroupKey, RadioGroupOutcome, RadioTokens,
};
pub use rating::{emit_rating, RatingDesc, RatingState};
pub use rich_text::{
    emit_rich_text, parse_markdown, BlockKind, RichBlock, RichDocument, RichTextDesc,
    HEADING_SCALE, RICH_TEXT_SIZE,
};
pub use ruler::{
    emit_rulers, nice_step, GuideAxis, GuideLine, RulerDesc, RulerOutput, RulerState, RULER_BAND,
    RULER_MINOR_PER_MAJOR, RULER_MIN_STEP,
};
pub use scroll::{
    emit_scroll_view, update_virtual_list, ScrollAxis, ScrollPointerEvent, ScrollState,
    ScrollViewDesc, ScrollViewOutput, ScrollViewWidget, ScrollbarAxis, VirtualWindow,
};
pub use segmented::{SegmentedDesc, SegmentedTokens};
pub use select::{emit_select, SelectDesc, SelectState, SelectWidget};
pub use skeleton::{emit_skeleton, SkeletonDesc, SkeletonState, SKELETON_PERIOD, SKELETON_SHIMMER};
pub use slider::{SliderDesc, SliderTokens};
pub use spinner::{emit_loading_overlay, emit_spinner, LoadingOverlayDesc, SpinnerDesc};
pub use split_button::{emit_split_button, SplitButtonDesc, SplitButtonState, DROPDOWN_WIDTH};
pub use split_pane::{
    emit_split_pane, emit_split_pane_panes, reduce_split_key, SplitKey, SplitLayout, SplitPaneDesc,
    SplitPaneState, SplitPaneTokens, SplitSide,
};
pub use stepper::{emit_stepper, StepItem, StepStatus, StepperDesc, StepperOrientation};
pub use switch::{SwitchDesc, SwitchTokens};
pub use table::{
    emit_table, AlignX as TableAlignX, ColumnSpec, RowWindow, SortDir, TableDesc, TableRow,
    TableState, TableTokens,
};
pub use tabs::{
    emit_tab, emit_tab_close, emit_tabs_add, emit_tabs_strip, reduce_tabs_key, TabItem, TabsDesc,
    TabsKey, TabsState, TabsTokens,
};
pub use tag_input::{
    emit_tag_input, layout_tags, TagInputDesc, TagInputOutcome, TagInputState, TagKey, TagLayout,
    TAG_CLOSE, TAG_GAP, TAG_HEIGHT, TAG_INSET, TAG_PADDING, TAG_RADIUS,
};
pub use terminal_view::{
    emit_terminal_view, TerminalViewDesc, TerminalViewState, TERMINAL_LINE_HEIGHT,
};
pub use text_area::{
    emit_text_area, logical_lines, AreaLine, TextAreaDesc, TextAreaState, TextAreaVisual,
    AREA_PADDING, AREA_RADIUS,
};
pub use text_input::{
    emit_text_input, reduce_text_input, TextEditState, TextInputDesc, TextInputEvent,
    TextInputOutcome, TextInputVisual, TextInputWidget,
};
pub use time_picker::{emit_time_picker, TimePickerDesc, TimePickerState, TimeValue};
pub use timeline::{emit_timeline, TimelineDesc, TimelineItem};
pub use timeline_track::{emit_timeline_track, Keyframe, TimelineTrackDesc, TimelineTrackState};
pub use toast::{
    emit_toast, ToastEntry, ToastLayout, ToastOutput, ToastSpring, ToastStack, ToastTick,
    TOAST_ACTION_SIZE, TOAST_CLOSE_SIZE, TOAST_DURATION_MS, TOAST_RADIUS, TOAST_SLIDE_IN,
};
pub use toggle_button::{emit_toggle_button, ToggleButtonDesc, ToggleButtonState};
pub use toolbar::{
    emit_status_bar, emit_status_segment, emit_toolbar, emit_toolbar_item, emit_toolbar_overflow,
    StatusAlign, StatusBarDesc, StatusSegment, ToolbarDesc, ToolbarItem, ToolbarItemKind,
    ToolbarLayout, ToolbarState, ToolbarTokens,
};
pub use tooltip::{emit_tooltip, TooltipDesc, TooltipState, TooltipWidget};
pub use tour::{emit_tour, TourDesc, TourState, TourStep};
pub use transfer::{emit_transfer, TransferDesc, TransferItem, TransferState};
pub use tree_table::{
    emit_tree_table, TreeTableDesc, TreeTableRow, TreeTableState, TREE_TABLE_HEADER_HEIGHT,
    TREE_TABLE_ROW_HEIGHT,
};
pub use tree_view::{
    emit_tree_background, emit_tree_row, TreeViewDesc, TreeViewRow, TreeViewState, TreeViewTokens,
};
pub use virtual_masonry::{emit_virtual_masonry, VirtualMasonryDesc, VirtualMasonryState};
pub use virtual_views::{emit_virtual_row, VariableVirtualList, VariableWindow, VirtualRowTokens};
pub use watermark::{emit_watermark, WatermarkDesc};

/// 无头控件会话渲染使用的默认视口。
pub const DEFAULT_VIEWPORT: (f32, f32) = (1280.0, 720.0);

/// 指针相对于控件的位置阶段。
///
/// 这是控件唯一需要的指针维度。焦点是另一个正交维度（[`ControlState::focused`]）：
/// 通过键盘到达的按钮即使在鼠标悬停于别处时也必须显示焦点环。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerPhase {
    /// 指针不在控件上方。
    #[default]
    Idle,
    /// 指针位于控件上方。
    Hovered,
    /// 指针位于控件上方且按键按下。
    Pressed,
}

/// 单帧内控件的视觉状态。
///
/// 每个字段均为派生值而非存储状态：[`ControlState::resolve`] 是引擎使用的唯一构造函数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlState {
    /// 指针相对于控件的位置阶段。
    pub phase: PointerPhase,
    /// 控件是否持有键盘焦点。
    pub focused: bool,
    /// 控件是否禁用交互。
    pub disabled: bool,
    /// 控件是否处于开启（切换）状态。
    pub selected: bool,
}

impl ControlState {
    /// 从指针路由器与焦点槽位派生控件状态。
    ///
    /// 禁用控件绝不显示指针阶段：它不是交互目标，因此指针不可能悬停其上。
    #[must_use]
    pub fn resolve(
        node_key: u64,
        router: &Router,
        focused_key: Option<u64>,
        disabled: bool,
        selected: bool,
    ) -> Self {
        let phase = if disabled {
            PointerPhase::Idle
        } else if router.pointer().pressed() == Some(node_key) {
            PointerPhase::Pressed
        } else if router.hover() == Some(node_key) {
            PointerPhase::Hovered
        } else {
            PointerPhase::Idle
        };
        Self {
            phase,
            focused: focused_key == Some(node_key),
            disabled,
            selected,
        }
    }
}

/// 单个控件在单帧内发射的输出。
///
/// 三部分刻意保持独立：纯装饰控件不发射命中区域，不在屏幕内的控件不发射图元，
/// 辅助技术不可达的控件不发射语义节点。
#[derive(Clone, Debug, Default)]
pub struct WidgetOutput {
    /// 按绘制顺序排列的图元。
    pub primitives: Vec<GpuPrimitiveInstance>,
    /// 图元引用的载荷数据。
    pub payloads: Vec<GpuPayload>,
    /// 可拾取表面（若控件接受输入）。
    pub hit: Option<HitRegion>,
    /// 多部件子命中区域（例如确认/取消按钮、关闭按钮、分页项）。
    pub sub_hits: smallvec::SmallVec<[HitRegion; 4]>,
    /// 语义节点（若控件可达）。
    pub a11y: Option<A11yNode>,
    /// 多部件子语义节点 `(child_key, A11yNode)`。
    pub sub_a11y: smallvec::SmallVec<[(u64, A11yNode); 4]>,
    /// 控件是否参与键盘 Tab 顺序。
    pub focusable: bool,
    /// 多部件控件的内部焦点顺序。
    pub sub_focus_order: smallvec::SmallVec<[u64; 4]>,
}

impl WidgetOutput {
    /// 注册带独立键、命中区域、无障碍节点与键盘可聚焦性的子控件（例如行内确认按钮或关闭按钮）。
    pub fn with_sub_control(
        &mut self,
        sub_key: u64,
        hit: HitRegion,
        a11y: A11yNode,
        focusable: bool,
    ) -> &mut Self {
        self.sub_hits.push(hit);
        self.sub_a11y.push((sub_key, a11y));
        if focusable {
            self.sub_focus_order.push(sub_key);
        }
        self
    }
}

/// 将多个控件的输出收集到同一帧中。
///
/// 收集器还为每个控件记录其拥有的图元半开区间。该映射使*局部*重绘成为可能：
/// 悬停若仅改变五个图元，只需重写 `primitives[start..end]` 并上传这些字节，而非重传整个场景。
#[derive(Default)]
pub struct SceneCollector {
    /// 按绘制顺序排列的图元。
    pub primitives: Vec<GpuPrimitiveInstance>,
    /// 变换表。
    pub transforms: Vec<GpuTransform>,
    /// 裁剪节点表。
    pub clips: Vec<GpuClipNode>,
    /// 图元引用的载荷数据。
    pub payloads: Vec<GpuPayload>,
    /// 可拾取表面列表。
    pub regions: Vec<HitRegion>,
    /// 语义树。
    pub semantics: Semantics,
    /// 键盘 Tab 顺序。
    pub focus_order: Vec<u64>,
    /// 按控件标识索引的各自图元区间。
    pub node_spans: FxHashMap<u64, (u32, u32)>,
    /// 本帧浮层产生的背景模糊请求。
    pub backdrops: Vec<crate::overlay::BackdropRequest>,
    /// 当前激活的裁剪作用域：`(clip index, screen-space bounds)`。
    clip_stack: Vec<(u32, Rect)>,
    /// 当前激活的变换作用域：`(transform index, cumulative translation)`。
    transform_stack: Vec<(u32, Vec2)>,
}

/// 将裁剪槽位的 `(centre, half-extent)` 形式还原为矩形。
fn center_half_rect(slot: [f32; 4]) -> Rect {
    Rect::new(
        slot[0] - slot[2],
        slot[1] - slot[3],
        slot[2] * 2.0,
        slot[3] * 2.0,
    )
}

impl SceneCollector {
    /// 创建语义树以 `root` 为根的空收集器。
    #[must_use]
    pub fn new(root: u64) -> Self {
        Self {
            semantics: Semantics::new(root),
            ..Self::default()
        }
    }

    /// 为新的一帧重置收集器，复用既有内存分配。
    pub fn reset(&mut self, root: u64) {
        self.primitives.clear();
        self.transforms.clear();
        self.clips.clear();
        self.payloads.clear();
        self.regions.clear();
        self.focus_order.clear();
        self.node_spans.clear();
        self.backdrops.clear();
        self.clip_stack.clear();
        self.transform_stack.clear();
        self.semantics.reset(root);
    }

    /// 为随后推入的所有控件开启裁剪作用域，直至调用 [`Self::pop_clip`]。
    ///
    /// `node` 描述新的裁剪区域；当存在父作用域时，其圆角槽位会折叠进 `rounded_clip_1`，
    /// 有效边界取两者的交集。返回的索引供子图元引用。
    pub fn push_clip(&mut self, mut node: GpuClipNode, world_bounds: Rect) -> u32 {
        let mut bounds = world_bounds;
        if let Some(&(parent_idx, parent_bounds)) = self.clip_stack.last() {
            bounds = bounds.intersect(&parent_bounds);
            let [min_x, min_y, max_x, max_y] = node.aabb_clip;
            let own = Rect::new(
                min_x,
                min_y,
                (max_x - min_x).max(0.0),
                (max_y - min_y).max(0.0),
            );
            let folded = own.intersect(&bounds);
            node.aabb_clip = [
                folded.min().x,
                folded.min().y,
                folded.max().x,
                folded.max().y,
            ];
            if let Some(parent) = self.clips.get(parent_idx as usize) {
                node.rounded_clip_1 = parent.rounded_clip_0;
                node.radii_1 = parent.radii_0;
                // 第四个圆角祖先会被丢弃：将其圆弧折叠进保留的槽位，
                // 使 3 层以上嵌套仍保持外层圆角而非退化为直角。
                // 无操作槽位的圆角半径为零，因此投影不会影响保留槽位。
                let kept_rect = center_half_rect(node.rounded_clip_1);
                let dropped_rect = center_half_rect(parent.rounded_clip_1);
                let kept_radii = node.radii_1.map(f16_to_f32);
                let dropped_radii = parent.radii_1.map(f16_to_f32);
                node.radii_1 =
                    project_rounded_radii(kept_rect, kept_radii, dropped_rect, dropped_radii)
                        .map(f32_to_f16);
            }
        }
        let index = u32::try_from(self.clips.len()).unwrap_or(CLIP_NONE - 1);
        self.clips.push(node);
        self.clip_stack.push((index, bounds));
        index
    }

    /// 关闭最内层裁剪作用域。
    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    /// 为随后推入的所有控件开启变换作用域，直至调用 [`Self::pop_transform`]。
    ///
    /// 作用域平移量为所有激活作用域之和，因此子节点只需引用最内层变换索引。
    /// 表的槽位 `0` 始终为单位变换。
    pub fn push_transform(&mut self, mut transform: GpuTransform) -> u32 {
        if self.transforms.is_empty() {
            self.transforms.push(GpuTransform::IDENTITY);
        }
        if let Some(&(_, parent_offset)) = self.transform_stack.last() {
            transform.translation[0] += parent_offset.x;
            transform.translation[1] += parent_offset.y;
        }
        let offset = Vec2::new(transform.translation[0], transform.translation[1]);
        let index = u32::try_from(self.transforms.len()).unwrap_or(CLIP_NONE - 1);
        self.transforms.push(transform);
        self.transform_stack.push((index, offset));
        index
    }

    /// 关闭最内层变换作用域。
    pub fn pop_transform(&mut self) {
        self.transform_stack.pop();
    }

    /// 追加单个控件的输出，并预留固定数量的图元槽位，使动态字形串不会导致后续控件的区间偏移。
    ///
    /// 未使用槽位以全透明实例填充，因此增减字符仅改变实际绘制的槽位而不会整体移动场景。
    /// 当输出超出预留数量时，区间长度向上取整到下一个 2 的幂。
    pub fn push_with_capacity(
        &mut self,
        key: u64,
        mut output: WidgetOutput,
        reserved_slots: usize,
    ) {
        let target = if output.primitives.len() > reserved_slots {
            output.primitives.len().next_power_of_two()
        } else {
            reserved_slots
        };
        output
            .primitives
            .resize_with(target, GpuPrimitiveInstance::default);
        self.push(key, output);
    }

    /// 追加单个控件的输出并记录其拥有的图元区间。
    pub fn push(&mut self, key: u64, mut output: WidgetOutput) {
        let active_transform = self.transform_stack.last().copied();
        if let Some((transform_idx, offset)) = active_transform {
            for primitive in &mut output.primitives {
                if primitive.transform_idx == 0 {
                    primitive.transform_idx = transform_idx;
                }
            }
            if let Some(node) = output.a11y.as_mut() {
                node.bounds = node.bounds.translate(offset);
            }
        }

        if let Some((clip_idx, clip_rect)) = self.clip_stack.last().copied() {
            for primitive in &mut output.primitives {
                if primitive.clip_idx == CLIP_NONE {
                    primitive.clip_idx = clip_idx;
                }
            }
            if let Some(node) = output.a11y.as_mut() {
                node.bounds = node.bounds.intersect(&clip_rect);
            }
        }

        if let Some(hit) = output.hit.as_mut() {
            self.scope_hit(hit);
        }
        for sub_hit in &mut output.sub_hits {
            self.scope_hit(sub_hit);
        }

        // 每个 widget 都从零开始索引*自己的* `payloads` 切片，而收集器把
        // 所有 widget 的 payload 拼成一个全局数组，因此追加图元前必须重定位
        // 这些索引。否则第二个标签的字形会读到第一个标签的 UV，每段文字
        // 都会渲染成其它文字的乱码混合。
        let payload_base = u32::try_from(self.payloads.len()).unwrap_or(u32::MAX);
        if payload_base != 0 {
            for primitive in &mut output.primitives {
                primitive.extra_payload_idx =
                    primitive.extra_payload_idx.saturating_add(payload_base);
            }
        }

        let start = u32::try_from(self.primitives.len()).unwrap_or(u32::MAX);
        self.primitives.extend(output.primitives);
        let end = u32::try_from(self.primitives.len()).unwrap_or(u32::MAX);
        self.node_spans.insert(key, (start, end));
        self.payloads.extend(output.payloads);
        if let Some(hit) = output.hit {
            self.regions.push(hit);
        }
        self.regions.extend(output.sub_hits);
        if let Some(node) = output.a11y {
            self.semantics.upsert(key, node);
        }
        for (sub_key, mut sub_node) in output.sub_a11y {
            if let Some((_, offset)) = active_transform {
                sub_node.bounds = sub_node.bounds.translate(offset);
            }
            if let Some((_, clip_rect)) = self.clip_stack.last().copied() {
                sub_node.bounds = sub_node.bounds.intersect(&clip_rect);
            }
            self.semantics.upsert(sub_key, sub_node);
            self.semantics.reparent(sub_key, key);
        }
        if output.focusable {
            self.focus_order.push(key);
        }
        for sub_focus in output.sub_focus_order {
            self.focus_order.push(sub_focus);
        }
    }

    /// 在当前激活的裁剪与变换作用域下注册独立命中区域（例如滚动条滑块），不拥有任何图元。
    pub fn push_hit(&mut self, mut hit: HitRegion) {
        self.scope_hit(&mut hit);
        self.regions.push(hit);
    }

    /// 将当前激活的变换与裁剪作用域折叠进命中区域边界。
    fn scope_hit(&self, hit: &mut HitRegion) {
        if let Some(&(_, offset)) = self.transform_stack.last() {
            hit.bounds = hit.bounds.translate(offset);
            if let Some(clip) = hit.clip.as_mut() {
                clip.rect = clip.rect.translate(offset);
            }
        }
        if let Some(&(_, clip_rect)) = self.clip_stack.last() {
            hit.bounds = hit.bounds.intersect(&clip_rect);
            hit.clip = Some(match hit.clip.take() {
                Some(existing) => ClipRect {
                    rect: existing.rect.intersect(&clip_rect),
                    radii: existing.radii,
                },
                None => ClipRect::new(clip_rect),
            });
        }
    }

    /// 单个控件拥有的图元切片。
    #[must_use]
    pub fn span(&self, key: u64) -> Option<&[GpuPrimitiveInstance]> {
        let (start, end) = *self.node_spans.get(&key)?;
        let start = usize::try_from(start).ok()?;
        let end = usize::try_from(end).ok()?;
        self.primitives.get(start..end)
    }
}

/// 控件描述符：决定控件场景的纯数据。
///
/// 该 trait 使同一个测试工具（[`WidgetHarness`]）能驱动任意控件：
/// 描述符只需回答“我在哪里”和“在此状态下我长什么样”，别无其他。
pub trait WidgetDesc {
    /// 控件的交互矩形。
    fn rect(&self) -> Rect;

    /// 控件实例的稳定标识。
    ///
    /// 该标识必须在重建后保持不变：路由器捕获它，焦点槽位指向它，语义树也以它为键。
    fn key(&self) -> u64;

    /// 控件是否禁用交互。
    fn disabled(&self) -> bool {
        false
    }

    /// 控件是否处于开启（切换）状态。
    fn selected(&self) -> bool {
        false
    }

    /// 为给定状态发射控件的场景输出。
    ///
    /// `glyphs` 携带控件绘制的已排版文本；发射器可调整其位置，但绝不重新排版。
    fn emit(&self, state: ControlState, glyphs: &mut [GlyphQuad]) -> WidgetOutput;
}

/// 围绕单个控件的无头交互会话。
///
/// 测试工具持有交互会话，在每次输入事件后从路由器重新派生视觉状态，
/// 仅当状态确实改变时才重新发射控件——与窗口应用的重绘策略一致。
pub struct WidgetHarness<D: WidgetDesc> {
    /// 共享交互会话。
    pub app: TestApp,
    /// 待测描述符。
    pub desc: D,
    /// 控件绘制的已排版字形。
    pub glyphs: Vec<GlyphQuad>,
    /// 最近一次发射的输出。
    pub output: WidgetOutput,
    /// 当前烘焙在 `output` 中的状态。
    baked: ControlState,
}

impl<D: WidgetDesc> WidgetHarness<D> {
    /// 使用默认视口创建会话。
    #[must_use]
    pub fn new(desc: D, glyphs: Vec<GlyphQuad>) -> Self {
        Self::with_viewport(desc, glyphs, DEFAULT_VIEWPORT)
    }

    /// 使用显式视口创建会话。
    #[must_use]
    pub fn with_viewport(desc: D, glyphs: Vec<GlyphQuad>, viewport: (f32, f32)) -> Self {
        let mut harness = Self {
            app: TestApp::new(viewport),
            desc,
            glyphs,
            output: WidgetOutput::default(),
            baked: ControlState::default(),
        };
        harness.rebuild();
        harness
    }

    /// 控件标识。
    #[must_use]
    pub fn node_key(&self) -> u64 {
        self.desc.key()
    }

    /// 从路由器与焦点槽位派生的状态。
    #[must_use]
    pub fn state(&self) -> ControlState {
        ControlState::resolve(
            self.desc.key(),
            &self.app.router,
            self.app.focused_key(),
            self.desc.disabled(),
            self.desc.selected(),
        )
    }

    /// 发射控件并将结果安装到会话中。
    pub fn rebuild(&mut self) {
        let state = self.state();
        self.output = self.desc.emit(state, &mut self.glyphs);
        self.install();
        self.baked = state;
    }

    /// 当视觉状态发生变化时重新发射控件。
    ///
    /// 返回场景是否被重建。
    pub fn sync(&mut self) -> bool {
        let state = self.state();
        if state == self.baked {
            return false;
        }
        self.rebuild();
        true
    }

    fn install(&mut self) {
        let key = self.desc.key();
        let mut semantics = Semantics::new(key);
        if let Some(node) = self.output.a11y.clone() {
            semantics.upsert(key, node);
        }
        let regions = self.output.hit.clone().into_iter().collect();
        self.app.install(regions, semantics);
    }

    /// 控件中心在窗口像素中的坐标。
    #[must_use]
    pub fn screen_center(&self) -> Vec2 {
        self.app.canvas_to_screen(self.desc.rect().center())
    }

    /// 将指针移动到控件上方。
    pub fn hover(&mut self) {
        self.hover_at(self.screen_center());
    }

    /// 将指针移动到指定窗口坐标。
    pub fn hover_at(&mut self, screen: Vec2) {
        self.app.move_to(screen);
        self.sync();
    }

    /// 在当前指针位置按下左键。
    pub fn press(&mut self) {
        self.app.pointer_down();
        self.sync();
    }

    /// 在当前指针位置释放左键。
    pub fn release(&mut self) -> Vec<PointerEvent> {
        let events = self.app.pointer_up();
        self.sync();
        events
    }

    /// 一步完成悬停、按下并释放控件。
    pub fn click(&mut self) -> ClickReport {
        let report = self.app.click(self.screen_center());
        self.sync();
        report
    }

    /// 赋予控件键盘焦点。
    pub fn focus(&mut self) {
        self.app.set_focus_key(self.desc.key());
        self.sync();
    }

    /// 移除控件键盘焦点。
    pub fn blur(&mut self) {
        self.app.clear_focus();
        self.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_input::PointerConfig;

    /// 创建一个包含归属于 `node` 的正方形命中区域的路由器。
    fn router_with(node: u64) -> Router {
        let mut router = Router::new(PointerConfig::default());
        router.set_regions(vec![HitRegion::new(
            node,
            1,
            0,
            Rect::new(0.0, 0.0, 100.0, 100.0),
        )]);
        router
    }

    #[test]
    fn the_pointer_phase_follows_the_router() {
        let mut router = router_with(7);
        router.pointer_move(Vec2::new(10.0, 10.0));
        let state = ControlState::resolve(7, &router, None, false, false);
        assert_eq!(state.phase, PointerPhase::Hovered);

        router.pointer_move(Vec2::new(500.0, 500.0));
        let state = ControlState::resolve(7, &router, None, false, false);
        assert_eq!(state.phase, PointerPhase::Idle);
    }

    #[test]
    fn a_captured_pointer_stays_pressed_after_it_leaves() {
        let mut router = router_with(7);
        router.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        // 指针已被捕获，因此移出区域不得结束按下状态。
        router.pointer_move(Vec2::new(500.0, 500.0));
        let state = ControlState::resolve(7, &router, None, false, false);
        assert_eq!(state.phase, PointerPhase::Pressed);
    }

    #[test]
    fn a_disabled_control_never_shows_a_pointer_phase() {
        let mut router = router_with(7);
        router.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        let state = ControlState::resolve(7, &router, None, true, false);
        assert_eq!(state.phase, PointerPhase::Idle);
        assert!(state.disabled);
    }

    #[test]
    fn the_focus_slot_is_orthogonal_to_the_pointer_phase() {
        let mut router = router_with(7);
        router.pointer_move(Vec2::new(10.0, 10.0));
        let state = ControlState::resolve(7, &router, Some(7), false, false);
        assert_eq!(state.phase, PointerPhase::Hovered);
        assert!(state.focused);

        // 焦点落在其他控件上时不影响指针阶段。
        let state = ControlState::resolve(7, &router, Some(9), false, false);
        assert_eq!(state.phase, PointerPhase::Hovered);
        assert!(!state.focused);
    }

    #[test]
    fn the_collector_records_a_span_per_widget() {
        let mut collector = SceneCollector::new(1);
        let first = WidgetOutput {
            primitives: vec![GpuPrimitiveInstance::default(); 3],
            ..WidgetOutput::default()
        };
        let second = WidgetOutput {
            primitives: vec![GpuPrimitiveInstance::default(); 2],
            ..WidgetOutput::default()
        };
        collector.push(10, first);
        collector.push(20, second);

        assert_eq!(collector.node_spans.get(&10), Some(&(0, 3)));
        assert_eq!(collector.node_spans.get(&20), Some(&(3, 5)));
        assert_eq!(
            collector.span(20).map(<[GpuPrimitiveInstance]>::len),
            Some(2)
        );
        assert_eq!(collector.span(30), None);
    }

    #[test]
    fn a_focusable_widget_joins_the_tab_order() {
        let mut collector = SceneCollector::new(1);
        let output = WidgetOutput {
            focusable: true,
            ..WidgetOutput::default()
        };
        collector.push(10, output);
        assert_eq!(collector.focus_order, vec![10]);
    }

    /// 构造带圆角半径的裁剪节点，采用着色器圆角槽位使用的 `(centre, half-extent)` 形式。
    fn rounded_clip(rect: Rect, radius: f32) -> GpuClipNode {
        let mut node = GpuClipNode::from_aabb(rect.to_min_max_array());
        node.rounded_clip_0 = rect.to_center_half();
        node.radii_0 = [f32_to_f16(radius); 4];
        node
    }

    #[test]
    fn a_third_nested_rounded_clip_is_folded_not_dropped() {
        let mut collector = SceneCollector::new(1);
        let outer = Rect::new(0.0, 0.0, 200.0, 200.0);
        let middle = Rect::new(10.0, 10.0, 180.0, 180.0);
        let inner = Rect::new(20.0, 20.0, 160.0, 160.0);

        collector.push_clip(rounded_clip(outer, 40.0), outer);
        collector.push_clip(rounded_clip(middle, 16.0), middle);
        let index = collector.push_clip(rounded_clip(inner, 8.0), inner);

        let node = collector.clips[index as usize];
        // 最内层容器在槽位 0 中保留自身圆角半径。
        assert_eq!(node.rounded_clip_0, inner.to_center_half());
        // 中间层容器保留自身槽位，但其圆角现在也受外层圆弧约束：
        // 中间层位于 40px 圆弧内缩 10px 处，因此有效半径从 16px 增至 30px。
        assert_eq!(node.rounded_clip_1, middle.to_center_half());
        let folded = f16_to_f32(node.radii_1[0]);
        assert!(
            (folded - 30.0).abs() < 0.05,
            "outer arc must raise the kept radius: {folded}"
        );
        assert!(
            folded < 40.0,
            "the folded radius never exceeds the outer one"
        );
    }

    /// 每个 widget 从零索引自己的 payload 切片，而收集器持有拼接后的
    /// 全局表。第二个标签绝不能采样到第一个标签的字形 UV。
    #[test]
    fn payload_indices_are_rebased_for_every_widget() {
        let widget = |indices: &[u32]| WidgetOutput {
            primitives: indices
                .iter()
                .map(|&index| GpuPrimitiveInstance {
                    extra_payload_idx: index,
                    ..Default::default()
                })
                .collect(),
            payloads: indices.iter().map(|_| GpuPayload::default()).collect(),
            ..Default::default()
        };

        let mut collector = SceneCollector::new(0);
        collector.push(1, widget(&[0, 1]));
        collector.push(2, widget(&[0, 1, 2]));

        assert_eq!(collector.payloads.len(), 5);
        let indices: Vec<u32> = collector
            .primitives
            .iter()
            .map(|p| p.extra_payload_idx)
            .collect();
        assert_eq!(indices, vec![0, 1, 2, 3, 4]);
    }
}
