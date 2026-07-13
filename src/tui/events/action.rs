use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;

use crate::tui::state::MetadataKind;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReaderAction {
    ScrollLines(i16),
    ScrollPages(i16),
    ScrollToStart,
    ScrollToEnd,
    SetFullscreen(bool),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InsightsAction {
    ScrollLines(i16),
    ScrollPages(i16),
    ScrollToStart,
    ScrollToEnd,
    SetFullscreen(bool),
    ToggleScope,
    CycleTimeframe,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    PointerInput {
        event: MouseEvent,
        area: Rect,
    },
    PointerScroll {
        event: MouseEvent,
        area: Rect,
        delta: i16,
    },
    PointerHover {
        column: u16,
        row: u16,
        area: Rect,
    },
    // Global
    Quit,
    RefreshLibrary,
    // Background startup-cache reconciliation.
    LibraryValidated(Box<notema_storage::LibrarySnapshot>),
    LibraryValidationFailed(String),
    // Browse / search navigation
    FocusLeft,
    FocusRight,
    MoveSelection(isize),
    Reader(ReaderAction),
    Insights(InsightsAction),
    // Browse operations
    BeginSearch,
    ExitSearch,
    EditSelected,
    // Internal editor.
    EditorSave,
    EditorRequestDiscard,
    EditorDiscard,
    EditorToggleFullscreen,
    EditorOpenMetadataMenu,
    EditorOpenHelp,
    EditorClosePrompt,
    EditorScrollHelp(i16),
    EditorInput(KeyEvent),
    EditorSelectAll,
    EditorScroll(i16),
    EditorStartSelection {
        col: u16,
        row: u16,
    },
    EditorDragSelection {
        col: u16,
        row: u16,
    },
    EditorEndSelection,
    ViewSelected,
    OpenReaderLink(String),
    BeginDelete,
    ConfirmDelete,
    /// Move the selected button in whichever confirm dialog is open (delete
    /// overlay or editor discard prompt). `true` selects the destructive button.
    ConfirmSelect(bool),
    // Cancel / close — covers Esc across all overlays
    CancelOverlay,
    OpenMetadataMenu,
    BeginEditMetadata(MetadataKind),
    BeginEditFeelings,
    BeginEditMood,
    ToggleStarred,
    NewEntry,
    NewJournal,
    ToggleArchiveJournal,
    // New-journal input overlay
    JournalInputSubmit,
    // Tags overlay
    MoveDialogSelection(isize),
    MetadataToggle,
    MetadataSwitchFocus,
    MetadataAddFromInput,
    MetadataSave,
    // Feelings overlay
    FeelingsToggle,
    FeelingsExpand,
    FeelingsCollapse,
    FeelingsSwitchFocus,
    FeelingsSave,
    // Mood overlay
    AdjustMood(i8),
    MoodSave,
    MoodClear,
    // Location overlay
    BeginEditLocation,
    LocationSwitchFocus,
    LocationResolve,
    LocationGrabDevice,
    LocationSelectRow,
    LocationSave,
    LocationClear,
    // Settings menu + theme picker overlays
    OpenSettingsMenu,
    OpenThemePicker,
    /// Select (and show in the reader) the row at this index — mouse click.
    ThemePickerSelect(usize),
    ThemePickerConfirm,
    ThemePickerCancel,
    /// Cycle the chrome override: default → flat → bordered → default.
    ThemePickerCycleChrome,
    /// Cycle the color mode: auto → dark → light → auto.
    ThemePickerCycleMode,
    /// Toggle the picker scope between this journal and the global default.
    ThemePickerToggleScope,
    // Image viewer overlay
    OpenImageViewer(usize),
    StepImageViewer(isize),
    // Search text input (only active when mode=Search and focus=Entries)
    /// A key press for whichever text field currently owns the caret (search
    /// box or an open dialog's focused input): chars, backspace, caret
    /// movement, shift-selection — everything the single-line textarea handles.
    InputKey(KeyEvent),
    /// Select all text in the focused single-line field (Ctrl+A / hint click).
    InputSelectAll,
    ToggleHints,
    ToggleJournals,
}
