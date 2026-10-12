//! Reconcile only files delivered by the development server. The application's
//! own databases, downloads, and other runtime-created files remain untouched.
//! Shared with the native shell and tested by the host CLI suite.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const RECORD: &str = ".zkite-managed-files.json";
static NEXT_RECORD: AtomicU64 = AtomicU64::new(0);

pub fn destination(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty() || relative.contains(['\\', ':']) {
        return Err(format!("invalid managed path: {relative}"));
    }
    let mut destination = root.to_path_buf();
    for part in Path::new(relative).components() {
        let Component::Normal(name) = part else {
            return Err(format!("refusing path outside the app directory: {relative}"));
        };
        if name.to_string_lossy().starts_with('.') {
            return Err(format!("refusing a hidden managed path: {relative}"));
        }
        destination.push(name);
        match std::fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("refusing a symlink in managed path: {relative}"));
            }
            Ok(_) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(destination)
}

pub fn sync(root: &Path, paths: &[String]) -> Result<(), String> {
    let next: BTreeSet<String> = paths.iter().cloned().collect();
    // Validate the entire incoming list before deleting any previous source.
    for path in &next {
        destination(root, path)?;
    }
    let record = root.join(RECORD);
    if std::fs::symlink_metadata(&record).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err("managed-file record is a symlink".into());
    }
    let previous: BTreeSet<String> = match std::fs::read(&record) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| format!("invalid managed-file record: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(error) => return Err(error.to_string()),
    };
    let removed = previous.difference(&next)
        .map(|path| destination(root, path))
        .collect::<Result<Vec<_>, _>>()?;
    for path in removed {
        match std::fs::remove_file(&path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(format!("removing managed file {}: {error}", path.display())),
        }
        // Empty package directories can disappear; runtime data prevents removal.
        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|directory| *directory != root) {
            if std::fs::remove_dir(directory).is_err() {
                break;
            }
            parent = directory.parent();
        }
    }
    std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(&next).map_err(|error| error.to_string())?;
    // Atomic publication prevents a disconnect/process death from leaving a
    // truncated manifest that blocks the next development session. create_new
    // refuses a preexisting file or symlink at the temporary name.
    let temporary = root.join(format!("{RECORD}.{}.{}.tmp", std::process::id(), NEXT_RECORD.fetch_add(1, Ordering::Relaxed)));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)
        .map_err(|error| error.to_string())?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, &record)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}
