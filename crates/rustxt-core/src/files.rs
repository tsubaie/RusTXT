//! Reading and atomically writing documents on disk. Tauri-free.

use crate::storage::{DocumentState, LineEnding};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Files larger than this are refused. Documents are held in memory whole
/// and copied into the recovery database, and a text file this size is not
/// something a notepad can usefully edit.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub fn content_fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn disk_fingerprint(path: &Path) -> Result<String, String> {
    read_regular_file(path)
        .map(|bytes| content_fingerprint(&bytes))
        .map_err(|reason| format!("Could not inspect file: {reason}"))
}

/// Read a user-supplied path, refusing anything that is not a regular file
/// of a sensible size. Directories, FIFOs, devices and sockets would hang
/// the editor or yield nothing usable, and a huge file would exhaust memory.
/// The error is the reason alone, for callers to prefix.
fn read_regular_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    let kind = metadata.file_type();
    if kind.is_dir() {
        return Err(format!("{} is a directory", path.display()));
    }
    if !kind.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(format!("{} is larger than 64 MiB", path.display()));
    }
    fs::read(path).map_err(|error| error.to_string())
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

/// Convert any line-break style to `\n` for in-memory editing: Windows
/// `\r\n` and the lone `\r` of classic Mac files alike. A UTF-8 byte order
/// mark is not a line break and is left in place as U+FEFF, so a file that
/// starts with one is written back byte for byte.
pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Re-apply the document's line-ending style for writing to disk.
pub fn encode(content: &str, line_ending: LineEnding) -> String {
    match line_ending {
        LineEnding::Lf => content.to_string(),
        LineEnding::Crlf => content.replace('\n', "\r\n"),
    }
}

pub fn read_document(path: &str) -> Result<DocumentState, String> {
    let bytes = read_regular_file(Path::new(path))
        .map_err(|reason| format!("Could not open file: {reason}"))?;
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
///
/// Any other trouble reading a file (permissions, a file grown past the
/// size limit, a path that is now a directory) keeps the document with
/// whatever it had, so a passing error never loses a tab. Every document
/// that could not be reloaded is described in the returned list, one line
/// each, for the frontend to show; an empty list means all is well.
pub fn refresh_from_disk(documents: &mut Vec<DocumentState>) -> Vec<String> {
    let mut problems = Vec::new();
    documents.retain_mut(|document| {
        if document.dirty {
            return true;
        }
        let Some(path) = document.file_path.as_deref() else {
            return true;
        };
        let loaded = read_regular_file(Path::new(path))
            .and_then(|bytes| String::from_utf8(bytes).map_err(|error| error.to_string()));
        match loaded {
            Ok(raw) => {
                document.line_ending = LineEnding::detect(&raw);
                document.content = normalize(&raw);
                document.disk_fingerprint = Some(content_fingerprint(raw.as_bytes()));
                true
            }
            Err(reason) => {
                let gone = matches!(fs::symlink_metadata(path),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound);
                let title = &document.title;
                if gone && document.content.is_empty() {
                    problems.push(format!("{title} is no longer on disk and was closed."));
                    return false;
                }
                if document.content.is_empty() {
                    problems.push(format!(
                        "{title} could not be read, so its tab is empty for now: {reason}"
                    ));
                } else {
                    document.dirty = true;
                    problems.push(format!(
                        "{title} could not be read; showing its last recovered text: {reason}"
                    ));
                }
                true
            }
        }
    });
    problems
}

/// Write through a temporary file in the same directory, then rename it over
/// the target. Follows symlinks so the linked file is updated rather than the
/// link replaced, and preserves the existing file's permissions. A new file
/// gets the usual mode for a freshly created file, 0666 less the umask.
pub fn atomic_save(path: &Path, content: &[u8]) -> Result<(), String> {
    let target = resolve_symlink(path);
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let existing_permissions = fs::metadata(&target).ok().map(|meta| meta.permissions());

    // Only Unix adjusts the builder; keep the binding plain elsewhere.
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        // Temporary files default to 0600, which is right for secrets but
        // would leave every document the user creates private to them.
        // Asking for 0666 lets the umask decide, as any other editor does.
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut temporary = builder.tempfile_in(parent).map_err(|e| e.to_string())?;
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
    #[cfg(unix)]
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
    fn atomic_save_gives_a_new_file_the_usual_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let saved = dir.path().join("saved.txt");
        atomic_save(&saved, b"hello").unwrap();
        // File::create asks for 0666 and lets the umask trim it, so its
        // result is what the user's other tools would produce here.
        let reference = dir.path().join("reference.txt");
        fs::File::create(&reference).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&saved), mode(&reference));
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
    fn read_document_normalizes_lone_cr_and_mixed_endings() {
        let dir = tempfile::tempdir().unwrap();
        let mac = dir.path().join("mac.txt");
        fs::write(&mac, "a\rb\r").unwrap();
        let doc = read_document(mac.to_str().unwrap()).unwrap();
        assert_eq!(doc.line_ending, LineEnding::Lf, "lone CR files become LF");
        assert_eq!(doc.content, "a\nb\n");

        let mixed = dir.path().join("mixed.txt");
        fs::write(&mixed, "a\r\nb\rc\nd").unwrap();
        let doc = read_document(mixed.to_str().unwrap()).unwrap();
        assert_eq!(doc.line_ending, LineEnding::Crlf, "any CRLF makes it CRLF");
        assert_eq!(doc.content, "a\nb\nc\nd");
        assert_eq!(
            encode(&doc.content, doc.line_ending),
            "a\r\nb\r\nc\r\nd",
            "mixed files are written back uniformly"
        );
    }

    #[test]
    fn utf8_bom_round_trips_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bom.txt");
        fs::write(&path, "\u{FEFF}hello\r\n").unwrap();
        let mut doc = read_document(path.to_str().unwrap()).unwrap();
        assert!(
            doc.content.starts_with('\u{FEFF}'),
            "the BOM stays in the text"
        );
        doc.content.push_str("more\n");
        save_document(&mut doc, &path).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            "\u{FEFF}hello\r\nmore\r\n".as_bytes()
        );
    }

    #[test]
    fn read_document_rejects_directories() {
        let dir = tempfile::tempdir().unwrap();
        let error = read_document(dir.path().to_str().unwrap()).unwrap_err();
        assert!(error.starts_with("Could not open file: "), "{error}");
        assert!(error.ends_with("is a directory"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn read_document_rejects_devices_and_sockets() {
        let error = read_document("/dev/null").unwrap_err();
        assert!(
            error.ends_with("/dev/null is not a regular file"),
            "{error}"
        );
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("editor.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let error = read_document(socket.to_str().unwrap()).unwrap_err();
        assert!(error.ends_with("is not a regular file"), "{error}");
        assert!(disk_fingerprint(&socket).is_err());
    }

    #[test]
    fn read_document_rejects_files_over_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let huge = dir.path().join("huge.txt");
        // A sparse file: the size is what matters, not the bytes behind it.
        fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        let error = read_document(huge.to_str().unwrap()).unwrap_err();
        assert!(error.ends_with("is larger than 64 MiB"), "{error}");
        let limit = dir.path().join("limit.txt");
        fs::File::create(&limit)
            .unwrap()
            .set_len(MAX_FILE_BYTES)
            .unwrap();
        assert!(read_document(limit.to_str().unwrap()).is_ok());
    }

    #[test]
    fn refresh_keeps_and_reports_documents_whose_files_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let huge = dir.path().join("huge.txt");
        fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        let clean = |id: &str, path: &Path, content: &str| DocumentState {
            id: id.into(),
            file_path: Some(path.to_string_lossy().into_owned()),
            title: title_for(path),
            content: content.into(),
            dirty: false,
            cursor_offset: 0,
            scroll_top: 0.0,
            tab_position: 0,
            line_ending: LineEnding::Lf,
            disk_fingerprint: None,
        };
        let mut docs = vec![
            clean("textless-huge", &huge, ""),
            clean("directory", dir.path(), "old text"),
        ];
        let problems = refresh_from_disk(&mut docs);
        assert_eq!(docs.len(), 2, "unreadable files never drop a tab");
        assert_eq!(docs[0].content, "");
        assert!(!docs[0].dirty, "nothing to save, so not marked unsaved");
        assert_eq!(docs[1].content, "old text");
        assert!(docs[1].dirty, "recovered text is flagged as not on disk");
        assert_eq!(problems.len(), 2);
        assert!(
            problems[0].contains("huge.txt") && problems[0].contains("64 MiB"),
            "{problems:?}"
        );
        assert!(problems[1].contains("is a directory"), "{problems:?}");
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
        let problems = refresh_from_disk(&mut docs);
        assert_eq!(
            docs.len(),
            3,
            "a missing file with no stored text is dropped"
        );
        assert_eq!(
            problems.len(),
            2,
            "both missing files are reported: {problems:?}"
        );
        assert!(
            problems[0].starts_with("gone.txt could not be read"),
            "{problems:?}"
        );
        assert!(
            problems[1].starts_with("gone-too.txt is no longer on disk"),
            "{problems:?}"
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
