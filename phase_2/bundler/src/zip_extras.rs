//! Shared by `apk.rs` and `aab.rs`: reading a staging directory into verbatim zip
//! entries, and rejecting paths that could escape the archive on extraction.
//! Promoted unchanged (in substance) from `phase_0/spike2_pack_apk/src/apk.rs`'s
//! `validate_entry_path`/`read_staging_dir`/`read_res_dir` — see `../PLAN.md` §1.1.

use std::path::Path;

use pack_asset_compiler::resource_internal_types::FileResource;

/// A file to embed in the archive exactly as-is.
pub struct RawEntry {
    /// Path inside the zip, e.g. `classes.dex` or `lib/arm64-v8a/libzkite.so`.
    pub path: String,
    pub data: Vec<u8>,
}

/// Reject zip entry paths that could escape the archive on extraction (zip-slip) or
/// silently clobber another entry. Real input here is a wheel/dependency tree
/// (`../PLAN.md`'s wheel resolution, M5) plus our own staged files, so this is a
/// trust boundary, not a formality.
pub fn validate_entry_path(path: &str, seen: &mut std::collections::HashSet<String>) {
    assert!(!path.is_empty(), "zip entry path is empty");
    assert!(!path.starts_with('/'), "zip entry path {path:?} is absolute");
    assert!(
        !path.contains(['\\', ':', '\0']),
        "zip entry path {path:?} contains a Windows path separator, drive prefix, or NUL"
    );
    assert!(
        !path.split('/').any(|c| c == ".." || c == "."),
        "zip entry path {path:?} contains a `.` or `..` component"
    );
    assert!(seen.insert(path.to_string()), "duplicate zip entry path {path:?}");
}

/// Collect a staging directory into verbatim zip entries, preserving relative paths.
/// Deterministic: same inputs must produce the same archive bytes.
pub fn read_staging_dir(root: &Path) -> std::io::Result<Vec<RawEntry>> {
    let mut out = vec![];
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<RawEntry>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push(RawEntry { path: rel, data: std::fs::read(&path)? });
        }
    }
    Ok(())
}

/// Read a `res/` tree into `FileResource`s. `pack`'s compatibility table says
/// qualifiers (`drawable-xhdpi/`, `values-es/`) are unsupported, so this rejects
/// them loudly instead of silently producing an archive where half the resources
/// vanished (plan1.md §7's "diagnosable rather than mysterious") — same rationale
/// and same code as phase 0's `apk.rs::read_res_dir`.
pub fn read_res_dir(res_path: &Path) -> std::io::Result<Vec<FileResource>> {
    let mut out = vec![];
    for type_dir in std::fs::read_dir(res_path)? {
        let type_dir = type_dir?;
        if !type_dir.file_type()?.is_dir() {
            continue;
        }
        let name = type_dir.file_name().to_string_lossy().into_owned();
        assert!(
            !name.contains('-'),
            "res/{name}: pack does not support resource qualifiers yet (see phase_0/README.md)"
        );
        for file in std::fs::read_dir(type_dir.path())? {
            let file = file?;
            if file.file_type()?.is_dir() {
                continue;
            }
            out.push(FileResource {
                subdirectory: name.clone(),
                name: file.file_name().to_string_lossy().into_owned(),
                resource_id: 0,
                contents: std::fs::read(file.path())?,
            });
        }
    }
    Ok(out)
}
