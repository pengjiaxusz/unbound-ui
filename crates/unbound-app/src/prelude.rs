//! lang-allow: 中文注释与文档
//! 常用图元、应用契约、描述符与运行时类型重导出。

pub use crate::appearance::{
    apply_appearance_diff_to_platform, apply_appearance_diff_to_window, backdrop_channel_request,
    dwm_material_to_platform, AppearanceCaps, AppearanceDiff, AppearanceIntent, BackdropChannel,
    WindowAppearance,
};
pub use crate::backdrop::{is_mica_ready_event, MicaBackdrop, MICA_READY_EVENT_ID};
pub use crate::floating_host::{
    compute_drag_proxy_rect, DragProxyGhostState, DEFAULT_DRAG_PROXY_CURSOR_OFFSET,
};
pub use crate::form::{FormBindingStore, FormMutator, WidgetValue};
pub use crate::multi_window::{
    run_multi_window, run_multi_window_with_config, CrossWindowDragRoute,
    CrossWindowDragTargetState, MultiWindowConfig, MultiWindowCoordinator, MultiWindowFactory,
    WindowContext, WindowCreatedHook, WindowGeometry,
};
pub use crate::router::{RouteEntry, RouterStack, TransitionDirection};
pub use crate::scheduler::{AppWaker, TimerRequest, TimerToken};
pub use crate::settings::{
    overflow_menu_item_key, overflow_menu_popup_key, select_option_key, select_popup_key,
    SettingsCard,
};
pub use crate::state_store::{LifecycleChanges, LifecycleEvent, WidgetStateStore};
pub use crate::window_runner::{
    run_windowed, run_windowed_with_hwnd, ViewportPassFactory, WindowedRunner,
};
pub use crate::{
    const_fnv1a_hash, global_locale, mix_key, set_global_locale, ui_key, AppContext, AppEvent,
    AppEventSender, AppKey, AppRuntime, Application, ClipboardProvider, DirtyFlags, ErasedUiWidget,
    EventOutcome, ExternalTextureDesc, ExternalTextureHandle, ExternalTextureOp,
    ExternalTextureState, FrameReport, I18nBundle, KeyDiscriminator, KeyScope, LayoutDirection,
    Locale, Modifiers, OverlayManager, PendingWidget, Propagation, TabHitTarget, ThemeSnapshotKey,
    ThemeSyncOutcome, ThemeTransitionDriver, TitleBarSlots, VideoSurfaceDesc, VideoSurfaceHandle,
    VideoSurfaceMetrics, VideoSurfaceOp, VideoSurfaceState, VideoUploadTier, ViewBuilder,
    ViewContext, Viewport3DFrameEntry, WidgetSpec, WindowCommand, DEFAULT_VIDEO_VRAM_BUDGET_BYTES,
    ROOT,
};

pub use accesskit::Role;
pub use unbound_a11y::{A11yAction, SemanticIntent};
pub use unbound_abi::{Align, Dim, Display, FlexDirection, Justify, NodeFlags, RawNodeDesc};
pub use unbound_canvas::{Handle, HandleDrag, SnapGuide};
pub use unbound_gpu::{
    CustomViewportPass, ViewportCamera3D, ViewportCameraConstraints, ViewportDepthMode, ViewportId,
    ViewportMsaa, ViewportPassContext, ViewportReadback, ViewportRenderPassDesc,
};
pub use unbound_input::{CursorKind, KeyChord, KeyName, PointerButton, PointerEvent};
pub use unbound_math::{Affine2, Color, Rect, Vec2};
pub use unbound_platform::{
    active_tray_menu_items, enumerate_monitor_work_areas, is_tray_heartbeat_stalled, open_file,
    open_file_async, open_files, poll_tray_events, read_system_accent_color, reattach_tray_icon,
    save_file, save_file_async, set_taskbar_progress, set_tray_icon, set_tray_menu,
    set_tray_rescue_menu, show_tray_balloon, system_accent_color, update_tray_heartbeat,
    DisplayWorkArea, FileDialogConfig, FileDragEffect, FrameHitMetrics, MonitorWorkArea,
    OutboundFileDragOutcome, OutboundFileDragRequest, PlatformError, PopupWindowConfig,
    RenderThrottlePolicy, RenderThrottleState, ScreenRect, SurfaceAlpha, TaskbarProgressState,
    TrayEvent, TrayMenuItem, TrayNotificationSeverity, WindowBackdropMaterial, WindowConfig,
    WindowId, WindowPlacement, WindowPosition,
};
pub use unbound_text::{
    global_pinyin_config, global_shuangpin_scheme, set_global_pinyin_config,
    set_global_shuangpin_scheme, PinyinConfig, PinyinMatch, PinyinMatcher, ShuangpinScheme,
    TextStyle,
};
pub use unbound_theme::typography::geometry::READABLE_FORM_MAX_WIDTH;
pub use unbound_theme::{
    snap_ui_scale, step_ui_scale, typography, AccentBrand, ProductPreset, ThemeKind, ThemeState,
    ThemeTransition, ThemeTransitionConfig, Token, CANONICAL_SCALE_STEPS,
    DEFAULT_THEME_TRANSITION_DURATION_SECS,
};
pub use unbound_widgets::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
pub use unbound_widgets::overlay::{OverlayEntry, OverlayKind, OverlayStack};
pub use unbound_widgets::virtual_masonry::MasonryItem;
pub use unbound_widgets::{
    column_header_sub_node, compute_column_header_layout, emit_brand_badge, emit_column_header_bar,
    emit_column_header_bar_chrome, emit_column_header_bar_text_layer, emit_feature_card,
    emit_gizmo, emit_hero_backdrop, emit_icon_button, emit_scrollbar, emit_shell_icon_primitives,
    emit_widget_icon_primitives, emit_window_caption_buttons, grip_rect_for, grip_rect_rotated,
    schema_form_trailing_action_node, scrollbar_thumb_node, scrollbar_track_node, tab_add_node,
    tab_close_node, tab_item_node, AddressBarDesc, AddressBarDropdownLayout,
    AddressBarDropdownMode, AddressBarEvent, AddressBarHitKind, AddressBarState, AlertDesc,
    AppBrandBadgeDesc, AutoCompleteDesc, AutoCompleteState, BadgeDesc, BadgeVariant, BannerDesc,
    ButtonDesc, ButtonSize, ButtonTokens, ButtonVariant, CalloutDesc, CalloutState, CardDesc,
    ChartDesc, ChartSeries, ChartState, CheckState, CheckboxDesc, CodeBlockDesc, CodeBlockState,
    CodeEditorDesc, CodeEditorState, CodeLanguage, ColorFormat, ColorPart, ColorPickerDesc,
    ColorPickerState, ColorSwatchDesc, ColumnHeaderAlign, ColumnHeaderBarDesc,
    ColumnHeaderBarLayout, ColumnHeaderBarState, ColumnHeaderCellLayout, ColumnHeaderEvent,
    ColumnHeaderHitKind, ColumnHeaderSpec, ColumnResizeDrag, ColumnSpec, ComboItem, ComboboxDesc,
    ComboboxState, ControlState, CurveEditorDesc, DataColumnSpec, DataGridDesc, DataGridState,
    DescriptionItem, DescriptionsDesc, DescriptionsState, DividerDesc, DraggableModalState,
    EmptyStateDesc, FeatureCardDesc, FieldInheritanceState, FileDropDesc, FilterPopupState,
    GenericDataTableDesc, GenericDataTableState, GizmoDesc, GizmoOutput, HeroBackdropDesc,
    HotkeyInputDesc, HotkeyInputState, IconButtonDesc, KeybindingManagerState, MultiSplitDesc,
    MultiSplitState, NumberInputDesc, NumberInputState, PaginationDesc, PaginationState,
    PathSegment, PipelineStageItem, PipelineViewDesc, Placement, PopconfirmDesc, ProgressDesc,
    ProgressState, QueryBuilderDesc, QueryCombinator, QueryRule, ReorderableListState,
    SchemaFieldKind, SchemaFieldSpec, SchemaFormDesc, SchemaFormEvent, SchemaFormRowLayout,
    SchemaFormState, SchemaLabelColumnConfig, SchemaValue, ScrollBarDesc, ScrollBarOutput,
    ScrollBarVisibility, ScrollbarAxis, SelectDesc, SelectState, SeriesKind, Severity, ShellIcon,
    ShellIconDesc, ShortcutBarDesc, ShortcutEntry, SliderDesc, SmoothWheelHandler, SortDir,
    SortDirection, SplitButtonDesc, SplitButtonState, SplitPaneDesc, SplitPaneState, SwitchDesc,
    TabItem, TabViewEvent, TabViewStyle, TabWidthMode, TableDesc, TableRow, TableState, TabsDesc,
    TabsState, TaskActivityItem, TaskHudDesc, TerminalViewDesc, TerminalViewState, TextEditState,
    TextInputDesc, ThemeSettingsState, TimelineTrackDesc, TimelineTrackState, TitleBarDesc,
    ToggleButtonDesc, ToggleButtonState, TooltipDesc, TooltipLayout, TooltipPadding, TooltipState,
    TourDesc, TransferDesc, TransferState, TreeRowIcon, TreeTableDesc, TreeTableState,
    TreeViewDensity, TreeViewDesc, TreeViewEvent, TreeViewState, Viewport3DDesc, Viewport3DGesture,
    Viewport3DGestureEvent, VirtualMasonryDesc, VirtualMasonryState, WatermarkDesc,
    WidgetCatalogId, WidgetIcon, WidgetOutput, WindowCaptionButton,
    WindowCaptionButtonControlStates, WindowCaptionButtonEntry, WindowCaptionButtons,
    WindowCaptionButtonsLayout, WindowCaptionButtonsOutput, WindowCaptionFrameHitContract,
    WindowCaptionKind, ADDRESS_BAR_FONT_SIZE, ADDRESS_BAR_HEIGHT, COLUMN_DEFAULT_MAX_WIDTH,
    COLUMN_DEFAULT_MIN_WIDTH, COLUMN_DIVIDER_WIDTH, COLUMN_FILTER_BUTTON_WIDTH,
    COLUMN_HEADER_BAR_HEIGHT, COLUMN_RESIZE_HANDLE_WIDTH, TAB_CLOSE, TAB_SHELL_ITEM_HEIGHT,
    TAB_STRIP_HEIGHT, TITLE_BAR_CAPTION_WIDTH, TITLE_BAR_CLOSE_ID, TITLE_BAR_DRAG_ID,
    TITLE_BAR_FUSED_HEIGHT, TITLE_BAR_HEIGHT, TITLE_BAR_MAXIMIZE_ID, TITLE_BAR_MINIMIZE_ID,
    TITLE_BAR_MIN_DRAG_WIDTH, TREE_DIVIDER_HEIGHT, TREE_FONT_SIZE, TREE_ICON_GAP, TREE_ROW_COMPACT,
};
pub use unbound_widgets::{
    emit_menu, emit_menu_with_rows, menu_item_node, Command, CommandPaletteDesc,
    CommandPaletteState, CommandPaletteTokens, ContextClose, ContextKey, ContextMenuState,
    DockNode, DockPane, DockState, DockTab, DockTokens, DockTree, DockZone, ImageDesc, ImageSource,
    MenuDesc, MenuItem, MenuItemRenderRow, MenuState, MinimapDesc, MinimapItem, MinimapState,
    ObjectFit, PopoverDesc, PopoverOutput, TreeViewRow, TreeViewTokens, VariableVirtualList,
    VariableWindow, CONTEXT_MENU_ITEM_HEIGHT, MENU_ICON_SIZE, MENU_ITEM_RADIUS, MENU_PANEL_PADDING,
    MENU_SEPARATOR_HEIGHT,
};
pub use unbound_world::{NodeId, TreeRow};

pub use unbound_anim::{DurationToken, EasingToken, MotionPolicy, MotionToken, Tween};

pub use unbound_widgets::{ColoredGlyphQuad, RichTextLineDesc, SpanDecoration};

pub use unbound_text::TextSpan;
