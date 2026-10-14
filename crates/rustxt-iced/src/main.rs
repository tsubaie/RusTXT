#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod document;
mod fonts;
mod icons;
mod instance;
mod platform;
mod search;
mod style;

fn main() -> iced::Result {
    app::run()
}
