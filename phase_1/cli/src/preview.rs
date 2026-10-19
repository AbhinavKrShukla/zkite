//! Repackage a shipped reload shell with this project's launcher identity.
//! Native code, Java and Python stay prebuilt: no Android SDK is needed by users.

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use pack_asset_compiler::resource_internal_types::FileResource;
use zkite_bundler::{apk::{self, Package}, manifest_gen, zip_extras::RawEntry};
use crate::build_cmd::Backend;

pub struct Preview {
    pub apk: PathBuf,
    pub package: String,
    pub component: String,
}

pub fn prepare(project: &Path, backend: Backend, abi: &str, source: &Path) -> Result<Preview> {
    let (identity, icon) = crate::build_cmd::preview_identity(project, backend)?;
    let manifest = manifest_gen::android_manifest(&identity);
    let strings = manifest_gen::strings_xml(&identity);
    let shell = std::fs::read(source).context("reading the prebuilt development shell")?;
    let key_path = crate::run_cmd::debug_key(project)?;
    let key = std::fs::read(&key_path)?;
    let notices = crate::build_cmd::runtime_notices(crate::payload::root()?)?;
    let mut hash = Sha256::new();
    hash.update(b"adaptive-icon-v2");
    for part in [&shell, &manifest, &strings, &icon, &key] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    for notice in &notices {
        hash.update(&notice.path);
        hash.update(&notice.data);
    }
    let fingerprint = format!("{:x}", hash.finalize());
    let directory = project.join(".zkite").join("preview");
    std::fs::create_dir_all(&directory)?;
    let apk = directory.join(format!("{}-{abi}.apk", backend.name()));
    let stamp = apk.with_extension("sha256");
    if !apk.is_file() || std::fs::read_to_string(&stamp).ok().as_deref() != Some(&fingerprint) {
        let package = Package {
            android_manifest: manifest,
            resources: {
                let mut resources = zkite_bundler::launcher_icon::resources(icon);
                resources.push(FileResource { subdirectory: "values".into(), name: "strings.xml".into(), resource_id: 0, contents: strings });
                resources
            },
        };
        let mut extras = runtime_entries(&shell)?;
        extras.retain(|entry| !notices.iter().any(|notice| notice.path == entry.path));
        extras.extend(notices);
        apk::assert_native_libs_are_sane(&extras).map_err(anyhow::Error::msg)?;
        let keys = zkite_bundler::keystore::load_pem(&key_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let bytes = apk::compile_and_sign_apk_with_extras(&package, extras, &keys)
            .map_err(|e| anyhow::anyhow!("packaging the development preview: {e}"))?;
        crate::permissions::verify_archive(&bytes, &identity.permissions, false)?;
        let mut staged = tempfile::NamedTempFile::new_in(&directory)?;
        staged.write_all(&bytes)?;
        staged.persist(&apk).context("saving the development preview")?;
        std::fs::write(stamp, fingerprint)?;
        println!("  updated development preview: {}", identity.label);
    }
    Ok(Preview {
        component: format!("{}/{}", identity.package, identity.activity),
        package: identity.package,
        apk,
    })
}

fn runtime_entries(bytes: &[u8]) -> Result<Vec<RawEntry>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let path = entry.name().to_owned();
        // Resource tables and signatures belong to the old manifest. Only carry
        // runtime code/assets into a fresh, signed resource compilation.
        if entry.is_dir() || !(path.starts_with("assets/") || path.starts_with("lib/")
            || (path.starts_with("classes") && path.ends_with(".dex") && !path.contains('/'))) {
            continue;
        }
        anyhow::ensure!(entry.enclosed_name().is_some() && !path.contains(['\\', ':']),
            "unsafe development shell entry: {path}");
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        entries.push(RawEntry { path, data });
    }
    anyhow::ensure!(entries.iter().any(|e| e.path == "classes.dex"), "development shell has no classes.dex");
    // The WebView Dev Shell always reloads; the native shell requires this marker.
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repack_keeps_runtime_and_dev_marker_but_discards_old_identity_and_signatures() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in ["classes.dex", "assets/zkite-devmode", "assets/zkite-framework.zip",
            "lib/arm64-v8a/runtime.so", "AndroidManifest.xml", "resources.arsc",
            "res/drawable/icon_art.png", "META-INF/CERT.RSA"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(name.as_bytes()).unwrap();
        }
        let entries = runtime_entries(&zip.finish().unwrap().into_inner()).unwrap();
        assert_eq!(entries.len(), 4);
        assert!(entries.iter().any(|e| e.path == "assets/zkite-devmode"));
        assert!(entries.iter().all(|e| e.data == e.path.as_bytes()));
    }

    #[test]
    fn empty_shell_is_rejected() {
        let zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        assert!(runtime_entries(&zip.finish().unwrap().into_inner()).is_err());
    }

    #[test]
    fn changing_icon_contents_refreshes_cached_apk_without_changing_package_or_key() {
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        std::fs::write(root.join("pyproject.toml"),
            "[project]\nname='previewtest'\n[tool.zkite.android]\nicon='icon.png'\n").unwrap();
        let icon = std::fs::read(crate::payload::root().unwrap().join(
            "phase_2/appshell/app/res/drawable/icon.png")).unwrap();
        std::fs::write(root.join("icon.png"), &icon).unwrap();
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in ["classes.dex", "assets/zkite-devmode"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"fixture").unwrap();
        }
        let shell = root.join("shell.apk");
        std::fs::write(&shell, zip.finish().unwrap().into_inner()).unwrap();
        let first = prepare(root, Backend::Native, "arm64-v8a", &shell).unwrap();
        let original = std::fs::read(&first.apk).unwrap();
        let key = std::fs::read(root.join(".zkite/debug.pem")).unwrap();
        prepare(root, Backend::Native, "arm64-v8a", &shell).unwrap();
        assert_eq!(std::fs::read(&first.apk).unwrap(), original);
        // Extra trailing bytes leave the PNG image intact, but change its content
        // hash. A same-path replacement must invalidate the package cache.
        let mut changed = icon;
        changed.extend_from_slice(b"new icon file contents");
        std::fs::write(root.join("icon.png"), &changed).unwrap();
        let second = prepare(root, Backend::Native, "arm64-v8a", &shell).unwrap();
        assert_eq!(first.package, second.package);
        assert_eq!(std::fs::read(root.join(".zkite/debug.pem")).unwrap(), key);
        let bytes = std::fs::read(second.apk).unwrap();
        assert_ne!(bytes, original);
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut packaged = Vec::new();
        archive.by_name("res/drawable/icon_art.png").unwrap().read_to_end(&mut packaged).unwrap();
        assert_eq!(packaged, changed);
        let mut adaptive = Vec::new();
        archive.by_name("res/drawable/icon.xml").unwrap().read_to_end(&mut adaptive).unwrap();
        assert_eq!(&adaptive[..4], &[3, 0, 8, 0], "drawable XML must be compiled Android XML");
        assert!(archive.by_name("assets/zkite-devmode").is_ok());
    }
}
