//! APK packing — promoted from `phase_0/spike2_pack_apk/src/apk.rs`, unchanged in
//! substance (see `../PLAN.md` §1.1). The upstream shape this wants is still one
//! `extra_files: Vec<pack_zip::File>` field on `pack_api::Package` (plan1.md §11:
//! "contribute rather than fork").

use std::io::Cursor;

use deku::DekuContainerWrite;
use pack_asset_compiler::{
    resource_external_types::ResChunk,
    resource_internal_types::{FileResource, Resource},
    resource_table::construct_resource_table,
    strings_xml_parser::parse_strings_xml,
    xml_file::xml_to_res_chunk,
};
use pack_common::{PackError, Result};
use pack_sign::crypto_keys::Keys;

use crate::zip_extras::{self, RawEntry};

pub use zip_extras::{read_res_dir, read_staging_dir};

/// The inputs `pack` already understands: a manifest and a flat `res/` tree.
pub struct Package {
    pub android_manifest: Vec<u8>,
    pub resources: Vec<FileResource>,
}

/// Compile + sign an APK from a manifest, a `res/` tree, and verbatim extra entries
/// (`classes.dex`, `lib/<abi>/*.so`, `assets/**`).
pub fn compile_and_sign_apk_with_extras(
    package: &Package,
    extras: Vec<RawEntry>,
    keys: &Keys,
) -> Result<Vec<u8>> {
    let mut zip_buf = compile_apk_with_extras(package, extras)?;
    pack_sign::sign_apk_buffer(&mut zip_buf, keys)
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

    for f in &apk_files {
        seen_paths.insert(f.path.clone());
    }

    for e in extras {
        zip_extras::validate_entry_path(&e.path, &mut seen_paths);
        apk_files.push(pack_zip::File { path: e.path, data: e.data });
    }

    let mut zip_buf = vec![];
    pack_zip::zip_apk(&apk_files, Cursor::new(&mut zip_buf))?;
    Ok(zip_buf)
}

fn chunk_file(path: &str, chunk: &ResChunk) -> Result<pack_zip::File> {
    Ok(pack_zip::File { path: path.to_string(), data: chunk.to_bytes()? })
}

/// plan1.md §9: "16 KB page size... add a `zkite build` assertion that fails loudly
/// rather than at Play upload." Promoted from `spike2_pack_apk/src/main.rs`, which
/// had it as a CLI-only check; every `zkite build apk` needs it, not just the spike.
pub fn assert_native_libs_are_sane(extras: &[RawEntry]) -> std::result::Result<usize, String> {
    let mut checked = 0;
    for e in extras {
        if !(e.path.starts_with("lib/") && e.path.ends_with(".so")) {
            continue;
        }
        let align = min_load_segment_alignment(&e.data)
            .ok_or_else(|| format!("{}: not a parseable 64-bit ELF", e.path))?;
        if align < 16384 {
            return Err(format!(
                "{}: PT_LOAD alignment is {align} bytes; Play requires 16 KB alignment. \
                 Rebuild with `-Wl,-z,max-page-size=16384` (NDK r27+).",
                e.path
            ));
        }
        checked += 1;
    }
    Ok(checked)
}

fn min_load_segment_alignment(data: &[u8]) -> Option<u64> {
    if data.len() < 64 || &data[..4] != b"\x7fELF" || data[4] != 2 || !matches!(data[5], 1 | 2) {
        return None;
    }
    let le = data[5] == 1;
    let u16at = |o: usize| {
        let b = [data[o], data[o + 1]];
        if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) }
    };
    let u64at = |o: usize| {
        let b: [u8; 8] = data.get(o..o.checked_add(8)?)?.try_into().ok()?;
        Some(if le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) })
    };
    let phoff = usize::try_from(u64at(0x20)?).ok()?;
    let phentsize = u16at(0x36) as usize;
    let phnum = u16at(0x38) as usize;

    if phentsize < 56 {
        return None;
    }
    let mut min = None;
    for i in 0..phnum {
        let off = phoff.checked_add(i.checked_mul(phentsize)?)?;
        if off.checked_add(phentsize)? > data.len() {
            return None;
        }
        let p_type = {
            let b: [u8; 4] = data.get(off..off + 4)?.try_into().ok()?;
            if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }
        };
        if p_type != 1 {
            continue; // PT_LOAD
        }
        let alignment = u64at(off + 0x30)?;
        if !alignment.is_power_of_two() {
            return None;
        }
        // Every PT_LOAD must be compatible with 16 KiB pages; taking the largest
        // alignment let one good segment hide another segment aligned to 4 KiB.
        min = Some(min.map_or(alignment, |previous: u64| previous.min(alignment)));
    }
    min
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elf(alignments: &[u64]) -> Vec<u8> {
        let mut bytes = vec![0; 64 + alignments.len() * 56];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[0x20..0x28].copy_from_slice(&64u64.to_le_bytes());
        bytes[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        bytes[0x38..0x3a].copy_from_slice(&(alignments.len() as u16).to_le_bytes());
        for (i, alignment) in alignments.iter().enumerate() {
            let offset = 64 + i * 56;
            bytes[offset..offset + 4].copy_from_slice(&1u32.to_le_bytes());
            bytes[offset + 48..offset + 56].copy_from_slice(&alignment.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn every_load_segment_must_support_16_kib_pages() {
        let entries = |alignments: &[u64]| vec![RawEntry { path: "lib/arm64-v8a/libtest.so".into(), data: elf(alignments) }];
        assert_eq!(assert_native_libs_are_sane(&entries(&[16384, 65536])).unwrap(), 1);
        assert!(assert_native_libs_are_sane(&entries(&[16384, 4096])).is_err());
        assert!(assert_native_libs_are_sane(&entries(&[4096, 16384])).is_err());
    }

    #[test]
    fn invalid_elf_offsets_are_errors_not_panics() {
        let mut bytes = elf(&[16384]);
        bytes[0x20..0x28].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(min_load_segment_alignment(&bytes).is_none());
        assert!(min_load_segment_alignment(&elf(&[17000])).is_none());
    }
}
