use crate::{
    document::{self, Document},
    instance::Instance,
    search::Search,
};
use iced::{
    keyboard::{self, key::Named, Key},
    widget::{
        self, button, checkbox, column, container, operation, pick_list, row, scrollable, slider,
        space, text, text_editor, text_input,
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

const EDITOR: &str = "editor";
const FIND: &str = "find";
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run() -> iced::Result {
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
        default_text_size: iced::Pixels(14.0),
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
    settings_open: bool,
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
    theme: Option<Theme>,
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
            settings_open: false,
            about_open: false,
            go_open: false,
            go_line: String::new(),
            discard_id: None,
            closed: Vec::new(),
            instance,
            reload,
            _watcher: watcher,
            editor_font: Font::MONOSPACE,
            editor_size: 16.0,
            font_names: Vec::new(),
            theme: None,
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
            .unwrap_or((description, 16.0));
        self.editor_size = size.clamp(6.0, 96.0);
        let family = family.split(',').next().unwrap_or("").trim();
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
                self.doc_mut().action(action);
                self.refresh_search();
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
            Message::CloseMenu => {
                self.menu = None;
                return operation::focus(EDITOR);
            }
            Message::Menu(menu) => {
                self.menu = if self.menu == Some(menu) {
                    None
                } else {
                    Some(menu)
                };
            }
            Message::Dismiss => {
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
                self.find_open = true;
                self.replace_open = replace;
                self.refresh_search();
                return operation::focus(FIND);
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
                self.config.editor.word_wrap = value;
                return self.save_settings();
            }
            Message::Status(value) => {
                self.config.window.status_bar = value;
                return self.save_settings();
            }
            Message::Theme(value) => {
                self.config.appearance.theme = value;
                return self.save_settings();
            }
            Message::Font(value) => self.font_draft = value,
            Message::ApplyFont => {
                self.config.editor.font = self.font_draft.clone();
                return self.save_settings();
            }
            Message::Zoom(value) => {
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
        let menus = row![
            button("File")
                .on_press(Message::Menu(Menu::File))
                .style(button::text),
            button("Edit")
                .on_press(Message::Menu(Menu::Edit))
                .style(button::text),
            button("View")
                .on_press(Message::Menu(Menu::View))
                .style(button::text),
            button("Settings")
                .on_press(Message::Menu(Menu::Settings))
                .style(button::text),
            space::horizontal(),
            button("−")
                .on_press(Message::ZoomBy(-10))
                .style(button::text),
            text(format!("{}%", self.config.appearance.zoom)),
            button("+")
                .on_press(Message::ZoomBy(10))
                .style(button::text),
        ]
        .align_y(iced::Center)
        .spacing(4)
        .padding([4, 8]);
        let tabs = self
            .docs
            .iter()
            .enumerate()
            .fold(row![].spacing(3), |tabs, (index, doc)| {
                let title = format!(
                    "{}{}",
                    if doc.state.dirty { "• " } else { "" },
                    doc.state.title
                );
                tabs.push(
                    row![
                        button(text(title)).on_press(Message::Select(index)).style(
                            if index == self.active {
                                button::primary
                            } else {
                                button::secondary
                            }
                        ),
                        button("×")
                            .on_press(Message::Close(index))
                            .style(button::text)
                    ]
                    .align_y(iced::Center),
                )
            })
            .push(button("+").on_press(Message::New).style(button::text));
        let tabs = scrollable(tabs)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new(),
            ))
            .height(42);
        let doc = self.doc();
        let editor = text_editor(&doc.content)
            .id(EDITOR)
            .height(Fill)
            .padding(12)
            .font(self.editor_font)
            .size(self.editor_size * self.config.appearance.zoom as f32 / 100.0)
            .wrapping(if self.config.editor.word_wrap {
                text::Wrapping::WordOrGlyph
            } else {
                text::Wrapping::None
            })
            .on_action(Message::Action)
            .style(|theme, status| {
                let mut style = text_editor::default(theme, status);
                style.border = iced::Border::default();
                style.background = theme.palette().background.into();
                style
            })
            .key_binding(|press| {
                shortcut(press.key.as_ref(), press.modifiers)
                    .map(text_editor::Binding::Custom)
                    .or_else(|| text_editor::Binding::from_key_press(press))
            });
        let mut body = column![menus, container(tabs).padding([0, 8])];
        if self.find_open {
            let count = if self.search.error.is_some() {
                "Invalid expression".to_owned()
            } else {
                format!("{} matches", self.search.matches.len())
            };
            let mut find = column![
                row![
                    text_input("Find", &self.search.query)
                        .id(FIND)
                        .on_input(Message::Query)
                        .on_submit(Message::FindNext(false)),
                    button("Previous")
                        .on_press(Message::FindNext(true))
                        .style(button::secondary),
                    button("Next")
                        .on_press(Message::FindNext(false))
                        .style(button::secondary),
                    button("×").on_press(Message::Dismiss).style(button::text)
                ]
                .spacing(6)
                .align_y(iced::Center),
                row![
                    checkbox(self.search.case_sensitive)
                        .label("Match case")
                        .on_toggle(Message::MatchCase),
                    checkbox(self.search.whole_word)
                        .label("Whole word")
                        .on_toggle(Message::WholeWord),
                    checkbox(self.search.regex)
                        .label("Regex")
                        .on_toggle(Message::Regex),
                    text(count)
                ]
                .spacing(16)
            ]
            .spacing(8);
            if self.replace_open {
                find = find.push(
                    row![
                        text_input("Replace with", &self.search.replacement)
                            .on_input(Message::Replacement)
                            .on_submit(Message::Replace),
                        button("Replace").on_press(Message::Replace),
                        button("Replace all").on_press(Message::ReplaceAll)
                    ]
                    .spacing(6),
                );
            }
            body = body.push(container(find).padding([8, 12]));
        }
        body = body.push(editor);
        if self.config.window.status_bar {
            let cursor = doc.content.cursor().position;
            let column = doc
                .content
                .line(cursor.line)
                .map(|l| l.text[..cursor.column.min(l.text.len())].chars().count())
                .unwrap_or(0);
            body = body.push(
                container(
                    row![
                        text(format!("Ln {}, Col {}", cursor.line + 1, column + 1)),
                        space::horizontal(),
                        text(format!("{} characters", doc.state.content.chars().count())),
                        text(doc.state.line_ending.as_str()),
                        text("UTF-8")
                    ]
                    .spacing(20),
                )
                .padding([6, 12]),
            );
        }
        let base: Element<'_, Message> = container(body).height(Fill).width(Fill).into();
        if let Some(error) = &self.error {
            return modal(
                base,
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
            return modal(
                base,
                column![
                    row![
                        text("Settings").size(22),
                        space::horizontal(),
                        button("×").on_press(Message::Dismiss).style(button::text)
                    ]
                    .align_y(iced::Center),
                    row![
                        text("Theme").width(120),
                        pick_list(
                            themes,
                            Some(self.config.appearance.theme.clone()),
                            Message::Theme
                        )
                    ]
                    .align_y(iced::Center),
                    row![
                        text("Title bar").width(120),
                        pick_list(["auto", "show", "hide"], Some(titlebar), |value| {
                            Message::Titlebar(value.into())
                        })
                    ]
                    .align_y(iced::Center),
                    text("Font family and optional point size"),
                    row![
                        text_input("System monospace", &self.font_draft)
                            .on_input(Message::Font)
                            .on_submit(Message::ApplyFont),
                        button("Apply").on_press(Message::ApplyFont)
                    ]
                    .spacing(8),
                    row![
                        text(format!("Zoom: {}%", self.config.appearance.zoom)).width(120),
                        slider(10..=500, self.config.appearance.zoom, Message::Zoom).step(10u32)
                    ]
                    .align_y(iced::Center),
                    checkbox(self.config.editor.word_wrap)
                        .label("Word wrap")
                        .on_toggle(Message::Wrap),
                    checkbox(self.config.window.status_bar)
                        .label("Status bar")
                        .on_toggle(Message::Status),
                    button("Done").on_press(Message::Dismiss)
                ]
                .spacing(18),
            );
        }
        if self.about_open {
            return modal(
                base,
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
            let entries: Vec<(&str, Message)> = match menu {
                Menu::File => vec![
                    ("New tab                 Ctrl+N", Message::New),
                    ("Open…                    Ctrl+O", Message::Open),
                    ("Save                       Ctrl+S", Message::Save(false)),
                    ("Save as…       Ctrl+Shift+S", Message::Save(true)),
                    ("Save all          Ctrl+Alt+S", Message::SaveAll),
                    ("Print…                     Ctrl+P", Message::Print),
                    (
                        "Close tab                Ctrl+W",
                        Message::Close(self.active),
                    ),
                    ("Reopen closed  Ctrl+Shift+T", Message::ReopenLast),
                    ("Discard changes and close…", Message::Discard),
                    ("Exit", Message::Quit),
                ],
                Menu::Edit => vec![
                    ("Undo                       Ctrl+Z", Message::Undo(false)),
                    ("Redo               Ctrl+Shift+Z", Message::Undo(true)),
                    ("Cut                          Ctrl+X", Message::Copy(true)),
                    ("Copy                       Ctrl+C", Message::Copy(false)),
                    ("Paste                      Ctrl+V", Message::Paste),
                    ("Delete", Message::Delete),
                    ("Select all                Ctrl+A", Message::SelectAll),
                    ("Find…                     Ctrl+F", Message::Find(false)),
                    ("Replace…               Ctrl+H", Message::Find(true)),
                    ("Go to line…             Ctrl+G", Message::GoTo),
                    ("Time and date                  F5", Message::Date),
                ],
                Menu::View => vec![
                    ("Zoom in", Message::ZoomBy(10)),
                    ("Zoom out", Message::ZoomBy(-10)),
                    ("Reset zoom", Message::Zoom(100)),
                    (
                        "Toggle word wrap",
                        Message::Wrap(!self.config.editor.word_wrap),
                    ),
                    (
                        "Toggle status bar",
                        Message::Status(!self.config.window.status_bar),
                    ),
                ],
                Menu::Settings => vec![
                    ("Preferences…          Ctrl+,", Message::Settings),
                    ("About RusTXT", Message::About),
                ],
            };
            let mut items = column![].spacing(2);
            for (label, message) in entries {
                let enabled = match &message {
                    Message::Undo(false) => doc.can_undo(),
                    Message::Undo(true) => doc.can_redo(),
                    _ => true,
                };
                items = items.push(
                    button(text(if cfg!(target_os = "macos") {
                        label.replace("Ctrl", "Cmd")
                    } else {
                        label.to_owned()
                    }))
                    .on_press_maybe(enabled.then_some(message))
                    .width(Fill)
                    .style(button::text),
                );
            }
            if menu == Menu::File && !self.closed.is_empty() {
                items = items.push(text("Recently closed").size(12));
                for closed in &self.closed {
                    items = items.push(
                        button(text(&closed.title))
                            .on_press(Message::Reopen(closed.id.clone()))
                            .width(Fill)
                            .style(button::text),
                    );
                }
            }
            let panel = container(scrollable(items))
                .padding(8)
                .width(300)
                .max_height(620)
                .style(container::rounded_box);
            return widget::stack![
                base,
                widget::opaque(
                    widget::mouse_area(container(space::vertical()).width(Fill).height(Fill))
                        .on_press(Message::CloseMenu)
                ),
                container(widget::opaque(panel)).padding(iced::Padding {
                    top: 38.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: match menu {
                        Menu::File => 8.0,
                        Menu::Edit => 58.0,
                        Menu::View => 108.0,
                        Menu::Settings => 160.0,
                    }
                })
            ]
            .into();
        }
        base
    }
}

fn modal<'a>(
    base: Element<'a, Message>,
    contents: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    widget::stack![
        base,
        widget::opaque(
            container(
                container(contents)
                    .padding(24)
                    .width(520)
                    .style(container::rounded_box)
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
        Key::Named(Named::Escape) => Some(Message::Dismiss),
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
