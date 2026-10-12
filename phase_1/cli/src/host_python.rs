//! Host interpreter discovery shared by build, dependency resolution and doctor.
//!
//! Run the interpreter to identify it: an executable named `python3.14` need not
//! actually be CPython 3.14, and Windows normally has `python.exe` or `py.exe`.
//! Return `sys.executable` so launchers never need to be threaded through callers.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PROBE: &str = "import json,sys; print(json.dumps({'version':list(sys.version_info[:2]),'implementation':sys.implementation.name,'executable':sys.executable}))";

#[derive(Deserialize)]
struct Interpreter {
    version: (u8, u8),
    implementation: String,
    executable: PathBuf,
}

fn parse_probe(bytes: &[u8], required: Option<(u8, u8)>) -> Option<PathBuf> {
    let interpreter: Interpreter = serde_json::from_slice(bytes).ok()?;
    if interpreter.implementation != "cpython"
        || interpreter.version.0 != 3
        || required.is_some_and(|version| version != interpreter.version)
        || interpreter.executable.as_os_str().is_empty()
    {
        return None;
    }
    Some(interpreter.executable)
}

fn probe(program: &Path, args: &[String], required: Option<(u8, u8)>) -> Option<PathBuf> {
    let output = Command::new(program)
        .args(args)
        .args(["-I", "-S", "-c", PROBE])
        .stdin(Stdio::null())
        // The legacy Windows launcher otherwise permits install-on-demand when
        // PYLAUNCHER_ALLOW_INSTALL is set in the environment.
        .env_remove("PYLAUNCHER_ALLOW_INSTALL")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let executable = parse_probe(&output.stdout, required)?;
    executable.is_file().then_some(executable)
}

fn prefix_python(prefix: &Path) -> PathBuf {
    if cfg!(windows) {
        prefix.join("Scripts/python.exe")
    } else {
        prefix.join("bin/python")
    }
}

/// Find a working CPython interpreter, requiring an exact major/minor version
/// when compiling bytecode or analysing the Android stdlib. `ZKITE_PYTHON` is an
/// explicit executable path (not a shell command) and is never silently ignored.
pub fn find(required: Option<(u8, u8)>) -> Result<PathBuf> {
    let wanted = required
        .map(|(major, minor)| format!("CPython {major}.{minor}"))
        .unwrap_or_else(|| "CPython 3".into());
    if let Some(explicit) = std::env::var_os("ZKITE_PYTHON") {
        let path = PathBuf::from(explicit);
        return probe(&path, &[], required).with_context(|| {
            format!(
                "ZKITE_PYTHON={} could not run as {wanted}; set it to a matching Python executable",
                path.display()
            )
        });
    }

    // Prefer the environment that installed ZKite, even if its scripts directory
    // is not on PATH. This also avoids selecting a different global Python for pip.
    let mut candidates = Vec::new();
    if let Some(venv) = std::env::var_os("VIRTUAL_ENV") {
        candidates.push(prefix_python(Path::new(&venv)));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(if cfg!(windows) { "python.exe" } else { "python" }));
            // A system Windows install places python.exe at the prefix root,
            // whereas a virtualenv places it in Scripts beside zkite.exe.
            if cfg!(windows) {
                if let Some(prefix) = dir.parent() {
                    candidates.push(prefix.join("python.exe"));
                }
            }
        }
    }
    for candidate in candidates {
        if let Some(python) = probe(&candidate, &[], required) {
            return Ok(python);
        }
    }

    // The launcher is the usual version-selecting entry point on Windows; probe
    // it before python.exe, which may be a Microsoft Store installation alias.
    if cfg!(windows) {
        let selector = required
            .map(|(major, minor)| format!("-{major}.{minor}"))
            .unwrap_or_else(|| "-3".into());
        if let Some(python) = probe(Path::new("py"), &[selector], required) {
            return Ok(python);
        }
    }
    let mut names = Vec::new();
    if let Some((major, minor)) = required {
        names.push(format!("python{major}.{minor}"));
    }
    names.extend(["python3".into(), "python".into()]);
    for name in names {
        // Let the OS resolve executable suffixes and PATH, rather than looking
        // for a literal file named "python" (which cannot find python.exe).
        if let Some(python) = probe(Path::new(&name), &[], required) {
            return Ok(python);
        }
    }

    if let Some((major, minor)) = required {
        if let Ok(output) = Command::new("uv")
            .args(["python", "find", "--no-python-downloads", &format!("{major}.{minor}")])
            .stdin(Stdio::null())
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout);
                if let Some(python) = probe(Path::new(path.trim()), &[], required) {
                    return Ok(python);
                }
            }
        }
    }

    bail!(
        "no working {wanted} interpreter found; install {wanted}, activate its virtual environment, \
         or set ZKITE_PYTHON to its executable (Windows: the `py` launcher is supported)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_actual_version_and_implementation() {
        let python = br#"{"version":[3,14],"implementation":"cpython","executable":"C:\\Program Files\\Python314\\python.exe"}"#;
        assert_eq!(
            parse_probe(python, Some((3, 14))),
            Some(PathBuf::from(r"C:\Program Files\Python314\python.exe"))
        );
        assert!(parse_probe(python, Some((3, 13))).is_none());
        assert!(parse_probe(
            br#"{"version":[3,14],"implementation":"pypy","executable":"python"}"#,
            Some((3, 14))
        )
        .is_none());
        assert!(parse_probe(b"Microsoft Store alias", None).is_none());
    }

    #[test]
    fn rejects_missing_or_empty_interpreters() {
        assert!(probe(Path::new("zkite-nonexistent-python-for-test"), &[], None).is_none());
        assert!(parse_probe(
            br#"{"version":[3,14],"implementation":"cpython","executable":""}"#,
            None
        )
        .is_none());
    }

    #[test]
    fn discovers_a_runnable_python_and_rejects_a_wrong_version() {
        let python = find(None).expect("host tests require a CPython 3 interpreter");
        assert!(probe(&python, &[], None).is_some());
        assert!(probe(&python, &[], Some((0, 0))).is_none());
    }
}
