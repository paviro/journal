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
    EditorScroll(i16),
    EditorStartSelection { col: u16, row: u16 },
    EditorDragSelection { col: u16, row: u16 },
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
    JournalInputChar(char),
    JournalInputBackspace,
    JournalInputSubmit,
    // Tags overlay
    MetadataMoveUp,
    MetadataMoveDown,
    MetadataToggle,
    MetadataSwitchFocus,
    MetadataInput(char),
    MetadataBackspace,
    MetadataAddFromInput,
    MetadataSave,
    // Feelings overlay
    FeelingsMoveUp,
    FeelingsMoveDown,
    FeelingsToggle,
    FeelingsExpand,
    FeelingsCollapse,
    FeelingsSwitchFocus,
    FeelingsInput(char),
    FeelingsBackspace,
    FeelingsSave,
    // Mood overlay
    MoodDecrease,
    MoodIncrease,
    MoodSave,
    MoodClear,
    // Location overlay
    BeginEditLocation,
    LocationSwitchFocus,
    LocationInput(char),
    LocationBackspace,
    LocationMoveUp,
    LocationMoveDown,
    LocationResolve,
    LocationGrabDevice,
    LocationSelectRow,
    LocationSave,
    LocationClear,
    // Image viewer overlay
    OpenImageViewer(usize),
    ImageViewerNext,
    ImageViewerPrev,
    // Search text input (only active when mode=Search and focus=Entries)
    SearchInput(char),
    SearchBackspace,
    SearchCursorLeft,
    SearchCursorRight,
    ToggleHints,
    ToggleJournals,
}
