//! Structural checks on the APK `zkite-pack` produced.
//!
//! These are the parts of "is this a real APK" that can be asserted without a device:
//! entry layout, zip alignment, the v2/v3 signing block, and the 16 KB-page guard. The
//! parts that *cannot* be asserted here — does it install, does it launch — are what
//! `scripts/run_on_device.sh` is for.
//!
//! Requires `ABI=x86_64 scripts/build_apk.sh` to have run; skips with a clear message
//! otherwise rather than failing, so `cargo test` is useful on a machine with no SDK.

use std::io::Read;
use std::path::PathBuf;

fn apk_path() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent()?.join("artifacts");
    for abi in ["x86_64", "arm64-v8a"] {
        let p = root.join(format!("zkite-phase0-{abi}.apk"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

macro_rules! apk_or_skip {
    () => {
        match apk_path() {
            Some(p) => p,
            None => {
                eprintln!("skipping: no APK built yet (run `ABI=x86_64 scripts/build_apk.sh`)");
                return;
            }
        }
    };
}

fn entries(path: &PathBuf) -> Vec<String> {
    let f = std::fs::File::open(path).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect()
}

#[test]
fn contains_everything_an_apk_needs() {
    let apk = apk_or_skip!();
    let names = entries(&apk);
    for required in ["AndroidManifest.xml", "resources.arsc", "classes.dex"] {
        assert!(names.iter().any(|n| n == required), "missing {required} in {names:?}");
    }
    assert!(
        names.iter().any(|n| n.starts_with("res/drawable/")),
        "the icon resource did not survive compilation"
    );
    assert!(
        names.iter().any(|n| n.starts_with("lib/") && n.ends_with("libpython3.14.so")),
        "libpython is not in the APK"
    );
    assert!(
        names.iter().any(|n| n.starts_with("lib/") && n.ends_with("libzkitespike1.so")),
        "the spike 1 cdylib is not in the APK"
    );
    assert!(
        names.iter().any(|n| n == "assets/python-stdlib.zip"),
        "the stdlib asset is not in the APK"
    );
}

#[test]
fn manifest_and_arsc_are_binary_not_text() {
    // pack's job is to *compile* XML, not copy it. A text manifest would install fine on
    // nothing at all, so this is worth asserting explicitly.
    let apk = apk_or_skip!();
    let f = std::fs::File::open(&apk).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();

    let mut head = [0u8; 4];
    z.by_name("AndroidManifest.xml").unwrap().read_exact(&mut head).unwrap();
    // ResChunk header: RES_XML_TYPE (0x0003), header size 8.
    assert_eq!(head, [0x03, 0x00, 0x08, 0x00], "AndroidManifest.xml is not binary AXML");

    let mut head = [0u8; 4];
    z.by_name("resources.arsc").unwrap().read_exact(&mut head).unwrap();
    // RES_TABLE_TYPE (0x0002), header size 12.
    assert_eq!(head, [0x02, 0x00, 0x0c, 0x00], "resources.arsc is not an ARSC table");
}

#[test]
fn resources_arsc_is_stored_uncompressed() {
    // The platform mmaps resources.arsc directly; a deflated one is a runtime failure.
    let apk = apk_or_skip!();
    let f = std::fs::File::open(&apk).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let e = z.by_name("resources.arsc").unwrap();
    assert_eq!(e.compression(), zip::CompressionMethod::Stored);
}

#[test]
fn has_an_apk_signing_block() {
    // APK Signature Scheme v2/v3 puts a block with this magic immediately before the
    // central directory. Its presence is what `adb install` and Play both check first.
    let apk = apk_or_skip!();
    let bytes = std::fs::read(&apk).unwrap();
    let magic = b"APK Sig Block 42";
    assert!(
        bytes.windows(magic.len()).any(|w| w == magic),
        "no APK Signing Block — the APK is unsigned"
    );
}

#[test]
fn stored_entries_are_four_byte_aligned() {
    // zipalign's whole job. Misaligned stored entries cannot be mmapped.
    let apk = apk_or_skip!();
    let f = std::fs::File::open(&apk).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let mut checked = 0;
    for i in 0..z.len() {
        let e = z.by_index(i).unwrap();
        if e.compression() != zip::CompressionMethod::Stored {
            continue;
        }
        let off = e.data_start();
        assert_eq!(off % 4, 0, "{} starts at {off}, not 4-byte aligned", e.name());
        checked += 1;
    }
    assert!(checked > 0, "no stored entries found to check");
}

#[test]
fn native_libraries_are_16kb_page_aligned() {
    // plan1.md §9: Play requires 16 KB-aligned native libs. Catch it here, not at upload.
    let apk = apk_or_skip!();
    let f = std::fs::File::open(&apk).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let names: Vec<String> = (0..z.len())
        .map(|i| z.by_index(i).unwrap().name().to_string())
        .filter(|n| n.starts_with("lib/") && n.ends_with(".so"))
        .collect();
    assert!(!names.is_empty(), "no native libraries in the APK");

    for name in &names {
        let mut buf = vec![];
        z.by_name(name).unwrap().read_to_end(&mut buf).unwrap();
        let align = max_load_alignment(&buf)
            .unwrap_or_else(|| panic!("{name}: could not parse ELF program headers"));
        assert!(align >= 16384, "{name}: PT_LOAD alignment {align} < 16384");
    }
    eprintln!("checked {} native libraries", names.len());
}

/// Same ELF64 walk as `src/main.rs`, duplicated so the test proves the property from the
/// finished artifact rather than trusting the code that produced it.
fn max_load_alignment(data: &[u8]) -> Option<u64> {
    if data.len() < 64 || &data[..4] != b"\x7fELF" || data[4] != 2 || data[5] != 1 {
        return None;
    }
    let u16at = |o: usize| u16::from_le_bytes([data[o], data[o + 1]]);
    let u64at = |o: usize| -> Option<u64> {
        Some(u64::from_le_bytes(data.get(o..o + 8)?.try_into().ok()?))
    };
    let phoff = u64at(0x20)? as usize;
    let phentsize = u16at(0x36) as usize;
    let phnum = u16at(0x38) as usize;
    let mut max = 0;
    for i in 0..phnum {
        let off = phoff + i * phentsize;
        let p_type = u32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?);
        if p_type == 1 {
            max = max.max(u64at(off + 0x30)?);
        }
    }
    (max > 0).then_some(max)
}

#[test]
fn staged_entries_are_in_deterministic_order() {
    // Reproducible builds need a stable entry order. `read_staging_dir` sorts, so
    // directory-iteration order cannot leak into the APK; this asserts that held, and
    // that nothing got added twice. (The signing key is still a source of run-to-run
    // difference — pass `--pem` for a byte-stable APK.)
    let apk = apk_or_skip!();
    let f = std::fs::File::open(&apk).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let mut seen = std::collections::HashSet::new();
    for i in 0..z.len() {
        let e = z.by_index(i).unwrap();
        assert!(seen.insert(e.name().to_string()), "duplicate entry {}", e.name());
    }
    let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
    let mut sorted = names.clone();
    sorted[3..].sort();
    assert_eq!(
        &names[3..],
        &sorted[3..],
        "staged entries are not in sorted order, so builds are not reproducible"
    );
}
