//! `pack` builds APKs out of a manifest plus a `res/` tree, and nothing else — it was
//! written for Wear OS watch faces, which contain no code. A real app APK also needs
//! `classes.dex`, `lib/<abi>/*.so` and `assets/`.
//!
//! This module is the smallest possible addition that gets us there: it reproduces
//! `pack_api::compile_apk` and splices extra, byte-verbatim zip entries in before the
//! zip is written and signed. Every piece of format knowledge — AXML, ARSC, alignment,
//! v2/v3 signing — still comes from `pack`.
//!
//! Upstream shape: this is one `extra_files: Vec<pack_zip::File>` field on
//! `pack_api::Package`. That is the PR plan1.md §11 argues for ("contribute rather than
//! fork"), and it would delete this file.

use std::io::Cursor;
use std::path::Path;

use deku::DekuContainerWrite;
use pack_asset_compiler::{
    resource_external_types::ResChunk,
    resource_internal_types::{FileResource, Resource},
    resource_table::construct_resource_table, strings_xml_parser::parse_strings_xml,
    xml_file::xml_to_res_chunk,
};
use pack_common::{PackError, Result};
use pack_sign::crypto_keys::Keys;

/// The inputs `pack` already understands: a manifest and a flat `res/` tree.
/// (`pack_api::Package` verbatim — redeclared here only to avoid depending on that
/// crate, which drags in the AAB path and its `protoc` requirement.)
pub struct Package {
    pub android_manifest: Vec<u8>,
    pub resources: Vec<FileResource>,
}

/// A file to embed in the APK exactly as-is.
pub struct RawEntry {
    /// Path inside the zip, e.g. `classes.dex` or `lib/arm64-v8a/libzkite.so`.
    pub path: String,
    pub data: Vec<u8>,
}

/// Compile + sign an APK from a manifest, a `res/` tree, and verbatim extra entries.
pub fn compile_and_sign_apk_with_extras(
    package: &Package,
    extras: Vec<RawEntry>,
    keys: &Keys,
) -> Result<Vec<u8>> {
    let mut zip_buf = compile_apk_with_extras(package, extras)?;
    pack_sign::sign_apk_buffer(&mut zip_buf, keys)
}

/// Reject zip entry paths that could escape the archive on extraction (zip-slip) or
/// silently clobber another entry. `walk()` only ever sees a locally-built staging
/// directory today, so nothing here can trigger it yet — but this function is the
/// trust boundary the phase-2 bundler will call with real wheel/dependency trees, and
/// that's exactly the input this needs to reject loudly rather than pass through.
fn validate_entry_path(path: &str, seen: &mut std::collections::HashSet<String>) {
    assert!(!path.is_empty(), "zip entry path is empty");
    assert!(!path.starts_with('/'), "zip entry path {path:?} is absolute");
    assert!(
        !path.split('/').any(|c| c == ".." || c == "."),
        "zip entry path {path:?} contains a `.` or `..` component"
    );
    assert!(seen.insert(path.to_string()), "duplicate zip entry path {path:?}");
}

fn compile_apk_with_extras(package: &Package, extras: Vec<RawEntry>) -> Result<Vec<u8>> {
    let mut seen_paths = std::collections::HashSet::new();
    // ---- identical to pack_api::compile_apk from here ...
    let mut resources = vec![];
    for res in &package.resources {
        if res.subdirectory == "values" && res.name == "strings.xml" {
            let mut cur = Cursor::new(&res.contents);
            resources.extend(parse_strings_xml(&mut cur));
        } else {
            resources.push(Resource::File(res.clone()));
        }
    }
    resources.sort_by(|a, b| a.get_subdirectory().cmp(b.get_subdirectory()));

    let manifest_cursor = Cursor::new(&package.android_manifest);
    let mut reader = std::io::BufReader::new(manifest_cursor);
    let (manifest_chunk, manifest_info) = xml_to_res_chunk(&mut reader, &resources)?;
    let package_name = manifest_info
        .package_name
        .ok_or(PackError::ManifestDoesNotHavePackageName)?;

    let mut apk_files: Vec<pack_zip::File> = vec![];
    apk_files.push(chunk_file("AndroidManifest.xml", &manifest_chunk)?);

    let table = construct_resource_table(&package_name, &mut resources)?;
    apk_files.push(chunk_file("resources.arsc", &table)?);

    for res in &resources {
        if let Resource::File(file) = res {
            apk_files.push(pack_zip::File {
                path: format!("res/{}/{}", file.subdirectory, file.name),
                data: file.as_bytes_for_apk(&resources)?,
            });
        }
    }
    // ... to here. ----

    // pack's own entries are internally generated, not attacker-controlled — track their
    // paths only for duplicate detection, don't re-validate their shape.
    for f in &apk_files {
        seen_paths.insert(f.path.clone());
    }

    // The addition: dex, native libs and assets, byte for byte. Unlike the block above,
    // this is exactly the input phase 2's bundler will fill with real wheel contents, so
    // every entry is validated.
    for e in extras {
        validate_entry_path(&e.path, &mut seen_paths);
        apk_files.push(pack_zip::File { path: e.path, data: e.data });
    }

    let mut zip_buf = vec![];
    pack_zip::zip_apk(&apk_files, Cursor::new(&mut zip_buf))?;
    Ok(zip_buf)
}

fn chunk_file(path: &str, chunk: &ResChunk) -> Result<pack_zip::File> {
    Ok(pack_zip::File { path: path.to_string(), data: chunk.to_bytes()? })
}

/// Read a `res/` tree into `FileResource`s.
///
/// `pack`'s compatibility table says qualifiers (`drawable-xhdpi/`, `values-es/`) are
/// unsupported, so this rejects them loudly instead of silently producing an APK where
/// half the resources vanished — plan1.md §7's "diagnosable rather than mysterious".
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
            "res/{name}: pack does not support resource qualifiers yet (see README)"
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

/// Collect a staging directory into verbatim zip entries, preserving relative paths.
pub fn read_staging_dir(root: &Path) -> std::io::Result<Vec<RawEntry>> {
    let mut out = vec![];
    walk(root, root, &mut out)?;
    // Deterministic output: same inputs must produce the same APK bytes.
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
