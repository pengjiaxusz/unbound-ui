//! Common primitives, application contracts, descriptors, and runtime types.

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
pub use unbound_platform::{PlatformError, WindowConfig};
pub use unbound_text::TextStyle;
pub use unbound_theme::{ThemeKind, ThemeState, Token};
pub use unbound_widgets::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
pub use unbound_widgets::{
    AlertDesc, BadgeDesc, ButtonDesc, ButtonSize, ButtonVariant, CardDesc, CheckState,
    CheckboxDesc, CodeEditorDesc, CodeEditorState, ControlState, DataGridDesc, DataGridState,
    DividerDesc, PaginationDesc, PaginationState, PopconfirmDesc, ProgressDesc, ProgressState,
    SelectDesc, SelectState, SliderDesc, SwitchDesc, TableDesc, TableRow, TableState,
    TextEditState, TextInputDesc, TimelineTrackDesc, TimelineTrackState, TourDesc, TransferDesc,
    TransferState, WidgetCatalogId, WidgetOutput,
};
pub use unbound_world::NodeId;
