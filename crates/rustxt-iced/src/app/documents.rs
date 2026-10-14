//! Document lifecycle: tabs, files, persistence, external changes and printing.

use super::{App, Message, EDITOR};
use crate::document::Document;
use iced::{
    widget::{operation, text_editor},
    window, Task,
};
use rustxt_core::{
    config, files,
    storage::{DocumentState, LineEnding},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

const INBOX_INTERVAL: Duration = Duration::from_secs(1);
/// Settle time before a burst of zoom notches or resize events hits disk.
const DEBOUNCE: Duration = Duration::from_millis(400);
const PRINT_MAX_AGE: Duration = Duration::from_secs(60 * 60);

impl App {
    pub(super) fn persist(&mut self) -> Result<(), String> {
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
    pub(super) fn flush_deferred(&mut self, force: bool) -> Result<(), String> {
        let due = |since: Option<Instant>| since.is_some_and(|t| force || t.elapsed() >= DEBOUNCE);
        if due(self.settings_dirty) {
            self.settings_dirty = None;
            config::save(&self.paths, &self.config)?;
            self.written_config = Some(self.config.clone());
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
    pub(super) fn new_document(&mut self) {
        let state = DocumentState::untitled(
            self.docs.iter().map(|d| d.state.title.as_str()),
            LineEnding::Lf,
        );
        self.docs.push(Document::new(state));
        self.active = self.docs.len() - 1;
    }
    pub(super) fn open_path(&mut self, path: PathBuf) {
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
            Err(error) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                let error = if error.contains(&name) {
                    error
                } else {
                    format!("{name}: {error}")
                };
                self.report(Err(error));
            }
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
    pub(super) fn update_documents(&mut self, message: Message) -> Task<Message> {
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
            Message::Tick => {
                if let Some(doc) = self.docs.get_mut(self.active) {
                    doc.restore_viewport();
                }
                match self.flush() {
                    Ok(()) => self.last_error = None,
                    Err(error) => {
                        if self.last_error.as_ref() != Some(&error) {
                            self.report(Err(format!(
                                "Your drafts could not be stored: {error}. Editing continues, \
                                 but changes are not recoverable until storing works again; \
                                 save important text to a file."
                            )));
                            self.last_error = Some(error);
                        }
                    }
                }
                let result = self.flush_deferred(false);
                self.report(result);
                if self.reload.swap(false, Ordering::Relaxed) {
                    let loaded = config::load(&self.paths);
                    if let Some(error) = loaded.error {
                        self.report(Err(error));
                    } else if self.written_config.as_ref() == Some(&loaded.config) {
                        // Our own save coming back through the watcher; the
                        // in-memory settings may already be ahead of it.
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
            Message::Print => {
                self.menu = None;
                let html = print_html(&self.doc().state.content);
                let result = write_print_file(&self.paths.cache_dir, &html).and_then(|path| {
                    webbrowser::open(&path.to_string_lossy()).map_err(|e| e.to_string())
                });
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
            _ => unreachable!("not a document message"),
        }
        Task::none()
    }
    pub(super) fn update_editing(&mut self, message: Message) -> Task<Message> {
        match message {
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
            Message::Date => {
                self.menu = None;
                let value = chrono::Local::now().format("%H:%M %Y-%m-%d").to_string();
                let range = self.doc().selected_range();
                self.doc_mut().replace(range, &value);
                return operation::focus(EDITOR);
            }
            _ => unreachable!("not an editing message"),
        }
        Task::none()
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

#[cfg(test)]
mod tests {
    use super::*;

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
