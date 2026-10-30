//! Every menu action as a `Command`, plus the
//! default keyboard shortcuts. On macOS, "Ctrl" maps to ⌘.

use egui::{Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};

use crate::textops::{CaseMode, HashAlgo, SortMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HashSource {
    Text,
    Files,
    SelectionToClipboard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MimeOp {
    Base64Encode,
    Base64EncodeByLine,
    Base64UrlEncode,
    Base64Decode,
    Base64DecodeByLine,
    UrlEncode,
    UrlDecode,
    UrlDecodeForm,
    QpEncode,
    QpDecode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatOp {
    /// Pick JSON or XML from the document's language / content.
    Auto,
    JsonPretty,
    JsonMinify,
    JsonValidate,
    XmlPretty,
    XmlMinify,
    XmlValidate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabSort {
    NameAsc,
    NameDesc,
    PathAsc,
    PathDesc,
    TypeAsc,
    TypeDesc,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    // File
    New,
    Open,
    OpenFolderAsWorkspace,
    Reload,
    Save,
    SaveAs,
    SaveCopyAs,
    SaveAll,
    Rename,
    Close,
    CloseAll,
    CloseAllButCurrent,
    CloseAllButPinned,
    CloseAllToLeft,
    CloseAllToRight,
    CloseAllUnchanged,
    RestoreLastClosed,
    OpenAllRecent,
    ClearRecent,
    OpenRecent(String),
    LoadSession,
    SaveSession,
    OpenContainingFolder,
    OpenTerminalHere,
    OpenInDefaultViewer,
    Exit,
    // Edit
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    DeleteSelection,
    SelectAll,
    BeginEndSelect,
    BeginEndSelectColumn,
    InsertDateTimeShort,
    InsertDateTimeLong,
    InsertDateTimeCustom,
    CopyFullPath,
    CopyFileName,
    CopyDirPath,
    CopyAllNames,
    CopyAllPaths,
    IndentIncrease,
    IndentDecrease,
    ConvertCase(CaseMode),
    DuplicateLine,
    RemoveDuplicateLines,
    RemoveConsecutiveDuplicates,
    SplitLines,
    JoinLines,
    MoveLineUp,
    MoveLineDown,
    RemoveEmptyLines,
    RemoveEmptyLinesWithBlank,
    InsertBlankAbove,
    InsertBlankBelow,
    Sort(SortMode),
    DeleteLine,
    TransposeLine,
    ToggleComment,
    BlockComment,
    BlockUncomment,
    StreamComment,
    StreamUncomment,
    WordCompletion,
    FunctionCompletion,
    EolWindows,
    EolUnix,
    EolMac,
    TrimTrailing,
    TrimLeading,
    TrimBoth,
    EolToSpace,
    TrimAndEolToSpace,
    TabToSpace,
    SpaceToTabAll,
    SpaceToTabLeading,
    ColumnEditor,
    CharacterPanel,
    ClipboardHistory,
    ToggleReadOnly,
    MultiSelectAll {
        match_case: bool,
        whole_word: bool,
    },
    MultiSelectNext {
        match_case: bool,
        whole_word: bool,
    },
    MultiSelectUndo,
    MultiSelectSkip,
    Redact,
    SearchOnInternet,
    OpenSelectedFile,
    // Search
    Find,
    FindInFiles,
    FindNext,
    FindPrev,
    SelectAndFindNext,
    SelectAndFindPrev,
    VolatileFindNext,
    VolatileFindPrev,
    Replace,
    Mark,
    IncrementalSearch,
    SearchResultsWindow,
    NextSearchResult,
    PrevSearchResult,
    GoTo,
    GoToMatchingBrace,
    SelectToMatchingBrace,
    ChangeHistoryNext,
    ChangeHistoryPrev,
    ChangeHistoryClear,
    StyleAll(u8),
    StyleOne(u8),
    ClearStyle(u8),
    ClearAllStyles,
    JumpNextStyle(u8),
    JumpPrevStyle(u8),
    CopyStyledText(u8),
    ToggleBookmark,
    NextBookmark,
    PrevBookmark,
    ClearBookmarks,
    CutBookmarkedLines,
    CopyBookmarkedLines,
    PasteToBookmarkedLines,
    RemoveBookmarkedLines,
    RemoveUnbookmarkedLines,
    InverseBookmarks,
    FindCharsInRange,
    // View
    AlwaysOnTop,
    FullScreen,
    PostIt,
    DistractionFree,
    ShowWhitespace,
    ShowEol,
    ShowAllChars,
    ShowIndentGuides,
    ShowWrapSymbol,
    ZoomIn,
    ZoomOut,
    ZoomRestore,
    MoveToOtherView,
    CloneToOtherView,
    GoToTab(usize),
    NextTab,
    PrevTab,
    MoveTabForward,
    MoveTabBackward,
    WordWrap,
    SyntaxHighlighting,
    FocusOtherView,
    FoldAll,
    UnfoldAll,
    FoldCurrent,
    UnfoldCurrent,
    FoldLevel(usize),
    UnfoldLevel(usize),
    Summary,
    ToggleDocMap,
    ToggleDocList,
    ToggleFunctionList,
    ToggleFolderTree,
    SyncScrollV,
    SyncScrollH,
    Monitoring,
    ViewInBrowser,
    ToggleToolbar,
    ToggleStatusbar,
    TabColor(Option<u8>),
    TogglePin,
    // Encoding / language
    SetEncoding(String),
    ConvertEncoding(String),
    SetLanguage(String),
    // Settings
    Preferences,
    StyleConfigurator,
    ShortcutMapper,
    // Tools
    Hash(HashAlgo, HashSource),
    Mime(MimeOp),
    // Compare
    Compare,
    CompareWith(u64, u64),
    CompareClipboard,
    CompareSaved,
    CompareClear,
    CompareRefresh,
    CompareNext,
    ComparePrev,
    CompareCopyToRight,
    CompareCopyToLeft,
    /// 0 = ignore whitespace, 1 = ignore case, 2 = ignore empty lines, 3 = word diff, 4 = auto re-compare.
    CompareOption(u8),
    Format(FormatOp),
    // Run
    RunDialog,
    RunSaved(usize),
    // Window
    WindowsDialog,
    SortTabs(TabSort),
    // Help
    About,
    CommandLineHelp,
    DebugInfo,
}

#[derive(Clone, Copy)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

impl Shortcut {
    const fn new(ctrl: bool, alt: bool, shift: bool, key: Key) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            key,
        }
    }

    pub fn to_egui(self) -> KeyboardShortcut {
        let mut m = Modifiers::NONE;
        if self.ctrl {
            m |= Modifiers::COMMAND;
        }
        if self.alt {
            m |= Modifiers::ALT;
        }
        if self.shift {
            m |= Modifiers::SHIFT;
        }
        KeyboardShortcut::new(m, self.key)
    }

    pub fn label(self) -> String {
        let mut s = String::new();
        let mac = cfg!(target_os = "macos");
        if self.ctrl {
            s.push_str(if mac { "⌘" } else { "Ctrl+" });
        }
        if self.alt {
            s.push_str(if mac { "⌥" } else { "Alt+" });
        }
        if self.shift {
            s.push_str(if mac { "⇧" } else { "Shift+" });
        }
        s.push_str(self.key.name());
        s
    }

    /// Number of modifiers: more specific shortcuts are matched first.
    pub fn specificity(self) -> u8 {
        u8::from(self.ctrl) + u8::from(self.alt) + u8::from(self.shift)
    }
}

const C: bool = true;
const N: bool = false;

/// Default keyboard shortcuts.
pub fn default_shortcuts() -> Vec<(Command, Shortcut)> {
    use Command as K;
    let s = Shortcut::new;
    let mut v = vec![
        (K::New, s(C, N, N, Key::N)),
        (K::Open, s(C, N, N, Key::O)),
        (K::Reload, s(C, N, N, Key::R)),
        (K::Save, s(C, N, N, Key::S)),
        (K::SaveAs, s(C, C, N, Key::S)),
        (K::SaveAll, s(C, N, C, Key::S)),
        (K::Close, s(C, N, N, Key::W)),
        (K::CloseAll, s(C, N, C, Key::W)),
        (K::RestoreLastClosed, s(C, N, C, Key::T)),
        (K::Exit, s(C, N, N, Key::Q)),
        (K::Undo, s(C, N, N, Key::Z)),
        (K::Redo, s(C, N, N, Key::Y)),
        (K::Redo, s(C, N, C, Key::Z)),
        (K::SelectAll, s(C, N, N, Key::A)),
        (K::InsertDateTimeShort, s(N, N, N, Key::F5)),
        (K::ConvertCase(CaseMode::Upper), s(C, N, C, Key::U)),
        (K::ConvertCase(CaseMode::Lower), s(C, N, N, Key::U)),
        (K::ConvertCase(CaseMode::ProperForce), s(N, C, N, Key::U)),
        (K::ConvertCase(CaseMode::ProperBlend), s(N, C, C, Key::U)),
        (K::ConvertCase(CaseMode::SentenceForce), s(C, C, N, Key::U)),
        (K::ConvertCase(CaseMode::SentenceBlend), s(C, C, C, Key::U)),
        (K::DuplicateLine, s(C, N, N, Key::D)),
        (K::SplitLines, s(C, N, N, Key::I)),
        (K::JoinLines, s(C, N, N, Key::J)),
        (K::MoveLineUp, s(C, N, C, Key::ArrowUp)),
        (K::MoveLineDown, s(C, N, C, Key::ArrowDown)),
        (K::InsertBlankAbove, s(C, C, N, Key::Enter)),
        (K::InsertBlankBelow, s(C, C, C, Key::Enter)),
        (K::DeleteLine, s(C, N, C, Key::L)),
        (K::TransposeLine, s(C, N, N, Key::T)),
        (K::ToggleComment, s(C, N, N, Key::Q)),
        (K::BlockComment, s(C, N, N, Key::K)),
        (K::BlockUncomment, s(C, N, C, Key::K)),
        (K::StreamComment, s(C, N, C, Key::Q)),
        (K::FunctionCompletion, s(C, N, N, Key::Space)),
        (K::WordCompletion, s(C, N, N, Key::Enter)),
        (K::ColumnEditor, s(N, C, N, Key::C)),
        (K::Find, s(C, N, N, Key::F)),
        (K::FindInFiles, s(C, N, C, Key::F)),
        (K::FindNext, s(N, N, N, Key::F3)),
        (K::FindPrev, s(N, N, C, Key::F3)),
        (K::SelectAndFindNext, s(C, N, N, Key::F3)),
        (K::SelectAndFindPrev, s(C, N, C, Key::F3)),
        (K::VolatileFindNext, s(C, C, N, Key::F3)),
        (K::VolatileFindPrev, s(C, C, C, Key::F3)),
        (K::Replace, s(C, N, N, Key::H)),
        (K::Mark, s(C, N, N, Key::M)),
        (K::IncrementalSearch, s(C, C, N, Key::I)),
        (K::SearchResultsWindow, s(N, N, N, Key::F7)),
        (K::NextSearchResult, s(N, N, N, Key::F4)),
        (K::PrevSearchResult, s(N, N, C, Key::F4)),
        (K::GoTo, s(C, N, N, Key::G)),
        (K::GoToMatchingBrace, s(C, N, N, Key::B)),
        (K::SelectToMatchingBrace, s(C, C, N, Key::B)),
        (K::ToggleBookmark, s(C, N, N, Key::F2)),
        (K::NextBookmark, s(N, N, N, Key::F2)),
        (K::PrevBookmark, s(N, N, C, Key::F2)),
        (K::FullScreen, s(N, N, N, Key::F11)),
        (K::PostIt, s(N, N, N, Key::F12)),
        (K::ZoomIn, s(C, N, N, Key::Plus)),
        (K::ZoomIn, s(C, N, N, Key::Equals)),
        (K::ZoomOut, s(C, N, N, Key::Minus)),
        (K::ZoomRestore, s(C, N, N, Key::Slash)),
        (K::ZoomRestore, s(C, N, N, Key::Num0)),
        (K::NextTab, s(C, N, N, Key::PageDown)),
        (K::PrevTab, s(C, N, N, Key::PageUp)),
        (K::NextTab, s(C, N, N, Key::Tab)),
        (K::PrevTab, s(C, N, C, Key::Tab)),
        (K::MoveTabForward, s(C, N, C, Key::PageDown)),
        (K::MoveTabBackward, s(C, N, C, Key::PageUp)),
        (K::FocusOtherView, s(N, N, N, Key::F8)),
        (K::FoldAll, s(N, C, N, Key::Num0)),
        (K::UnfoldAll, s(N, C, C, Key::Num0)),
        (K::FoldCurrent, s(C, C, N, Key::F)),
        (K::UnfoldCurrent, s(C, C, C, Key::F)),
        (K::RunDialog, s(N, N, N, Key::F5)),
        (K::Compare, s(C, C, N, Key::C)),
        (K::CompareClear, s(C, C, N, Key::X)),
        (K::CompareNext, s(C, C, N, Key::N)),
        (K::ComparePrev, s(C, C, N, Key::P)),
        (K::CompareCopyToRight, s(C, C, N, Key::Period)),
        (K::CompareCopyToLeft, s(C, C, N, Key::Comma)),
        (K::Format(FormatOp::JsonPretty), s(C, C, N, Key::M)),
        (K::Format(FormatOp::XmlPretty), s(C, C, C, Key::B)),
        (K::Format(FormatOp::Auto), s(N, C, C, Key::F)),
        (K::About, s(N, N, N, Key::F1)),
        (K::StyleAll(1), s(C, C, N, Key::Num1)),
    ];
    // F5 is "Run..."; date-time has no default shortcut.
    v.retain(|(c, _)| *c != K::InsertDateTimeShort);
    let digit = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    v.retain(|(c, _)| *c != K::StyleAll(1));
    for (i, k) in digit.iter().enumerate() {
        v.push((K::GoToTab(i), s(C, N, N, *k)));
        if i < 8 {
            v.push((K::FoldLevel(i + 1), s(N, C, N, *k)));
            v.push((K::UnfoldLevel(i + 1), s(N, C, C, *k)));
        }
    }
    v.sort_by_key(|(_, sc)| std::cmp::Reverse(sc.specificity()));
    v
}

pub fn shortcut_for(table: &[(Command, Shortcut)], cmd: &Command) -> Option<String> {
    table.iter().find(|(c, _)| c == cmd).map(|(_, s)| s.label())
}
