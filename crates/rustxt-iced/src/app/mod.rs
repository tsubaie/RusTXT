use crate::{
    document::Document,
    instance::Instance,
    platform::{system_font, system_ui_font},
    search::Search,
};
use iced::{
    keyboard::{self, key::Named, Key},
    widget::operation,
    window, Event, Font, Subscription, Task, Theme,
};
use rustxt_core::{
    config::{self, Config, Paths},
    desktop, files,
    storage::{ClosedDocument, Storage},
    watch,
};
use std::{
    cell::RefCell,
    ops::Range,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

mod documents;
mod find;
mod menus;
mod message;
mod settings;
mod view;

use menus::shortcut;
pub use message::{Menu, Message};
use settings::DEFAULT_FONT_SIZE;

const UI: f32 = 44.0 / 3.0;
const CAPTION: f32 = UI * 0.85;
/// Tick cadence while snapshots, view state or deferred writes are pending.
const FAST_TICK: Duration = Duration::from_millis(250);
/// Tick cadence when idle: only the single-instance inbox needs polling.
const SLOW_TICK: Duration = Duration::from_secs(1);

const EDITOR: &str = "editor";

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
        // Wayland app_id / X11 WM_CLASS; must match the .desktop file name.
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: "com.tsubaie.rustxt".to_owned(),
            ..Default::default()
        },
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
    /// The configuration the app last wrote, so the watcher's echo of our own
    /// save is not mistaken for a user edit and does not revert pending changes.
    written_config: Option<config::Config>,
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
            written_config: None,
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
    /// Show an error. One already on screen is kept and the new one appended,
    /// so several failing files at startup do not hide each other.
    fn report(&mut self, result: Result<(), String>) {
        if let Err(error) = result {
            self.error = Some(match self.error.take() {
                Some(shown) if shown != error && shown.len() < 2000 => {
                    format!("{shown}\n\n{error}")
                }
                Some(shown) => shown,
                None => error,
            });
        }
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
            Message::CloseActive
            | Message::NextTab(_)
            | Message::Tick
            | Message::New
            | Message::Open
            | Message::Opened(_)
            | Message::Save(_)
            | Message::SaveAll
            | Message::SaveNext
            | Message::SavePicked(..)
            | Message::Select(_)
            | Message::Close(_)
            | Message::Quit
            | Message::ReopenLast
            | Message::Reopen(_)
            | Message::Discard
            | Message::ConfirmDiscard(_)
            | Message::Print
            | Message::Window(_) => self.update_documents(message),
            Message::Modifiers(_)
            | Message::Action(_)
            | Message::Undo(_)
            | Message::Copy(_)
            | Message::Paste
            | Message::Pasted(_)
            | Message::SelectAll
            | Message::Delete
            | Message::Date => self.update_editing(message),
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
            | Message::Titlebar(_) => self.update_settings(message),
            Message::ToggleReplace
            | Message::Find(_)
            | Message::Query(_)
            | Message::Replacement(_)
            | Message::MatchCase(_)
            | Message::WholeWord(_)
            | Message::Regex(_)
            | Message::FindNext(_)
            | Message::Replace
            | Message::ReplaceAll => self.update_find(message),
            Message::CloseMenu | Message::Submenu(_) | Message::Menu(_) => {
                self.update_menus(message)
            }
            Message::Dismiss
            | Message::About
            | Message::GoTo
            | Message::GoLine(_)
            | Message::Go
            | Message::OpenLink(_) => self.update_dialogs(message),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::text_editor::{Action, Edit, Motion};
    use rustxt_core::storage::{DocumentState, LineEnding};

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
}
