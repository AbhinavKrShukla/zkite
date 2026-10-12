//! `zkite-pack` — Phase 0 spike 2.
//!
//! Turns a manifest + `res/` tree + staging directory into a signed, installable APK.
//! No Gradle, no `aapt2`, no `zipalign`, no `apksigner`, no JDK, no `android.jar`.
//! The only inputs are files; the only dependency is `pack`.
//!
//! ```text
//! zkite-pack --app <dir> --stage <dir> --out <file.apk> [--pem <keys.pem>]
//! ```
//!
//! `--app` holds `AndroidManifest.xml` and `res/`; `--stage` holds everything that goes
//! into the APK verbatim (`classes.dex`, `lib/<abi>/*.so`, `assets/`).

mod apk;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use apk::Package;
use pack_sign::crypto_keys::Keys;

struct Args {
    app: PathBuf,
    stage: PathBuf,
    out: PathBuf,
    pem: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut app = None;
    let mut stage = None;
    let mut out = None;
    let mut pem = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--app" => app = Some(PathBuf::from(val()?)),
            "--stage" => stage = Some(PathBuf::from(val()?)),
            "--out" => out = Some(PathBuf::from(val()?)),
            "--pem" => pem = Some(PathBuf::from(val()?)),
            "-h" | "--help" => return Err("help".into()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Args {
        app: app.ok_or("--app is required")?,
        stage: stage.ok_or("--stage is required")?,
        out: out.ok_or("--out is required")?,
        pem,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("zkite-pack --app <dir> --stage <dir> --out <file.apk> [--pem <keys.pem>]");
            if e != "help" {
                eprintln!("error: {e}");
                return ExitCode::from(2);
            }
            return ExitCode::SUCCESS;
        }
    };

    match build(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("zkite-pack: {e}");
            ExitCode::from(1)
        }
    }
}

fn build(args: &Args) -> Result<(), String> {
    let t0 = Instant::now();

    let manifest_path = args.app.join("AndroidManifest.xml");
    let android_manifest =
        std::fs::read(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;

    let res_dir = args.app.join("res");
    let resources = apk::read_res_dir(&res_dir).map_err(|e| format!("{}: {e}", res_dir.display()))?;

    let extras =
        apk::read_staging_dir(&args.stage).map_err(|e| format!("{}: {e}", args.stage.display()))?;
    let staged_bytes: usize = extras.iter().map(|e| e.data.len()).sum();
    println!(
        "  staged {} files ({:.1} MB) from {}",
        extras.len(),
        staged_bytes as f64 / 1e6,
        args.stage.display()
    );
    for e in &extras {
        if e.data.len() > 512 * 1024 {
            println!("    {:<44} {:>8.1} MB", e.path, e.data.len() as f64 / 1e6);
        }
    }
    assert_native_libs_are_sane(&extras)?;

    let keys = match &args.pem {
        Some(p) => {
            let pem = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            Keys::from_combined_pem_string(&pem).map_err(|e| format!("{}: {e}", p.display()))?
        }
        None => {
            eprintln!(
                "!! no --pem given — signing with an ephemeral testing key. This APK will \
                 not update-install over one signed with a real key, and must never be the \
                 key a release build ships with."
            );
            Keys::generate_random_testing_keys()
                .map_err(|e| format!("could not generate a testing key: {e}"))?
        }
    };

    let package = Package { android_manifest, resources };
    let bytes = apk::compile_and_sign_apk_with_extras(&package, extras, &keys)
        .map_err(|e| format!("pack failed: {e}"))?;

    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&args.out, &bytes).map_err(|e| format!("{}: {e}", args.out.display()))?;

    println!(
        "  wrote {} ({:.1} MB) in {:.2}s — resources compiled, zip aligned, signed v2+v3",
        args.out.display(),
        bytes.len() as f64 / 1e6,
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

/// plan1.md §9 lists "16 KB page size" as a medium risk with the mitigation "add a
/// `zkite build` assertion that fails loudly rather than at Play upload". This is that
/// assertion, in its phase-0 form: every `lib/<abi>/*.so` must have `p_align >= 16384`
/// on its loadable segments.
fn assert_native_libs_are_sane(extras: &[apk::RawEntry]) -> Result<(), String> {
    let mut checked = 0;
    for e in extras {
        if !(e.path.starts_with("lib/") && e.path.ends_with(".so")) {
            continue;
        }
        let align = max_load_segment_alignment(&e.data)
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
    println!("  16 KB page alignment: {checked} native libraries OK");
    Ok(())
}

/// Minimal ELF64 program-header walk. Returns the largest `p_align` over `PT_LOAD`.
fn max_load_segment_alignment(data: &[u8]) -> Option<u64> {
    if data.len() < 64 || &data[..4] != b"\x7fELF" || data[4] != 2 {
        return None;
    }
    let le = data[5] == 1;
    let u16at = |o: usize| {
        let b = [data[o], data[o + 1]];
        if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) }
    };
    let u64at = |o: usize| {
        let b: [u8; 8] = data.get(o..o + 8)?.try_into().ok()?;
        Some(if le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) })
    };
    let phoff = u64at(0x20)? as usize;
    let phentsize = u16at(0x36) as usize;
    let phnum = u16at(0x38) as usize;

    let mut max = 0;
    for i in 0..phnum {
        let off = phoff + i * phentsize;
        if off + phentsize > data.len() {
            return None;
        }
        let p_type = {
            let b: [u8; 4] = data.get(off..off + 4)?.try_into().ok()?;
            if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }
        };
        if p_type != 1 {
            continue; // PT_LOAD
        }
        max = max.max(u64at(off + 0x30)?);
    }
    (max > 0).then_some(max)
}
