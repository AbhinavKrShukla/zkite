//! Standalone on-device prober.
//!
//! `adb push`ed to `/data/local/tmp` and run over `adb shell`, this proves the CPython
//! embedding independently of any APK, manifest, or Activity — so if spike 2 breaks, we
//! still know whether spike 1 passed. Spike 2 then re-proves the same thing through the
//! `.so` inside a real app process.
//!
//! Usage: `zkite-py-probe <prefix>` where `<prefix>` contains `lib/python3.14/`.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let prefix = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: zkite-py-probe <prefix-containing-lib/python3.14>");
            return ExitCode::from(2);
        }
    };

    let path = Path::new(&prefix);
    if !path.join("lib/python3.14").is_dir() {
        println!(
            "ZKITE_SPIKE1_RESULT ok=0 error=\"{}/lib/python3.14 not found\"",
            path.display()
        );
        return ExitCode::from(1);
    }

    zkitespike1::set_python_home(path);
    let report = zkitespike1::boot();

    // One machine-readable line, then the detail a human wants when it fails.
    println!("{}", report.to_line());
    if !report.ok {
        eprintln!("report: {report:#?}");
        return ExitCode::from(1);
    }
    eprintln!("python: {}", report.version);
    ExitCode::SUCCESS
}
