//! RusTXT: a dead simple, recoverable plain text editor
//! built with GTK 4, libadwaita and GtkSourceView on top of `rustxt-core`.

mod about_dialog;
mod document;
mod find_bar;
mod menus;
mod printing;
mod settings_dialog;
mod theme;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "com.tsubaie.rustxt";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
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

    app.run()
}
