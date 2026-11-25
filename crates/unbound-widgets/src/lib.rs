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

// 基础设施模块：场景差分、图元打包辅助工具与默认标题栏描述符。它们没有独立的
// `emit_*` 发射器，不属于业务组件，因此不纳入展示目录。
pub mod canonical;
pub mod catalog;
pub mod column_header_bar;
pub mod diff;
pub mod docking_overlay;
pub mod ergonomics;
pub mod primitive;
pub mod shell;
pub mod slot_fixture;
pub mod title_bar;

pub use slot_fixture::{
    apply_recursive_disabled_propagation, emit_canonical_annotated_scroll_bar_fixture,
    emit_canonical_app_bar_button_fixture, emit_canonical_app_bar_toggle_button_fixture,
    emit_canonical_command_bar_fixture, emit_canonical_expander_fixture,
    emit_canonical_flip_view_fixture, emit_canonical_flyout_fixture,
    emit_canonical_font_icon_fixture, emit_canonical_grid_fixture, emit_canonical_image_fixture,
    emit_canonical_info_bar_fixture, emit_canonical_list_rows_fixture,
    emit_canonical_menu_bar_fixture, emit_canonical_menu_flyout_fixture,
    emit_canonical_path_fixture, emit_canonical_person_picture_fixture,
    emit_canonical_pips_pager_fixture, emit_canonical_pivot_fixture,
    emit_canonical_refresh_fixture, emit_canonical_segmented_fixture,
    emit_canonical_split_view_fixture, emit_canonical_tab_view_fixture,
    emit_canonical_text_rows_fixture, emit_canonical_time_picker_fixture,
    emit_canonical_title_bar_fixture, emit_canonical_tree_fixture, remap_token_to_disabled,
    AnnotatedScrollBarFixtureOutput, AppBarButtonFixtureOutput, AppBarToggleButtonFixtureOutput,
    CommandBarFixtureOutput, ExpanderFixtureOutput, FixtureState, FlipViewFixtureOutput,
    FlyoutFixtureOutput, FontIconFixtureOutput, GridFixtureOutput, ImageFixtureOutput,
    InfoBarFixtureOutput, ListRowsFixtureOutput, MenuBarFixtureOutput, MenuFlyoutFixtureOutput,
    PathFixtureOutput, PersonPictureFixtureOutput, PipsPagerFixtureOutput, PivotFixtureOutput,
    RefreshFixtureOutput, SplitViewFixtureOutput, TabViewFixtureOutput, TextRowsFixtureOutput,
    TimePickerFixtureOutput, TitleBarFixtureOutput, TreeFixtureOutput,
    CANONICAL_APP_BAR_BUTTON_GLYPH, CANONICAL_APP_BAR_BUTTON_HEIGHT,
    CANONICAL_APP_BAR_BUTTON_LABEL, CANONICAL_APP_BAR_BUTTON_WIDTH,
    CANONICAL_APP_BAR_TOGGLE_BUTTON_GLYPH, CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT,
    CANONICAL_APP_BAR_TOGGLE_BUTTON_LABEL, CANONICAL_APP_BAR_TOGGLE_BUTTON_WIDTH,
    CANONICAL_COMMAND_ITEMS, CANONICAL_COMMAND_MORE_GLYPH, CANONICAL_EXPANDER_BODY,
    CANONICAL_EXPANDER_CHEVRON_DOWN, CANONICAL_EXPANDER_CHEVRON_UP,
    CANONICAL_EXPANDER_CONTENT_HEIGHT, CANONICAL_EXPANDER_HEADER, CANONICAL_EXPANDER_HEADER_HEIGHT,
    CANONICAL_EXPANDER_WIDTH, CANONICAL_FLIP_PAGES, CANONICAL_FLYOUT_HEIGHT, CANONICAL_FLYOUT_TEXT,
    CANONICAL_FLYOUT_WIDTH, CANONICAL_FONT_ICON_FONT_SIZE, CANONICAL_FONT_ICON_GLYPH,
    CANONICAL_FONT_ICON_SIZE, CANONICAL_GRID_ITEMS, CANONICAL_IMAGE_SIZE,
    CANONICAL_INFOBAR_CLOSE_GLYPH, CANONICAL_INFOBAR_HEIGHT, CANONICAL_INFOBAR_ICON_GLYPH,
    CANONICAL_INFOBAR_MESSAGE, CANONICAL_INFOBAR_TITLE, CANONICAL_INFOBAR_WIDTH,
    CANONICAL_LIST_ITEMS, CANONICAL_MENU_FLYOUT_ITEMS, CANONICAL_MENU_FLYOUT_ITEM_HEIGHT,
    CANONICAL_MENU_FLYOUT_WIDTH, CANONICAL_MENU_ITEMS, CANONICAL_PATH_ITEMS,
    CANONICAL_PATH_SEPARATOR, CANONICAL_PERSON_PICTURE_INITIALS, CANONICAL_PERSON_PICTURE_SIZE,
    CANONICAL_PIPS_COUNT, CANONICAL_PIPS_SELECTED_INDEX, CANONICAL_PIP_SIZE_NORMAL,
    CANONICAL_PIP_SIZE_SELECTED, CANONICAL_PIP_SPACING, CANONICAL_PIVOT_CONTENT,
    CANONICAL_PIVOT_HEADER_HEIGHT, CANONICAL_PIVOT_HEIGHT, CANONICAL_PIVOT_ITEMS,
    CANONICAL_PIVOT_WIDTH, CANONICAL_REFRESH_TEXT, CANONICAL_ROW_ITEMS, CANONICAL_SEGMENTED_ITEMS,
    CANONICAL_SPLIT_VIEW_CONTENT_TEXT, CANONICAL_SPLIT_VIEW_HEIGHT, CANONICAL_SPLIT_VIEW_PANE_TEXT,
    CANONICAL_SPLIT_VIEW_PANE_WIDTH, CANONICAL_SPLIT_VIEW_WIDTH, CANONICAL_TAB_CLOSE_GLYPH,
    CANONICAL_TAB_CONTENT, CANONICAL_TAB_ITEMS, CANONICAL_TIME_COLUMNS,
    CANONICAL_TIME_PICKER_HEIGHT, CANONICAL_TIME_PICKER_WIDTH, CANONICAL_TITLE_BAR_SUBTITLE,
    CANONICAL_TITLE_BAR_TITLE, CANONICAL_TREE_CHILD, CANONICAL_TREE_ROOT, STANDARD_FLIP_HEIGHT,
    STANDARD_FLIP_WIDTH, STANDARD_FONT_SIZE, STANDARD_GRID_CELL_RADIUS, STANDARD_GRID_HEIGHT,
    STANDARD_GRID_WIDTH, STANDARD_LIST_ITEM_HEIGHT, STANDARD_PILL_HEIGHT, STANDARD_PILL_RADIUS,
    STANDARD_PILL_WIDTH, STANDARD_REFRESH_HEIGHT, STANDARD_REFRESH_INDICATOR_SIZE,
    STANDARD_REFRESH_WIDTH, STANDARD_SCROLL_BAR_HEIGHT, STANDARD_SCROLL_BAR_TRACK_WIDTH,
    STANDARD_SCROLL_BAR_WIDTH, STANDARD_STACK_SPACING, STANDARD_TREE_INDENT,
    STANDARD_TREE_ITEM_HEIGHT, WINUI_DISABLED_OPACITY,
};

pub use column_header_bar::{
    column_header_sub_node, compute_column_header_layout, emit_column_header_bar,
    emit_column_header_bar_chrome, emit_column_header_bar_text_layer, ColumnHeaderAlign,
    ColumnHeaderBarDesc, ColumnHeaderBarLayout, ColumnHeaderBarState, ColumnHeaderCellLayout,
    ColumnHeaderEvent, ColumnHeaderHitKind, ColumnHeaderSpec, ColumnResizeDrag,
    COLUMN_DEFAULT_MAX_WIDTH, COLUMN_DEFAULT_MIN_WIDTH, COLUMN_DIVIDER_WIDTH,
    COLUMN_FILTER_BUTTON_WIDTH, COLUMN_HEADER_BAR_HEIGHT, COLUMN_RESIZE_HANDLE_WIDTH,
};
pub use shell::{
    emit_brand_badge, emit_feature_card, emit_hero_backdrop, emit_icon_button,
    emit_shell_icon_primitives, emit_widget_icon_primitives, AppBrandBadgeDesc, FeatureCardDesc,
    HeroBackdropDesc, IconButtonDesc, ShellIcon, ShellIconDesc, WidgetIcon,
};
pub use title_bar::{
    emit_window_caption_buttons, TitleBarChrome, TitleBarDesc, WindowCaptionButton,
    WindowCaptionButtonControlStates, WindowCaptionButtonEntry, WindowCaptionButtons,
    WindowCaptionButtonsLayout, WindowCaptionButtonsOutput, WindowCaptionFrameHitContract,
    WindowCaptionKind, TITLE_BAR_CAPTION_WIDTH, TITLE_BAR_CLOSE_ID, TITLE_BAR_DRAG_ID,
    TITLE_BAR_FUSED_HEIGHT, TITLE_BAR_HEIGHT, TITLE_BAR_MAXIMIZE_ID, TITLE_BAR_MINIMIZE_ID,
    TITLE_BAR_MIN_DRAG_WIDTH,
};

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
        address_bar,
        alert,
        alert_dialog,
        autocomplete,
        avatar,
        badge,
        banner,
        breadcrumb,
        button,
        calendar_date_picker,
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
        color_swatch,
        combobox,
        command_palette,
        containers,
        context_menu,
        curve_editor,
        data_grid,
        date_picker,
        date_picker_wheel,
        date_range_picker,
        descriptions,
        divider,
        dock,
        drawer,
        drop_down_button,
        file_drop,
        form,
        generic_data_table,
        gizmo,
        hotkey_input,
        hover_card,
        hyperlink_button,
        image,
        masonry,
        menu,
        menubar,
        minimap,
        modal,
        multi_split,
        navigation_view,
        node_graph,
        notification_center,
        number_input,
        overlay,
        pagination,
        password_box,
        pin_input,
        pipeline_view,
        pivot,
        popconfirm,
        popover,
        progress,
        property_grid,
        query_builder,
        radio,
        rating,
        repeat_button,
        rich_text,
        rich_text_line,
        ruler,
        schema_form,
        scroll,
        segmented,
        select,
        shortcut_bar,
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
        task_hud,
        terminal_view,
        text_area,
        text_input,
        time_picker,
        timeline,
        timeline_track,
        toast,
        toggle_button,
        toggle_split_button,
        toolbar,
        tooltip,
        tour,
        transfer,
        tree_table,
        tree_view,
        virtual_masonry,
        virtual_views,
        watermark,
        annotated_scroll_bar,
        app_bar,
        blade_view,
        command_bar_flyout,
        grid_view,
        headered_content,
        image_cropper,
        items_view,
        list_box,
        orbit_view,
        radial_gauge,
        range_selector,
        refresh_container,
        rich_edit_box,
        swipe_control,
        teaching_tip,
        scale_osd,
        inline_editable_text,
        elided_text,
        copy_button,
        duration_input,
        preset_number_input,
        kbd,
        virtual_grid,
        viewport_3d,
    ],
    catalog: [
        ExpanderGroup => accordion,
        AddressBar => address_bar,
        InfoBar => alert,
        AlertDialog => alert_dialog,
        AspectRatioBox => masonry,
        AutoSuggestBox => autocomplete,
        PersonPicture => avatar,
        AvatarGroup => avatar,
        InfoBadge => badge,
        Banner => banner,
        BreadcrumbBar => breadcrumb,
        Button => button,
        CalendarDatePicker => calendar_date_picker,
        Callout => callout,
        CanvasToolbar => canvas_toolbar,
        FlipView => carousel,
        Cascader => cascader,
        Chart => chart,
        CheckBox => checkbox,
        Chip => badge,
        CodeBlock => code_block,
        CodeEditor => code_editor,
        Expander => collapsible,
        ColorPicker => color_picker,
        ColorSwatch => color_swatch,
        ComboBoxEditable => combobox,
        CommandPalette => command_palette,
        Containers => containers,
        ContextMenu => context_menu,
        CurveEditor => curve_editor,
        DataGrid => data_grid,
        CalendarView => date_picker,
        DatePicker => date_picker_wheel,
        DateRangePicker => date_range_picker,
        Descriptions => descriptions,
        Divider => divider,
        Dock => dock,
        SplitView => drawer,
        DropDownButton => drop_down_button,
        EmptyState => hover_card,
        FileDrop => file_drop,
        Form => form,
        GenericDataTable => generic_data_table,
        Gizmo => gizmo,
        GradientEditor => curve_editor,
        HotkeyInput => hotkey_input,
        HoverCard => hover_card,
        HyperlinkButton => hyperlink_button,
        FontIcon => image,
        Image => image,
        LoadingOverlay => spinner,
        Masonry => masonry,
        MenuFlyout => menu,
        MenuBar => menubar,
        Minimap => minimap,
        ContentDialog => modal,
        MultiSplit => multi_split,
        NinePatch => image,
        NodeGraph => node_graph,
        NotificationCenter => notification_center,
        NumberBox => number_input,
        Overlay => overlay,
        PipsPager => pagination,
        PasswordBox => password_box,
        PinInput => pin_input,
        PipelineView => pipeline_view,
        Pivot => pivot,
        Popconfirm => popconfirm,
        Flyout => popover,
        ProgressBar => progress,
        PropertyGrid => property_grid,
        QueryBuilder => query_builder,
        RadioButton => radio,
        RatingControl => rating,
        RepeatButton => repeat_button,
        RichTextBlock => rich_text,
        RichTextLine => rich_text_line,
        Ruler => ruler,
        SchemaForm => schema_form,
        ScrollView => scroll,
        SelectorBar => segmented,
        ComboBox => select,
        ShortcutBar => shortcut_bar,
        NavigationView => navigation_view,
        Skeleton => skeleton,
        Slider => slider,
        ProgressRing => spinner,
        SplitButton => split_button,
        SplitPane => split_pane,
        StackPanel => containers,
        StatusBar => toolbar,
        Stepper => stepper,
        ToggleSwitch => switch,
        Table => table,
        TabView => tabs,
        TagInput => tag_input,
        TaskHud => task_hud,
        TerminalView => terminal_view,
        TextBoxMultiline => text_area,
        TextBox => text_input,
        TimePicker => time_picker,
        Timeline => timeline,
        TimelineTrack => timeline_track,
        Toast => toast,
        ToggleButton => toggle_button,
        ToggleSplitButton => toggle_split_button,
        CommandBar => toolbar,
        ToolTip => tooltip,
        Tour => tour,
        Transfer => transfer,
        TreeTable => tree_table,
        TreeView => tree_view,
        VirtualMasonry => virtual_masonry,
        ListView => virtual_views,
        Watermark => watermark,
        Wrap => containers,
        AnnotatedScrollBar => annotated_scroll_bar,
        AppBarButton => app_bar,
        AppBarSeparator => app_bar,
        AppBarToggleButton => app_bar,
        BladeView => blade_view,
        CommandBarFlyout => command_bar_flyout,
        GridView => grid_view,
        HeaderedContentControl => headered_content,
        ImageCropper => image_cropper,
        ItemsView => items_view,
        ListBox => list_box,
        OrbitView => orbit_view,
        RadialGauge => radial_gauge,
        RangeSelector => range_selector,
        RefreshContainer => refresh_container,
        RichEditBox => rich_edit_box,
        SwipeControl => swipe_control,
        TeachingTip => teaching_tip,
        Viewport3D => viewport_3d,
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

use rustc_hash::{FxHashMap, FxHashSet};
use unbound_a11y::{A11yNode, Semantics};
use unbound_gpu::data::{
    GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform, CLIP_NONE, TRANSFORM_ANIMATED_BIT,
    TRANSFORM_INDEX_MASK,
};
use unbound_headless::driver::{ClickReport, TestApp};
use unbound_input::{ClipRect, HitRegion, PointerEvent, Router};
use unbound_math::{f16_to_f32, f32_to_f16, project_rounded_radii, Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_world::{NodeId, UiWorld};

pub use address_bar::{
    address_bar_sub_node, compute_layout, compute_text_edit_visual, emit_address_bar,
    emit_address_bar_chrome, emit_address_bar_dropdown_text_layer, emit_address_bar_text_layer,
    emit_address_bar_with_dropdown, emit_address_bar_with_visual, AddressBarChromeStyle,
    AddressBarDesc, AddressBarDropdownLayout, AddressBarDropdownMode, AddressBarEvent,
    AddressBarHitKind, AddressBarLayout, AddressBarState, AddressBarTextVisual, PathSegment,
    SearchIconPlacement, SegmentLayout, ADDRESS_BAR_FONT_SIZE, ADDRESS_BAR_HEIGHT,
};
pub use copy_button::{
    emit_copy_button, emit_read_only_input, read_only_copy_node, CopyButtonDesc, CopyButtonState,
    ReadOnlyInputDesc, ReadOnlyInputState, COPY_FEEDBACK_DURATION_MS,
};
pub use duration_input::{
    duration_spin_down_node, duration_spin_up_node, emit_duration_input, format_duration_ms,
    parse_duration_ms, DurationInputDesc, DurationInputState, DurationUnit,
};
pub use elided_text::{
    elide_text_with_measure, emit_elided_text, ElideMode, ElideResult, ElidedTextDesc,
    ElidedTextState, ELLIPSIS,
};
pub use generic_data_table::{
    emit_generic_data_table, DataColumnSpec, GenericDataTableDesc, GenericDataTableState,
    SortDirection,
};
pub use inline_editable_text::{
    emit_inline_editable_text, BlurBehavior, EditTrigger, InlineEditEvent, InlineEditableTextDesc,
    InlineEditableTextState,
};
pub use kbd::{
    emit_kbd, emit_kbd_chord, key_labels_from_chord, normalize_key_label, KbdChordDesc, KbdDesc,
    KbdSize,
};
pub use pipeline_view::{
    emit_pipeline_view, PipelineStageItem, PipelineStageStatus, PipelineSubStep, PipelineViewDesc,
    PipelineViewState,
};
pub use pivot::{
    emit_pivot_item, PivotDesc, PivotItem, PivotState, PIVOT_GAP, PIVOT_HEIGHT,
    PIVOT_INDICATOR_HEIGHT,
};
pub use preset_number_input::{
    emit_preset_number_input, emit_preset_number_popup, preset_dropdown_node, preset_item_node,
    NumberPreset, PresetDisplayMode, PresetNumberInputDesc, PresetNumberInputState,
};
pub use query_builder::{
    emit_query_builder, evaluate_rule, FilterPopupState, QueryBuilderDesc, QueryBuilderState,
    QueryCombinator, QueryGroup, QueryRule,
};
pub use scale_osd::{
    emit_scale_osd, scale_osd_reset_node, scale_osd_zoom_in_node, scale_osd_zoom_out_node,
    ScaleOsdAnchor, ScaleOsdDesc, ScaleOsdState, SCALE_OSD_DEFAULT_TIMEOUT_MS, SCALE_OSD_HEIGHT,
    SCALE_OSD_MARGIN, SCALE_OSD_WIDTH,
};
pub use schema_form::{
    emit_schema_form, estimate_label_text_width, schema_form_trailing_action_node,
    FieldInheritanceState, SchemaFieldKind, SchemaFieldSpec, SchemaFormDesc, SchemaFormEvent,
    SchemaFormRowLayout, SchemaFormState, SchemaLabelColumnConfig, SchemaValue,
    DEFAULT_LABEL_MAX_RATIO, DEFAULT_LABEL_MAX_WIDTH, DEFAULT_LABEL_MIN_WIDTH,
    DEFAULT_TRAILING_ACTION_WIDTH, LABEL_CONTROL_GAP, LABEL_FONT_SIZE, TRAILING_ACTION_GAP,
};
pub use shortcut_bar::{
    emit_shortcut_bar, BindingOrigin, KeybindingManagerState, ShortcutBarDesc, ShortcutEntry,
};
pub use task_hud::{emit_task_hud, TaskActivityItem, TaskHudDesc, TaskHudState, TaskStatus};
pub use virtual_grid::{
    emit_virtual_grid, emit_virtual_grid_scoped, virtual_grid_cell_node, GridColumnSizing,
    VirtualGridDesc, VirtualGridState, VirtualGridWindow,
};

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
pub use callout::{emit_callout, emit_callout_with_glyphs, CalloutDesc, CalloutState};
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
    emit_code_block, highlight_line, CodeBlockDesc, CodeBlockState, CodeLanguage, CodeSpan,
    SyntaxToken, CODE_GUTTER, CODE_LINE_HEIGHT, COPY_CONFIRM_SECONDS,
};
pub use code_editor::{emit_code_editor, CodeEditorDesc, CodeEditorState};
pub use collapsible::{emit_collapsible, CollapsibleDesc, CollapsibleState};
pub use color_picker::{
    emit_color_picker, parse_hex, parse_oklch, parse_rgba, ColorFormat, ColorPart, ColorPickerDesc,
    ColorPickerLayout, ColorPickerState, Hsva, ThemeSettingsState,
};
pub use color_swatch::{
    emit_color_swatch, ColorSwatchDesc, SWATCH_GAP, SWATCH_RADIUS, SWATCH_SIZE,
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
    emit_border, emit_card, emit_stack, CardDesc, CardTokens, StackDesc, WrapDesc,
    CARD_BORDER_WIDTH, CARD_ELEVATIONS, CARD_RADIUS, WRAP_FLAG,
};
pub use context_menu::{ContextClose, ContextKey, ContextMenuState};
pub use curve_editor::{
    emit_curve_editor, emit_gradient_editor, CurveEditorDesc, CurveEditorState, CurveHandle,
    GradientEditorState, GradientStop,
};
pub use data_grid::{emit_data_grid, DataGridDesc, DataGridState};
pub use date_picker::{
    calendar_cell_node, calendar_header_node, calendar_next_node, calendar_prev_node,
    days_in_month, emit_calendar, emit_calendar_view, is_leap_year, CalendarCell,
    CalendarDisplayMode, CalendarHitTarget, CalendarSelectionMode, CalendarState, DatePickerDesc,
    NaiveDate, SelectionMode, YearMonth, CALENDAR_CHEVRON_DOWN_GLYPH, CALENDAR_CHEVRON_UP_GLYPH,
};
pub use date_range_picker::{
    emit_date_range_picker, DateRange, DateRangePickerDesc, DateRangePickerState,
};
pub use descriptions::{
    emit_descriptions, emit_descriptions_with_glyphs, DescriptionItem, DescriptionsDesc,
    DescriptionsState,
};
pub use divider::{emit_divider, DividerDesc, DividerOrientation, DividerTokens};
pub use dock::{
    emit_dock_pane, emit_dock_preview, emit_dock_tab, DockDrag, DockHover, DockNode, DockPane,
    DockState, DockTab, DockTokens, DockTree, DockZone,
};
pub use docking_overlay::{
    clamp_rect_to_bounds, elide_hint_label, emit_docking_overlay, emit_drag_proxy,
    layout_hint_pill, resolve_candidates, DockCandidatePane, DockCandidateWindow, DockDragProxy,
    DockDragSource, DockDropKind, DockDropSemantic, DockDropTarget, DockEdge, DockOverlayState,
    DockOverlayTokens, ALLOWED_FILL_ALPHA, BLOCKED_FILL_ALPHA, HINT_PILL_HEIGHT,
    HINT_PILL_MAX_WIDTH, HINT_PILL_OFFSET, NEST_FILL_ALPHA, PREVIEW_BORDER_WIDTH,
    REORDER_INDICATOR_WIDTH, ROOT_EDGE_BAND_PX, SHELL_DWELL_SECS, SHELL_DWELL_TOLERANCE_PX,
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
    emit_gizmo, grip_rect_for, grip_rect_rotated, GizmoDesc, GizmoOutput, GRIP_SIZE, ROTATE_GRIP,
    ROTATE_OFFSET, SIZE_BUBBLE,
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
pub use menu::{
    emit_menu, emit_menu_with_rows, menu_item_node, MenuDesc, MenuItem, MenuItemRenderRow,
    MenuState, MenuWidget, CONTEXT_MENU_ITEM_HEIGHT, MENU_ACRYLIC_SURFACE_OPACITY,
    MENU_BACKDROP_BLUR, MENU_ICON_SIZE, MENU_ITEM_HEIGHT, MENU_ITEM_RADIUS, MENU_PANEL_PADDING,
    MENU_RADIUS, MENU_SEPARATOR_HEIGHT,
};
pub use menubar::{
    emit_menubar, emit_menubar_item, reduce_menubar_key, MenuBarDesc, MenuBarItem, MenuBarKey,
    MenuBarState, MenuBarTokens,
};
pub use minimap::{emit_minimap, MinimapDesc, MinimapItem, MinimapState, MINIMAP_INSET};
pub use modal::{emit_modal, DraggableModalState, ModalDesc, ModalOutput, ModalWidget};
pub use multi_split::{emit_multi_split, MultiSplitDesc, MultiSplitState, MULTI_SPLIT_GUTTER_SIZE};
pub use navigation_view::{
    emit_navigation_view, navigation_view_row_node, NavigationViewBackButtonVisible,
    NavigationViewDesc, NavigationViewDisplayMode, NavigationViewEvent, NavigationViewIcon,
    NavigationViewItem, NavigationViewItemHeader, NavigationViewItemSeparator,
    NavigationViewLayout, NavigationViewMenuItem, NavigationViewOutput,
    NavigationViewOverflowLabelMode, NavigationViewPaneDisplayMode, NavigationViewRow,
    NavigationViewRowKind, NavigationViewState, NavigationViewTitleBarSync,
    NAVIGATION_VIEW_BACK_BUTTON_ID, NAVIGATION_VIEW_BADGE_HEIGHT, NAVIGATION_VIEW_BADGE_WIDTH,
    NAVIGATION_VIEW_CHILD_INDENT, NAVIGATION_VIEW_COMPACT_PANE_LENGTH,
    NAVIGATION_VIEW_COMPACT_THRESHOLD_WIDTH, NAVIGATION_VIEW_CONTENT_CORNER_RADIUS,
    NAVIGATION_VIEW_EXPANDED_THRESHOLD_WIDTH, NAVIGATION_VIEW_HEADER_HEIGHT,
    NAVIGATION_VIEW_INDICATOR_HEIGHT, NAVIGATION_VIEW_INDICATOR_RADIUS,
    NAVIGATION_VIEW_INDICATOR_WIDTH, NAVIGATION_VIEW_ITEM_HEIGHT, NAVIGATION_VIEW_ITEM_MARGIN_Y,
    NAVIGATION_VIEW_OPEN_PANE_LENGTH, NAVIGATION_VIEW_OVERFLOW_BUTTON_ID,
    NAVIGATION_VIEW_OVERLAY_BLUR, NAVIGATION_VIEW_PANE_PADDING, NAVIGATION_VIEW_PANE_TOGGLE_ID,
    NAVIGATION_VIEW_SEARCH_BUTTON_ID, NAVIGATION_VIEW_SECTION_HEADER_HEIGHT,
    NAVIGATION_VIEW_SEPARATOR_HEIGHT, NAVIGATION_VIEW_SETTINGS_ID,
    NAVIGATION_VIEW_TOP_INDICATOR_HEIGHT, NAVIGATION_VIEW_TOP_INDICATOR_WIDTH,
    NAVIGATION_VIEW_TOP_PANE_HEIGHT,
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
    place_anchored_popup, place_combo_popup, place_tooltip_in_window, BackdropRequest,
    OverlayEntry, OverlayKind, OverlayStack, PlacedPopup, Placement, TOOLTIP_VIEWPORT_MARGIN,
};
pub use pagination::{emit_pagination, PaginationDesc, PaginationState, PaginationStyle};
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
pub use rating::{
    emit_rating, render_rating_with_text, RatingDesc, RatingState, RATING_STAR_FILLED_GLYPH,
    RATING_STAR_OUTLINE_GLYPH,
};
pub use rich_text::{
    emit_rich_text, parse_markdown, BlockKind, RichBlock, RichDocument, RichTextDesc,
    HEADING_SCALE, RICH_TEXT_SIZE,
};
pub use rich_text_line::{
    emit_rich_text_line, ColoredGlyphQuad, RichTextLineDesc, SpanDecoration,
    RICH_TEXT_LINE_DEFAULT_SIZE,
};
pub use ruler::{
    emit_rulers, nice_step, GuideAxis, GuideLine, RulerDesc, RulerOutput, RulerState, RULER_BAND,
    RULER_MINOR_PER_MAJOR, RULER_MIN_STEP,
};
pub use scroll::{
    emit_scroll_view, emit_scrollbar, scrollbar_thumb_node, scrollbar_track_node,
    update_virtual_list, ScrollAxis, ScrollBarDesc, ScrollBarOutput, ScrollBarVisibility,
    ScrollPointerEvent, ScrollState, ScrollViewDesc, ScrollViewOutput, ScrollViewWidget,
    ScrollbarAxis, SmoothWheelHandler, VirtualWindow, SCROLLBAR_EDGE_INSET,
    SCROLLBAR_EXPANDED_OPACITY, SCROLLBAR_HORIZONTAL_THUMB_HIT_MASK,
    SCROLLBAR_HORIZONTAL_TRACK_HIT_MASK, SCROLLBAR_IDLE_OPACITY, SCROLLBAR_THUMB_IDLE_WIDTH,
    SCROLLBAR_THUMB_WIDTH, SCROLLBAR_TRACK_CAPSULE_OPACITY, SCROLLBAR_VERTICAL_THUMB_HIT_MASK,
    SCROLLBAR_VERTICAL_TRACK_HIT_MASK, SCROLLBAR_WIDTH,
};
pub use segmented::{SegmentedDesc, SegmentedTokens};
pub use select::{emit_select, SelectDesc, SelectState, SelectWidget};
pub use skeleton::{emit_skeleton, SkeletonDesc, SkeletonState, SKELETON_PERIOD, SKELETON_SHIMMER};
pub use slider::{SliderDesc, SliderTokens};
pub use spinner::{emit_loading_overlay, emit_spinner, LoadingOverlayDesc, SpinnerDesc};
pub use split_button::{
    emit_split_button, emit_split_button_with_glyphs, SplitButtonDesc, SplitButtonState,
    DROPDOWN_WIDTH,
};
pub use split_pane::{
    emit_split_pane, emit_split_pane_panes, reduce_split_key, SplitKey, SplitLayout, SplitPaneDesc,
    SplitPaneState, SplitPaneTokens, SplitSide,
};
pub use stepper::{emit_stepper, StepItem, StepStatus, StepperDesc, StepperOrientation};
pub use switch::{SwitchDesc, SwitchLayout, SwitchTokens};
pub use table::{
    emit_table, AlignX as TableAlignX, ColumnSpec, RowWindow, SortDir, TableDesc, TableRow,
    TableState, TableTokens,
};
pub use tabs::{
    emit_tab, emit_tab_close, emit_tabs_add, emit_tabs_strip, emit_tabs_with_control_states,
    reduce_tabs_key, tab_add_node, tab_close_node, tab_item_node, TabItem, TabViewEvent,
    TabViewStyle, TabWidthMode, TabsDesc, TabsKey, TabsState, TabsTokens, TAB_ADD_GAP,
    TAB_ADD_HEIGHT, TAB_ADD_WIDTH, TAB_CLOSE, TAB_CLOSE_HIT_SIZE, TAB_MAX_WIDTH, TAB_MIN_WIDTH,
    TAB_SHELL_CORNER_RADIUS, TAB_SHELL_ITEM_HEIGHT, TAB_SHELL_TOP_INSET, TAB_STRIP_HEIGHT,
};
pub use tag_input::{
    emit_tag_input, layout_tags, TagInputDesc, TagInputOutcome, TagInputState, TagKey, TagLayout,
    TAG_CLOSE, TAG_GAP, TAG_HEIGHT, TAG_INSET, TAG_PADDING, TAG_RADIUS,
};
pub use terminal_view::{
    emit_terminal_view, emit_terminal_view_with_glyphs, TerminalViewDesc, TerminalViewState,
    TERMINAL_LINE_HEIGHT,
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
pub use toggle_button::{
    emit_toggle_button, emit_toggle_button_with_glyphs, ToggleButtonDesc, ToggleButtonState,
};
pub use toolbar::{
    emit_status_bar, emit_status_segment, emit_toolbar, emit_toolbar_item, emit_toolbar_overflow,
    StatusAlign, StatusBarDesc, StatusSegment, ToolbarDesc, ToolbarItem, ToolbarItemKind,
    ToolbarLayout, ToolbarState, ToolbarTokens,
};
pub use tooltip::{
    emit_tooltip, wrap_text, TooltipDelay, TooltipDesc, TooltipLayout, TooltipPadding,
    TooltipState, TooltipWidget, TOOLTIP_BORDER_WIDTH, TOOLTIP_DELAY_MS, TOOLTIP_FONT_SIZE,
    TOOLTIP_LINE_HEIGHT, TOOLTIP_MAX_WIDTH, TOOLTIP_PADDING, TOOLTIP_RADIUS, TOOLTIP_SHADOW,
    TOOLTIP_WARMUP_MS,
};
pub use tour::{emit_tour, TourDesc, TourState, TourStep};
pub use transfer::{emit_transfer, TransferDesc, TransferItem, TransferState};
pub use tree_table::{
    emit_tree_table, emit_tree_table_with_glyphs, TreeTableDesc, TreeTableRow, TreeTableState,
    TREE_TABLE_HEADER_HEIGHT, TREE_TABLE_ROW_HEIGHT,
};
pub use tree_view::{
    emit_tree_background, emit_tree_row, emit_tree_row_divider, emit_tree_row_with_icon,
    emit_tree_row_with_options, emit_tree_scrollbar, tree_row_label_max_width, tree_row_label_x,
    tree_row_trailing_icon_rect, tree_toggle_node, TreeRowIcon, TreeRowOptions, TreeViewDensity,
    TreeViewDesc, TreeViewEvent, TreeViewRow, TreeViewState, TreeViewTokens, TREE_DIVIDER_HEIGHT,
    TREE_FONT_SIZE, TREE_ICON_GAP, TREE_ICON_SIZE, TREE_ROW_COMPACT, TREE_TRAILING_ICON_SIZE,
};
pub use virtual_masonry::{emit_virtual_masonry, VirtualMasonryDesc, VirtualMasonryState};
pub use virtual_views::{
    emit_virtual_row, ReorderableListState, VariableVirtualList, VariableWindow, VirtualRowTokens,
};
pub use watermark::{emit_watermark, WatermarkDesc};

pub use calendar_date_picker::{
    calendar_icon_node, emit_calendar_date_picker, CalendarDatePickerDesc, CalendarDatePickerState,
    CALENDAR_DATE_PICKER_HEIGHT,
};
pub use date_picker_wheel::{
    emit_date_picker_wheel, DatePickerColumnOrder, DatePickerMonthFormat, DatePickerWheelDesc,
    DatePickerWheelState, WheelColumn, DATE_PICKER_WHEEL_TRIGGER_HEIGHT,
};
pub use drop_down_button::{
    emit_drop_down_button, DropDownButtonDesc, DropDownButtonKey, DropDownButtonState,
    DROP_DOWN_BUTTON_CHEVRON_INSET, DROP_DOWN_BUTTON_HEIGHT, DROP_DOWN_BUTTON_RADIUS,
};
pub use hyperlink_button::{
    emit_hyperlink_button, HyperlinkButtonDesc, HYPERLINK_BUTTON_HEIGHT, HYPERLINK_RADIUS,
};
pub use password_box::{
    emit_password_box, reveal_button_node, PasswordBoxDesc, PasswordBoxState, MASK_CHAR,
};
pub use repeat_button::{
    emit_repeat_button, RepeatButtonDesc, RepeatButtonState, DEFAULT_REPEAT_DELAY_MS,
    DEFAULT_REPEAT_INTERVAL_MS, REPEAT_BUTTON_HEIGHT, REPEAT_BUTTON_RADIUS,
};
pub use toggle_split_button::{
    emit_toggle_split_button, ToggleSplitButtonDesc, ToggleSplitButtonState, ToggleSplitKey,
    ToggleSplitPart, TOGGLE_SPLIT_DROPDOWN_WIDTH, TOGGLE_SPLIT_HEIGHT, TOGGLE_SPLIT_RADIUS,
};

pub use canonical::{
    ControlCategory, ControlClass, OverflowStrategy, WidgetCapabilities, BASELINE_COUNT,
    EXCLUSIVE_COUNT, TOOLKIT_COUNT,
};

// 扩展控件（P1 / P2）：WinUI 3 官方核心控件与 Community Toolkit 选补控件。
pub use annotated_scroll_bar::{emit_annotated_scroll_bar, AnnotatedScrollBarDesc};
pub use app_bar::{
    emit_app_bar_button, emit_app_bar_separator, emit_app_bar_toggle_button, AppBarButtonDesc,
    AppBarSeparatorDesc, AppBarToggleButtonDesc,
};
pub use blade_view::{emit_blade_view, BladeViewDesc};
pub use command_bar_flyout::{emit_command_bar_flyout, CommandBarFlyoutDesc};
pub use grid_view::{emit_grid_view, GridViewDesc};
pub use headered_content::{emit_headered_content, HeaderedContentDesc};
pub use image_cropper::{emit_image_cropper, ImageCropperDesc};
pub use items_view::{emit_items_view, ItemsLayout, ItemsViewDesc};
pub use list_box::{emit_list_box, ListBoxDesc};
pub use orbit_view::{emit_orbit_view, OrbitViewDesc};
pub use radial_gauge::{emit_radial_gauge, RadialGaugeDesc};
pub use range_selector::{emit_range_selector, RangeSelectorDesc};
pub use refresh_container::{emit_refresh_container, RefreshContainerDesc};
pub use rich_edit_box::{
    emit_rich_edit_box, RichEditBoxDesc, RICH_EDIT_PADDING_X, RICH_EDIT_PADDING_Y, RICH_EDIT_RADIUS,
};
pub use swipe_control::{emit_swipe_control, SwipeControlDesc, SwipeSide};
pub use teaching_tip::{emit_teaching_tip, TeachingTipDesc, TeachingTipTail};
pub use viewport_3d::{
    clamp_render_scale, compute_viewport_physical_extent, emit_viewport_3d, Viewport3DDesc,
    Viewport3DDragMode, Viewport3DGesture, Viewport3DGestureEvent, Viewport3DPointerTracker,
    ViewportCamera3D, ViewportCameraConstraints, ViewportId, VIEWPORT_3D_DEFAULT_Z,
    VIEWPORT_3D_RADIUS,
};

// ---------------------------------------------------------------------------
// WinUI 3 规范名类型别名（扩展期）
//
// 目录枚举已统一使用 WinUI 3 官方规范名；下列别名保留为公开 API 的官方入口，
// 让差分器、展台与后续工单可以直接用规范名书写描述符类型，底层描述符类型名
// 继续沿用历史模块命名。
// ---------------------------------------------------------------------------

/// WinUI 3 `CheckBox` 的规范名别名。
pub type CheckBoxDesc = CheckboxDesc;
/// WinUI 3 `ToggleSwitch` 的规范名别名。
pub type ToggleSwitchDesc = SwitchDesc;
/// WinUI 3 `ToggleSwitch` 布局别名。
pub type ToggleSwitchLayout = SwitchLayout;
/// WinUI 3 `ToggleSwitch` 令牌别名。
pub type ToggleSwitchTokens = SwitchTokens;
/// WinUI 3 `ComboBox`（不可编辑下拉）的规范名别名。
pub type ComboBoxDesc = SelectDesc;
/// WinUI 3 `ComboBox` 状态别名。
pub type ComboBoxState = SelectState;
/// WinUI 3 `ComboBox (IsEditable = true)` 的规范名别名。
pub type ComboBoxEditableDesc = ComboboxDesc;
/// WinUI 3 `TextBox` 的规范名别名。
pub type TextBoxDesc = TextInputDesc;
/// WinUI 3 `TextBox (AcceptsReturn = true)` 的规范名别名。
pub type TextBoxAreaDesc = TextAreaDesc;
/// WinUI 3 `RadioButton` 的规范名别名。
pub type RadioButtonDesc = RadioDesc;
/// WinUI 3 `NumberBox` 的规范名别名。
pub type NumberBoxDesc = NumberInputDesc;
/// WinUI 3 `AutoSuggestBox` 的规范名别名。
pub type AutoSuggestBoxDesc = AutoCompleteDesc;
/// WinUI 3 `RichTextBlock` 的规范名别名。
pub type RichTextBlockDesc = RichTextDesc;
/// WinUI 3 `ProgressBar` 的规范名别名。
pub type ProgressBarDesc = ProgressDesc;
/// WinUI 3 `ProgressRing` 的规范名别名。
pub type ProgressRingDesc = SpinnerDesc;
/// WinUI 3 `InfoBar` 的规范名别名。
pub type InfoBarDesc = AlertDesc;
/// WinUI 3 `InfoBadge` 的规范名别名。
pub type InfoBadgeDesc = BadgeDesc;
/// WinUI 3 `ToolTip` 的规范名别名。
pub type ToolTipDesc = TooltipDesc;
/// WinUI 3 `CommandBar` 的规范名别名。
pub type CommandBarDesc = ToolbarDesc;
/// WinUI 3 `TabView` 的规范名别名。
pub type TabViewDesc = TabsDesc;
/// WinUI 3 `BreadcrumbBar` 的规范名别名。
pub type BreadcrumbBarDesc = BreadcrumbDesc;
/// WinUI 3 `SelectorBar` 的规范名别名。
pub type SelectorBarDesc = SegmentedDesc;
/// WinUI 3 `PipsPager` 的规范名别名。
pub type PipsPagerDesc = PaginationDesc;
/// WinUI 3 `Expander` 的规范名别名。
pub type ExpanderDesc = CollapsibleDesc;
/// WinUI 3 `SplitView` 的规范名别名。
pub type SplitViewDesc = DrawerDesc;
/// WinUI 3 `FlipView` 的规范名别名。
pub type FlipViewDesc = CarouselDesc;
/// WinUI 3 `PersonPicture` 的规范名别名。
pub type PersonPictureDesc = AvatarDesc;
/// WinUI 3 `StackPanel` 的规范名别名。
pub type StackPanelDesc = StackDesc;
/// WinUI 3 `FontIcon` 的规范名别名。
pub type FontIconDesc = IconDesc;
/// WinUI 3 `ContentDialog` 的规范名别名。
pub type ContentDialogDesc = ModalDesc;
/// WinUI 3 `Flyout` 的规范名别名。
pub type FlyoutDesc = PopoverDesc;
/// WinUI 3 `MenuFlyout` 的规范名别名。
pub type MenuFlyoutDesc = MenuDesc;
/// WinUI 3 `CalendarView` 的规范名别名。
pub type CalendarViewDesc = DatePickerDesc;
/// WinUI 3 `Border` 的规范名别名。
pub type BorderDesc = CardDesc;

/// Community Toolkit `TableView` 的规范名别名。
pub type TableViewDesc = TableDesc;
/// Community Toolkit `TokenizingTextBox` 的规范名别名。
pub type TokenizingTextBoxDesc = TagInputDesc;
/// Community Toolkit `SettingsCard` 的规范名别名。
pub type SettingsCardDesc = FormDesc;
/// Community Toolkit `SettingsExpander` 的规范名别名。
pub type SettingsExpanderDesc = DescriptionsDesc;
/// Community Toolkit `WrapPanel` 的规范名别名。
pub type WrapPanelDesc = WrapDesc;
/// Community Toolkit `StaggeredPanel` 的规范名别名。
pub type StaggeredPanelDesc = MasonryDesc;
/// Community Toolkit `Shimmer` 的规范名别名。
pub type ShimmerDesc = SkeletonDesc;
/// Community Toolkit `ColorPalette` 的规范名别名。
pub type ColorPaletteDesc = ColorSwatchDesc;
/// Community Toolkit `GridSplitter` 的规范名别名。
pub type GridSplitterDesc = SplitPaneDesc;

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
    /// 控件内部声明的局部裁剪节点表（图元通过局部索引引用，合入全局场景时自动求交并重定位）。
    pub clips: Vec<GpuClipNode>,
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

    /// 注册标准 UI 层命中区域与悬停光标。
    ///
    /// 控件只需给出自身绘制深度 `z`，无需知道 `layer` 常量或浮层基准值：
    /// 位于浮层内时由 `AppRuntime` 按节点 `z_base` 统一抬升 `layer` / `z`；
    /// 命中区域归属（含多部件子区域）也由收集器自动登记，事件因此能沿
    /// 逻辑父链冒泡，控件不必再为「让父级收到事件」额外发射命中区域。
    #[must_use]
    pub fn with_hit_ui(
        mut self,
        node: u64,
        z: u32,
        rect: Rect,
        cursor: unbound_input::CursorKind,
    ) -> Self {
        self.hit = Some(HitRegion::ui(node, z, rect).with_cursor(cursor));
        self
    }

    /// 判断图元是否引用载荷表（字形、彩色字形、图像、贝塞尔曲线或线性渐变）。
    #[must_use]
    pub fn primitive_uses_payload(prim: &GpuPrimitiveInstance) -> bool {
        let uses_by_type = matches!(
            prim.prim_type(),
            unbound_gpu::data::PrimType::Glyph
                | unbound_gpu::data::PrimType::GlyphColor
                | unbound_gpu::data::PrimType::Image
                | unbound_gpu::data::PrimType::Curve
        );
        let has_gradient = ((prim.prim_type_and_flags >> 16) as u16
            & unbound_gpu::data::PrimFlags::GRADIENT_LINEAR.bits())
            != 0;
        uses_by_type || has_gradient
    }

    /// 追加一条局部裁剪节点并返回其局部索引。
    pub fn push_clip(&mut self, node: GpuClipNode) -> u32 {
        let index = u32::try_from(self.clips.len()).unwrap_or(CLIP_NONE - 1);
        self.clips.push(node);
        index
    }

    /// 合并另一份 [`WidgetOutput`]，自动重定位载荷与局部裁剪索引，并保留命中区域、无障碍与焦点顺序。
    pub fn extend(&mut self, mut other: WidgetOutput) {
        let payload_base = u32::try_from(self.payloads.len()).unwrap_or(u32::MAX);
        let clip_base = u32::try_from(self.clips.len()).unwrap_or(CLIP_NONE - 1);
        if payload_base != 0 || clip_base != 0 {
            for prim in &mut other.primitives {
                if payload_base != 0 && Self::primitive_uses_payload(prim) {
                    prim.extra_payload_idx = prim.extra_payload_idx.saturating_add(payload_base);
                }
                if clip_base != 0 && prim.clip_idx != CLIP_NONE {
                    prim.clip_idx = prim.clip_idx.saturating_add(clip_base);
                }
            }
        }
        self.primitives.extend(other.primitives);
        self.payloads.extend(other.payloads);
        self.clips.extend(other.clips);
        let other_root_node = other.hit.as_ref().map(|hit| hit.node);
        if self.hit.is_none() {
            self.hit = other.hit;
        } else if let Some(hit) = other.hit {
            self.sub_hits.push(hit);
        }
        self.sub_hits.extend(other.sub_hits);
        if self.a11y.is_none() {
            self.a11y = other.a11y;
        } else if let Some(node) = other.a11y {
            let sub_key = other_root_node
                .or_else(|| other.sub_a11y.first().map(|(k, _)| k.wrapping_sub(1)))
                .unwrap_or_else(|| {
                    self.hit
                        .as_ref()
                        .map_or(0, |hit| hit.node)
                        .wrapping_add(self.sub_a11y.len() as u64 + 1)
                });
            self.sub_a11y.push((sub_key, node));
        }
        self.sub_a11y.extend(other.sub_a11y);
        self.focusable |= other.focusable;
        self.sub_focus_order.extend(other.sub_focus_order);
    }

    /// 合并子控件输出的图元、载荷与局部裁剪节点，并在拼接前自动重定位 `extra_payload_idx` 与 `clip_idx`。
    pub fn append_child_primitives(&mut self, mut child: WidgetOutput) {
        let payload_base = u32::try_from(self.payloads.len()).unwrap_or(u32::MAX);
        let clip_base = u32::try_from(self.clips.len()).unwrap_or(CLIP_NONE - 1);
        if payload_base != 0 || clip_base != 0 {
            for prim in &mut child.primitives {
                if payload_base != 0 && Self::primitive_uses_payload(prim) {
                    prim.extra_payload_idx = prim.extra_payload_idx.saturating_add(payload_base);
                }
                if clip_base != 0 && prim.clip_idx != CLIP_NONE {
                    prim.clip_idx = prim.clip_idx.saturating_add(clip_base);
                }
            }
        }
        self.primitives.extend(child.primitives);
        self.payloads.extend(child.payloads);
        self.clips.extend(child.clips);
    }
}

/// 将外层裁剪节点与控件局部裁剪节点求交，生成合并后的 [`GpuClipNode`]。
///
/// AABB 取精确矩形交集；非退化圆角 SDF 槽位（`radii != [0; 4]`）按外层优先顺序填入
/// `rounded_clip_0` / `rounded_clip_1`，若外层已占满 2 槽则保留外层圆角槽并叠加精确 AABB 交集。
#[must_use]
pub fn intersect_clip_nodes(outer: &GpuClipNode, inner: &GpuClipNode) -> GpuClipNode {
    let aabb = [
        outer.aabb_clip[0].max(inner.aabb_clip[0]),
        outer.aabb_clip[1].max(inner.aabb_clip[1]),
        outer.aabb_clip[2].min(inner.aabb_clip[2]),
        outer.aabb_clip[3].min(inner.aabb_clip[3]),
    ];
    let mut node = GpuClipNode::from_aabb(aabb);
    let mut slots: [Option<([f32; 4], [u16; 4])>; 2] = [None, None];
    let mut count = 0usize;
    for (rect, radii) in [
        (outer.rounded_clip_0, outer.radii_0),
        (outer.rounded_clip_1, outer.radii_1),
        (inner.rounded_clip_0, inner.radii_0),
        (inner.rounded_clip_1, inner.radii_1),
    ] {
        if radii != [0; 4] && count < 2 {
            slots[count] = Some((rect, radii));
            count += 1;
        }
    }
    if let Some((rect, radii)) = slots[0] {
        node.rounded_clip_0 = rect;
        node.radii_0 = radii;
    }
    if let Some((rect, radii)) = slots[1] {
        node.rounded_clip_1 = rect;
        node.radii_1 = radii;
    }
    node
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
    /// 命中区域键 → 所属控件节点键。
    ///
    /// 多部件控件的 `sub_hits` 常用 `node ^ (1 << 63)` 一类合成键，它们不在
    /// 世界树中；运行时据此把这类命中映射回逻辑父链，使子部件同样参与冒泡。
    pub hit_owners: Vec<(u64, u64)>,
    /// 语义树。
    pub semantics: Semantics,
    /// 真正承载语义节点的控件键集合。
    ///
    /// 这里必须**显式登记**而不能用「语义树里是否存在该键」来反推：
    /// 语义根本身也占据一个键，若拿它做成员判断，任何恰好撞上该键的控件都会被
    /// 误判成语义节点，进而让语义根成为自己的子节点——遍历时陷入死循环。
    ///
    /// 登记时排除语义根，因此这里只含有真实控件的键。
    semantic_nodes: FxHashSet<u64>,
    /// 键盘 Tab 顺序。
    pub focus_order: Vec<u64>,
    /// 按控件标识索引的各自图元区间。
    pub node_spans: FxHashMap<u64, (u32, u32)>,
    /// 本帧浮层产生的背景模糊请求。
    pub backdrops: Vec<crate::overlay::BackdropRequest>,
    /// 当前激活的裁剪作用域：`(clip index, screen-space bounds)`。
    clip_stack: Vec<(u32, Rect)>,
    /// 当前激活的变换作用域：`(transform index, cumulative translation)`。
    transform_stack: Vec<(u32, unbound_math::Affine2)>,
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
        self.hit_owners.clear();
        self.focus_order.clear();
        self.node_spans.clear();
        self.backdrops.clear();
        self.clip_stack.clear();
        self.transform_stack.clear();
        self.semantic_nodes.clear();
        self.semantics.reset(root);
        // 语义根节点必须存在：AccessKit 的树以它为根，语义路径也从它起步。
        // 若根节点缺席，读取方会收到一棵没有根的树，路径也会缺掉窗口这一段。
        //
        // 根不登记进 `semantic_nodes`：它是语义树本身的根，不是某个控件。
        self.semantics.upsert(
            root,
            A11yNode::new(accesskit::Role::Window, String::new(), Rect::ZERO),
        );
    }

    /// 登记一个控件的语义节点。
    ///
    /// 与 [`Semantics::upsert`] 的区别在于：本方法会把键记入 `semantic_nodes`，
    /// 使该控件在后续的层级重建中被识别为「承载语义」的世界节点。
    /// 语义根不经过本方法登记。
    pub fn upsert_semantic(&mut self, key: u64, node: A11yNode) {
        self.semantic_nodes.insert(key);
        self.semantics.upsert(key, node);
    }

    /// 该键是否承载语义节点（不含语义根）。
    #[must_use]
    pub fn has_semantic_node(&self, key: u64) -> bool {
        self.semantic_nodes.contains(&key)
    }

    /// 依据世界树的层级关系重建语义树的父子结构。
    ///
    /// 语义节点的键与控件键一致；世界树里的纯布局容器（行、列、内边距盒）没有
    /// 语义节点，其子孙会自动上浮到**最近的语义祖先**之下。这样 Agent 看到的
    /// 层级与界面上的视觉嵌套一致，而不是一棵扁平的节点列表。
    ///
    /// 必须在全部控件推入之后调用：此时世界树与语义节点都已齐备。
    pub fn rebuild_semantic_hierarchy(&mut self, world: &UiWorld) {
        if self.semantics.is_empty() {
            return;
        }
        let root = self.semantics.root();
        // 语义子节点 → 语义父键。
        let mut links: Vec<(u64, u64)> = Vec::new();
        self.collect_semantic_links(world, world.root(), root, &mut links);
        if links.is_empty() {
            return;
        }
        // 按父键分组。分组顺序必须让父节点先于子节点写入：
        // `set_children` 会把子节点从旧父节点摘下，顺序颠倒会让子节点的父链
        // 被后写入的父节点清掉。
        let mut index_of: FxHashMap<u64, usize> = FxHashMap::default();
        let mut groups: Vec<(u64, Vec<u64>)> = Vec::new();
        for (child, parent) in links {
            let index = match index_of.get(&parent) {
                Some(index) => *index,
                None => {
                    groups.push((parent, Vec::new()));
                    let index = groups.len() - 1;
                    index_of.insert(parent, index);
                    index
                }
            };
            groups[index].1.push(child);
        }
        for (parent, children) in groups {
            self.semantics.set_children(parent, &children);
        }
    }

    /// 递归收集语义节点与其最近语义祖先的对应关系。
    fn collect_semantic_links(
        &self,
        world: &UiWorld,
        id: NodeId,
        ancestor: u64,
        links: &mut Vec<(u64, u64)>,
    ) {
        for child in world.children(id) {
            let Some(data) = world.get(child) else {
                continue;
            };
            let key = data.desc.key;
            if key == self.semantics.root() {
                // 世界节点恰好占用了语义根键：它不是语义节点，语义根另有其人。
                // 把它当成普通语义节点会产出「根是根的子节点」的自环。
                // 按纯布局容器的规则处理，让其子孙上浮到最近的语义祖先。
                self.collect_semantic_links(world, child, ancestor, links);
                continue;
            }
            if self.has_semantic_node(key) {
                links.push((key, ancestor));
                self.collect_semantic_links(world, child, key, links);
            } else {
                // 纯布局容器自身没有语义，其子孙上浮到最近的语义祖先。
                self.collect_semantic_links(world, child, ancestor, links);
            }
        }
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
    pub fn push_transform(&mut self, transform: GpuTransform) -> u32 {
        if self.transforms.is_empty() {
            self.transforms.push(GpuTransform::IDENTITY);
        }
        let flags = transform.flags;
        let composed = if let Some(&(_, parent_affine)) = self.transform_stack.last() {
            parent_affine.then(&transform.to_affine())
        } else {
            transform.to_affine()
        };
        let mut gpu = GpuTransform::from_affine(composed);
        gpu.flags = flags;
        let index = u32::try_from(self.transforms.len()).unwrap_or(CLIP_NONE - 1);
        self.transforms.push(gpu);
        self.transform_stack.push((index, composed));
        index
    }

    /// 关闭最内层变换作用域。
    pub fn pop_transform(&mut self) {
        self.transform_stack.pop();
    }

    /// 返回当前最内层变换作用域累积的屏幕空间平移偏移量。
    #[must_use]
    pub fn active_offset(&self) -> Vec2 {
        self.transform_stack
            .last()
            .map_or(Vec2::ZERO, |&(_, affine)| Vec2::new(affine.tx, affine.ty))
    }

    /// Return the composed 2D affine transform of the active transform scope.
    #[must_use]
    pub fn active_transform(&self) -> unbound_math::Affine2 {
        self.transform_stack
            .last()
            .map_or(unbound_math::Affine2::IDENTITY, |&(_, affine)| affine)
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

    /// 追加一份自包含的预构建场景（例如外壳自绘的场景表）。
    ///
    /// 预构建场景的载荷、变换、裁剪索引都从 0 起算，而收集器把这些表拼成单一的
    /// 全局数组。直接按切片拼接会让整段文本读到前序控件的 UV——每段文字都渲染成
    /// 其它文字的乱码混合。这里统一重定位三类索引后再追加。
    ///
    /// `transforms` 须自带索引 0 的单位变换，与收集器的约定一致。
    pub fn append_scene(
        &mut self,
        primitives: &[GpuPrimitiveInstance],
        transforms: &[GpuTransform],
        clips: &[GpuClipNode],
        payloads: &[GpuPayload],
    ) {
        let payload_base = u32::try_from(self.payloads.len()).unwrap_or(u32::MAX);
        let transform_base = u32::try_from(self.transforms.len()).unwrap_or(u32::MAX);
        let clip_base = u32::try_from(self.clips.len()).unwrap_or(u32::MAX);
        self.primitives.extend(primitives.iter().map(|primitive| {
            let mut primitive = *primitive;
            primitive.extra_payload_idx = primitive.extra_payload_idx.saturating_add(payload_base);
            let animated = primitive.transform_idx & TRANSFORM_ANIMATED_BIT;
            let index = (primitive.transform_idx & TRANSFORM_INDEX_MASK)
                .saturating_add(transform_base)
                & TRANSFORM_INDEX_MASK;
            primitive.transform_idx = animated | index;
            if primitive.clip_idx != CLIP_NONE {
                primitive.clip_idx = primitive.clip_idx.saturating_add(clip_base);
            }
            primitive
        }));
        self.transforms.extend_from_slice(transforms);
        self.clips.extend_from_slice(clips);
        self.payloads.extend_from_slice(payloads);
    }

    /// 追加单个控件的输出并记录其拥有的图元区间。
    pub fn push(&mut self, key: u64, mut output: WidgetOutput) {
        let active_transform = self.transform_stack.last().copied();
        if let Some((transform_idx, affine)) = active_transform {
            for primitive in &mut output.primitives {
                if primitive.transform_idx == 0 {
                    primitive.transform_idx = transform_idx;
                }
            }
            if let Some(node) = output.a11y.as_mut() {
                node.bounds = affine.transform_rect_bounds(&node.bounds);
            }
        }

        let active_clip = self.clip_stack.last().copied();
        let local_clip_base = if output.clips.is_empty() {
            None
        } else {
            let base = u32::try_from(self.clips.len()).unwrap_or(CLIP_NONE - 1);
            let outer = active_clip
                .filter(|&(idx, _)| idx != CLIP_NONE)
                .and_then(|(idx, _)| self.clips.get(idx as usize).copied());
            for local_clip in &output.clips {
                let merged = match outer {
                    Some(ref outer_node) => intersect_clip_nodes(outer_node, local_clip),
                    None => *local_clip,
                };
                self.clips.push(merged);
            }
            Some(base)
        };

        for primitive in &mut output.primitives {
            if primitive.clip_idx == CLIP_NONE {
                if let Some((clip_idx, _)) = active_clip {
                    primitive.clip_idx = clip_idx;
                }
            } else if let Some(clip_base) = local_clip_base {
                primitive.clip_idx = primitive.clip_idx.saturating_add(clip_base);
            }
        }

        if let Some((_, clip_rect)) = active_clip {
            if let Some(node) = output.a11y.as_mut() {
                node.bounds = node.bounds.intersect(&clip_rect);
            }
        }

        if let Some(hit) = output.hit.as_mut() {
            self.scope_hit(hit);
            self.hit_owners.push((hit.node, key));
        }
        for sub_hit in &mut output.sub_hits {
            self.scope_hit(sub_hit);
            self.hit_owners.push((sub_hit.node, key));
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
            self.upsert_semantic(key, node);
        }
        for (sub_key, mut sub_node) in output.sub_a11y {
            if let Some((_, affine)) = active_transform {
                sub_node.bounds = affine.transform_rect_bounds(&sub_node.bounds);
            }
            if let Some((_, clip_rect)) = self.clip_stack.last().copied() {
                sub_node.bounds = sub_node.bounds.intersect(&clip_rect);
            }
            self.upsert_semantic(sub_key, sub_node);
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
        if let Some(&(_, affine)) = self.transform_stack.last() {
            hit.bounds = affine.transform_rect_bounds(&hit.bounds);
            if let Some(clip) = hit.clip.as_mut() {
                clip.rect = affine.transform_rect_bounds(&clip.rect);
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

    /// 外壳自绘的整份场景同样从 0 起算索引。收集器里已有背板等图元时，
    /// 直接拼接会让每个字形都采样到别人的 UV，必须整体重定位。
    #[test]
    fn appended_scenes_are_rebased_after_existing_primitives() {
        let mut collector = SceneCollector::new(0);
        // 先模拟云母背板：占用载荷 0、变换 0 与裁剪 0。
        collector.push(
            1,
            WidgetOutput {
                primitives: vec![GpuPrimitiveInstance::default()],
                payloads: vec![GpuPayload::default()],
                ..Default::default()
            },
        );
        collector.push_transform(GpuTransform::IDENTITY);
        collector.push_clip(GpuClipNode::default(), Rect::ZERO);

        let scene = [
            GpuPrimitiveInstance {
                extra_payload_idx: 3,
                transform_idx: 1,
                clip_idx: 2,
                ..Default::default()
            },
            GpuPrimitiveInstance {
                extra_payload_idx: 0,
                transform_idx: TRANSFORM_ANIMATED_BIT | 1,
                clip_idx: CLIP_NONE,
                ..Default::default()
            },
        ];
        collector.append_scene(
            &scene,
            &[GpuTransform::IDENTITY],
            &[GpuClipNode::default()],
            &[GpuPayload::default()],
        );

        let rebased = collector.primitives[1];
        assert_eq!(rebased.extra_payload_idx, 4);
        // `push_transform` 会在空表里补上槽位 0 的单位变换，因此基准为 2。
        assert_eq!(rebased.transform_idx, 3);
        assert_eq!(rebased.clip_idx, 3);
        // 动画标志位必须保留；`CLIP_NONE` 不得被加上偏移。
        let animated = collector.primitives[2];
        assert_eq!(animated.extra_payload_idx, 1);
        assert_eq!(animated.transform_idx, TRANSFORM_ANIMATED_BIT | 3);
        assert_eq!(animated.clip_idx, CLIP_NONE);
    }

    #[test]
    fn widget_output_extend_relocates_local_clips_and_preserves_clip_none() {
        let mut first = WidgetOutput::default();
        let c0 = first.push_clip(GpuClipNode::from_aabb([0.0, 0.0, 100.0, 100.0]));
        assert_eq!(c0, 0);
        first.primitives.push(GpuPrimitiveInstance {
            clip_idx: c0,
            ..Default::default()
        });

        let mut second = WidgetOutput::default();
        let s0 = second.push_clip(GpuClipNode::from_aabb([10.0, 10.0, 80.0, 80.0]));
        let s1 = second.push_clip(GpuClipNode::from_aabb([20.0, 20.0, 60.0, 60.0]));
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        second.primitives.push(GpuPrimitiveInstance {
            clip_idx: s0,
            ..Default::default()
        });
        second.primitives.push(GpuPrimitiveInstance {
            clip_idx: CLIP_NONE,
            ..Default::default()
        });
        second.primitives.push(GpuPrimitiveInstance {
            clip_idx: s1,
            ..Default::default()
        });

        first.extend(second);

        assert_eq!(first.clips.len(), 3);
        assert_eq!(first.primitives[0].clip_idx, 0);
        assert_eq!(first.primitives[1].clip_idx, 1);
        assert_eq!(first.primitives[2].clip_idx, CLIP_NONE);
        assert_eq!(first.primitives[3].clip_idx, 2);
    }

    #[test]
    fn intersect_clip_nodes_handles_aabb_single_rounded_and_full_slots() {
        // 1. 纯 AABB 与纯 AABB 求精确矩形交集。
        let outer_aabb = GpuClipNode::from_aabb([0.0, 10.0, 200.0, 300.0]);
        let inner_aabb = GpuClipNode::from_aabb([50.0, 0.0, 150.0, 250.0]);
        let merged_aabb = intersect_clip_nodes(&outer_aabb, &inner_aabb);
        assert_eq!(merged_aabb.aabb_clip, [50.0, 10.0, 150.0, 250.0]);
        assert_eq!(merged_aabb.radii_0, [0; 4]);
        assert_eq!(merged_aabb.radii_1, [0; 4]);

        // 2. 单圆角外层 + 纯 AABB 内层：保留外层圆角槽并收紧 AABB。
        let outer_r = Rect::new(0.0, 0.0, 200.0, 200.0);
        let outer_rounded = rounded_clip(outer_r, 12.0);
        let inner_rect = GpuClipNode::from_aabb([20.0, 30.0, 180.0, 190.0]);
        let merged_single = intersect_clip_nodes(&outer_rounded, &inner_rect);
        assert_eq!(merged_single.aabb_clip, [20.0, 30.0, 180.0, 190.0]);
        assert_eq!(merged_single.rounded_clip_0, outer_rounded.rounded_clip_0);
        assert_eq!(merged_single.radii_0, outer_rounded.radii_0);
        assert_eq!(merged_single.radii_1, [0; 4]);

        // 3. 双圆角满槽外层 + 圆角内层：保留外层 2 个圆角槽并叠加精确 AABB 交集。
        let outer_r1 = Rect::new(10.0, 10.0, 180.0, 180.0);
        let mut outer_full = rounded_clip(outer_r, 16.0);
        outer_full.rounded_clip_1 = outer_r1.to_center_half();
        outer_full.radii_1 = [f32_to_f16(8.0); 4];
        let inner_r = Rect::new(20.0, 25.0, 120.0, 100.0);
        let inner_rounded = rounded_clip(inner_r, 6.0);
        let merged_full = intersect_clip_nodes(&outer_full, &inner_rounded);
        assert_eq!(merged_full.aabb_clip, [20.0, 25.0, 140.0, 125.0]);
        assert_eq!(merged_full.rounded_clip_0, outer_full.rounded_clip_0);
        assert_eq!(merged_full.radii_0, outer_full.radii_0);
        assert_eq!(merged_full.rounded_clip_1, outer_full.rounded_clip_1);
        assert_eq!(merged_full.radii_1, outer_full.radii_1);
    }

    #[test]
    fn scene_collector_push_intersects_and_relocates_local_clips() {
        let mut collector = SceneCollector::new(0);
        let outer_rect = Rect::new(10.0, 10.0, 200.0, 200.0);
        let outer_idx = collector.push_clip(rounded_clip(outer_rect, 8.0), outer_rect);
        assert_eq!(outer_idx, 0);

        let mut out = WidgetOutput::default();
        let local_idx = out.push_clip(GpuClipNode::from_aabb([0.0, 40.0, 180.0, 160.0]));
        out.primitives.push(GpuPrimitiveInstance {
            clip_idx: local_idx,
            ..Default::default()
        });
        out.primitives.push(GpuPrimitiveInstance {
            clip_idx: CLIP_NONE,
            ..Default::default()
        });

        collector.push(42, out);

        assert_eq!(collector.clips.len(), 2);
        // 局部裁剪图元重定位到索引 1，且与外层求交保留圆角槽。
        assert_eq!(collector.primitives[0].clip_idx, 1);
        assert_eq!(collector.clips[1].aabb_clip, [10.0, 40.0, 180.0, 160.0]);
        assert_eq!(collector.clips[1].radii_0, collector.clips[0].radii_0);
        // `CLIP_NONE` 图元沿用外层栈顶索引 0。
        assert_eq!(collector.primitives[1].clip_idx, outer_idx);
    }
}
