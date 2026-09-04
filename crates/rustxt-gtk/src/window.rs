//! The main window: tab strip, classic menu bar, editor area with the
//! floating find bar, status bar, and all the actions behind the menus.
//! Persistence, configuration and theming come from `rustxt-core`.

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use rustxt_core::{
    config::{self, Config, Paths, Settings},
    files,
    storage::{ClosedDocument, DocumentState, LineEnding, Storage},
    watch::{self, ConfigWatcher},
};
use sourceview5::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::Path,
    rc::Rc,
    time::Duration,
};

use crate::{
    about_dialog,
    document::Document,
    find_bar::FindBar,
    menus::{self, Menus},
    printing, settings_dialog, theme,
};

const ZOOM_STEP: u32 = 10;
const SNAPSHOT_DELAY: Duration = Duration::from_millis(400);

thread_local! {
    static INSTANCE: RefCell<Option<Rc<RustxtWindow>>> = const { RefCell::new(None) };
}

struct StatusBar {
    root: gtk::Box,
    position: gtk::Label,
    count: gtk::Label,
    zoom: gtk::Label,
    eol: gtk::Label,
}

impl StatusBar {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("rustxt-status");
        let label = |text: &str| {
            let label = gtk::Label::new(Some(text));
            label.set_xalign(0.0);
            label
        };
        let position = label("Ln 1, Col 1");
        position.set_hexpand(true);
        let count = label("0 characters");
        let zoom = label("100%");
        let eol = label("Unix (LF)");
        let encoding = label("UTF-8");
        root.append(&position);
        for widget in [&count, &zoom, &eol, &encoding] {
            root.append(&gtk::Separator::new(gtk::Orientation::Vertical));
            root.append(widget);
        }
        Self {
            root,
            position,
            count,
            zoom,
            eol,
        }
    }
}

pub struct RustxtWindow {
    pub window: adw::ApplicationWindow,
    header: adw::HeaderBar,
    title_widget: adw::WindowTitle,
    tab_view: adw::TabView,
    overlay: gtk::Overlay,
    find_bar: Rc<FindBar>,
    status: StatusBar,
    theme_css: gtk::CssProvider,
    zoom_css: gtk::CssProvider,
    menus: Menus,
    docs: RefCell<Vec<Rc<Document>>>,
    storage: RefCell<Storage>,
    paths: Paths,
    settings: RefCell<Settings>,
    scheme: RefCell<Option<sourceview5::StyleScheme>>,
    closed: RefCell<Vec<ClosedDocument>>,
    pending: RefCell<HashSet<String>>,
    flush_source: RefCell<Option<glib::SourceId>>,
    menu_page: RefCell<Option<adw::TabPage>>,
    discarding: RefCell<Option<adw::TabPage>>,
    restoring: Cell<bool>,
    wheel_accumulator: Cell<f64>,
    _watcher: RefCell<Option<ConfigWatcher>>,
    _emergency_storage: Option<tempfile::TempDir>,
}

impl RustxtWindow {
    /// The single main window, created on first use.
    pub fn obtain(app: &adw::Application) -> Rc<Self> {
        INSTANCE.with(|instance| {
            instance
                .borrow_mut()
                .get_or_insert_with(|| Self::new(app))
                .clone()
        })
    }

    pub fn present(&self) {
        self.window.present();
    }

    fn new(app: &adw::Application) -> Rc<Self> {
        let paths = Paths::discover();
        if let Err(error) = config::ensure_config_file(&paths) {
            eprintln!("RusTXT: could not write default config: {error}");
        }
        let (storage, emergency_storage) = match Storage::open(&paths.session_db()) {
            Ok(storage) => (storage, None),
            Err(error) => {
                eprintln!("RusTXT: recovery database unavailable ({error}); using a private temporary one");
                let directory = tempfile::Builder::new()
                    .prefix("rustxt-")
                    .tempdir()
                    .expect("cannot create private temporary recovery directory");
                let storage = Storage::open(&directory.path().join("session.db"))
                    .expect("cannot open a recovery database anywhere");
                (storage, Some(directory))
            }
        };
        let settings = config::settings(&paths);

        // --- widgets -----------------------------------------------------------
        let window = adw::ApplicationWindow::new(app);
        window.set_title(Some("RusTXT"));
        window.set_size_request(560, 360);
        let mut maximized = false;
        let mut size = (1000, 700);
        if let Ok(Some(saved)) = storage.get_state("window") {
            let parts: Vec<i32> = saved.split(' ').filter_map(|p| p.parse().ok()).collect();
            if parts.len() == 3 {
                size = (parts[0].max(560), parts[1].max(360));
                maximized = parts[2] == 1;
            }
        }
        window.set_default_size(size.0, size.1);
        if maximized {
            window.maximize();
        }

        let title_widget = adw::WindowTitle::new("RusTXT", "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title_widget));

        let tab_view = adw::TabView::new();
        let tab_bar = adw::TabBar::new();
        tab_bar.set_view(Some(&tab_view));
        tab_bar.set_autohide(false);
        tab_bar.set_expand_tabs(false);
        let new_tab = gtk::Button::from_icon_name("list-add-symbolic");
        new_tab.add_css_class("flat");
        new_tab.set_tooltip_text(Some("New tab (Ctrl+N)"));
        new_tab.set_action_name(Some("win.new-tab"));
        tab_bar.set_end_action_widget(Some(&new_tab));
        let tab_handle = gtk::WindowHandle::new();
        tab_handle.set_child(Some(&tab_bar));

        let menus = menus::build();
        let menubar = gtk::PopoverMenuBar::from_model(Some(&menus.bar));
        let menu_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        menu_row.add_css_class("rustxt-menubar");
        menu_row.append(&menubar);

        let search_settings = sourceview5::SearchSettings::new();
        let find_bar = FindBar::new(search_settings);
        let overlay = gtk::Overlay::new();
        overlay.set_vexpand(true);
        overlay.set_child(Some(&tab_view));
        overlay.add_overlay(&find_bar.root);
        tab_view.set_menu_model(Some(&menus.tab_context));

        let status = StatusBar::new();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&tab_handle);
        content.append(&menu_row);
        content.append(&overlay);
        content.append(&status.root);
        window.set_content(Some(&content));

        let this = Rc::new(Self {
            window,
            header,
            title_widget,
            tab_view,
            overlay,
            find_bar,
            status,
            theme_css: theme::install_provider(),
            zoom_css: theme::install_provider(),
            menus,
            docs: RefCell::new(Vec::new()),
            storage: RefCell::new(storage),
            paths,
            settings: RefCell::new(settings),
            scheme: RefCell::new(None),
            closed: RefCell::new(Vec::new()),
            pending: RefCell::new(HashSet::new()),
            flush_source: RefCell::new(None),
            menu_page: RefCell::new(None),
            discarding: RefCell::new(None),
            restoring: Cell::new(false),
            wheel_accumulator: Cell::new(0.0),
            _watcher: RefCell::new(None),
            _emergency_storage: emergency_storage,
        });

        this.install_actions();
        this.connect_signals();
        this.apply_settings();
        this.start_watcher();
        this.restore_session();
        this.refresh_closed();
        this
    }

    // -----------------------------------------------------------------------
    // Documents

    fn active(&self) -> Option<Rc<Document>> {
        let page = self.tab_view.selected_page()?;
        self.doc_for(&page)
    }

    fn doc_for(&self, page: &adw::TabPage) -> Option<Rc<Document>> {
        self.docs
            .borrow()
            .iter()
            .find(|doc| doc.page == *page)
            .cloned()
    }

    fn docs_in_order(&self) -> Vec<Rc<Document>> {
        (0..self.tab_view.n_pages())
            .filter_map(|i| self.doc_for(&self.tab_view.nth_page(i)))
            .collect()
    }

    fn restore_session(self: &Rc<Self>) {
        self.restoring.set(true);
        let session = self.storage.borrow().restore_session();
        match session {
            Ok(mut session) => {
                let before: Vec<String> = session.documents.iter().map(|d| d.id.clone()).collect();
                files::refresh_from_disk(&mut session.documents);
                for id in before
                    .iter()
                    .filter(|id| !session.documents.iter().any(|d| &d.id == *id))
                {
                    // A saved file that no longer exists has nothing to restore.
                    self.report(self.storage.borrow().close_document(id, true));
                }
                for state in session.documents {
                    self.add_document(state);
                }
                if let Some(active) = session.active_id {
                    if let Some(doc) = self.docs.borrow().iter().find(|d| d.id() == active) {
                        self.tab_view.set_selected_page(&doc.page);
                    }
                }
            }
            Err(error) => self.show_error(&error),
        }
        self.restoring.set(false);
        if self.tab_view.n_pages() == 0 {
            // Nothing to restore: start with one empty tab, recorded right away.
            self.new_document();
        }
        self.persist_layout();
        self.on_active_changed();
    }

    fn add_document(self: &Rc<Self>, state: DocumentState) -> Rc<Document> {
        let doc = Document::new(
            &self.tab_view,
            state,
            &self.find_bar.settings,
            self.scheme.borrow().as_ref(),
            self.settings.borrow().config.editor.word_wrap,
        );
        self.docs.borrow_mut().push(doc.clone());
        self.connect_document(&doc);
        self.tab_view.set_selected_page(&doc.page);
        doc.restore_scroll();
        if !self.restoring.get() {
            self.report(self.storage.borrow().save_snapshot(&doc.snapshot()));
            self.persist_layout();
        }
        doc.view.grab_focus();
        doc
    }

    fn new_document(self: &Rc<Self>) -> Rc<Document> {
        let titles: Vec<String> = self.docs.borrow().iter().map(|doc| doc.title()).collect();
        let state = DocumentState::untitled(titles.iter().map(String::as_str), LineEnding::Lf);
        self.add_document(state)
    }

    pub fn open_path(self: &Rc<Self>, path: &Path) {
        let path_text = path.to_string_lossy().into_owned();
        let existing = self
            .docs
            .borrow()
            .iter()
            .find(|d| {
                d.file_path()
                    .is_some_and(|open| files::same_file(Path::new(&open), path))
            })
            .cloned();
        if let Some(doc) = existing {
            self.tab_view.set_selected_page(&doc.page);
            return;
        }
        match files::read_document(&path_text) {
            Ok(state) => {
                self.add_document(state);
            }
            Err(error) => self.show_error(&error),
        }
    }

    fn open_dialog(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::new();
        dialog.set_title("Open");
        let this = self.clone();
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        this.open_path(&path);
                    }
                }
            },
        );
    }

    fn save(self: &Rc<Self>, doc: Rc<Document>, force_picker: bool) {
        match doc.file_path() {
            Some(path) if !force_picker => self.write_document(&doc, Path::new(&path)),
            current => {
                let dialog = gtk::FileDialog::new();
                dialog.set_title("Save as");
                match current {
                    Some(path) => dialog.set_initial_file(Some(&gio::File::for_path(path))),
                    None => dialog.set_initial_name(Some(&format!("{}.txt", doc.title()))),
                }
                let this = self.clone();
                dialog.save(
                    Some(&self.window),
                    None::<&gio::Cancellable>,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                this.write_document(&doc, &path);
                            }
                        }
                    },
                );
            }
        }
    }

    fn write_document(&self, doc: &Rc<Document>, path: &Path) {
        let mut state = doc.snapshot();
        let saving_over_original = state
            .file_path
            .as_deref()
            .is_some_and(|original| files::same_file(Path::new(original), path));
        if saving_over_original {
            match files::changed_on_disk(&state, path) {
                Ok(true) => {
                    self.show_error(
                        "This file changed on disk after RusTXT loaded it. To protect those changes, reload the file or use Save As.",
                    );
                    return;
                }
                Err(error) => {
                    self.show_error(&format!(
                        "The original file is no longer available ({error}). Use Save As to keep this text."
                    ));
                    return;
                }
                Ok(false) => {}
            }
        }
        if let Err(error) = files::save_document(&mut state, path) {
            self.show_error(&error);
            return;
        }
        self.pending.borrow_mut().remove(&state.id);
        doc.mark_saved(state.clone());
        self.report(self.storage.borrow().save_snapshot(&state));
        self.update_title();
        self.update_status();
        self.update_actions();
    }

    /// Save every unsaved tab. Untitled tabs need a name, so the first one is
    /// brought forward with the save dialog.
    fn save_all(self: &Rc<Self>) {
        let dirty: Vec<Rc<Document>> = self
            .docs_in_order()
            .into_iter()
            .filter(|d| d.is_dirty())
            .collect();
        for doc in &dirty {
            if let Some(path) = doc.file_path() {
                self.write_document(doc, Path::new(&path));
            }
        }
        if let Some(untitled) = dirty.into_iter().find(|d| d.file_path().is_none()) {
            self.tab_view.set_selected_page(&untitled.page);
            self.save(untitled, true);
        }
    }

    fn close_for_now(&self, page: &adw::TabPage) {
        self.tab_view.close_page(page);
    }

    fn discard(self: &Rc<Self>, page: adw::TabPage) {
        let Some(doc) = self.doc_for(&page) else {
            return;
        };
        if !doc.is_dirty() {
            *self.discarding.borrow_mut() = Some(page.clone());
            self.tab_view.close_page(&page);
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some("Discard changes?"),
            Some(&format!(
                "Unsaved changes to \"{}\" will be permanently lost.",
                doc.title()
            )),
        );
        dialog.add_responses(&[("cancel", "_Cancel"), ("discard", "_Discard")]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let this = self.clone();
        dialog.choose(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |response| {
                if response == "discard" {
                    *this.discarding.borrow_mut() = Some(page.clone());
                    this.tab_view.close_page(&page);
                }
            },
        );
    }

    /// Runs for every tab close, whether from the close button, Ctrl+W or the
    /// discard flow. Keeps the snapshot unless a discard was requested.
    fn handle_close_page(self: &Rc<Self>, page: &adw::TabPage) {
        let discard = self
            .discarding
            .borrow_mut()
            .take()
            .is_some_and(|marked| marked == *page);
        if let Some(doc) = self.doc_for(page) {
            let id = doc.id();
            if !discard && self.pending.borrow_mut().remove(&id) {
                self.report(self.storage.borrow().save_snapshot(&doc.snapshot()));
            }
            self.pending.borrow_mut().remove(&id);
            self.report(self.storage.borrow().close_document(&id, discard));
        }
        self.docs.borrow_mut().retain(|doc| doc.page != *page);
        self.tab_view.close_page_finish(page, true);
        self.refresh_closed();
        if self.tab_view.n_pages() == 0 {
            // Closing the last tab closes RusTXT. The snapshot
            // stays in the recovery database unless it was discarded.
            self.persist_layout();
            self.window.close();
            return;
        }
        self.persist_layout();
        self.on_active_changed();
    }

    fn reopen(self: &Rc<Self>, id: &str) {
        let reopened = self.storage.borrow().reopen_document(id);
        match reopened {
            Ok(Some(state)) => {
                let mut refreshed = vec![state];
                files::refresh_from_disk(&mut refreshed);
                let Some(mut state) = refreshed.pop() else {
                    self.report(self.storage.borrow().close_document(id, true));
                    self.show_error("That file is no longer on disk.");
                    self.refresh_closed();
                    return;
                };
                let titles: Vec<String> = self.docs.borrow().iter().map(|d| d.title()).collect();
                state.ensure_unique_title(titles.iter().map(String::as_str));
                let existing = state.file_path.as_deref().and_then(|path| {
                    self.docs
                        .borrow()
                        .iter()
                        .find(|d| d.file_path().as_deref() == Some(path))
                        .cloned()
                });
                match existing {
                    Some(doc) => {
                        self.tab_view.set_selected_page(&doc.page);
                        self.persist_layout(); // re-closes the duplicate row
                    }
                    None => {
                        self.add_document(state);
                    }
                }
            }
            Ok(None) => self.show_error("That document is no longer available."),
            Err(error) => self.show_error(&error),
        }
        self.refresh_closed();
    }

    fn refresh_closed(&self) {
        match self.storage.borrow().closed_documents() {
            Ok(closed) => *self.closed.borrow_mut() = closed,
            Err(error) => eprintln!("RusTXT: {error}"),
        }
        let menu = &self.menus.recently_closed;
        menu.remove_all();
        let closed = self.closed.borrow();
        if closed.is_empty() {
            // An action that does not exist renders the item insensitive.
            menu.append(Some("Nothing to reopen"), Some("win.unavailable"));
        }
        for doc in closed.iter() {
            let item = gio::MenuItem::new(Some(&doc.menu_label()), None);
            item.set_action_and_target_value(Some("win.reopen"), Some(&doc.id.to_variant()));
            menu.append_item(&item);
        }
        if let Some(action) = self.action("reopen-last") {
            action.set_enabled(!closed.is_empty());
        }
    }

    // -----------------------------------------------------------------------
    // Persistence

    fn persist_layout(&self) {
        if self.restoring.get() {
            return;
        }
        let order: Vec<String> = self.docs_in_order().iter().map(|doc| doc.id()).collect();
        let active = self.active().map(|doc| doc.id()).unwrap_or_default();
        self.report(self.storage.borrow_mut().update_layout(&order, &active));
    }

    fn schedule_flush(self: &Rc<Self>) {
        if let Some(source) = self.flush_source.borrow_mut().take() {
            source.remove();
        }
        let this = self.clone();
        let source = glib::timeout_add_local_once(SNAPSHOT_DELAY, move || {
            this.flush_source.borrow_mut().take();
            this.flush();
        });
        *self.flush_source.borrow_mut() = Some(source);
    }

    /// Write content snapshots for changed documents and the view state of
    /// the active one.
    fn flush(&self) {
        let pending: Vec<String> = self.pending.borrow_mut().drain().collect();
        let docs = self.docs.borrow().clone();
        let storage = self.storage.borrow();
        for doc in docs.iter().filter(|doc| pending.contains(&doc.id())) {
            self.report(storage.save_snapshot(&doc.snapshot()));
        }
        if let Some(doc) = self.active() {
            if !pending.contains(&doc.id()) {
                let (cursor, scroll) = doc.view_state();
                self.report(storage.save_view_state(&doc.id(), cursor, scroll));
            }
        }
    }

    fn save_window_state(&self) {
        let (width, height) = self.window.default_size();
        let value = format!(
            "{width} {height} {}",
            if self.window.is_maximized() { 1 } else { 0 }
        );
        self.report(self.storage.borrow().set_state("window", &value));
    }

    // -----------------------------------------------------------------------
    // Settings

    fn update_config(self: &Rc<Self>, mutate: impl FnOnce(&mut Config)) {
        let mut next = self.settings.borrow().config.clone();
        mutate(&mut next);
        if let Err(error) = config::save(&self.paths, &next) {
            self.show_error(&error);
        }
        self.apply_settings();
    }

    fn apply_settings(self: &Rc<Self>) {
        let settings = config::settings(&self.paths);
        let applied = theme::apply(&settings.theme, &self.theme_css, &self.paths.cache_dir);
        *self.scheme.borrow_mut() = applied.scheme.clone();
        self.zoom_css.load_from_string(&theme::editor_font_css(
            &settings.config.editor.font,
            settings.config.appearance.zoom,
        ));
        for doc in self.docs.borrow().iter() {
            doc.buffer.set_style_scheme(applied.scheme.as_ref());
            doc.set_wrap(settings.config.editor.word_wrap);
        }
        self.status
            .root
            .set_visible(settings.config.window.status_bar);
        self.window.set_decorated(settings.decorated);
        self.header.set_visible(settings.decorated);
        if let Some(action) = self.action("status-bar") {
            action.set_state(&settings.config.window.status_bar.to_variant());
        }
        if let Some(action) = self.action("word-wrap") {
            action.set_state(&settings.config.editor.word_wrap.to_variant());
        }
        *self.settings.borrow_mut() = settings;
        self.update_status();
    }

    fn zoom_by(self: &Rc<Self>, delta: i32) {
        let current = self.settings.borrow().config.appearance.zoom as i32;
        let next = ((current + delta) / ZOOM_STEP as i32 * ZOOM_STEP as i32).clamp(10, 500) as u32;
        if next != current as u32 {
            self.update_config(|config| config.appearance.zoom = next);
        }
    }

    fn start_watcher(self: &Rc<Self>) {
        let (sender, receiver) = async_channel::unbounded::<()>();
        match watch::watch(self.paths.watch_dirs(), move || {
            let _ = sender.send_blocking(());
        }) {
            Ok(watcher) => *self._watcher.borrow_mut() = Some(watcher),
            Err(error) => eprintln!("RusTXT: config watching disabled: {error}"),
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            while receiver.recv().await.is_ok() {
                this.apply_settings();
            }
        });
    }

    fn open_settings(self: &Rc<Self>) {
        let this = self.clone();
        let settings = self.settings.borrow().clone();
        settings_dialog::present(&self.window, settings, move |config| {
            this.update_config(|current| *current = config);
        });
    }

    // -----------------------------------------------------------------------
    // Presentation

    fn on_active_changed(&self) {
        self.update_title();
        self.update_status();
        self.update_actions();
        match self.active() {
            Some(doc) => self.find_bar.attach(Some((&doc.search, &doc.view))),
            None => self.find_bar.attach(None),
        }
    }

    fn update_title(&self) {
        let title = self
            .active()
            .map(|doc| doc.window_title())
            .unwrap_or_else(|| "RusTXT".into());
        self.window.set_title(Some(&title));
        self.title_widget.set_title(&title);
    }

    fn update_status(&self) {
        let Some(doc) = self.active() else {
            self.status.position.set_text("Ready");
            return;
        };
        let cursor = doc.buffer.iter_at_mark(&doc.buffer.get_insert());
        self.status.position.set_text(&format!(
            "Ln {}, Col {}",
            cursor.line() + 1,
            cursor.line_offset() + 1
        ));
        self.status
            .count
            .set_text(&format!("{} characters", doc.buffer.char_count()));
        self.status.zoom.set_text(&format!(
            "{}%",
            self.settings.borrow().config.appearance.zoom
        ));
        self.status
            .eol
            .set_text(match doc.state.borrow().line_ending {
                LineEnding::Crlf => "Windows (CRLF)",
                LineEnding::Lf => "Unix (LF)",
            });
    }

    fn update_actions(&self) {
        let doc = self.active();
        let enabled = |name: &str, on: bool| {
            if let Some(action) = self.action(name) {
                action.set_enabled(on);
            }
        };
        let has_selection = doc.as_ref().is_some_and(|d| d.buffer.has_selection());
        enabled("undo", doc.as_ref().is_some_and(|d| d.buffer.can_undo()));
        enabled("redo", doc.as_ref().is_some_and(|d| d.buffer.can_redo()));
        enabled("cut", has_selection);
        enabled("copy", has_selection);
        enabled("delete", has_selection);
        enabled("save-all", self.docs.borrow().iter().any(|d| d.is_dirty()));
        enabled("tab-close-others", self.tab_view.n_pages() > 1);
    }

    fn show_error(&self, message: &str) {
        eprintln!("RusTXT: {message}");
        let dialog = adw::AlertDialog::new(Some("RusTXT"), Some(message));
        dialog.add_responses(&[("ok", "_OK")]);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("ok");
        dialog.present(Some(&self.window));
    }

    fn report(&self, result: Result<(), String>) {
        if let Err(error) = result {
            self.show_error(&error);
        }
    }

    fn insert_time_date(&self) {
        let Some(doc) = self.active() else { return };
        let stamp = glib::DateTime::now_local()
            .ok()
            .and_then(|now| now.format("%X %x").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        doc.insert_at_cursor(&stamp);
    }

    fn go_to_line(self: &Rc<Self>) {
        let Some(doc) = self.active() else { return };
        let entry = gtk::Entry::new();
        entry.set_input_purpose(gtk::InputPurpose::Digits);
        entry.set_activates_default(true);
        let current = doc.buffer.iter_at_mark(&doc.buffer.get_insert()).line() + 1;
        entry.set_text(&current.to_string());
        let dialog = adw::AlertDialog::new(Some("Go to line"), None);
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "_Cancel"), ("go", "_Go to")]);
        dialog.set_response_appearance("go", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("go"));
        dialog.set_close_response("cancel");
        let field = entry.clone();
        dialog.choose(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |response| {
                if response == "go" {
                    if let Ok(line) = field.text().trim().parse::<i32>() {
                        doc.go_to_line(line);
                    }
                }
            },
        );
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    // -----------------------------------------------------------------------
    // Wiring

    fn action(&self, name: &str) -> Option<gio::SimpleAction> {
        self.window
            .lookup_action(name)
            .and_downcast::<gio::SimpleAction>()
    }

    fn add_action(self: &Rc<Self>, name: &str, handler: impl Fn(&Rc<Self>) + 'static) {
        let action = gio::SimpleAction::new(name, None);
        let this = self.clone();
        action.connect_activate(move |_, _| handler(&this));
        self.window.add_action(&action);
    }

    fn context_page(&self) -> Option<adw::TabPage> {
        self.menu_page
            .borrow()
            .clone()
            .or_else(|| self.tab_view.selected_page())
    }

    fn install_actions(self: &Rc<Self>) {
        self.add_action("new-tab", |w| {
            w.new_document();
        });
        self.add_action("open", |w| w.open_dialog());
        self.add_action("save", |w| {
            if let Some(doc) = w.active() {
                w.save(doc, false);
            }
        });
        self.add_action("save-as", |w| {
            if let Some(doc) = w.active() {
                w.save(doc, true);
            }
        });
        self.add_action("save-all", |w| w.save_all());
        self.add_action("print", |w| {
            if let Some(doc) = w.active() {
                printing::print(&w.window, &doc.view, &doc.title());
            }
        });
        self.add_action("close-tab", |w| {
            if let Some(page) = w.tab_view.selected_page() {
                w.close_for_now(&page);
            }
        });
        self.add_action("discard", |w| {
            if let Some(page) = w.tab_view.selected_page() {
                w.discard(page);
            }
        });
        self.add_action("reopen-last", |w| {
            let id = w.closed.borrow().first().map(|doc| doc.id.clone());
            if let Some(id) = id {
                w.reopen(&id);
            }
        });
        let reopen = gio::SimpleAction::new("reopen", Some(glib::VariantTy::STRING));
        let this = self.clone();
        reopen.connect_activate(move |_, parameter| {
            if let Some(id) = parameter.and_then(|p| p.get::<String>()) {
                this.reopen(&id);
            }
        });
        self.window.add_action(&reopen);
        self.add_action("settings", |w| w.open_settings());
        self.add_action("about", |w| about_dialog::present(&w.window));
        self.add_action("exit", |w| w.window.close());

        self.add_action("undo", |w| {
            if let Some(doc) = w.active() {
                doc.buffer.undo();
            }
        });
        self.add_action("redo", |w| {
            if let Some(doc) = w.active() {
                doc.buffer.redo();
            }
        });
        for (name, signal) in [
            ("cut", "cut-clipboard"),
            ("copy", "copy-clipboard"),
            ("paste", "paste-clipboard"),
        ] {
            self.add_action(name, move |w| {
                if let Some(doc) = w.active() {
                    doc.view.emit_by_name::<()>(signal, &[]);
                    doc.view.grab_focus();
                }
            });
        }
        self.add_action("delete", |w| {
            if let Some(doc) = w.active() {
                doc.buffer.delete_selection(true, true);
            }
        });
        self.add_action("select-all", |w| {
            if let Some(doc) = w.active() {
                doc.view.emit_by_name::<()>("select-all", &[&true]);
                doc.view.grab_focus();
            }
        });
        self.add_action("find", |w| w.find_bar.open(false));
        self.add_action("replace", |w| w.find_bar.open(true));
        self.add_action("find-next", |w| {
            if !w.find_bar.is_open() {
                w.find_bar.open(false);
            }
            w.find_bar.find_next();
        });
        self.add_action("find-previous", |w| {
            if !w.find_bar.is_open() {
                w.find_bar.open(false);
            }
            w.find_bar.find_previous();
        });
        self.add_action("goto", |w| w.go_to_line());
        self.add_action("time-date", |w| w.insert_time_date());

        self.add_action("zoom-in", |w| w.zoom_by(ZOOM_STEP as i32));
        self.add_action("zoom-out", |w| w.zoom_by(-(ZOOM_STEP as i32)));
        self.add_action("zoom-reset", |w| {
            w.update_config(|config| config.appearance.zoom = 100);
        });
        for (name, toggle) in [
            (
                "status-bar",
                (|config: &mut Config| config.window.status_bar = !config.window.status_bar)
                    as fn(&mut Config),
            ),
            ("word-wrap", |config: &mut Config| {
                config.editor.word_wrap = !config.editor.word_wrap
            }),
        ] {
            let action = gio::SimpleAction::new_stateful(name, None, &true.to_variant());
            let this = self.clone();
            action.connect_activate(move |_, _| this.update_config(toggle));
            self.window.add_action(&action);
        }

        // Tab context menu.
        self.add_action("tab-close", |w| {
            if let Some(page) = w.context_page() {
                w.close_for_now(&page);
            }
        });
        self.add_action("tab-close-others", |w| {
            if let Some(keep) = w.context_page() {
                for doc in w.docs_in_order() {
                    if doc.page != keep {
                        w.close_for_now(&doc.page);
                    }
                }
            }
        });
        self.add_action("tab-discard", |w| {
            if let Some(page) = w.context_page() {
                w.discard(page);
            }
        });
        self.add_action("tab-save", |w| {
            if let Some(doc) = w.context_page().and_then(|p| w.doc_for(&p)) {
                w.save(doc, false);
            }
        });
        self.add_action("tab-save-as", |w| {
            if let Some(doc) = w.context_page().and_then(|p| w.doc_for(&p)) {
                w.save(doc, true);
            }
        });
    }

    fn connect_signals(self: &Rc<Self>) {
        let this = self.clone();
        self.tab_view.connect_close_page(move |_, page| {
            this.handle_close_page(page);
            glib::Propagation::Stop
        });
        let this = self.clone();
        self.tab_view.connect_selected_page_notify(move |_| {
            this.on_active_changed();
            this.persist_layout();
        });
        let this = self.clone();
        self.tab_view
            .connect_page_reordered(move |_, _, _| this.persist_layout());
        let this = self.clone();
        self.tab_view.connect_setup_menu(move |_, page| {
            *this.menu_page.borrow_mut() = page.cloned();
        });

        let this = self.clone();
        self.window.connect_close_request(move |_| {
            if let Some(source) = this.flush_source.borrow_mut().take() {
                source.remove();
            }
            this.flush();
            this.persist_layout();
            this.save_window_state();
            glib::Propagation::Proceed
        });

        // Ctrl + wheel zooms. Capture phase, so the scrolled window does not
        // consume the event first.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = self.clone();
        scroll.connect_scroll(move |controller, _dx, dy| {
            if !controller
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let total = this.wheel_accumulator.get() + dy;
            if total.abs() >= 1.0 {
                this.zoom_by(if total < 0.0 {
                    ZOOM_STEP as i32
                } else {
                    -(ZOOM_STEP as i32)
                });
                this.wheel_accumulator.set(0.0);
            } else {
                this.wheel_accumulator.set(total);
            }
            glib::Propagation::Stop
        });
        self.overlay.add_controller(scroll);

        // Follow the desktop light/dark switch while in "system" mode.
        let this = self.clone();
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if this.settings.borrow().theme.mode == "system" {
                this.apply_settings();
            }
        });
    }

    fn connect_document(self: &Rc<Self>, doc: &Rc<Document>) {
        let id = doc.id();
        let this = self.clone();
        let doc_id = id.clone();
        doc.buffer.connect_changed(move |_| {
            this.pending.borrow_mut().insert(doc_id.clone());
            this.schedule_flush();
            this.update_status();
        });
        let this = self.clone();
        let weak = Rc::downgrade(doc);
        doc.buffer.connect_modified_changed(move |_| {
            if let Some(doc) = weak.upgrade() {
                doc.refresh_tab();
            }
            this.update_title();
            this.update_actions();
        });
        let this = self.clone();
        doc.buffer.connect_cursor_position_notify(move |_| {
            this.update_status();
            this.schedule_flush();
        });
        let this = self.clone();
        doc.buffer
            .connect_has_selection_notify(move |_| this.update_actions());
        let this = self.clone();
        doc.buffer
            .connect_can_undo_notify(move |_| this.update_actions());
        let this = self.clone();
        doc.buffer
            .connect_can_redo_notify(move |_| this.update_actions());
        let this = self.clone();
        doc.scroller
            .vadjustment()
            .connect_value_changed(move |_| this.schedule_flush());
    }
}
