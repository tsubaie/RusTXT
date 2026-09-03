//! Desktop environment detection for window decoration decisions. Tauri-free.
//!
//! Tiling Wayland compositors never draw a server-side title bar, so GTK's
//! client-side header bar is pure clutter there. Everywhere else the native
//! title bar is how people move, close and maximize windows, so it stays.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TitlebarMode {
    #[default]
    Auto,
    Show,
    Hide,
}

impl TitlebarMode {
    /// Whether the window should ask for native decorations.
    pub fn decorated(self, tiling_compositor: bool) -> bool {
        match self {
            Self::Show => true,
            Self::Hide => false,
            Self::Auto => !tiling_compositor,
        }
    }
}

const TILING_DESKTOPS: &[&str] = &["hyprland", "sway", "river", "niri", "qtile", "dwl", "mango"];
const TILING_SOCKETS: &[&str] = &["HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK", "NIRI_SOCKET"];

/// Decide from environment variables whether a tiling compositor is running.
pub fn is_tiling_compositor(env: &dyn Fn(&str) -> Option<String>) -> bool {
    if TILING_SOCKETS
        .iter()
        .any(|key| env(key).is_some_and(|value| !value.is_empty()))
    {
        return true;
    }
    let desktop = env("XDG_CURRENT_DESKTOP")
        .or_else(|| env("XDG_SESSION_DESKTOP"))
        .unwrap_or_default()
        .to_lowercase();
    desktop
        .split(':')
        .any(|name| TILING_DESKTOPS.contains(&name.trim()))
}

pub fn running_on_tiling_compositor() -> bool {
    cfg!(target_os = "linux") && is_tiling_compositor(&|key| std::env::var(key).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn detect(vars: HashMap<String, String>) -> bool {
        is_tiling_compositor(&|key| vars.get(key).cloned())
    }

    #[test]
    fn detects_hyprland_by_socket() {
        assert!(detect(env(&[
            ("HYPRLAND_INSTANCE_SIGNATURE", "abc"),
            ("XDG_CURRENT_DESKTOP", "Hyprland")
        ])));
    }

    #[test]
    fn detects_sway_by_desktop_name_list() {
        assert!(detect(env(&[("XDG_CURRENT_DESKTOP", "sway:wlroots")])));
    }

    #[test]
    fn gnome_and_kde_keep_decorations() {
        assert!(!detect(env(&[("XDG_CURRENT_DESKTOP", "GNOME")])));
        assert!(!detect(env(&[
            ("XDG_CURRENT_DESKTOP", "KDE"),
            ("XDG_SESSION_DESKTOP", "plasma")
        ])));
        assert!(!detect(HashMap::new()));
    }

    #[test]
    fn user_override_beats_detection() {
        assert!(TitlebarMode::Show.decorated(true));
        assert!(!TitlebarMode::Hide.decorated(false));
        assert!(!TitlebarMode::Auto.decorated(true));
        assert!(TitlebarMode::Auto.decorated(false));
    }
}
