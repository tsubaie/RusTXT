//! A per-session lock and atomic file inbox; works without a desktop bus.
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Instance {
    _lock: File,
    inbox: PathBuf,
}
impl Instance {
    pub fn acquire(data: &Path, paths: &[PathBuf]) -> Result<Option<Self>, String> {
        fs::create_dir_all(data).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(data.join("iced.lock"))
            .map_err(|e| e.to_string())?;
        let inbox = data.join("iced-inbox");
        fs::create_dir_all(&inbox).map_err(|e| e.to_string())?;
        match lock.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _lock: lock, inbox })),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos();
                let target = inbox.join(format!("{}-{stamp}.json", std::process::id()));
                let paths: Vec<_> = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                rustxt_core::files::atomic_save(
                    &target,
                    serde_json::to_string(&paths)
                        .map_err(|e| e.to_string())?
                        .as_bytes(),
                )?;
                Ok(None)
            }
            Err(error) => Err(error.to_string()),
        }
    }
    /// Collect messages left by later launches. A file that cannot be read or
    /// parsed is set aside as `.bad` (or removed) rather than reported on
    /// every poll, and a missing inbox directory is recreated.
    pub fn receive(&self) -> Result<Vec<Vec<PathBuf>>, String> {
        let mut messages = Vec::new();
        let entries = match fs::read_dir(&self.inbox) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir_all(&self.inbox).map_err(|e| e.to_string())?;
                return Ok(messages);
            }
            Err(error) => return Err(error.to_string()),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let parsed = fs::read(&path)
                .ok()
                .and_then(|content| serde_json::from_slice::<Vec<PathBuf>>(&content).ok());
            match parsed {
                Some(message) => {
                    fs::remove_file(&path).map_err(|e| e.to_string())?;
                    messages.push(message);
                }
                None => {
                    if fs::rename(&path, path.with_extension("bad")).is_err() {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_launch_forwards_paths_and_lock_is_released_on_exit() {
        let dir = tempfile::tempdir().unwrap();
        let first = Instance::acquire(dir.path(), &[]).unwrap().unwrap();
        let paths = vec![dir.path().join("مرحبا world.txt")];
        assert!(Instance::acquire(dir.path(), &paths).unwrap().is_none());
        assert_eq!(first.receive().unwrap(), vec![paths]);
        assert!(first.receive().unwrap().is_empty());
        drop(first);
        assert!(Instance::acquire(dir.path(), &[]).unwrap().is_some());
    }
    #[test]
    fn unparsable_inbox_files_are_set_aside_and_a_missing_inbox_is_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let instance = Instance::acquire(dir.path(), &[]).unwrap().unwrap();
        let inbox = dir.path().join("iced-inbox");
        fs::write(inbox.join("broken.json"), b"{not json").unwrap();
        fs::write(inbox.join("notes.txt"), b"ignored").unwrap();
        let good = vec![dir.path().join("ok.txt")];
        fs::write(
            inbox.join("good.json"),
            serde_json::to_vec(&[good[0].to_string_lossy()]).unwrap(),
        )
        .unwrap();
        assert_eq!(instance.receive().unwrap(), vec![good]);
        assert!(!inbox.join("broken.json").exists());
        assert!(inbox.join("broken.bad").exists());
        assert!(inbox.join("notes.txt").exists());
        assert!(instance.receive().unwrap().is_empty());
        fs::remove_dir_all(&inbox).unwrap();
        assert!(instance.receive().unwrap().is_empty());
        assert!(inbox.is_dir());
        assert!(instance.receive().unwrap().is_empty());
    }
}
