//! Standalone `adb shell` probe, same role as phase_0/spike1's `zkite-py-probe`: a fast
//! way to catch Rust/Python-glue bugs before paying for a full APK rebuild+install.
//!
//! Not a substitute for the in-app check — phase_0's README is explicit that the
//! standalone-vs-in-app distinction is exactly what caught the `PYTHONHOME` bug, since
//! `argv[0]` landmark-searching can make a standalone probe pass for the wrong reason.
//! This is a diagnostic tool for iteration speed, not the pass/fail check.
//!
//! Usage: zkite-devshell-probe <python-prefix> <zkite-pkg-dir> <app-dir>

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: zkite-devshell-probe <python-prefix> <zkite-pkg-dir> <app-dir>");
        std::process::exit(2);
    }
    let prefix = std::path::Path::new(&args[1]);
    let zkite_pkg_dir = &args[2];
    let app_dir = &args[3];

    match zkitedevshell::setup(prefix, zkite_pkg_dir, app_dir) {
        Ok(html) => {
            println!("ZKITE_DEVSHELL_PROBE ok=1");
            println!("{html}");
        }
        Err(e) => {
            println!("ZKITE_DEVSHELL_PROBE ok=0 error={e:?}");
            std::process::exit(1);
        }
    }

    if let Some(html) = zkitedevshell::dispatch("r.1", "tap", "") {
        println!("--- after a simulated tap dispatch ---");
        println!("{html}");
    } else {
        println!("--- dispatch('r.1', 'tap', '') was not handled (expected unless the demo app has a button there) ---");
    }
}
