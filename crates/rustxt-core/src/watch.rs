//! Filesystem watching for live configuration and theme reloads. Tauri-free.

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, RecvTimeoutError},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

/// Time to wait after the last event before notifying, so a burst of writes
/// (Omarchy rewrites every themed file on a theme change) becomes one reload.
const QUIET_PERIOD: Duration = Duration::from_millis(200);

/// Keeps the watcher alive; drop it to stop watching.
pub struct ConfigWatcher {
    _watcher: Arc<Mutex<RecommendedWatcher>>,
}

/// Watch `directories` and call `on_change` once per burst of events. A
/// directory that does not exist yet (a themes/ folder the user has not
/// made) is not an error: its nearest existing ancestor is watched instead,
/// and the directory itself is picked up as soon as it appears.
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

    let mut watched: Vec<PathBuf> = Vec::new();
    let mut pending: Vec<PathBuf> = Vec::new();
    for directory in directories {
        if directory.is_dir() {
            add_watch(&mut watcher, &mut watched, &directory)?;
        } else {
            // Creation of the missing directory shows up as an event in its
            // closest existing ancestor.
            if let Some(ancestor) = existing_ancestor(&directory) {
                add_watch(&mut watcher, &mut watched, &ancestor)?;
            }
            pending.push(directory);
        }
    }

    let watcher = Arc::new(Mutex::new(watcher));
    let shared = Arc::clone(&watcher);
    thread::Builder::new()
        .name("rustxt-config-watch".into())
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
            if !pending.is_empty() {
                if let Ok(mut watcher) = shared.lock() {
                    pending.retain(|directory| {
                        !(directory.is_dir()
                            && add_watch(&mut watcher, &mut watched, directory).is_ok())
                    });
                }
            }
            on_change();
        })
        .map_err(|e| e.to_string())?;

    Ok(ConfigWatcher { _watcher: watcher })
}

fn add_watch(
    watcher: &mut RecommendedWatcher,
    watched: &mut Vec<PathBuf>,
    directory: &Path,
) -> Result<(), String> {
    if watched.iter().any(|known| known == directory) {
        return Ok(());
    }
    watcher
        .watch(directory, RecursiveMode::NonRecursive)
        .map_err(|e| format!("{}: {e}", directory.display()))?;
    watched.push(directory.to_path_buf());
    Ok(())
}

fn existing_ancestor(directory: &Path) -> Option<PathBuf> {
    directory
        .ancestors()
        .skip(1)
        .find(|ancestor| ancestor.is_dir())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };

    fn wait_for(hits: &AtomicUsize, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while hits.load(Ordering::SeqCst) < count && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(QUIET_PERIOD * 2);
    }

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
        wait_for(&hits, 1);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_directory_created_later_is_watched_once_it_appears() {
        let dir = tempfile::tempdir().unwrap();
        let themes = dir.path().join("rustxt/themes");
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let _watcher = watch(vec![themes.clone()], move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        // Creating the directory is noticed through its ancestor...
        fs::create_dir_all(&themes).unwrap();
        wait_for(&hits, 1);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // ...after which writes inside it are noticed directly, even though
        // the ancestor watch is not recursive.
        fs::write(themes.join("solar.toml"), "x").unwrap();
        wait_for(&hits, 2);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }
}
