//! Reading and atomically writing documents on disk. Tauri-free.

use crate::storage::{DocumentState, LineEnding};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn content_fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn disk_fingerprint(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|bytes| content_fingerprint(&bytes))
        .map_err(|error| format!("Could not inspect file: {error}"))
}

pub fn changed_on_disk(state: &DocumentState, path: &Path) -> Result<bool, String> {
    disk_fingerprint(path).map(|current| state.disk_fingerprint.as_deref() != Some(&current))
}

pub fn same_file(left: &Path, right: &Path) -> bool {
    matches!(
        (fs::canonicalize(left), fs::canonicalize(right)),
        (Ok(left), Ok(right)) if left == right
    )
}

pub fn title_for(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .to_string()
}

/// Convert any line-break style to `\n` for in-memory editing.
pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Re-apply the document's line-ending style for writing to disk.
pub fn encode(content: &str, line_ending: LineEnding) -> String {
    match line_ending {
        LineEnding::Lf => content.to_string(),
        LineEnding::Crlf => content.replace('\n', "\r\n"),
    }
}

pub fn read_document(path: &str) -> Result<DocumentState, String> {
    let bytes = fs::read(path).map_err(|error| format!("Could not open file: {error}"))?;
    let disk_fingerprint = Some(content_fingerprint(&bytes));
    let raw = String::from_utf8(bytes).map_err(|error| format!("Could not open file: {error}"))?;
    Ok(DocumentState {
        id: uuid::Uuid::new_v4().to_string(),
        title: title_for(Path::new(path)),
        file_path: Some(path.to_string()),
        line_ending: LineEnding::detect(&raw),
        disk_fingerprint,
        content: normalize(&raw),
        dirty: false,
        cursor_offset: 0,
        scroll_top: 0.0,
        tab_position: 0,
    })
}

/// Write `state.content` to `path` with the document's line endings and
/// update the state to point at the saved file.
pub fn save_document(state: &mut DocumentState, path: &Path) -> Result<(), String> {
    let bytes = encode(&state.content, state.line_ending);
    atomic_save(path, bytes.as_bytes())?;
    state.title = title_for(path);
    state.file_path = Some(path.to_string_lossy().into_owned());
    state.disk_fingerprint = Some(content_fingerprint(bytes.as_bytes()));
    state.dirty = false;
    Ok(())
}

/// Clean file-backed documents carry no snapshot text, because the file is
/// authoritative, so reload them from disk. One whose file has gone missing
/// is dropped from the list when there is nothing to show. Databases written
/// before snapshots went text-free may still hold its text; that is kept and
/// marked dirty so the user knows it is no longer on disk.
pub fn refresh_from_disk(documents: &mut Vec<DocumentState>) {
    documents.retain_mut(|document| {
        if document.dirty {
            return true;
        }
        let Some(path) = document.file_path.as_deref() else {
            return true;
        };
        match fs::read_to_string(path) {
            Ok(raw) => {
                document.line_ending = LineEnding::detect(&raw);
                document.content = normalize(&raw);
                document.disk_fingerprint = Some(content_fingerprint(raw.as_bytes()));
                true
            }
            Err(_) if document.content.is_empty() => false,
            Err(_) => {
                document.dirty = true;
                true
            }
        }
    });
}

/// Write through a temporary file in the same directory, then rename it over
/// the target. Follows symlinks so the linked file is updated rather than the
/// link replaced, and preserves the existing file's permissions.
pub fn atomic_save(path: &Path, content: &[u8]) -> Result<(), String> {
    let target = resolve_symlink(path);
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let existing_permissions = fs::metadata(&target).ok().map(|meta| meta.permissions());

    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary.write_all(content).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    if let Some(permissions) = existing_permissions {
        temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|e| e.to_string())?;
    }
    temporary
        .persist(&target)
        .map_err(|e| e.error.to_string())?;
    // Make the rename durable as well as atomic. Without syncing the directory,
    // a power loss can forget the new directory entry even after the file sync.
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn resolve_symlink(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_save_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        fs::write(&path, "old").unwrap();
        atomic_save(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "new");
    }

    #[test]
    fn atomic_save_creates_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.txt");
        atomic_save(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "hello");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.sh");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        atomic_save(&path, b"new").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_writes_through_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.txt");
        let link = dir.path().join("link.txt");
        fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        atomic_save(&link, b"new").unwrap();
        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn equivalent_paths_identify_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        fs::write(&path, "hello").unwrap();
        assert!(same_file(&path, &dir.path().join("./note.txt")));
        assert!(!same_file(
            &dir.path().join("missing-a"),
            &dir.path().join("missing-b")
        ));
    }

    #[test]
    fn detects_a_file_changed_after_it_was_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        fs::write(&path, "original").unwrap();
        let state = read_document(path.to_str().unwrap()).unwrap();
        assert!(!changed_on_disk(&state, &path).unwrap());
        fs::write(&path, "external edit").unwrap();
        assert!(changed_on_disk(&state, &path).unwrap());
    }

    #[test]
    fn save_document_encodes_line_endings_and_updates_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        let mut state = DocumentState::untitled(std::iter::empty(), LineEnding::Crlf);
        state.content = "one\ntwo".into();
        state.dirty = true;
        save_document(&mut state, &path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one\r\ntwo");
        assert_eq!(state.title, "notes.txt");
        assert_eq!(state.file_path.as_deref(), Some(path.to_str().unwrap()));
        assert!(!state.dirty);
    }

    #[test]
    fn read_document_detects_and_normalizes_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("win.txt");
        fs::write(&path, "a\r\nb\r\n").unwrap();
        let doc = read_document(path.to_str().unwrap()).unwrap();
        assert_eq!(doc.line_ending, LineEnding::Crlf);
        assert_eq!(doc.content, "a\nb\n");
        assert_eq!(encode(&doc.content, doc.line_ending), "a\r\nb\r\n");
    }

    #[test]
    fn refresh_reloads_clean_documents_and_flags_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present.txt");
        fs::write(&present, "on disk").unwrap();
        let base = |id: &str, path: &Path, dirty: bool| DocumentState {
            id: id.into(),
            file_path: Some(path.to_string_lossy().into_owned()),
            title: title_for(path),
            content: "snapshot".into(),
            dirty,
            cursor_offset: 0,
            scroll_top: 0.0,
            tab_position: 0,
            line_ending: LineEnding::Lf,
            disk_fingerprint: None,
        };
        let mut docs = vec![
            base("clean", &present, false),
            base("dirty", &present, true),
            base("missing", &dir.path().join("gone.txt"), false),
            DocumentState {
                content: String::new(),
                ..base("textless", &dir.path().join("gone-too.txt"), false)
            },
        ];
        refresh_from_disk(&mut docs);
        assert_eq!(
            docs.len(),
            3,
            "a missing file with no stored text is dropped"
        );
        assert_eq!(docs[0].content, "on disk");
        assert_eq!(
            docs[1].content, "snapshot",
            "dirty snapshots are never overwritten"
        );
        assert!(
            docs[2].dirty,
            "missing file leaves the snapshot marked unsaved"
        );
        assert_eq!(docs[2].content, "snapshot");
    }
}
