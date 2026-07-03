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
    BeginDelete,
    ConfirmDelete,
    // Cancel / close — covers Esc across all overlays and the expanded-entry close gesture
    CancelOverlay,
    BeginEditTags,
    BeginEditFeelings,
    BeginEditMood,
    NewEntry,
    NewJournal,
    // New-journal input overlay
    JournalInputChar(char),
    JournalInputBackspace,
    JournalInputSubmit,
    // Tags overlay
    TagsMoveUp,
    TagsMoveDown,
    TagsToggle,
    TagsSwitchFocus,
    TagsInput(char),
    TagsBackspace,
    TagsAddFromInput,
    TagsSave,
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
    // Search text input (only active when mode=Search and focus=Entries)
    SearchInput(char),
    SearchBackspace,
}
