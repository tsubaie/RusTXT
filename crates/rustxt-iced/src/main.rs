#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod document;
mod instance;
mod search;

fn main() -> iced::Result {
    app::run()
}
