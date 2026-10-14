use crate::{
    document::{self, Document},
    icons::{icon, Icon},
    instance::Instance,
    platform::{system_font, system_ui_font},
    search::Search,
};
use iced::{
    keyboard::{self, key::Named, Key},
    widget::{
        self, button, column, container, operation, row, scrollable, space, text, text_editor,
        text_input,
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
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

mod find;
mod menus;
mod message;
mod settings;

use menus::shortcut;
pub use message::{Menu, Message};
use settings::DEFAULT_FONT_SIZE;

const UI: f32 = 44.0 / 3.0;
const CAPTION: f32 = UI * 0.85;
/// Tick cadence while snapshots, view state or deferred writes are pending.
const FAST_TICK: Duration = Duration::from_millis(250);
/// Tick cadence when idle: only the single-instance inbox needs polling.
const SLOW_TICK: Duration = Duration::from_secs(1);
const INBOX_INTERVAL: Duration = Duration::from_secs(1);
/// Settle time before a burst of zoom notches or resize events hits disk.
const DEBOUNCE: Duration = Duration::from_millis(400);
const PRINT_MAX_AGE: Duration = Duration::from_secs(60 * 60);

const EDITOR: &str = "editor";
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
    /// Family names paired with their lowercase form for filtering.
    font_families: Vec<(String, String)>,
    custom_themes: Vec<String>,
    about_open: bool,
    go_open: bool,
    go_line: String,
    discard_id: Option<String>,
    error: Option<String>,
    /// The flush error currently on screen (or dismissed); identical repeats
    /// are suppressed until a flush succeeds again.
    last_error: Option<String>,
    /// Persisting failed on Quit; the next Quit exits regardless.
    quit_pending: bool,
    /// Matches are only rebuilt while the find bar is open; this marks them
    /// stale for the next Find Next while it is closed.
    search_stale: bool,
    /// Active document selection, refreshed once per update for the view.
    selection: Range<usize>,
    settings_dirty: Option<Instant>,
    layout_dirty: Option<Instant>,
    last_inbox_poll: Instant,
    color_cache: crate::style::ColorCache,
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
        let mut notices = files::refresh_from_disk(&mut session.documents);
        if let Some(moved) = storage.recovered_from() {
            notices.insert(
                0,
                format!(
                    "The recovery database could not be opened and was replaced. \
                     The old file was kept as {} in case anything can be salvaged from it.",
                    moved.display()
                ),
            );
        }
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
            error: loaded
                .error
                .into_iter()
                .chain(notices)
                .reduce(|a, b| format!("{a}\n\n{b}")),
            search: Search::default(),
            find_open: false,
            replace_open: false,
            menu: None,
            submenu: None,
            settings_open: false,
            font_picker: false,
            font_filter: String::new(),
            font_families: Vec::new(),
            custom_themes: Vec::new(),
            about_open: false,
            go_open: false,
            go_line: String::new(),
            discard_id: None,
            last_error: None,
            quit_pending: false,
            search_stale: true,
            selection: 0..0,
            settings_dirty: None,
            layout_dirty: None,
            last_inbox_poll: Instant::now(),
            color_cache: Default::default(),
            closed: Vec::new(),
            instance,
            reload,
            _watcher: watcher,
            system_monospace: system_font("monospace").unwrap_or_default(),
            editor_font: Font::MONOSPACE,
            editor_size: DEFAULT_FONT_SIZE as f32 * 4.0 / 3.0,
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
        app.selection = app.doc().selected_range();
        Ok(app)
    }
    fn colors(&self) -> crate::style::Resolver<'_> {
        crate::style::Resolver::new(&self.color_cache, self.ui_palette.as_ref())
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
    fn subscription(&self) -> Subscription<Message> {
        let period = tick_period(
            &self.docs,
            self.active,
            self.settings_dirty.is_some() || self.layout_dirty.is_some(),
            self.reload.load(Ordering::Relaxed),
        );
        Subscription::batch([
            iced::time::every(period).map(|_| Message::Tick),
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
    fn persist(&mut self) -> Result<(), String> {
        for doc in &mut self.docs {
            doc.persist(&self.storage)?;
        }
        let order: Vec<_> = self.docs.iter().map(|d| d.state.id.clone()).collect();
        let active = order.get(self.active).map(String::as_str).unwrap_or("");
        self.storage.update_layout(&order, active)?;
        self.closed = self.storage.closed_documents()?;
        Ok(())
    }
    /// Write what changed since the last tick: a full snapshot for edited
    /// text, or only cursor and scroll when just the view moved.
    fn flush(&mut self) -> Result<(), String> {
        for doc in &mut self.docs {
            if doc.content_dirty {
                doc.persist(&self.storage)?;
            } else if doc.view_dirty {
                doc.persist_view(&self.storage)?;
            }
        }
        Ok(())
    }
    /// Debounced config and window-size writes; `force` flushes them now.
    fn flush_deferred(&mut self, force: bool) -> Result<(), String> {
        let due = |since: Option<Instant>| since.is_some_and(|t| force || t.elapsed() >= DEBOUNCE);
        if due(self.settings_dirty) {
            self.settings_dirty = None;
            config::save(&self.paths, &self.config)?;
        }
        if due(self.layout_dirty) {
            self.layout_dirty = None;
            self.storage.set_state(
                "window",
                &format!(
                    "{} {} 0",
                    self.window_size.width as u32, self.window_size.height as u32
                ),
            )?;
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
    fn update(&mut self, message: Message) -> Task<Message> {
        if self.docs.is_empty() {
            // Only reachable between `iced::exit()` and shutdown.
            return Task::none();
        }
        let task = self.handle(message);
        if let Some(doc) = self.docs.get(self.active) {
            self.selection = doc.selected_range();
            if self.find_open {
                self.search.locate(&self.selection);
            }
        }
        task
    }
    fn handle(&mut self, message: Message) -> Task<Message> {
        let modal_open =
            self.settings_open || self.about_open || self.go_open || self.discard_id.is_some();
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
                let count = self.docs.len();
                if count == 0 {
                    return Task::none();
                }
                return self.update(Message::Select(if backwards {
                    (self.active + count - 1) % count
                } else {
                    (self.active + 1) % count
                }));
            }
            Message::ZoomBy(_)
            | Message::Settings
            | Message::Wrap(_)
            | Message::Status(_)
            | Message::Theme(_)
            | Message::Font(_)
            | Message::ChooseFont
            | Message::FontFilter(_)
            | Message::FontChosen(_)
            | Message::ResetFont
            | Message::ApplyFont
            | Message::Zoom(_)
            | Message::Titlebar(_) => return self.update_settings(message),
            Message::Tick => {
                if let Some(doc) = self.docs.get_mut(self.active) {
                    doc.restore_viewport();
                }
                match self.flush() {
                    Ok(()) => self.last_error = None,
                    Err(error) => {
                        if self.last_error.as_ref() != Some(&error) {
                            self.error = Some(error.clone());
                            self.last_error = Some(error);
                        }
                    }
                }
                let result = self.flush_deferred(false);
                self.report(result);
                if self.reload.swap(false, Ordering::Relaxed) {
                    let loaded = config::load(&self.paths);
                    if loaded.error.is_some() {
                        self.error = loaded.error;
                    } else {
                        self.config = loaded.config;
                        self.apply_appearance();
                    }
                    if self.settings_open {
                        self.custom_themes = config::list_custom_themes(&self.paths);
                    }
                }
                if self.last_inbox_poll.elapsed() < INBOX_INTERVAL {
                    return Task::none();
                }
                self.last_inbox_poll = Instant::now();
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
                    let _ = self.flush_deferred(true);
                    return iced::exit();
                }
                self.refresh_search();
                return operation::focus(EDITOR);
            }
            Message::Quit => {
                if self.quit_pending {
                    return iced::exit();
                }
                match self.persist().and_then(|_| self.flush_deferred(true)) {
                    Ok(()) => return iced::exit(),
                    Err(error) => {
                        self.quit_pending = true;
                        self.error = Some(format!(
                            "{error}\n\nYour unsaved drafts could not be stored, so they may \
                             not be recovered next time. Save them to files first, or choose \
                             Exit again to quit anyway."
                        ));
                    }
                }
            }
            Message::ToggleReplace
            | Message::Find(_)
            | Message::Query(_)
            | Message::Replacement(_)
            | Message::MatchCase(_)
            | Message::WholeWord(_)
            | Message::Regex(_)
            | Message::FindNext(_)
            | Message::Replace
            | Message::ReplaceAll => return self.update_find(message),
            Message::CloseMenu | Message::Submenu(_) | Message::Menu(_) => {
                return self.update_menus(message)
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
                self.quit_pending = false;
                self.find_open = false;
                return operation::focus(EDITOR);
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
                self.doc_mut().action(text_editor::Action::SelectAll);
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
                        for problem in files::refresh_from_disk(&mut states) {
                            self.report(Err(problem));
                        }
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
                        let _ = self.flush_deferred(true);
                        return iced::exit();
                    }
                }
            }
            Message::About => {
                self.menu = None;
                self.about_open = true;
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
                let html = print_html(&self.doc().state.content);
                let result = write_print_file(&self.paths.cache_dir, &html).and_then(|path| {
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
                self.layout_dirty = Some(Instant::now());
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
        let colors = self.colors();
        let flat = move |theme: &Theme, status| colors.get(theme).flat(status, false);
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
                    .style(move |theme, status| colors.get(theme).flat(status, selected)),
            )
        });
        let menus = container(menus)
            .padding([0, 4])
            .width(Fill)
            .height(31)
            .style(move |theme| colors.get(theme).bar());
        let tabs = self
            .docs
            .iter()
            .enumerate()
            .fold(row![].spacing(4), |tabs, (index, doc)| {
                let selected = index == self.active;
                let dot = container(space::horizontal().width(14).height(14)).style(move |theme| {
                    container::Style {
                        background: doc.state.dirty.then(|| colors.get(theme).foreground.into()),
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
                .style(move |theme, status| colors.get(theme).flat(status, selected));
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
                    colors,
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
        .style(move |theme| colors.get(theme).bar());
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
                let colors = colors.get(theme);
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
            widget::stack![
                editor,
                container(widget::opaque(self.find_bar()))
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
                    background: Some(colors.get(theme).border.scale_alpha(0.5).into()),
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
                        background: Some(colors.get(theme).border.into()),
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
                        text(format!("{} characters", doc.char_count())).size(CAPTION),
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
                    let colors = colors.get(theme);
                    container::Style {
                        text_color: Some(colors.foreground.scale_alpha(0.75)),
                        ..colors.bar()
                    }
                }),
            );
        }
        let base: Element<'_, Message> = container(body).height(Fill).width(Fill).into();
        if let Some(error) = &self.error {
            let mut actions = row![button("Close").on_press(Message::Dismiss)].spacing(12);
            if self.quit_pending {
                actions = actions.push(
                    button("Exit anyway")
                        .on_press(Message::Quit)
                        .style(button::danger),
                );
            }
            return modal(
                base,
                colors,
                column![
                    text("Could not complete the action").size(20),
                    text(error),
                    actions
                ]
                .spacing(16),
            );
        }
        if let Some(id) = &self.discard_id {
            return modal(
                base,
                colors,
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
                colors,
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
            return widget::stack![
                base,
                widget::opaque(
                    container(self.settings_panel())
                        .padding(24)
                        .center(Fill)
                        .style(|_| {
                            container::Style {
                                background: Some(iced::Color::BLACK.scale_alpha(0.4).into()),
                                ..Default::default()
                            }
                        })
                )
            ]
            .into();
        }

        if self.about_open {
            return modal(
                base,
                colors,
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
}

fn hint<'a>(
    control: impl Into<Element<'a, Message>>,
    description: &'a str,
    colors: crate::style::Resolver<'a>,
) -> Element<'a, Message> {
    widget::tooltip(
        control,
        text(description).size(UI),
        widget::tooltip::Position::Bottom,
    )
    .delay(Duration::from_millis(450))
    .gap(6)
    .padding(8)
    .style(move |theme| colors.get(theme).tooltip())
    .into()
}

/// Tick quickly while anything is waiting to be written or restored;
/// otherwise only often enough to notice a second launch's inbox message.
fn tick_period(docs: &[Document], active: usize, deferred: bool, reload: bool) -> Duration {
    let busy = deferred
        || reload
        || docs.iter().any(|d| d.content_dirty || d.view_dirty)
        || docs.get(active).is_some_and(Document::viewport_pending);
    if busy {
        FAST_TICK
    } else {
        SLOW_TICK
    }
}

fn print_html(content: &str) -> String {
    let escaped = content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<!doctype html><meta charset=utf-8><title>RusTXT print</title><style>body{{font:12pt monospace}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;unicode-bidi:plaintext}}@page{{margin:20mm}}</style><pre>{escaped}</pre><script>onload=()=>print()</script>")
}

/// Write the print page under a fresh random name, private to the user (the
/// atomic save goes through a 0600 temporary file and the target is new, so
/// that mode is kept), after clearing out pages older than `PRINT_MAX_AGE`.
fn write_print_file(cache_dir: &Path, html: &str) -> Result<PathBuf, String> {
    use std::hash::{BuildHasher, Hasher};
    std::fs::create_dir_all(cache_dir).map_err(|e| e.to_string())?;
    prune_print_files(cache_dir, PRINT_MAX_AGE);
    let nonce = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    let path = cache_dir.join(format!("print-{}-{nonce:016x}.html", std::process::id()));
    files::atomic_save(&path, html.as_bytes())?;
    // The page is a copy of the user's text; keep it owner-only regardless of umask.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn prune_print_files(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("print-") && name.ends_with(".html")) {
            continue;
        }
        let expired = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > max_age);
        if expired {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn modal<'a>(
    base: Element<'a, Message>,
    colors: crate::style::Resolver<'a>,
    contents: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    widget::stack![
        base,
        widget::opaque(
            container(
                container(contents)
                    .padding(24)
                    .width(520)
                    .style(move |theme| colors.get(theme).panel())
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

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::text_editor::{Action, Edit, Motion};
    use rustxt_core::storage::DocumentState;

    fn doc(text: &str) -> Document {
        let mut state = DocumentState::untitled(std::iter::empty(), LineEnding::Lf);
        state.content = text.into();
        Document::new(state)
    }

    #[test]
    fn timer_only_runs_fast_while_work_is_pending() {
        let mut docs = vec![doc("a"), doc("b")];
        // A fresh document still has its viewport to restore on the first tick.
        assert_eq!(tick_period(&docs, 0, false, false), FAST_TICK);
        for d in &mut docs {
            d.restore_viewport();
        }
        assert_eq!(tick_period(&docs, 0, false, false), SLOW_TICK);
        docs[1].action(Action::Move(Motion::Right));
        assert_eq!(tick_period(&docs, 0, false, false), FAST_TICK);
        docs[1].view_dirty = false;
        assert_eq!(tick_period(&docs, 0, false, false), SLOW_TICK);
        docs[0].action(Action::Edit(Edit::Insert('x')));
        assert_eq!(tick_period(&docs, 0, false, false), FAST_TICK);
        docs[0].content_dirty = false;
        docs[0].view_dirty = false;
        assert_eq!(tick_period(&docs, 0, true, false), FAST_TICK);
        assert_eq!(tick_period(&docs, 0, false, true), FAST_TICK);
        assert_eq!(tick_period(&docs, 0, false, false), SLOW_TICK);
        assert_eq!(tick_period(&[], 0, false, false), SLOW_TICK);
    }

    #[test]
    fn print_files_are_private_random_and_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        let first = write_print_file(&cache, &print_html("<a&b>")).unwrap();
        let second = write_print_file(&cache, "x").unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with(&cache));
        assert!(std::fs::read_to_string(&first)
            .unwrap()
            .contains("&lt;a&amp;b&gt;"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&first).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
        }
        let stale = std::time::SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&first)
            .unwrap()
            .set_modified(stale)
            .unwrap();
        std::fs::write(cache.join("other.html"), "keep").unwrap();
        let third = write_print_file(&cache, "y").unwrap();
        assert!(!first.exists(), "hour-old print page should be pruned");
        assert!(second.exists() && third.exists());
        assert!(cache.join("other.html").exists());
    }
}
