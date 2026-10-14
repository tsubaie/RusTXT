//! Menu bar panels, their entry tables and keyboard shortcuts.

use super::{App, Menu, Message, EDITOR, UI};
use crate::icons::{icon, Icon};
use iced::{
    keyboard::{self, key::Named, Key},
    widget::{self, button, column, container, operation, row, space, text},
    Element, Fill, Task,
};

impl App {
    pub(super) fn update_menus(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::CloseMenu => {
                self.menu = None;
                return operation::focus(EDITOR);
            }
            Message::Submenu(menu) => self.submenu = menu,
            Message::Menu(menu) => {
                self.submenu = None;
                self.menu = if self.menu == Some(menu) {
                    None
                } else {
                    Some(menu)
                };
            }
            _ => unreachable!("not a menu message"),
        }
        Task::none()
    }
    pub(super) fn menu_panel(&self, menu: Menu) -> Element<'_, Message> {
        let colors = self.colors();
        let doc = self.doc();
        let entries = match menu {
            Menu::File => vec![
                ("New tab", "Ctrl+N", Message::New),
                ("Open…", "Ctrl+O", Message::Open),
                ("Recently closed", "", Message::Submenu(Some(Menu::Recent))),
                ("", "", Message::CloseMenu),
                ("Save", "Ctrl+S", Message::Save(false)),
                ("Save as…", "Shift+Ctrl+S", Message::Save(true)),
                ("Save all", "Ctrl+Alt+S", Message::SaveAll),
                ("", "", Message::CloseMenu),
                ("Print…", "Ctrl+P", Message::Print),
                ("", "", Message::CloseMenu),
                ("Close tab", "Ctrl+W", Message::Close(self.active)),
                ("Discard changes and close", "", Message::Discard),
                ("Reopen closed tab", "Shift+Ctrl+T", Message::ReopenLast),
                ("", "", Message::CloseMenu),
                ("Exit", "Shift+Ctrl+W", Message::Quit),
            ],
            Menu::Edit => vec![
                ("Undo", "Ctrl+Z", Message::Undo(false)),
                ("Redo", "Ctrl+Y", Message::Undo(true)),
                ("", "", Message::CloseMenu),
                ("Cut", "Ctrl+X", Message::Copy(true)),
                ("Copy", "Ctrl+C", Message::Copy(false)),
                ("Paste", "Ctrl+V", Message::Paste),
                ("Delete", "Delete", Message::Delete),
                ("", "", Message::CloseMenu),
                ("Find…", "Ctrl+F", Message::Find(false)),
                ("Find next", "F3", Message::FindNext(false)),
                ("Find previous", "Shift+F3", Message::FindNext(true)),
                ("Replace…", "Ctrl+H", Message::Find(true)),
                ("Go to…", "Ctrl+G", Message::GoTo),
                ("", "", Message::CloseMenu),
                ("Select all", "Ctrl+A", Message::SelectAll),
                ("Time/Date", "F5", Message::Date),
            ],
            Menu::View => vec![
                ("Zoom", "", Message::Submenu(Some(Menu::Zoom))),
                ("", "", Message::CloseMenu),
                (
                    "Status bar",
                    "",
                    Message::Status(!self.config.window.status_bar),
                ),
                (
                    "Word wrap",
                    "",
                    Message::Wrap(!self.config.editor.word_wrap),
                ),
            ],
            Menu::Settings => vec![
                ("Settings…", "Ctrl+,", Message::Settings),
                ("", "", Message::CloseMenu),
                ("About RusTXT", "", Message::About),
            ],
            Menu::Zoom => vec![
                ("Zoom in", "Ctrl++", Message::ZoomBy(10)),
                ("Zoom out", "Ctrl+-", Message::ZoomBy(-10)),
                ("Restore default zoom", "Ctrl+0", Message::Zoom(100)),
            ],
            Menu::Recent => self
                .closed
                .iter()
                .map(|closed| {
                    (
                        closed.title.as_str(),
                        "",
                        Message::Reopen(closed.id.clone()),
                    )
                })
                .collect(),
        };
        let mut items = column![];
        for (name, shortcut, message) in entries {
            if name.is_empty() {
                items = items.push(
                    container(container(space::horizontal().height(1)).width(Fill).style(
                        move |theme| container::Style {
                            background: Some(colors.get(theme).foreground.scale_alpha(0.15).into()),
                            ..Default::default()
                        },
                    ))
                    .padding([6, 0]),
                );
                continue;
            }
            let enabled = match &message {
                Message::Undo(false) => doc.can_undo(),
                Message::Undo(true) => doc.can_redo(),
                Message::Copy(_) | Message::Delete => !self.selection.is_empty(),
                Message::ReopenLast | Message::Submenu(Some(Menu::Recent)) => {
                    !self.closed.is_empty()
                }
                _ => true,
            };
            let mut contents = row![].spacing(8).align_y(iced::Center);
            if menu == Menu::View {
                let checked = match message {
                    Message::Status(_) => self.config.window.status_bar,
                    Message::Wrap(_) => self.config.editor.word_wrap,
                    _ => false,
                };
                let check: Element<'_, Message> = if checked {
                    icon(Icon::Check).into()
                } else {
                    space::horizontal().width(16).into()
                };
                contents = contents.push(check);
            }
            contents = contents
                .push(text(name).size(UI).line_height(iced::Pixels(18.0)))
                .push(space::horizontal());
            if matches!(message, Message::Submenu(_)) {
                contents = contents.push(icon(Icon::Right));
            } else if !shortcut.is_empty() {
                contents = contents.push(
                    text(if cfg!(target_os = "macos") {
                        shortcut.replace("Ctrl", "Cmd")
                    } else {
                        shortcut.into()
                    })
                    .size(UI)
                    .line_height(iced::Pixels(18.0))
                    .style(move |theme| text::Style {
                        color: Some(colors.get(theme).foreground.scale_alpha(0.55)),
                    }),
                );
            }
            let submenu = if let Message::Submenu(value) = message {
                value
            } else {
                None
            };
            let item = button(contents)
                .width(Fill)
                .height(32)
                .padding([7, 12])
                .on_press_maybe(enabled.then_some(message))
                .style(move |theme, status| {
                    let mut style = colors.get(theme).flat(status, false);
                    if status == button::Status::Disabled {
                        style.text_color = colors.get(theme).foreground.scale_alpha(0.5);
                    }
                    style.border.radius = 4.0.into();
                    style
                });
            let item: Element<'_, Message> = if matches!(menu, Menu::Zoom | Menu::Recent) {
                item.into()
            } else {
                widget::mouse_area(item)
                    .on_enter(Message::Submenu(if enabled { submenu } else { None }))
                    .into()
            };
            items = items.push(item);
        }
        if menu == Menu::Recent && self.closed.is_empty() {
            items = items.push(text("No recently closed tabs").size(UI));
        }
        container(items)
            .padding(6)
            .width(match menu {
                Menu::File => 264,
                Menu::Edit => 204,
                Menu::View => 136,
                Menu::Settings => 212,
                _ => 300,
            })
            .style(move |theme| {
                let colors = colors.get(theme);
                container::Style {
                    border: iced::Border {
                        color: colors.foreground.scale_alpha(0.2),
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..colors.panel()
                }
            })
            .into()
    }
}

/// Editor-level bindings that Iced does not provide by default: Tab inserts a
/// tab character instead of moving keyboard focus, as in any plain text editor.
pub(super) fn editor_binding(
    key: Key<&str>,
    modifiers: keyboard::Modifiers,
) -> Option<iced::widget::text_editor::Binding<Message>> {
    match key {
        Key::Named(Named::Tab) if !modifiers.command() && !modifiers.alt() => {
            Some(iced::widget::text_editor::Binding::Insert('\t'))
        }
        _ => None,
    }
}

pub(super) fn shortcut(key: Key<&str>, modifiers: keyboard::Modifiers) -> Option<Message> {
    if modifiers.command() {
        if let Key::Character(character) = key {
            return match character.to_lowercase().as_str() {
                "n" => Some(Message::New),
                "t" => Some(if modifiers.shift() {
                    Message::ReopenLast
                } else {
                    Message::New
                }),
                "o" => Some(Message::Open),
                "s" => Some(if modifiers.alt() {
                    Message::SaveAll
                } else {
                    Message::Save(modifiers.shift())
                }),
                "w" => Some(if modifiers.shift() {
                    Message::Quit
                } else {
                    Message::CloseActive
                }),
                "q" => Some(Message::Quit),
                "f" => Some(Message::Find(false)),
                "h" => Some(Message::Find(true)),
                "g" => Some(Message::GoTo),
                "z" => Some(Message::Undo(modifiers.shift())),
                "y" => Some(Message::Undo(true)),
                "p" => Some(Message::Print),
                "," => Some(Message::Settings),
                "+" | "=" => Some(Message::ZoomBy(10)),
                "-" => Some(Message::ZoomBy(-10)),
                "0" => Some(Message::Zoom(100)),
                _ => None,
            };
        }
    }
    match key {
        Key::Named(Named::F3) => Some(Message::FindNext(modifiers.shift())),
        Key::Named(Named::F5) => Some(Message::Date),
        Key::Named(Named::F10) => Some(Message::Menu(Menu::File)),
        Key::Named(Named::Tab) if modifiers.control() => Some(Message::NextTab(modifiers.shift())),
        Key::Character("f") if modifiers.alt() => Some(Message::Menu(Menu::File)),
        Key::Character("e") if modifiers.alt() => Some(Message::Menu(Menu::Edit)),
        Key::Character("v") if modifiers.alt() => Some(Message::Menu(Menu::View)),
        Key::Character("s") if modifiers.alt() => Some(Message::Menu(Menu::Settings)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_inserts_a_tab_character() {
        use iced::widget::text_editor::Binding;
        assert!(matches!(
            editor_binding(Key::Named(Named::Tab), keyboard::Modifiers::empty()),
            Some(Binding::Insert('\t'))
        ));
        assert!(matches!(
            editor_binding(Key::Named(Named::Tab), keyboard::Modifiers::SHIFT),
            Some(Binding::Insert('\t'))
        ));
        assert!(editor_binding(Key::Named(Named::Tab), keyboard::Modifiers::COMMAND).is_none());
    }

    #[test]
    fn shortcuts_map_quit_and_find_next() {
        assert!(matches!(
            shortcut(Key::Character("q"), keyboard::Modifiers::COMMAND),
            Some(Message::Quit)
        ));
        assert!(matches!(
            shortcut(Key::Named(Named::F3), keyboard::Modifiers::SHIFT),
            Some(Message::FindNext(true))
        ));
        assert!(shortcut(Key::Character("q"), keyboard::Modifiers::empty()).is_none());
    }
}
