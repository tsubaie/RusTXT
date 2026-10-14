//! The top-level view: tab bar, menu bar, editor, status bar and dialogs.

use super::{menus::shortcut, App, Menu, Message, CAPTION, EDITOR, UI};
use crate::{
    document,
    icons::{icon, Icon},
};
use iced::{
    widget::{
        self, button, column, container, operation, row, scrollable, space, text, text_editor,
        text_input,
    },
    Element, Fill, Task, Theme,
};
use rustxt_core::storage::LineEnding;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");

impl App {
    pub(super) fn update_dialogs(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Dismiss => {
                if self.font_picker {
                    self.font_picker = false;
                    self.font_draft = self.config.editor.font.clone();
                    return Task::none();
                }
                self.menu = None;
                if self.settings_open && self.font_draft != self.config.editor.font {
                    self.config.editor.font = self.font_draft.clone();
                    let _ = self.save_settings();
                }
                self.settings_open = false;
                self.about_open = false;
                self.go_open = false;
                self.discard_id = None;
                self.error = None;
                self.quit_pending = false;
                self.find_open = false;
                return operation::focus(EDITOR);
            }
            Message::About => {
                self.menu = None;
                self.about_open = true;
            }
            Message::GoTo => {
                self.menu = None;
                self.go_open = true;
                self.go_line.clear();
                return operation::focus("go-line");
            }
            Message::GoLine(value) => self.go_line = value,
            Message::Go => {
                if let Ok(line) = self.go_line.parse::<usize>() {
                    let line = line
                        .saturating_sub(1)
                        .min(self.doc().content.line_count().saturating_sub(1));
                    let offset = document::byte_offset(
                        &self.doc().state.content,
                        text_editor::Position { line, column: 0 },
                    );
                    self.doc_mut().select(offset..offset);
                    self.go_open = false;
                    return operation::focus(EDITOR);
                }
            }
            Message::OpenLink(url) => {
                let result = webbrowser::open(&url).map_err(|e| e.to_string());
                self.report(result);
            }
            _ => unreachable!("not a dialog message"),
        }
        Task::none()
    }
    pub(super) fn view(&self) -> Element<'_, Message> {
        if self.docs.is_empty() {
            return container(text("Closing…")).into();
        }
        let colors = self.colors();
        let flat = move |theme: &Theme, status| colors.get(theme).flat(status, false);
        let menus = [
            ("File", Menu::File),
            ("Edit", Menu::Edit),
            ("View", Menu::View),
            ("Settings", Menu::Settings),
        ]
        .into_iter()
        .fold(row![].spacing(0), |menus, (label, menu)| {
            let selected = self.menu == Some(menu);
            menus.push(
                button(text(label).size(UI))
                    .padding([5, 10])
                    .on_press(Message::Menu(menu))
                    .style(move |theme, status| colors.get(theme).flat(status, selected)),
            )
        });
        let menus = container(menus)
            .padding([0, 4])
            .width(Fill)
            .height(31)
            .style(move |theme| colors.get(theme).bar());
        let tabs = self
            .docs
            .iter()
            .enumerate()
            .fold(row![].spacing(4), |tabs, (index, doc)| {
                let selected = index == self.active;
                let dot = container(space::horizontal().width(14).height(14)).style(move |theme| {
                    container::Style {
                        background: doc.state.dirty.then(|| colors.get(theme).foreground.into()),
                        border: iced::Border {
                            radius: 7.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                });
                let title: String = doc.state.title.chars().take(23).collect();
                let title = if title.len() < doc.state.title.len() {
                    format!("{title}…")
                } else {
                    title
                };
                let base = button(
                    row![
                        dot,
                        text(title)
                            .size(UI)
                            .width(Fill)
                            .align_x(iced::Center)
                            .wrapping(text::Wrapping::None),
                        space::horizontal().width(22)
                    ]
                    .height(Fill)
                    .align_y(iced::Center)
                    .spacing(8),
                )
                .padding([0, 10])
                .width(216)
                .height(34)
                .on_press(Message::Select(index))
                .style(move |theme, status| colors.get(theme).flat(status, selected));
                let close = container(
                    button(container(icon(Icon::Close)).center(Fill))
                        .width(24)
                        .height(24)
                        .padding(0)
                        .style(flat)
                        .on_press(Message::Close(index)),
                )
                .align_right(Fill)
                .center_y(Fill)
                .padding([0, 6]);
                let tab: Element<'_, Message> = if selected {
                    widget::stack![base, close].into()
                } else {
                    widget::hover(base, close)
                };
                tabs.push(hint(
                    widget::mouse_area(tab).on_middle_press(Message::Close(index)),
                    &doc.state.title,
                    colors,
                ))
            });
        let tabs = container(
            row![
                scrollable(tabs)
                    .direction(scrollable::Direction::Horizontal(
                        scrollable::Scrollbar::new().width(2).scroller_width(2)
                    ))
                    .width(Fill)
                    .height(34),
                button(container(icon(Icon::Plus)).center(Fill))
                    .width(36)
                    .height(34)
                    .padding(0)
                    .style(flat)
                    .on_press(Message::New)
            ]
            .align_y(iced::Center)
            .spacing(8),
        )
        .padding(iced::Padding {
            top: 7.0,
            bottom: 6.0,
            left: 6.0,
            right: 6.0,
        })
        .height(47)
        .width(Fill)
        .style(move |theme| colors.get(theme).bar());
        let doc = self.doc();
        let editor = text_editor(&doc.content)
            .id(EDITOR)
            .height(Fill)
            .padding(iced::Padding {
                top: 8.0,
                right: 12.0,
                bottom: 24.0,
                left: 12.0,
            })
            .font(self.editor_font)
            .size(self.editor_size * self.config.appearance.zoom as f32 / 100.0)
            .wrapping(if self.config.editor.word_wrap {
                text::Wrapping::WordOrGlyph
            } else {
                text::Wrapping::None
            })
            .on_action(Message::Action)
            .style(move |theme, status| {
                let colors = colors.get(theme);
                let mut style = text_editor::default(theme, status);
                style.border = iced::Border::default();
                style.background = colors.background.into();
                style.value = colors.foreground;
                style.selection = colors.selection;
                style
            })
            .key_binding(|press| {
                shortcut(press.key.as_ref(), press.modifiers)
                    .map(text_editor::Binding::Custom)
                    .or_else(|| text_editor::Binding::from_key_press(press))
            });
        let editor: Element<'_, Message> = if self.find_open {
            widget::stack![
                editor,
                container(widget::opaque(self.find_bar()))
                    .align_right(Fill)
                    .padding([8, 18])
            ]
            .into()
        } else {
            editor.into()
        };
        let separator = || {
            container(space::horizontal().height(1))
                .width(Fill)
                .style(move |theme| container::Style {
                    background: Some(colors.get(theme).border.scale_alpha(0.5).into()),
                    ..Default::default()
                })
        };
        let mut body = column![tabs, separator(), menus, editor];
        if self.config.window.status_bar {
            let cursor = doc.content.cursor().position;
            let column = doc
                .content
                .line(cursor.line)
                .map(|l| l.text[..cursor.column.min(l.text.len())].chars().count())
                .unwrap_or(0);
            let divider = || {
                container(space::horizontal().width(1).height(10)).style(move |theme| {
                    container::Style {
                        background: Some(colors.get(theme).border.into()),
                        ..Default::default()
                    }
                })
            };
            let ending = match doc.state.line_ending {
                LineEnding::Lf => "Unix (LF)",
                LineEnding::Crlf => "Windows (CRLF)",
            };
            body = body.push(separator()).push(
                container(
                    row![
                        text(format!("Ln {}, Col {}", cursor.line + 1, column + 1)).size(CAPTION),
                        space::horizontal(),
                        divider(),
                        text(format!("{} characters", doc.char_count())).size(CAPTION),
                        divider(),
                        text(format!("{}%", self.config.appearance.zoom)).size(CAPTION),
                        divider(),
                        text(ending).size(CAPTION),
                        divider(),
                        text("UTF-8").size(CAPTION)
                    ]
                    .spacing(10)
                    .align_y(iced::Center),
                )
                .padding([3, 12])
                .width(Fill)
                .style(move |theme| {
                    let colors = colors.get(theme);
                    container::Style {
                        text_color: Some(colors.foreground.scale_alpha(0.75)),
                        ..colors.bar()
                    }
                }),
            );
        }
        let base: Element<'_, Message> = container(body).height(Fill).width(Fill).into();
        if let Some(error) = &self.error {
            let mut actions = row![button("Close").on_press(Message::Dismiss)].spacing(12);
            if self.quit_pending {
                actions = actions.push(
                    button("Exit anyway")
                        .on_press(Message::Quit)
                        .style(button::danger),
                );
            }
            return modal(
                base,
                colors,
                column![
                    text("Could not complete the action").size(20),
                    text(error),
                    actions
                ]
                .spacing(16),
            );
        }
        if let Some(id) = &self.discard_id {
            return modal(
                base,
                colors,
                column![
                    text("Discard this tab permanently?").size(20),
                    text("Its recovery copy will be deleted. This cannot be undone."),
                    row![
                        button("Cancel").on_press(Message::Dismiss),
                        button("Discard")
                            .on_press(Message::ConfirmDiscard(id.clone()))
                            .style(button::danger)
                    ]
                    .spacing(12)
                ]
                .spacing(16),
            );
        }
        if self.go_open {
            return modal(
                base,
                colors,
                column![
                    text("Go to line").size(20),
                    text_input("Line number", &self.go_line)
                        .id("go-line")
                        .on_input(Message::GoLine)
                        .on_submit(Message::Go),
                    row![
                        button("Cancel").on_press(Message::Dismiss),
                        button("Go").on_press(Message::Go)
                    ]
                    .spacing(12)
                ]
                .spacing(16),
            );
        }
        if self.settings_open {
            return widget::stack![
                base,
                widget::opaque(
                    container(self.settings_panel())
                        .padding(24)
                        .center(Fill)
                        .style(|_| {
                            container::Style {
                                background: Some(iced::Color::BLACK.scale_alpha(0.4).into()),
                                ..Default::default()
                            }
                        })
                )
            ]
            .into();
        }

        if self.about_open {
            return modal(
                base,
                colors,
                column![
                    text("RusTXT").size(30),
                    text(format!("Version {VERSION}")),
                    text("A dead simple, recoverable plain text editor."),
                    text("Built with Rust and Iced. MIT license."),
                    button("View latest release").on_press(Message::OpenLink(format!(
                        "{}/releases/latest",
                        rustxt_core::update::REPOSITORY
                    ))),
                    button("Close").on_press(Message::Dismiss)
                ]
                .spacing(16),
            );
        }
        if let Some(menu) = self.menu {
            let left = match menu {
                Menu::File => 4.0,
                Menu::Edit => 50.0,
                Menu::View => 97.0,
                _ => 151.0,
            };
            let mut layers = widget::stack![
                base,
                widget::opaque(
                    widget::mouse_area(container(space::vertical()).width(Fill).height(Fill))
                        .on_press(Message::CloseMenu)
                ),
                container(widget::opaque(self.menu_panel(menu))).padding(iced::Padding {
                    top: 79.0,
                    left,
                    right: 0.0,
                    bottom: 0.0
                })
            ];
            if let Some(submenu) = self.submenu {
                let (x, y) = match submenu {
                    Menu::Recent => (left + 262.0, 149.0),
                    _ => (left + 134.0, 79.0),
                };
                layers = layers.push(container(widget::opaque(self.menu_panel(submenu))).padding(
                    iced::Padding {
                        top: y,
                        left: x,
                        right: 0.0,
                        bottom: 0.0,
                    },
                ));
            }
            return layers.into();
        }

        base
    }
}

pub(super) fn hint<'a>(
    control: impl Into<Element<'a, Message>>,
    description: &'a str,
    colors: crate::style::Resolver<'a>,
) -> Element<'a, Message> {
    widget::tooltip(
        control,
        text(description).size(UI),
        widget::tooltip::Position::Bottom,
    )
    .delay(Duration::from_millis(450))
    .gap(6)
    .padding(8)
    .style(move |theme| colors.get(theme).tooltip())
    .into()
}

fn modal<'a>(
    base: Element<'a, Message>,
    colors: crate::style::Resolver<'a>,
    contents: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    widget::stack![
        base,
        widget::opaque(
            container(
                container(contents)
                    .padding(24)
                    .width(520)
                    .style(move |theme| colors.get(theme).panel())
            )
            .center(Fill)
            .style(|_| container::Style {
                background: Some(iced::Color::BLACK.scale_alpha(0.4).into()),
                ..Default::default()
            })
        )
    ]
    .into()
}
