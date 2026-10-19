//! AAB packing, checked with `bundletool` — literally the same validator Google
//! Play's own backend runs, per `../PLAN.md` §1.5/§1.6. This is the closest thing to
//! "accepted by Play Console" (plan1.md §10's phase 2 exit criterion) reachable
//! without an actual Play Console account and upload.
//!
//! Gated on the `aab` feature (needs `pack-aab`, which needs `protoc` at *this
//! crate's* build time — `../PLAN.md` §1.5): `cargo test --features aab --test aab`.

#![cfg(feature = "aab")]

use std::path::PathBuf;

use zkite_bundler::aab::compile_and_sign_aab_with_extras;
use zkite_bundler::apk::Package;
use zkite_bundler::keystore::{generate, KeygenOptions};
use zkite_bundler::manifest_gen::{android_manifest, strings_xml, AppIdentity, APPSHELL_ACTIVITY};
use zkite_bundler::zip_extras::RawEntry;
use pack_asset_compiler::resource_internal_types::FileResource;

const ICON_PNG: &[u8] = include_bytes!("../../../phase_1/devshell/app/res/drawable/icon.png");

fn bundletool_jar() -> Option<PathBuf> {
    let candidates = [
        concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/bundletool/bundletool.jar"),
    ];
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

fn build_test_aab(package: &str) -> Vec<u8> {
    let id = AppIdentity {
        package: package.into(),
        label: "Bundler AAB Test".into(),
        version_code: 1,
        version_name: "0.1.0".into(),
        min_sdk: 26,
        target_sdk: 34,
            activity: APPSHELL_ACTIVITY.into(),
            permissions: vec!["android.permission.CAMERA".into(), "android.permission.RECORD_AUDIO".into(), "android.permission.VIBRATE".into()],
    };
    let manifest = android_manifest(&id);
    let resources = vec![
        FileResource {
            subdirectory: "values".into(),
            name: "strings.xml".into(),
            resource_id: 0,
            contents: strings_xml(&id),
        },
        FileResource {
            subdirectory: "drawable".into(),
            name: "icon.png".into(),
            resource_id: 0,
            contents: ICON_PNG.to_vec(),
        },
    ];
    let pkg = Package { android_manifest: manifest, resources };

    let extras = vec![
        RawEntry { path: "dex/classes.dex".into(), data: b"not a real dex, just bytes".to_vec() },
        RawEntry { path: "assets/app.zip".into(), data: b"pretend app payload".to_vec() },
        RawEntry {
            path: "lib/arm64-v8a/libzkiteappshell.so".into(),
            data: b"not a real .so, just bytes".to_vec(),
        },
    ];

    let keys = generate(&KeygenOptions::default()).expect("keygen");
    let keys = pack_sign::crypto_keys::Keys::from_combined_pem_string(&keys.combined_pem).unwrap();

    compile_and_sign_aab_with_extras(&pkg, extras, &keys).expect("pack+sign aab")
}

#[test]
fn packed_aab_validates_with_bundletool() {
    let Some(jar) = bundletool_jar() else {
        eprintln!("bundletool.jar not vendored, skipping (see ../PLAN.md §8)");
        return;
    };

    let bytes = build_test_aab("dev.zkite.bundlertest.aab");
    let dir = tempfile::tempdir().unwrap();
    let aab_path = dir.path().join("test.aab");
    std::fs::write(&aab_path, &bytes).unwrap();

    let result = std::process::Command::new("java")
        .args(["-jar", jar.to_str().unwrap(), "validate", "--bundle"])
        .arg(&aab_path)
        .output()
        .expect("run bundletool validate");

    assert!(
        result.status.success(),
        "bundletool validate failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn extras_land_under_the_base_module() {
    let bytes = build_test_aab("dev.zkite.bundlertest.aab2");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open as zip");

    let mut buf = Vec::new();
    {
        let mut dex = zip.by_name("base/dex/classes.dex").expect("base/dex/classes.dex present");
        std::io::Read::read_to_end(&mut dex, &mut buf).unwrap();
    }
    assert_eq!(buf, b"not a real dex, just bytes");

    assert!(zip.by_name("base/lib/arm64-v8a/libzkiteappshell.so").is_ok());
    assert!(zip.by_name("base/assets/app.zip").is_ok());
    // Bundle-level (not module-relative) required entries, per `construct_aab`.
    assert!(zip.by_name("BundleConfig.pb").is_ok());
    assert!(zip.by_name("base/manifest/AndroidManifest.xml").is_ok());
}

#[test]
fn compiled_permissions_are_root_children_with_android_name() {
    let bytes = build_test_aab("dev.zkite.permissiontest");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut manifest = Vec::new();
    use std::io::Read;
    zip.by_name("base/manifest/AndroidManifest.xml").unwrap().read_to_end(&mut manifest).unwrap();
    let found = zkite_bundler::manifest_verify::aab_permissions(&manifest).unwrap();
    assert_eq!(found.into_iter().collect::<Vec<_>>(), vec!["android.permission.CAMERA", "android.permission.RECORD_AUDIO", "android.permission.VIBRATE"]);
}
