//! AAB packing — `pack_aab::construct_aab` has the exact same "no code" gap
//! `apk.rs` closes for APK, for the exact same reason: `pack` was written for
//! code-free Wear OS watch faces. See `../PLAN.md` §1.6.
//!
//! An Android App Bundle's `base` module lays out code and native libs under
//! `base/dex/`, `base/lib/<abi>/`, `base/assets/**` — this splices verbatim
//! `pack_zip::File` entries at those paths into what `construct_aab` returns,
//! before v1 signing (bundles need it; Play's backend never adopted v2 skip-v1)
//! and the v2/v3 signing block `apk.rs` also uses.
//!
//! Gated behind the `aab` feature (see `Cargo.toml`) because `pack-aab`'s build
//! script needs `protoc` — a release-pipeline-only requirement (`../PLAN.md` §1.5),
//! exactly like the NDK/JDK, never something a `zkite build apk`-only user needs.

use std::io::Cursor;

use pack_asset_compiler::{
    resource_internal_types::Resource, strings_xml_parser::parse_strings_xml,
};
use pack_common::Result;
use pack_sign::crypto_keys::Keys;
use pack_sign::v1_signing::add_v1_signature_files;

use crate::apk::Package;
use crate::zip_extras::{self, RawEntry};

/// Compile + sign an AAB from a manifest, a `res/` tree, and verbatim extra entries.
/// `extras` paths are given relative to the `base` module root (e.g.
/// `dex/classes.dex`, `lib/arm64-v8a/libzkite.so`, `assets/app.zip` — no `base/`
/// prefix; this function adds it), matching how `apk.rs`'s extras are relative to
/// the APK root.
pub fn compile_and_sign_aab_with_extras(
    package: &Package,
    extras: Vec<RawEntry>,
    keys: &Keys,
) -> Result<Vec<u8>> {
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

    let manifest_str = String::from_utf8(package.android_manifest.clone())
        .map_err(|_e| pack_common::PackError::ManifestIsNotUTF8)?;
    let (_manifest_chunk, package_name, label) =
        parse_manifest_info(&package.android_manifest, &resources)?;

    let mut aab_files =
        pack_aab::construct_aab(&package_name, &label, manifest_str, &mut resources)?;

    let mut seen_paths: std::collections::HashSet<String> =
        aab_files.iter().map(|f| f.path.clone()).collect();
    for e in extras {
        let path = format!("base/{}", e.path);
        zip_extras::validate_entry_path(&path, &mut seen_paths);
        aab_files.push(pack_zip::File { path, data: e.data });
    }

    add_v1_signature_files(&mut aab_files, keys)?;

    let mut aab_buf = vec![];
    pack_zip::zip_apk(&aab_files, Cursor::new(&mut aab_buf))?;
    pack_sign::sign_apk_buffer(&mut aab_buf, keys)
}

fn parse_manifest_info(
    manifest: &[u8],
    resources: &[Resource],
) -> Result<(pack_asset_compiler::resource_external_types::ResChunk, String, Option<String>)> {
    let manifest_cursor = Cursor::new(manifest);
    let mut reader = std::io::BufReader::new(manifest_cursor);
    let (chunk, info) = pack_asset_compiler::xml_file::xml_to_res_chunk(&mut reader, resources)?;
    let package_name =
        info.package_name.ok_or(pack_common::PackError::ManifestDoesNotHavePackageName)?;
    Ok((chunk, package_name, info.label))
}
