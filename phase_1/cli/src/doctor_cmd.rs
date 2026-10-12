//! `zkite doctor` — plan1.md §10: "which deps lack Android wheels, adb reachable,
//! etc." Every check runs on the user's own machine using only what `pip install
//! zkite` already implies (a Python interpreter, this binary) plus the vendored
//! `pip.pyz` (../../phase_2/PLAN.md §1.3) — no NDK/JDK/Android SDK checks here,
//! because `zkite build` never touches them (§1.1/§1.5).

use std::process::Command;

use crate::build_cmd;
use crate::wheels::{self, AndroidTarget, WheelOutcome};

pub fn run(adb: &str) -> anyhow::Result<()> {
    let mut ok = true;

    // **First, and before anything that reads it.** §23.7 step 1: "`zkite doctor` must
    // report which root it used, or the first person to debug a wheel gets a path that
    // does not exist and no clue why." Every check below except adb and the host
    // Python resolves against this, so reporting it after them would bury the cause
    // under its own symptoms.
    ok &= check("build payload (phase_3/PLAN.md §23.2)", check_payload());
    // Everything `zkite build` reads, not just the two markers the resolver probes for.
    // A payload missing `pip.pyz` resolves fine and then degrades silently — §23.2
    // called that "the quiet one" — and nothing before this asked.
    ok &= check("payload is complete (phase_3/PLAN.md §29.7)", check_payload_complete());
    // And whether the shell inside it came from the source beside it (§34.8).
    ok &= check("prebuilt shell matches its source (phase_3/PLAN.md §34.8)", check_provenance());

    ok &= check("adb on PATH + a device/emulator connected", check_adb(adb));
    ok &= check("Python 3.14 for .pyc compilation (phase_2/PLAN.md §1.4)", check_pyc_python());
    ok &= check("pip (vendored or system) for dependency resolution", check_pip());
    // **Only the *default* backend's artifacts are fatal.** Both are reported either
    // way, because the failure that prevents is a `zkite build` which stops with an
    // artifact error *after* already having done the pyc/prune/wheel work.
    //
    // This split used to be hardcoded the other way round — webview fatal, native
    // informational, with a comment reading "the native renderer is not the default".
    // That was true when it was written and false since §15.4 flipped the default, and
    // it had a consequence nobody had hit: §23.4's own recommended default wheel is
    // **native-only, arm64-only**, so a correctly-packaged `pip install zkite` would
    // have made `zkite doctor` exit 1 on a perfectly working machine. Found by running
    // `doctor` against a staged payload (§29), not by reading it. Keyed off
    // `Backend::DEFAULT` now, so a future flip cannot desynchronise it again.
    for backend in [build_cmd::Backend::Native, build_cmd::Backend::WebView] {
        let (rel, libs, assets): (_, &[&str], &[&str]) = match backend {
            build_cmd::Backend::Native => (
                "phase_3/gpu_harness",
                &["libzkitegpuharness.so", "libpython3.14.so", "libc++_shared.so"],
                &["fonts.zip"],
            ),
            build_cmd::Backend::WebView => {
                ("phase_2/appshell", &["libzkiteappshell.so", "libpython3.14.so"], &[])
            }
        };
        let name = backend.name();
        let is_default = backend == build_cmd::Backend::DEFAULT;
        let result = check_shell_artifacts(rel, libs, assets);
        if is_default {
            ok &= check(
                &format!("prebuilt {name}-backend artifacts (the default backend)"),
                result,
            );
        } else {
            match result {
                Ok(detail) => println!("  ok    prebuilt {name}-backend artifacts\n        {detail}"),
                Err(detail) => println!(
                    "  --    {name} backend unavailable — not required, since the default is \
                     {}. `--backend {name}` will not work here.\n        {detail}",
                    build_cmd::Backend::DEFAULT.name(),
                ),
            }
        }
    }

    if std::path::Path::new("app.json").is_file() || std::path::Path::new("pyproject.toml").is_file() {
        println!("\nproject dependencies:");
        match build_cmd::project_document(std::path::Path::new(".")) {
            Ok(doc) => check_dependencies(&toml::to_string(&doc)?),
            Err(error) => { println!("  ! {error:#}"); ok = false; }
        }
    }

    println!();
    if ok {
        println!("all checks passed");
        Ok(())
    } else {
        anyhow::bail!("one or more checks failed — see above")
    }
}

fn check(label: &str, result: Result<String, String>) -> bool {
    match result {
        Ok(detail) => {
            println!("  ok    {label}\n        {detail}");
            true
        }
        Err(detail) => {
            println!("  FAIL  {label}\n        {detail}");
            false
        }
    }
}

fn check_payload() -> Result<String, String> {
    let payload = crate::payload::get().map_err(|e| e.to_string())?;
    let mut detail = payload.describe();
    // Say so out loud rather than only in the origin label. A wheel that silently
    // resolved a developer's checkout would otherwise look completely healthy here,
    // which is the one failure mode this whole resolver introduces.
    if payload.origin() == crate::payload::Origin::Checkout {
        detail.push_str(&format!(
            "\n        note: this is the compile-time fallback, so `zkite build` works \
             here and would not off this machine.\n        \
             Set ${} to override.",
            crate::payload::ENV_VAR
        ));
    }
    Ok(detail)
}

/// Is every path `zkite build` will read actually in this payload?
///
/// `check_payload` only proves the resolver picked a directory; `check_shell_artifacts`
/// only covers the shell's own `.so`s and dex. Between them sits everything shared —
/// `zkite_py`'s source, the pybuild scripts, the vendored `pip.pyz`, the default icon,
/// the pure-Python stdlib — and a payload missing one of those was, until §37,
/// something you found out about mid-build or (for `pip.pyz`) never.
///
/// Reported per ABI, because a `arm64-v8a`-only payload is the recommended default
/// wheel (§23.4) and is complete, not broken.
fn check_payload_complete() -> Result<String, String> {
    let root = crate::payload::root().map_err(|e| e.to_string())?;
    let backend = build_cmd::Backend::DEFAULT.name();
    let mut complete = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for abi in ["arm64-v8a", "x86_64"] {
        let entries = crate::payload::staged_paths(backend, &[abi.to_string()], false)
            .map_err(|e| e.to_string())?;
        let absent: Vec<String> = entries
            .iter()
            .filter(|e| !root.join(e.path()).exists())
            .map(|e| e.path().to_string())
            .collect();
        if absent.is_empty() {
            complete.push(abi);
        } else if missing.is_empty() {
            missing = absent;
        }
    }
    if complete.is_empty() {
        return Err(format!(
            "the {backend} payload at {} is missing {} entr{}: {}{}",
            root.display(),
            missing.len(),
            if missing.len() == 1 { "y" } else { "ies" },
            missing.iter().take(6).cloned().collect::<Vec<_>>().join(", "),
            if missing.len() > 6 { ", …" } else { "" },
        ));
    }
    Ok(format!("{backend} backend, complete for: {}", complete.join(", ")))
}

/// What the shell's own provenance record says, and whether it still holds (§34.8).
fn check_provenance() -> Result<String, String> {
    let root = crate::payload::root().map_err(|e| e.to_string())?;
    let backend = build_cmd::Backend::DEFAULT;
    let rel = match backend {
        build_cmd::Backend::Native => "phase_3/gpu_harness",
        build_cmd::Backend::WebView => "phase_2/appshell",
    };
    let artifacts = root.join(rel).join("artifacts");
    let record = crate::provenance::read(&artifacts).map_err(|e| e.to_string())?;
    let Some(record) = record else {
        return Err(format!(
            "no provenance.json under {} — nothing records which build produced these \
             artifacts. Run {rel}/scripts/build.sh",
            artifacts.display()
        ));
    };
    let mut lines = Vec::new();
    let mut stale = Vec::new();
    for (name, group) in &record.groups {
        let verdict = crate::provenance::check_group(root, group);
        if verdict.is_stale() {
            stale.push(format!("{name} (rebuild: {})", group.rebuild));
        }
        // Which verdict it was, not just when it was built. A packaged payload carries
        // no sources, so most of these are *reported* rather than *verified* — and a
        // line that reads identically either way is the same silence §34.8 is about.
        // This module already made that mistake once, in this function.
        let how = match verdict {
            crate::provenance::Verdict::Matches => "verified",
            crate::provenance::Verdict::SourcesAbsent { .. } => "not re-derived, no sources here",
            crate::provenance::Verdict::NotDeterminedBySource { .. } => "not determined by source",
            crate::provenance::Verdict::Stale { .. } => "STALE",
        };
        lines.push(format!("{name} {} ({how})", group.built_at));
    }
    if !stale.is_empty() {
        return Err(format!(
            "{} shell artifacts were built from different source than this payload: {}",
            record.shell,
            stale.join("; ")
        ));
    }
    Ok(format!("{} shell: {}", record.shell, lines.join(", ")))
}

fn check_adb(adb: &str) -> Result<String, String> {
    let out = Command::new(adb)
        .arg("devices")
        .output()
        .map_err(|e| format!("could not run `{adb} devices`: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let devices: Vec<&str> = text.lines().skip(1).filter(|l| l.contains("\tdevice")).collect();
    if devices.is_empty() {
        return Err(format!("`{adb} devices` found no connected device/emulator"));
    }
    Ok(format!("{} device(s): {}", devices.len(), devices.join(", ")))
}

fn check_pyc_python() -> Result<String, String> {
    crate::host_python::find(Some((3, 14)))
        .map(|path| path.display().to_string())
        .map_err(|error| error.to_string())
}

fn check_pip() -> Result<String, String> {
    if let Some(pyz) = wheels::vendored_pip_pyz() {
        return Ok(format!("vendored {}", pyz.display()));
    }
    let python = wheels::find_host_python().ok_or("no python3/python on PATH at all")?;
    let out = Command::new(&python)
        .args(["-m", "pip", "--version"])
        .output()
        .map_err(|e| format!("could not run `{} -m pip --version`: {e}", python.display()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "no vendored pip.pyz and `{} -m pip` is not available — dependency \
             resolution will fail for any project with non-empty [project.dependencies]",
            python.display()
        ))
    }
}

/// Mirrors `build_cmd::resolve_abis` — an ABI counts only if *every* library that
/// shell needs is present for it, not just its cdylib, since a half-staged artifacts
/// directory otherwise passes here and fails at `System.loadLibrary` on a device.
fn check_shell_artifacts(
    shell_rel: &str,
    libs: &[&str],
    shared_assets: &[&str],
) -> Result<String, String> {
    // §23.2 called this the worst of the four anchors, out of proportion to its size:
    // `doctor` is the command a stuck user runs, and off this checkout it answered a
    // real question with a path that did not exist on their machine. A diagnostic that
    // is itself broken is worse than no diagnostic.
    let base = crate::payload::root()
        .map_err(|e| e.to_string())?
        .join(shell_rel)
        .join("artifacts");
    let abis: Vec<&str> = ["arm64-v8a", "x86_64"]
        .into_iter()
        .filter(|abi| libs.iter().all(|lib| base.join(abi).join(lib).exists()))
        .collect();
    if abis.is_empty() {
        return Err(format!(
            "no complete set of [{}] under {} — run {shell_rel}/scripts/build.sh",
            libs.join(", "),
            base.display()
        ));
    }
    for asset in shared_assets.iter().chain(["classes.dex"].iter()) {
        if !base.join(asset).exists() {
            return Err(format!(
                "{abis} present but no {}/{asset} — run {shell_rel}/scripts/build{}.sh",
                base.display(),
                if *asset == "classes.dex" { "_dex" } else { "" },
                abis = abis.join(", "),
            ));
        }
    }
    Ok(format!("{} ({})", abis.join(", "), base.display()))
}

fn check_dependencies(pyproject: &str) {
    let doc: toml::Value = match pyproject.parse() {
        Ok(d) => d,
        Err(e) => {
            println!("  ! could not parse pyproject.toml: {e}");
            return;
        }
    };
    let deps: Vec<String> = doc
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|d| package_name(d) != "zkite")
        .collect();

    if deps.is_empty() {
        println!("  (no third-party dependencies declared)");
        return;
    }

    let Some(python) = wheels::find_host_python() else {
        println!("  ! no python3/python on PATH, cannot check dependency wheels");
        return;
    };
    let target = AndroidTarget { api_level: 24, abi: "arm64_v8a".into(), py_version: "314".into() };
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            println!("  ! could not create a scratch dir: {e}");
            return;
        }
    };

    for spec in &deps {
        match wheels::resolve_one(&python, &target, spec, dir.path()) {
            WheelOutcome::Resolved(w) if w.is_android_tagged => {
                println!("  ok    {spec}: real android-tagged wheel");
            }
            WheelOutcome::Resolved(w) if w.version_mismatch.is_some() => {
                let (resolved, latest) = w.version_mismatch.unwrap();
                println!(
                    "  WARN  {spec}: resolved {resolved} but latest is {latest} — likely a \
                     stale platform-compatible fallback, not real Android support \
                     (phase_2/PLAN.md §1.3)"
                );
            }
            WheelOutcome::Resolved(w) if w.is_pure_python => {
                println!("  ok    {spec}: pure-Python wheel, no native code involved");
            }
            WheelOutcome::Resolved(_) => {
                println!("  WARN  {spec}: resolved, but not android-tagged or pure-Python — verify it manually");
            }
            WheelOutcome::Failed { detail, .. } => {
                println!("  FAIL  {spec}: {detail}");
            }
        }
    }
}

fn package_name(spec: &str) -> String {
    spec.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.')
        .next()
        .unwrap_or(spec)
        .to_string()
}
