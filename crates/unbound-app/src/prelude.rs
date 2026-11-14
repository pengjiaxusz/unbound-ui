//! 常用图元、应用契约、描述符与运行时类型重导出。

pub use crate::router::{RouteEntry, RouterStack, TransitionDirection};
pub use crate::scheduler::{AppWaker, TimerRequest, TimerToken};
pub use crate::state_store::{LifecycleChanges, LifecycleEvent, WidgetStateStore};
pub use crate::window_runner::{run_windowed, WindowedRunner};
pub use crate::{
    const_fnv1a_hash, mix_key, ui_key, AppContext, AppEvent, AppKey, AppRuntime, Application,
    ClipboardProvider, DirtyFlags, ErasedUiWidget, EventOutcome, FrameReport, KeyDiscriminator,
    Modifiers, PendingWidget, Propagation, ViewBuilder, ViewContext, WidgetSpec, ROOT,
};

pub use unbound_a11y::A11yAction;
pub use unbound_abi::{Align, Dim, Display, FlexDirection, Justify, NodeFlags, RawNodeDesc};
pub use unbound_input::{CursorKind, KeyChord, KeyName, PointerButton, PointerEvent};
pub use unbound_math::{Color, Rect, Vec2};
pub use unbound_platform::{
    open_file, open_file_async, open_files, save_file, save_file_async, set_taskbar_progress,
    set_tray_icon, show_tray_balloon, FileDialogConfig, FrameHitMetrics, PlatformError,
    TaskbarProgressState, TrayNotificationSeverity, WindowConfig,
};
pub use unbound_text::TextStyle;
pub use unbound_theme::{ThemeKind, ThemeState, Token};
pub use unbound_widgets::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
pub use unbound_widgets::{
    AlertDesc, BadgeDesc, BannerDesc, ButtonDesc, ButtonSize, ButtonVariant, CardDesc, CheckState,
    CheckboxDesc, CodeEditorDesc, CodeEditorState, ColumnSpec, ControlState, CurveEditorDesc,
    DataGridDesc, DataGridState, DividerDesc, FileDropDesc, PaginationDesc, PaginationState,
    PopconfirmDesc, ProgressDesc, ProgressState, SelectDesc, SelectState, Severity, SliderDesc,
    SwitchDesc, TableDesc, TableRow, TableState, TextEditState, TextInputDesc, TimelineTrackDesc,
    TimelineTrackState, TourDesc, TransferDesc, TransferState, WatermarkDesc, WidgetCatalogId,
    WidgetOutput,
};
pub use unbound_world::NodeId;
