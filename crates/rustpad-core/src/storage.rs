//! SQLite-backed session and recovery storage.
//!
//! The filesystem stays authoritative for saved documents. This module only
//! stores what is needed to restore the workspace: open tabs, unsaved
//! snapshots, cursor and scroll positions, and recently closed tabs.
//! It has no dependency on Tauri so it can be tested on its own.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

/// How many "closed for now" tabs are retained for reopening.
pub const MAX_CLOSED: i64 = 20;
/// How many recently closed tabs the UI lists.
pub const LISTED_CLOSED: i64 = 10;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    #[serde(rename = "LF")]
    Lf,
    #[serde(rename = "CRLF")]
    Crlf,
}

impl LineEnding {
    pub fn detect(text: &str) -> Self {
        if text.contains("\r\n") {
            Self::Crlf
        } else {
            Self::Lf
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value == "CRLF" {
            Self::Crlf
        } else {
            Self::Lf
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentState {
    pub id: String,
    pub file_path: Option<String>,
    pub title: String,
    /// Always stored with `\n` line breaks; `line_ending` says how to write it out.
    pub content: String,
    pub dirty: bool,
    pub cursor_offset: i64,
    pub scroll_top: f64,
    pub tab_position: i64,
    #[serde(default)]
    pub line_ending: LineEnding,
}

impl DocumentState {
    /// A new empty document titled "Untitled N", using the smallest N that is
    /// not already taken by one of `existing_titles`.
    pub fn untitled<'a>(
        existing_titles: impl Iterator<Item = &'a str>,
        line_ending: LineEnding,
    ) -> Self {
        let used: std::collections::HashSet<u32> = existing_titles
            .filter_map(|title| title.strip_prefix("Untitled ")?.parse().ok())
            .collect();
        let number = (1..).find(|n| !used.contains(n)).unwrap_or(1);
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            file_path: None,
            title: format!("Untitled {number}"),
            content: String::new(),
            dirty: false,
            cursor_offset: 0,
            scroll_top: 0.0,
            tab_position: 0,
            line_ending,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClosedDocument {
    pub id: String,
    pub title: String,
    pub file_path: Option<String>,
    pub dirty: bool,
    pub closed_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub documents: Vec<DocumentState>,
    pub active_id: Option<String>,
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS documents (
        id TEXT PRIMARY KEY,
        file_path TEXT,
        title TEXT NOT NULL,
        content TEXT NOT NULL,
        dirty INTEGER NOT NULL,
        cursor_offset INTEGER NOT NULL,
        scroll_top REAL NOT NULL,
        tab_position INTEGER NOT NULL,
        is_open INTEGER NOT NULL DEFAULT 1,
        updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
        closed_at INTEGER,
        line_ending TEXT NOT NULL DEFAULT 'LF'
    );
    CREATE TABLE IF NOT EXISTS app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

const DOCUMENT_COLUMNS: &str =
    "id, file_path, title, content, dirty, cursor_offset, scroll_top, tab_position, line_ending";

fn text(error: impl ToString) -> String {
    error.to_string()
}

fn read_document(row: &Row) -> rusqlite::Result<DocumentState> {
    let line_ending: String = row.get(8)?;
    Ok(DocumentState {
        id: row.get(0)?,
        file_path: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        dirty: row.get(4)?,
        cursor_offset: row.get(5)?,
        scroll_top: row.get(6)?,
        tab_position: row.get(7)?,
        line_ending: LineEnding::parse(&line_ending),
    })
}

pub struct Storage {
    connection: Connection,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(text)?;
        }
        let connection = Connection::open(path).map_err(text)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(text)?;
        connection.execute_batch(SCHEMA).map_err(text)?;
        // Databases created by earlier versions predate these columns.
        ensure_column(&connection, "closed_at", "INTEGER")?;
        ensure_column(&connection, "line_ending", "TEXT NOT NULL DEFAULT 'LF'")?;
        Ok(Self { connection })
    }

    /// Write the full recovery snapshot for one open document.
    pub fn save_snapshot(&self, document: &DocumentState) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO documents (id, file_path, title, content, dirty, cursor_offset, scroll_top,
                                        tab_position, line_ending, is_open, closed_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, NULL, unixepoch())
                 ON CONFLICT(id) DO UPDATE SET
                    file_path = excluded.file_path, title = excluded.title, content = excluded.content,
                    dirty = excluded.dirty, cursor_offset = excluded.cursor_offset,
                    scroll_top = excluded.scroll_top, tab_position = excluded.tab_position,
                    line_ending = excluded.line_ending, is_open = 1, closed_at = NULL,
                    updated_at = unixepoch()",
                params![
                    document.id,
                    document.file_path,
                    document.title,
                    document.content,
                    document.dirty,
                    document.cursor_offset,
                    document.scroll_top,
                    document.tab_position,
                    document.line_ending.as_str(),
                ],
            )
            .map(drop)
            .map_err(text)
    }

    /// Update only cursor and scroll position; avoids rewriting the content.
    pub fn save_view_state(
        &self,
        id: &str,
        cursor_offset: i64,
        scroll_top: f64,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE documents SET cursor_offset = ?2, scroll_top = ?3 WHERE id = ?1",
                params![id, cursor_offset, scroll_top],
            )
            .map(drop)
            .map_err(text)
    }

    /// Record which documents are open, in what order, and which is active.
    /// Open documents missing from `order` become "closed for now".
    pub fn update_layout(&mut self, order: &[String], active_id: &str) -> Result<(), String> {
        let transaction = self.connection.transaction().map_err(text)?;
        transaction
            .execute(
                "UPDATE documents SET is_open = 0, closed_at = unixepoch() WHERE is_open = 1",
                [],
            )
            .map_err(text)?;
        for (position, id) in order.iter().enumerate() {
            transaction
                .execute(
                    "UPDATE documents SET is_open = 1, closed_at = NULL, tab_position = ?2 WHERE id = ?1",
                    params![id, position as i64],
                )
                .map_err(text)?;
        }
        transaction
            .execute(
                "INSERT INTO app_state (key, value) VALUES ('active_id', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [active_id],
            )
            .map_err(text)?;
        prune_closed(&transaction)?;
        transaction.commit().map_err(text)
    }

    /// Close a tab. `discard` permanently deletes the recovery snapshot;
    /// otherwise the snapshot is kept so the tab can be reopened.
    pub fn close_document(&self, id: &str, discard: bool) -> Result<(), String> {
        let statement = if discard {
            "DELETE FROM documents WHERE id = ?1"
        } else {
            "UPDATE documents SET is_open = 0, closed_at = unixepoch() WHERE id = ?1"
        };
        self.connection.execute(statement, [id]).map_err(text)?;
        prune_closed(&self.connection)
    }

    pub fn restore_session(&self) -> Result<Session, String> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {DOCUMENT_COLUMNS} FROM documents WHERE is_open = 1 ORDER BY tab_position"
            ))
            .map_err(text)?;
        let documents = statement
            .query_map([], read_document)
            .map_err(text)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(text)?;
        let active_id = self
            .connection
            .query_row(
                "SELECT value FROM app_state WHERE key = 'active_id'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(text)?;
        Ok(Session {
            documents,
            active_id,
        })
    }

    pub fn closed_documents(&self) -> Result<Vec<ClosedDocument>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, title, file_path, dirty, closed_at FROM documents
                 WHERE is_open = 0 ORDER BY closed_at DESC, updated_at DESC LIMIT ?1",
            )
            .map_err(text)?;
        let closed = statement
            .query_map([LISTED_CLOSED], |row| {
                Ok(ClosedDocument {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    file_path: row.get(2)?,
                    dirty: row.get(3)?,
                    closed_at: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                })
            })
            .map_err(text)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(text)?;
        Ok(closed)
    }

    pub fn get_state(&self, key: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row("SELECT value FROM app_state WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(text)
    }

    pub fn set_state(&self, key: &str, value: &str) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO app_state (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [key, value],
            )
            .map(drop)
            .map_err(text)
    }

    /// Bring a closed tab back as the last open tab.
    pub fn reopen_document(&self, id: &str) -> Result<Option<DocumentState>, String> {
        let next_position: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(MAX(tab_position) + 1, 0) FROM documents WHERE is_open = 1",
                [],
                |row| row.get(0),
            )
            .map_err(text)?;
        self.connection
            .execute(
                "UPDATE documents SET is_open = 1, closed_at = NULL, tab_position = ?2 WHERE id = ?1",
                params![id, next_position],
            )
            .map_err(text)?;
        self.connection
            .query_row(
                &format!("SELECT {DOCUMENT_COLUMNS} FROM documents WHERE id = ?1"),
                [id],
                read_document,
            )
            .optional()
            .map_err(text)
    }
}

fn ensure_column(connection: &Connection, name: &str, definition: &str) -> Result<(), String> {
    let exists: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('documents') WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .map_err(text)?;
    if exists == 0 {
        connection
            .execute(
                &format!("ALTER TABLE documents ADD COLUMN {name} {definition}"),
                [],
            )
            .map_err(text)?;
    }
    Ok(())
}

fn prune_closed(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM documents WHERE is_open = 0 AND id NOT IN (
                SELECT id FROM documents WHERE is_open = 0
                ORDER BY closed_at DESC, updated_at DESC LIMIT ?1)",
            [MAX_CLOSED],
        )
        .map(drop)
        .map_err(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(id: &str, position: i64) -> DocumentState {
        DocumentState {
            id: id.into(),
            file_path: None,
            title: format!("Untitled {id}"),
            content: format!("content {id}"),
            dirty: true,
            cursor_offset: 4,
            scroll_top: 2.0,
            tab_position: position,
            line_ending: LineEnding::Lf,
        }
    }

    fn storage() -> (tempfile::TempDir, Storage) {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("state.db")).unwrap();
        (dir, storage)
    }

    #[test]
    fn untitled_picks_the_smallest_free_number() {
        let titles = ["Untitled 1", "notes.txt", "Untitled 3"];
        let doc = DocumentState::untitled(titles.iter().copied(), LineEnding::Lf);
        assert_eq!(doc.title, "Untitled 2");
        assert_eq!(
            DocumentState::untitled(std::iter::empty(), LineEnding::Lf).title,
            "Untitled 1"
        );
    }

    #[test]
    fn snapshot_round_trips_through_session() {
        let (_dir, mut storage) = storage();
        let doc = document("one", 0);
        storage.save_snapshot(&doc).unwrap();
        storage.update_layout(&["one".into()], "one").unwrap();
        let session = storage.restore_session().unwrap();
        assert_eq!(session.documents, vec![doc]);
        assert_eq!(session.active_id.as_deref(), Some("one"));
    }

    #[test]
    fn layout_orders_tabs_and_closes_missing_ones() {
        let (_dir, mut storage) = storage();
        for id in ["a", "b", "c"] {
            storage.save_snapshot(&document(id, 0)).unwrap();
        }
        storage
            .update_layout(&["c".into(), "a".into()], "a")
            .unwrap();
        let session = storage.restore_session().unwrap();
        let ids: Vec<_> = session.documents.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["c", "a"]);
        let closed = storage.closed_documents().unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].id, "b");
    }

    #[test]
    fn close_for_now_keeps_snapshot_and_reopen_restores_it() {
        let (_dir, mut storage) = storage();
        storage.save_snapshot(&document("a", 0)).unwrap();
        storage.save_snapshot(&document("b", 1)).unwrap();
        storage
            .update_layout(&["a".into(), "b".into()], "b")
            .unwrap();
        storage.close_document("a", false).unwrap();
        assert_eq!(storage.restore_session().unwrap().documents.len(), 1);

        let reopened = storage.reopen_document("a").unwrap().unwrap();
        assert_eq!(reopened.content, "content a");
        assert_eq!(reopened.tab_position, 2, "reopened tab goes to the end");
        assert!(storage.closed_documents().unwrap().is_empty());
    }

    #[test]
    fn discard_deletes_snapshot() {
        let (_dir, storage) = storage();
        storage.save_snapshot(&document("a", 0)).unwrap();
        storage.close_document("a", true).unwrap();
        assert!(storage.reopen_document("a").unwrap().is_none());
    }

    #[test]
    fn closed_documents_are_pruned() {
        let (_dir, storage) = storage();
        for index in 0..(MAX_CLOSED + 5) {
            let id = format!("doc{index}");
            storage.save_snapshot(&document(&id, index)).unwrap();
            storage.close_document(&id, false).unwrap();
        }
        let total: i64 = storage
            .connection
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE is_open = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total, MAX_CLOSED);
    }

    #[test]
    fn view_state_updates_without_touching_content() {
        let (_dir, storage) = storage();
        storage.save_snapshot(&document("a", 0)).unwrap();
        storage.save_view_state("a", 42, 7.5).unwrap();
        let doc = storage.reopen_document("a").unwrap().unwrap();
        assert_eq!(
            (doc.cursor_offset, doc.scroll_top, doc.content.as_str()),
            (42, 7.5, "content a")
        );
    }

    #[test]
    fn line_ending_survives_storage() {
        let (_dir, storage) = storage();
        let mut doc = document("crlf", 0);
        doc.line_ending = LineEnding::Crlf;
        storage.save_snapshot(&doc).unwrap();
        assert_eq!(
            storage
                .reopen_document("crlf")
                .unwrap()
                .unwrap()
                .line_ending,
            LineEnding::Crlf
        );
    }

    #[test]
    fn app_state_round_trips() {
        let (_dir, storage) = storage();
        assert_eq!(storage.get_state("titlebar").unwrap(), None);
        storage.set_state("titlebar", "hide").unwrap();
        storage.set_state("titlebar", "show").unwrap();
        assert_eq!(
            storage.get_state("titlebar").unwrap().as_deref(),
            Some("show")
        );
    }

    #[test]
    fn old_schema_is_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let legacy = Connection::open(&path).unwrap();
        legacy
            .execute_batch(
                "CREATE TABLE documents (id TEXT PRIMARY KEY, file_path TEXT, title TEXT NOT NULL,
                 content TEXT NOT NULL, dirty INTEGER NOT NULL, cursor_offset INTEGER NOT NULL,
                 scroll_top REAL NOT NULL, tab_position INTEGER NOT NULL,
                 is_open INTEGER NOT NULL DEFAULT 1, updated_at INTEGER NOT NULL DEFAULT (unixepoch()));
                 INSERT INTO documents VALUES ('old', NULL, 'Untitled', 'text', 1, 0, 0.0, 0, 1, 0);",
            )
            .unwrap();
        drop(legacy);
        let storage = Storage::open(&path).unwrap();
        let session = storage.restore_session().unwrap();
        assert_eq!(session.documents[0].line_ending, LineEnding::Lf);
    }
}
