//! Standalone `adb shell` probe, same role as `phase_1/devshell`'s probe (and, before
//! it, `phase_0/spike1`'s): a fast way to catch Rust/Python-glue bugs before paying
//! for a full APK rebuild+install. Not a substitute for the in-app check — see
//! `phase_0/README.md`'s `PYTHONHOME` writeup for why standalone-vs-in-app is a real
//! distinction, not a formality.
//!
//! Usage: zkite-appshell-probe <python-prefix> <zkite-pkg-dir> <app-dir>
//! (unlike devshell's probe, <app-dir> is expected to already contain a real
//! main.py — a release shell has no reload socket to push one over)

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: zkite-appshell-probe <python-prefix> <zkite-pkg-dir> <app-dir>");
        std::process::exit(2);
    }
    let prefix = std::path::Path::new(&args[1]);
    let zkite_pkg_dir = &args[2];
    let app_dir = &args[3];

    match zkiteappshell::setup(prefix, zkite_pkg_dir, app_dir) {
        Ok(html) => {
            println!("ZKITE_APPSHELL_PROBE ok=1");
            println!("{html}");
        }
        Err(e) => {
            println!("ZKITE_APPSHELL_PROBE ok=0 error={e:?}");
            std::process::exit(1);
        }
    }

    if let Some(html) = zkiteappshell::dispatch("r.1", "tap", "") {
        println!("--- after a simulated tap dispatch ---");
        println!("{html}");
    } else {
        println!("--- dispatch('r.1', 'tap', '') was not handled (expected unless the app has a button there) ---");
    }
}
