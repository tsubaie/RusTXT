//! One open tab: a GtkSourceView buffer and view plus the persisted state.

use gtk::prelude::*;
use gtk::{gio, glib};
use rustpad_core::storage::DocumentState;
use sourceview5::prelude::*;
use std::{cell::RefCell, rc::Rc, time::Duration};

pub struct Document {
    pub state: RefCell<DocumentState>,
    pub buffer: sourceview5::Buffer,
    pub view: sourceview5::View,
    pub scroller: gtk::ScrolledWindow,
    pub page: adw::TabPage,
    pub search: sourceview5::SearchContext,
}

pub fn wrap_mode(wrap: bool) -> gtk::WrapMode {
    if wrap {
        gtk::WrapMode::WordChar
    } else {
        gtk::WrapMode::None
    }
}

impl Document {
    pub fn new(
        tab_view: &adw::TabView,
        state: DocumentState,
        search_settings: &sourceview5::SearchSettings,
        scheme: Option<&sourceview5::StyleScheme>,
        wrap: bool,
    ) -> Rc<Self> {
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_highlight_syntax(false);
        buffer.set_highlight_matching_brackets(false);
        buffer.set_style_scheme(scheme);
        // Loading the text must not become the first undo step.
        buffer.begin_irreversible_action();
        buffer.set_text(&state.content);
        buffer.end_irreversible_action();
        let offset = (state.cursor_offset as i32).clamp(0, buffer.char_count());
        buffer.place_cursor(&buffer.iter_at_offset(offset));
        buffer.set_modified(state.dirty);

        let view = sourceview5::View::with_buffer(&buffer);
        view.set_monospace(true);
        view.set_wrap_mode(wrap_mode(wrap));
        view.set_left_margin(12);
        view.set_right_margin(12);
        view.set_top_margin(8);
        view.set_bottom_margin(24);
        view.set_highlight_current_line(false);
        view.set_show_line_numbers(false);
        view.set_auto_indent(false);
        view.set_tab_width(4);
        view.set_smart_home_end(sourceview5::SmartHomeEndType::Before);
        view.add_css_class("rustpad-editor");

        let scroller = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();

        let search = sourceview5::SearchContext::new(&buffer, Some(search_settings));
        search.set_highlight(false);

        let page = tab_view.append(&scroller);
        let document = Rc::new(Self {
            state: RefCell::new(state),
            buffer,
            view,
            scroller,
            page,
            search,
        });
        document.refresh_tab();
        document
    }

    pub fn id(&self) -> String {
        self.state.borrow().id.clone()
    }

    pub fn title(&self) -> String {
        self.state.borrow().title.clone()
    }

    pub fn file_path(&self) -> Option<String> {
        self.state.borrow().file_path.clone()
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_modified()
    }

    pub fn text(&self) -> String {
        self.buffer
            .text(&self.buffer.start_iter(), &self.buffer.end_iter(), true)
            .to_string()
    }

    /// Full state for the recovery database.
    pub fn snapshot(&self) -> DocumentState {
        let mut state = self.state.borrow().clone();
        state.content = self.text();
        state.dirty = self.is_dirty();
        state.cursor_offset = self.buffer.cursor_position() as i64;
        state.scroll_top = self.scroller.vadjustment().value();
        state
    }

    pub fn view_state(&self) -> (i64, f64) {
        (
            self.buffer.cursor_position() as i64,
            self.scroller.vadjustment().value(),
        )
    }

    pub fn window_title(&self) -> String {
        format!(
            "{}{} - RustPad",
            if self.is_dirty() { "*" } else { "" },
            self.title()
        )
    }

    /// Tab label, tooltip and the unsaved indicator dot.
    pub fn refresh_tab(&self) {
        let state = self.state.borrow();
        self.page.set_title(&state.title);
        self.page
            .set_tooltip(state.file_path.as_deref().unwrap_or(&state.title));
        let icon: Option<gio::Icon> = self
            .is_dirty()
            .then(|| gio::ThemedIcon::new("media-record-symbolic").upcast());
        self.page.set_indicator_icon(icon.as_ref());
        self.page.set_indicator_activatable(false);
    }

    /// Scroll positions can only be applied once the view has been laid out.
    pub fn restore_scroll(&self) {
        let target = self.state.borrow().scroll_top;
        if target <= 0.0 {
            return;
        }
        let adjustment = self.scroller.vadjustment();
        glib::timeout_add_local_once(Duration::from_millis(60), move || {
            adjustment.set_value(target)
        });
    }

    pub fn set_wrap(&self, wrap: bool) {
        self.view.set_wrap_mode(wrap_mode(wrap));
    }

    /// Record a successful save (or load) as the clean baseline.
    pub fn mark_saved(&self, state: DocumentState) {
        *self.state.borrow_mut() = state;
        self.buffer.set_modified(false);
        self.refresh_tab();
    }

    pub fn insert_at_cursor(&self, text: &str) {
        self.buffer.begin_user_action();
        self.buffer.delete_selection(true, true);
        self.buffer.insert_at_cursor(text);
        self.buffer.end_user_action();
        self.view.scroll_mark_onscreen(&self.buffer.get_insert());
    }

    pub fn go_to_line(&self, line: i32) {
        let line = line.clamp(1, self.buffer.line_count());
        if let Some(iter) = self.buffer.iter_at_line(line - 1) {
            self.buffer.place_cursor(&iter);
            self.view
                .scroll_to_mark(&self.buffer.get_insert(), 0.1, true, 0.0, 0.5);
        }
        self.view.grab_focus();
    }
}
