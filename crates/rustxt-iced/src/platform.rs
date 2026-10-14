//! System font discovery: GTK/fontconfig queries that run at most once per process.

use iced::Font;
use std::sync::OnceLock;

/// Match GTK/fontconfig aliases on Linux; retain Iced's portable fallback
/// elsewhere. Each alias spawns `fc-match` at most once per process.
pub fn system_font(alias: &str) -> Option<String> {
    static SANS: OnceLock<Option<String>> = OnceLock::new();
    static MONOSPACE: OnceLock<Option<String>> = OnceLock::new();
    match alias {
        "sans-serif" => SANS.get_or_init(|| query_system_font(alias)).clone(),
        "monospace" => MONOSPACE.get_or_init(|| query_system_font(alias)).clone(),
        other => query_system_font(other),
    }
}
fn query_system_font(alias: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("fc-match")
            .args(["-f", "%{family}", alias])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let name = String::from_utf8(output.stdout)
            .ok()?
            .split(',')
            .next()?
            .trim()
            .to_owned();
        (!name.is_empty()).then_some(name)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = alias;
        None
    }
}
/// The desktop's interface font family, queried from the system at most once.
pub fn system_ui_font() -> Option<String> {
    static FAMILY: OnceLock<Option<String>> = OnceLock::new();
    FAMILY.get_or_init(detect_system_ui_font).clone()
}
fn detect_system_ui_font() -> Option<String> {
    #[cfg(target_os = "linux")]
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "font-name"])
        .output()
    {
        if output.status.success() {
            let description = String::from_utf8_lossy(&output.stdout)
                .trim()
                .trim_matches('\'')
                .to_owned();
            if let Some((family, _)) = description.rsplit_once(' ') {
                return Some(family.to_owned());
            }
        }
    }
    system_font("sans-serif")
}
pub fn bold_font() -> Font {
    static FAMILY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let family = FAMILY.get_or_init(|| system_ui_font().unwrap_or_default());
    Font {
        weight: iced::font::Weight::Bold,
        ..if family.is_empty() {
            Font::DEFAULT
        } else {
            Font::with_name(family)
        }
    }
}
