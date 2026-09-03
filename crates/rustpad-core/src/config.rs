//! User configuration and theme resolution. Tauri-free.
//!
//! Everything the user can tune lives in `$XDG_CONFIG_HOME/rustpad/config.toml`
//! (normally `~/.config/rustpad/config.toml`), following the same convention as
//! every other desktop application. Custom themes are TOML files in
//! `~/.config/rustpad/themes/`. On Omarchy the active theme's `colors.toml`
//! is read directly, so RustPad follows the desktop theme with no setup.

use crate::desktop::{self, TitlebarMode};
use crate::files;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const CONFIG_HEADER: &str = "\
# RustPad configuration.
#
# appearance.theme: \"auto\" (Omarchy theme when available, otherwise the system
#   light/dark setting), \"system\", \"light\", \"dark\", \"omarchy\", or the name of a
#   file in the themes/ directory next to this file (without .toml).
# appearance.zoom: editor zoom percentage, 10-500.
# editor.font: empty follows the system monospace font, with Noto Naskh Arabic
#   for Arabic text. A single family such as \"JetBrainsMono Nerd Font 12\" is
#   paired with Noto Naskh Arabic the same way; list two families yourself,
#   e.g. \"JetBrainsMono Nerd Font, Amiri 12\", to choose a different Arabic face.
# window.title_bar: \"auto\" hides the native title bar on tiling compositors such
#   as Hyprland and keeps it elsewhere; \"show\" and \"hide\" force it.
#
# Changes made here apply immediately while RustPad is running.

";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Config {
    pub appearance: Appearance,
    pub editor: EditorConfig,
    pub window: WindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Appearance {
    pub theme: String,
    pub zoom: u32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "auto".into(),
            zoom: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EditorConfig {
    pub word_wrap: bool,
    /// Pango font description such as "JetBrainsMono Nerd Font 12". Empty follows
    /// the system monospace font. Noto Naskh Arabic covers Arabic unless a
    /// comma-separated list names a second family, e.g. "Fira Code, Amiri 12".
    pub font: String,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            word_wrap: true,
            font: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowConfig {
    pub status_bar: bool,
    pub title_bar: TitlebarMode,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            status_bar: true,
            title_bar: TitlebarMode::Auto,
        }
    }
}

impl Config {
    pub fn normalized(mut self) -> Self {
        self.appearance.zoom = self.appearance.zoom.clamp(10, 500);
        if self.appearance.theme.trim().is_empty() {
            self.appearance.theme = "auto".into();
        }
        self
    }
}

/// Where RustPad's own files and the Omarchy theme state live.
#[derive(Debug, Clone)]
pub struct Paths {
    /// `~/.config/rustpad`
    pub config_dir: PathBuf,
    /// `~/.local/share/rustpad`
    pub data_dir: PathBuf,
    /// `~/.cache/rustpad`
    pub cache_dir: PathBuf,
    /// `~/.local/state/omarchy/current/theme`
    pub omarchy_theme_dir: PathBuf,
}

impl Paths {
    /// Standard XDG locations: `~/.config/rustpad` and the Omarchy theme state.
    /// The same layout is used on macOS so the config is where users expect a
    /// dotfile-style editor to keep it.
    pub fn discover() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let xdg = |var: &str, fallback: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
        };
        Self {
            config_dir: xdg("XDG_CONFIG_HOME", ".config").join("rustpad"),
            data_dir: xdg("XDG_DATA_HOME", ".local/share").join("rustpad"),
            cache_dir: xdg("XDG_CACHE_HOME", ".cache").join("rustpad"),
            omarchy_theme_dir: xdg("XDG_STATE_HOME", ".local/state").join("omarchy/current/theme"),
        }
    }

    /// Recovery database location.
    pub fn session_db(&self) -> PathBuf {
        self.data_dir.join("session.db")
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join("themes")
    }

    pub fn omarchy_available(&self) -> bool {
        self.omarchy_theme_dir.join("colors.toml").is_file()
    }

    /// Directories worth watching for live reloads. Follows the config file's
    /// symlink so edits to a dotfiles-managed target are noticed too.
    pub fn watch_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.config_dir.clone(), self.themes_dir()];
        if let Ok(real) = fs::canonicalize(self.config_file()) {
            if let Some(parent) = real.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
        if let Some(current) = self.omarchy_theme_dir.parent() {
            dirs.push(current.to_path_buf());
        }
        dirs.push(self.omarchy_theme_dir.clone());
        dirs.sort();
        dirs.dedup();
        dirs.into_iter().filter(|dir| dir.is_dir()).collect()
    }
}

/// Result of reading the config file: the config plus a parse error, if any,
/// so the UI can tell the user instead of silently falling back.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub config: Config,
    pub error: Option<String>,
}

pub fn load(paths: &Paths) -> Loaded {
    let path = paths.config_file();
    match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(config) => Loaded {
                config: config.normalized(),
                error: None,
            },
            Err(error) => Loaded {
                config: Config::default(),
                error: Some(format!(
                    "{} could not be parsed: {}",
                    path.display(),
                    error.message()
                )),
            },
        },
        Err(_) => Loaded {
            config: Config::default(),
            error: None,
        },
    }
}

pub fn save(paths: &Paths, config: &Config) -> Result<(), String> {
    fs::create_dir_all(&paths.config_dir).map_err(|e| e.to_string())?;
    let body = toml::to_string_pretty(config).map_err(|e| e.to_string())?;
    files::atomic_save(
        &paths.config_file(),
        format!("{CONFIG_HEADER}{body}").as_bytes(),
    )
}

/// Everything the interface needs to apply the user's configuration.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub config: Config,
    pub theme: ResolvedTheme,
    pub custom_themes: Vec<String>,
    pub config_path: String,
    pub omarchy_available: bool,
    pub tiling_compositor: bool,
    /// Whether the window should show native decorations.
    pub decorated: bool,
    /// Set when config.toml exists but could not be parsed.
    pub config_error: Option<String>,
}

pub fn settings(paths: &Paths) -> Settings {
    let loaded = load(paths);
    let tiling_compositor = desktop::running_on_tiling_compositor();
    Settings {
        theme: resolve_theme(&loaded.config.appearance.theme, paths),
        custom_themes: list_custom_themes(paths),
        config_path: paths.config_file().display().to_string(),
        omarchy_available: paths.omarchy_available(),
        tiling_compositor,
        decorated: loaded.config.window.title_bar.decorated(tiling_compositor),
        config_error: loaded.error,
        config: loaded.config,
    }
}

/// Create a commented default config.toml if none exists yet, so the file is
/// easy to discover. Errors are returned for logging; startup continues.
pub fn ensure_config_file(paths: &Paths) -> Result<bool, String> {
    if paths.config_file().exists() {
        return Ok(false);
    }
    save(paths, &Config::default())?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Themes

/// Colors a theme may define. Only `mode`, `background` and `foreground` are
/// required; the interface derives sensible values for anything omitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Palette {
    /// "dark" or "light"; controls native widget colors and derived shades.
    pub mode: String,
    /// Editor background.
    pub background: String,
    /// Tab strip background; defaults to a darker shade of `background`.
    pub chrome: Option<String>,
    pub foreground: String,
    pub muted: Option<String>,
    pub accent: Option<String>,
    pub selection: Option<String>,
    pub border: Option<String>,
    /// Menu and popup background.
    pub menu: Option<String>,
}

impl Palette {
    fn valid(&self) -> bool {
        (self.mode == "dark" || self.mode == "light")
            && !self.background.is_empty()
            && !self.foreground.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTheme {
    /// What the config asked for.
    pub requested: String,
    /// "system", "light", "dark", "omarchy", "custom" or "fallback".
    pub source: String,
    /// "light", "dark" or "system" (let the OS preference decide).
    pub mode: String,
    pub palette: Option<Palette>,
    /// Why a fallback happened, for the settings page.
    pub note: Option<String>,
}

impl ResolvedTheme {
    fn builtin(requested: &str, mode: &str) -> Self {
        Self {
            requested: requested.into(),
            source: mode.into(),
            mode: mode.into(),
            palette: None,
            note: None,
        }
    }

    fn fallback(requested: &str, note: String) -> Self {
        Self {
            requested: requested.into(),
            source: "fallback".into(),
            mode: "system".into(),
            palette: None,
            note: Some(note),
        }
    }
}

pub fn resolve_theme(requested: &str, paths: &Paths) -> ResolvedTheme {
    match requested {
        "system" | "light" | "dark" => ResolvedTheme::builtin(requested, requested),
        "auto" => {
            if paths.omarchy_available() {
                omarchy_theme("auto", paths)
            } else {
                ResolvedTheme::builtin("auto", "system")
            }
        }
        "omarchy" => omarchy_theme("omarchy", paths),
        name => {
            let path = paths.themes_dir().join(format!("{name}.toml"));
            match read_palette(&path) {
                Ok(palette) => ResolvedTheme {
                    requested: name.into(),
                    source: "custom".into(),
                    mode: palette.mode.clone(),
                    palette: Some(palette),
                    note: None,
                },
                Err(error) => ResolvedTheme::fallback(
                    name,
                    format!("Theme \"{name}\" could not be loaded: {error}"),
                ),
            }
        }
    }
}

fn omarchy_theme(requested: &str, paths: &Paths) -> ResolvedTheme {
    // A theme may ship an explicit rustpad.toml; otherwise derive from colors.toml.
    let explicit = paths.omarchy_theme_dir.join("rustpad.toml");
    let palette = if explicit.is_file() {
        read_palette(&explicit)
    } else {
        fs::read_to_string(paths.omarchy_theme_dir.join("colors.toml"))
            .map_err(|e| e.to_string())
            .and_then(|text| palette_from_omarchy_colors(&text))
    };
    match palette {
        Ok(palette) => ResolvedTheme {
            requested: requested.into(),
            source: "omarchy".into(),
            mode: palette.mode.clone(),
            palette: Some(palette),
            note: None,
        },
        Err(error) => {
            ResolvedTheme::fallback(requested, format!("Omarchy theme unavailable: {error}"))
        }
    }
}

fn read_palette(path: &Path) -> Result<Palette, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let palette: Palette = toml::from_str(&text).map_err(|e| e.message().to_string())?;
    if palette.valid() {
        Ok(palette)
    } else {
        Err("a theme needs mode (\"dark\" or \"light\"), background and foreground".into())
    }
}

/// Map an Omarchy `colors.toml` onto RustPad's palette.
pub fn palette_from_omarchy_colors(text: &str) -> Result<Palette, String> {
    let table: toml::Table = toml::from_str(text).map_err(|e| e.message().to_string())?;
    let get = |key: &str| table.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let palette = Palette {
        mode: get("mode").unwrap_or_else(|| "dark".into()),
        background: get("background").ok_or("colors.toml has no background")?,
        chrome: get("dark_background"),
        foreground: get("foreground").ok_or("colors.toml has no foreground")?,
        muted: get("dark_foreground").or_else(|| get("muted")),
        accent: get("accent").or_else(|| get("blue")),
        selection: get("selection"),
        border: get("lighter_background"),
        menu: get("lighter_background"),
    };
    if palette.valid() {
        Ok(palette)
    } else {
        Err("colors.toml is missing mode, background or foreground".into())
    }
}

pub fn list_custom_themes(paths: &Paths) -> Vec<String> {
    let Ok(entries) = fs::read_dir(paths.themes_dir()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension()? == "toml").then(|| path.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    const OMARCHY_COLORS: &str = r##"
mode = "dark"
accent = "#89b4fa"
selection = "#45475a"
muted = "#585b70"
background = "#1e1e2e"
dark_background = "#161622"
lighter_background = "#313244"
foreground = "#cdd6f4"
dark_foreground = "#6c7086"
blue = "#89b4fa"
"##;

    fn paths(root: &Path) -> Paths {
        Paths {
            config_dir: root.join("config/rustpad"),
            data_dir: root.join("data/rustpad"),
            cache_dir: root.join("cache/rustpad"),
            omarchy_theme_dir: root.join("state/omarchy/current/theme"),
        }
    }

    #[test]
    fn missing_config_yields_defaults_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(&paths(dir.path()));
        assert_eq!(
            loaded,
            Loaded {
                config: Config::default(),
                error: None
            }
        );
        assert_eq!(loaded.config.appearance.theme, "auto");
    }

    #[test]
    fn config_round_trips_and_partial_files_fill_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let mut config = Config::default();
        config.appearance.theme = "omarchy".into();
        config.appearance.zoom = 130;
        config.editor.word_wrap = false;
        config.window.title_bar = TitlebarMode::Hide;
        save(&paths, &config).unwrap();
        assert!(fs::read_to_string(paths.config_file())
            .unwrap()
            .starts_with("# RustPad configuration."));
        assert_eq!(load(&paths).config, config);

        fs::write(paths.config_file(), "[appearance]\nzoom = 900\n").unwrap();
        let loaded = load(&paths);
        assert_eq!(loaded.error, None);
        assert_eq!(loaded.config.appearance.zoom, 500, "zoom is clamped");
        assert!(
            loaded.config.editor.word_wrap,
            "unspecified sections keep defaults"
        );
    }

    #[test]
    fn broken_config_reports_error_and_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        fs::create_dir_all(&paths.config_dir).unwrap();
        fs::write(paths.config_file(), "appearance = [").unwrap();
        let loaded = load(&paths);
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.error.unwrap().contains("could not be parsed"));
    }

    #[test]
    fn omarchy_colors_map_onto_palette() {
        let palette = palette_from_omarchy_colors(OMARCHY_COLORS).unwrap();
        assert_eq!(palette.mode, "dark");
        assert_eq!(palette.background, "#1e1e2e");
        assert_eq!(palette.chrome.as_deref(), Some("#161622"));
        assert_eq!(palette.accent.as_deref(), Some("#89b4fa"));
        assert_eq!(palette.muted.as_deref(), Some("#6c7086"));
        assert_eq!(palette.menu.as_deref(), Some("#313244"));
    }

    #[test]
    fn auto_prefers_omarchy_when_present_and_system_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        assert_eq!(resolve_theme("auto", &paths).mode, "system");

        fs::create_dir_all(&paths.omarchy_theme_dir).unwrap();
        fs::write(paths.omarchy_theme_dir.join("colors.toml"), OMARCHY_COLORS).unwrap();
        let resolved = resolve_theme("auto", &paths);
        assert_eq!(resolved.source, "omarchy");
        assert_eq!(resolved.palette.unwrap().foreground, "#cdd6f4");
    }

    #[test]
    fn explicit_rustpad_toml_in_theme_wins_over_derivation() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        fs::create_dir_all(&paths.omarchy_theme_dir).unwrap();
        fs::write(paths.omarchy_theme_dir.join("colors.toml"), OMARCHY_COLORS).unwrap();
        fs::write(
            paths.omarchy_theme_dir.join("rustpad.toml"),
            "mode = \"light\"\nbackground = \"#ffffff\"\nforeground = \"#000000\"\n",
        )
        .unwrap();
        let resolved = resolve_theme("omarchy", &paths);
        assert_eq!(resolved.mode, "light");
        assert_eq!(resolved.palette.unwrap().background, "#ffffff");
    }

    #[test]
    fn custom_themes_are_listed_and_loaded_and_missing_ones_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        fs::create_dir_all(paths.themes_dir()).unwrap();
        fs::write(
            paths.themes_dir().join("solar.toml"),
            "mode = \"dark\"\nbackground = \"#002b36\"\nforeground = \"#839496\"\naccent = \"#b58900\"\n",
        )
        .unwrap();
        fs::write(paths.themes_dir().join("notes.txt"), "ignored").unwrap();
        assert_eq!(list_custom_themes(&paths), vec!["solar".to_string()]);

        let resolved = resolve_theme("solar", &paths);
        assert_eq!(resolved.source, "custom");
        assert_eq!(resolved.palette.unwrap().accent.as_deref(), Some("#b58900"));

        let missing = resolve_theme("nope", &paths);
        assert_eq!(missing.source, "fallback");
        assert_eq!(missing.mode, "system");
        assert!(missing.note.unwrap().contains("nope"));
    }

    #[test]
    fn builtin_names_resolve_directly() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        for name in ["system", "light", "dark"] {
            let resolved = resolve_theme(name, &paths);
            assert_eq!(resolved.mode, name);
            assert!(resolved.palette.is_none());
        }
    }
}
