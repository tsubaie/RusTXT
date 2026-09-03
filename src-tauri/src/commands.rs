//! The thin Tauri command boundary over the Tauri-free core modules.

use crate::{
    config::{self, Config, Paths, ResolvedTheme},
    desktop, files,
    storage::{ClosedDocument, DocumentState, Session, Storage},
    Db,
};
use serde::Serialize;
use std::{path::Path, sync::MutexGuard};
use tauri::State;

fn lock<'a>(db: &'a State<'_, Db>) -> Result<MutexGuard<'a, Storage>, String> {
    db.0.lock().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Documents and session

#[tauri::command]
pub fn restore_session(db: State<Db>) -> Result<Session, String> {
    let mut session = lock(&db)?.restore_session()?;
    files::refresh_from_disk(&mut session.documents);
    Ok(session)
}

#[tauri::command]
pub fn save_snapshot(db: State<Db>, document: DocumentState) -> Result<(), String> {
    lock(&db)?.save_snapshot(&document)
}

#[tauri::command]
pub fn save_view_state(
    db: State<Db>,
    id: String,
    cursor_offset: i64,
    scroll_top: f64,
) -> Result<(), String> {
    lock(&db)?.save_view_state(&id, cursor_offset, scroll_top)
}

#[tauri::command]
pub fn update_layout(db: State<Db>, order: Vec<String>, active_id: String) -> Result<(), String> {
    lock(&db)?.update_layout(&order, &active_id)
}

#[tauri::command]
pub fn close_document(db: State<Db>, id: String, discard: bool) -> Result<(), String> {
    lock(&db)?.close_document(&id, discard)
}

#[tauri::command]
pub fn list_closed_documents(db: State<Db>) -> Result<Vec<ClosedDocument>, String> {
    lock(&db)?.closed_documents()
}

#[tauri::command]
pub fn reopen_document(db: State<Db>, id: String) -> Result<DocumentState, String> {
    let mut document = lock(&db)?
        .reopen_document(&id)?
        .ok_or_else(|| "That document is no longer available.".to_string())?;
    files::refresh_from_disk(std::slice::from_mut(&mut document));
    Ok(document)
}

#[tauri::command]
pub fn open_document(path: String) -> Result<DocumentState, String> {
    files::read_document(&path)
}

#[tauri::command]
pub fn save_document(
    db: State<Db>,
    mut document: DocumentState,
    path: String,
) -> Result<DocumentState, String> {
    let bytes = files::encode(&document.content, document.line_ending);
    files::atomic_save(Path::new(&path), bytes.as_bytes())?;
    document.title = files::title_for(Path::new(&path));
    document.file_path = Some(path);
    document.dirty = false;
    lock(&db)?.save_snapshot(&document)?;
    Ok(document)
}

// ---------------------------------------------------------------------------
// Settings

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub config: Config,
    pub theme: ResolvedTheme,
    pub custom_themes: Vec<String>,
    pub config_path: String,
    pub omarchy_available: bool,
    pub tiling_compositor: bool,
    pub decorated: bool,
    /// Set when config.toml exists but could not be parsed.
    pub config_error: Option<String>,
}

pub fn settings(paths: &Paths) -> Settings {
    let loaded = config::load(paths);
    let tiling_compositor = desktop::running_on_tiling_compositor();
    Settings {
        theme: config::resolve_theme(&loaded.config.appearance.theme, paths),
        custom_themes: config::list_custom_themes(paths),
        config_path: paths.config_file().display().to_string(),
        omarchy_available: paths.omarchy_available(),
        tiling_compositor,
        decorated: loaded.config.window.title_bar.decorated(tiling_compositor),
        config_error: loaded.error,
        config: loaded.config,
    }
}

#[tauri::command]
pub fn get_settings(window: tauri::WebviewWindow, paths: State<Paths>) -> Result<Settings, String> {
    let settings = settings(&paths);
    // The config file may have changed on disk; keep the window in step with it.
    window
        .set_decorations(settings.decorated)
        .map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
pub fn update_config(
    window: tauri::WebviewWindow,
    paths: State<Paths>,
    config: Config,
) -> Result<Settings, String> {
    config::save(&paths, &config.normalized())?;
    let settings = settings(&paths);
    window
        .set_decorations(settings.decorated)
        .map_err(|e| e.to_string())?;
    Ok(settings)
}
