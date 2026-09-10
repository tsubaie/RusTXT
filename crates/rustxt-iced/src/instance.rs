//! A per-session lock and atomic file inbox; works without a desktop bus.
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
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
    pub fn receive(&self) -> Result<Vec<Vec<PathBuf>>, String> {
        let mut messages = Vec::new();
        for entry in fs::read_dir(&self.inbox).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = fs::read(&path).map_err(|e| e.to_string())?;
            messages
                .push(serde_json::from_slice::<Vec<PathBuf>>(&content).map_err(|e| e.to_string())?);
            fs::remove_file(path).map_err(|e| e.to_string())?;
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
}
