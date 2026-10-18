mod build_cmd;
mod dev_cmd;
mod device;
mod preview;
mod doctor_cmd;
mod host_python;
mod keygen_cmd;
mod manifest;
mod new_cmd;
mod payload;
mod provenance;
mod run_cmd;
mod watch;
mod wheels;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "zkite", version, about = "Zero-boilerplate Python framework for Android")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a new zkite project: `zkite new myapp && cd myapp`.
    New {
        name: String,
        /// Use `.` to scaffold in the current directory.
        destination: Option<String>,
    },
    /// Hot-reloading dev loop against a connected device.
    Dev {
        /// Use a custom Dev Shell APK. Normally a project preview is prepared
        /// from the bundled runtime with your app's icon and label.
        #[arg(long)]
        devshell_apk: Option<PathBuf>,
        #[arg(long, default_value_t = 8765)]
        port: u16,
        #[arg(long, default_value = "adb")]
        adb: String,
        /// Android device serial; also accepts ANDROID_SERIAL.
        /// Otherwise, multiple devices open an arrow-key picker.
        #[arg(long)]
        serial: Option<String>,
        /// Rendering backend: "native" (default) or "webview". Picks which Dev Shell
        /// to talk to; your app's source is identical either way.
        #[arg(long)]
        backend: Option<String>,
    },
    /// Build, install, and launch a standalone app. No `zkite dev` session required.
    Run {
        #[arg(long, default_value = "adb")]
        adb: String,
        /// Android device serial; also accepts ANDROID_SERIAL.
        /// Otherwise, multiple devices open an arrow-key picker.
        #[arg(long)]
        serial: Option<String>,
        /// Overrides the project backend; defaults to native.
        #[arg(long)]
        backend: Option<String>,
        /// Signing PEM. Defaults to a persistent local development key in .zkite/.
        #[arg(long)]
        keystore: Option<PathBuf>,
        #[arg(long)]
        no_prune_stdlib: bool,
        /// Keep streaming this app's Android logs after launching it.
        #[arg(long)]
        logs: bool,
    },
    /// Build a signed, standalone APK or AAB.
    Build {
        /// "apk" or "aab".
        format: String,
        /// Restrict to one or more ABIs (arm64-v8a, x86_64). Default: all with
        /// prebuilt appshell artifacts.
        #[arg(long = "abi")]
        abis: Vec<String>,
        /// A combined PEM (CERTIFICATE + PRIVATE KEY) from `zkite keygen`. Omit to
        /// sign with an ephemeral testing key (loud warning, not for release).
        #[arg(long)]
        keystore: Option<PathBuf>,
        /// Output directory. Default: <project>/dist/.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Ship the full, unpruned stdlib — for
        /// apps using dynamic imports that static reachability cannot detect.
        #[arg(long)]
        no_prune_stdlib: bool,
        /// Rendering backend: "native" (default, the phase-3 GPU renderer) or
        /// "webview". Overrides `backend` in app.json. Your app's
        /// source is identical either way — this picks the shell it runs in.
        #[arg(long)]
        backend: Option<String>,
    },
    /// Generate a signing keystore: a combined PEM and a PKCS#12.
    Keygen {
        /// Where to write "<name>.pem" and "<name>.p12". Default: keystore/release.
        #[arg(long, default_value = "keystore/release")]
        out: PathBuf,
        #[arg(long, default_value = "zkite")]
        cn: String,
        #[arg(long, default_value = "android")]
        password: String,
    },
    /// Check the local environment: adb, a Python interpreter for pyc/pruning, pip
    /// for wheel resolution, and prebuilt appshell artifacts.
    Doctor {
        #[arg(long, default_value = "adb")]
        adb: String,
    },
    /// Print the payload-relative paths `zkite build` reads, for a release pipeline to
    /// stage. Plumbing, not a user command — `phase_1/cli/scripts/stage_payload.sh`
    /// is its one consumer, and it exists so that script has no list of its own to
    /// drift from this binary's.
    #[command(hide = true)]
    PayloadManifest {
        #[arg(long, default_value = "native")]
        backend: String,
        #[arg(long = "abi")]
        abis: Vec<String>,
        /// Also stage the prebuilt Dev Shell APK (39 MB per ABI, opt-in — §24.3).
        #[arg(long)]
        devshell: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::New { name, destination } => {
            anyhow::ensure!(destination.as_deref().is_none_or(|d| d == "."), "the optional destination must be `.`");
            let current = destination.is_some();
            if current { new_cmd::run_at(&name, &std::env::current_dir()?, true)?; }
            else { new_cmd::run(&name, &std::env::current_dir()?)?; }
            println!("created {}", if current { "." } else { &name });
            if !current { println!("  cd {name}"); }
            println!("  zkite dev");
        }
        Command::Dev { devshell_apk, port, adb, serial, backend } => {
            dev_cmd::run(dev_cmd::DevArgs {
                project_dir: std::env::current_dir()?,
                port,
                devshell_apk,
                adb,
                serial,
                backend: backend.map(|b| build_cmd::Backend::parse(&b)).transpose()?,
            })?;
        }
        Command::Run { adb, serial, backend, keystore, no_prune_stdlib, logs } => {
            run_cmd::run(run_cmd::RunArgs {
                project_dir: std::env::current_dir()?,
                adb,
                serial,
                backend: backend.map(|b| build_cmd::Backend::parse(&b)).transpose()?,
                keystore,
                no_prune_stdlib,
                logs,
            })?;
        }
        Command::Build { format, abis, keystore, out, no_prune_stdlib, backend } => {
            let format = match format.as_str() {
                "apk" => build_cmd::BuildFormat::Apk,
                "aab" => build_cmd::BuildFormat::Aab,
                other => anyhow::bail!("unknown build format {other:?} — expected \"apk\" or \"aab\""),
            };
            let backend = backend.map(|b| build_cmd::Backend::parse(&b)).transpose()?;
            build_cmd::run(build_cmd::BuildArgs {
                project_dir: std::env::current_dir()?,
                format,
                abis,
                keystore,
                out,
                no_prune_stdlib,
                backend,
            })?;
        }
        Command::Keygen { out, cn, password } => {
            keygen_cmd::run(&out, &cn, &password)?;
        }
        Command::Doctor { adb } => {
            doctor_cmd::run(&adb)?;
        }
        Command::PayloadManifest { backend, abis, devshell } => {
            let abis = if abis.is_empty() {
                vec!["arm64-v8a".to_string()]
            } else {
                abis
            };
            payload::print_manifest(&backend, &abis, devshell)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn version_is_available_without_a_project_or_device() {
        let output = Cli::try_parse_from(["zkite", "--version"]).err().unwrap();
        assert_eq!(output.kind(), clap::error::ErrorKind::DisplayVersion);
        assert!(output.to_string().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn run_is_a_standalone_command_with_explicit_device_selection() {
        let cli = Cli::try_parse_from(["zkite", "run", "--serial", "emulator-5554", "--logs"]).unwrap();
        assert!(matches!(cli.command, Command::Run { serial: Some(s), logs: true, backend: None, .. } if s == "emulator-5554"));
    }

    #[test]
    fn dev_can_inherit_the_project_backend() {
        let cli = Cli::try_parse_from(["zkite", "dev"]).unwrap();
        assert!(matches!(cli.command, Command::Dev { backend: None, port: 8765, .. }));
    }
}
