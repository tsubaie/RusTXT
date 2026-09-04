//! Find and replace as a small floating tool window over the text:
//! draggable, with a match counter and an optional
//! replace row. Search itself is GtkSourceView's search context.

use gtk::prelude::*;
use gtk::{gdk, glib};
use sourceview5::prelude::*;
use std::{cell::RefCell, rc::Rc};

pub struct FindBar {
    pub root: gtk::Box,
    pub settings: sourceview5::SearchSettings,
    entry: gtk::SearchEntry,
    replace_entry: gtk::Entry,
    count: gtk::Label,
    revealer: gtk::Revealer,
    expander: gtk::ToggleButton,
    case_toggle: gtk::ToggleButton,
    word_toggle: gtk::ToggleButton,
    regexp_toggle: gtk::ToggleButton,
    attached: RefCell<Option<Attached>>,
}

struct Attached {
    context: sourceview5::SearchContext,
    view: sourceview5::View,
    count_handler: glib::SignalHandlerId,
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button.set_focus_on_click(false);
    button
}

fn option_toggle(label: &str, tooltip: &str) -> gtk::ToggleButton {
    let toggle = gtk::ToggleButton::with_label(label);
    toggle.add_css_class("flat");
    toggle.add_css_class("option");
    toggle.set_tooltip_text(Some(tooltip));
    toggle.set_focus_on_click(false);
    toggle
}

impl FindBar {
    pub fn new(settings: sourceview5::SearchSettings) -> Rc<Self> {
        settings.set_wrap_around(true);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.add_css_class("card");
        root.add_css_class("find-card");
        root.set_halign(gtk::Align::End);
        root.set_valign(gtk::Align::Start);
        root.set_margin_top(8);
        root.set_margin_end(18);
        root.set_visible(false);

        let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let expander = gtk::ToggleButton::new();
        expander.set_icon_name("pan-end-symbolic");
        expander.add_css_class("flat");
        expander.set_tooltip_text(Some("Toggle replace (Ctrl+H)"));
        expander.set_focus_on_click(false);
        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some("Find"));
        entry.set_hexpand(true);
        let count = gtk::Label::new(None);
        count.add_css_class("find-count");
        let previous = icon_button("go-up-symbolic", "Find previous (Shift+F3)");
        let next = icon_button("go-down-symbolic", "Find next (F3)");
        let case_toggle = option_toggle("Aa", "Match case");
        let word_toggle = option_toggle("ab", "Match whole word");
        let regexp_toggle = option_toggle(".*", "Use regular expression");
        let close = icon_button("window-close-symbolic", "Close (Esc)");
        for widget in [
            expander.upcast_ref::<gtk::Widget>(),
            entry.upcast_ref(),
            count.upcast_ref(),
            previous.upcast_ref(),
            next.upcast_ref(),
            case_toggle.upcast_ref(),
            word_toggle.upcast_ref(),
            regexp_toggle.upcast_ref(),
            close.upcast_ref(),
        ] {
            find_row.append(widget);
        }

        let replace_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        replace_row.set_margin_start(34);
        let replace_entry = gtk::Entry::new();
        replace_entry.set_placeholder_text(Some("Replace with"));
        replace_entry.set_hexpand(true);
        let replace_one = gtk::Button::with_label("Replace");
        let replace_every = gtk::Button::with_label("Replace all");
        replace_row.append(&replace_entry);
        replace_row.append(&replace_one);
        replace_row.append(&replace_every);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_child(Some(&replace_row));

        root.append(&find_row);
        root.append(&revealer);

        let bar = Rc::new(Self {
            root,
            settings,
            entry,
            replace_entry,
            count,
            revealer,
            expander,
            case_toggle,
            word_toggle,
            regexp_toggle,
            attached: RefCell::new(None),
        });

        let this = bar.clone();
        bar.entry.connect_search_changed(move |_| this.commit());
        let this = bar.clone();
        bar.entry.connect_stop_search(move |_| this.close());
        let this = bar.clone();
        bar.replace_entry
            .connect_activate(move |_| this.replace_current());
        let this = bar.clone();
        bar.expander
            .connect_toggled(move |toggle| this.show_replace(toggle.is_active()));
        for toggle in [&bar.case_toggle, &bar.word_toggle, &bar.regexp_toggle] {
            let this = bar.clone();
            toggle.connect_toggled(move |_| this.commit());
        }
        let this = bar.clone();
        previous.connect_clicked(move |_| this.find_previous());
        let this = bar.clone();
        next.connect_clicked(move |_| this.find_next());
        let this = bar.clone();
        replace_one.connect_clicked(move |_| this.replace_current());
        let this = bar.clone();
        replace_every.connect_clicked(move |_| this.replace_all());
        let this = bar.clone();
        close.connect_clicked(move |_| this.close());

        // Enter / Shift+Enter / Escape anywhere in the bar.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = bar.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let in_replace = this.replace_entry.has_focus();
            match key {
                gdk::Key::Escape => {
                    this.close();
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => {
                    if in_replace {
                        this.replace_current();
                    } else if state.contains(gdk::ModifierType::SHIFT_MASK) {
                        this.find_previous();
                    } else {
                        this.find_next();
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        bar.root.add_controller(keys);

        bar.install_drag();
        bar
    }

    /// Behave like a tool window: drag it by any empty spot.
    fn install_drag(self: &Rc<Self>) {
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let origin: Rc<RefCell<(f64, f64)>> = Rc::new(RefCell::new((0.0, 0.0)));
        let root = self.root.clone();
        let start = origin.clone();
        drag.connect_drag_begin(move |gesture, x, y| {
            let over_control = root.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|w| {
                w.is::<gtk::Text>() || w.is::<gtk::Button>() || w.is::<gtk::Entry>()
            });
            if over_control {
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            let Some(parent) = root.parent() else { return };
            let Some(bounds) = root.compute_bounds(&parent) else {
                return;
            };
            *start.borrow_mut() = (bounds.x() as f64, bounds.y() as f64);
            root.set_halign(gtk::Align::Start);
            root.set_margin_start(bounds.x() as i32);
            root.set_margin_end(0);
            root.set_margin_top(bounds.y() as i32);
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        let root = self.root.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(parent) = root.parent() else { return };
            let (x0, y0) = *origin.borrow();
            let max_x = (parent.width() - root.width()).max(0) as f64;
            let max_y = (parent.height() - root.height()).max(0) as f64;
            root.set_margin_start((x0 + dx).clamp(0.0, max_x) as i32);
            root.set_margin_top((y0 + dy).clamp(0.0, max_y) as i32);
        });
        self.root.add_controller(drag);
    }

    pub fn is_open(&self) -> bool {
        self.root.is_visible()
    }

    /// Point the bar at the active document's search context.
    pub fn attach(&self, context: Option<(&sourceview5::SearchContext, &sourceview5::View)>) {
        if let Some(previous) = self.attached.borrow_mut().take() {
            previous.context.set_highlight(false);
            previous.context.disconnect(previous.count_handler);
        }
        if let Some((context, view)) = context {
            context.set_highlight(self.is_open());
            let count = self.count.clone();
            let settings = self.settings.clone();
            let buffer = view.buffer();
            let count_handler = context.connect_occurrences_count_notify(move |context| {
                update_count_label(&count, context, &settings, &buffer);
            });
            *self.attached.borrow_mut() = Some(Attached {
                context: context.clone(),
                view: view.clone(),
                count_handler,
            });
        }
        self.update_count();
    }

    pub fn open(&self, replace: bool) {
        // Seed the field with a single-line selection.
        if let Some(attached) = self.attached.borrow().as_ref() {
            let buffer = attached.view.buffer();
            if let Some((start, end)) = buffer.selection_bounds() {
                if start.line() == end.line() {
                    let selected = buffer.text(&start, &end, false);
                    if !selected.is_empty() {
                        self.entry.set_text(&selected);
                    }
                }
            }
            attached.context.set_highlight(true);
        }
        self.root.set_visible(true);
        if replace || self.revealer.reveals_child() {
            self.show_replace(true);
        }
        self.commit();
        if replace {
            self.replace_entry.grab_focus();
        } else {
            self.entry.grab_focus();
            self.entry.select_region(0, -1);
        }
    }

    pub fn close(&self) {
        self.root.set_visible(false);
        if let Some(attached) = self.attached.borrow().as_ref() {
            attached.context.set_highlight(false);
            attached.view.grab_focus();
        }
    }

    fn show_replace(&self, show: bool) {
        self.revealer.set_reveal_child(show);
        if self.expander.is_active() != show {
            self.expander.set_active(show);
        }
        self.expander.set_icon_name(if show {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        });
        if show && self.is_open() {
            self.replace_entry.grab_focus();
        }
    }

    fn commit(&self) {
        let text = self.entry.text();
        self.settings
            .set_search_text((!text.is_empty()).then_some(text.as_str()));
        self.settings
            .set_case_sensitive(self.case_toggle.is_active());
        self.settings
            .set_at_word_boundaries(self.word_toggle.is_active());
        self.settings
            .set_regex_enabled(self.regexp_toggle.is_active());
        self.update_count();
    }

    fn update_count(&self) {
        match self.attached.borrow().as_ref() {
            Some(attached) => update_count_label(
                &self.count,
                &attached.context,
                &self.settings,
                &attached.view.buffer(),
            ),
            None => self.count.set_text(""),
        }
    }

    fn with_attached(
        &self,
        f: impl FnOnce(&sourceview5::SearchContext, &sourceview5::View, &gtk::TextBuffer),
    ) {
        let attached = self.attached.borrow();
        if let Some(attached) = attached.as_ref() {
            let buffer = attached.view.buffer();
            f(&attached.context, &attached.view, &buffer);
        }
    }

    pub fn find_next(&self) {
        self.with_attached(|context, view, buffer| {
            let (_, end) = selection_or_cursor(buffer);
            if let Some((start, finish, _wrapped)) = context.forward(&end) {
                select_match(view, buffer, &start, &finish);
            }
        });
        self.update_count();
    }

    pub fn find_previous(&self) {
        self.with_attached(|context, view, buffer| {
            let (start, _) = selection_or_cursor(buffer);
            if let Some((begin, finish, _wrapped)) = context.backward(&start) {
                select_match(view, buffer, &begin, &finish);
            }
        });
        self.update_count();
    }

    pub fn replace_current(&self) {
        let replacement = self.replace_entry.text();
        let mut advanced = false;
        self.with_attached(|context, _view, buffer| {
            if let Some((mut start, mut end)) = buffer.selection_bounds() {
                if context.occurrence_position(&start, &end) > 0 {
                    advanced = context.replace(&mut start, &mut end, &replacement).is_ok();
                }
            }
        });
        self.find_next();
        let _ = advanced;
    }

    pub fn replace_all(&self) {
        let replacement = self.replace_entry.text();
        self.with_attached(|context, _, _| {
            if let Err(error) = context.replace_all(&replacement) {
                eprintln!("RusTXT: replace all failed: {error}");
            }
        });
        self.update_count();
    }
}

fn selection_or_cursor(buffer: &gtk::TextBuffer) -> (gtk::TextIter, gtk::TextIter) {
    buffer.selection_bounds().unwrap_or_else(|| {
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        (cursor, cursor)
    })
}

fn select_match(
    view: &sourceview5::View,
    buffer: &gtk::TextBuffer,
    start: &gtk::TextIter,
    end: &gtk::TextIter,
) {
    buffer.select_range(start, end);
    view.scroll_to_mark(&buffer.get_insert(), 0.1, false, 0.0, 0.0);
}

fn update_count_label(
    label: &gtk::Label,
    context: &sourceview5::SearchContext,
    settings: &sourceview5::SearchSettings,
    buffer: &gtk::TextBuffer,
) {
    label.remove_css_class("error");
    if settings.search_text().is_none_or(|text| text.is_empty()) {
        label.set_text("");
        return;
    }
    let total = context.occurrences_count();
    if total < 0 {
        label.set_text("…");
    } else if total == 0 {
        label.set_text("No results");
        label.add_css_class("error");
    } else {
        let position = buffer
            .selection_bounds()
            .map(|(start, end)| context.occurrence_position(&start, &end))
            .unwrap_or(0);
        if position > 0 {
            label.set_text(&format!("{position} of {total}"));
        } else {
            label.set_text(&total.to_string());
        }
    }
}
