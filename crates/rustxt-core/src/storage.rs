//! SQLite-backed session and recovery storage.
//!
//! The filesystem stays authoritative for saved documents. This module only
//! stores what is needed to restore the workspace: open tabs, unsaved
//! snapshots, cursor and scroll positions, and recently closed tabs.
//! It has no dependency on Tauri so it can be tested on its own.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// How many clean "closed for now" tabs (saved files) are retained for
/// reopening. Tabs holding unsaved text are never pruned; only a discard
/// removes them.
pub const MAX_CLOSED: i64 = 20;
/// Longest first-line preview used to label a closed note.
const PREVIEW_CHARS: usize = 40;
/// How many recently closed tabs the UI lists.
const LISTED_CLOSED: i64 = 10;
/// How many characters of a closed note are fetched to build its preview
/// label: enough for any first line worth showing, without loading whole notes.
const PREVIEW_FETCH: i64 = 2048;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    #[serde(rename = "LF")]
    Lf,
    #[serde(rename = "CRLF")]
    Crlf,
}

impl LineEnding {
    /// Any `\r\n` at all makes the file CRLF, so a Windows file with a few
    /// stray `\n` is written back the way it came. A file with only lone
    /// `\r` (classic Mac) is treated as LF: nothing writes that style any
    /// more, and the text is normalized to `\n` on load either way.
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
    /// Fingerprint of the file version this buffer was loaded from. Used to
    /// refuse saves that would overwrite an external change, including after recovery.
    #[serde(default)]
    pub disk_fingerprint: Option<String>,
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
            disk_fingerprint: None,
        }
    }

    /// Give an untitled document a fresh "Untitled N" when its title is
    /// already in use, as happens when a closed note is reopened next to a
    /// tab that inherited its number. Files keep their name.
    pub fn ensure_unique_title<'a>(&mut self, existing_titles: impl Iterator<Item = &'a str>) {
        if self.file_path.is_some() {
            return;
        }
        let titles: Vec<&str> = existing_titles.collect();
        if titles.contains(&self.title.as_str()) {
            self.title = Self::untitled(titles.into_iter(), self.line_ending).title;
        }
    }
}

/// The label a closed note gets in the Recently closed menu: its first
/// non-blank line, shortened, so notes are told apart by what they say
/// instead of all reading "Untitled 1". Files keep their file name.
fn preview_title(content: &str, fallback: &str) -> String {
    let Some(line) = content.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return fallback.to_string();
    };
    let mut preview: String = line.chars().take(PREVIEW_CHARS).collect();
    if line.chars().count() > PREVIEW_CHARS {
        preview.push('\u{2026}');
    }
    preview
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
        , disk_fingerprint TEXT
    );
    CREATE TABLE IF NOT EXISTS app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS documents_is_open ON documents (is_open);
";

/// Per-document frontend state (such as undo history) is stored in
/// `app_state` under `<anything>:<document id>`. This predicate selects the
/// rows belonging to the document bound as `?1`, by exact suffix rather than
/// LIKE so ids containing `_` or `%` cannot match other documents' rows.
const STATE_FOR_DOCUMENT: &str = "substr(key, length(key) - length(?1)) = ':' || ?1";

const DOCUMENT_COLUMNS: &str =
    "id, file_path, title, content, dirty, cursor_offset, scroll_top, tab_position, line_ending, disk_fingerprint";

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
        disk_fingerprint: row.get(9)?,
    })
}

pub struct Storage {
    connection: Connection,
    recovered_from: Option<PathBuf>,
}

impl Storage {
    /// Open, or create, the recovery database. A database that can no
    /// longer be opened, because the file is corrupt or written by something
    /// else, is moved aside and replaced by an empty one rather than keeping
    /// the editor from starting; [`Storage::recovered_from`] says where the
    /// old file went so the user can be told.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        match connect(path) {
            Ok(connection) => Ok(Self {
                connection,
                recovered_from: None,
            }),
            // Only a database SQLite itself reports as corrupt or not a
            // database is replaced. A busy lock held by another process, a
            // permission problem, or a full disk must not move drafts aside.
            Err(ConnectError::Corrupt(error)) if path.exists() => {
                let moved = move_aside(path).map_err(|rename_error| {
                    format!("{error}; the file could not be moved aside either: {rename_error}")
                })?;
                let connection = connect(path).map_err(|retry| {
                    let (ConnectError::Corrupt(retry) | ConnectError::Other(retry)) = retry;
                    format!("{error}; a fresh database failed too: {retry}")
                })?;
                Ok(Self {
                    connection,
                    recovered_from: Some(moved),
                })
            }
            Err(ConnectError::Corrupt(error)) | Err(ConnectError::Other(error)) => Err(error),
        }
    }

    /// Where an unreadable database was moved to during [`Storage::open`],
    /// if that happened. The session it held is gone; the file is kept for
    /// the user to inspect or delete.
    pub fn recovered_from(&self) -> Option<&Path> {
        self.recovered_from.as_deref()
    }

    /// Write the recovery snapshot for one open document. A clean file-backed
    /// document stores no text: the file is authoritative and is reloaded on
    /// restore, so the database only ever holds what is not on disk.
    pub fn save_snapshot(&self, document: &DocumentState) -> Result<(), String> {
        let content = if document.file_path.is_some() && !document.dirty {
            ""
        } else {
            document.content.as_str()
        };
        self.connection
            .execute(
                "INSERT INTO documents (id, file_path, title, content, dirty, cursor_offset, scroll_top,
                                        tab_position, line_ending, disk_fingerprint, is_open, closed_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, NULL, unixepoch())
                 ON CONFLICT(id) DO UPDATE SET
                    file_path = excluded.file_path, title = excluded.title, content = excluded.content,
                    dirty = excluded.dirty, cursor_offset = excluded.cursor_offset,
                    scroll_top = excluded.scroll_top, tab_position = excluded.tab_position,
                    line_ending = excluded.line_ending, disk_fingerprint = excluded.disk_fingerprint,
                    is_open = 1, closed_at = NULL,
                    updated_at = unixepoch()",
                params![
                    document.id,
                    document.file_path,
                    document.title,
                    content,
                    document.dirty,
                    document.cursor_offset,
                    document.scroll_top,
                    document.tab_position,
                    document.line_ending.as_str(),
                    document.disk_fingerprint,
                ],
            )
            .map(drop)
            .map_err(text)
    }

    /// Atomically persist a snapshot and a frontend's associated editor metadata.
    pub fn save_snapshot_with_state(
        &self,
        document: &DocumentState,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        let transaction = self.connection.unchecked_transaction().map_err(text)?;
        self.save_snapshot(document)?;
        self.set_state(key, value)?;
        transaction.commit().map_err(text)
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

    /// Close a tab. `discard` permanently deletes the recovery snapshot and
    /// any frontend state stored for the document; otherwise the snapshot is
    /// kept so the tab can be reopened.
    pub fn close_document(&self, id: &str, discard: bool) -> Result<(), String> {
        let transaction = self.connection.unchecked_transaction().map_err(text)?;
        if discard {
            delete_document(&transaction, id)?;
        } else {
            transaction
                .execute(
                    "UPDATE documents SET is_open = 0, closed_at = unixepoch() WHERE id = ?1",
                    [id],
                )
                .map_err(text)?;
        }
        prune_closed(&transaction)?;
        transaction.commit().map_err(text)
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
                "SELECT id, title, file_path, dirty, closed_at, substr(content, 1, ?2)
                 FROM documents WHERE is_open = 0
                 ORDER BY closed_at DESC, updated_at DESC LIMIT ?1",
            )
            .map_err(text)?;
        let closed = statement
            .query_map([LISTED_CLOSED, PREVIEW_FETCH], |row| {
                let file_path: Option<String> = row.get(2)?;
                let stored_title: String = row.get(1)?;
                let title = if file_path.is_some() {
                    stored_title
                } else {
                    preview_title(&row.get::<_, String>(5)?, &stored_title)
                };
                Ok(ClosedDocument {
                    id: row.get(0)?,
                    title,
                    file_path,
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

/// The data directory holds unsaved text, so it is created readable by its
/// owner only. An existing directory keeps whatever mode it has.
fn create_private_dir(dir: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir).map_err(text)
}

/// Open the database at `path` and bring its schema up to date. Any failure
/// here means the file cannot be used as it is.
/// Why a database could not be opened: corruption SQLite itself detected,
/// which `Storage::open` recovers from, or anything else, which it reports.
enum ConnectError {
    Corrupt(String),
    Other(String),
}

impl From<rusqlite::Error> for ConnectError {
    fn from(error: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode::{DatabaseCorrupt, NotADatabase};
        match error.sqlite_error_code() {
            Some(DatabaseCorrupt) | Some(NotADatabase) => Self::Corrupt(error.to_string()),
            _ => Self::Other(error.to_string()),
        }
    }
}

impl From<String> for ConnectError {
    fn from(error: String) -> Self {
        Self::Other(error)
    }
}

fn connect(path: &Path) -> Result<Connection, ConnectError> {
    // Create the file ourselves, owner-only, before SQLite does: the -wal
    // and -shm files it adds later copy the main file's mode. The mode is
    // also applied to a database created by an earlier version, which
    // SQLite opened with the default umask.
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .and_then(|_| fs::set_permissions(path, fs::Permissions::from_mode(0o600)))
            .map_err(|e| ConnectError::Other(e.to_string()))?;
    }
    let connection = Connection::open(path)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    // With WAL, synchronous=NORMAL still makes every committed transaction
    // survive an application crash; only a power loss or OS crash can lose
    // the last transactions, which is the same guarantee the user's files
    // get from the filesystem. It avoids an fsync on every keystroke snapshot.
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    // Overwrite freed pages with zeros so discarded text does not linger in
    // the file after its row is deleted.
    connection.pragma_update(None, "secure_delete", "ON")?;
    connection.execute_batch(SCHEMA)?;
    // Databases created by earlier versions predate these columns.
    ensure_column(&connection, "closed_at", "INTEGER")?;
    ensure_column(&connection, "line_ending", "TEXT NOT NULL DEFAULT 'LF'")?;
    ensure_column(&connection, "disk_fingerprint", "TEXT")?;
    Ok(connection)
}

/// Rename an unusable database and its WAL companions to
/// `<name>.corrupt-<unix time>` so a fresh one can take their place.
/// Returns the main file's new path.
fn move_aside(path: &Path) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let aside = |file: &Path| {
        let mut name = file.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".corrupt-{stamp}"));
        file.with_file_name(name)
    };
    let moved = aside(path);
    fs::rename(path, &moved).map_err(text)?;
    for suffix in ["-wal", "-shm"] {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        let companion = path.with_file_name(name);
        if companion.exists() {
            fs::rename(&companion, aside(&companion)).map_err(text)?;
        }
    }
    Ok(moved)
}

/// Remove a document together with the frontend state stored for it.
fn delete_document(connection: &Connection, id: &str) -> Result<(), String> {
    connection
        .execute("DELETE FROM documents WHERE id = ?1", [id])
        .map_err(text)?;
    connection
        .execute(
            &format!("DELETE FROM app_state WHERE {STATE_FOR_DOCUMENT}"),
            [id],
        )
        .map(drop)
        .map_err(text)
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

/// Closed tabs are kept so they can be reopened, within limits. An empty
/// untitled tab has nothing to recover and is dropped. Tabs with unsaved
/// text are kept until the user discards them. Everything else, which is
/// saved files, is capped at the most recent [`MAX_CLOSED`]. Pruned
/// documents take their frontend state with them. Callers run this inside
/// a transaction so the selection and the deletes see the same rows.
fn prune_closed(connection: &Connection) -> Result<(), String> {
    // Blankness is judged on the first characters only, so a huge note is
    // never loaded just to be pruned; one longer than that is simply kept.
    let blank = format!(
        "is_open = 0 AND file_path IS NULL AND length(content) <= {PREVIEW_FETCH}
            AND trim(substr(content, 1, {PREVIEW_FETCH}), ' \t\r\n') = ''"
    );
    let surplus = "is_open = 0 AND dirty = 0 AND id NOT IN (
            SELECT id FROM documents WHERE is_open = 0 AND dirty = 0
            ORDER BY closed_at DESC, updated_at DESC LIMIT ?1)";
    for (condition, parameters) in [(blank.as_str(), vec![]), (surplus, vec![MAX_CLOSED])] {
        let mut statement = connection
            .prepare(&format!("SELECT id FROM documents WHERE {condition}"))
            .map_err(text)?;
        let ids = statement
            .query_map(rusqlite::params_from_iter(parameters), |row| {
                row.get::<_, String>(0)
            })
            .map_err(text)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(text)?;
        for id in ids {
            delete_document(connection, &id)?;
        }
    }
    Ok(())
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
            disk_fingerprint: None,
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
    fn reopened_note_takes_a_free_number_only_when_its_title_is_taken() {
        let mut note = document("n", 0);
        note.title = "Untitled 1".into();
        note.ensure_unique_title(["Untitled 1", "Untitled 2"].into_iter());
        assert_eq!(note.title, "Untitled 3");
        note.ensure_unique_title(["Untitled 1"].into_iter());
        assert_eq!(note.title, "Untitled 3", "a free title is left alone");
        let mut file = saved_file("f", 0);
        file.ensure_unique_title(["f.txt"].into_iter());
        assert_eq!(file.title, "f.txt", "files keep their name");
    }

    #[cfg(unix)]
    #[test]
    fn database_and_its_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("share/rustxt");
        let path = data.join("session.db");
        let storage = Storage::open(&path).unwrap();
        storage.save_snapshot(&document("a", 0)).unwrap();
        let mode = |file: &Path| fs::metadata(file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&data), 0o700);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&data.join("session.db-wal")), 0o600);
        // A database left world-readable by an earlier version is tightened.
        drop(storage);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let _storage = Storage::open(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_but_healthy_database_is_not_moved_aside() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.db");
        drop(Storage::open(&path).unwrap());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let error = Storage::open(&path)
            .err()
            .expect("open fails without permissions");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(path.exists(), "the database stays in place: {error}");
        assert!(
            !fs::read_dir(dir.path()).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("corrupt")),
            "nothing was moved aside"
        );
        assert!(Storage::open(&path).unwrap().recovered_from().is_none());
    }

    #[test]
    fn corrupt_database_is_moved_aside_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.db");
        fs::write(&path, b"this is not a database").unwrap();

        let storage = Storage::open(&path).unwrap();

        let moved = storage.recovered_from().expect("recovery is reported");
        assert_eq!(fs::read(moved).unwrap(), b"this is not a database");
        let moved_name = moved.file_name().unwrap().to_str().unwrap();
        assert!(
            moved_name.starts_with("session.db.corrupt-"),
            "{moved_name}"
        );
        storage.save_snapshot(&document("a", 0)).unwrap();
        assert_eq!(storage.restore_session().unwrap().documents.len(), 1);
        assert!(
            Storage::open(&path).unwrap().recovered_from().is_none(),
            "a healthy database opens without a notice"
        );
    }

    #[test]
    fn moving_aside_takes_the_wal_companions_along() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.db");
        for name in ["session.db", "session.db-wal", "session.db-shm"] {
            fs::write(dir.path().join(name), name).unwrap();
        }
        let moved = move_aside(&path).unwrap();
        let stamp = moved
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .strip_prefix("session.db.corrupt-")
            .unwrap()
            .to_string();
        for name in ["session.db", "session.db-wal", "session.db-shm"] {
            assert!(!dir.path().join(name).exists(), "{name} was left behind");
            let aside = dir.path().join(format!("{name}.corrupt-{stamp}"));
            assert_eq!(fs::read_to_string(&aside).unwrap(), name);
        }
    }

    #[test]
    fn discard_removes_the_frontend_state_of_that_document_only() {
        let (_dir, storage) = storage();
        storage.save_snapshot(&document("a", 0)).unwrap();
        storage.save_snapshot(&document("ab", 1)).unwrap();
        storage.set_state("iced-history:a", "history").unwrap();
        storage.set_state("iced-history:ab", "other").unwrap();
        storage.set_state("window", "800 600").unwrap();

        storage.close_document("a", true).unwrap();

        assert_eq!(storage.get_state("iced-history:a").unwrap(), None);
        assert_eq!(
            storage.get_state("iced-history:ab").unwrap().as_deref(),
            Some("other"),
            "only an exact `:<id>` suffix matches"
        );
        assert_eq!(
            storage.get_state("window").unwrap().as_deref(),
            Some("800 600")
        );
        storage.close_document("ab", false).unwrap();
        assert_eq!(
            storage.get_state("iced-history:ab").unwrap().as_deref(),
            Some("other"),
            "closing for now keeps the state for reopening"
        );
    }

    #[test]
    fn pruning_removes_the_frontend_state_of_pruned_documents() {
        let (_dir, mut storage) = storage();
        let mut blank = document("blank", 0);
        blank.content = "  \n".into();
        storage.save_snapshot(&blank).unwrap();
        storage.set_state("iced-history:blank", "x").unwrap();
        for index in 0..(MAX_CLOSED + 1) {
            let id = format!("file{index}");
            storage.save_snapshot(&saved_file(&id, index + 1)).unwrap();
            storage
                .set_state(&format!("iced-history:{id}"), "x")
                .unwrap();
        }
        // Closing everything through a layout update prunes the blank note
        // and one surplus saved file in the same transaction.
        storage.update_layout(&[], "").unwrap();

        assert_eq!(storage.get_state("iced-history:blank").unwrap(), None);
        assert_eq!(closed_count(&storage), MAX_CLOSED);
        let file_states: i64 = storage
            .connection
            .query_row(
                "SELECT COUNT(*) FROM app_state WHERE key LIKE 'iced-history:file%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(file_states, MAX_CLOSED, "the pruned file's state went too");
    }

    #[test]
    fn closed_tabs_are_indexed() {
        let (_dir, storage) = storage();
        let indexed: i64 = storage
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index'
                 AND name = 'documents_is_open'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexed, 1);
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

    fn saved_file(id: &str, position: i64) -> DocumentState {
        DocumentState {
            file_path: Some(format!("/tmp/{id}.txt")),
            title: format!("{id}.txt"),
            dirty: false,
            ..document(id, position)
        }
    }

    fn closed_count(storage: &Storage) -> i64 {
        storage
            .connection
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE is_open = 0",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn closed_saved_files_are_pruned_to_the_cap() {
        let (_dir, storage) = storage();
        for index in 0..(MAX_CLOSED + 5) {
            let id = format!("doc{index}");
            storage.save_snapshot(&saved_file(&id, index)).unwrap();
            storage.close_document(&id, false).unwrap();
        }
        assert_eq!(closed_count(&storage), MAX_CLOSED);
    }

    #[test]
    fn closed_notes_with_unsaved_text_are_never_pruned() {
        let (_dir, storage) = storage();
        for index in 0..(MAX_CLOSED + 5) {
            let id = format!("note{index}");
            storage.save_snapshot(&document(&id, index)).unwrap();
            storage.close_document(&id, false).unwrap();
        }
        // Saved files still get capped on their own, without touching notes.
        for index in 0..(MAX_CLOSED + 5) {
            let id = format!("file{index}");
            storage.save_snapshot(&saved_file(&id, index)).unwrap();
            storage.close_document(&id, false).unwrap();
        }
        assert_eq!(closed_count(&storage), MAX_CLOSED + 5 + MAX_CLOSED);
        assert!(storage.reopen_document("note0").unwrap().is_some());
    }

    #[test]
    fn closing_an_empty_untitled_tab_drops_it() {
        let (_dir, storage) = storage();
        let mut blank = document("blank", 0);
        blank.content = " \n\t\n".into();
        storage.save_snapshot(&blank).unwrap();
        storage.save_snapshot(&document("kept", 1)).unwrap();
        storage.close_document("blank", false).unwrap();
        storage.close_document("kept", false).unwrap();
        let ids: Vec<_> = storage
            .closed_documents()
            .unwrap()
            .into_iter()
            .map(|doc| doc.id)
            .collect();
        assert_eq!(ids, ["kept"]);
    }

    #[test]
    fn clean_saved_files_store_no_text() {
        let (_dir, storage) = storage();
        storage.save_snapshot(&saved_file("clean", 0)).unwrap();
        let mut edited = saved_file("edited", 1);
        edited.dirty = true;
        storage.save_snapshot(&edited).unwrap();
        let stored = |id: &str| storage.reopen_document(id).unwrap().unwrap().content;
        assert_eq!(stored("clean"), "");
        assert_eq!(stored("edited"), "content edited");
    }

    #[test]
    fn closed_notes_are_labelled_by_their_first_line() {
        let (_dir, storage) = storage();
        let mut note = document("note", 0);
        note.content = "\n  Shopping list  \nmilk\n".into();
        storage.save_snapshot(&note).unwrap();
        let mut long = document("long", 1);
        long.content = "x".repeat(PREVIEW_CHARS + 1);
        storage.save_snapshot(&long).unwrap();
        storage.save_snapshot(&saved_file("file", 2)).unwrap();
        for id in ["note", "long", "file"] {
            storage.close_document(id, false).unwrap();
        }
        let titles: Vec<_> = storage
            .closed_documents()
            .unwrap()
            .into_iter()
            .map(|doc| (doc.id, doc.title))
            .collect();
        let mut expected_long = "x".repeat(PREVIEW_CHARS);
        expected_long.push('\u{2026}');
        assert!(titles.contains(&("note".into(), "Shopping list".into())));
        assert!(titles.contains(&("long".into(), expected_long)));
        assert!(titles.contains(&("file".into(), "file.txt".into())));
        assert_eq!(preview_title("", "Untitled 4"), "Untitled 4");
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
