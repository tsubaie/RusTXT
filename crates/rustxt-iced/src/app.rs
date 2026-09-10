use crate::{
    document::{self, Document},
    icons::{icon, Icon},
    instance::Instance,
    search::Search,
};
use iced::{
    keyboard::{self, key::Named, Key},
    widget::{
        self, button, column, container, operation, pick_list, row, scrollable, space, text,
        text_editor, text_input, toggler,
    },
    window, Element, Event, Fill, Font, Subscription, Task, Theme,
};
use rustxt_core::{
    config::{self, Config, Paths},
    desktop, files,
    storage::{ClosedDocument, DocumentState, LineEnding, Storage},
    watch,
};
use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

const UI: f32 = 44.0 / 3.0;
const CAPTION: f32 = UI * 0.85;
const REPLACE: &str = "replace";

const EDITOR: &str = "editor";
const FIND: &str = "find";
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run() -> iced::Result {
    crate::fonts::configure(system_font("sans-serif"));
    let paths = Paths::discover();
    let arguments: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .map(|p| std::path::absolute(&p).unwrap_or(p))
        .collect();
    let instance = match Instance::acquire(&paths.data_dir, &arguments) {
        Ok(Some(instance)) => instance,
        Ok(None) => return Ok(()),
        Err(error) => {
            eprintln!("RusTXT: {error}");
            rfd::MessageDialog::new()
                .set_title("RusTXT could not start")
                .set_description(&error)
                .set_level(rfd::MessageLevel::Error)
                .show();
            std::process::exit(1);
        }
    };
    let app = match App::new(paths, arguments, instance) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("RusTXT: {error}");
            rfd::MessageDialog::new()
                .set_title("RusTXT could not start")
                .set_description(&error)
                .set_level(rfd::MessageLevel::Error)
                .show();
            std::process::exit(1);
        }
    };
    let initial_size = app.window_size;
    let decorations = app
        .config
        .window
        .title_bar
        .decorated(desktop::running_on_tiling_compositor());
    let state = RefCell::new(Some(app));
    iced::application(
        move || {
            (
                state
                    .borrow_mut()
                    .take()
                    .expect("application initialized once"),
                operation::focus(EDITOR),
            )
        },
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(App::theme)
    .subscription(App::subscription)
    .window(window::Settings {
        size: initial_size,
        min_size: Some(iced::Size::new(540.0, 360.0)),
        decorations,
        exit_on_close_request: false,
        ..Default::default()
    })
    .settings(iced::Settings {
        default_font: system_ui_font()
            .map(|family| Font::with_name(Box::leak(family.into_boxed_str())))
            .unwrap_or_default(),
        default_text_size: iced::Pixels(UI),
        ..Default::default()
    })
    .run()
}

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

pub struct App {
    docs: Vec<Document>,
    active: usize,
    storage: Storage,
    paths: Paths,
    config: Config,
    search: Search,
    find_open: bool,
    replace_open: bool,
    menu: Option<Menu>,
    submenu: Option<Menu>,
    settings_open: bool,
    font_picker: bool,
    font_filter: String,
    font_families: Vec<String>,
    about_open: bool,
    go_open: bool,
    go_line: String,
    discard_id: Option<String>,
    error: Option<String>,
    closed: Vec<ClosedDocument>,
    instance: Instance,
    reload: Arc<AtomicBool>,
    _watcher: Option<watch::ConfigWatcher>,
    editor_font: Font,
    editor_size: f32,
    font_names: Vec<&'static str>,
    font_draft: String,
    system_monospace: String,
    theme: Option<Theme>,
    ui_palette: Option<config::Palette>,
    save_queue: std::collections::VecDeque<String>,
    window_size: iced::Size,
    modifiers: keyboard::Modifiers,
}

impl App {
    fn new(paths: Paths, arguments: Vec<PathBuf>, instance: Instance) -> Result<Self, String> {
        let loaded = config::load(&paths);
        if !paths.config_file().exists() {
            config::save(&paths, &loaded.config)?;
        }
        let storage = Storage::open(&paths.session_db())?;
        let mut session = storage.restore_session()?;
        let window_size = storage
            .get_state("window")?
            .and_then(|value| {
                let mut parts = value
                    .split_whitespace()
                    .filter_map(|part| part.parse::<f32>().ok());
                Some(iced::Size::new(
                    parts.next()?.clamp(540.0, 4000.0),
                    parts.next()?.clamp(360.0, 3000.0),
                ))
            })
            .unwrap_or(iced::Size::new(1000.0, 700.0));
        files::refresh_from_disk(&mut session.documents);
        let active = session
            .documents
            .iter()
            .position(|d| Some(&d.id) == session.active_id.as_ref())
            .unwrap_or(0);
        let reload = Arc::new(AtomicBool::new(false));
        let changed = reload.clone();
        let watcher = watch::watch(paths.watch_dirs(), move || {
            changed.store(true, Ordering::Relaxed);
        })
        .ok();
        let mut app = Self {
            docs: session.documents.into_iter().map(Document::new).collect(),
            active,
            storage,
            paths,
            font_draft: loaded.config.editor.font.clone(),
            config: loaded.config,
            error: loaded.error,
            search: Search::default(),
            find_open: false,
            replace_open: false,
            menu: None,
            submenu: None,
            settings_open: false,
            font_picker: false,
            font_filter: String::new(),
            font_families: Vec::new(),
            about_open: false,
            go_open: false,
            go_line: String::new(),
            discard_id: None,
            closed: Vec::new(),
            instance,
            reload,
            _watcher: watcher,
            system_monospace: system_font("monospace").unwrap_or_default(),
            editor_font: Font::MONOSPACE,
            editor_size: 15.0,
            font_names: Vec::new(),
            theme: None,
            ui_palette: None,
            save_queue: std::collections::VecDeque::new(),
            window_size,
            modifiers: keyboard::Modifiers::default(),
        };
        for path in arguments {
            app.open_path(path);
        }
        if app.docs.is_empty() {
            app.new_document();
        }
        for doc in &mut app.docs {
            doc.restore_history(&app.storage)?;
        }
        app.persist()?;
        app.apply_appearance();
        Ok(app)
    }
    fn title(&self) -> String {
        self.docs
            .get(self.active)
            .map(|d| {
                format!(
                    "{}{} — RusTXT",
                    if d.state.dirty { "*" } else { "" },
                    d.state.title
                )
            })
            .unwrap_or_else(|| "RusTXT".into())
    }
    fn theme(&self) -> Option<Theme> {
        self.theme.clone()
    }
    fn apply_appearance(&mut self) {
        let resolved = config::resolve_theme(&self.config.appearance.theme, &self.paths);
        self.ui_palette = resolved.palette.clone();
        self.theme = if let Some(p) = resolved.palette {
            let base = if p.mode == "dark" {
                Theme::Dark
            } else {
                Theme::Light
            }
            .palette();
            Some(Theme::custom(
                "RusTXT",
                iced::theme::Palette {
                    background: p.background.parse().ok().unwrap_or(base.background),
                    text: p.foreground.parse().ok().unwrap_or(base.text),
                    primary: p
                        .accent
                        .as_deref()
                        .and_then(|c| c.parse().ok())
                        .unwrap_or(base.primary),
                    ..base
                },
            ))
        } else {
            match resolved.mode.as_str() {
                "dark" => Some(Theme::Dark),
                "light" => Some(Theme::Light),
                _ => None,
            }
        };
        let description = self.config.editor.font.trim();
        let (family, size) = description
            .rsplit_once(' ')
            .and_then(|(family, size)| {
                size.parse::<f32>()
                    .ok()
                    .map(|size| (family, size * 4.0 / 3.0))
            })
            .unwrap_or((description, 15.0));
        self.editor_size = size.clamp(6.0, 96.0);
        let family = family.split(',').next().unwrap_or("").trim();
        let family = if family.is_empty() {
            self.system_monospace.as_str()
        } else {
            family
        };
        self.editor_font = if family.is_empty() {
            Font::MONOSPACE
        } else {
            let name = self
                .font_names
                .iter()
                .copied()
                .find(|name| *name == family)
                .unwrap_or_else(|| {
                    // Font requires a static family. Intern each distinct preference once.
                    let name = Box::leak(family.to_owned().into_boxed_str());
                    self.font_names.push(name);
                    name
                });
            Font::with_name(name)
        };
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick),
            window::events().map(|(_, event)| Message::Window(event)),
            iced::event::listen_with(|event, status, _| {
                if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
                    return Some(Message::Modifiers(modifiers));
                }
                if matches!(
                    &event,
                    Event::Keyboard(keyboard::Event::KeyPressed {
                        key: Key::Named(Named::Escape),
                        ..
                    })
                ) {
                    return Some(Message::Dismiss);
                }
                if status == iced::event::Status::Ignored {
                    if let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) =
                        event
                    {
                        return shortcut(key.as_ref(), modifiers);
                    }
                }
                None
            }),
        ])
    }
    fn doc(&self) -> &Document {
        &self.docs[self.active]
    }
    fn doc_mut(&mut self) -> &mut Document {
        &mut self.docs[self.active]
    }
    fn report(&mut self, result: Result<(), String>) {
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
    fn refresh_search(&mut self) {
        if let Some(doc) = self.docs.get(self.active) {
            self.search.refresh(&doc.state.content);
        }
    }
    fn persist(&mut self) -> Result<(), String> {
        for doc in &mut self.docs {
            doc.capture_cursor();
            doc.persist(&self.storage)?;
            doc.pending = false;
        }
        let order: Vec<_> = self.docs.iter().map(|d| d.state.id.clone()).collect();
        let active = order.get(self.active).map(String::as_str).unwrap_or("");
        self.storage.update_layout(&order, active)?;
        self.closed = self.storage.closed_documents()?;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        for doc in &mut self.docs {
            if doc.pending {
                doc.persist(&self.storage)?;
                doc.pending = false;
            }
        }
        Ok(())
    }
    fn new_document(&mut self) {
        let state = DocumentState::untitled(
            self.docs.iter().map(|d| d.state.title.as_str()),
            LineEnding::Lf,
        );
        self.docs.push(Document::new(state));
        self.active = self.docs.len() - 1;
    }
    fn open_path(&mut self, path: PathBuf) {
        if let Some(index) = self.docs.iter().position(|d| {
            d.state
                .file_path
                .as_ref()
                .is_some_and(|p| files::same_file(std::path::Path::new(p), &path))
        }) {
            self.active = index;
            return;
        }
        match files::read_document(&path.to_string_lossy()) {
            Ok(state) => {
                self.docs.push(Document::new(state));
                self.active = self.docs.len() - 1;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn write(&mut self, id: &str, path: PathBuf) -> Result<(), String> {
        let Some(index) = self.docs.iter().position(|d| d.state.id == id) else {
            return Ok(());
        };
        let mut state = self.docs[index].state.clone();
        if state
            .file_path
            .as_ref()
            .is_some_and(|p| files::same_file(std::path::Path::new(p), &path))
            && files::changed_on_disk(&state, &path)?
        {
            return Err("This file changed on disk. Use Save As to preserve both versions.".into());
        }
        if self.docs.iter().enumerate().any(|(i, d)| {
            i != index
                && d.state
                    .file_path
                    .as_ref()
                    .is_some_and(|p| files::same_file(std::path::Path::new(p), &path))
        }) {
            return Err("This file is already open in another tab.".into());
        }
        files::save_document(&mut state, &path)?;
        self.docs[index].mark_saved(state);
        self.flush()
    }
    fn save(&mut self, index: usize, picker: bool) -> Task<Message> {
        let doc = &self.docs[index];
        let id = doc.state.id.clone();
        if !picker {
            if let Some(path) = &doc.state.file_path {
                let path = PathBuf::from(path);
                let result = self.write(&id, path);
                self.report(result);
                return Task::none();
            }
        }
        let name = doc.state.title.clone();
        window::oldest().and_then(move |window_id| {
            let id = id.clone();
            let name = name.clone();
            window::run(window_id, move |window| {
                let dialog = rfd::AsyncFileDialog::new()
                    .set_title("Save as")
                    .set_file_name(name)
                    .set_parent(&window);
                async move {
                    Message::SavePicked(id, dialog.save_file().await.map(|f| f.path().to_owned()))
                }
            })
            .then(Task::future)
        })
    }
    fn save_settings(&mut self) -> Task<Message> {
        self.apply_appearance();
        let result = config::save(&self.paths, &self.config);
        self.report(result);
        Task::none()
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        let modal_open = self.settings_open
            || self.about_open
            || self.go_open
            || self.discard_id.is_some()
            || self.error.is_some();
        if modal_open
            && matches!(
                message,
                Message::Action(_)
                    | Message::Undo(_)
                    | Message::Copy(_)
                    | Message::Paste
                    | Message::SelectAll
                    | Message::Delete
                    | Message::Find(_)
                    | Message::FindNext(_)
                    | Message::Date
                    | Message::CloseActive
                    | Message::NextTab(_)
                    | Message::New
                    | Message::Open
            )
        {
            return Task::none();
        }
        match message {
            Message::CloseActive => return self.update(Message::Close(self.active)),
            Message::NextTab(backwards) => {
                return self.update(Message::Select(if backwards {
                    (self.active + self.docs.len() - 1) % self.docs.len()
                } else {
                    (self.active + 1) % self.docs.len()
                }))
            }
            Message::ZoomBy(amount) => {
                return self.update(Message::Zoom(
                    (self.config.appearance.zoom as i32 + amount).clamp(10, 500) as u32,
                ))
            }
            Message::Tick => {
                if let Some(doc) = self.docs.get_mut(self.active) {
                    doc.restore_viewport();
                }
                let result = self.flush();
                self.report(result);
                if self.reload.swap(false, Ordering::Relaxed) {
                    let loaded = config::load(&self.paths);
                    if loaded.error.is_some() {
                        self.error = loaded.error;
                    } else {
                        self.config = loaded.config;
                        self.apply_appearance();
                    }
                }
                match self.instance.receive() {
                    Ok(messages) if !messages.is_empty() => {
                        for paths in messages {
                            for path in paths {
                                self.open_path(path);
                            }
                        }
                        let result = self.persist();
                        self.report(result);
                        return window::oldest().and_then(window::gain_focus);
                    }
                    Err(error) => self.error = Some(error),
                    _ => {}
                }
            }
            Message::Modifiers(modifiers) => self.modifiers = modifiers,
            Message::Action(action) => {
                if self.modifiers.command() {
                    if let text_editor::Action::Scroll { lines } = action {
                        return self.update(Message::ZoomBy(-lines.signum() * 10));
                    }
                }
                let edits = action.is_edit();
                self.doc_mut().action(action);
                if edits {
                    self.refresh_search();
                }
            }
            Message::New => {
                self.menu = None;
                self.new_document();
                let result = self.persist();
                self.report(result);
                return operation::focus(EDITOR);
            }
            Message::Open => {
                self.menu = None;
                return window::oldest().and_then(|id| {
                    window::run(id, |window| {
                        let dialog = rfd::AsyncFileDialog::new()
                            .set_title("Open text files")
                            .set_parent(&window);
                        async move {
                            Message::Opened(dialog.pick_files().await.map(|files| {
                                files.into_iter().map(|f| f.path().to_owned()).collect()
                            }))
                        }
                    })
                    .then(Task::future)
                });
            }
            Message::Opened(paths) => {
                if let Some(paths) = paths {
                    for path in paths {
                        self.open_path(path);
                    }
                }
                let result = self.persist();
                self.report(result);
                self.refresh_search();
                return operation::focus(EDITOR);
            }
            Message::Save(picker) => {
                self.menu = None;
                return self.save(self.active, picker);
            }
            Message::SaveAll => {
                self.menu = None;
                self.save_queue = self
                    .docs
                    .iter()
                    .filter(|d| d.state.dirty || d.state.file_path.is_none())
                    .map(|d| d.state.id.clone())
                    .collect();
                return self.update(Message::SaveNext);
            }
            Message::SaveNext => {
                while let Some(id) = self.save_queue.pop_front() {
                    let Some(index) = self.docs.iter().position(|d| d.state.id == id) else {
                        continue;
                    };
                    if let Some(path) = self.docs[index].state.file_path.clone() {
                        if let Err(error) = self.write(&id, PathBuf::from(path)) {
                            self.error = Some(error);
                            self.save_queue.clear();
                            break;
                        }
                    } else {
                        return self.save(index, true);
                    }
                }
            }
            Message::SavePicked(id, path) => {
                match path {
                    Some(path) => {
                        if let Err(error) = self.write(&id, path) {
                            self.error = Some(error);
                            self.save_queue.clear();
                        } else if !self.save_queue.is_empty() {
                            return self.update(Message::SaveNext);
                        }
                    }
                    None => self.save_queue.clear(),
                }
                return operation::focus(EDITOR);
            }
            Message::Select(index) => {
                if index < self.docs.len() {
                    self.active = index;
                }
                self.menu = None;
                self.refresh_search();
                let result = self.persist();
                self.report(result);
                return operation::focus(EDITOR);
            }
            Message::Close(index) => {
                if index >= self.docs.len() {
                    return Task::none();
                }
                if let Err(error) = self.persist() {
                    self.error = Some(error);
                    return Task::none();
                }
                if let Err(error) = self
                    .storage
                    .close_document(&self.docs[index].state.id, false)
                {
                    self.error = Some(error);
                    return Task::none();
                }
                self.docs.remove(index);
                if self.active > index {
                    self.active -= 1;
                }
                self.active = self.active.min(self.docs.len().saturating_sub(1));
                let result = self.persist();
                self.report(result);
                if self.docs.is_empty() {
                    return iced::exit();
                }
                self.refresh_search();
                return operation::focus(EDITOR);
            }
            Message::Quit => {
                if let Err(error) = self.persist() {
                    self.error = Some(error);
                } else {
                    return iced::exit();
                }
            }
            Message::ToggleReplace => {
                self.replace_open = !self.replace_open;
            }
            Message::CloseMenu => {
                self.menu = None;
                return operation::focus(EDITOR);
            }
            Message::Submenu(menu) => self.submenu = menu,
            Message::Menu(menu) => {
                self.submenu = None;
                self.menu = if self.menu == Some(menu) {
                    None
                } else {
                    Some(menu)
                };
            }
            Message::Dismiss => {
                if self.font_picker {
                    self.font_picker = false;
                    self.font_draft = self.config.editor.font.clone();
                    return Task::none();
                }
                self.menu = None;
                if self.settings_open && self.font_draft != self.config.editor.font {
                    self.config.editor.font = self.font_draft.clone();
                    let _ = self.save_settings();
                }
                self.settings_open = false;
                self.about_open = false;
                self.go_open = false;
                self.discard_id = None;
                self.error = None;
                self.find_open = false;
                return operation::focus(EDITOR);
            }
            Message::Find(replace) => {
                self.menu = None;
                let selection = self.doc().selected_range();
                let selected = &self.doc().state.content[selection];
                if !selected.is_empty() && !selected.contains('\n') {
                    self.search.query = selected.to_owned();
                }
                self.find_open = true;
                self.replace_open |= replace;
                self.refresh_search();
                return operation::focus(if replace { REPLACE } else { FIND });
            }
            Message::Query(query) => {
                self.search.query = query;
                self.refresh_search();
            }
            Message::Replacement(value) => self.search.replacement = value,
            Message::MatchCase(value) => {
                self.search.case_sensitive = value;
                self.refresh_search();
            }
            Message::WholeWord(value) => {
                self.search.whole_word = value;
                self.refresh_search();
            }
            Message::Regex(value) => {
                self.search.regex = value;
                self.refresh_search();
            }
            Message::FindNext(backwards) => {
                self.menu = None;
                self.submenu = None;
                if let Some(range) = self.search.next(self.doc().selected_range(), backwards) {
                    self.doc_mut().select(range);
                }
                return operation::focus(EDITOR);
            }
            Message::Replace => {
                let range = self.doc().selected_range();
                if self.search.matches.contains(&range) {
                    let replacement = self
                        .search
                        .replacement_for(&self.doc().state.content, range.clone());
                    self.doc_mut().replace(range, &replacement);
                    self.refresh_search();
                }
                return self.update(Message::FindNext(false));
            }
            Message::ReplaceAll => {
                if !self.search.query.is_empty() {
                    match self.search.replace_all(&self.doc().state.content) {
                        Ok(next) => {
                            let len = self.doc().state.content.len();
                            self.doc_mut().replace(0..len, &next);
                            self.refresh_search();
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            Message::Undo(redo) => {
                self.menu = None;
                self.doc_mut().undo(redo);
                self.refresh_search();
                return operation::focus(EDITOR);
            }
            Message::Copy(cut) => {
                self.menu = None;
                if let Some(value) = self.doc().content.selection() {
                    if cut {
                        let range = self.doc().selected_range();
                        self.doc_mut().replace(range, "");
                        self.refresh_search();
                    }
                    return iced::clipboard::write(value);
                }
            }
            Message::Paste => {
                self.menu = None;
                return iced::clipboard::read().map(Message::Pasted);
            }
            Message::Pasted(value) => {
                if let Some(value) = value {
                    let range = self.doc().selected_range();
                    self.doc_mut().replace(range, &files::normalize(&value));
                    self.refresh_search();
                }
                return operation::focus(EDITOR);
            }
            Message::SelectAll => {
                self.menu = None;
                self.doc_mut()
                    .content
                    .perform(text_editor::Action::SelectAll);
                return operation::focus(EDITOR);
            }
            Message::Delete => {
                self.menu = None;
                self.doc_mut()
                    .action(text_editor::Action::Edit(text_editor::Edit::Delete));
                self.refresh_search();
                return operation::focus(EDITOR);
            }
            Message::ReopenLast => {
                if let Some(doc) = self.closed.first() {
                    return self.update(Message::Reopen(doc.id.clone()));
                }
            }
            Message::Reopen(id) => {
                self.menu = None;
                match self.storage.reopen_document(&id) {
                    Ok(Some(mut state)) => {
                        state.ensure_unique_title(self.docs.iter().map(|d| d.state.title.as_str()));
                        let mut states = vec![state];
                        files::refresh_from_disk(&mut states);
                        for state in states {
                            let mut doc = Document::new(state);
                            let result = doc.restore_history(&self.storage);
                            self.report(result);
                            self.docs.push(doc);
                            self.active = self.docs.len() - 1;
                        }
                        let result = self.persist();
                        self.report(result);
                    }
                    Err(error) => self.error = Some(error),
                    _ => {}
                }
                return operation::focus(EDITOR);
            }
            Message::Discard => {
                self.menu = None;
                self.discard_id = Some(self.doc().state.id.clone());
            }
            Message::ConfirmDiscard(id) => {
                if let Some(index) = self.docs.iter().position(|d| d.state.id == id) {
                    if let Err(error) = self.storage.close_document(&id, true) {
                        self.error = Some(error);
                        return Task::none();
                    }
                    self.docs.remove(index);
                    self.active = self.active.min(self.docs.len().saturating_sub(1));
                    self.discard_id = None;
                    let result = self.persist();
                    self.report(result);
                    if self.docs.is_empty() {
                        return iced::exit();
                    }
                }
            }
            Message::Settings => {
                self.menu = None;
                self.font_draft = self.config.editor.font.clone();
                self.settings_open = true;
            }
            Message::About => {
                self.menu = None;
                self.about_open = true;
            }
            Message::Wrap(value) => {
                self.menu = None;
                self.submenu = None;
                self.config.editor.word_wrap = value;
                return self.save_settings();
            }
            Message::Status(value) => {
                self.menu = None;
                self.submenu = None;
                self.config.window.status_bar = value;
                return self.save_settings();
            }
            Message::Theme(value) => {
                self.config.appearance.theme = value;
                return self.save_settings();
            }
            Message::Font(value) => self.font_draft = value,
            Message::ChooseFont => {
                self.font_filter.clear();
                self.font_picker = true;
                if self.font_families.is_empty() {
                    let mut system = iced_graphics::text::font_system()
                        .write()
                        .expect("font system");
                    self.font_families = system
                        .raw()
                        .db()
                        .faces()
                        .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
                        .collect();
                    self.font_families.sort_unstable();
                    self.font_families.dedup();
                }
            }
            Message::FontFilter(value) => self.font_filter = value,
            Message::FontChosen(value) => {
                let size = self
                    .font_draft
                    .rsplit_once(' ')
                    .and_then(|(_, size)| size.parse::<u32>().ok())
                    .unwrap_or(12);
                self.font_draft = format!("{value} {size}");
            }
            Message::ResetFont => {
                self.font_draft.clear();
                self.config.editor.font.clear();
                return self.save_settings();
            }
            Message::ApplyFont => {
                self.font_picker = false;
                self.config.editor.font = self.font_draft.clone();
                return self.save_settings();
            }
            Message::Zoom(value) => {
                self.menu = None;
                self.submenu = None;
                self.config.appearance.zoom = value.clamp(10, 500);
                return self.save_settings();
            }
            Message::Titlebar(value) => {
                let old = self
                    .config
                    .window
                    .title_bar
                    .decorated(desktop::running_on_tiling_compositor());
                self.config.window.title_bar = match value.as_str() {
                    "show" => desktop::TitlebarMode::Show,
                    "hide" => desktop::TitlebarMode::Hide,
                    _ => desktop::TitlebarMode::Auto,
                };
                let _ = self.save_settings();
                if old
                    != self
                        .config
                        .window
                        .title_bar
                        .decorated(desktop::running_on_tiling_compositor())
                {
                    return window::oldest().and_then(window::toggle_decorations);
                }
            }
            Message::GoTo => {
                self.menu = None;
                self.go_open = true;
                self.go_line.clear();
                return operation::focus("go-line");
            }
            Message::GoLine(value) => self.go_line = value,
            Message::Go => {
                if let Ok(line) = self.go_line.parse::<usize>() {
                    let line = line
                        .saturating_sub(1)
                        .min(self.doc().content.line_count().saturating_sub(1));
                    let offset = document::byte_offset(
                        &self.doc().state.content,
                        text_editor::Position { line, column: 0 },
                    );
                    self.doc_mut().select(offset..offset);
                    self.go_open = false;
                    return operation::focus(EDITOR);
                }
            }
            Message::Date => {
                self.menu = None;
                let value = chrono::Local::now().format("%H:%M %Y-%m-%d").to_string();
                let range = self.doc().selected_range();
                self.doc_mut().replace(range, &value);
                return operation::focus(EDITOR);
            }
            Message::Print => {
                self.menu = None;
                let escaped = self
                    .doc()
                    .state
                    .content
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                let html = format!("<!doctype html><meta charset=utf-8><title>RusTXT print</title><style>body{{font:12pt monospace}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;unicode-bidi:plaintext}}@page{{margin:20mm}}</style><pre>{escaped}</pre><script>onload=()=>print()</script>");
                let path = self.paths.cache_dir.join("print.html");
                let result = std::fs::create_dir_all(&self.paths.cache_dir)
                    .map_err(|e| e.to_string())
                    .and_then(|_| files::atomic_save(&path, html.as_bytes()))
                    .and_then(|_| {
                        webbrowser::open(&path.to_string_lossy()).map_err(|e| e.to_string())
                    });
                self.report(result);
            }
            Message::OpenLink(url) => {
                let result = webbrowser::open(&url).map_err(|e| e.to_string());
                self.report(result);
            }
            Message::Window(window::Event::Resized(size)) => {
                self.window_size = size;
                let result = self.storage.set_state(
                    "window",
                    &format!("{} {} 0", size.width as u32, size.height as u32),
                );
                self.report(result);
            }
            Message::Window(window::Event::CloseRequested) => return self.update(Message::Quit),
            Message::Window(window::Event::FileDropped(path)) => {
                self.open_path(path);
                let result = self.persist();
                self.report(result);
            }
            Message::Window(_) => {}
        }
        Task::none()
    }
    fn view(&self) -> Element<'_, Message> {
        if self.docs.is_empty() {
            return container(text("Closing…")).into();
        }
        let palette = self.ui_palette.as_ref();
        let flat = move |theme: &Theme, status| {
            crate::style::Colors::new(theme, palette).flat(status, false)
        };
        let input_style = move |theme: &Theme, status| {
            crate::style::Colors::new(theme, palette).input(theme, status)
        };
        let menus = [
            ("File", Menu::File),
            ("Edit", Menu::Edit),
            ("View", Menu::View),
            ("Settings", Menu::Settings),
        ]
        .into_iter()
        .fold(row![].spacing(0), |menus, (label, menu)| {
            let selected = self.menu == Some(menu);
            menus.push(
                button(text(label).size(UI))
                    .padding([5, 10])
                    .on_press(Message::Menu(menu))
                    .style(move |theme, status| {
                        crate::style::Colors::new(theme, palette).flat(status, selected)
                    }),
            )
        });
        let menus = container(menus)
            .padding([0, 4])
            .width(Fill)
            .height(31)
            .style(move |theme| crate::style::Colors::new(theme, palette).bar());
        let tabs = self
            .docs
            .iter()
            .enumerate()
            .fold(row![].spacing(4), |tabs, (index, doc)| {
                let selected = index == self.active;
                let dot = container(space::horizontal().width(14).height(14)).style(move |theme| {
                    container::Style {
                        background: doc
                            .state
                            .dirty
                            .then(|| crate::style::Colors::new(theme, palette).foreground.into()),
                        border: iced::Border {
                            radius: 7.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                });
                let title: String = doc.state.title.chars().take(23).collect();
                let title = if title.len() < doc.state.title.len() {
                    format!("{title}…")
                } else {
                    title
                };
                let base = button(
                    row![
                        dot,
                        text(title).size(UI).width(Fill).align_x(iced::Center),
                        space::horizontal().width(22)
                    ]
                    .height(Fill)
                    .align_y(iced::Center)
                    .spacing(8),
                )
                .padding([0, 10])
                .width(216)
                .height(34)
                .on_press(Message::Select(index))
                .style(move |theme, status| {
                    crate::style::Colors::new(theme, palette).flat(status, selected)
                });
                let close = container(
                    button(container(icon(Icon::Close)).center(Fill))
                        .width(24)
                        .height(24)
                        .padding(0)
                        .style(flat)
                        .on_press(Message::Close(index)),
                )
                .align_right(Fill)
                .center_y(Fill)
                .padding([0, 6]);
                let tab: Element<'_, Message> = if selected {
                    widget::stack![base, close].into()
                } else {
                    widget::hover(base, close)
                };
                tabs.push(hint(
                    widget::mouse_area(tab).on_middle_press(Message::Close(index)),
                    &doc.state.title,
                    palette,
                ))
            });
        let tabs = container(
            row![
                scrollable(tabs)
                    .direction(scrollable::Direction::Horizontal(
                        scrollable::Scrollbar::new().width(2).scroller_width(2)
                    ))
                    .width(Fill)
                    .height(34),
                button(container(icon(Icon::Plus)).center(Fill))
                    .width(36)
                    .height(34)
                    .padding(0)
                    .style(flat)
                    .on_press(Message::New)
            ]
            .align_y(iced::Center)
            .spacing(8),
        )
        .padding(iced::Padding {
            top: 7.0,
            bottom: 6.0,
            left: 6.0,
            right: 6.0,
        })
        .height(47)
        .width(Fill)
        .style(move |theme| crate::style::Colors::new(theme, palette).bar());
        let doc = self.doc();
        let editor = text_editor(&doc.content)
            .id(EDITOR)
            .height(Fill)
            .padding(iced::Padding {
                top: 8.0,
                right: 12.0,
                bottom: 24.0,
                left: 12.0,
            })
            .font(self.editor_font)
            .size(self.editor_size * self.config.appearance.zoom as f32 / 100.0)
            .wrapping(if self.config.editor.word_wrap {
                text::Wrapping::WordOrGlyph
            } else {
                text::Wrapping::None
            })
            .on_action(Message::Action)
            .style(move |theme, status| {
                let colors = crate::style::Colors::new(theme, palette);
                let mut style = text_editor::default(theme, status);
                style.border = iced::Border::default();
                style.background = colors.background.into();
                style.value = colors.foreground;
                style.selection = colors.selection;
                style
            })
            .key_binding(|press| {
                shortcut(press.key.as_ref(), press.modifiers)
                    .map(text_editor::Binding::Custom)
                    .or_else(|| text_editor::Binding::from_key_press(press))
            });
        let editor: Element<'_, Message> = if self.find_open {
            let count = if self.search.error.is_some() {
                "!".to_owned()
            } else if self.search.query.is_empty() {
                String::new()
            } else if self.search.matches.is_empty() {
                "No results".to_owned()
            } else if let Some(index) = self
                .search
                .matches
                .iter()
                .position(|range| *range == doc.selected_range())
            {
                format!("{} of {}", index + 1, self.search.matches.len())
            } else {
                self.search.matches.len().to_string()
            };
            let count_width = count.chars().count() as f32 * CAPTION * 0.65;
            let option = |label, value, message| {
                let description = match label {
                    "Aa" => "Match case: distinguish uppercase and lowercase",
                    "ab" => "Match whole words only",
                    _ => "Use regular expressions",
                };
                hint(
                    button(
                        text(label)
                            .size(CAPTION)
                            .font(bold_font())
                            .width(Fill)
                            .height(Fill)
                            .center(),
                    )
                    .width(50)
                    .height(34)
                    .padding(0)
                    .on_press(message)
                    .style(move |theme, status| {
                        crate::style::Colors::new(theme, palette).flat(status, value)
                    }),
                    description,
                    palette,
                )
            };
            let tool = |kind, message| {
                let description = match &message {
                    Message::ToggleReplace if self.replace_open => "Hide replacement controls",
                    Message::ToggleReplace => "Show replacement controls (Ctrl+H)",
                    Message::FindNext(true) => "Find previous match (Shift+F3)",
                    Message::FindNext(false) => "Find next match (F3)",
                    _ => "Close Find and Replace (Esc)",
                };
                hint(
                    button(container(icon(kind)).center(Fill))
                        .width(34)
                        .height(34)
                        .padding(0)
                        .style(flat)
                        .on_press(message),
                    description,
                    palette,
                )
            };
            let field = text_input("Find", &self.search.query)
                .id(FIND)
                .on_input(Message::Query)
                .on_submit(Message::FindNext(false))
                .size(UI)
                .padding(iced::Padding {
                    left: 30.0,
                    right: 28.0,
                    top: 6.0,
                    bottom: 6.0,
                })
                .style(input_style);
            let mut field = widget::stack![
                field,
                container(icon(Icon::Search)).padding([0, 8]).center_y(Fill)
            ];
            if !self.search.query.is_empty() {
                field = field.push(
                    container(hint(
                        button(container(icon(Icon::Close)).center(Fill))
                            .width(24)
                            .height(28)
                            .padding(0)
                            .style(flat)
                            .on_press(Message::Query(String::new())),
                        "Clear search",
                        palette,
                    ))
                    .align_right(Fill)
                    .center_y(Fill)
                    .padding([0, 4]),
                );
            }
            let field = container(field).width(212).height(34);
            let find_row = row![
                tool(
                    if self.replace_open {
                        Icon::Down
                    } else {
                        Icon::Right
                    },
                    Message::ToggleReplace
                ),
                field,
                container(text(count).size(CAPTION))
                    .width(count_width + 8.0)
                    .padding([0, 4]),
                tool(Icon::Up, Message::FindNext(true)),
                tool(Icon::Down, Message::FindNext(false)),
                option(
                    "Aa",
                    self.search.case_sensitive,
                    Message::MatchCase(!self.search.case_sensitive)
                ),
                option(
                    "ab",
                    self.search.whole_word,
                    Message::WholeWord(!self.search.whole_word)
                ),
                option(".*", self.search.regex, Message::Regex(!self.search.regex)),
                tool(Icon::Close, Message::Dismiss)
            ]
            .spacing(2)
            .align_y(iced::Center);
            let mut find = column![find_row].spacing(6);
            if self.replace_open {
                let raised = move |theme: &Theme, status| {
                    let colors = crate::style::Colors::new(theme, palette);
                    let mut style = colors.flat(status, true);
                    if status == button::Status::Active {
                        style.background = Some(colors.control.into());
                    }
                    style
                };
                find = find.push(
                    row![
                        space::horizontal().width(30),
                        text_input("Replace with", &self.search.replacement)
                            .id(REPLACE)
                            .on_input(Message::Replacement)
                            .on_submit(Message::Replace)
                            .size(UI)
                            .padding([6, 8])
                            .style(input_style),
                        hint(
                            button(text("Replace").font(bold_font()))
                                .height(34)
                                .padding([6, 16])
                                .style(raised)
                                .on_press(Message::Replace),
                            "Replace current match and find the next",
                            palette
                        ),
                        hint(
                            button(text("Replace all").font(bold_font()))
                                .height(34)
                                .padding([6, 16])
                                .style(raised)
                                .on_press(Message::ReplaceAll),
                            "Replace all matches in this document",
                            palette
                        )
                    ]
                    .spacing(4)
                    .align_y(iced::Center),
                );
            } else {
                find = find.push(space::vertical().height(0));
            }
            let card = container(find)
                .width(534.0 + count_width)
                .padding(6)
                .style(move |theme| crate::style::Colors::new(theme, palette).panel());
            widget::stack![
                editor,
                container(widget::opaque(card))
                    .align_right(Fill)
                    .padding([8, 18])
            ]
            .into()
        } else {
            editor.into()
        };
        let separator = || {
            container(space::horizontal().height(1))
                .width(Fill)
                .style(move |theme| container::Style {
                    background: Some(
                        crate::style::Colors::new(theme, palette)
                            .border
                            .scale_alpha(0.5)
                            .into(),
                    ),
                    ..Default::default()
                })
        };
        let mut body = column![tabs, separator(), menus, editor];
        if self.config.window.status_bar {
            let cursor = doc.content.cursor().position;
            let column = doc
                .content
                .line(cursor.line)
                .map(|l| l.text[..cursor.column.min(l.text.len())].chars().count())
                .unwrap_or(0);
            let divider = || {
                container(space::horizontal().width(1).height(10)).style(move |theme| {
                    container::Style {
                        background: Some(crate::style::Colors::new(theme, palette).border.into()),
                        ..Default::default()
                    }
                })
            };
            let ending = match doc.state.line_ending {
                LineEnding::Lf => "Unix (LF)",
                LineEnding::Crlf => "Windows (CRLF)",
            };
            body = body.push(separator()).push(
                container(
                    row![
                        text(format!("Ln {}, Col {}", cursor.line + 1, column + 1)).size(CAPTION),
                        space::horizontal(),
                        divider(),
                        text(format!("{} characters", doc.state.content.chars().count()))
                            .size(CAPTION),
                        divider(),
                        text(format!("{}%", self.config.appearance.zoom)).size(CAPTION),
                        divider(),
                        text(ending).size(CAPTION),
                        divider(),
                        text("UTF-8").size(CAPTION)
                    ]
                    .spacing(10)
                    .align_y(iced::Center),
                )
                .padding([3, 12])
                .width(Fill)
                .style(move |theme| {
                    let colors = crate::style::Colors::new(theme, palette);
                    container::Style {
                        text_color: Some(colors.foreground.scale_alpha(0.75)),
                        ..colors.bar()
                    }
                }),
            );
        }
        let base: Element<'_, Message> = container(body).height(Fill).width(Fill).into();
        if let Some(error) = &self.error {
            return modal(
                base,
                palette,
                column![
                    text("Could not complete the action").size(20),
                    text(error),
                    button("Close").on_press(Message::Dismiss)
                ]
                .spacing(16),
            );
        }
        if let Some(id) = &self.discard_id {
            return modal(
                base,
                palette,
                column![
                    text("Discard this tab permanently?").size(20),
                    text("Its recovery copy will be deleted. This cannot be undone."),
                    row![
                        button("Cancel").on_press(Message::Dismiss),
                        button("Discard")
                            .on_press(Message::ConfirmDiscard(id.clone()))
                            .style(button::danger)
                    ]
                    .spacing(12)
                ]
                .spacing(16),
            );
        }
        if self.go_open {
            return modal(
                base,
                palette,
                column![
                    text("Go to line").size(20),
                    text_input("Line number", &self.go_line)
                        .id("go-line")
                        .on_input(Message::GoLine)
                        .on_submit(Message::Go),
                    row![
                        button("Cancel").on_press(Message::Dismiss),
                        button("Go").on_press(Message::Go)
                    ]
                    .spacing(12)
                ]
                .spacing(16),
            );
        }
        if self.settings_open {
            let titlebar = match self.config.window.title_bar {
                desktop::TitlebarMode::Auto => "auto",
                desktop::TitlebarMode::Show => "show",
                desktop::TitlebarMode::Hide => "hide",
            };
            let mut themes: Vec<String> = ["auto", "system", "light", "dark", "omarchy"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            if let Ok(entries) = std::fs::read_dir(self.paths.themes_dir()) {
                for entry in entries.flatten() {
                    if entry.path().extension().is_some_and(|e| e == "toml") {
                        themes.push(
                            entry
                                .path()
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                        );
                    }
                }
            }
            let choice = |value: &str| Choice {
                value: value.to_owned(),
                label: match value {
                    "auto" => "Automatic",
                    "system" => "System",
                    "light" => "Light",
                    "dark" => "Dark",
                    "omarchy" => "Omarchy",
                    "show" => "Always show",
                    "hide" => "Always hide",
                    _ => value,
                }
                .to_owned(),
            };
            let pick_style = move |theme: &Theme, status| {
                let colors = crate::style::Colors::new(theme, palette);
                let mut style = pick_list::default(theme, status);
                style.background = iced::Color::TRANSPARENT.into();
                style.border = iced::Border::default();
                style.text_color = colors.foreground;
                style.handle_color = colors.foreground;
                style
            };
            let separator = || {
                container(space::horizontal().height(1))
                    .width(Fill)
                    .style(move |theme| container::Style {
                        background: Some(
                            crate::style::Colors::new(theme, palette)
                                .chrome
                                .scale_alpha(0.45)
                                .into(),
                        ),
                        ..Default::default()
                    })
            };
            let theme_choice = |value: &str| {
                let mut result = choice(value);
                result.label = match value {
                    "auto" if self.paths.omarchy_available() => "Automatic (Omarchy theme)".into(),
                    "auto" => "Automatic (system setting)".into(),
                    "system" => "Use system setting".into(),
                    "omarchy" => "Follow Omarchy theme".into(),
                    "light" | "dark" => result.label,
                    _ => format!("Custom: {value}"),
                };
                result
            };
            let appearance = settings_group(
                palette,
                column![
                    setting(
                        palette,
                        "App theme",
                        if self.config.appearance.theme == "auto" && self.paths.omarchy_available()
                        {
                            "Following the active Omarchy theme".into()
                        } else {
                            String::new()
                        },
                        pick_list(
                            themes
                                .iter()
                                .map(|value| theme_choice(value))
                                .collect::<Vec<_>>(),
                            Some(theme_choice(&self.config.appearance.theme)),
                            |choice: Choice| Message::Theme(choice.value)
                        )
                        .text_size(UI)
                        .width(220)
                        .padding(4)
                        .style(pick_style)
                        .into()
                    ),
                    separator(),
                    setting(
                        palette,
                        "Window title bar",
                        if desktop::running_on_tiling_compositor() {
                            "Automatic hides it here because a tiling compositor is running".into()
                        } else {
                            "Automatic follows your desktop".into()
                        },
                        pick_list(
                            [choice("auto"), choice("show"), choice("hide")],
                            Some(choice(titlebar)),
                            |choice: Choice| Message::Titlebar(choice.value)
                        )
                        .text_size(UI)
                        .padding(4)
                        .style(pick_style)
                        .into()
                    )
                ]
                .into(),
            );
            let round = move |theme: &Theme, status| {
                let mut style = crate::style::Colors::new(theme, palette).flat(status, true);
                style.border.radius = 16.0.into();
                if matches!(status, button::Status::Active) {
                    style.background =
                        Some(crate::style::Colors::new(theme, palette).control.into());
                }
                style
            };
            let zoom = row![
                text(format!("{}", self.config.appearance.zoom)).size(UI),
                button(container(icon(Icon::Minus)).center(Fill))
                    .width(30)
                    .height(30)
                    .padding(0)
                    .style(round)
                    .on_press(Message::ZoomBy(-10)),
                button(container(icon(Icon::Plus)).center(Fill))
                    .width(30)
                    .height(30)
                    .padding(0)
                    .style(round)
                    .on_press(Message::ZoomBy(10))
            ]
            .spacing(8)
            .align_y(iced::Center);
            let font_label = if self.config.editor.font.is_empty() {
                "Sans Regular 12".to_owned()
            } else {
                let (family, size) = self
                    .config
                    .editor
                    .font
                    .rsplit_once(' ')
                    .unwrap_or((&self.config.editor.font, "12"));
                format!("{family} Regular {size}")
            };
            let font = row![
                button(text(font_label).size(UI).font(bold_font()))
                    .padding([8, 10])
                    .style(move |theme, status| {
                        let colors = crate::style::Colors::new(theme, palette);
                        let mut style = colors.flat(status, true);
                        if status == button::Status::Active {
                            style.background = Some(colors.control.into());
                        }
                        style
                    })
                    .on_press(Message::ChooseFont),
                button(text("System font").size(UI).font(bold_font()))
                    .padding([8, 10])
                    .style(flat)
                    .on_press_maybe(
                        (!self.config.editor.font.is_empty()).then_some(Message::ResetFont)
                    )
            ]
            .spacing(12)
            .align_y(iced::Center);
            let toggle_style = |theme: &Theme, status| {
                let mut style = toggler::default(theme, status);
                style.foreground = iced::Color::WHITE.into();
                style
            };
            let text_settings = settings_group(
                palette,
                column![
                    setting(
                        palette,
                        "Zoom",
                        "Percent. Also Ctrl + mouse wheel, Ctrl + plus and Ctrl + minus".into(),
                        zoom.into()
                    ),
                    separator(),
                    setting(
                        palette,
                        "Font",
                        if self.config.editor.font.is_empty() { "Following the system monospace font, with Noto Naskh Arabic for Arabic text.".into() } else { "Paired with Noto Naskh Arabic for Arabic text. List two families in config.toml to pick another, e.g. \"JetBrainsMono Nerd Font, Amiri 12\"".into() },
                        font.into()
                    ),
                    separator(),
                    setting(
                        palette,
                        "Word wrap",
                        "Wrap long lines to the window width".into(),
                        toggler(self.config.editor.word_wrap)
                            .size(24)
                            .style(toggle_style)
                            .on_toggle(Message::Wrap)
                            .into()
                    ),
                    separator(),
                    setting(
                        palette,
                        "Status bar",
                        "Line and column, character count, zoom, line endings and encoding".into(),
                        toggler(self.config.window.status_bar)
                            .size(24)
                            .style(toggle_style)
                            .on_toggle(Message::Status)
                            .into()
                    )
                ]
                .into(),
            );
            let paths = settings_group(
                palette,
                column![
                    setting(
                        palette,
                        "Settings",
                        self.paths.config_file().display().to_string(),
                        space::horizontal().width(0).into()
                    ),
                    separator(),
                    setting(
                        palette,
                        "Custom themes",
                        format!("{}/  — TOML files with mode, background, foreground and optional accent, muted, selection, border, chrome, menu", self.paths.themes_dir().display()),
                        space::horizontal().width(0).into()
                    )
                ]
                .into(),
            );
            let contents = column![
                text("Appearance").size(UI).font(bold_font()),
                appearance,
                space::vertical().height(12),
                text("Text").size(UI).font(bold_font()),
                text_settings,
                space::vertical().height(12),
                text("Configuration files").size(UI).font(bold_font()),
                text("Edits made to these files, and Omarchy theme changes, apply immediately.")
                    .size(UI)
                    .style(move |theme| text::Style {
                        color: Some(
                            crate::style::Colors::new(theme, palette)
                                .foreground
                                .scale_alpha(0.55)
                        )
                    }),
                paths
            ]
            .spacing(12);
            let header = row![
                space::horizontal().width(30),
                text("Settings")
                    .size(UI)
                    .font(bold_font())
                    .width(Fill)
                    .align_x(iced::Center),
                button(container(icon(Icon::Close)).center(Fill))
                    .width(30)
                    .height(30)
                    .padding(0)
                    .style(round)
                    .on_press(Message::Dismiss)
            ]
            .align_y(iced::Center);
            let panel = container(column![
                container(header).padding([8, 10]),
                scrollable(container(contents).padding(iced::Padding {
                    top: 30.0,
                    bottom: 24.0,
                    left: 48.0,
                    right: 48.0
                }))
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::new().width(3).scroller_width(3)
                ))
                .height(Fill)
            ])
            .width(640)
            .height(Fill)
            .max_height(786)
            .style(move |theme| {
                let colors = crate::style::Colors::new(theme, palette);
                container::Style {
                    background: Some(colors.chrome.into()),
                    border: iced::Border {
                        color: colors.border.scale_alpha(0.5),
                        width: 1.0,
                        radius: 14.0.into(),
                    },
                    ..colors.panel()
                }
            });
            let panel: Element<'_, Message> = if self.font_picker {
                let filter = self.font_filter.to_lowercase();
                let families = self
                    .font_families
                    .iter()
                    .filter(|name| name.to_lowercase().contains(&filter))
                    .fold(column![].spacing(2), |list, family| {
                        list.push(
                            button(text(family).size(UI))
                                .width(Fill)
                                .padding([8, 12])
                                .style(flat)
                                .on_press(Message::FontChosen(family.clone())),
                        )
                    });
                container(
                    column![
                        row![
                            button("Cancel").style(flat).on_press(Message::Dismiss),
                            text("Select Font")
                                .font(bold_font())
                                .width(Fill)
                                .align_x(iced::Center),
                            button("Select").on_press(Message::ApplyFont)
                        ]
                        .align_y(iced::Center),
                        text_input("Search fonts", &self.font_filter)
                            .style(input_style)
                            .padding(10)
                            .on_input(Message::FontFilter),
                        scrollable(families).height(Fill),
                        text_input("Font family and size", &self.font_draft)
                            .style(input_style)
                            .padding(10)
                            .on_input(Message::Font)
                            .on_submit(Message::ApplyFont)
                    ]
                    .spacing(12),
                )
                .padding(16)
                .width(640)
                .height(Fill)
                .max_height(650)
                .style(move |theme| crate::style::Colors::new(theme, palette).panel())
                .into()
            } else {
                panel.into()
            };
            return widget::stack![
                base,
                widget::opaque(container(panel).padding(24).center(Fill).style(|_| {
                    container::Style {
                        background: Some(iced::Color::BLACK.scale_alpha(0.4).into()),
                        ..Default::default()
                    }
                }))
            ]
            .into();
        }

        if self.about_open {
            return modal(
                base,
                palette,
                column![
                    text("RusTXT").size(30),
                    text(format!("Version {VERSION}")),
                    text("A dead simple, recoverable plain text editor."),
                    text("Built with Rust and Iced. MIT license."),
                    button("View latest release").on_press(Message::OpenLink(format!(
                        "{}/releases/latest",
                        rustxt_core::update::REPOSITORY
                    ))),
                    button("Close").on_press(Message::Dismiss)
                ]
                .spacing(16),
            );
        }
        if let Some(menu) = self.menu {
            let left = match menu {
                Menu::File => 4.0,
                Menu::Edit => 50.0,
                Menu::View => 97.0,
                _ => 151.0,
            };
            let mut layers = widget::stack![
                base,
                widget::opaque(
                    widget::mouse_area(container(space::vertical()).width(Fill).height(Fill))
                        .on_press(Message::CloseMenu)
                ),
                container(widget::opaque(self.menu_panel(menu))).padding(iced::Padding {
                    top: 79.0,
                    left,
                    right: 0.0,
                    bottom: 0.0
                })
            ];
            if let Some(submenu) = self.submenu {
                let (x, y) = match submenu {
                    Menu::Recent => (left + 262.0, 149.0),
                    _ => (left + 134.0, 79.0),
                };
                layers = layers.push(container(widget::opaque(self.menu_panel(submenu))).padding(
                    iced::Padding {
                        top: y,
                        left: x,
                        right: 0.0,
                        bottom: 0.0,
                    },
                ));
            }
            return layers.into();
        }

        base
    }
    fn menu_panel(&self, menu: Menu) -> Element<'_, Message> {
        let palette = self.ui_palette.as_ref();
        let doc = self.doc();
        let entries = match menu {
            Menu::File => vec![
                ("New tab", "Ctrl+N", Message::New),
                ("Open…", "Ctrl+O", Message::Open),
                ("Recently closed", "", Message::Submenu(Some(Menu::Recent))),
                ("", "", Message::CloseMenu),
                ("Save", "Ctrl+S", Message::Save(false)),
                ("Save as…", "Shift+Ctrl+S", Message::Save(true)),
                ("Save all", "Ctrl+Alt+S", Message::SaveAll),
                ("", "", Message::CloseMenu),
                ("Print…", "Ctrl+P", Message::Print),
                ("", "", Message::CloseMenu),
                ("Close tab", "Ctrl+W", Message::Close(self.active)),
                ("Discard changes and close", "", Message::Discard),
                ("Reopen closed tab", "Shift+Ctrl+T", Message::ReopenLast),
                ("", "", Message::CloseMenu),
                ("Exit", "Shift+Ctrl+W", Message::Quit),
            ],
            Menu::Edit => vec![
                ("Undo", "Ctrl+Z", Message::Undo(false)),
                ("Redo", "Ctrl+Y", Message::Undo(true)),
                ("", "", Message::CloseMenu),
                ("Cut", "Ctrl+X", Message::Copy(true)),
                ("Copy", "Ctrl+C", Message::Copy(false)),
                ("Paste", "Ctrl+V", Message::Paste),
                ("Delete", "Delete", Message::Delete),
                ("", "", Message::CloseMenu),
                ("Find…", "Ctrl+F", Message::Find(false)),
                ("Find next", "F3", Message::FindNext(false)),
                ("Find previous", "Shift+F3", Message::FindNext(true)),
                ("Replace…", "Ctrl+H", Message::Find(true)),
                ("Go to…", "Ctrl+G", Message::GoTo),
                ("", "", Message::CloseMenu),
                ("Select all", "Ctrl+A", Message::SelectAll),
                ("Time/Date", "F5", Message::Date),
            ],
            Menu::View => vec![
                ("Zoom", "", Message::Submenu(Some(Menu::Zoom))),
                ("", "", Message::CloseMenu),
                (
                    "Status bar",
                    "",
                    Message::Status(!self.config.window.status_bar),
                ),
                (
                    "Word wrap",
                    "",
                    Message::Wrap(!self.config.editor.word_wrap),
                ),
            ],
            Menu::Settings => vec![
                ("Settings…", "Ctrl+,", Message::Settings),
                ("", "", Message::CloseMenu),
                ("About RusTXT", "", Message::About),
            ],
            Menu::Zoom => vec![
                ("Zoom in", "Ctrl++", Message::ZoomBy(10)),
                ("Zoom out", "Ctrl+-", Message::ZoomBy(-10)),
                ("Restore default zoom", "Ctrl+0", Message::Zoom(100)),
            ],
            Menu::Recent => self
                .closed
                .iter()
                .map(|closed| {
                    (
                        closed.title.as_str(),
                        "",
                        Message::Reopen(closed.id.clone()),
                    )
                })
                .collect(),
        };
        let mut items = column![];
        for (name, shortcut, message) in entries {
            if name.is_empty() {
                items = items.push(
                    container(container(space::horizontal().height(1)).width(Fill).style(
                        move |theme| {
                            container::Style {
                                background: Some(
                                    crate::style::Colors::new(theme, palette)
                                        .foreground
                                        .scale_alpha(0.15)
                                        .into(),
                                ),
                                ..Default::default()
                            }
                        },
                    ))
                    .padding([6, 0]),
                );
                continue;
            }
            let enabled = match &message {
                Message::Undo(false) => doc.can_undo(),
                Message::Undo(true) => doc.can_redo(),
                Message::Copy(_) | Message::Delete => !doc.selected_range().is_empty(),
                Message::ReopenLast | Message::Submenu(Some(Menu::Recent)) => {
                    !self.closed.is_empty()
                }
                _ => true,
            };
            let mut contents = row![].spacing(8).align_y(iced::Center);
            if menu == Menu::View {
                let checked = match message {
                    Message::Status(_) => self.config.window.status_bar,
                    Message::Wrap(_) => self.config.editor.word_wrap,
                    _ => false,
                };
                let check: Element<'_, Message> = if checked {
                    icon(Icon::Check).into()
                } else {
                    space::horizontal().width(16).into()
                };
                contents = contents.push(check);
            }
            contents = contents
                .push(text(name).size(UI).line_height(iced::Pixels(18.0)))
                .push(space::horizontal());
            if matches!(message, Message::Submenu(_)) {
                contents = contents.push(icon(Icon::Right));
            } else if !shortcut.is_empty() {
                contents = contents.push(
                    text(if cfg!(target_os = "macos") {
                        shortcut.replace("Ctrl", "Cmd")
                    } else {
                        shortcut.into()
                    })
                    .size(UI)
                    .line_height(iced::Pixels(18.0))
                    .style(move |theme| text::Style {
                        color: Some(
                            crate::style::Colors::new(theme, palette)
                                .foreground
                                .scale_alpha(0.55),
                        ),
                    }),
                );
            }
            let submenu = if let Message::Submenu(value) = message {
                value
            } else {
                None
            };
            let item = button(contents)
                .width(Fill)
                .height(32)
                .padding([7, 12])
                .on_press_maybe(enabled.then_some(message))
                .style(move |theme, status| {
                    let mut style = crate::style::Colors::new(theme, palette).flat(status, false);
                    if status == button::Status::Disabled {
                        style.text_color = crate::style::Colors::new(theme, palette)
                            .foreground
                            .scale_alpha(0.5);
                    }
                    style.border.radius = 4.0.into();
                    style
                });
            let item: Element<'_, Message> = if matches!(menu, Menu::Zoom | Menu::Recent) {
                item.into()
            } else {
                widget::mouse_area(item)
                    .on_enter(Message::Submenu(if enabled { submenu } else { None }))
                    .into()
            };
            items = items.push(item);
        }
        if menu == Menu::Recent && self.closed.is_empty() {
            items = items.push(text("No recently closed tabs").size(UI));
        }
        container(items)
            .padding(6)
            .width(match menu {
                Menu::File => 264,
                Menu::Edit => 204,
                Menu::View => 136,
                Menu::Settings => 212,
                _ => 300,
            })
            .style(move |theme| {
                let colors = crate::style::Colors::new(theme, palette);
                container::Style {
                    border: iced::Border {
                        color: colors.foreground.scale_alpha(0.2),
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..colors.panel()
                }
            })
            .into()
    }
}

fn hint<'a>(
    control: impl Into<Element<'a, Message>>,
    description: &'a str,
    palette: Option<&'a config::Palette>,
) -> Element<'a, Message> {
    widget::tooltip(
        control,
        text(description).size(UI),
        widget::tooltip::Position::Bottom,
    )
    .delay(Duration::from_millis(450))
    .gap(6)
    .padding(8)
    .style(move |theme| crate::style::Colors::new(theme, palette).tooltip())
    .into()
}

fn setting<'a>(
    palette: Option<&'a config::Palette>,
    label: &'static str,
    subtitle: String,
    control: Element<'a, Message>,
) -> widget::Container<'a, Message> {
    let mut description = column![text(label).size(UI).line_height(iced::Pixels(18.0))].spacing(0);
    if !subtitle.is_empty() {
        description = description.push(
            text(subtitle)
                .size(CAPTION)
                .line_height(iced::Pixels(15.0))
                .style(move |theme| text::Style {
                    color: Some(
                        crate::style::Colors::new(theme, palette)
                            .foreground
                            .scale_alpha(0.55),
                    ),
                }),
        );
    }
    container(
        row![description.width(Fill), control]
            .spacing(16)
            .align_y(iced::Center),
    )
    .padding([10, 14])
}
fn settings_group<'a>(
    palette: Option<&'a config::Palette>,
    contents: Element<'a, Message>,
) -> widget::Container<'a, Message> {
    container(contents).width(Fill).style(move |theme| {
        let colors = crate::style::Colors::new(theme, palette);
        container::Style {
            shadow: iced::Shadow::default(),
            ..colors.panel()
        }
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Choice {
    value: String,
    label: String,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn system_ui_font() -> Option<String> {
    #[cfg(target_os = "linux")]
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "font-name"])
        .output()
    {
        if output.status.success() {
            let description = String::from_utf8_lossy(&output.stdout)
                .trim()
                .trim_matches('\'')
                .to_owned();
            if let Some((family, _)) = description.rsplit_once(' ') {
                return Some(family.to_owned());
            }
        }
    }
    system_font("sans-serif")
}
fn bold_font() -> Font {
    static FAMILY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let family = FAMILY.get_or_init(|| system_ui_font().unwrap_or_default());
    Font {
        weight: iced::font::Weight::Bold,
        ..if family.is_empty() {
            Font::DEFAULT
        } else {
            Font::with_name(family)
        }
    }
}

fn modal<'a>(
    base: Element<'a, Message>,
    palette: Option<&'a config::Palette>,
    contents: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    widget::stack![
        base,
        widget::opaque(
            container(
                container(contents)
                    .padding(24)
                    .width(520)
                    .style(move |theme| crate::style::Colors::new(theme, palette).panel())
            )
            .center(Fill)
            .style(|_| container::Style {
                background: Some(iced::Color::BLACK.scale_alpha(0.4).into()),
                ..Default::default()
            })
        )
    ]
    .into()
}

fn shortcut(key: Key<&str>, modifiers: keyboard::Modifiers) -> Option<Message> {
    if modifiers.command() {
        if let Key::Character(character) = key {
            return match character.to_lowercase().as_str() {
                "n" => Some(Message::New),
                "t" => Some(if modifiers.shift() {
                    Message::ReopenLast
                } else {
                    Message::New
                }),
                "o" => Some(Message::Open),
                "s" => Some(if modifiers.alt() {
                    Message::SaveAll
                } else {
                    Message::Save(modifiers.shift())
                }),
                "w" => Some(if modifiers.shift() {
                    Message::Quit
                } else {
                    Message::CloseActive
                }),
                "q" => Some(Message::Quit),
                "f" => Some(Message::Find(false)),
                "h" => Some(Message::Find(true)),
                "g" => Some(Message::GoTo),
                "z" => Some(Message::Undo(modifiers.shift())),
                "y" => Some(Message::Undo(true)),
                "p" => Some(Message::Print),
                "," => Some(Message::Settings),
                "+" | "=" => Some(Message::ZoomBy(10)),
                "-" => Some(Message::ZoomBy(-10)),
                "0" => Some(Message::Zoom(100)),
                _ => None,
            };
        }
    }
    match key {
        Key::Named(Named::F3) => Some(Message::FindNext(modifiers.shift())),
        Key::Named(Named::F5) => Some(Message::Date),
        Key::Named(Named::F10) => Some(Message::Menu(Menu::File)),
        Key::Named(Named::Tab) if modifiers.control() => Some(Message::NextTab(modifiers.shift())),
        Key::Character("f") if modifiers.alt() => Some(Message::Menu(Menu::File)),
        Key::Character("e") if modifiers.alt() => Some(Message::Menu(Menu::Edit)),
        Key::Character("v") if modifiers.alt() => Some(Message::Menu(Menu::View)),
        Key::Character("s") if modifiers.alt() => Some(Message::Menu(Menu::Settings)),
        _ => None,
    }
}

/// Match GTK/fontconfig aliases on Linux; retain Iced's portable fallback elsewhere.
fn system_font(alias: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("fc-match")
            .args(["-f", "%{family}", alias])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let name = String::from_utf8(output.stdout)
            .ok()?
            .split(',')
            .next()?
            .trim()
            .to_owned();
        (!name.is_empty()).then_some(name)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = alias;
        None
    }
}
