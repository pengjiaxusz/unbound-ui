//! 常用图元、应用契约、描述符与运行时类型重导出。

pub use crate::router::{RouteEntry, RouterStack, TransitionDirection};
pub use crate::scheduler::{AppWaker, TimerRequest, TimerToken};
pub use crate::state_store::{LifecycleChanges, LifecycleEvent, WidgetStateStore};
pub use crate::window_runner::{run_windowed, run_windowed_with_hwnd, WindowedRunner};
pub use crate::{
    const_fnv1a_hash, global_locale, mix_key, set_global_locale, ui_key, AppContext, AppEvent,
    AppEventSender, AppKey, AppRuntime, Application, ClipboardProvider, DirtyFlags, ErasedUiWidget,
    EventOutcome, FrameReport, I18nBundle, KeyDiscriminator, LayoutDirection, Locale, Modifiers,
    PendingWidget, Propagation, TitleBarSlots, ViewBuilder, ViewContext, WidgetSpec, WindowCommand,
    ROOT,
};

pub use unbound_a11y::A11yAction;
pub use unbound_abi::{Align, Dim, Display, FlexDirection, Justify, NodeFlags, RawNodeDesc};
pub use unbound_input::{CursorKind, KeyChord, KeyName, PointerButton, PointerEvent};
pub use unbound_math::{Color, Rect, Vec2};
pub use unbound_platform::{
    open_file, open_file_async, open_files, save_file, save_file_async, set_taskbar_progress,
    set_tray_icon, show_tray_balloon, FileDialogConfig, FrameHitMetrics, PlatformError,
    TaskbarProgressState, TrayNotificationSeverity, WindowConfig, WindowPlacement, WindowPosition,
};
pub use unbound_text::{
    global_pinyin_config, global_shuangpin_scheme, set_global_pinyin_config,
    set_global_shuangpin_scheme, PinyinConfig, PinyinMatch, PinyinMatcher, ShuangpinScheme,
    TextStyle,
};
pub use unbound_theme::{
    snap_ui_scale, step_ui_scale, AccentBrand, ProductPreset, ThemeKind, ThemeState, Token,
    CANONICAL_SCALE_STEPS,
};
pub use unbound_widgets::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
pub use unbound_widgets::{
    emit_brand_badge, emit_feature_card, emit_hero_backdrop, emit_icon_button,
    emit_shell_icon_primitives, emit_sidebar_item, AddressBarDesc, AddressBarState, AlertDesc,
    AppBrandBadgeDesc, AutoCompleteDesc, AutoCompleteState, BadgeDesc, BadgeVariant, BannerDesc,
    ButtonDesc, ButtonSize, ButtonVariant, CalloutDesc, CalloutState, CardDesc, ChartDesc,
    ChartSeries, ChartState, CheckState, CheckboxDesc, CodeBlockDesc, CodeBlockState,
    CodeEditorDesc, CodeEditorState, CodeLanguage, ColorFormat, ColorPart, ColorPickerDesc,
    ColorPickerState, ColumnSpec, ComboItem, ComboboxDesc, ComboboxState, ControlState,
    CurveEditorDesc, DataColumnSpec, DataGridDesc, DataGridState, DescriptionItem,
    DescriptionsDesc, DescriptionsState, DividerDesc, DraggableModalState, EmptyStateDesc,
    FeatureCardDesc, FieldInheritanceState, FileDropDesc, FilterPopupState, GenericDataTableDesc,
    GenericDataTableState, HeroBackdropDesc, HotkeyInputDesc, HotkeyInputState, IconButtonDesc,
    KeybindingManagerState, MultiSplitDesc, MultiSplitState, NumberInputDesc, NumberInputState,
    PaginationDesc, PaginationState, PipelineStageItem, PipelineViewDesc, Placement,
    PopconfirmDesc, ProgressDesc, ProgressState, QueryBuilderDesc, QueryCombinator, QueryRule,
    ReorderableListState, SchemaFieldKind, SchemaFieldSpec, SchemaFormDesc, SchemaFormState,
    SchemaValue, SelectDesc, SelectState, SeriesKind, Severity, ShellIcon, ShellIconDesc,
    ShortcutBarDesc, ShortcutEntry, SidebarItemDesc, SidebarSectionHeaderDesc, SliderDesc,
    SmoothWheelHandler, SortDirection, SplitButtonDesc, SplitButtonState, SplitPaneDesc,
    SplitPaneState, SwitchDesc, TabItem, TableDesc, TableRow, TableState, TabsDesc, TabsState,
    TaskActivityItem, TaskHudDesc, TerminalViewDesc, TerminalViewState, TextEditState,
    TextInputDesc, ThemeSettingsState, TimelineTrackDesc, TimelineTrackState, TitleBarDesc,
    ToggleButtonDesc, ToggleButtonState, TooltipDesc, TooltipState, TourDesc, TransferDesc,
    TransferState, TreeTableDesc, TreeTableState, TreeViewDesc, TreeViewState, VirtualMasonryDesc,
    VirtualMasonryState, WatermarkDesc, WidgetCatalogId, WidgetOutput, WindowCaptionButton,
    WindowCaptionKind, TITLE_BAR_CAPTION_WIDTH, TITLE_BAR_CLOSE_ID, TITLE_BAR_DRAG_ID,
    TITLE_BAR_HEIGHT, TITLE_BAR_MAXIMIZE_ID, TITLE_BAR_MINIMIZE_ID,
};
pub use unbound_world::NodeId;
