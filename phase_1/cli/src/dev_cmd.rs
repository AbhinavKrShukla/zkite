//! `zkite dev` (M5, see ../../PLAN.md): the host side of the reload loop. The device
//! side (`Native.startReloadServer`, `devshell/src/lib.rs`) dials `127.0.0.1:<port>`,
//! reached via `adb reverse tcp:<port> tcp:<port>` — so this process is the TCP
//! *server*, matching ../../PLAN.md §4's reasoning for why `adb reverse` (not
//! `adb forward`) is the right direction.
//!
//! Frame wire format is newline-delimited JSON, matching devshell/src/lib.rs's
//! `Frame` enum exactly — both sides are Rust, so keeping the shapes textually
//! identical (rather than sharing a crate, which would mean cross-compiling this CLI
//! for Android too) is the cheapest way to keep them honest.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use crate::device::Device;
use std::time::{Duration, Instant};

use crate::manifest::{Change, Manifest};
use crate::watch::ProjectWatcher;

use crate::build_cmd::Backend;

/// Which Dev Shell to talk to. The reload *protocol* is identical on both backends —
/// `phase_3/gpu_harness/src/reload.rs` is wire-compatible with
/// `phase_1/devshell`'s on purpose — so the only per-backend facts are which package
/// to launch and which prebuilt APK to install. See `phase_3/PLAN.md` §15.
struct DevShell {
    package: &'static str,
    activity: &'static str,
    /// Payload-relative APK path with an ABI placeholder. The installed payload
    /// and contributor checkout use the same directory layout.
    artifact_rel: &'static str,
}

/// The package of the backend that is *not* `expected` — i.e. the one to force-stop
/// when a mismatch is reported.
fn other_package(expected: Backend) -> &'static str {
    dev_shell(match expected {
        Backend::Native => Backend::WebView,
        Backend::WebView => Backend::Native,
    })
    .package
}

fn dev_shell(backend: Backend) -> DevShell {
    match backend {
        Backend::Native => DevShell {
            package: "dev.zkite.gpuharness",
            activity: "dev.zkite.gpuharness/.ZKiteActivity",
            artifact_rel: "phase_3/gpu_harness/artifacts/zkite-native-devshell-{abi}.apk",
        },
        Backend::WebView => DevShell {
            package: "dev.zkite.devshell",
            activity: "dev.zkite.devshell/.ZKiteActivity",
            artifact_rel: "phase_1/devshell/artifacts/zkite-devshell-{abi}.apk",
        },
    }
}

/// Explicit paths are authoritative: a typo must not silently pick an older APK.
fn find_devshell_apk(shell: &DevShell, abi: &str, explicit: Option<&Path>) -> Result<Option<std::path::PathBuf>> {
    let env = std::env::var_os("ZKITE_DEVSHELL_APK").map(std::path::PathBuf::from);
    if let Some(path) = explicit.or(env.as_deref()) {
        anyhow::ensure!(path.is_file(), "Dev Shell APK does not exist: {}", path.display());
        return Ok(Some(path.to_path_buf()));
    }
    let root = crate::payload::root()?;
    let candidate = root.join(shell.artifact_rel.replace("{abi}", abi));
    Ok(candidate.is_file().then_some(candidate))
}

/// The `zkite` package fingerprint of the payload this `zkite dev` is running from —
/// the host half of `runtime.py::_zkite_framework_fingerprint` (`phase_3/PLAN.md`
/// §31.6). The two must produce byte-identical digests over the same sources, so the
/// canonical stream is defined once, here and in that function's docstring:
///
/// > for each `*.py` directly in `zkite/`, sorted by name:
/// > `name` · `NUL` · file bytes · `NUL`; sha256; first 12 hex characters.
///
/// Only this directory's own `.py` files — `zkite` has no subpackages, and
/// `__pycache__` must not be part of an identity that has to match what a Dev Shell
/// unpacked out of `zkite-framework.zip`.
///
/// Returns `None` on any failure, which the caller turns into silence rather than a
/// mismatch: a false "your Dev Shell is stale" costs someone a pointless rebuild.
pub(crate) fn framework_fingerprint() -> Option<String> {
    fingerprint_of(&crate::payload::root().ok()?.join("phase_1/zkite_py/src/zkite"))
}

/// The digest itself, over one directory — split out so the algorithm is testable
/// without a payload root, which is what pins it against `runtime.py`'s twin.
fn fingerprint_of(dir: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".py"))
        .collect();
    names.sort();
    if names.is_empty() {
        return None;
    }
    let mut h = Sha256::new();
    for name in &names {
        let body = std::fs::read(dir.join(name)).ok()?;
        h.update(name.as_bytes());
        h.update(b"\0");
        h.update(&body);
        h.update(b"\0");
    }
    Some(format!("{:x}", h.finalize())[..12].to_string())
}

pub struct DevArgs {
    pub project_dir: std::path::PathBuf,
    pub port: u16,
    pub devshell_apk: Option<std::path::PathBuf>,
    pub adb: String,
    /// `--backend`. Defaults to [`Backend::Native`] (`phase_3/PLAN.md` §15.4).
    pub backend: Option<Backend>,
    pub serial: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Frame<'a> {
    Sync { paths: Vec<&'a str> },
    File { path: &'a str, content_b64: String },
    Reload,
}

/// What the device sends back — the counterpart of `devshell/src/lib.rs`'s
/// `OutFrame`. A bare `{"ok":true}` ack (no "op" field) fails to deserialize as this
/// and is silently ignored, which is fine — it exists only to keep the connection's
/// read side alive, not to carry information.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum InFrame {
    /// Sent once, immediately on connect, naming which backend actually dialled in.
    /// Added after `--backend native` silently served a still-running *WebView* Dev
    /// Shell: the reload protocol is identical on both backends and they share port
    /// 8765, so without this there is nothing in the wire format that distinguishes
    /// them, and `--backend` only ever selected which package to *launch*. See
    /// `phase_3/PLAN.md` §15.6.
    ///
    /// `framework` is the shell's own `zkite` package fingerprint, absent on any Dev
    /// Shell built before §31.6 — hence `Option` and `#[serde(default)]`, so an old
    /// shell still connects and is simply not checked.
    Hello {
        backend: String,
        #[serde(default)]
        framework: Option<String>,
        #[serde(default)]
        sync_files: bool,
    },
    Log { text: String },
    Error { text: String },
}

// Shipped shells dial this device-side port. A custom --port changes only the
// host side of adb reverse; it does not require rebuilding either Android shell.
const DEVICE_RELOAD_PORT: u16 = 8765;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

fn reverse_ports(host_port: u16) -> [String; 3] {
    ["reverse".into(), format!("tcp:{DEVICE_RELOAD_PORT}"), format!("tcp:{host_port}")]
}

pub fn run(args: DevArgs) -> Result<()> {
    let backend = crate::build_cmd::project_backend(&args.project_dir, args.backend)?;
    anyhow::ensure!(args.port != 0, "--port must be between 1 and 65535");
    let device = Device::connect(&args.adb, args.serial.as_deref())?;
    let abi = device.abi()?;
    let shell = dev_shell(backend);
    println!("zkite dev on the {} backend ({}) on {}", backend.name(), shell.package, device.serial);

    // Bind before installing or launching so an occupied port has no device side
    // effects. Keep the reverse mapping independent of the shell's fixed port.
    let listener = TcpListener::bind(("127.0.0.1", args.port))
        .with_context(|| format!("binding 127.0.0.1:{}; choose another --port if it is occupied", args.port))?;
    listener.set_nonblocking(true)?;
    let ports = reverse_ports(args.port);
    device.output(&[&ports[0], &ports[1], &ports[2]])?;

    let explicit = args.devshell_apk.is_some() || std::env::var_os("ZKITE_DEVSHELL_APK").is_some();
    let preview = if !explicit {
        let source = find_devshell_apk(&shell, &abi, None)?
            .context("no prebuilt Dev Shell in the installed payload; run zkite doctor")?;
        Some(crate::preview::prepare(&args.project_dir, backend, &abi, &source)?)
    } else { None };
    if let Some(preview) = &preview {
        // Only framework-owned preview packages are stopped. A previous project's
        // retrying reload socket must not receive this project's source files.
        let packages = device.output(&["shell", "pm", "list", "packages", "dev.zkite.preview."])?;
        for package in String::from_utf8_lossy(&packages.stdout).lines()
            .filter_map(|line| line.trim().strip_prefix("package:dev.zkite.preview.")) {
            if package.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_')) {
                device.output(&["shell", "am", "force-stop", &format!("dev.zkite.preview.{package}")])?;
            }
        }
        device.output(&["shell", "am", "force-stop", shell.package])?;
        device.install(&preview.apk)?;
    } else if explicit || !device.installed(shell.package)? {
        let apk = find_devshell_apk(&shell, &abi, args.devshell_apk.as_deref())?
            .with_context(|| format!(
                "no {} Dev Shell APK for {abi} in the installed payload. Install a ZKite distribution \
                 containing Dev Shells, or provide a matching prebuilt APK with \
                 zkite dev --devshell-apk <path>. Run zkite doctor to inspect the payload",
                backend.name()))?;
        device.install(&apk)?;
    }
    // Stop a previous renderer before either can attach to the shared reload port.
    device.output(&["shell", "am", "force-stop", other_package(backend)])?;
    device.launch(preview.as_ref().map(|p| p.component.as_str()).unwrap_or(shell.activity))?;
    if let Some(preview) = &preview {
        println!("  project preview: {} (your icon and label; separate app data)", preview.package);
        println!("  rerun zkite dev after changing the launcher icon or Android configuration");
    } else {
        println!("  using the identity embedded in the explicit Dev Shell APK");
    }

    loop {
        println!("listening on 127.0.0.1:{} — waiting for the Dev Shell…", args.port);
        let stream = wait_for_connection(&listener, CONNECT_TIMEOUT, || {
            device.output(&[&ports[0], &ports[1], &ports[2]]).map(|_| ())
        })?;
        if let Err(error) = serve_connection(stream, &args.project_dir, backend, &abi) {
            eprintln!("connection ended: {error}");
        }
    }
}

/// Re-establish reverse mappings during USB reconnection, with a deadline even
/// when the shell is missing or has crashed before opening its reload socket.
fn wait_for_connection(
    listener: &TcpListener,
    timeout: Duration,
    mut restore_reverse: impl FnMut() -> Result<()>,
) -> Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let mut reverse_due = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                return Ok(stream);
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
            Err(error) => return Err(error).context("accepting Dev Shell connection"),
        }
        anyhow::ensure!(Instant::now() < deadline,
            "Dev Shell did not connect within {} seconds. Check the device screen and adb connection. \
             Reinstall a matching shell with zkite dev --devshell-apk <path> if it fails to start",
            timeout.as_secs());
        if Instant::now() >= reverse_due {
            if let Err(error) = restore_reverse() {
                eprintln!("waiting for the device: {error}");
            }
            reverse_due = Instant::now() + Duration::from_secs(2);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Compare the Dev Shell's `zkite` fingerprint against the payload's, and say so when
/// they differ (`phase_3/PLAN.md` §31.6).
///
/// Silent unless there is something to report, and silent when either side is `None`:
/// an older Dev Shell sends no `framework` at all, and a payload whose `zkite/` cannot
/// be read is this process's problem, not the user's. **A false staleness report costs
/// a real rebuild**, so every uncertain case resolves to saying nothing.
///
/// Framework skew remains a warning because application-only edits can still run.
/// A renderer mismatch is checked separately and always rejects the connection.
fn report_framework_skew(shell: Option<&str>, payload: Option<&str>, expected: Backend, abi: &str) {
    let (Some(shell), Some(payload)) = (shell, payload) else { return };
    if shell == payload {
        return;
    }
    eprintln!();
    eprintln!("  !! STALE DEV SHELL — its zkite framework differs from the installed payload.");
    eprintln!("  !! Dev Shell: {shell}; payload: {payload}");
    eprintln!("  !! Install the matching {} APK for {abi} with zkite dev --devshell-apk <path>.", expected.name());
    eprintln!("  !! Framework changes require an updated Dev Shell; app changes still reload live.");
    eprintln!();
}

struct CloseConnection(TcpStream);

impl Drop for CloseConnection {
    fn drop(&mut self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

fn serve_connection(stream: TcpStream, project_dir: &Path, expected: Backend, abi: &str) -> Result<()> {
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

    let _close = CloseConnection(stream.try_clone()?);
    stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut hello = String::new();
    reader.read_line(&mut hello).context("waiting for the Dev Shell handshake; update the shell if it does not identify its backend")?;
    let (framework, sync_files) = match serde_json::from_str::<InFrame>(hello.trim()) {
        Ok(InFrame::Hello { backend, framework, sync_files }) if backend == expected.name() => (framework, sync_files),
        Ok(InFrame::Hello { backend, .. }) => anyhow::bail!(
            "backend mismatch: expected {}, connected shell is {backend}; no project files were sent", expected.name()),
        _ => anyhow::bail!("Dev Shell sent no valid handshake; install a matching prebuilt Dev Shell"),
    };
    stream.set_read_timeout(None)?;
    report_framework_skew(framework.as_deref(), framework_fingerprint().as_deref(), expected, abi);
    println!("{} Dev Shell connected — pushing the full project", expected.name());
    if !sync_files {
        eprintln!("  ! This Dev Shell predates file reconciliation. Install an updated Dev Shell to remove deleted files and switch projects cleanly.");
    }

    // Arm the watcher before scanning/pushing; saves during the initial transfer
    // must be observed rather than becoming its unnoticed starting snapshot.
    let mut watcher = ProjectWatcher::new(project_dir)?;
    let mut writer = stream;
    let mut last = Manifest::default();
    let fresh = Manifest::scan(project_dir)?;
    push_changes(&mut writer, project_dir, &last, &fresh, sync_files)?;
    last = fresh;

    let connected = Arc::new(AtomicBool::new(true));
    let reader_connected = connected.clone();
    std::thread::spawn(move || {
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            match serde_json::from_str::<InFrame>(line.trim()) {
                Ok(InFrame::Log { text }) => {
                    for line in text.lines().filter(|line| !line.is_empty()) {
                        println!("  [device] {line}");
                    }
                }
                Ok(InFrame::Error { text }) => eprintln!("  [device error]\n{text}"),
                _ => {},
            }
            line.clear();
        }
        reader_connected.store(false, Ordering::Release);
    });

    loop {
        anyhow::ensure!(connected.load(Ordering::Acquire), "Dev Shell disconnected; waiting to reconnect");
        let changes = watcher.next_change_batch(Duration::from_millis(250))?;
        if changes.is_empty() {
            continue;
        }
        let t0 = Instant::now();
        let fresh = Manifest::scan(project_dir)?;
        push_changes(&mut writer, project_dir, &last, &fresh, sync_files)?;
        last = fresh;
        println!("  pushed {} change(s) in {:.0}ms: {}", changes.len(),
            t0.elapsed().as_secs_f64() * 1000.0, summarize(&changes));
    }
}

fn summarize(changes: &[Change]) -> String {
    changes
        .iter()
        .map(|c| match c {
            Change::Added(p) => format!("+{p}"),
            Change::Modified(p) => format!("~{p}"),
            Change::Removed(p) => format!("-{p}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Reconcile the complete managed path set, push changed bytes, then reload once.
/// Older shells omit sync_files in their hello and retain the original protocol.
fn push_changes(writer: &mut impl Write, project_dir: &Path, old: &Manifest, new: &Manifest, sync_files: bool) -> Result<()> {
    let changes = old.diff(new);
    if sync_files {
        let frame = Frame::Sync { paths: new.files.keys().map(String::as_str).collect() };
        writeln!(writer, "{}", serde_json::to_string(&frame)?)?;
    }
    for change in &changes {
        if let Change::Added(path) | Change::Modified(path) = change {
            let bytes = std::fs::read(project_dir.join(path))
                .with_context(|| format!("reading {path}"))?;
            let frame = Frame::File { path, content_b64: base64_encode(&bytes) };
            writeln!(writer, "{}", serde_json::to_string(&frame)?)?;
        }
    }
    if !changes.is_empty() || old.files.is_empty() {
        writeln!(writer, "{}", serde_json::to_string(&Frame::Reload)?)?;
    }
    Ok(())
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
#[path = "../../devshell/src/managed_files.rs"]
mod managed_files;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_sync_removes_deleted_sources_and_previous_project_files_but_keeps_app_data() {
        let app = write_fixture(&[("main.py", "old"), ("old_helper.py", "old helper"), ("user.db", "saved data")]);
        managed_files::sync(app.path(), &["main.py".into(), "old_helper.py".into()]).unwrap();
        // A reconnect with a different project supplies the authoritative list.
        managed_files::sync(app.path(), &["main.py".into(), "new_helper.py".into()]).unwrap();
        assert!(!app.path().join("old_helper.py").exists());
        assert_eq!(std::fs::read_to_string(app.path().join("user.db")).unwrap(), "saved data");
        std::fs::write(app.path().join("new_helper.py"), "new helper").unwrap();
        // A later save that deletes the helper removes it, too.
        managed_files::sync(app.path(), &["main.py".into()]).unwrap();
        assert!(!app.path().join("new_helper.py").exists());
        assert!(app.path().join("main.py").exists());
    }

    #[test]
    fn managed_sync_validates_all_paths_before_removing_any_sources() {
        let app = write_fixture(&[("main.py", "source")]);
        managed_files::sync(app.path(), &["main.py".into()]).unwrap();
        for bad in ["../outside.py", "/outside.py", ".zkite-managed-files.json", "C:\\outside.py", ""] {
            assert!(managed_files::sync(app.path(), &[bad.into()]).is_err());
            assert!(app.path().join("main.py").exists());
        }
    }

    #[test]
    #[cfg(unix)]
    fn managed_files_never_follow_app_created_symlinks() {
        let app = tempfile::tempdir().unwrap();
        let outside = write_fixture(&[("secret.py", "private")]);
        std::os::unix::fs::symlink(outside.path(), app.path().join("linked")).unwrap();
        assert!(managed_files::destination(app.path(), "linked/secret.py").is_err());
        assert!(managed_files::sync(app.path(), &["linked/secret.py".into()]).is_err());
        assert_eq!(std::fs::read_to_string(outside.path().join("secret.py")).unwrap(), "private");
    }

    #[test]
    fn deletion_only_batch_reconciles_then_reloads_and_legacy_shells_keep_old_frames() {
        let app = tempfile::tempdir().unwrap();
        let mut before = Manifest::default();
        before.files.insert("removed.py".into(), "old hash".into());
        let after = Manifest::default();
        let mut wire = Vec::new();
        push_changes(&mut wire, app.path(), &before, &after, true).unwrap();
        let frames: Vec<serde_json::Value> = String::from_utf8(wire).unwrap().lines()
            .map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(frames, [serde_json::json!({"op": "sync", "paths": []}), serde_json::json!({"op": "reload"})]);
        let mut legacy = Vec::new();
        push_changes(&mut legacy, app.path(), &before, &after, false).unwrap();
        assert_eq!(String::from_utf8(legacy).unwrap(), "{\"op\":\"reload\"}\n");
    }

    #[test]
    fn custom_host_port_keeps_the_shipped_device_port() {
        assert_eq!(reverse_ports(9876), ["reverse", "tcp:8765", "tcp:9876"]);
    }

    #[test]
    fn a_missing_or_crashed_shell_has_a_bounded_connection_wait() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let started = Instant::now();
        let mut reverse_attempts = 0;
        let result = wait_for_connection(&listener, Duration::from_millis(120), || {
            reverse_attempts += 1;
            anyhow::bail!("device disconnected")
        });
        assert!(result.unwrap_err().to_string().contains("did not connect"));
        assert_eq!(reverse_attempts, 1);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn devshell_lookup_and_distribution_manifest_agree_for_both_backends() {
        for backend in [Backend::Native, Backend::WebView] {
            let shell = dev_shell(backend);
            for abi in ["arm64-v8a", "x86_64"] {
                let expected = shell.artifact_rel.replace("{abi}", abi);
                let entries = crate::payload::staged_paths(backend.name(), &[abi.into()], true).unwrap();
                assert!(entries.iter().any(|entry| entry.path() == expected));
            }
        }
    }

    #[test]
    fn explicit_missing_devshell_is_an_error_instead_of_a_fallback() {
        let temp = tempfile::tempdir().unwrap();
        assert!(find_devshell_apk(&dev_shell(Backend::Native), "arm64-v8a", Some(&temp.path().join("missing.apk"))).is_err());
    }

    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (server, client)
    }

    #[test]
    fn wrong_renderer_is_rejected_before_project_files_are_sent() {
        use std::io::Read;
        let temp = write_fixture(&[("main.py", "private application source")]);
        let (server, mut client) = socket_pair();
        writeln!(client, r#"{{"op":"hello","backend":"webview"}}"#).unwrap();
        let error = serve_connection(server, temp.path(), Backend::Native, "arm64-v8a").unwrap_err();
        assert!(error.to_string().contains("backend mismatch"));
        client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut received = Vec::new();
        client.read_to_end(&mut received).unwrap();
        assert!(received.is_empty());
    }

    #[test]
    fn initial_push_then_disconnect_returns_without_waiting_for_a_file_edit() {
        let temp = write_fixture(&[("main.py", "print('hi')")]);
        let (server, mut client) = socket_pair();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = serve_connection(server, temp.path(), Backend::Native, "arm64-v8a");
            tx.send(result.map_err(|error| error.to_string())).unwrap();
        });
        writeln!(client, r#"{{"op":"hello","backend":"native"}}"#).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let file: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(file["op"], "file");
        assert_eq!(file["path"], "main.py");
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&line).unwrap()["op"], "reload");
        drop(reader);
        let result = rx.recv_timeout(Duration::from_secs(3)).expect("disconnect must wake the dev loop");
        assert!(result.unwrap_err().contains("disconnected"));
    }

    #[test]
    fn project_backend_defaults_to_native_and_respects_explicit_overrides() {
        let temp = write_fixture(&[("main.py", "pass"), ("pyproject.toml", "[project]\nname='app'\n[tool.zkite]\nbackend='webview'\n")]);
        assert_eq!(crate::build_cmd::project_backend(temp.path(), None).unwrap(), Backend::WebView);
        assert_eq!(crate::build_cmd::project_backend(temp.path(), Some(Backend::Native)).unwrap(), Backend::Native);
        std::fs::write(temp.path().join("pyproject.toml"), "[project]\nname='app'\n").unwrap();
        assert_eq!(crate::build_cmd::project_backend(temp.path(), None).unwrap(), Backend::Native);
    }

    /// The shared fixture for the cross-language contract. `zkite_pybridge`'s
    /// `the_framework_fingerprint_matches_the_hosts_own_algorithm` builds a directory
    /// with *these exact contents* and asserts `runtime.py`'s Python implementation
    /// produces the same literal below. Two implementations of one spec, pinned by a
    /// constant neither of them computes — if either drifts, one of the two tests
    /// goes red (phase_3/PLAN.md §31.6).
    ///
    /// Do not "fix" this constant if it fails. Work out which side moved.
    const FIXTURE: &[(&str, &str)] = &[("__init__.py", "A\n"), ("widgets.py", "B\n")];
    const FIXTURE_DIGEST: &str = "4db0afb50304";

    fn write_fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let td = tempfile::tempdir().unwrap();
        for (name, body) in files {
            std::fs::write(td.path().join(name), body).unwrap();
        }
        td
    }

    #[test]
    fn the_framework_fingerprint_is_the_digest_both_languages_agree_on() {
        let td = write_fixture(FIXTURE);
        assert_eq!(fingerprint_of(td.path()).as_deref(), Some(FIXTURE_DIGEST));
    }

    #[test]
    fn editing_one_framework_file_moves_the_fingerprint() {
        let before = write_fixture(FIXTURE);
        let after = write_fixture(&[("__init__.py", "A\n"), ("widgets.py", "B!\n")]);
        let (a, b) = (fingerprint_of(before.path()), fingerprint_of(after.path()));
        assert!(a.is_some() && b.is_some());
        assert_ne!(a, b, "a content edit must move the digest — that is the whole point");
    }

    /// The `Checkbox` case in miniature: a shell built before a widget existed differs
    /// from the payload only by files it does not have.
    #[test]
    fn a_missing_framework_file_moves_the_fingerprint() {
        let full = write_fixture(FIXTURE);
        let partial = write_fixture(&[("__init__.py", "A\n")]);
        assert_ne!(fingerprint_of(full.path()), fingerprint_of(partial.path()));
    }

    /// A rename with identical bytes must move it too — the name is in the stream on
    /// purpose, since `from zkite import X` resolves by module name.
    #[test]
    fn renaming_a_framework_file_moves_the_fingerprint() {
        let a = write_fixture(&[("__init__.py", "A\n"), ("widgets.py", "B\n")]);
        let b = write_fixture(&[("__init__.py", "A\n"), ("widgets2.py", "B\n")]);
        assert_ne!(fingerprint_of(a.path()), fingerprint_of(b.path()));
    }

    /// Non-`.py` files and `__pycache__` must not participate: the Dev Shell hashes
    /// what it unpacked out of `zkite-framework.zip` and the host hashes a source tree,
    /// and those two agree on `.py` and on nothing else.
    #[test]
    fn only_py_files_participate() {
        let td = write_fixture(FIXTURE);
        std::fs::write(td.path().join("README.txt"), "ignored").unwrap();
        std::fs::create_dir(td.path().join("__pycache__")).unwrap();
        std::fs::write(td.path().join("__pycache__/widgets.cpython-314.pyc"), "x").unwrap();
        assert_eq!(fingerprint_of(td.path()).as_deref(), Some(FIXTURE_DIGEST));
    }

    /// Fails open rather than reporting a bogus mismatch.
    #[test]
    fn a_directory_with_no_framework_in_it_has_no_fingerprint() {
        let td = tempfile::tempdir().unwrap();
        assert_eq!(fingerprint_of(td.path()), None);
        assert_eq!(fingerprint_of(&td.path().join("does-not-exist")), None);
    }
}
