//! Generates the per-app `AndroidManifest.xml` and `res/values/strings.xml` that
//! `zkite build` feeds into `apk.rs`/`aab.rs`. `../PLAN.md` §3: unlike the Dev Shell
//! (one generic APK, `dev.zkite.devshell`), every built app needs its own package
//! name, label, and icon.
//!
//! No resource qualifiers (`values-es/`, `drawable-xhdpi/`) — same sidestep phase 0's
//! README documents for the Dev Shell, same reason: a framework that owns its own
//! rendering doesn't need them, and `pack` doesn't support them yet regardless.

/// Everything `zkite build` needs to know about *this* app that isn't already in the
/// staged payload. Comes from `pyproject.toml`'s `[project]` table plus the optional
/// `[tool.zkite.android]` override (`../PLAN.md` §3).
pub struct AppIdentity {
    pub package: String,
    pub label: String,
    pub version_code: u32,
    pub version_name: String,
    pub min_sdk: u32,
    pub target_sdk: u32,
    /// Fully-qualified launcher Activity — the shell class that hosts the app, which
    /// is what selects the *rendering backend* (`phase_3/PLAN.md` §14). Not the app's
    /// own package: `android:name` is independent of the manifest's `package`
    /// attribute, which is why one prebuilt shell class can front an app published
    /// under any applicationId, exactly as `dev.zkite.appshell.ZKiteActivity` already
    /// did for every phase-2 build.
    ///
    /// Use [`APPSHELL_ACTIVITY`] / [`NATIVESHELL_ACTIVITY`] rather than a literal —
    /// a typo here builds, signs, installs, and then fails at launch with
    /// `ClassNotFoundException`, with nothing upstream of the device to catch it.
    pub activity: String,
    /// Android permission names from `[tool.zkite.android] permissions`, already
    /// normalised to fully-qualified form by [`normalize_permission`].
    ///
    /// **Empty by default, and that default is why this field had to exist**
    /// (`phase_3/PLAN.md` §34.4). A generated manifest requested *no* permissions at
    /// all — a deliberate phase-2 decision (`../PLAN.md` §1.2: a shipped app must not
    /// carry the Dev Shell's `INTERNET`, which existed only for the reload socket) —
    /// and there was no way for an app to ask for one. So the first ZKite app to try
    /// to load remote data would have failed inside `socket`, on a device, with a
    /// bare `PermissionError` and nothing anywhere naming the manifest as the cause.
    /// The permission and its consumer stay absent *together* by default; an app that
    /// wants one now says so.
    pub permissions: Vec<String>,
}

/// `INTERNET` -> `android.permission.INTERNET`; anything already qualified (contains
/// a `.`) is left exactly as written.
///
/// Both spellings are accepted because both are what people write: Android's own
/// documentation uses the bare constant name in prose and the qualified one in XML,
/// and a `permissions = ["INTERNET"]` that silently produced a manifest entry Android
/// ignores would be the worst of the three possible behaviours. A vendor-specific or
/// custom permission (`com.example.SOMETHING`) passes through untouched.
pub fn normalize_permission(name: &str) -> String {
    let name = name.trim();
    if name.contains('.') {
        name.to_string()
    } else {
        format!("android.permission.{name}")
    }
}

/// Rejects a permission name that would produce a malformed manifest.
///
/// Deliberately conservative rather than a fixed allow-list of known Android
/// permissions: the set grows every release, vendors add their own, and a framework
/// that refused a permission Android had just introduced would be worse than useless.
/// What is checked is that the name is a legal AXML identifier-ish token, which is
/// what stops a stray quote or angle bracket from corrupting the XML.
pub fn validate_permission(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("permission name is empty".to_string());
    }
    if let Some(bad) = name.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '.' || *c == '_')) {
        return Err(format!(
            "permission '{name}' contains an illegal character {bad:?} — expected something like \
             \"INTERNET\" or \"android.permission.INTERNET\""
        ));
    }
    Ok(())
}

/// The WebView backend's shell class (`phase_2/appshell`) — the default.
pub const APPSHELL_ACTIVITY: &str = "dev.zkite.appshell.ZKiteActivity";

/// The native GPU renderer's shell class (`phase_3/gpu_harness`). Named for the
/// directory it is still built from; see `phase_3/PLAN.md` §14's note on why that
/// rename was deliberately not bundled into the switch that made it shippable.
pub const NATIVESHELL_ACTIVITY: &str = "dev.zkite.gpuharness.ZKiteActivity";

/// `dev.zkite.app.<sanitized-name>` — the placeholder namespace used when
/// `pyproject.toml` doesn't set `[tool.zkite.android] package`. Deliberately not a
/// real developer's namespace: zkite cannot guess a domain the developer owns, and a
/// shared, guessable default is a footgun for anyone who forgets to override it
/// before actually publishing — callers are expected to print a warning alongside
/// this (see `../../phase_1/cli/src/build_cmd.rs`).
pub fn placeholder_package(project_name: &str) -> String {
    format!("dev.zkite.app.{}", sanitize_package_segment(project_name))
}

/// A project name (`my-cool_app`) as a legal package *segment* (`my_cool_app`).
/// Project names allow `-`, package segments do not, so anything scaffolding or
/// deriving a package from a project name has to go through this — `zkite new
/// my-app` writing `com.example.my-app` would produce a pyproject that fails
/// [`validate_package`] on the user's very first build.
pub fn sanitize_package_segment(project_name: &str) -> String {
    project_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

/// The namespace `zkite new` scaffolds into a fresh `pyproject.toml`
/// (`phase_3/PLAN.md` §21.2). Reserved for documentation by convention, and Google
/// Play rejects uploads under it — which is the point: it cannot be shipped by
/// accident, and [`placeholder_namespace`] turns that into a build-time warning
/// rather than an upload-time rejection.
pub const SCAFFOLD_NAMESPACE: &str = "com.example.";

/// `Some(why)` if `package` is one of the two namespaces zkite hands out itself and
/// no developer should publish under. Callers warn; this is deliberately not a hard
/// error, because both are correct for local builds and device testing — it is only
/// *publishing* under them that fails.
pub fn placeholder_namespace(package: &str) -> Option<&'static str> {
    if package.starts_with(SCAFFOLD_NAMESPACE) {
        Some("com.example.* is reserved for documentation and Google Play rejects it at upload")
    } else if package.starts_with("dev.zkite.app.") {
        Some("dev.zkite.app.* is zkite's own fallback namespace, not one you own")
    } else {
        None
    }
}

/// Validates that `package` is a syntactically legal Android application ID:
/// dot-separated, each segment starts with a letter, ASCII alphanumerics and
/// underscores only, at least two segments. Android's own installer rejects
/// anything else; failing here at build time is `zkite build`'s job to do with a
/// message that names the actual problem, not a bare install failure later.
pub fn validate_package(package: &str) -> Result<(), String> {
    let segments: Vec<&str> = package.split('.').collect();
    if segments.len() < 2 {
        return Err(format!(
            "{package:?} isn't a valid Android package name — needs at least two \
             dot-separated segments, e.g. \"com.example.myapp\""
        ));
    }
    for seg in &segments {
        let mut chars = seg.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
        let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !first_ok || !rest_ok || seg.is_empty() {
            return Err(format!(
                "{package:?} isn't a valid Android package name — segment {seg:?} must \
                 start with a letter and contain only letters, digits, or underscores"
            ));
        }
    }
    Ok(())
}

/// Google Play's hard ceiling on `android:versionCode`. An upload above it is
/// rejected outright.
pub const MAX_VERSION_CODE: u32 = 2_100_000_000;

/// Per-ABI rank for [`abi_version_code`]. **These numbers are permanent.**
///
/// Play's multiple-APK rule is that when more than one APK in a release can serve a
/// device, the device gets the one with the **higher** version code. ABI support
/// overlaps — a 64-bit ARM device also runs `armeabi-v7a`, an x86_64 device also runs
/// `x86` — so the more specific ABI must rank higher or 64-bit phones would be served
/// the 32-bit build.
///
/// ZKite only ships `arm64-v8a` and `x86_64` today. The two 32-bit ranks are listed
/// anyway, and that is the load-bearing part: adding an ABI later must not *renumber*
/// an existing one. A renumber that lowered a shipped ABI's code would be rejected by
/// Play forever, because a version code can never go down for a given APK.
const ABI_RANKS: [(&str, u32); 4] =
    [("armeabi-v7a", 1), ("x86", 2), ("arm64-v8a", 3), ("x86_64", 4)];

/// The `versionCode` for one ABI's APK, derived from the project's base
/// `[tool.zkite.android] version_code`.
///
/// **The bug this exists for (`phase_3/PLAN.md` §17.4 item 2).** `build_cmd.rs` built
/// the manifest *once*, outside the per-ABI loop, so every per-ABI APK carried a
/// byte-identical `versionCode` — a pair Play rejects, and it failed at *upload* with
/// a confusing message rather than at build.
///
/// **The scheme is `base * 10 + rank`, which is deliberately not the one Google's own
/// docs show.** Their example is `rank * 1000 + base`, which satisfies "distinct
/// within a release" but is not monotonic in `base`: at `rank*1000`, arm64 at base 1
/// (3001) outranks x86_64 at base 2 (4002)… but arm64 at base 2 (3002) is *lower* than
/// x86_64 at base 1 (4001), so bumping the base does not raise every ABI above every
/// code of the previous base. `base * 10 + rank` does, so "I bumped my version" means
/// what a developer expects for every ABI at once. The cost is a ceiling on `rank`
/// (9 ABIs), which is five more than Android has.
pub fn abi_version_code(base: u32, abi: &str) -> Result<u32, String> {
    let rank = ABI_RANKS
        .iter()
        .find(|(name, _)| *name == abi)
        .map(|(_, r)| *r)
        .ok_or_else(|| {
            format!(
                "no version-code rank for ABI {abi:?} — add it to ABI_RANKS with a *new* \
                 number (never renumber an existing one: a version code can never go down \
                 for an already-published APK)"
            )
        })?;
    // Fail loudly at build, not at upload — the same posture
    // `assert_native_libs_are_sane` already takes for 16 KB alignment.
    let code = (base as u64) * 10 + rank as u64;
    if code > MAX_VERSION_CODE as u64 {
        return Err(format!(
            "version_code {base} for {abi} becomes {code}, above Play's ceiling of \
             {MAX_VERSION_CODE}. Multi-ABI APK builds multiply the base by 10 to leave \
             room for a per-ABI rank, so the base must stay under {}.",
            MAX_VERSION_CODE / 10
        ));
    }
    Ok(code as u32)
}

pub fn android_manifest(id: &AppIdentity) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="{package}"
    android:versionCode="{version_code}"
    android:versionName="{version_name}">

    <uses-sdk
        android:minSdkVersion="{min_sdk}"
        android:targetSdkVersion="{target_sdk}" />
{permissions}
    <uses-feature android:name="android.hardware.camera" android:required="false" />
    <uses-feature android:name="android.hardware.camera.autofocus" android:required="false" />
    <uses-feature android:name="android.hardware.microphone" android:required="false" />
    <uses-feature android:name="android.hardware.location" android:required="false" />
    <uses-feature android:name="android.hardware.location.gps" android:required="false" />
    <uses-feature android:name="android.hardware.location.network" android:required="false" />
    <application
        android:label="@string/app_name"
        android:icon="@drawable/icon"
        android:extractNativeLibs="true"
        android:hasCode="true">
        <!-- NOT SET, and deliberately: android:enableOnBackInvokedCallback="true".
             It would opt into the modern (API 33+) back dispatcher and predictive
             back. vendor/pack's AXML compiler does not know the attribute and fails
             the whole pack with UnknownAndroidInternalAttribute — the same shape as
             the android:configChanges flag-constant gap phase_1/PLAN.md §7 M4
             already hit and dropped for the same reason. Caught by bundler's own
             test suite rather than at install time, which is why it is recorded
             here instead of shipped.

             Back still works on every API level: with this flag absent the platform
             uses legacy back dispatch and calls Activity.onBackPressed(), which
             both shells override (phase_3/PLAN.md §16). Both shells *also* register
             an OnBackInvokedCallback, which is inert while this is unset and
             becomes the live path the moment it can be set — so the only thing
             missing today is the predictive-back animation, and there are no screen
             transitions to animate yet anyway (§15.1: no animation driver exists).
             The fix is one entry in pack's attribute table, which is the same
             small, self-contained upstream contribution phase 0's README already
             recommends for extra_files. -->

        <!-- This was "false", which is the better setting for size and install
             speed and is genuinely safe here — vendor/pack does store lib/**/*.so
             uncompressed and 16 KiB-aligned, so the platform can mmap them
             straight out of the APK rather than extracting a copy at install
             time. It is nonetheless wrong for ZKite, and the reason is not
             size-related at all:

             with extractNativeLibs="false" nothing is written to the app's
             nativeLibraryDir, so there is no filesystem path for CPython to
             dlopen an extension module from. `System.loadLibrary` still works
             (the platform loader understands APK-internal entries), but
             `_imp.create_dynamic` takes a path, and there isn't one. Every
             C-extension stdlib module — math, random, zlib, ssl, sqlite3 — is
             therefore unimportable, which is a much larger cost than the install
             footprint this flag saves. Measured directly: with "false", the
             runtime reported `0 native extension modules available`; with "true",
             67. See phase_3/PLAN.md §11.6.

             The unexplored third option is to keep "false" and dlopen through
             bionic's APK-internal `<apk>!/lib/<abi>/<name>.so` path form, which
             would give both. It needs the module list to come from somewhere
             other than a directory listing, and it was not attempted — noted
             here so the tradeoff is visible rather than looking like an
             oversight. -->

        <provider android:name="dev.zkite.runtime.SharedFiles"
            android:authorities="{package}.zkite.files"
            android:exported="false" android:grantUriPermissions="true" />
        <activity
            android:name="{activity}"
            android:label="@string/app_name"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>"#,
        package = id.package,
        version_code = id.version_code,
        version_name = xml_escape(&id.version_name),
        min_sdk = id.min_sdk,
        target_sdk = id.target_sdk,
        activity = id.activity,
        permissions = uses_permissions(&id.permissions),
    )
    .into_bytes()
}

/// The `<uses-permission>` block, or the empty string when there are none.
///
/// Empty rather than a blank line so an app that asks for nothing produces a manifest
/// byte-identical to every one built before this existed — which is what makes "no
/// permissions by default" checkable rather than merely intended.
fn uses_permissions(permissions: &[String]) -> String {
    if permissions.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n");
    for p in permissions {
        out.push_str(&format!("    <uses-permission android:name=\"{}\" />\n", xml_escape(p)));
    }
    out
}

pub fn strings_xml(id: &AppIdentity) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<resources>
    <string name="app_name">{label}</string>
</resources>"#,
        label = xml_escape(&id.label),
    )
    .into_bytes()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_package_sanitizes_and_lowercases() {
        assert_eq!(placeholder_package("My-App"), "dev.zkite.app.my_app");
    }

    #[test]
    fn a_scaffolded_package_is_always_valid() {
        // `zkite new` allows '-' in project names; package segments do not. This is the
        // regression that would hit every user of `zkite new my-app` on their first build.
        let pkg = format!("{}{}", SCAFFOLD_NAMESPACE, sanitize_package_segment("my-cool_App"));
        assert_eq!(pkg, "com.example.my_cool_app");
        assert!(validate_package(&pkg).is_ok());
    }

    #[test]
    fn both_zkite_owned_namespaces_are_flagged_and_a_real_one_is_not() {
        assert!(placeholder_namespace("com.example.myapp").is_some());
        assert!(placeholder_namespace("dev.zkite.app.myapp").is_some());
        assert!(placeholder_namespace("com.acme.myapp").is_none());
    }

    #[test]
    fn validate_package_accepts_a_normal_reverse_dns_name() {
        assert!(validate_package("com.example.myapp").is_ok());
        assert!(validate_package("dev.zkite.app.myapp").is_ok());
    }

    #[test]
    fn validate_package_rejects_single_segment() {
        assert!(validate_package("myapp").is_err());
    }

    #[test]
    fn validate_package_rejects_segment_starting_with_digit() {
        assert!(validate_package("com.1example.myapp").is_err());
    }

    #[test]
    fn validate_package_rejects_hyphens() {
        assert!(validate_package("com.example.my-app").is_err());
    }

    /// `phase_3/PLAN.md` §17.4 item 2: every per-ABI APK carried a byte-identical
    /// `versionCode`, which Play's multiple-APK support rejects — at *upload*, with a
    /// confusing message, after a build that looked fine.
    ///
    /// The negative control for this is the pre-fix behaviour itself: `build_cmd.rs`
    /// used `id.version_code` directly at every ABI, i.e. `abi_version_code` returning
    /// `base` unchanged. Under that, `distinct` below collapses to 1 and this fails.
    #[test]
    fn every_abi_gets_a_distinct_version_code() {
        let abis = ["arm64-v8a", "x86_64", "armeabi-v7a", "x86"];
        let codes: Vec<u32> =
            abis.iter().map(|a| abi_version_code(7, a).unwrap()).collect();
        let distinct: std::collections::HashSet<&u32> = codes.iter().collect();
        assert_eq!(
            distinct.len(),
            abis.len(),
            "each ABI in one release needs its own code; got {codes:?}"
        );
    }

    /// The rule that makes the ranks non-arbitrary: when two APKs in a release can both
    /// serve a device, Play hands over the one with the **higher** code. A 64-bit
    /// device also runs the 32-bit ABI of its family, so the 64-bit build has to win.
    #[test]
    fn the_64_bit_abi_outranks_the_32_bit_one_of_its_own_family() {
        assert!(abi_version_code(1, "arm64-v8a").unwrap() > abi_version_code(1, "armeabi-v7a").unwrap());
        assert!(abi_version_code(1, "x86_64").unwrap() > abi_version_code(1, "x86").unwrap());
    }

    /// Monotonicity in the base is the reason for `base * 10 + rank` rather than
    /// Google's own `rank * 1000 + base` example: bumping `version_code` must raise
    /// *every* ABI above *every* code of the previous base, or "I bumped my version"
    /// silently means something else per ABI.
    #[test]
    fn bumping_the_base_raises_every_abi_above_every_previous_code() {
        let abis = ["armeabi-v7a", "x86", "arm64-v8a", "x86_64"];
        let lowest_at_2 = abis.iter().map(|a| abi_version_code(2, a).unwrap()).min().unwrap();
        let highest_at_1 = abis.iter().map(|a| abi_version_code(1, a).unwrap()).max().unwrap();
        assert!(
            lowest_at_2 > highest_at_1,
            "base 2's lowest ({lowest_at_2}) must beat base 1's highest ({highest_at_1}) — \
             this is what rank*1000+base does NOT satisfy"
        );
    }

    #[test]
    fn a_base_over_the_play_ceiling_fails_at_build_not_at_upload() {
        let err = abi_version_code(MAX_VERSION_CODE, "arm64-v8a").unwrap_err();
        assert!(err.contains("ceiling"), "{err}");
        // And the boundary is where the arithmetic says it is, not one off.
        assert!(abi_version_code(MAX_VERSION_CODE / 10 - 1, "x86_64").is_ok());
        assert!(abi_version_code(MAX_VERSION_CODE / 10 + 1, "armeabi-v7a").is_err());
    }

    #[test]
    fn an_unknown_abi_is_refused_rather_than_defaulted() {
        // A silent default would give two ABIs the same code again, which is the whole
        // bug. The message also has to warn against renumbering.
        let err = abi_version_code(1, "riscv64").unwrap_err();
        assert!(err.contains("riscv64"), "{err}");
        assert!(err.contains("never renumber"), "{err}");
    }

    #[test]
    fn the_manifest_carries_whatever_version_code_it_was_given() {
        // The join between `abi_version_code` and the emitted XML. Without this, the
        // arithmetic could be right and the manifest still hardcode something.
        let mut id = AppIdentity {
            package: "com.example.myapp".into(),
            label: "My App".into(),
            version_code: abi_version_code(42, "x86_64").unwrap(),
            version_name: "0.1.0".into(),
            min_sdk: 26,
            target_sdk: 34,
            activity: APPSHELL_ACTIVITY.into(),
            permissions: Vec::new(),
        };
        assert_eq!(id.version_code, 424);
        let xml = String::from_utf8(android_manifest(&id)).unwrap();
        assert!(xml.contains(r#"android:versionCode="424""#), "{xml}");

        id.version_code = abi_version_code(42, "arm64-v8a").unwrap();
        let other = String::from_utf8(android_manifest(&id)).unwrap();
        assert!(other.contains(r#"android:versionCode="423""#), "{other}");
        assert_ne!(xml, other, "two ABIs must not produce the same manifest bytes");
    }

    #[test]
    fn manifest_embeds_the_package_and_activity() {
        let id = AppIdentity {
            package: "com.example.myapp".into(),
            label: "My App".into(),
            version_code: 1,
            version_name: "0.1.0".into(),
            min_sdk: 26,
            target_sdk: 34,
            activity: APPSHELL_ACTIVITY.into(),
            permissions: Vec::new(),
        };
        let xml = String::from_utf8(android_manifest(&id)).unwrap();
        assert!(xml.contains(r#"package="com.example.myapp""#));
        assert!(xml.contains("dev.zkite.appshell.ZKiteActivity"));
        assert!(!xml.contains("uses-permission")); // no INTERNET — see ../PLAN.md §1.2
    }

    /// `[tool.zkite.android] permissions` (`phase_3/PLAN.md` §34.4). An app could not
    /// request a permission at all, so the first one to try to load remote data would
    /// have failed on the device inside `socket`, with nothing anywhere naming the
    /// manifest as the cause.
    #[test]
    fn declared_permissions_reach_the_manifest_and_nothing_else_changes() {
        let base = || AppIdentity {
            package: "com.example.myapp".into(),
            label: "My App".into(),
            version_code: 1,
            version_name: "0.1.0".into(),
            min_sdk: 26,
            target_sdk: 34,
            activity: APPSHELL_ACTIVITY.into(),
            permissions: Vec::new(),
        };

        let none = String::from_utf8(android_manifest(&base())).unwrap();

        let mut id = base();
        id.permissions = vec![normalize_permission("INTERNET")];
        let with_internet = String::from_utf8(android_manifest(&id)).unwrap();
        assert!(
            with_internet.contains(r#"<uses-permission android:name="android.permission.INTERNET" />"#),
            "{with_internet}"
        );

        // **The default is the half worth pinning.** Requesting nothing must produce
        // the byte-identical manifest every build before this made — `../PLAN.md`
        // §1.2's "a shipped app must not carry the Dev Shell's INTERNET" is a promise
        // about apps that never asked, and adding a way to ask must not weaken it.
        assert_eq!(
            none,
            with_internet.replace(
                "\n    <uses-permission android:name=\"android.permission.INTERNET\" />\n",
                ""
            ),
            "declaring a permission must change exactly the permission block"
        );
    }

    #[test]
    fn a_bare_permission_name_is_qualified_and_a_qualified_one_is_left_alone() {
        assert_eq!(normalize_permission("INTERNET"), "android.permission.INTERNET");
        assert_eq!(normalize_permission("  INTERNET  "), "android.permission.INTERNET");
        // Already qualified — including a vendor/custom one, which must not be
        // rewritten into a namespace it does not belong to.
        assert_eq!(
            normalize_permission("android.permission.CAMERA"),
            "android.permission.CAMERA"
        );
        assert_eq!(normalize_permission("com.example.OWN"), "com.example.OWN");
    }

    #[test]
    fn a_permission_name_that_would_corrupt_the_xml_is_rejected() {
        assert!(validate_permission("INTERNET").is_ok());
        assert!(validate_permission("android.permission.ACCESS_FINE_LOCATION").is_ok());
        assert!(validate_permission("").is_err());
        assert!(validate_permission("A\" />< ").is_err());
        assert!(validate_permission("has space").is_err());
    }

    /// The backend switch (`phase_3/PLAN.md` §14) is *only* visible in the built APK
    /// as this one attribute plus which `.so`/`classes.dex` got staged — the app's
    /// own package, label and every other manifest field are identical either way.
    /// So this asserts the two really do differ here and agree everywhere else: an
    /// `activity` that silently stayed on the appshell default would produce a
    /// native-staged APK that boots the WebView class and dies on
    /// `System.loadLibrary("zkiteappshell")`, which is a launch-time failure with
    /// nothing host-side to catch it.
    #[test]
    fn the_backend_selects_the_activity_and_changes_nothing_else() {
        let identity = |activity: &str| AppIdentity {
            package: "com.example.myapp".into(),
            label: "My App".into(),
            version_code: 1,
            version_name: "0.1.0".into(),
            min_sdk: 26,
            target_sdk: 34,
            activity: activity.into(),
            permissions: Vec::new(),
        };
        let web = String::from_utf8(android_manifest(&identity(APPSHELL_ACTIVITY))).unwrap();
        let native = String::from_utf8(android_manifest(&identity(NATIVESHELL_ACTIVITY))).unwrap();

        assert!(web.contains(r#"android:name="dev.zkite.appshell.ZKiteActivity""#));
        assert!(native.contains(r#"android:name="dev.zkite.gpuharness.ZKiteActivity""#));
        assert!(!native.contains("appshell"));
        assert_eq!(
            web.replace(APPSHELL_ACTIVITY, "<ACTIVITY>"),
            native.replace(NATIVESHELL_ACTIVITY, "<ACTIVITY>"),
            "the two backends' manifests must differ in the activity name and nothing else"
        );
    }

    #[test]
    fn extract_native_libs_is_true_so_c_extensions_can_be_dlopened() {
        // Deliberately "true" even though pack stores lib/**/*.so uncompressed and
        // page-aligned, which would make "false" safe and cheaper. With "false"
        // nothing lands in nativeLibraryDir, so CPython has no path to dlopen an
        // extension module from and every C-extension stdlib module becomes
        // unimportable — measured on a real device as `0 native extension modules
        // available` vs 67. Do not flip this back without first providing another
        // way to load them (see the manifest's own comment). phase_3/PLAN.md §11.6.
        let id = AppIdentity {
            package: "com.example.myapp".into(),
            label: "My App".into(),
            version_code: 1,
            version_name: "0.1.0".into(),
            min_sdk: 26,
            target_sdk: 34,
            activity: APPSHELL_ACTIVITY.into(),
            permissions: Vec::new(),
        };
        let xml = String::from_utf8(android_manifest(&id)).unwrap();
        assert!(xml.contains(r#"android:extractNativeLibs="true""#));
    }

    #[test]
    fn strings_xml_escapes_the_label() {
        let id = AppIdentity {
            package: "com.example.myapp".into(),
            label: "Bob & Sons \"App\"".into(),
            version_code: 1,
            version_name: "0.1.0".into(),
            min_sdk: 26,
            target_sdk: 34,
            activity: APPSHELL_ACTIVITY.into(),
            permissions: Vec::new(),
        };
        let xml = String::from_utf8(strings_xml(&id)).unwrap();
        assert!(xml.contains("Bob &amp; Sons &quot;App&quot;"));
    }
}
