//! Device discovery and subprocess arguments shared by `run` and `dev`.
//! Every device command is pinned to a serial so reconnects cannot switch phones.

use anyhow::{bail, Context, Result};
use console::{truncate_str, Key, Term};
use std::io::IsTerminal;
use std::path::Path;
use std::process::{Command, Output};

pub struct Device {
    adb: String,
    pub serial: String,
}

impl Device {
    pub fn connect(adb: &str, serial: Option<&str>) -> Result<Self> {
        let output = Command::new(adb).args(["devices", "-l"]).output()
            .with_context(|| format!("running {adb} devices -l; install Android platform-tools and put adb on PATH"))?;
        check_output(&output, "adb devices -l")?;
        let environment_serial = std::env::var("ANDROID_SERIAL").ok();
        let requested = serial.or(environment_serial.as_deref());
        let serial = select_serial(&String::from_utf8_lossy(&output.stdout), requested, pick_device)?;
        Ok(Self { adb: adb.to_owned(), serial })
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.adb);
        command.args(["-s", &self.serial]);
        command
    }

    pub fn output(&self, args: &[&str]) -> Result<Output> {
        let output = self.command().args(args).output()
            .with_context(|| format!("running adb {} on {}", args.join(" "), self.serial))?;
        check_output(&output, &format!("adb {}", args.join(" ")))?;
        Ok(output)
    }

    pub fn abi(&self) -> Result<String> {
        let output = self.output(&["shell", "getprop", "ro.product.cpu.abi"])?;
        supported_abi(String::from_utf8_lossy(&output.stdout).trim())
    }

    pub fn installed(&self, package: &str) -> Result<bool> {
        let output = self.command().args(["shell", "pm", "path", package]).output()
            .context("checking whether the Dev Shell is installed")?;
        // PackageManager reports an absent package with empty output and either
        // status 0 or 1 depending on Android version. Transport errors are different.
        if !output.status.success() && !output.stderr.is_empty() {
            check_output(&output, "adb shell pm path")?;
        }
        Ok(output.stdout.starts_with(b"package:"))
    }

    pub fn install(&self, apk: &Path) -> Result<()> {
        anyhow::ensure!(apk.is_file(), "APK does not exist: {}", apk.display());
        println!("installing {} on {}", apk.display(), self.serial);
        let output = self.command().args(["install", "-r"]).arg(apk).output()
            .with_context(|| format!("installing {}", apk.display()))?;
        check_output(&output, "adb install -r").context(
            "installation failed; the existing app and its data were preserved. If Android reports \
             INSTALL_FAILED_UPDATE_INCOMPATIBLE, use the same signing key as the installed app, \
             or explicitly uninstall that app if you can discard its data",
        )
    }

    pub fn launch(&self, component: &str) -> Result<()> {
        println!("launching {component}");
        let output = self.output(&["shell", "am", "start", "-W", "-n", component])?;
        // ActivityManager may print an error while returning exit status zero.
        let message = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        anyhow::ensure!(!message.lines().any(|line| line.trim_start().starts_with("Error")),
            "Android could not launch {component}: {}", message.trim());
        Ok(())
    }
}

fn check_output(output: &Output, operation: &str) -> Result<()> {
    if !output.status.success() {
        bail!("{operation} failed ({}): {}{}", output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

fn supported_abi(abi: &str) -> Result<String> {
    match abi {
        "arm64-v8a" | "x86_64" => Ok(abi.to_owned()),
        "" => bail!("the device did not report its CPU ABI; check the adb connection"),
        other => bail!("unsupported device ABI {other:?}; ZKite currently ships arm64-v8a and x86_64 artifacts"),
    }
}

struct ListedDevice<'a> {
    serial: &'a str,
    state: &'a str,
    model: Option<&'a str>,
}

impl ListedDevice<'_> {
    fn label(&self) -> String {
        let model = self.model.unwrap_or_else(|| {
            if self.serial.starts_with("emulator-") { "Android emulator" } else { "Android device" }
        }).replace('_', " ");
        // A serial always disambiguates devices with identical model names.
        format!("{model} — {}", self.serial)
    }
}

fn select_serial(
    list: &str,
    requested: Option<&str>,
    choose: impl FnOnce(&[&ListedDevice<'_>]) -> Result<Option<usize>>,
) -> Result<String> {
    let devices: Vec<_> = list.lines().filter_map(|line| {
        let mut fields = line.split_whitespace();
        let serial = fields.next()?;
        let state = fields.next()?;
        if matches!(serial, "List" | "*" | "adb:") {
            return None;
        }
        let model = fields.find_map(|field| field.strip_prefix("model:").filter(|m| !m.is_empty()));
        Some(ListedDevice { serial, state, model })
    }).collect();
    if let Some(requested) = requested {
        return match devices.iter().find(|device| device.serial == requested) {
            Some(device) if device.state == "device" => Ok(device.serial.to_owned()),
            Some(device) => bail!("device {requested} is {}; unlock it and approve USB debugging, or reconnect it", device.state),
            None => bail!("device {requested} was not found by adb devices"),
        };
    }
    let ready: Vec<_> = devices.iter().filter(|device| device.state == "device").collect();
    match ready.as_slice() {
        [device] => Ok(device.serial.to_owned()),
        [] => bail!("no authorized Android device/emulator is connected. Enable USB debugging and approve the computer on the device. adb devices: {}", list.trim()),
        _ => {
            let selected = choose(&ready)?.context("device selection cancelled")?;
            let device = ready.get(selected).context("invalid device selection")?;
            Ok(device.serial.to_owned())
        },
    }
}

fn pick_device(devices: &[&ListedDevice<'_>]) -> Result<Option<usize>> {
    let terminal = Term::stderr();
    // Prompts must never consume piped input or leave a CI job waiting for a key.
    anyhow::ensure!(
        std::io::stdin().is_terminal() && terminal.is_term()
            && std::env::var("TERM").as_deref() != Ok("dumb"),
        "multiple devices are connected: {}. Interactive selection needs a terminal; use --serial <serial> or ANDROID_SERIAL",
        devices.iter().map(|device| device.serial).collect::<Vec<_>>().join(", ")
    );
    let labels: Vec<_> = devices.iter().map(|device| device.label()).collect();
    let selection = {
        let mut display = PickerDisplay { terminal: &terminal, lines: 0 };
        terminal.hide_cursor()?;
        let mut selected = 0;
        loop {
            display.clear()?;
            display.line("Select an Android device")?;
            display.line("Up/Down: move | Enter: connect | Esc/Ctrl+C: cancel")?;
            let page_size = usize::from(terminal.size().0.saturating_sub(4)).clamp(1, 8);
            let start = selected / page_size * page_size;
            for (index, label) in labels.iter().enumerate().skip(start).take(page_size) {
                let marker = if index == selected { ">" } else { " " };
                display.line(&format!("{marker} {label}"))?;
            }
            terminal.flush()?;
            // read_key_raw returns Ctrl+C as a key, so Drop restores the cursor
            // on cancellation as well as success and I/O errors. The terminal
            // library restores input mode after each read on Unix and Windows.
            match terminal.read_key_raw().context("could not read device selection; use --serial <serial> or ANDROID_SERIAL")? {
                Key::ArrowDown => selected = (selected + 1) % devices.len(),
                Key::ArrowUp => selected = (selected + devices.len() - 1) % devices.len(),
                Key::Enter => break Some(selected),
                Key::Escape | Key::CtrlC => break None,
                _ => {},
            }
        }
    };
    if let Some(index) = selection {
        terminal.write_line(&format!("Selected device: {}", labels[index]))?;
    }
    Ok(selection)
}

struct PickerDisplay<'a> {
    terminal: &'a Term,
    lines: usize,
}

impl PickerDisplay<'_> {
    fn clear(&mut self) -> std::io::Result<()> {
        self.terminal.clear_last_lines(self.lines)?;
        self.lines = 0;
        Ok(())
    }

    fn line(&mut self, text: &str) -> std::io::Result<()> {
        // Keep each row on one terminal line, including on narrow consoles.
        let width = usize::from(self.terminal.size().1.saturating_sub(1)).max(1);
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.terminal.write_line(&truncate_str(&text, width, "…"))?;
        self.lines += 1;
        Ok(())
    }
}

impl Drop for PickerDisplay<'_> {
    fn drop(&mut self) {
        let _ = self.clear();
        let _ = self.terminal.show_cursor();
        let _ = self.terminal.flush();
    }
}

#[cfg(all(test, unix))]
pub(crate) mod test_support {
    use std::os::unix::fs::PermissionsExt;

    pub fn fake_adb(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fake adb");
        std::fs::write(&path, format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$(dirname \"$0\")/commands\"\n{script}"
        )).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (directory, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_picker(_: &[&ListedDevice<'_>]) -> Result<Option<usize>> {
        panic!("this case must not open the device picker");
    }

    #[test]
    fn selects_only_authorized_devices_and_handles_windows_lines() {
        let list = "List of devices attached\r\nphone\tunauthorized\r\nemulator-5554\tdevice\r\n";
        assert_eq!(select_serial(list, None, no_picker).unwrap(), "emulator-5554");
        assert!(select_serial(list, Some("phone"), no_picker).unwrap_err().to_string().contains("unauthorized"));
    }

    #[test]
    fn an_explicit_choice_bypasses_the_picker_even_with_multiple_devices() {
        let list = "List of devices attached\nphone\tdevice\nemulator-5554\tdevice\n";
        assert_eq!(select_serial(list, Some("phone"), no_picker).unwrap(), "phone");
        assert!(select_serial(list, Some("missing"), no_picker).is_err());
    }

    #[test]
    fn picker_shows_models_and_selects_only_authorized_serials() {
        let list = "* daemon started successfully\r\nList of devices attached\r\n\
            blocked\tunauthorized model:Pixel_9\r\n\
            phone-a\tdevice usb:1-2 product:pixel model:Pixel_9 device:pixel transport_id:1\r\n\
            sleeping\toffline model:Pixel_9\r\n\
            phone-b\tdevice product:pixel model:Pixel_9 transport_id:3\r\n";
        let selected = select_serial(list, None, |devices| {
            assert_eq!(devices.len(), 2);
            assert_eq!(devices[0].label(), "Pixel 9 — phone-a");
            assert_eq!(devices[1].label(), "Pixel 9 — phone-b");
            Ok(Some(1))
        }).unwrap();
        assert_eq!(selected, "phone-b");
    }

    #[test]
    fn cancellation_does_not_default_to_the_first_device() {
        let list = "phone\tdevice\nemulator-5554\tdevice\n";
        let error = select_serial(list, None, |_| Ok(None)).unwrap_err();
        assert!(error.to_string().contains("selection cancelled"));
        assert!(select_serial(list, None, |_| Ok(Some(9))).is_err());
        let error = select_serial(list, None, |_| bail!("terminal unavailable")).unwrap_err();
        assert!(error.to_string().contains("terminal unavailable"));
    }

    #[test]
    fn no_ready_devices_never_opens_a_picker_and_missing_models_have_labels() {
        for list in ["List of devices attached\n", "phone\tunauthorized\nemulator-5554\toffline\n"] {
            assert!(select_serial(list, None, no_picker).unwrap_err().to_string().contains("no authorized"));
        }
        assert_eq!(select_serial("phone\tdevice\nemulator-5554\tdevice\n", None, |devices| {
            assert_eq!(devices[0].label(), "Android device — phone");
            assert_eq!(devices[1].label(), "Android emulator — emulator-5554");
            Ok(Some(1))
        }).unwrap(), "emulator-5554");
    }

    #[test]
    fn rejects_missing_or_unsupported_abis_instead_of_guessing_arm64() {
        assert_eq!(supported_abi("x86_64").unwrap(), "x86_64");
        assert!(supported_abi("").is_err());
        assert!(supported_abi("armeabi-v7a").is_err());
    }

    #[test]
    fn subprocess_arguments_preserve_paths_and_device_serials() {
        let device = Device { adb: "C:\\Android SDK\\adb.exe".into(), serial: "192.0.2.1:5555".into() };
        let mut command = device.command();
        command.args(["install", "-r"]).arg(Path::new("a directory/app.apk"));
        assert_eq!(command.get_program(), "C:\\Android SDK\\adb.exe");
        let args: Vec<_> = command.get_args().map(|s| s.to_string_lossy()).collect();
        assert_eq!(args, ["-s", "192.0.2.1:5555", "install", "-r", "a directory/app.apk"]);
    }

    #[test]
    #[cfg(unix)]
    fn failed_install_never_uninstalls_the_existing_app() {
        let (fixture, adb) = test_support::fake_adb("echo 'INSTALL_FAILED_UPDATE_INCOMPATIBLE' >&2; exit 1");
        let apk = fixture.path().join("new.apk");
        std::fs::write(&apk, "APK").unwrap();
        let device = Device { adb: adb.to_string_lossy().into_owned(), serial: "phone".into() };
        let error = device.install(&apk).unwrap_err();
        assert!(format!("{error:#}").contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE"));
        let commands = std::fs::read_to_string(fixture.path().join("commands")).unwrap();
        assert_eq!(commands.lines().count(), 1);
        assert!(!commands.contains("uninstall"));
    }

    #[test]
    #[cfg(unix)]
    fn activity_manager_error_with_exit_zero_is_still_a_launch_failure() {
        let (_fixture, adb) = test_support::fake_adb("echo 'Error type 3'; echo 'Error: Activity class does not exist.'");
        let device = Device { adb: adb.to_string_lossy().into_owned(), serial: "phone".into() };
        assert!(device.launch("com.example.app/.Activity").unwrap_err().to_string().contains("could not launch"));
    }
}
