//! Settings panel, font picker and appearance handling.

use super::{message::Choice, App, Message, CAPTION, UI};
use crate::{
    icons::{icon, Icon},
    platform::bold_font,
};
use iced::{
    widget::{
        self, button, column, container, pick_list, row, scrollable, space, text, text_input,
        toggler,
    },
    window, Element, Fill, Font, Task, Theme,
};
use rustxt_core::{config, desktop};
use std::time::Instant;

/// Editor text size in points when no font is configured; shown in Settings.
pub(super) const DEFAULT_FONT_SIZE: u32 = 11;

impl App {
    pub(super) fn apply_appearance(&mut self) {
        let resolved = config::resolve_theme(&self.config.appearance.theme, &self.paths);
        if let Some(note) = resolved.note.clone() {
            self.report(Err(note));
        }
        self.ui_palette = resolved.palette.clone();
        self.color_cache.set(None);
        self.theme = if let Some(p) = resolved.palette {
            let base = if p.mode == "dark" {
                Theme::Dark
            } else {
                Theme::Light
            }
            .palette();
            Some(Theme::custom(
                "RusTXT",
                iced::theme::Palette {
                    background: p.background.parse().ok().unwrap_or(base.background),
                    text: p.foreground.parse().ok().unwrap_or(base.text),
                    primary: p
                        .accent
                        .as_deref()
                        .and_then(|c| c.parse().ok())
                        .unwrap_or(base.primary),
                    ..base
                },
            ))
        } else {
            match resolved.mode.as_str() {
                "dark" => Some(Theme::Dark),
                "light" => Some(Theme::Light),
                _ => None,
            }
        };
        let (family, size) = parse_font(&self.config.editor.font);
        self.editor_size = size.clamp(6.0, 96.0);
        let family = family.split(',').next().unwrap_or("").trim();
        let family = if family.is_empty() {
            self.system_monospace.as_str()
        } else {
            family
        };
        self.editor_font = if family.is_empty() {
            Font::MONOSPACE
        } else {
            let name = self
                .font_names
                .iter()
                .copied()
                .find(|name| *name == family)
                .unwrap_or_else(|| {
                    // Font requires a static family. Intern each distinct preference once.
                    let name = Box::leak(family.to_owned().into_boxed_str());
                    self.font_names.push(name);
                    name
                });
            Font::with_name(name)
        };
    }
    pub(super) fn save_settings(&mut self) -> Task<Message> {
        self.apply_appearance();
        self.settings_dirty = None;
        let result = config::save(&self.paths, &self.config);
        if result.is_ok() {
            self.written_config = Some(self.config.clone());
        }
        self.report(result);
        Task::none()
    }
    pub(super) fn update_settings(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ZoomBy(amount) => {
                return self.update(Message::Zoom(
                    (self.config.appearance.zoom as i32 + amount).clamp(10, 500) as u32,
                ))
            }
            Message::Settings => {
                self.menu = None;
                self.font_draft = self.config.editor.font.clone();
                self.custom_themes = config::list_custom_themes(&self.paths);
                self.settings_open = true;
            }
            Message::Wrap(value) => {
                self.menu = None;
                self.submenu = None;
                self.config.editor.word_wrap = value;
                return self.save_settings();
            }
            Message::Status(value) => {
                self.menu = None;
                self.submenu = None;
                self.config.window.status_bar = value;
                return self.save_settings();
            }
            Message::Theme(value) => {
                self.config.appearance.theme = value;
                return self.save_settings();
            }
            Message::Font(value) => self.font_draft = value,
            Message::ChooseFont => {
                self.font_filter.clear();
                self.font_picker = true;
                if self.font_families.is_empty() {
                    let mut system = iced_graphics::text::font_system()
                        .write()
                        .expect("font system");
                    let mut families: Vec<String> = system
                        .raw()
                        .db()
                        .faces()
                        .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
                        .collect();
                    families.sort_unstable();
                    families.dedup();
                    self.font_families = families
                        .into_iter()
                        .map(|name| {
                            let lower = name.to_lowercase();
                            (name, lower)
                        })
                        .collect();
                }
            }
            Message::FontFilter(value) => self.font_filter = value,
            Message::FontChosen(value) => {
                let size = self
                    .font_draft
                    .rsplit_once(' ')
                    .and_then(|(_, size)| size.parse::<u32>().ok())
                    .unwrap_or(DEFAULT_FONT_SIZE);
                self.font_draft = format!("{value} {size}");
            }
            Message::ResetFont => {
                self.font_draft.clear();
                self.config.editor.font.clear();
                return self.save_settings();
            }
            Message::ApplyFont => {
                self.font_picker = false;
                self.config.editor.font = self.font_draft.clone();
                return self.save_settings();
            }
            Message::Zoom(value) => {
                self.menu = None;
                self.submenu = None;
                self.config.appearance.zoom = value.clamp(10, 500);
                // Wheel notches arrive in bursts: apply now, write once they settle.
                self.apply_appearance();
                self.settings_dirty = Some(Instant::now());
            }
            Message::Titlebar(value) => {
                let old = self
                    .config
                    .window
                    .title_bar
                    .decorated(desktop::running_on_tiling_compositor());
                self.config.window.title_bar = match value.as_str() {
                    "show" => desktop::TitlebarMode::Show,
                    "hide" => desktop::TitlebarMode::Hide,
                    _ => desktop::TitlebarMode::Auto,
                };
                let _ = self.save_settings();
                if old
                    != self
                        .config
                        .window
                        .title_bar
                        .decorated(desktop::running_on_tiling_compositor())
                {
                    return window::oldest().and_then(window::toggle_decorations);
                }
            }
            _ => unreachable!("not a settings message"),
        }
        Task::none()
    }
    pub(super) fn settings_panel(&self) -> Element<'_, Message> {
        let colors = self.colors();
        let flat = move |theme: &Theme, status| colors.get(theme).flat(status, false);
        let input_style = move |theme: &Theme, status| colors.get(theme).input(theme, status);
        let titlebar = match self.config.window.title_bar {
            desktop::TitlebarMode::Auto => "auto",
            desktop::TitlebarMode::Show => "show",
            desktop::TitlebarMode::Hide => "hide",
        };
        let mut themes: Vec<String> = ["auto", "system", "light", "dark", "omarchy"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        themes.extend(self.custom_themes.iter().cloned());
        let choice = |value: &str| Choice {
            value: value.to_owned(),
            label: match value {
                "auto" => "Automatic",
                "system" => "System",
                "light" => "Light",
                "dark" => "Dark",
                "omarchy" => "Omarchy",
                "show" => "Always show",
                "hide" => "Always hide",
                _ => value,
            }
            .to_owned(),
        };
        let pick_style = move |theme: &Theme, status| {
            let colors = colors.get(theme);
            let mut style = pick_list::default(theme, status);
            style.background = iced::Color::TRANSPARENT.into();
            style.border = iced::Border::default();
            style.text_color = colors.foreground;
            style.handle_color = colors.foreground;
            style
        };
        let separator = || {
            container(space::horizontal().height(1))
                .width(Fill)
                .style(move |theme| container::Style {
                    background: Some(colors.get(theme).chrome.scale_alpha(0.45).into()),
                    ..Default::default()
                })
        };
        let theme_choice = |value: &str| {
            let mut result = choice(value);
            result.label = match value {
                "auto" if self.paths.omarchy_available() => "Automatic (Omarchy theme)".into(),
                "auto" => "Automatic (system setting)".into(),
                "system" => "Use system setting".into(),
                "omarchy" => "Follow Omarchy theme".into(),
                "light" | "dark" => result.label,
                _ => format!("Custom: {value}"),
            };
            result
        };
        let appearance = settings_group(
            colors,
            column![
                setting(
                    colors,
                    "App theme",
                    if self.config.appearance.theme == "auto" && self.paths.omarchy_available() {
                        "Following the active Omarchy theme".into()
                    } else {
                        String::new()
                    },
                    pick_list(
                        themes
                            .iter()
                            .map(|value| theme_choice(value))
                            .collect::<Vec<_>>(),
                        Some(theme_choice(&self.config.appearance.theme)),
                        |choice: Choice| Message::Theme(choice.value)
                    )
                    .text_size(UI)
                    .width(220)
                    .padding(4)
                    .style(pick_style)
                    .into()
                ),
                separator(),
                setting(
                    colors,
                    "Window title bar",
                    if desktop::running_on_tiling_compositor() {
                        "Automatic hides it here because a tiling compositor is running".into()
                    } else {
                        "Automatic follows your desktop".into()
                    },
                    pick_list(
                        [choice("auto"), choice("show"), choice("hide")],
                        Some(choice(titlebar)),
                        |choice: Choice| Message::Titlebar(choice.value)
                    )
                    .text_size(UI)
                    .padding(4)
                    .style(pick_style)
                    .into()
                )
            ]
            .into(),
        );
        let round = move |theme: &Theme, status| {
            let mut style = colors.get(theme).flat(status, true);
            style.border.radius = 16.0.into();
            if matches!(status, button::Status::Active) {
                style.background = Some(colors.get(theme).control.into());
            }
            style
        };
        let zoom = row![
            text(format!("{}", self.config.appearance.zoom)).size(UI),
            button(container(icon(Icon::Minus)).center(Fill))
                .width(30)
                .height(30)
                .padding(0)
                .style(round)
                .on_press(Message::ZoomBy(-10)),
            button(container(icon(Icon::Plus)).center(Fill))
                .width(30)
                .height(30)
                .padding(0)
                .style(round)
                .on_press(Message::ZoomBy(10))
        ]
        .spacing(8)
        .align_y(iced::Center);
        let font_label = font_label(&self.config.editor.font, &self.system_monospace);
        let font = row![
            button(text(font_label).size(UI).font(bold_font()))
                .padding([8, 10])
                .style(move |theme, status| {
                    let colors = colors.get(theme);
                    let mut style = colors.flat(status, true);
                    if status == button::Status::Active {
                        style.background = Some(colors.control.into());
                    }
                    style
                })
                .on_press(Message::ChooseFont),
            button(text("System font").size(UI).font(bold_font()))
                .padding([8, 10])
                .style(flat)
                .on_press_maybe(
                    (!self.config.editor.font.is_empty()).then_some(Message::ResetFont)
                )
        ]
        .spacing(12)
        .align_y(iced::Center);
        let toggle_style = |theme: &Theme, status| {
            let mut style = toggler::default(theme, status);
            style.foreground = iced::Color::WHITE.into();
            style
        };
        let text_settings = settings_group(
                colors,
                column![
                    setting(
                        colors,
                        "Zoom",
                        "Percent. Also Ctrl + mouse wheel, Ctrl + plus and Ctrl + minus".into(),
                        zoom.into()
                    ),
                    separator(),
                    setting(
                        colors,
                        "Font",
                        if self.config.editor.font.is_empty() { "Following the system monospace font, with Noto Naskh Arabic for Arabic text.".into() } else { "Paired with Noto Naskh Arabic for Arabic text. List two families in config.toml to pick another, e.g. \"JetBrainsMono Nerd Font, Amiri 12\"".into() },
                        font.into()
                    ),
                    separator(),
                    setting(
                        colors,
                        "Word wrap",
                        "Wrap long lines to the window width".into(),
                        toggler(self.config.editor.word_wrap)
                            .size(24)
                            .style(toggle_style)
                            .on_toggle(Message::Wrap)
                            .into()
                    ),
                    separator(),
                    setting(
                        colors,
                        "Status bar",
                        "Line and column, character count, zoom, line endings and encoding".into(),
                        toggler(self.config.window.status_bar)
                            .size(24)
                            .style(toggle_style)
                            .on_toggle(Message::Status)
                            .into()
                    )
                ]
                .into(),
            );
        let paths = settings_group(
                colors,
                column![
                    setting(
                        colors,
                        "Settings",
                        self.paths.config_file().display().to_string(),
                        space::horizontal().width(0).into()
                    ),
                    separator(),
                    setting(
                        colors,
                        "Custom themes",
                        format!("{}/  — TOML files with mode, background, foreground and optional accent, muted, selection, border, chrome, menu", self.paths.themes_dir().display()),
                        space::horizontal().width(0).into()
                    )
                ]
                .into(),
            );
        let contents = column![
            text("Appearance").size(UI).font(bold_font()),
            appearance,
            space::vertical().height(12),
            text("Text").size(UI).font(bold_font()),
            text_settings,
            space::vertical().height(12),
            text("Configuration files").size(UI).font(bold_font()),
            text("Edits made to these files, and Omarchy theme changes, apply immediately.")
                .size(UI)
                .style(move |theme| text::Style {
                    color: Some(colors.get(theme).foreground.scale_alpha(0.55))
                }),
            paths
        ]
        .spacing(12);
        let header = row![
            space::horizontal().width(30),
            text("Settings")
                .size(UI)
                .font(bold_font())
                .width(Fill)
                .align_x(iced::Center),
            button(container(icon(Icon::Close)).center(Fill))
                .width(30)
                .height(30)
                .padding(0)
                .style(round)
                .on_press(Message::Dismiss)
        ]
        .align_y(iced::Center);
        let panel = container(column![
            container(header).padding([8, 10]),
            scrollable(container(contents).padding(iced::Padding {
                top: 30.0,
                bottom: 24.0,
                left: 48.0,
                right: 48.0
            }))
            .direction(scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(3).scroller_width(3)
            ))
            .height(Fill)
        ])
        .width(640)
        .height(Fill)
        .max_height(786)
        .style(move |theme| {
            let colors = colors.get(theme);
            container::Style {
                background: Some(colors.chrome.into()),
                border: iced::Border {
                    color: colors.border.scale_alpha(0.5),
                    width: 1.0,
                    radius: 14.0.into(),
                },
                ..colors.panel()
            }
        });
        let panel: Element<'_, Message> = if self.font_picker {
            let filter = self.font_filter.to_lowercase();
            let families = self
                .font_families
                .iter()
                .filter(|(_, lower)| lower.contains(&filter))
                .fold(column![].spacing(2), |list, (family, _)| {
                    list.push(
                        button(text(family).size(UI))
                            .width(Fill)
                            .padding([8, 12])
                            .style(flat)
                            .on_press(Message::FontChosen(family.clone())),
                    )
                });
            container(
                column![
                    row![
                        button("Cancel").style(flat).on_press(Message::Dismiss),
                        text("Select Font")
                            .font(bold_font())
                            .width(Fill)
                            .align_x(iced::Center),
                        button("Select").on_press(Message::ApplyFont)
                    ]
                    .align_y(iced::Center),
                    text_input("Search fonts", &self.font_filter)
                        .style(input_style)
                        .padding(10)
                        .on_input(Message::FontFilter),
                    scrollable(families).height(Fill),
                    text_input("Font family and size", &self.font_draft)
                        .style(input_style)
                        .padding(10)
                        .on_input(Message::Font)
                        .on_submit(Message::ApplyFont)
                ]
                .spacing(12),
            )
            .padding(16)
            .width(640)
            .height(Fill)
            .max_height(650)
            .style(move |theme| colors.get(theme).panel())
            .into()
        } else {
            panel.into()
        };
        panel
    }
}

fn setting<'a>(
    colors: crate::style::Resolver<'a>,
    label: &'static str,
    subtitle: String,
    control: Element<'a, Message>,
) -> widget::Container<'a, Message> {
    let mut description = column![text(label).size(UI).line_height(iced::Pixels(18.0))].spacing(0);
    if !subtitle.is_empty() {
        description = description.push(
            text(subtitle)
                .size(CAPTION)
                .line_height(iced::Pixels(15.0))
                .style(move |theme| text::Style {
                    color: Some(colors.get(theme).foreground.scale_alpha(0.55)),
                }),
        );
    }
    container(
        row![description.width(Fill), control]
            .spacing(16)
            .align_y(iced::Center),
    )
    .padding([10, 14])
}
fn settings_group<'a>(
    colors: crate::style::Resolver<'a>,
    contents: Element<'a, Message>,
) -> widget::Container<'a, Message> {
    container(contents).width(Fill).style(move |theme| {
        let colors = colors.get(theme);
        container::Style {
            shadow: iced::Shadow::default(),
            ..colors.panel()
        }
    })
}

/// Pixel size and family for a Pango-style description such as "Fira Code 12";
/// a missing or unparsable size means [`DEFAULT_FONT_SIZE`] points.
fn parse_font(description: &str) -> (&str, f32) {
    let description = description.trim();
    description
        .rsplit_once(' ')
        .and_then(|(family, size)| {
            size.parse::<f32>()
                .ok()
                .map(|size| (family, size * 4.0 / 3.0))
        })
        .unwrap_or((description, DEFAULT_FONT_SIZE as f32 * 4.0 / 3.0))
}

/// The Settings button caption: family and point size that are in effect.
fn font_label(font: &str, system_monospace: &str) -> String {
    if font.is_empty() {
        let family = if system_monospace.is_empty() {
            "Monospace"
        } else {
            system_monospace
        };
        return format!("{family} Regular {DEFAULT_FONT_SIZE}");
    }
    match font.rsplit_once(' ') {
        Some((family, size)) if size.parse::<f32>().is_ok() => format!("{family} Regular {size}"),
        _ => format!("{font} Regular {DEFAULT_FONT_SIZE}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_font_label_and_editor_size_agree() {
        let (family, pixels) = parse_font("");
        assert_eq!(family, "");
        assert_eq!(pixels, DEFAULT_FONT_SIZE as f32 * 4.0 / 3.0);
        assert_eq!(
            font_label("", "JetBrains Mono"),
            format!("JetBrains Mono Regular {DEFAULT_FONT_SIZE}")
        );
        assert_eq!(
            font_label("", ""),
            format!("Monospace Regular {DEFAULT_FONT_SIZE}")
        );
        assert_eq!(parse_font("Fira Code 12"), ("Fira Code", 16.0));
        assert_eq!(font_label("Fira Code 12", "x"), "Fira Code Regular 12");
        assert_eq!(parse_font("Fira Code").1, parse_font("").1);
        assert_eq!(
            font_label("Fira Code", "x"),
            format!("Fira Code Regular {DEFAULT_FONT_SIZE}")
        );
    }
}
