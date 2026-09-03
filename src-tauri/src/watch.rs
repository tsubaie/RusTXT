//! Filesystem watching for live configuration and theme reloads. Tauri-free.

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::PathBuf,
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

/// Time to wait after the last event before notifying, so a burst of writes
/// (Omarchy rewrites every themed file on a theme change) becomes one reload.
const QUIET_PERIOD: Duration = Duration::from_millis(200);

/// Keeps the watcher alive; drop it to stop watching.
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
}

pub fn watch(
    directories: Vec<PathBuf>,
    on_change: impl Fn() + Send + 'static,
) -> Result<ConfigWatcher, String> {
    let (sender, receiver) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if result.is_ok() {
            let _ = sender.send(());
        }
    })
    .map_err(|e| e.to_string())?;

    for directory in &directories {
        watcher
            .watch(directory, RecursiveMode::NonRecursive)
            .map_err(|e| format!("{}: {e}", directory.display()))?;
    }

    thread::Builder::new()
        .name("rustpad-config-watch".into())
        .spawn(move || loop {
            // Block for the first event, then absorb the burst that follows it.
            if receiver.recv().is_err() {
                return;
            }
            loop {
                match receiver.recv_timeout(QUIET_PERIOD) {
                    Ok(()) => continue,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            on_change();
        })
        .map_err(|e| e.to_string())?;

    Ok(ConfigWatcher { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    #[test]
    fn coalesces_bursts_into_one_notification() {
        let dir = tempfile::tempdir().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let _watcher = watch(vec![dir.path().to_path_buf()], move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        for index in 0..5 {
            fs::write(dir.path().join(format!("file{index}.toml")), "x").unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while hits.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(QUIET_PERIOD * 2);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
