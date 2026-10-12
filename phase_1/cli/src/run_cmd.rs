//! Build a standalone app from prebuilt runtime artifacts, install it, and launch
//! it. This path never uses the generic Dev Shell or a host reload server.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::build_cmd::{self, Backend, BuildArgs, BuildFormat};
use crate::device::Device;

pub struct RunArgs {
    pub project_dir: PathBuf,
    pub adb: String,
    pub serial: Option<String>,
    pub backend: Option<Backend>,
    pub keystore: Option<PathBuf>,
    pub no_prune_stdlib: bool,
    pub logs: bool,
}

pub fn run(args: RunArgs) -> Result<()> {
    run_with_builder(args, build_cmd::run)
}

fn run_with_builder(
    args: RunArgs,
    build: impl FnOnce(BuildArgs) -> Result<build_cmd::BuildOutput>,
) -> Result<()> {
    // Validate the project and device before packaging anything.
    let backend = build_cmd::project_backend(&args.project_dir, args.backend)?;
    let device = Device::connect(&args.adb, args.serial.as_deref())?;
    let abi = device.abi()?;
    let key = match args.keystore {
        Some(path) => path,
        None => {
            let path = debug_key(&args.project_dir)?;
            println!("  using local development signing key {}", path.display());
            path
        }
    };
    let output = build(BuildArgs {
        project_dir: args.project_dir,
        format: BuildFormat::Apk,
        abis: vec![abi],
        keystore: Some(key),
        out: None,
        no_prune_stdlib: args.no_prune_stdlib,
        backend: Some(backend),
    })?;
    let apk = output.artifacts.first().context("build did not produce an APK")?;
    device.install(apk)?;
    device.launch(&format!("{}/{}", output.package, output.activity))?;
    println!("{} is running independently on {}", output.package, device.serial);
    if args.logs {
        follow_logs(&device, &output.package)?;
    }
    Ok(())
}

/// A stable, per-project development identity allows repeat runs to update the app
/// without uninstalling it and erasing data. NamedTempFile keeps the private key
/// private on Unix; persist_noclobber makes creation atomic even with two runs.
pub(crate) fn debug_key(project: &Path) -> Result<PathBuf> {
    let directory = project.join(".zkite");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("debug.pem");
    if path.is_file() {
        return Ok(path);
    }
    let ignore = directory.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(ignore, "*\n")?;
    }
    let generated = zkite_bundler::keystore::generate(&zkite_bundler::keystore::KeygenOptions {
        common_name: "ZKite local development",
        ..Default::default()
    }).map_err(anyhow::Error::msg)?;
    let mut file = tempfile::NamedTempFile::new_in(&directory)?;
    file.write_all(generated.combined_pem.as_bytes())?;
    file.as_file().sync_all()?;
    match file.persist_noclobber(&path) {
        Ok(_) => {},
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {},
        Err(error) => return Err(error.error).context("saving the local development key"),
    }
    Ok(path)
}

fn follow_logs(device: &Device, package: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let pid = loop {
        let output = device.command().args(["shell", "pidof", "-s", package]).output()?;
        let pid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if output.status.success() && !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()) {
            break pid;
        }
        anyhow::ensure!(Instant::now() < deadline,
            "{package} exited before log streaming started; inspect adb logcat for its startup failure");
        std::thread::sleep(Duration::from_millis(100));
    };
    println!("streaming logs for {package} (PID {pid}); press Ctrl+C to stop");
    let status = device.command().args(["logcat", "-v", "brief", &format!("--pid={pid}")]).status()?;
    anyhow::ensure!(status.success(), "adb logcat exited with {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_runs_keep_the_same_signing_key_and_do_not_bundle_it() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("main.py"), "pass\n").unwrap();
        let path = debug_key(project.path()).unwrap();
        let first = std::fs::read(&path).unwrap();
        assert_eq!(debug_key(project.path()).unwrap(), path);
        assert_eq!(std::fs::read(&path).unwrap(), first);
        assert!(zkite_bundler::keystore::load_pem(&path).is_ok());
        let manifest = crate::manifest::Manifest::scan(project.path()).unwrap();
        assert_eq!(manifest.files.len(), 1);
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
        }
    }

    #[cfg(unix)]
    fn project_and_device() -> (tempfile::TempDir, tempfile::TempDir, RunArgs) {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("main.py"), "pass\n").unwrap();
        std::fs::write(project.path().join("pyproject.toml"), "[project]\nname='app'\n").unwrap();
        // A supplied path isolates these transport tests from RSA generation. The
        // builder test double only validates that the requested path is preserved.
        let key = project.path().join("test-signing.pem");
        let (fixture, adb) = crate::device::test_support::fake_adb(r#"
case "$*" in
  'devices -l') printf 'List of devices attached\ntest-phone\tdevice\n' ;;
  '-s test-phone shell getprop ro.product.cpu.abi') printf 'x86_64\n' ;;
  '-s test-phone install -r '*) test -f "$5" ;;
  '-s test-phone shell am start -W -n com.example.app/dev.zkite.gpuharness.ZKiteActivity') printf 'Status: ok\n' ;;
  *) printf 'unexpected adb operation: %s\n' "$*" >&2; exit 9 ;;
esac
"#);
        let args = RunArgs {
            project_dir: project.path().to_owned(),
            adb: adb.to_string_lossy().into_owned(),
            serial: Some("test-phone".into()),
            backend: None,
            keystore: Some(key),
            no_prune_stdlib: false,
            logs: false,
        };
        (project, fixture, args)
    }

    #[test]
    #[cfg(unix)]
    fn fresh_run_builds_for_device_then_installs_and_launches_without_a_dev_shell() {
        let (_project, fixture, args) = project_and_device();
        run_with_builder(args, |build| {
            assert_eq!(build.abis, ["x86_64"]);
            assert_eq!(build.backend, Some(Backend::Native));
            assert!(build.format == BuildFormat::Apk);
            assert_eq!(build.keystore, Some(build.project_dir.join("test-signing.pem")));
            // Return a nonstandard filename: run must use this invocation's exact
            // artifact, not guess a name or install a stale file from dist/.
            let apk = build.project_dir.join("freshly built application.apk");
            std::fs::write(&apk, "prebuilt-runtime test APK").unwrap();
            Ok(build_cmd::BuildOutput {
                artifacts: vec![apk],
                package: "com.example.app".into(),
                activity: "dev.zkite.gpuharness.ZKiteActivity".into(),
            })
        }).unwrap();
        let commands = std::fs::read_to_string(fixture.path().join("commands")).unwrap();
        let commands: Vec<_> = commands.lines().collect();
        assert_eq!(commands.len(), 4);
        assert_eq!(commands[0], "devices -l");
        assert!(commands[2].starts_with("-s test-phone install -r "));
        assert_eq!(commands[3], "-s test-phone shell am start -W -n com.example.app/dev.zkite.gpuharness.ZKiteActivity");
    }

    #[test]
    #[cfg(unix)]
    fn a_build_failure_never_installs_a_previous_apk() {
        let (_project, fixture, args) = project_and_device();
        let error = run_with_builder(args, |_| anyhow::bail!("invalid application"));
        assert!(error.unwrap_err().to_string().contains("invalid application"));
        let commands = std::fs::read_to_string(fixture.path().join("commands")).unwrap();
        assert_eq!(commands.lines().count(), 2);
        assert!(!commands.contains("install"));
    }
}
