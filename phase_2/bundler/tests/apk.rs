//! Full pack-and-sign pipeline, checked against the real Android tools it replaces —
//! same pattern phase 0's spike 2 used (`apksigner verify`, `zipalign -c`, `aapt2
//! dump packagename`), now exercised through the promoted library (`../PLAN.md` §1.1)
//! instead of the spike's standalone binary.

use std::path::PathBuf;

use zkite_bundler::apk::{compile_and_sign_apk_with_extras, Package};
use zkite_bundler::keystore::{generate, KeygenOptions};
use zkite_bundler::manifest_gen::{android_manifest, strings_xml, AppIdentity, APPSHELL_ACTIVITY};
use zkite_bundler::zip_extras::RawEntry;
use pack_asset_compiler::resource_internal_types::FileResource;

const ICON_PNG: &[u8] = include_bytes!("../../../phase_1/devshell/app/res/drawable/icon.png");

fn android_build_tools() -> Option<PathBuf> {
    let home = std::env::var_os("ANDROID_HOME")
        .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
        .map(PathBuf::from)
        .or_else(|| {
            if cfg!(windows) {
                std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Android/Sdk"))
            } else {
                std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Android/Sdk"))
            }
        })?;
    for version in ["34.0.0", "35.0.0", "36.0.0"] {
        let dir = home.join("build-tools").join(version);
        if dir.join(if cfg!(windows) { "apksigner.bat" } else { "apksigner" }).exists() {
            return Some(dir);
        }
    }
    None
}

fn build_signed_test_apk(package: &str) -> Vec<u8> {
    let id = AppIdentity {
        package: package.into(),
        label: "Bundler Test".into(),
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
        RawEntry { path: "classes.dex".into(), data: b"not a real dex, just bytes".to_vec() },
        RawEntry { path: "assets/app.zip".into(), data: b"pretend app payload".to_vec() },
    ];

    let keys = generate(&KeygenOptions::default()).expect("keygen");
    let combined_pem_keys = pack_sign::crypto_keys::Keys::from_combined_pem_string(&keys.combined_pem)
        .expect("parse generated keystore");

    compile_and_sign_apk_with_extras(&pkg, extras, &combined_pem_keys).expect("pack+sign")
}

#[test]
fn packed_apk_verifies_with_real_android_tools() {
    let Some(build_tools) = android_build_tools() else {
        eprintln!("no Android build-tools found under $ANDROID_HOME, skipping");
        return;
    };

    let bytes = build_signed_test_apk("dev.zkite.bundlertest");
    let dir = tempfile::tempdir().unwrap();
    let apk_path = dir.path().join("test.apk");
    std::fs::write(&apk_path, &bytes).unwrap();

    let apksigner = std::process::Command::new(build_tools.join(if cfg!(windows) { "apksigner.bat" } else { "apksigner" }))
        .args(["verify", "--verbose", apk_path.to_str().unwrap()])
        .output()
        .expect("run apksigner");
    let apksigner_out = String::from_utf8_lossy(&apksigner.stdout);
    assert!(
        apksigner.status.success(),
        "apksigner verify failed:\n{apksigner_out}\n{}",
        String::from_utf8_lossy(&apksigner.stderr)
    );
    assert!(apksigner_out.contains("Verified using v2 scheme (APK Signature Scheme v2): true"));
    assert!(apksigner_out.contains("Verified using v3 scheme (APK Signature Scheme v3): true"));

    let zipalign = std::process::Command::new(build_tools.join(if cfg!(windows) { "zipalign.exe" } else { "zipalign" }))
        .args(["-c", "-v", "4", apk_path.to_str().unwrap()])
        .output()
        .expect("run zipalign");
    assert!(
        zipalign.status.success(),
        "zipalign -c failed:\n{}",
        String::from_utf8_lossy(&zipalign.stdout)
    );

    let aapt2 = std::process::Command::new(build_tools.join(if cfg!(windows) { "aapt2.exe" } else { "aapt2" }))
        .args(["dump", "packagename", apk_path.to_str().unwrap()])
        .output()
        .expect("run aapt2");
    assert!(aapt2.status.success());
    assert_eq!(String::from_utf8_lossy(&aapt2.stdout).trim(), "dev.zkite.bundlertest");
}

#[test]
fn extras_land_in_the_apk_byte_for_byte() {
    let bytes = build_signed_test_apk("dev.zkite.bundlertest2");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open as zip");

    let mut dex = zip.by_name("classes.dex").expect("classes.dex present");
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut dex, &mut buf).unwrap();
    assert_eq!(buf, b"not a real dex, just bytes");
}

#[test]
fn duplicate_extra_paths_panic_instead_of_silently_clobbering() {
    let id = AppIdentity {
        package: "dev.zkite.bundlertest3".into(),
        label: "Dup Test".into(),
        version_code: 1,
        version_name: "0.1.0".into(),
        min_sdk: 26,
        target_sdk: 34,
            activity: APPSHELL_ACTIVITY.into(),
            permissions: vec!["android.permission.CAMERA".into(), "android.permission.RECORD_AUDIO".into(), "android.permission.VIBRATE".into()],
    };
    let pkg = Package {
        android_manifest: android_manifest(&id),
        resources: vec![
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
        ],
    };
    let extras = vec![
        RawEntry { path: "classes.dex".into(), data: b"a".to_vec() },
        RawEntry { path: "classes.dex".into(), data: b"b".to_vec() },
    ];
    let keys = generate(&KeygenOptions::default()).expect("keygen");
    let combined_pem_keys =
        pack_sign::crypto_keys::Keys::from_combined_pem_string(&keys.combined_pem).unwrap();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compile_and_sign_apk_with_extras(&pkg, extras, &combined_pem_keys)
    }));
    assert!(result.is_err(), "expected a panic on duplicate zip entry paths");
}

#[test]
fn compiled_permissions_are_root_children_with_android_name() {
    let bytes = build_signed_test_apk("dev.zkite.permissiontest");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut manifest = Vec::new();
    use std::io::Read;
    zip.by_name("AndroidManifest.xml").unwrap().read_to_end(&mut manifest).unwrap();
    let found = zkite_bundler::manifest_verify::apk_permissions(&manifest).unwrap();
    assert_eq!(found.into_iter().collect::<Vec<_>>(), vec!["android.permission.CAMERA", "android.permission.RECORD_AUDIO", "android.permission.VIBRATE"]);
}
