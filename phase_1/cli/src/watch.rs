//! Filesystem watcher: turns raw `notify` events into debounced [`Change`] batches.
//!
//! This half is genuinely host-testable — watching a local directory needs no
//! device. What's *not* here, and is M4/M5 (see ../../PLAN.md): pushing those
//! changes down the `adb reverse tcp:8765` socket into a running app, and the
//! device-side listener that receives them. `zkite dev` (main.rs) wires this watcher
//! to that transport once M4 exists.

use crate::manifest::{Change, Manifest};
use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::Duration;

/// How long to wait after the last filesystem event before re-scanning. Editors
/// often emit several events for one save (write-then-rename, temp-file swaps); this
/// coalesces them into a single manifest diff instead of firing once per event.
const DEBOUNCE: Duration = Duration::from_millis(80);

pub struct ProjectWatcher {
    root: PathBuf,
    last: Manifest,
    _watcher: notify::RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
}

impl ProjectWatcher {
    pub fn new(root: &Path) -> Result<Self> {
        let last = Manifest::scan(root).with_context(|| "initial manifest scan")?;
        let (tx, rx) = channel();
        let mut watcher =
            notify::recommended_watcher(tx).context("constructing filesystem watcher")?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", root.display()))?;
        Ok(ProjectWatcher {
            root: root.to_path_buf(),
            last,
            _watcher: watcher,
            events: rx,
        })
    }

    /// Blocks until at least one filesystem event arrives, debounces, then returns
    /// the diff since the last call. Empty on spurious wakeups (e.g. a write that
    /// reproduced identical bytes).
    pub fn next_change_batch(&mut self, timeout: Duration) -> Result<Vec<Change>> {
        // Block for the first event.
        match self.events.recv_timeout(timeout) {
            Ok(event) => { event.context("watching project files")?; }
            Err(RecvTimeoutError::Timeout) => return Ok(Vec::new()),
            Err(RecvTimeoutError::Disconnected) => anyhow::bail!("project filesystem watcher disconnected"),
        }
        // Drain anything else that arrives within the debounce window.
        loop {
            match self.events.recv_timeout(DEBOUNCE) {
                Ok(event) => { event.context("watching project files")?; },
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let fresh = Manifest::scan(&self.root)?;
        let changes = self.last.diff(&fresh);
        self.last = fresh;
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;
    use tempfile::tempdir;

    #[test]
    fn detects_a_single_file_edit() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.py"), b"n = 0").unwrap();

        let mut watcher = ProjectWatcher::new(dir.path()).unwrap();

        // give the OS watcher a moment to arm before we touch the file
        std::thread::sleep(Duration::from_millis(150));
        fs::write(dir.path().join("main.py"), b"n = 1").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut changes = Vec::new();
        while Instant::now() < deadline {
            let batch = watcher.next_change_batch(Duration::from_millis(250)).unwrap();
            if !batch.is_empty() {
                changes = batch;
                break;
            }
        }
        assert_eq!(changes, vec![Change::Modified("main.py".into())]);
    }

    #[test]
    fn detects_a_new_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.py"), b"n = 0").unwrap();
        let mut watcher = ProjectWatcher::new(dir.path()).unwrap();

        std::thread::sleep(Duration::from_millis(150));
        fs::write(dir.path().join("widgets.py"), b"# extra").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut changes = Vec::new();
        while Instant::now() < deadline {
            let batch = watcher.next_change_batch(Duration::from_millis(250)).unwrap();
            if !batch.is_empty() {
                changes = batch;
                break;
            }
        }
        assert_eq!(changes, vec![Change::Added("widgets.py".into())]);
    }
}
