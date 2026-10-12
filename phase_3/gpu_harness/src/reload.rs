//! The dev-loop reload socket for the native backend (`../PLAN.md` §15).
//!
//! Wire-compatible with `phase_1/devshell/src/lib.rs`'s: same newline-delimited JSON
//! frames, same direction (the *device* dials out, because `adb reverse` forwards a
//! connection made on the device — see `phase_1/PLAN.md` §4), same `Log`/`Error`
//! replies. `zkite dev`'s host half (`phase_1/cli/src/dev_cmd.rs`) therefore needs no
//! per-backend protocol handling at all; only which package it launches differs.
//!
//! **This is structurally simpler than the WebView shell's version, and the reason is
//! worth recording.** `devshell` had no way to push a new frame from a background
//! thread — a file-triggered reload has no Java call to return through — so it parks
//! HTML in a `Mutex<Option<String>>` and `ZKiteActivity` polls `Native.pollHtml()`
//! every 120ms, a trade `phase_1/PLAN.md` §4 documents as deliberately avoiding
//! cross-thread JNI. This backend already has exactly the mechanism that was missing:
//! a render thread that owns "take a new tree from Python, repaint," reached by an
//! `mpsc` channel that four other event sources already use. So the socket thread
//! never touches Python, never touches JNI, and never renders. It writes files and
//! sends one `Command::Reload`. No polling, no `PENDING_HTML`.
//!
//! The socket thread *does* block on the render thread's ack, so a traceback can be
//! written back to `zkite dev`'s terminal in the same turn that produced it — the same
//! `SyncSender` ack idiom `Command::DestroyWindow` already uses, and for the same
//! reason: the answer is only meaningful if the caller waits for it.
//!
//! **Never started in a shipped app.** `ZKiteActivity` only calls
//! `Native.startReloadServer` when the `zkite-devmode` marker asset is present, which
//! only the Dev Shell build stages. `phase_2/PLAN.md` §1.2 is explicit about why —
//! a release build has no `zkite dev` to reach, so the retry loop would be an
//! infinite background reconnect burning battery, behind an `INTERNET` permission
//! with no runtime purpose.

#[path = "../../../phase_1/devshell/src/managed_files.rs"]
mod managed_files;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::mpsc::sync_channel;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::render_thread::{Command, ReloadReport};

/// What `zkite dev` sends. Mirrors `dev_cmd.rs`'s `Frame` and devshell's; kept as a
/// hand-matched local type rather than a shared crate for the same reason devshell
/// does — the two sides cross-compile for different targets.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Frame {
    Sync { paths: Vec<String> },
    File { path: String, content_b64: String },
    Reload,
}

/// What goes back to `zkite dev`'s terminal.
#[derive(Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum OutFrame<'a> {
    /// Sent once on connect so `zkite dev` can tell *which* backend reached it. Both
    /// shells speak this identical protocol on the same port, so without this there
    /// is nothing distinguishing them on the wire and `--backend` silently becomes a
    /// no-op whenever the other shell happens to be running. See ../PLAN.md §15.6.
    ///
    /// `framework` is a content hash of the `zkite` package **this shell shipped**,
    /// which `zkite dev` compares against the payload's own (../PLAN.md §31.6). The
    /// Dev Shell pins the framework — `zkite dev` pushes only the project's `.py` and
    /// `zkite/` itself lives in `assets/zkite-framework.zip` — so without this a widget
    /// added after the shell was built reaches the user as a bare `ImportError`.
    /// `Option`, and skipped when absent, because an older `zkite dev` must still
    /// parse this frame and because a *false* staleness report is worse than none.
    Hello {
        backend: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        framework: Option<String>,
        sync_files: bool,
    },
    Log { text: &'a str },
    Error { text: &'a str },
}

/// Dials `host:port` in a retry loop forever — the dev server may not be up yet, or
/// may restart, and `zkite dev` re-establishes `adb reverse` on every reconnect wait
/// (`phase_1/PLAN.md` §4's own finding about the mapping dropping on a screen lock).
pub fn start_reload_server(host: String, port: u16, app_dir: String) {
    std::thread::spawn(move || loop {
        if let Ok(stream) = TcpStream::connect((host.as_str(), port)) {
            crate::logi!("reload socket connected to {host}:{port}");
            handle_connection(stream, &app_dir);
            crate::logi!("reload socket disconnected, retrying");
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}

fn handle_connection(stream: TcpStream, app_dir: &str) {
    let Ok(mut writer) = stream.try_clone() else { return };
    // Waits for the render thread to publish, because this connection is normally made
    // *before* CPython has finished booting — see `crate::FRAMEWORK_FINGERPRINT`'s doc
    // for why that is the common case and not the edge one. 20s is generous against a
    // measured ~650ms (emulator) / ~1.5s (Moto G54) boot; it exists only so a hung boot
    // cannot wedge the socket, in which case the field is omitted and `zkite dev` says
    // nothing about staleness.
    let framework = crate::framework_fingerprint_when_ready(Duration::from_secs(20));
    let hello = OutFrame::Hello { backend: "native", framework, sync_files: true };
    if let Ok(f) = serde_json::to_string(&hello) {
        let _ = writeln!(writer, "{f}");
    }
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            Ok(_) => continue,
            Err(_) => return,
        };
        let frame: Frame = match serde_json::from_str(&line) {
            Ok(f) => f,
            Err(e) => {
                crate::loge!("bad reload frame: {e}");
                continue;
            }
        };
        match frame {
            Frame::Sync { paths } => {
                if let Err(error) = managed_files::sync(Path::new(app_dir), &paths) {
                    if let Ok(frame) = serde_json::to_string(&OutFrame::Error { text: &error }) {
                        let _ = writeln!(writer, "{frame}");
                    }
                    return;
                }
            }
            Frame::File { path, content_b64 } => match apply_file(app_dir, &path, &content_b64) {
                Ok(()) => crate::logi!("wrote {path}"),
                Err(e) => crate::loge!("could not write {path}: {e}"),
            },
            Frame::Reload => {
                let report = request_reload();
                if let Some(report) = report {
                    if !report.output.trim().is_empty() {
                        if let Ok(f) =
                            serde_json::to_string(&OutFrame::Log { text: &report.output })
                        {
                            let _ = writeln!(writer, "{f}");
                        }
                    }
                    if let Some(err) = &report.error {
                        if let Ok(f) = serde_json::to_string(&OutFrame::Error { text: err }) {
                            let _ = writeln!(writer, "{f}");
                        }
                    }
                }
                let _ = writeln!(writer, "{{\"ok\":true}}");
            }
        }
    }
}

/// Hands the reload to the render thread and waits for it, so the reply written back
/// to `zkite dev` describes the render that actually happened rather than one that was
/// merely queued. `None` if the render thread is gone (the app is shutting down) or
/// never answered — in which case the caller still acks the frame, because a hung
/// `zkite dev` is a worse failure than a missing log line.
fn request_reload() -> Option<ReloadReport> {
    let (ack_tx, ack_rx) = sync_channel::<ReloadReport>(0);
    {
        let guard = crate::sender();
        let tx = guard.as_ref()?;
        tx.send(Command::Reload { ack: ack_tx }).ok()?;
    }
    match ack_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(report) => Some(report),
        Err(e) => {
            crate::loge!("reload: render thread did not answer ({e})");
            None
        }
    }
}

fn apply_file(app_dir: &str, rel_path: &str, content_b64: &str) -> Result<(), String> {
    let dest = managed_files::destination(Path::new(app_dir), rel_path)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = base64_decode(content_b64).map_err(|e| e.to_string())?;
    std::fs::write(&dest, bytes).map_err(|e| e.to_string())
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).map_err(|e| e.to_string())
}

// Shared managed-file reconciliation and path checks are tested by the host CLI.
