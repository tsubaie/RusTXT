//! The Settings page: an adaptive libadwaita preferences dialog whose every
//! control writes straight back to ~/.config/rustxt/config.toml.

use adw::prelude::*;
use rustxt_core::config::{Config, Settings};
use rustxt_core::desktop::TitlebarMode;
use std::{cell::RefCell, rc::Rc};

pub fn present(
    parent: &impl IsA<gtk::Widget>,
    settings: Settings,
    apply: impl Fn(Config) + 'static,
) {
    let apply: Rc<dyn Fn(Config)> = Rc::new(apply);
    let config = Rc::new(RefCell::new(settings.config.clone()));
    let change = {
        let config = config.clone();
        let apply = apply.clone();
        Rc::new(move |mutate: &dyn Fn(&mut Config)| {
            mutate(&mut config.borrow_mut());
            apply(config.borrow().clone());
        })
    };

    let page = adw::PreferencesPage::new();

    // Appearance ---------------------------------------------------------
    let appearance = adw::PreferencesGroup::new();
    appearance.set_title("Appearance");

    let (theme_values, theme_labels) = theme_options(&settings);
    let theme_row = adw::ComboRow::new();
    theme_row.set_title("App theme");
    theme_row.set_subtitle(&theme_description(&settings));
    let labels: Vec<&str> = theme_labels.iter().map(String::as_str).collect();
    theme_row.set_model(Some(&gtk::StringList::new(&labels)));
    let current = theme_values
        .iter()
        .position(|value| *value == settings.config.appearance.theme)
        .unwrap_or(0);
    theme_row.set_selected(current as u32);
    {
        let change = change.clone();
        theme_row.connect_selected_notify(move |row| {
            if let Some(value) = theme_values.get(row.selected() as usize).cloned() {
                change(&|config| config.appearance.theme = value.clone());
            }
        });
    }
    appearance.add(&theme_row);

    let titlebar_row = adw::ComboRow::new();
    titlebar_row.set_title("Window title bar");
    titlebar_row.set_subtitle(if settings.tiling_compositor {
        "Automatic hides it here because a tiling compositor is running"
    } else {
        "Automatic keeps the native title bar on this desktop"
    });
    titlebar_row.set_model(Some(&gtk::StringList::new(&[
        "Automatic",
        "Always show",
        "Always hide",
    ])));
    titlebar_row.set_selected(match settings.config.window.title_bar {
        TitlebarMode::Auto => 0,
        TitlebarMode::Show => 1,
        TitlebarMode::Hide => 2,
    });
    {
        let change = change.clone();
        titlebar_row.connect_selected_notify(move |row| {
            let mode = match row.selected() {
                1 => TitlebarMode::Show,
                2 => TitlebarMode::Hide,
                _ => TitlebarMode::Auto,
            };
            change(&|config| config.window.title_bar = mode);
        });
    }
    appearance.add(&titlebar_row);
    page.add(&appearance);

    // Text -----------------------------------------------------------------
    let text = adw::PreferencesGroup::new();
    text.set_title("Text");

    let zoom_row = adw::SpinRow::with_range(10.0, 500.0, 10.0);
    zoom_row.set_title("Zoom");
    zoom_row.set_subtitle("Percent. Also Ctrl + mouse wheel, Ctrl + plus and Ctrl + minus");
    zoom_row.set_value(settings.config.appearance.zoom as f64);
    {
        let change = change.clone();
        zoom_row.connect_value_notify(move |row| {
            let zoom = row.value().round() as u32;
            change(&|config| config.appearance.zoom = zoom);
        });
    }
    text.add(&zoom_row);

    let font_row = adw::ActionRow::new();
    font_row.set_title("Font");
    font_row.set_subtitle(if settings.config.editor.font.is_empty() {
        "Following the system monospace font, with Noto Naskh Arabic for Arabic text."
    } else {
        "Paired with Noto Naskh Arabic for Arabic text. List two families in config.toml to pick another, e.g. \"JetBrainsMono Nerd Font, Amiri 12\""
    });
    let font_button = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    font_button.set_valign(gtk::Align::Center);
    font_button.set_use_size(true);
    if !settings.config.editor.font.is_empty() {
        font_button.set_font_desc(&gtk::pango::FontDescription::from_string(
            &settings.config.editor.font,
        ));
    }
    {
        let change = change.clone();
        font_button.connect_font_desc_notify(move |button| {
            if let Some(description) = button.font_desc() {
                let font = description.to_str().to_string();
                change(&|config| config.editor.font = font.clone());
            }
        });
    }
    let font_reset = gtk::Button::with_label("System font");
    font_reset.set_valign(gtk::Align::Center);
    font_reset.add_css_class("flat");
    font_reset.set_sensitive(!settings.config.editor.font.is_empty());
    {
        let change = change.clone();
        font_reset.connect_clicked(move |_| change(&|config| config.editor.font.clear()));
    }
    font_row.add_suffix(&font_button);
    font_row.add_suffix(&font_reset);
    text.add(&font_row);

    let wrap_row = adw::SwitchRow::new();
    wrap_row.set_title("Word wrap");
    wrap_row.set_subtitle("Wrap long lines to the window width");
    wrap_row.set_active(settings.config.editor.word_wrap);
    {
        let change = change.clone();
        wrap_row.connect_active_notify(move |row| {
            let on = row.is_active();
            change(&|config| config.editor.word_wrap = on);
        });
    }
    text.add(&wrap_row);

    let status_row = adw::SwitchRow::new();
    status_row.set_title("Status bar");
    status_row.set_subtitle("Line and column, character count, zoom, line endings and encoding");
    status_row.set_active(settings.config.window.status_bar);
    {
        let change = change.clone();
        status_row.connect_active_notify(move |row| {
            let on = row.is_active();
            change(&|config| config.window.status_bar = on);
        });
    }
    text.add(&status_row);
    page.add(&text);

    // Configuration files ----------------------------------------------------
    let files = adw::PreferencesGroup::new();
    files.set_title("Configuration files");
    files.set_description(Some(
        "Edits made to these files, and Omarchy theme changes, apply immediately.",
    ));
    let config_row = adw::ActionRow::new();
    config_row.set_title("Settings");
    config_row.set_subtitle(&settings.config_path);
    config_row.set_subtitle_selectable(true);
    files.add(&config_row);
    let themes_row = adw::ActionRow::new();
    themes_row.set_title("Custom themes");
    themes_row.set_subtitle(&format!(
        "{}  — TOML files with mode, background, foreground and optional accent, muted, selection, border, chrome, menu",
        settings.config_path.replace("config.toml", "themes/")
    ));
    themes_row.set_subtitle_selectable(true);
    files.add(&themes_row);
    for warning in [
        settings.config_error.as_deref(),
        settings.theme.note.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let row = adw::ActionRow::new();
        row.set_title("Problem");
        row.set_subtitle(warning);
        row.add_css_class("error");
        files.add(&row);
    }
    page.add(&files);

    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Settings");
    dialog.set_search_enabled(false);
    dialog.add(&page);
    dialog.present(Some(parent));
}

fn theme_options(settings: &Settings) -> (Vec<String>, Vec<String>) {
    let mut values = vec!["auto", "system", "light", "dark"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let mut labels = vec![
        if settings.omarchy_available {
            "Automatic (Omarchy theme)".to_string()
        } else {
            "Automatic (system setting)".to_string()
        },
        "Use system setting".to_string(),
        "Light".to_string(),
        "Dark".to_string(),
    ];
    if settings.omarchy_available {
        values.push("omarchy".into());
        labels.push("Follow Omarchy theme".into());
    }
    for name in &settings.custom_themes {
        values.push(name.clone());
        labels.push(format!("Custom: {name}"));
    }
    let current = &settings.config.appearance.theme;
    if !values.contains(current) {
        values.push(current.clone());
        labels.push(format!("{current} (not found)"));
    }
    (values, labels)
}

fn theme_description(settings: &Settings) -> String {
    match settings.theme.source.as_str() {
        "omarchy" => "Following the active Omarchy theme".into(),
        "custom" => format!("Using themes/{}.toml", settings.theme.requested),
        "fallback" => "Falling back to the system setting".into(),
        _ => "Light and dark built-in looks, or follow the desktop".into(),
    }
}
