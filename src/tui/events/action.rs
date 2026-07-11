use crossterm::event::KeyEvent;

use crate::tui::state::MetadataKind;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    // Global
    Quit,
    // Browse / search navigation
    FocusLeft,
    FocusRight,
    MoveUp,
    MoveDown,
    // Entry view scroll
    ScrollEntryView(i16),
    PageEntryView(i16),
    ScrollEntryViewToStart,
    ScrollEntryViewToEnd,
    // Insights list scroll (Focus::Insights, People / Activities / Tags tabs)
    ScrollInsights(i16),
    PageInsights(i16),
    ScrollInsightsToStart,
    ScrollInsightsToEnd,
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
    EditorBeginMetadata(MetadataKind),
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
    // Expand the focused entry viewer to full screen (multi-column) / collapse back
    ExpandEntryView,
    CollapseEntryView,
    // Expand the focused insights panel to full screen (multi-column) / collapse back
    ExpandInsights,
    CollapseInsights,
    BeginDelete,
    ConfirmDelete,
    // Cancel / close — covers Esc across all overlays
    CancelOverlay,
    OpenMetadataMenu,
    BeginEditTags,
    BeginEditPeople,
    BeginEditActivities,
    BeginEditFeelings,
    BeginEditMood,
    ToggleStarred,
    NewEntry,
    NewJournal,
    ToggleArchiveJournal,
    // Journal insights panel (Focus::Insights). Tabs switch via FocusLeft/Right.
    ToggleInsightsScope,
    CycleInsightsTimeframe,
    // New-journal input overlay
    JournalInputSubmit,
    // Tags overlay
    MetadataMoveUp,
    MetadataMoveDown,
    MetadataToggle,
    MetadataSwitchFocus,
    MetadataAddFromInput,
    MetadataSave,
    // Feelings overlay
    FeelingsMoveUp,
    FeelingsMoveDown,
    FeelingsToggle,
    FeelingsExpand,
    FeelingsCollapse,
    FeelingsSwitchFocus,
    FeelingsSave,
    // Mood overlay
    MoodDecrease,
    MoodIncrease,
    MoodSave,
    MoodClear,
    // Location overlay
    BeginEditLocation,
    LocationSwitchFocus,
    LocationMoveUp,
    LocationMoveDown,
    LocationResolve,
    LocationGrabDevice,
    LocationSelectRow,
    LocationSave,
    LocationClear,
    // Settings menu + theme picker overlays
    OpenSettingsMenu,
    OpenThemePicker,
    ThemePickerMoveUp,
    ThemePickerMoveDown,
    /// Select (and live-preview) the row at this index — mouse click.
    ThemePickerSelect(usize),
    ThemePickerConfirm,
    ThemePickerCancel,
    /// Cycle the chrome override: auto → flat → bordered → auto.
    ThemePickerCycleChrome,
    // Image viewer overlay
    OpenImageViewer(usize),
    ImageViewerNext,
    ImageViewerPrev,
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
