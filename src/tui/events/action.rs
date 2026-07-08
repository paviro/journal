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
    // Browse operations
    BeginSearch,
    ExitSearch,
    EditSelected,
    ViewSelected,
    // Expand the focused entry viewer to full screen (multi-column) / collapse back
    ExpandEntryView,
    CollapseEntryView,
    BeginDelete,
    ConfirmDelete,
    // Cancel / close — covers Esc across all overlays
    CancelOverlay,
    BeginEditTags,
    BeginEditPeople,
    BeginEditActivities,
    BeginEditFeelings,
    BeginEditMood,
    ToggleStarred,
    NewEntry,
    NewJournal,
    ToggleArchiveJournal,
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
    FeelingsSave,
    // Mood overlay
    MoodDecrease,
    MoodIncrease,
    MoodSave,
    MoodClear,
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
