mod commands;
mod config;
mod desktop;
mod files;
mod storage;
mod watch;

use std::{path::PathBuf, sync::Mutex};
use tauri::{Emitter, Manager};

pub struct Db(pub Mutex<storage::Storage>);

/// Event sent to the interface whenever config or theme files change on disk.
pub const SETTINGS_CHANGED: &str = "settings-changed";

fn paths(app: &tauri::App) -> Result<config::Paths, String> {
    let resolver = app.path();
    let config_dir = resolver
        .config_dir()
        .map_err(|e| e.to_string())?
        .join("rustpad");
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            resolver
                .home_dir()
                .ok()
                .map(|home| home.join(".local/state"))
        })
        .unwrap_or_else(|| PathBuf::from(".local/state"));
    Ok(config::Paths {
        config_dir,
        omarchy_theme_dir: state_home.join("omarchy/current/theme"),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let db_path = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("session.db");
            app.manage(Db(Mutex::new(storage::Storage::open(&db_path)?)));

            let paths = paths(app)?;
            // Write a commented default config on first launch so it is easy to find.
            if !paths.config_file().exists() {
                if let Err(error) = config::save(&paths, &config::Config::default()) {
                    eprintln!("RustPad: could not write default config: {error}");
                }
            }
            let settings = commands::settings(&paths);
            if let Some(window) = app.get_webview_window("main") {
                window.set_decorations(settings.decorated)?;
            }

            // Live reload: config edits and Omarchy theme changes apply immediately.
            let handle = app.handle().clone();
            match watch::watch(paths.watch_dirs(), move || {
                let _ = handle.emit(SETTINGS_CHANGED, ());
            }) {
                Ok(watcher) => {
                    app.manage(watcher);
                }
                Err(error) => eprintln!("RustPad: config watching disabled: {error}"),
            }
            app.manage(paths);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::restore_session,
            commands::save_snapshot,
            commands::save_view_state,
            commands::update_layout,
            commands::close_document,
            commands::list_closed_documents,
            commands::reopen_document,
            commands::open_document,
            commands::save_document,
            commands::get_settings,
            commands::update_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running RustPad");
}
