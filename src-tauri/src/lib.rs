use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::{Path, PathBuf}, sync::Mutex};
use tauri::{Manager, State};

struct Database(Mutex<Connection>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DocumentState {
    id: String,
    file_path: Option<String>,
    title: String,
    content: String,
    dirty: bool,
    cursor_offset: i64,
    scroll_top: f64,
    tab_position: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Session { documents: Vec<DocumentState>, active_id: Option<String> }

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let connection = Connection::open(path).map_err(|e| e.to_string())?;
    connection.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS documents (
            id TEXT PRIMARY KEY, file_path TEXT, title TEXT NOT NULL, content TEXT NOT NULL,
            dirty INTEGER NOT NULL, cursor_offset INTEGER NOT NULL, scroll_top REAL NOT NULL,
            tab_position INTEGER NOT NULL, is_open INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE TABLE IF NOT EXISTS app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);"
    ).map_err(|e| e.to_string())?;
    Ok(connection)
}

fn upsert(connection: &Connection, document: &DocumentState) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO documents (id,file_path,title,content,dirty,cursor_offset,scroll_top,tab_position,is_open,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1,unixepoch())
         ON CONFLICT(id) DO UPDATE SET file_path=excluded.file_path,title=excluded.title,
         content=excluded.content,dirty=excluded.dirty,cursor_offset=excluded.cursor_offset,
         scroll_top=excluded.scroll_top,tab_position=excluded.tab_position,is_open=1,updated_at=unixepoch()",
        params![document.id, document.file_path, document.title, document.content,
            document.dirty, document.cursor_offset, document.scroll_top, document.tab_position],
    )?;
    Ok(())
}

#[tauri::command]
fn restore_session(database: State<Database>) -> Result<Session, String> {
    let connection = database.0.lock().map_err(|e| e.to_string())?;
    let mut statement = connection.prepare(
        "SELECT id,file_path,title,content,dirty,cursor_offset,scroll_top,tab_position
         FROM documents WHERE is_open=1 ORDER BY tab_position"
    ).map_err(|e| e.to_string())?;
    let documents = statement.query_map([], |row| Ok(DocumentState {
        id: row.get(0)?, file_path: row.get(1)?, title: row.get(2)?, content: row.get(3)?,
        dirty: row.get(4)?, cursor_offset: row.get(5)?, scroll_top: row.get(6)?, tab_position: row.get(7)?,
    })).map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    let active_id = connection.query_row("SELECT value FROM app_state WHERE key='active_id'", [], |row| row.get(0)).ok();
    Ok(Session { documents, active_id })
}

#[tauri::command]
fn persist_session(database: State<Database>, documents: Vec<DocumentState>, active_id: String) -> Result<(), String> {
    let mut connection = database.0.lock().map_err(|e| e.to_string())?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    transaction.execute("UPDATE documents SET is_open=0", []).map_err(|e| e.to_string())?;
    for document in &documents { upsert(&transaction, document).map_err(|e| e.to_string())?; }
    transaction.execute("INSERT INTO app_state(key,value) VALUES('active_id',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&active_id]).map_err(|e| e.to_string())?;
    transaction.commit().map_err(|e| e.to_string())
}

#[tauri::command]
fn close_document(database: State<Database>, id: String, discard: bool) -> Result<(), String> {
    let connection = database.0.lock().map_err(|e| e.to_string())?;
    if discard { connection.execute("DELETE FROM documents WHERE id=?1", [&id]) }
    else { connection.execute("UPDATE documents SET is_open=0 WHERE id=?1", [&id]) }
        .map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_document(path: String) -> Result<DocumentState, String> {
    let content = fs::read_to_string(&path).map_err(|e| format!("Could not open file: {e}"))?;
    let title = Path::new(&path).file_name().and_then(|v| v.to_str()).unwrap_or("Untitled").to_string();
    Ok(DocumentState { id: uuid::Uuid::new_v4().to_string(), file_path: Some(path), title, content,
        dirty: false, cursor_offset: 0, scroll_top: 0.0, tab_position: 0 })
}

fn atomic_save(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary.write_all(content).and_then(|_| temporary.as_file().sync_all()).map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}

#[tauri::command]
fn save_document(database: State<Database>, mut document: DocumentState, path: String) -> Result<DocumentState, String> {
    atomic_save(Path::new(&path), document.content.as_bytes())?;
    document.title = Path::new(&path).file_name().and_then(|v| v.to_str()).unwrap_or("Untitled").to_string();
    document.file_path = Some(path); document.dirty = false;
    let connection = database.0.lock().map_err(|e| e.to_string())?;
    upsert(&connection, &document).map_err(|e| e.to_string())?;
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn database_round_trips_session() {
        let dir = tempfile::tempdir().unwrap();
        let connection = open_database(&dir.path().join("state.db")).unwrap();
        let doc = DocumentState { id: "one".into(), file_path: None, title: "Untitled".into(), content: "recovered".into(), dirty: true, cursor_offset: 4, scroll_top: 2.0, tab_position: 0 };
        upsert(&connection, &doc).unwrap();
        let restored: String = connection.query_row("SELECT content FROM documents WHERE id='one'", [], |row| row.get(0)).unwrap();
        assert_eq!(restored, "recovered");
    }
    #[test]
    fn atomic_save_replaces_content() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("note.txt");
        fs::write(&path, "old").unwrap(); atomic_save(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "new");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path: PathBuf = app.path().app_data_dir().map_err(|e| e.to_string())?.join("session.db");
            app.manage(Database(Mutex::new(open_database(&path)?)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![restore_session, persist_session, close_document, open_document, save_document])
        .run(tauri::generate_context!()).expect("error while running RustPad");
}
