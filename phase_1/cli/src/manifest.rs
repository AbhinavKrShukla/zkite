//! Hash-diff manifest for the dev-loop push (see ../../PLAN.md §4: "A hash manifest
//! means a one-character edit sends one file, not a tree.").
//!
//! Pure and host-testable: scanning and diffing need no device, no adb, no network.
//! What *does* need a device — pushing the diff over the `adb reverse` socket into a
//! running app — is `watch.rs`'s job downstream of this, and is M4/M5 territory.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// Directories never worth watching or pushing: VCS metadata, caches, venvs.
const IGNORED_DIR_NAMES: &[&str] = &[
    ".git",
    "__pycache__",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
    "node_modules",
    ".zkite",
    "dist",
    "build",
    "target",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// project-root-relative path (forward-slash separated, stable across OSes) -> hex sha256
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added(String),
    Modified(String),
    Removed(String),
}

impl Manifest {
    pub fn scan(root: &Path) -> Result<Manifest> {
        Self::scan_filtered(root, true)
    }

    /// Installed wheels may contain hidden resources or packages named `build`.
    pub fn scan_dependencies(root: &Path) -> Result<Manifest> {
        Self::scan_filtered(root, false)
    }

    fn scan_filtered(root: &Path, project: bool) -> Result<Manifest> {
        let mut files = BTreeMap::new();
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| !project || !is_ignored(e))
        {
            let entry = entry.with_context(|| format!("walking {}", root.display()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(root)
                .expect("walkdir entries are under root")
                .to_slash_lossy();
            let bytes = std::fs::read(entry.path())
                .with_context(|| format!("reading {}", entry.path().display()))?;
            files.insert(rel, hex_sha256(&bytes));
        }
        Ok(Manifest { files })
    }

    /// Changes needed to turn `self` (the side that already has files, e.g. the
    /// device's last-known state) into `other` (the fresh scan).
    pub fn diff(&self, other: &Manifest) -> Vec<Change> {
        let mut changes = Vec::new();
        for (path, hash) in &other.files {
            match self.files.get(path) {
                None => changes.push(Change::Added(path.clone())),
                Some(old_hash) if old_hash != hash => changes.push(Change::Modified(path.clone())),
                _ => {}
            }
        }
        for path in self.files.keys() {
            if !other.files.contains_key(path) {
                changes.push(Change::Removed(path.clone()));
            }
        }
        changes
    }
}

fn is_ignored(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return false; // never filter the scan root itself
    }
    let name = entry.file_name().to_string_lossy();
    IGNORED_DIR_NAMES.contains(&name.as_ref()) || name.starts_with('.')
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Small local helper so this module doesn't need a `path-slash` dependency for one
/// call site; project paths are always UTF-8 in practice (zkite doesn't support
/// non-UTF8 filenames — same assumption the rest of the toolchain makes).
trait ToSlashLossy {
    fn to_slash_lossy(&self) -> String;
}

impl ToSlashLossy for Path {
    fn to_slash_lossy(&self) -> String {
        self.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn scan_finds_all_files_with_stable_relative_paths() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.py"), b"print(1)").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/util.py"), b"x = 1").unwrap();

        let manifest = Manifest::scan(dir.path()).unwrap();
        assert_eq!(manifest.files.len(), 2);
        assert!(manifest.files.contains_key("main.py"));
        assert!(manifest.files.contains_key("sub/util.py"));
    }

    #[test]
    fn scan_ignores_dot_dirs_and_pycache() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.py"), b"print(1)").unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/HEAD"), b"ref: refs/heads/main").unwrap();
        fs::create_dir(dir.path().join("__pycache__")).unwrap();
        fs::write(dir.path().join("__pycache__/main.cpython-312.pyc"), b"junk").unwrap();

        let manifest = Manifest::scan(dir.path()).unwrap();
        assert_eq!(manifest.files.len(), 1);
        assert!(manifest.files.contains_key("main.py"));
    }

    #[test]
    fn standalone_build_output_is_never_pushed_back_into_the_dev_shell() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.py"), "pass").unwrap();
        fs::create_dir(dir.path().join("dist")).unwrap();
        fs::write(dir.path().join("dist/app-arm64-v8a.apk"), "APK bytes").unwrap();
        assert_eq!(Manifest::scan(dir.path()).unwrap().files.len(), 1);
    }

    #[test]
    fn identical_content_hashes_identically() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.py"), b"same").unwrap();
        fs::write(dir.path().join("b.py"), b"same").unwrap();
        let manifest = Manifest::scan(dir.path()).unwrap();
        assert_eq!(manifest.files["a.py"], manifest.files["b.py"]);
    }

    #[test]
    fn diff_detects_added_modified_and_removed() {
        let mut old = Manifest::default();
        old.files.insert("unchanged.py".into(), "h1".into());
        old.files.insert("edited.py".into(), "h2".into());
        old.files.insert("deleted.py".into(), "h3".into());

        let mut new = Manifest::default();
        new.files.insert("unchanged.py".into(), "h1".into());
        new.files.insert("edited.py".into(), "h2-new".into());
        new.files.insert("created.py".into(), "h4".into());

        let mut changes = old.diff(&new);
        changes.sort_by_key(|c| match c {
            Change::Added(p) | Change::Modified(p) | Change::Removed(p) => p.clone(),
        });

        // sorted by path (the test's own sort key above), not by variant
        assert_eq!(
            changes,
            vec![
                Change::Added("created.py".into()),
                Change::Removed("deleted.py".into()),
                Change::Modified("edited.py".into()),
            ]
        );
    }

    #[test]
    fn empty_diff_against_self_is_empty() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.py"), b"print(1)").unwrap();
        let manifest = Manifest::scan(dir.path()).unwrap();
        assert!(manifest.diff(&manifest).is_empty());
    }

    #[test]
    fn one_character_edit_produces_exactly_one_modified_change() {
        // The property plan1.md §6 cares about: "a one-character edit sends one
        // file, not a tree."
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.py"), b"n: int = 0").unwrap();
        fs::write(dir.path().join("other.py"), b"unchanged").unwrap();
        let before = Manifest::scan(dir.path()).unwrap();

        fs::write(dir.path().join("main.py"), b"n: int = 1").unwrap();
        let after = Manifest::scan(dir.path()).unwrap();

        let changes = before.diff(&after);
        assert_eq!(changes, vec![Change::Modified("main.py".into())]);
    }
}
