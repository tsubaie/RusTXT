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
    #[serde(default)]
    scroll: Option<(usize, f32, f32)>,
}

pub struct Document {
    pub state: DocumentState,
    pub content: Content,
    undo: Vec<Change>,
    redo: Vec<Change>,
    clean_fingerprint: String,
    pub pending: bool,
    restore_scroll: Option<(usize, f32, f32)>,
}

impl Document {
    pub fn persist(&self, storage: &rustxt_core::storage::Storage) -> Result<(), String> {
        let history = History {
            fingerprint: files::content_fingerprint(self.state.content.as_bytes()),
            clean_fingerprint: self.clean_fingerprint.clone(),
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            scroll: Some(
                self.restore_scroll
                    .unwrap_or_else(|| self.content.scroll_position()),
            ),
        };
        storage.save_snapshot_with_state(
            &self.state,
            &format!("iced-history:{}", self.state.id),
            &serde_json::to_string(&history).map_err(|e| e.to_string())?,
        )
    }

    pub fn restore_history(
        &mut self,
        storage: &rustxt_core::storage::Storage,
    ) -> Result<(), String> {
        if let Some(saved) = storage.get_state(&format!("iced-history:{}", self.state.id))? {
            if let Ok(history) = serde_json::from_str::<History>(&saved) {
                if history.fingerprint == files::content_fingerprint(self.state.content.as_bytes())
                {
                    self.undo = history.undo;
                    self.redo = history.redo;
                    self.clean_fingerprint = history.clean_fingerprint;
                    self.restore_scroll = history.scroll;
                }
            }
        }
        Ok(())
    }

    pub fn new(state: DocumentState) -> Self {
        let mut content = Content::with_text(&state.content);
        content.move_to(Cursor {
            position: char_position(&state.content, state.cursor_offset.max(0) as usize),
            selection: None,
        });
        let clean_fingerprint = if state.dirty {
            String::new()
        } else {
            files::content_fingerprint(state.content.as_bytes())
        };
        let restore_scroll = Some((0, state.scroll_top.max(0.0) as f32, 0.0));
        Self {
            state,
            content,
            undo: Vec::new(),
            redo: Vec::new(),
            clean_fingerprint,
            pending: false,
            restore_scroll,
        }
    }

    pub fn restore_viewport(&mut self) {
        if let Some((line, vertical, horizontal)) = self.restore_scroll.take() {
            self.content.set_scroll_position(line, vertical, horizontal);
        }
    }

    pub fn action(&mut self, action: Action) {
        self.restore_scroll = None;
        self.pending = true;
        let edits = action.is_edit();
        let before = self.content.cursor();
        self.content.perform(action);
        if edits {
            let next = self.content.text();
            if next != self.state.content {
                self.undo.push(change(
                    &self.state.content,
                    &next,
                    before,
                    self.content.cursor(),
                ));
                self.redo.clear();
                // Bound history memory, retaining at least the latest operation.
                let mut bytes = 0;
                let keep = self
                    .undo
                    .iter()
                    .rev()
                    .take_while(|change| {
                        bytes += change.inserted.len()
                            + change.removed.len()
                            + std::mem::size_of::<Change>();
                        bytes <= 8 * 1024 * 1024
                    })
                    .count()
                    .max(1);
                self.undo.drain(..self.undo.len().saturating_sub(keep));
                self.state.content = next;
                self.changed();
            }
        }
        self.capture_cursor();
    }

    fn changed(&mut self) {
        self.state.dirty =
            files::content_fingerprint(self.state.content.as_bytes()) != self.clean_fingerprint;
        self.pending = true;
    }

    pub fn capture_cursor(&mut self) {
        let offset = byte_offset(&self.state.content, self.content.cursor().position);
        let chars = self.state.content[..offset].chars().count() as i64;
        if self.state.cursor_offset != chars {
            self.state.cursor_offset = chars;
            self.pending = true;
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
        self.state
            .content
            .replace_range(change.start..change.start + remove.len(), insert);
        self.content = Content::with_text(&self.state.content);
        self.content.move_to(cursor);
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
        self.clean_fingerprint = files::content_fingerprint(state.content.as_bytes());
        self.state = state;
        self.pending = true;
    }
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
    use rustxt_core::storage::LineEnding;
    fn doc(text: &str) -> Document {
        let mut state = DocumentState::untitled(std::iter::empty(), LineEnding::Lf);
        state.content = text.into();
        Document::new(state)
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
