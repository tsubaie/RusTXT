//! Editor state and compact undo records, independent of the window widgets.
use iced::widget::text_editor::{Action, Content, Cursor, Edit, Motion, Position};
use rustxt_core::{files, storage::DocumentState};
use serde::{Deserialize, Serialize};
use std::{ops::Range, sync::Arc};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Change {
    start: usize,
    removed: String,
    inserted: String,
    #[serde(with = "saved_cursor")]
    before: Cursor,
    #[serde(with = "saved_cursor")]
    after: Cursor,
}

mod saved_cursor {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        cursor: &Cursor,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        (
            [cursor.position.line, cursor.position.column],
            cursor.selection.map(|p| [p.line, p.column]),
        )
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Cursor, D::Error> {
        let (position, selection) = <([usize; 2], Option<[usize; 2]>)>::deserialize(deserializer)?;
        Ok(Cursor {
            position: Position {
                line: position[0],
                column: position[1],
            },
            selection: selection.map(|p| Position {
                line: p[0],
                column: p[1],
            }),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct History {
    fingerprint: String,
    clean_fingerprint: String,
    undo: Vec<Change>,
    redo: Vec<Change>,
    /// Older builds stored the viewport here; it now lives under `iced-scroll:<id>`.
    #[serde(default)]
    scroll: Option<(usize, f32, f32)>,
}

/// Viewport as cosmic-text reports it: first visible line plus pixel offsets.
type Scroll = (usize, f32, f32);

fn history_key(id: &str) -> String {
    format!("iced-history:{id}")
}

fn scroll_key(id: &str) -> String {
    format!("iced-scroll:{id}")
}

fn encode_scroll((line, vertical, horizontal): Scroll) -> String {
    format!("{line} {vertical} {horizontal}")
}

fn decode_scroll(value: &str) -> Option<Scroll> {
    let mut parts = value.split_whitespace();
    let line = parts.next()?.parse().ok()?;
    let vertical = parts.next()?.parse().ok()?;
    let horizontal = parts.next()?.parse().ok()?;
    Some((line, vertical, horizontal))
}

pub struct Document {
    pub state: DocumentState,
    pub content: Content,
    undo: Vec<Change>,
    redo: Vec<Change>,
    clean_fingerprint: String,
    /// SHA-256 of `state.content`, refreshed whenever the text changes so that
    /// the dirty check and the history blob share one computation.
    fingerprint: String,
    char_count: usize,
    /// The text changed since the last `persist`: a full snapshot is due.
    pub content_dirty: bool,
    /// Only the cursor or viewport changed: a cheap view-state update is due.
    pub view_dirty: bool,
    /// Bumped whenever the undo/redo stacks or clean baseline change; the
    /// history blob is rewritten only when it differs from `persisted_generation`.
    history_generation: u64,
    persisted_generation: u64,
    persisted_scroll: Option<Scroll>,
    restore_scroll: Option<Scroll>,
}

impl Document {
    /// Write the full recovery snapshot, plus the undo history when it changed.
    pub fn persist(&mut self, storage: &rustxt_core::storage::Storage) -> Result<(), String> {
        self.capture_cursor();
        if self.history_generation != self.persisted_generation {
            let history = History {
                fingerprint: self.fingerprint.clone(),
                clean_fingerprint: self.clean_fingerprint.clone(),
                undo: self.undo.clone(),
                redo: self.redo.clone(),
                scroll: None,
            };
            storage.save_snapshot_with_state(
                &self.state,
                &history_key(&self.state.id),
                &serde_json::to_string(&history).map_err(|e| e.to_string())?,
            )?;
            self.persisted_generation = self.history_generation;
        } else {
            storage.save_snapshot(&self.state)?;
        }
        self.content_dirty = false;
        self.persist_view(storage)
    }

    /// Store only the cursor and viewport; the text on record is unchanged.
    pub fn persist_view(&mut self, storage: &rustxt_core::storage::Storage) -> Result<(), String> {
        self.capture_cursor();
        storage.save_view_state(
            &self.state.id,
            self.state.cursor_offset,
            self.state.scroll_top,
        )?;
        let scroll = self.scroll();
        if self.persisted_scroll != Some(scroll) {
            storage.set_state(&scroll_key(&self.state.id), &encode_scroll(scroll))?;
            self.persisted_scroll = Some(scroll);
        }
        self.view_dirty = false;
        Ok(())
    }

    fn scroll(&self) -> Scroll {
        self.restore_scroll
            .unwrap_or_else(|| self.content.scroll_position())
    }

    pub fn restore_history(
        &mut self,
        storage: &rustxt_core::storage::Storage,
    ) -> Result<(), String> {
        if let Some(saved) = storage.get_state(&history_key(&self.state.id))? {
            if let Ok(history) = serde_json::from_str::<History>(&saved) {
                if history.fingerprint == self.fingerprint {
                    self.undo = history.undo;
                    self.redo = history.redo;
                    self.clean_fingerprint = history.clean_fingerprint;
                    self.history_generation += 1;
                    if let Some(scroll) = history.scroll {
                        self.restore_scroll = Some(scroll);
                    }
                }
            }
        }
        if let Some(scroll) = storage
            .get_state(&scroll_key(&self.state.id))?
            .as_deref()
            .and_then(decode_scroll)
        {
            self.restore_scroll = Some(scroll);
            self.persisted_scroll = Some(scroll);
        }
        Ok(())
    }

    pub fn new(state: DocumentState) -> Self {
        let mut content = Content::with_text(&state.content);
        content.move_to(Cursor {
            position: char_position(&state.content, state.cursor_offset.max(0) as usize),
            selection: None,
        });
        let fingerprint = files::content_fingerprint(state.content.as_bytes());
        let clean_fingerprint = if state.dirty {
            String::new()
        } else {
            fingerprint.clone()
        };
        let restore_scroll = Some((0, state.scroll_top.max(0.0) as f32, 0.0));
        Self {
            char_count: state.content.chars().count(),
            state,
            content,
            undo: Vec::new(),
            redo: Vec::new(),
            clean_fingerprint,
            fingerprint,
            content_dirty: false,
            view_dirty: false,
            history_generation: 1,
            persisted_generation: 0,
            persisted_scroll: None,
            restore_scroll,
        }
    }

    /// Whether a viewport restore is still waiting for the first layout.
    pub fn viewport_pending(&self) -> bool {
        self.restore_scroll.is_some()
    }

    pub fn restore_viewport(&mut self) {
        if let Some((line, vertical, horizontal)) = self.restore_scroll.take() {
            self.content.set_scroll_position(line, vertical, horizontal);
        }
    }

    pub fn char_count(&self) -> usize {
        self.char_count
    }

    pub fn action(&mut self, action: Action) {
        self.restore_scroll = None;
        self.view_dirty = true;
        let action = normalize_action(action);
        let edits = action.is_edit();
        let before = self.content.cursor();
        self.content.perform(action);
        if edits {
            let next = self.content.text();
            if next != self.state.content {
                self.record(change(
                    &self.state.content,
                    &next,
                    before,
                    self.content.cursor(),
                ));
                self.state.content = next;
                self.changed();
            }
        }
        self.capture_cursor();
    }

    fn record(&mut self, change: Change) {
        self.redo.clear();
        self.history_generation += 1;
        if let Some(last) = self.undo.last_mut() {
            if coalesce(last, &change) {
                return;
            }
        }
        self.undo.push(change);
        // Bound history memory, retaining at least the latest operation.
        let mut bytes = 0;
        let keep = self
            .undo
            .iter()
            .rev()
            .take_while(|change| {
                bytes +=
                    change.inserted.len() + change.removed.len() + std::mem::size_of::<Change>();
                bytes <= 8 * 1024 * 1024
            })
            .count()
            .max(1);
        self.undo.drain(..self.undo.len().saturating_sub(keep));
    }

    fn changed(&mut self) {
        self.fingerprint = files::content_fingerprint(self.state.content.as_bytes());
        self.char_count = self.state.content.chars().count();
        self.state.dirty = self.fingerprint != self.clean_fingerprint;
        self.content_dirty = true;
        self.view_dirty = true;
    }

    pub fn capture_cursor(&mut self) {
        let offset = byte_offset(&self.state.content, self.content.cursor().position);
        let chars = self.state.content[..offset].chars().count() as i64;
        if self.state.cursor_offset != chars {
            self.state.cursor_offset = chars;
            self.view_dirty = true;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, redo: bool) {
        let change = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        let Some(change) = change else { return };
        self.history_generation += 1;
        let (remove, insert, cursor) = if redo {
            (&change.removed, &change.inserted, change.after)
        } else {
            (&change.inserted, &change.removed, change.before)
        };
        // A stale/corrupt history must never overwrite unrelated text.
        if self
            .state
            .content
            .get(change.start..change.start.saturating_add(remove.len()))
            != Some(remove.as_str())
        {
            self.undo.clear();
            self.redo.clear();
            return;
        }
        // Apply the inverse edit in place so the laid-out buffer and viewport survive.
        self.restore_scroll = None;
        self.select(change.start..change.start + remove.len());
        self.content
            .perform(Action::Edit(Edit::Paste(Arc::new(insert.clone()))));
        self.content.move_to(cursor);
        self.state
            .content
            .replace_range(change.start..change.start + remove.len(), insert);
        if redo {
            self.undo.push(change);
        } else {
            self.redo.push(change);
        }
        self.changed();
        self.capture_cursor();
    }

    pub fn replace(&mut self, range: Range<usize>, replacement: &str) {
        self.select(range);
        self.action(Action::Edit(Edit::Paste(Arc::new(replacement.to_owned()))));
    }

    pub fn select(&mut self, range: Range<usize>) {
        self.content.perform(Action::Move(Motion::DocumentStart));
        self.content.move_to(Cursor {
            position: byte_position(&self.state.content, range.end),
            selection: (range.start != range.end)
                .then(|| byte_position(&self.state.content, range.start)),
        });
        self.view_dirty = true;
    }

    pub fn selected_range(&self) -> Range<usize> {
        let cursor = self.content.cursor();
        let end = byte_offset(&self.state.content, cursor.position);
        let start = cursor
            .selection
            .map(|p| byte_offset(&self.state.content, p))
            .unwrap_or(end);
        start.min(end)..start.max(end)
    }

    pub fn mark_saved(&mut self, state: DocumentState) {
        self.state = state;
        self.fingerprint = files::content_fingerprint(self.state.content.as_bytes());
        self.char_count = self.state.content.chars().count();
        self.clean_fingerprint = self.fingerprint.clone();
        self.history_generation += 1;
        self.content_dirty = true;
        self.view_dirty = true;
    }
}

/// cosmic-text breaks lines on a bare `\r`, but our offsets only count `\n`.
/// Convert incoming text so the two never disagree.
fn normalize_action(action: Action) -> Action {
    match action {
        Action::Edit(Edit::Paste(text)) if text.contains('\r') => {
            Action::Edit(Edit::Paste(Arc::new(normalize_line_breaks(&text))))
        }
        Action::Edit(Edit::Insert('\r')) => Action::Edit(Edit::Enter),
        other => other,
    }
}

pub fn normalize_line_breaks(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Merge `next` into `last` when both are single-character typing or deleting
/// at adjacent positions, so Ctrl+Z steps back a word at a time. A run breaks
/// where a non-blank character follows blank text: "hello " and "world" are
/// separate steps, and the blank stays attached to the word before it.
fn coalesce(last: &mut Change, next: &Change) -> bool {
    fn single(text: &str) -> Option<char> {
        let mut chars = text.chars();
        let first = chars.next()?;
        chars.next().is_none().then_some(first)
    }
    fn joins(left: char, right: char) -> bool {
        !(left.is_whitespace() && !right.is_whitespace())
    }
    if last.after != next.before {
        return false;
    }
    let typed = last.removed.is_empty() && next.removed.is_empty();
    let deleted = last.inserted.is_empty() && next.inserted.is_empty();
    if typed {
        let Some(added) = single(&next.inserted) else {
            return false;
        };
        let Some(edge) = last.inserted.chars().next_back() else {
            return false;
        };
        if last.start + last.inserted.len() != next.start || !joins(edge, added) {
            return false;
        }
        last.inserted.push(added);
    } else if deleted {
        let Some(gone) = single(&next.removed) else {
            return false;
        };
        if next.start + next.removed.len() == last.start {
            // Backspace: the deleted text grows towards the start.
            let Some(edge) = last.removed.chars().next() else {
                return false;
            };
            if !joins(gone, edge) {
                return false;
            }
            last.removed.insert(0, gone);
            last.start = next.start;
        } else if next.start == last.start {
            // Delete: the deleted text grows towards the end.
            let Some(edge) = last.removed.chars().next_back() else {
                return false;
            };
            if !joins(edge, gone) {
                return false;
            }
            last.removed.push(gone);
        } else {
            return false;
        }
    } else {
        return false;
    }
    last.after = next.after;
    true
}

fn change(old: &str, new: &str, before: Cursor, after: Cursor) -> Change {
    let mut start = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
        start -= 1;
    }
    let mut suffix = old[start..]
        .bytes()
        .rev()
        .zip(new[start..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    Change {
        start,
        removed: old[start..old.len() - suffix].into(),
        inserted: new[start..new.len() - suffix].into(),
        before,
        after,
    }
}

// Iced columns are UTF-8 byte offsets. The existing recovery format uses characters.
pub fn byte_position(text: &str, offset: usize) -> Position {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = &text[..offset];
    Position {
        line: prefix.bytes().filter(|&b| b == b'\n').count(),
        column: prefix.rsplit('\n').next().unwrap_or("").len(),
    }
}

pub fn char_position(text: &str, chars: usize) -> Position {
    byte_position(
        text,
        text.char_indices()
            .nth(chars)
            .map(|(i, _)| i)
            .unwrap_or(text.len()),
    )
}

pub fn byte_offset(text: &str, position: Position) -> usize {
    let mut offset = 0;
    for (line, value) in text.split('\n').enumerate() {
        if line == position.line {
            let mut column = position.column.min(value.len());
            while !value.is_char_boundary(column) {
                column -= 1;
            }
            return offset + column;
        }
        offset += value.len() + 1;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustxt_core::storage::{LineEnding, Storage};
    fn doc(text: &str) -> Document {
        let mut state = DocumentState::untitled(std::iter::empty(), LineEnding::Lf);
        state.content = text.into();
        Document::new(state)
    }
    fn type_text(doc: &mut Document, text: &str) {
        for c in text.chars() {
            doc.action(Action::Edit(if c == '\n' {
                Edit::Enter
            } else {
                Edit::Insert(c)
            }));
        }
    }
    #[test]
    fn preserves_exact_text_and_unicode_cursor() {
        for value in ["", "hello", "hello\n", "\n\n", "مرحبا\nEnglish 👋\n"] {
            let mut doc = doc(value);
            assert_eq!(doc.content.text(), value);
            doc.select(value.len()..value.len());
            doc.capture_cursor();
            assert_eq!(doc.state.cursor_offset as usize, value.chars().count());
            let restored = Document::new(doc.state);
            assert_eq!(
                byte_offset(value, restored.content.cursor().position),
                value.len()
            );
        }
    }
    #[test]
    fn replacement_undo_redo_and_clean_baseline() {
        let mut doc = doc("مرحبا world\n");
        doc.replace(0.."مرحبا".len(), "أهلا");
        assert_eq!(doc.state.content, "أهلا world\n");
        assert!(doc.state.dirty);
        doc.undo(false);
        assert_eq!(doc.state.content, "مرحبا world\n");
        assert!(!doc.state.dirty);
        doc.undo(true);
        assert_eq!(doc.state.content, "أهلا world\n");
        doc.mark_saved(doc.state.clone());
        doc.undo(false);
        assert!(doc.state.dirty);
    }
    #[test]
    fn typing_coalesces_into_word_steps() {
        let mut doc = doc("");
        type_text(&mut doc, "hello world");
        assert_eq!(doc.state.content, "hello world");
        assert_eq!(doc.char_count(), 11);
        let mut steps = 0;
        while doc.can_undo() {
            doc.undo(false);
            steps += 1;
        }
        assert_eq!(doc.state.content, "");
        assert!(steps <= 2, "undo took {steps} steps");
        doc.undo(true);
        assert_eq!(doc.state.content, "hello ");
        doc.undo(true);
        assert_eq!(doc.state.content, "hello world");
        assert_eq!(doc.content.text(), doc.state.content);
    }
    #[test]
    fn deleting_coalesces_and_a_selection_edit_starts_a_new_step() {
        let mut doc = doc("");
        type_text(&mut doc, "ab cd");
        for _ in 0..5 {
            doc.action(Action::Edit(Edit::Backspace));
        }
        assert_eq!(doc.state.content, "");
        let mut steps = 0;
        while doc.state.content != "ab cd" {
            assert!(doc.can_undo());
            doc.undo(false);
            steps += 1;
        }
        assert!(steps <= 2, "backspace undo took {steps} steps");
        doc.select(0..2);
        doc.action(Action::Edit(Edit::Insert('x')));
        assert_eq!(doc.state.content, "x cd");
        doc.undo(false);
        assert_eq!(doc.state.content, "ab cd");
        doc.select(0..0);
        for _ in 0..5 {
            doc.action(Action::Edit(Edit::Delete));
        }
        assert_eq!(doc.state.content, "");
        doc.undo(false);
        assert!(!doc.state.content.is_empty());
        assert!(doc.can_undo());
    }
    #[test]
    fn undo_redo_round_trip_keeps_content_cursor_and_layout_in_sync() {
        let mut doc = doc("one\ntwo\nthree\n");
        doc.select(4..4);
        type_text(&mut doc, "مرحبا ");
        let typed_cursor = doc.content.cursor();
        doc.replace(0..3, "ONE");
        assert_eq!(doc.state.content, "ONE\nمرحبا two\nthree\n");
        doc.undo(false);
        assert_eq!(doc.state.content, "one\nمرحبا two\nthree\n");
        assert_eq!(doc.content.text(), doc.state.content);
        doc.undo(false);
        assert_eq!(doc.state.content, "one\ntwo\nthree\n");
        assert_eq!(
            doc.content.cursor().position,
            Position { line: 1, column: 0 }
        );
        doc.undo(true);
        assert_eq!(doc.state.content, "one\nمرحبا two\nthree\n");
        assert_eq!(doc.content.cursor(), typed_cursor);
        doc.undo(true);
        assert_eq!(doc.state.content, "ONE\nمرحبا two\nthree\n");
        assert_eq!(doc.content.text(), doc.state.content);
        assert_eq!(
            doc.state.cursor_offset as usize,
            doc.state.content[..doc.selected_range().end]
                .chars()
                .count()
        );
    }
    #[test]
    fn pasted_carriage_returns_keep_offsets_aligned_with_the_editor() {
        let mut doc = doc("");
        doc.action(Action::Edit(Edit::Paste(Arc::new("a\rb\r\nc".into()))));
        assert_eq!(doc.state.content, "a\nb\nc");
        assert_eq!(doc.content.text(), doc.state.content);
        assert_eq!(doc.content.line_count(), 3);
        assert_eq!(
            byte_offset(&doc.state.content, doc.content.cursor().position),
            doc.state.content.len()
        );
        doc.action(Action::Edit(Edit::Insert('\r')));
        assert_eq!(doc.state.content, "a\nb\nc\n");
        assert_eq!(
            doc.content.cursor().position,
            Position { line: 3, column: 0 }
        );
        assert_eq!(doc.selected_range(), 6..6);
    }
    #[test]
    fn cursor_moves_only_mark_the_view_dirty() {
        let mut doc = doc("hello\nworld");
        assert!(!doc.content_dirty && !doc.view_dirty);
        doc.action(Action::Move(Motion::Right));
        assert!(doc.view_dirty && !doc.content_dirty);
        doc.view_dirty = false;
        doc.action(Action::Edit(Edit::Insert('x')));
        assert!(doc.content_dirty && doc.view_dirty);
    }
    #[test]
    fn view_flush_persists_cursor_and_scroll_without_rewriting_history() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("session.db")).unwrap();
        let mut doc = doc("");
        type_text(&mut doc, "first\nsecond");
        doc.persist(&storage).unwrap();
        let generation = doc.persisted_generation;
        doc.action(Action::Move(Motion::Left));
        doc.action(Action::Move(Motion::Left));
        doc.restore_scroll = Some((1, 2.5, 0.0));
        doc.persist_view(&storage).unwrap();
        assert!(!doc.view_dirty);
        assert_eq!(doc.persisted_generation, generation);
        let state = storage.restore_session().unwrap().documents.remove(0);
        assert_eq!(state.cursor_offset, 10);
        let mut restored = Document::new(state);
        restored.restore_history(&storage).unwrap();
        assert_eq!(restored.restore_scroll, Some((1, 2.5, 0.0)));
        assert!(restored.can_undo());
        // A second full persist with no history change writes only the snapshot.
        let blob = storage.get_state(&history_key(&doc.state.id)).unwrap();
        doc.action(Action::Move(Motion::Right));
        doc.persist(&storage).unwrap();
        assert_eq!(
            storage.get_state(&history_key(&doc.state.id)).unwrap(),
            blob
        );
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use iced_core::text::{editor::Editor as _, highlighter::PlainText, LineHeight, Wrapping};
    use rustxt_core::storage::{LineEnding, Storage};

    fn laid_out(text: &str) -> iced_graphics::text::Editor {
        let mut editor = iced_graphics::text::Editor::with_text(text);
        editor.update(
            iced::Size::new(800.0, 400.0),
            iced::Font::MONOSPACE,
            iced::Pixels(16.0),
            LineHeight::default(),
            Wrapping::WordOrGlyph,
            &mut PlainText,
        );
        editor
    }

    #[test]
    fn arabic_caret_selection_and_scroll_use_layout_coordinates() {
        let mut editor = laid_out("مرحبا بالعالم\nEnglish");
        let iced_core::text::editor::Selection::Caret(start) = editor.selection() else {
            panic!("Expected caret")
        };
        assert!(
            start.x > 700.0,
            "Arabic start must be on the right: {start:?}"
        );
        editor.move_to(Cursor {
            position: Position {
                line: 0,
                column: "مرحبا".len(),
            },
            selection: Some(Position { line: 0, column: 0 }),
        });
        let iced_core::text::editor::Selection::Range(regions) = editor.selection() else {
            panic!("Expected selection")
        };
        assert!(!regions.is_empty());
        assert!(
            regions.iter().all(|r| r.x > 650.0),
            "Arabic selection must overlap its text: {regions:?}"
        );
        editor.move_to(Cursor {
            position: Position { line: 1, column: 0 },
            selection: None,
        });
        assert!(
            editor.cursor().selection.is_none(),
            "Moving to a caret clears selection"
        );

        let mut editor = laid_out(&"line\n".repeat(1000));
        editor.set_scroll_position(50, 4.0, 0.0);
        assert_eq!(editor.scroll_position(), (50, 4.0, 0.0));
    }

    #[test]
    fn opening_large_document_only_lays_out_the_viewport() {
        let content = "English and Arabic: مرحبا بالعالم\n".repeat(16000);
        let editor = iced_graphics::text::Editor::with_text(&content);
        let laid_out = editor
            .buffer()
            .lines
            .iter()
            .filter(|line| line.layout_opt().is_some())
            .count();
        assert!(
            laid_out < 100,
            "Initial layout unexpectedly shaped {laid_out} lines"
        );
    }

    #[test]
    fn undo_survives_recovery_but_stale_history_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("session.db")).unwrap();
        let mut doc = Document::new(DocumentState::untitled(std::iter::empty(), LineEnding::Lf));
        doc.replace(0..0, "مرحبا\nhello");
        doc.persist(&storage).unwrap();
        let state = storage.restore_session().unwrap().documents.remove(0);
        let mut restored = Document::new(state.clone());
        restored.restore_history(&storage).unwrap();
        restored.undo(false);
        assert_eq!(restored.state.content, "");
        restored.undo(true);
        assert_eq!(restored.state.content, "مرحبا\nhello");
        let mut changed = Document::new(DocumentState {
            content: "external change".into(),
            ..state
        });
        changed.restore_history(&storage).unwrap();
        assert!(!changed.can_undo());
    }
}
