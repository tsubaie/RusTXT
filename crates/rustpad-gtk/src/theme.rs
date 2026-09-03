//! Turns a resolved theme into libadwaita CSS variables and a GtkSourceView
//! style scheme, so Omarchy and custom palettes color the whole window.

use rustpad_core::config::{Palette, ResolvedTheme};
use std::{cell::Cell, fs, path::Path};

const BASE_CSS: &str = r#"
.rustpad-menubar { padding: 0 4px; }
.rustpad-menubar menubar { background: transparent; }
.rustpad-menubar > menubar > item { padding: 6px 10px; border-radius: 6px; }
.rustpad-status {
  padding: 3px 12px;
  font-size: 0.85em;
  border-top: 1px solid alpha(currentColor, 0.12);
}
.rustpad-status label { opacity: 0.75; }
.rustpad-status separator { margin: 3px 10px; opacity: 0.5; }
.find-card { padding: 6px; }
.find-card entry { min-width: 170px; }
.find-count { font-size: 0.85em; opacity: 0.75; margin: 0 4px; }
.find-count.error { color: var(--error-color); opacity: 1; }
.find-card button.option { font-weight: bold; font-size: 0.85em; }
"#;

/// The application logo, embedded so About works without an installed icon theme.
pub fn logo_texture() -> gtk::gdk::Texture {
    static LOGO: &[u8] = include_bytes!("../../../data/icons/rustpad-256.png");
    gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_static(LOGO))
        .expect("embedded logo is a valid PNG")
}

/// CSS for the editor font: the configured description (or the system
/// monospace font when empty) scaled by the zoom percentage.
pub fn editor_font_css(font: &str, zoom: u32) -> String {
    let scale = zoom as f64 / 100.0;
    let font = font.trim();
    if font.is_empty() {
        return format!(
            "textview.rustpad-editor {{ font-size: {:.1}px; }}",
            15.0 * scale
        );
    }
    let description = gtk::pango::FontDescription::from_string(font);
    let families: Vec<String> = description
        .family()
        .map(|list| {
            list.split(',')
                .map(str::trim)
                .filter(|family| !family.is_empty())
                .map(|family| format!("\"{family}\""))
                .collect()
        })
        .unwrap_or_default();
    let mut rules = Vec::new();
    if !families.is_empty() {
        rules.push(format!("font-family: {};", families.join(", ")));
    }
    let points = description.size() as f64 / gtk::pango::SCALE as f64;
    let base_px = if description.size() > 0 {
        if description.is_size_absolute() {
            points
        } else {
            points * 96.0 / 72.0
        }
    } else {
        15.0
    };
    rules.push(format!("font-size: {:.1}px;", base_px * scale));
    if description.style() == gtk::pango::Style::Italic {
        rules.push("font-style: italic;".into());
    }
    use gtk::pango::Weight;
    let weight = match description.weight() {
        Weight::Thin => 100,
        Weight::Ultralight => 200,
        Weight::Light => 300,
        Weight::Semilight => 350,
        Weight::Book => 380,
        Weight::Medium => 500,
        Weight::Semibold => 600,
        Weight::Bold => 700,
        Weight::Ultrabold => 800,
        Weight::Heavy => 900,
        Weight::Ultraheavy => 1000,
        _ => 400,
    };
    if weight != 400 {
        rules.push(format!("font-weight: {weight};"));
    }
    format!("textview.rustpad-editor {{ {} }}", rules.join(" "))
}

/// Add the static application stylesheet once.
pub fn install_base_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(BASE_CSS);
    add_provider(&provider);
}

/// Create an empty provider the caller can reload at will.
pub fn install_provider() -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    add_provider(&provider);
    provider
}

fn add_provider(provider: &gtk::CssProvider) {
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

pub struct Applied {
    pub scheme: Option<sourceview5::StyleScheme>,
}

/// Apply `theme`: pick the libadwaita color scheme, load palette overrides
/// into `provider`, and return the matching GtkSourceView style scheme.
pub fn apply(theme: &ResolvedTheme, provider: &gtk::CssProvider, cache_dir: &Path) -> Applied {
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(match theme.mode.as_str() {
        "dark" => adw::ColorScheme::ForceDark,
        "light" => adw::ColorScheme::ForceLight,
        _ => adw::ColorScheme::Default,
    });
    let dark = match theme.mode.as_str() {
        "dark" => true,
        "light" => false,
        _ => manager.is_dark(),
    };

    match &theme.palette {
        Some(palette) => {
            provider.load_from_string(&palette_css(palette, dark));
            Applied {
                scheme: palette_scheme(palette, dark, cache_dir),
            }
        }
        None => {
            provider.load_from_string("");
            Applied {
                scheme: builtin_scheme(dark),
            }
        }
    }
}

fn builtin_scheme(dark: bool) -> Option<sourceview5::StyleScheme> {
    sourceview5::StyleSchemeManager::default().scheme(if dark { "Adwaita-dark" } else { "Adwaita" })
}

fn palette_css(palette: &Palette, dark: bool) -> String {
    let bg = &palette.background;
    let fg = &palette.foreground;
    let accent = palette
        .accent
        .clone()
        .unwrap_or_else(|| default_accent(dark).to_string());
    let chrome = palette.chrome.clone().unwrap_or_else(|| {
        format!(
            "color-mix(in srgb, {bg} {}%, black)",
            if dark { 82 } else { 95 }
        )
    });
    let menu = palette
        .menu
        .clone()
        .unwrap_or_else(|| format!("color-mix(in srgb, {bg} 92%, {fg})"));
    let accent_fg = if dark {
        bg.clone()
    } else {
        "#ffffff".to_string()
    };
    format!(
        ":root {{
  --window-bg-color: {chrome}; --window-fg-color: {fg};
  --view-bg-color: {bg}; --view-fg-color: {fg};
  --headerbar-bg-color: {chrome}; --headerbar-fg-color: {fg};
  --headerbar-backdrop-color: {chrome};
  --popover-bg-color: {menu}; --popover-fg-color: {fg};
  --card-bg-color: {menu}; --card-fg-color: {fg};
  --dialog-bg-color: {menu}; --dialog-fg-color: {fg};
  --sidebar-bg-color: {chrome}; --sidebar-fg-color: {fg};
  --accent-bg-color: {accent}; --accent-fg-color: {accent_fg}; --accent-color: {accent};
}}
.rustpad-status {{ background-color: {chrome}; }}"
    )
}

fn default_accent(dark: bool) -> &'static str {
    if dark {
        "#4cc2ff"
    } else {
        "#005fb8"
    }
}

thread_local! {
    static SEARCH_PATH_ADDED: Cell<bool> = const { Cell::new(false) };
}

/// GtkSourceView schemes are XML files, so write one for the palette into the
/// cache directory and load it through the scheme manager.
fn palette_scheme(
    palette: &Palette,
    dark: bool,
    cache_dir: &Path,
) -> Option<sourceview5::StyleScheme> {
    let fg = &palette.foreground;
    let bg = &palette.background;
    let accent = palette
        .accent
        .clone()
        .unwrap_or_else(|| default_accent(dark).to_string());
    let selection = palette
        .selection
        .clone()
        .or_else(|| mix_hex(&accent, bg, 0.35))
        .unwrap_or_else(|| accent.clone());
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="rustpad" name="RustPad" version="1.0">
  <style name="text" foreground="{fg}" background="{bg}"/>
  <style name="selection" background="{selection}"/>
  <style name="selection-unfocused" background="{selection}"/>
  <style name="cursor" foreground="{fg}"/>
  <style name="secondary-cursor" foreground="{fg}"/>
  <style name="current-line" background="{bg}"/>
  <style name="search-match" background="{accent}" foreground="{bg}"/>
</style-scheme>
"#
    );

    let dir = cache_dir.join("styles");
    if let Err(error) =
        fs::create_dir_all(&dir).and_then(|_| fs::write(dir.join("rustpad.xml"), xml))
    {
        eprintln!("RustPad: could not write style scheme: {error}");
        return builtin_scheme(dark);
    }
    let manager = sourceview5::StyleSchemeManager::default();
    if !SEARCH_PATH_ADDED.replace(true) {
        manager.append_search_path(&dir.to_string_lossy());
    }
    manager.force_rescan();
    manager.scheme("rustpad").or_else(|| builtin_scheme(dark))
}

/// Blend two `#rrggbb` colors; `amount` is the weight of `a`.
fn mix_hex(a: &str, b: &str, amount: f64) -> Option<String> {
    let (ra, ga, ba) = parse_hex(a)?;
    let (rb, gb, bb) = parse_hex(b)?;
    let mix = |x: u8, y: u8| ((x as f64) * amount + (y as f64) * (1.0 - amount)).round() as u8;
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        mix(ra, rb),
        mix(ga, gb),
        mix(ba, bb)
    ))
}

fn parse_hex(color: &str) -> Option<(u8, u8, u8)> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}
