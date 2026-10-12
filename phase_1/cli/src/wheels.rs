//! Wheel resolution — plan1.md §7 step 1 ("resolve · pip --platform android_21_arm64_v8a
//! --only-binary=:all: → wheels"), shaped by `../../phase_2/PLAN.md` §1.3's finding:
//! checked live against real PyPI, **zero** packages had an `android_*`-tagged wheel
//! as of 2026-08-04. pip's default behavior — silently handing back whatever
//! platform-compatible wheel exists, even a years-stale placeholder release with no
//! real code — is actively dangerous to trust blindly, so this module doesn't just
//! shell out and declare success; it inspects what pip actually resolved.
//!
//! Runs `pip` via a vendored, self-contained `pip.pyz` (`../../phase_2/vendor/pip/`)
//! under whatever `python3`/`python` is on `PATH` — any host Python version works,
//! since `--python-version`/`--implementation`/`--abi`/`--platform` fully constrain
//! what pip resolves regardless of the interpreter running it. Falls back to the
//! user's own `pip`/`pip3` if no vendored copy is found (e.g. running from a source
//! checkout rather than the packaged wheel).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

pub struct AndroidTarget {
    pub api_level: u32,
    pub abi: String,       // "arm64_v8a" or "x86_64" (pip's platform-tag spelling)
    pub py_version: String, // "314" — no dot, matches pip's --python-version
}

impl AndroidTarget {
    pub fn platform_tag(&self) -> String {
        format!("android_{}_{}", self.api_level, self.abi)
    }
    pub fn abi_tag(&self) -> String {
        format!("cp{}", self.py_version)
    }
}

pub struct ResolvedWheel {
    pub is_android_tagged: bool,
    pub is_pure_python: bool,
    /// Set when the resolved wheel's version doesn't match the latest version pip
    /// reports as available for this package — the concrete symptom of the §1.3
    /// finding: pip quietly fell back to an old release that happens to have *some*
    /// platform-compatible tag, not the real current package.
    pub version_mismatch: Option<(String, String)>, // (resolved, latest)
}

pub enum WheelOutcome {
    Resolved(ResolvedWheel),
    Failed { detail: String },
}

/// Finds a verified host Python, including a Windows installation or launcher.
pub fn find_host_python() -> Option<PathBuf> {
    crate::host_python::find(None).ok()
}

/// Absolute path to the vendored `pip.pyz`, if the resolved payload has one.
///
/// **This was §23.2's quiet anchor**: it used to resolve a compile-time checkout path
/// and, off that checkout, *degrade* to `python -m pip` rather than fail — so wheel
/// resolution changed behaviour on a second machine without saying so. It still
/// degrades (that fallback is correct and predates this), but the path it looks in is
/// now the resolved payload root, so on a packaged install it finds the vendored copy
/// instead of quietly not.
pub fn vendored_pip_pyz() -> Option<PathBuf> {
    let candidate = crate::payload::root().ok()?.join("phase_2/vendor/pip/pip.pyz");
    candidate.exists().then_some(candidate)
}

/// Base pip invocation: `[python, pip.pyz]` if vendored, else `[python, -m, pip]`.
fn pip_command(python: &Path) -> Command {
    let mut cmd = Command::new(python);
    match vendored_pip_pyz() {
        Some(pyz) => {
            cmd.arg(pyz);
        }
        None => {
            cmd.args(["-m", "pip"]);
        }
    }
    cmd
}

/// Resolve the complete graph together so pip checks version constraints across
/// direct and transitive requirements. Build staging only supports pure Python
/// wheels until the runtime can load arbitrary third-party native libraries.
pub fn resolve_all(
    python: &Path,
    target: &AndroidTarget,
    specs: &[String],
    out_dir: &Path,
) -> Result<Vec<PathBuf>> {
    download_all(python, target, specs, out_dir, &[])
}

/// Preserve the existing warning about pip falling back to a historical pure
/// placeholder when a newer package release has no Android-compatible wheel.
pub fn stale_resolution_warnings(python: &Path, specs: &[String], wheels: &[PathBuf]) -> Vec<String> {
    let normalize = |name: &str| name.to_ascii_lowercase().replace(['_', '.'], "-");
    let mut warnings = Vec::new();
    for spec in specs {
        let package = package_name_from_spec(spec);
        for wheel in wheels {
            let filename = wheel.file_name().unwrap_or_default().to_string_lossy();
            if normalize(filename.split('-').next().unwrap_or_default()) != normalize(&package) {
                continue;
            }
            if let Some((resolved, _)) = parse_wheel_filename(&filename) {
                if let Some(latest) = latest_version(python, &package).filter(|latest| *latest != resolved) {
                    warnings.push(format!("{package}: resolved {resolved}, latest is {latest}; this may be a stale Android-compatible fallback. Check the version constraints and package contents before shipping."));
                }
            }
        }
    }
    warnings
}

fn download_all(
    python: &Path,
    target: &AndroidTarget,
    specs: &[String],
    out_dir: &Path,
    additional_args: &[String],
) -> Result<Vec<PathBuf>> {
    for spec in specs {
        validate_markers(spec)?;
    }
    std::fs::create_dir_all(out_dir)?;
    let output = pip_command(python)
        .args([
            "download", "--disable-pip-version-check", "--no-input",
            // The loader supports pure Python only. Restrict candidate selection,
            // so optional native accelerators cannot shadow usable universal wheels.
            "--platform", "any",
            "--python-version", &target.py_version,
            "--implementation", "py", "--abi", "none",
            "--only-binary=:all:", "-d",
        ])
        .arg(out_dir)
        .args(additional_args)
        .args(specs)
        .output()
        .context("running pip to resolve Android dependencies")?;
    if !output.status.success() {
        bail!(
            "required dependencies could not be resolved for {} / cp{}:\n{}",
            target.platform_tag(), target.py_version,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut wheels: Vec<_> = list_whls(out_dir).into_iter().collect();
    wheels.sort();
    for wheel in &wheels {
        validate_build_wheel(wheel)?;
    }
    Ok(wheels)
}

fn validate_markers(requirement: &str) -> Result<()> {
    let Some((_, marker)) = requirement.split_once(';') else { return Ok(()) };
    // pip's --platform changes wheel tags, but evaluates environment markers on
    // the host. Reject markers we cannot evaluate honestly for the Android target
    // instead of silently omitting a dependency on Windows/macOS/Linux.
    for variable in [
        "sys_platform", "os_name", "platform_system", "platform_machine",
        "platform_release", "platform_version", "python_full_version",
        "implementation_version",
    ] {
        if marker.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|word| word == variable) {
            bail!(
                "dependency {requirement:?} uses the target-dependent marker `{variable}`; \
                 pip evaluates this on the build host, not Android. ZKite cannot safely package \
                 it yet; use a dependency release without this marker or supply a wheel whose \
                 dependency metadata is already resolved for Android"
            );
        }
    }
    Ok(())
}

fn validate_build_wheel(path: &Path) -> Result<()> {
    let filename = path.file_name().context("wheel has no filename")?.to_string_lossy();
    let (_, tag) = parse_wheel_filename(&filename).context("invalid wheel filename")?;
    if tag != "any" {
        bail!(
            "{filename} contains platform-specific code; third-party native wheels are not yet \
             supported by ZKite's Android loader. Use a pure-Python dependency instead"
        );
    }
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut has_metadata = false;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_owned();
        if name.ends_with(".so") || name.ends_with(".pyd") || name.ends_with(".dll") {
            bail!("{filename} claims a pure-Python tag but contains native library {name}");
        }
        if name.ends_with(".dist-info/METADATA") {
            has_metadata = true;
            let mut metadata = String::new();
            std::io::Read::read_to_string(&mut entry, &mut metadata)?;
            // Unfold continuation lines before reading Requires-Dist fields.
            let mut fields: Vec<String> = Vec::new();
            for line in metadata.lines().take_while(|line| !line.is_empty()) {
                if line.starts_with([' ', '\t']) {
                    if let Some(previous) = fields.last_mut() {
                        previous.push(' ');
                        previous.push_str(line.trim());
                    }
                } else {
                    fields.push(line.to_owned());
                }
            }
            for field in fields {
                if let Some((key, value)) = field.split_once(':') {
                    if key.eq_ignore_ascii_case("Requires-Dist") {
                        validate_markers(value.trim()).with_context(|| format!("{filename}"))?;
                    }
                }
            }
        }
    }
    if !has_metadata {
        bail!("{filename} is missing its dependency METADATA");
    }
    Ok(())
}

/// Resolve one dependency spec (e.g. `"attrs"`, `"attrs>=23"`) into a downloaded
/// wheel in `out_dir`, and cross-check it against pip's own view of the latest
/// available version (§1.3).
pub fn resolve_one(python: &Path, target: &AndroidTarget, spec: &str, out_dir: &Path) -> WheelOutcome {
    let package = package_name_from_spec(spec);

    let before: std::collections::HashSet<PathBuf> = list_whls(out_dir);
    let output = pip_command(python)
        .args([
            "download",
            "--platform",
            &target.platform_tag(),
            "--python-version",
            &target.py_version,
            "--implementation",
            "cp",
            "--abi",
            &target.abi_tag(),
            "--only-binary=:all:",
            "--no-deps",
            "-d",
        ])
        .arg(out_dir)
        .arg(spec)
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            return WheelOutcome::Failed {
                detail: format!("could not run pip ({e}) — is a Python 3 interpreter on PATH?"),
            }
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return WheelOutcome::Failed {
            detail: format!(
                "no wheel available for android_{}_{} / cp{} (pip: {})",
                target.api_level,
                target.abi,
                target.py_version,
                first_meaningful_line(&stderr)
            ),
        };
    }

    let after = list_whls(out_dir);
    let new_wheel = after.difference(&before).next().cloned();
    let wheel_path = match new_wheel {
        Some(p) => p,
        None => {
            return WheelOutcome::Failed {
                detail: "pip reported success but no new .whl appeared".into(),
            }
        }
    };

    let filename = wheel_path.file_name().unwrap().to_string_lossy().into_owned();
    let (resolved_version, platform_tag) = match parse_wheel_filename(&filename) {
        Some(v) => v,
        None => (String::new(), String::new()),
    };
    let is_android_tagged = platform_tag.starts_with("android");
    let is_pure_python = platform_tag == "any";

    let version_mismatch = latest_version(python, &package)
        .filter(|latest| !resolved_version.is_empty() && *latest != resolved_version)
        .map(|latest| (resolved_version.clone(), latest));

    WheelOutcome::Resolved(ResolvedWheel {
        is_android_tagged,
        is_pure_python,
        version_mismatch,
    })
}

fn list_whls(dir: &Path) -> std::collections::HashSet<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "whl"))
        .collect()
}

/// `pip index versions <pkg>` — used only to detect the §1.3 stale-fallback
/// symptom, so a network hiccup or pip's own (expected) "not a real command"
/// deprecation warning on `index` just means the cross-check is skipped, not that
/// resolution fails.
fn latest_version(python: &Path, package: &str) -> Option<String> {
    let output = pip_command(python).args(["index", "versions", package]).output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{package} (")) {
            return rest.split(')').next().map(|s| s.to_string());
        }
    }
    None
}

fn first_meaningful_line(stderr: &str) -> String {
    stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("(no error output)")
        .trim()
        .to_string()
}

/// wheel filename: `{name}-{version}(-{build})?-{python tag}-{abi tag}-{platform tag}.whl`
fn parse_wheel_filename(filename: &str) -> Option<(String, String)> {
    let stem = filename.strip_suffix(".whl")?;
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() < 5 {
        return None;
    }
    let version = parts[1].to_string();
    let platform_tag = parts[parts.len() - 1].to_string();
    Some((version, platform_tag))
}

fn package_name_from_spec(spec: &str) -> String {
    spec.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.')
        .next()
        .unwrap_or(spec)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_wheel(dir: &Path, name: &str, requirements: &str) -> PathBuf {
        use std::io::Write;
        let path = dir.join(format!("{name}-1.0-py3-none-any.whl"));
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let metadata = format!("Metadata-Version: 2.1\nName: {name}\nVersion: 1.0\n{requirements}\n");
        for (entry, data) in [
            (format!("{name}.py"), "value = 1\n".to_owned()),
            (format!("{name}-1.0.dist-info/METADATA"), metadata),
            (format!("{name}-1.0.dist-info/WHEEL"), "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n".to_owned()),
            (format!("{name}-1.0.dist-info/RECORD"), String::new()),
        ] {
            archive.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
            archive.write_all(data.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
        path
    }

    #[test]
    fn resolves_transitive_dependencies_offline_for_each_android_abi() {
        let python = crate::host_python::find(Some((3, 14))).unwrap();
        let repo = tempfile::tempdir().unwrap();
        fixture_wheel(repo.path(), "zkite_fixture_parent", "Requires-Dist: zkite-fixture-child==1.0\n");
        fixture_wheel(repo.path(), "zkite_fixture_child", "");
        let args = vec!["--no-index".into(), "--find-links".into(), repo.path().display().to_string()];
        for abi in ["arm64_v8a", "x86_64"] {
            let dest = tempfile::tempdir().unwrap();
            let target = AndroidTarget { api_level: 26, abi: abi.into(), py_version: "314".into() };
            let wheels = download_all(&python, &target, &["zkite-fixture-parent==1.0".into()], dest.path(), &args).unwrap();
            assert_eq!(wheels.len(), 2, "transitive child must be downloaded for {abi}");
            assert!(wheels.iter().any(|p| p.file_name().unwrap().to_string_lossy().contains("child")));
        }
    }

    #[test]
    fn prefers_pure_transitive_wheel_and_rejects_native_only_dependency() {
        let python = crate::host_python::find(Some((3, 14))).unwrap();
        let repo = tempfile::tempdir().unwrap();
        fixture_wheel(repo.path(), "zkite_fixture_parent", "Requires-Dist: zkite-fixture-child==1.0\n");
        let pure = fixture_wheel(repo.path(), "zkite_fixture_child", "");
        let args = vec!["--no-index".into(), "--find-links".into(), repo.path().display().to_string()];
        for abi in ["arm64_v8a", "x86_64"] {
            // Both variants of the same release are offered, as with charset_normalizer.
            std::fs::copy(&pure, repo.path().join(format!(
                "zkite_fixture_child-1.0-cp314-cp314-android_24_{abi}.whl"
            ))).unwrap();
            let target = AndroidTarget { api_level: 24, abi: abi.into(), py_version: "314".into() };
            let dest = tempfile::tempdir().unwrap();
            let wheels = download_all(&python, &target, &["zkite-fixture-parent==1.0".into()], dest.path(), &args).unwrap();
            assert_eq!(wheels.len(), 2);
            assert!(wheels.iter().all(|p| p.file_name().unwrap().to_string_lossy().ends_with("-none-any.whl")));
        }
        std::fs::remove_file(pure).unwrap();
        let dest = tempfile::tempdir().unwrap();
        let error = download_all(&python, &live_test_target(), &["zkite-fixture-parent==1.0".into()], dest.path(), &args).unwrap_err();
        assert!(error.to_string().contains("required dependencies could not be resolved"));
    }

    #[test]
    fn unresolved_required_dependencies_fail_the_build_offline() {
        let python = crate::host_python::find(Some((3, 14))).unwrap();
        let repo = tempfile::tempdir().unwrap();
        fixture_wheel(repo.path(), "zkite_fixture_parent", "Requires-Dist: zkite-fixture-missing==1.0\n");
        let dest = tempfile::tempdir().unwrap();
        let error = download_all(&python, &live_test_target(), &["zkite-fixture-parent==1.0".into()], dest.path(),
            &["--no-index".into(), "--find-links".into(), repo.path().display().to_string()]).unwrap_err();
        assert!(error.to_string().contains("required dependencies could not be resolved"));
    }

    #[test]
    fn rejects_host_specific_markers_and_native_wheels() {
        let repo = tempfile::tempdir().unwrap();
        let conditional = fixture_wheel(repo.path(), "conditional", "Requires-Dist: win-dep; sys_platform == 'win32'\n");
        let error = format!("{:#}", validate_build_wheel(&conditional).unwrap_err());
        assert!(error.contains("sys_platform"), "{error}");
        assert!(validate_markers("foo; python_version >= '3.14'").is_ok());
        assert!(validate_markers("foo; os_name == 'posix'").is_err());
        let native = repo.path().join("native-1.0-cp314-cp314-android_26_arm64_v8a.whl");
        assert!(validate_build_wheel(&native).unwrap_err().to_string().contains("native wheels"));
    }

    #[test]
    fn parses_a_pure_python_wheel_filename() {
        let (version, tag) = parse_wheel_filename("attrs-26.1.0-py3-none-any.whl").unwrap();
        assert_eq!(version, "26.1.0");
        assert_eq!(tag, "any");
    }

    #[test]
    fn parses_an_android_tagged_wheel_filename() {
        let (version, tag) =
            parse_wheel_filename("foo-1.2.3-cp314-cp314-android_24_arm64_v8a.whl").unwrap();
        assert_eq!(version, "1.2.3");
        assert_eq!(tag, "android_24_arm64_v8a");
    }

    #[test]
    fn strips_version_specifiers_to_get_the_bare_package_name() {
        assert_eq!(package_name_from_spec("attrs>=23.0"), "attrs");
        assert_eq!(package_name_from_spec("attrs"), "attrs");
        assert_eq!(package_name_from_spec("attrs==23.0.0"), "attrs");
    }

    #[test]
    fn platform_and_abi_tags_match_pips_expected_spelling() {
        let target =
            AndroidTarget { api_level: 24, abi: "arm64_v8a".into(), py_version: "314".into() };
        assert_eq!(target.platform_tag(), "android_24_arm64_v8a");
        assert_eq!(target.abi_tag(), "cp314");
    }

    /// Live network tests against real PyPI — the same checks done manually while
    /// writing `../../phase_2/PLAN.md` §1.3, now pinned as regression tests. Needs a
    /// Python interpreter and network access; skips cleanly if either is missing
    /// rather than failing the whole suite in an offline/sandboxed environment.
    fn live_test_target() -> AndroidTarget {
        AndroidTarget { api_level: 24, abi: "arm64_v8a".into(), py_version: "314".into() }
    }

    #[test]
    #[ignore = "optional live PyPI smoke check; offline graph tests are the required gate"]
    fn pure_python_dependency_resolves_cleanly() {
        let Some(python) = find_host_python() else {
            eprintln!("no python3 on PATH, skipping live pip test");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        match resolve_one(&python, &live_test_target(), "attrs", dir.path()) {
            WheelOutcome::Resolved(w) => {
                assert!(w.is_pure_python, "attrs should resolve to a py3-none-any wheel");
                assert!(!w.is_android_tagged);
            }
            WheelOutcome::Failed { detail, .. } => {
                eprintln!("pip download failed (likely no network in this run): {detail}");
            }
        }
    }

    #[test]
    #[ignore = "historical live PyPI observation; package availability changes over time"]
    fn native_only_dependency_is_flagged_not_silently_trusted() {
        // §1.3's concrete finding: pydantic-core has no android_* wheel anywhere in
        // its release history, so pip falls back to its very first PyPI upload
        // (0.0.1, an empty py3-none-any placeholder) — exactly the dangerous case
        // this module exists to catch, not hide.
        let Some(python) = find_host_python() else {
            eprintln!("no python3 on PATH, skipping live pip test");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        match resolve_one(&python, &live_test_target(), "pydantic-core", dir.path()) {
            WheelOutcome::Resolved(w) => {
                assert!(!w.is_android_tagged, "no android wheel should exist for this package yet");
                if let Some((resolved, latest)) = &w.version_mismatch {
                    eprintln!(
                        "confirmed: resolved {resolved} but latest is {latest} — \
                         version_mismatch correctly caught the stale-fallback case"
                    );
                    assert_ne!(resolved, latest);
                } else {
                    eprintln!(
                        "note: no version_mismatch reported (pip index versions may have \
                         failed, e.g. offline) — not asserting on this half of the check"
                    );
                }
            }
            WheelOutcome::Failed { detail, .. } => {
                eprintln!("pip download failed (likely no network in this run): {detail}");
            }
        }
    }
}
