use crossterm::event::KeyEvent;

use crate::tui::{
    features::{insights::InsightsTab, location::EditLocationFocus},
    state::{HoverTarget, MetadataKind},
    ui::interaction::{PanelId, TextFieldId},
};

pub(crate) use crate::tui::ui::interaction::ScrollbarMetrics;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextFieldTarget {
    Search,
    NewJournal,
    Metadata,
    Feelings,
    LocationQuery,
    LocationName,
}

impl From<TextFieldId> for TextFieldTarget {
    fn from(value: TextFieldId) -> Self {
        match value {
            TextFieldId::Search => Self::Search,
            TextFieldId::NewJournal => Self::NewJournal,
            TextFieldId::Metadata => Self::Metadata,
            TextFieldId::Feelings => Self::Feelings,
            TextFieldId::LocationQuery => Self::LocationQuery,
            TextFieldId::LocationName => Self::LocationName,
        }
    }
}

impl From<TextFieldTarget> for TextFieldId {
    fn from(value: TextFieldTarget) -> Self {
        match value {
            TextFieldTarget::Search => Self::Search,
            TextFieldTarget::NewJournal => Self::NewJournal,
            TextFieldTarget::Metadata => Self::Metadata,
            TextFieldTarget::Feelings => Self::Feelings,
            TextFieldTarget::LocationQuery => Self::LocationQuery,
            TextFieldTarget::LocationName => Self::LocationName,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogListTarget {
    Metadata,
    Feelings,
    Location,
    ThemePicker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetadataSearchTarget {
    Feelings,
    Metadata(MetadataKind),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MouseAction {
    DismissToast(usize),
    TextFieldPress {
        target: TextFieldTarget,
        column: u16,
    },
    TextFieldSelectWord {
        target: TextFieldTarget,
        column: u16,
    },
    TextFieldDrag {
        column: u16,
    },
    TextFieldRelease,
    JournalClick {
        index: Option<usize>,
        compact: bool,
    },
    EntryClick {
        index: Option<usize>,
        open_reader: bool,
        clear_empty: bool,
    },
    InsightsClick(Option<InsightsTab>),
    ReaderClick,
    MetadataSearch {
        kind: MetadataSearchTarget,
        value: String,
    },
    ScrollPanel {
        panel: PanelId,
        delta: i16,
        content_length: usize,
        viewport: u16,
    },
    ScrollbarPress {
        metrics: ScrollbarMetrics,
        row: u16,
    },
    ScrollbarDrag {
        metrics: ScrollbarMetrics,
        row: u16,
    },
    ScrollbarRelease,
    DialogRow {
        target: DialogListTarget,
        index: usize,
    },
    DialogFocusMetadata(EditMetadataFocusTarget),
    DialogFocusLocation(EditLocationFocus),
    DialogScroll {
        target: DialogListTarget,
        delta: i16,
        viewport: u16,
    },
    SetMood(i8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditMetadataFocusTarget {
    List,
    Input,
}

#[derive(Debug, PartialEq)]
pub(crate) enum BackgroundAction {
    LibraryValidated(Box<notema_storage::LibrarySnapshot>),
    LibraryValidationStale,
    LibraryValidationFailed(String),
    ExternalOpenCompleted(String),
    ExternalOpenFailed(String),
    PollImages,
    PollGeocode,
    PollEnvironment,
    PollTimers,
    LibraryPathsChanged(Vec<std::path::PathBuf>),
    ReloadTheme(String),
    CommitSearch,
}

#[derive(Debug, PartialEq)]
pub(crate) enum BrowserAction {
    FocusLeft,
    FocusRight,
    MoveSelection(isize),
    EditSelected,
    ViewSelected,
    OpenReaderLink {
        target: String,
        heading_line: Option<usize>,
    },
    BeginDelete,
    ConfirmDelete,
    ToggleStarred,
    NewEntry,
}

#[derive(Debug, PartialEq)]
pub(crate) enum SearchAction {
    Begin,
    Exit,
}

#[derive(Debug, PartialEq)]
pub(crate) enum EditorAction {
    Save,
    RequestDiscard,
    Discard,
    ToggleFullscreen,
    OpenMetadataMenu,
    OpenHelp,
    ClosePrompt,
    ScrollHelp(i16),
    Input(KeyEvent),
    SelectAll,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Scroll(i16),
    StartSelection { col: u16, row: u16 },
    SelectWord { col: u16, row: u16 },
    DragSelection { col: u16, row: u16 },
    EndSelection,
}

#[derive(Debug, PartialEq)]
pub(crate) enum MetadataAction {
    OpenMenu,
    BeginEdit(MetadataKind),
    BeginFeelings,
    BeginMood,
    MoveSelection(isize),
    Toggle,
    SwitchFocus,
    AddFromInput,
    Save,
    FeelingsToggle,
    FeelingsExpand,
    FeelingsCollapse,
    FeelingsSwitchFocus,
    FeelingsSave,
    AdjustMood(i8),
    MoodSave,
    MoodClear,
}

#[derive(Debug, PartialEq)]
pub(crate) enum LocationAction {
    BeginEdit,
    SwitchFocus,
    Resolve,
    GrabDevice,
    SelectRow,
    Save,
    Clear,
}

#[derive(Debug, PartialEq)]
pub(crate) enum SettingsAction {
    NewJournal,
    ToggleArchiveJournal,
    JournalInputSubmit,
    OpenMenu,
    OpenThemePicker,
    ThemePickerSelect(usize),
    ThemePickerConfirm,
    ThemePickerCancel,
    ThemePickerCycleChrome,
    ThemePickerCycleMode,
    ThemePickerToggleScope,
}

#[derive(Debug, PartialEq)]
pub(crate) enum ImageAction {
    OpenViewer(usize),
    StepViewer(isize),
}

#[derive(Debug, PartialEq)]
pub(crate) enum ReaderAction {
    ScrollLines(i16),
    ScrollPages(i16),
    ScrollToStart,
    ScrollToEnd,
    SetFullscreen(bool),
}

#[derive(Debug, PartialEq)]
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
pub(crate) enum OverlayAction {
    ConfirmSelect(bool),
    Cancel,
    OpenHelp,
    HelpScroll(i16),
    InputKey(KeyEvent),
    InputSelectAll,
    ToggleHints,
    ToggleJournals,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    Mouse(MouseAction),
    SetHover(HoverTarget),
    ViewRendered {
        reader_scroll: Option<u16>,
        insights_scroll: Option<u16>,
        journal_offset: Option<usize>,
        entry_offset: Option<usize>,
    },
    SyncImages(ratatui::layout::Size),
    // Global
    Quit,
    RefreshLibrary,
    Background(BackgroundAction),
    Browser(BrowserAction),
    Search(SearchAction),
    Editor(EditorAction),
    Metadata(MetadataAction),
    Location(LocationAction),
    Settings(SettingsAction),
    Images(ImageAction),
    Overlay(OverlayAction),
    Reader(ReaderAction),
    Insights(InsightsAction),
}
