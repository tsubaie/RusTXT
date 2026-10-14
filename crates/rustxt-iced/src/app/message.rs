//! Messages the UI emits and the menu identifiers they carry.

use iced::{keyboard, widget::text_editor, window};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    File,
    Edit,
    View,
    Settings,
    Zoom,
    Recent,
}
#[derive(Debug, Clone)]
pub enum Message {
    Action(text_editor::Action),
    Modifiers(keyboard::Modifiers),
    Tick,
    New,
    Open,
    Opened(Option<Vec<PathBuf>>),
    Save(bool),
    SaveAll,
    SaveNext,
    SavePicked(String, Option<PathBuf>),
    Select(usize),
    Close(usize),
    CloseActive,
    NextTab(bool),
    ZoomBy(i32),
    Quit,
    Menu(Menu),
    CloseMenu,
    Submenu(Option<Menu>),
    ToggleReplace,
    Dismiss,
    Find(bool),
    Query(String),
    Replacement(String),
    MatchCase(bool),
    WholeWord(bool),
    Regex(bool),
    FindNext(bool),
    Replace,
    ReplaceAll,
    Undo(bool),
    Copy(bool),
    Paste,
    Pasted(Option<String>),
    SelectAll,
    Delete,
    Reopen(String),
    ReopenLast,
    Discard,
    ConfirmDiscard(String),
    Settings,
    About,
    Wrap(bool),
    Status(bool),
    Theme(String),
    Font(String),
    ApplyFont,
    ResetFont,
    ChooseFont,
    FontFilter(String),
    FontChosen(String),
    Zoom(u32),
    Titlebar(String),
    GoTo,
    GoLine(String),
    Go,
    Date,
    Print,
    OpenLink(String),
    Window(window::Event),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Choice {
    pub(super) value: String,
    pub(super) label: String,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}
