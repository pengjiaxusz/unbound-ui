//! lang-allow: 中文注释与文档
//! 常用图元、应用契约、描述符与运行时类型重导出。

pub use crate::appearance::{
    apply_appearance_diff_to_platform, backdrop_channel_request, dwm_material_to_platform,
    AppearanceCaps, AppearanceDiff, AppearanceIntent, BackdropChannel, WindowAppearance,
};
pub use crate::backdrop::{is_mica_ready_event, MicaBackdrop, MICA_READY_EVENT_ID};
pub use crate::form::{FormBindingStore, FormMutator, WidgetValue};
pub use crate::multi_window::{
    run_multi_window, MultiWindowCoordinator, MultiWindowFactory, WindowContext,
};
pub use crate::router::{RouteEntry, RouterStack, TransitionDirection};
pub use crate::scheduler::{AppWaker, TimerRequest, TimerToken};
pub use crate::settings::{
    overflow_menu_item_key, overflow_menu_popup_key, select_option_key, select_popup_key,
    SettingsCard,
};
pub use crate::state_store::{LifecycleChanges, LifecycleEvent, WidgetStateStore};
pub use crate::window_runner::{run_windowed, run_windowed_with_hwnd, WindowedRunner};
pub use crate::{
    const_fnv1a_hash, global_locale, mix_key, set_global_locale, ui_key, AppContext, AppEvent,
    AppEventSender, AppKey, AppRuntime, Application, ClipboardProvider, DirtyFlags, ErasedUiWidget,
    EventOutcome, ExternalTextureHandle, ExternalTextureOp, ExternalTextureState, FrameReport,
    I18nBundle, KeyDiscriminator, KeyScope, LayoutDirection, Locale, Modifiers, OverlayManager,
    PendingWidget, Propagation, TitleBarSlots, ViewBuilder, ViewContext, WidgetSpec, WindowCommand,
    ROOT,
};

pub use accesskit::Role;
pub use unbound_a11y::{A11yAction, SemanticIntent};
pub use unbound_abi::{Align, Dim, Display, FlexDirection, Justify, NodeFlags, RawNodeDesc};
pub use unbound_input::{CursorKind, KeyChord, KeyName, PointerButton, PointerEvent};
pub use unbound_math::{Color, Rect, Vec2};
pub use unbound_platform::{
    open_file, open_file_async, open_files, save_file, save_file_async, set_taskbar_progress,
    set_tray_icon, show_tray_balloon, FileDialogConfig, FileDragEffect, FrameHitMetrics,
    OutboundFileDragOutcome, OutboundFileDragRequest, PlatformError, PopupWindowConfig,
    RenderThrottlePolicy, RenderThrottleState, ScreenRect, SurfaceAlpha, TaskbarProgressState,
    TrayNotificationSeverity, WindowBackdropMaterial, WindowConfig, WindowId, WindowPlacement,
    WindowPosition,
};
pub use unbound_text::{
    global_pinyin_config, global_shuangpin_scheme, set_global_pinyin_config,
    set_global_shuangpin_scheme, PinyinConfig, PinyinMatch, PinyinMatcher, ShuangpinScheme,
    TextStyle,
};
pub use unbound_theme::typography::geometry::READABLE_FORM_MAX_WIDTH;
pub use unbound_theme::{
    snap_ui_scale, step_ui_scale, typography, AccentBrand, ProductPreset, ThemeKind, ThemeState,
    Token, CANONICAL_SCALE_STEPS,
};
pub use unbound_widgets::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
pub use unbound_widgets::overlay::{OverlayEntry, OverlayKind, OverlayStack};
pub use unbound_widgets::virtual_masonry::MasonryItem;
pub use unbound_widgets::{
    emit_brand_badge, emit_feature_card, emit_hero_backdrop, emit_icon_button,
    emit_shell_icon_primitives, emit_sidebar_item, schema_form_trailing_action_node,
    AddressBarDesc, AddressBarEvent, AddressBarHitKind, AddressBarState, AlertDesc,
    AppBrandBadgeDesc, AutoCompleteDesc, AutoCompleteState, BadgeDesc, BadgeVariant, BannerDesc,
    ButtonDesc, ButtonSize, ButtonTokens, ButtonVariant, CalloutDesc, CalloutState, CardDesc,
    ChartDesc, ChartSeries, ChartState, CheckState, CheckboxDesc, CodeBlockDesc, CodeBlockState,
    CodeEditorDesc, CodeEditorState, CodeLanguage, ColorFormat, ColorPart, ColorPickerDesc,
    ColorPickerState, ColorSwatchDesc, ColumnSpec, ComboItem, ComboboxDesc, ComboboxState,
    ControlState, CurveEditorDesc, DataColumnSpec, DataGridDesc, DataGridState, DescriptionItem,
    DescriptionsDesc, DescriptionsState, DividerDesc, DraggableModalState, EmptyStateDesc,
    FeatureCardDesc, FieldInheritanceState, FileDropDesc, FilterPopupState, GenericDataTableDesc,
    GenericDataTableState, HeroBackdropDesc, HotkeyInputDesc, HotkeyInputState, IconButtonDesc,
    KeybindingManagerState, MultiSplitDesc, MultiSplitState, NumberInputDesc, NumberInputState,
    PaginationDesc, PaginationState, PathSegment, PipelineStageItem, PipelineViewDesc, Placement,
    PopconfirmDesc, ProgressDesc, ProgressState, QueryBuilderDesc, QueryCombinator, QueryRule,
    ReorderableListState, SchemaFieldKind, SchemaFieldSpec, SchemaFormDesc, SchemaFormEvent,
    SchemaFormRowLayout, SchemaFormState, SchemaLabelColumnConfig, SchemaValue, SelectDesc,
    SelectState, SeriesKind, Severity, ShellIcon, ShellIconDesc, ShortcutBarDesc, ShortcutEntry,
    SidebarItemDesc, SidebarSectionHeaderDesc, SliderDesc, SmoothWheelHandler, SortDirection,
    SplitButtonDesc, SplitButtonState, SplitPaneDesc, SplitPaneState, SwitchDesc, TabItem,
    TabViewEvent, TabViewStyle, TabWidthMode, TableDesc, TableRow, TableState, TabsDesc, TabsState,
    TaskActivityItem, TaskHudDesc, TerminalViewDesc, TerminalViewState, TextEditState,
    TextInputDesc, ThemeSettingsState, TimelineTrackDesc, TimelineTrackState, TitleBarDesc,
    ToggleButtonDesc, ToggleButtonState, TooltipDesc, TooltipState, TourDesc, TransferDesc,
    TransferState, TreeRowIcon, TreeTableDesc, TreeTableState, TreeViewDensity, TreeViewDesc,
    TreeViewEvent, TreeViewState, VirtualMasonryDesc, VirtualMasonryState, WatermarkDesc,
    WidgetCatalogId, WidgetOutput, WindowCaptionButton, WindowCaptionKind, ADDRESS_BAR_HEIGHT,
    TITLE_BAR_CAPTION_WIDTH, TITLE_BAR_CLOSE_ID, TITLE_BAR_DRAG_ID, TITLE_BAR_FUSED_HEIGHT,
    TITLE_BAR_HEIGHT, TITLE_BAR_MAXIMIZE_ID, TITLE_BAR_MINIMIZE_ID, TITLE_BAR_MIN_DRAG_WIDTH,
    TREE_ROW_COMPACT,
};
pub use unbound_widgets::{
    Command, CommandPaletteDesc, CommandPaletteState, CommandPaletteTokens, ContextClose,
    ContextKey, ContextMenuState, DockNode, DockPane, DockState, DockTab, DockTokens, DockTree,
    DockZone, ImageDesc, ImageSource, MenuDesc, MenuItem, MenuState, MinimapDesc, MinimapItem,
    MinimapState, ObjectFit, PopoverDesc, PopoverOutput, TreeViewRow, TreeViewTokens,
    VariableVirtualList, VariableWindow,
};
pub use unbound_world::{NodeId, TreeRow};
