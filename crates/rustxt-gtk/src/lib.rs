//! RusTXT: a dead simple, recoverable plain text editor
//! built with GTK 4, libadwaita and GtkSourceView on top of `rustxt-core`.
//!
//! The binary is a one-liner over [`application`]; keeping the application
//! here lets the end-to-end tests run the real window in a child process.

mod about_dialog;
mod document;
mod find_bar;
mod menus;
mod printing;
mod settings_dialog;
mod theme;
pub mod window;

use adw::prelude::*;
use gtk::gio;

pub const APP_ID: &str = "com.tsubaie.rustxt";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The application with its startup, activate and open handlers attached.
/// `non_unique` skips single-instance registration, so a test copy opens its
/// own window instead of handing its files to the copy the user is typing in.
pub fn application(non_unique: bool) -> adw::Application {
    let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
    if non_unique {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();

    app.connect_startup(|app| {
        sourceview5::init();
        theme::install_base_css();
        menus::install_accelerators(app);
    });
    app.connect_activate(|app| window::RustxtWindow::obtain(app).present());
    app.connect_open(|app, files, _hint| {
        let window = window::RustxtWindow::obtain(app);
        window.present();
        for file in files {
            if let Some(path) = file.path() {
                window.open_path(&path);
            }
        }
    });
    app
}
