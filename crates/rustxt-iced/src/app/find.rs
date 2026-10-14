//! Find and replace: the search handlers and the find bar.

use super::{view::hint, App, Message, CAPTION, EDITOR, UI};
use crate::{
    icons::{icon, Icon},
    platform::bold_font,
};
use iced::{
    widget::{self, button, column, container, operation, row, space, text, text_input},
    Element, Fill, Task, Theme,
};

const REPLACE: &str = "replace";
const FIND: &str = "find";

impl App {
    /// Rebuild matches now if the find bar is open; otherwise just note that
    /// they are stale so the next Find Next rebuilds them first.
    pub(super) fn refresh_search(&mut self) {
        if self.find_open {
            self.rebuild_search();
        } else {
            self.search_stale = true;
        }
    }
    fn rebuild_search(&mut self) {
        self.search_stale = false;
        if let Some(doc) = self.docs.get(self.active) {
            self.search.refresh(&doc.state.content);
        }
    }
    pub(super) fn update_find(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleReplace => {
                self.replace_open = !self.replace_open;
            }
            Message::Find(replace) => {
                self.menu = None;
                let selection = self.doc().selected_range();
                let selected = &self.doc().state.content[selection];
                if !selected.is_empty() && !selected.contains('\n') {
                    self.search.query = selected.to_owned();
                }
                self.find_open = true;
                self.replace_open |= replace;
                self.rebuild_search();
                return operation::focus(if replace { REPLACE } else { FIND });
            }
            Message::Query(query) => {
                self.search.query = query;
                self.refresh_search();
            }
            Message::Replacement(value) => self.search.replacement = value,
            Message::MatchCase(value) => {
                self.search.case_sensitive = value;
                self.refresh_search();
            }
            Message::WholeWord(value) => {
                self.search.whole_word = value;
                self.refresh_search();
            }
            Message::Regex(value) => {
                self.search.regex = value;
                self.refresh_search();
            }
            Message::FindNext(backwards) => {
                self.menu = None;
                self.submenu = None;
                if self.search_stale {
                    self.rebuild_search();
                }
                if let Some(range) = self.search.next(self.doc().selected_range(), backwards) {
                    self.doc_mut().select(range);
                }
                return operation::focus(EDITOR);
            }
            Message::Replace => {
                if self.search_stale {
                    self.rebuild_search();
                }
                let range = self.doc().selected_range();
                self.search.locate(&range);
                if self.search.current.is_some() {
                    let doc = &self.docs[self.active];
                    let replacement = self
                        .search
                        .replacement_for(&doc.state.content, range.clone());
                    self.doc_mut().replace(range, &replacement);
                    self.refresh_search();
                }
                return self.update(Message::FindNext(false));
            }
            Message::ReplaceAll => {
                if !self.search.query.is_empty() {
                    let doc = &self.docs[self.active];
                    match self.search.replace_all(&doc.state.content) {
                        Ok(next) => {
                            let len = self.doc().state.content.len();
                            self.doc_mut().replace(0..len, &next);
                            self.refresh_search();
                        }
                        Err(error) => self.report(Err(error)),
                    }
                }
            }
            _ => unreachable!("not a find message"),
        }
        Task::none()
    }
    pub(super) fn find_bar(&self) -> Element<'_, Message> {
        let colors = self.colors();
        let flat = move |theme: &Theme, status| colors.get(theme).flat(status, false);
        let input_style = move |theme: &Theme, status| colors.get(theme).input(theme, status);
        let count = self.search.count_label();
        let count_width = count.chars().count() as f32 * CAPTION * 0.65;
        let option = |label, value, message| {
            let description = match label {
                "Aa" => "Match case: distinguish uppercase and lowercase",
                "ab" => "Match whole words only",
                _ => "Use regular expressions",
            };
            hint(
                button(
                    text(label)
                        .size(CAPTION)
                        .font(bold_font())
                        .width(Fill)
                        .height(Fill)
                        .center(),
                )
                .width(50)
                .height(34)
                .padding(0)
                .on_press(message)
                .style(move |theme, status| colors.get(theme).flat(status, value)),
                description,
                colors,
            )
        };
        let tool = |kind, message| {
            let description = match &message {
                Message::ToggleReplace if self.replace_open => "Hide replacement controls",
                Message::ToggleReplace => "Show replacement controls (Ctrl+H)",
                Message::FindNext(true) => "Find previous match (Shift+F3)",
                Message::FindNext(false) => "Find next match (F3)",
                _ => "Close Find and Replace (Esc)",
            };
            hint(
                button(container(icon(kind)).center(Fill))
                    .width(34)
                    .height(34)
                    .padding(0)
                    .style(flat)
                    .on_press(message),
                description,
                colors,
            )
        };
        let field = text_input("Find", &self.search.query)
            .id(FIND)
            .on_input(Message::Query)
            .on_submit(Message::FindNext(false))
            .size(UI)
            .padding(iced::Padding {
                left: 30.0,
                right: 28.0,
                top: 6.0,
                bottom: 6.0,
            })
            .style(input_style);
        let mut field = widget::stack![
            field,
            container(icon(Icon::Search)).padding([0, 8]).center_y(Fill)
        ];
        if !self.search.query.is_empty() {
            field = field.push(
                container(hint(
                    button(container(icon(Icon::Close)).center(Fill))
                        .width(24)
                        .height(28)
                        .padding(0)
                        .style(flat)
                        .on_press(Message::Query(String::new())),
                    "Clear search",
                    colors,
                ))
                .align_right(Fill)
                .center_y(Fill)
                .padding([0, 4]),
            );
        }
        let field = container(field).width(212).height(34);
        let find_row = row![
            tool(
                if self.replace_open {
                    Icon::Down
                } else {
                    Icon::Right
                },
                Message::ToggleReplace
            ),
            field,
            container(text(count).size(CAPTION))
                .width(count_width + 8.0)
                .padding([0, 4]),
            tool(Icon::Up, Message::FindNext(true)),
            tool(Icon::Down, Message::FindNext(false)),
            option(
                "Aa",
                self.search.case_sensitive,
                Message::MatchCase(!self.search.case_sensitive)
            ),
            option(
                "ab",
                self.search.whole_word,
                Message::WholeWord(!self.search.whole_word)
            ),
            option(".*", self.search.regex, Message::Regex(!self.search.regex)),
            tool(Icon::Close, Message::Dismiss)
        ]
        .spacing(2)
        .align_y(iced::Center);
        let mut find = column![find_row].spacing(6);
        if self.replace_open {
            let raised = move |theme: &Theme, status| {
                let colors = colors.get(theme);
                let mut style = colors.flat(status, true);
                if status == button::Status::Active {
                    style.background = Some(colors.control.into());
                }
                style
            };
            find = find.push(
                row![
                    space::horizontal().width(30),
                    text_input("Replace with", &self.search.replacement)
                        .id(REPLACE)
                        .on_input(Message::Replacement)
                        .on_submit(Message::Replace)
                        .size(UI)
                        .padding([6, 8])
                        .style(input_style),
                    hint(
                        button(text("Replace").font(bold_font()))
                            .height(34)
                            .padding([6, 16])
                            .style(raised)
                            .on_press(Message::Replace),
                        "Replace current match and find the next",
                        colors
                    ),
                    hint(
                        button(text("Replace all").font(bold_font()))
                            .height(34)
                            .padding([6, 16])
                            .style(raised)
                            .on_press(Message::ReplaceAll),
                        "Replace all matches in this document",
                        colors
                    )
                ]
                .spacing(4)
                .align_y(iced::Center),
            );
        } else {
            find = find.push(space::vertical().height(0));
        }
        let card = container(find)
            .width(534.0 + count_width)
            .padding(6)
            .style(move |theme| colors.get(theme).panel());
        card.into()
    }
}
