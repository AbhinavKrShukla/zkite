//! `zkite build apk|aab` — phase 2 (`../../phase_2/PLAN.md`). Turns a project
//! directory into a signed, standalone APK/AAB. Everything here runs on **the
//! user's machine**; nothing shells out to `cargo`, `javac`, `d8`, or the NDK — those
//! already ran once, in *our* release pipeline, to produce the prebuilt artifacts
//! this command reads (`<shell>/artifacts/`, same bucket as `classes.dex` always
//! was). See `../../phase_2/PLAN.md` §1.1/§1.2/§1.5 for why that split matters.
//!
//! **Two shells, one pipeline** (`phase_3/PLAN.md` §14/§15). [`Backend`] picks
//! between `phase_3/gpu_harness` (the native GPU renderer — the default since §15.4)
//! and `phase_2/appshell` (WebView, `--backend webview`). Everything else below — wheel resolution, pyc, pruning, the app
//! zip, signing, alignment — is identical for both, because the app being built is
//! identical for both: plan1.md §5's "the Python API is backend-agnostic... this lets
//! us ship a boring backend first and swap in the good one later without breaking
//! anyone" is exactly this function having one code path and four switched inputs.
//!
//! Pipeline (plan1.md §7, as phase 2 actually built it):
//! 1. resolve   `wheels.rs` — pip + Android platform tags, pure-Python realistically
//! 2. compile   `.py` -> `.pyc` via a matching-version host Python (phase_2/PLAN.md §1.4)
//! 3. bundle    app (+ deps) -> `assets/app.zip`, zipimport-able
//! 4. stage     the selected shell's `.so`s + `classes.dex`, extension modules, pruned stdlib
//! 5. resources per-app `AndroidManifest.xml`/`strings.xml`/icon -> ARSC + binary AXML
//! 6. sign      user's `--keystore` or an ephemeral testing key (loud warning)
//! 7. align     4-byte zip alignment
//! 8. emit      `<name>-<abi>.apk` (one per ABI) or `<name>.aab` (all ABIs, one bundle)

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};

use zkite_bundler::apk::Package;
use zkite_bundler::keystore;
use zkite_bundler::manifest_gen::{self, AppIdentity, APPSHELL_ACTIVITY, NATIVESHELL_ACTIVITY};
use zkite_bundler::zip_extras;
use pack_asset_compiler::resource_internal_types::FileResource;
use pack_sign::crypto_keys::Keys;

use crate::wheels::{self, AndroidTarget};

const PY_VERSION: &str = "3.14";
const DEFAULT_MIN_SDK: u32 = 26;
// Google Play's phone/tablet submission floor as of August 2026. Raising a target
// is a platform behavior change; Android 16 device checks remain a release gate.
const DEFAULT_TARGET_SDK: u32 = 36;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BuildFormat {
    Apk,
    Aab,
}

/// Which renderer the built app runs on — plan1.md §5's "ship the WebView backend
/// first... the backend is swappable underneath without touching app code", made
/// real. `phase_3/PLAN.md` §14.
///
/// The app's own source is byte-identical either way: this picks a *shell*, and the
/// shell is what owns rendering. Nothing in `zkite_py` or a user's `main.py` knows
/// which one it is running under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    /// `phase_2/appshell` — a `WebView` running `phase_1/webruntime`. No longer the
    /// default (§15.4), but still the backend with the shorter ledger: it wins cold
    /// start and memory, and gets IME, selection, Autofill and accessibility from
    /// Blink for free (`phase_3/PLAN.md` §13.4).
    WebView,
    /// `phase_3/gpu_harness` — taffy + parley + vello_hybrid on a `SurfaceView`.
    /// **The default.** See `phase_3/PLAN.md` §15.4 for why that was flipped against
    /// §13.4's own measured ledger, and §15.1 for what it still cannot draw.
    Native,
}

impl Backend {
    /// What you get with no `--backend` and no `[tool.zkite] backend`.
    ///
    /// **Flipped to `Native` in §15.4 as an explicit product decision, against the
    /// ledger §13.4 measured** — native still loses cold start (~490ms), memory
    /// (~110MB) and 15.5MB of binary, and §13.4's own conclusion was "the switch does
    /// not flip on this evidence." It flipped anyway, for reasons that are about
    /// control rather than benchmarks (see §15.4). Recorded here so nobody reads this
    /// constant as implying the numbers changed.
    pub const DEFAULT: Self = Backend::Native;

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "webview" | "web" => Ok(Backend::WebView),
            "native" | "gpu" => Ok(Backend::Native),
            other => bail!(
                "unknown backend {other:?} — expected \"native\" (default) or \"webview\""
            ),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Backend::WebView => "webview",
            Backend::Native => "native",
        }
    }

    /// Everything that differs between the two backends at build time. Deliberately
    /// one struct rather than a `match` at each of the five staging sites: the way
    /// this goes wrong is a site that *isn't* switched (staging appshell's
    /// `classes.dex` next to the native `.so`, say), which builds and signs happily
    /// and dies at launch. Having one place that enumerates the differences makes
    /// "did I switch all of them" readable instead of a grep.
    fn shell(self, repo_root: &Path) -> Shell {
        match self {
            Backend::WebView => Shell {
                dir: repo_root.join("phase_2/appshell"),
                build_script: "phase_2/appshell/scripts/build.sh",
                libs: &["libzkiteappshell.so", "libpython3.14.so"],
                activity: APPSHELL_ACTIVITY,
            },
            Backend::Native => Shell {
                dir: repo_root.join("phase_3/gpu_harness"),
                build_script: "phase_3/gpu_harness/scripts/build.sh",
                // libc++_shared.so is a real DT_NEEDED of the native cdylib, via
                // vendor/gametextinput's C++ (phase_3/PLAN.md §7.3 item 1). Omitting
                // it is an `UnsatisfiedLinkError` at `System.loadLibrary`, before a
                // line of app code runs.
                libs: &["libzkitegpuharness.so", "libpython3.14.so", "libc++_shared.so"],
                activity: NATIVESHELL_ACTIVITY,
            },
        }
    }
}

struct Shell {
    dir: PathBuf,
    /// Repo-relative, for the "run this first" half of a missing-artifact error.
    build_script: &'static str,
    /// `lib/<abi>/` entries copied verbatim out of `<dir>/artifacts/<abi>/`.
    libs: &'static [&'static str],
    activity: &'static str,
}

impl Shell {
    fn artifacts(&self) -> PathBuf {
        self.dir.join("artifacts")
    }
}

pub struct BuildArgs {
    pub project_dir: PathBuf,
    pub format: BuildFormat,
    /// "arm64-v8a" / "x86_64" — empty means "every ABI we have prebuilt artifacts
    /// for, for the selected backend".
    pub abis: Vec<String>,
    pub keystore: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub no_prune_stdlib: bool,
    /// `--backend`. Overrides `[tool.zkite] backend` in pyproject.toml; if neither is
    /// set the default is [`Backend::Native`] (`phase_3/PLAN.md` §15.4).
    pub backend: Option<Backend>,
}

struct ProjectConfig {
    name: String,
    identity: AppIdentity,
    dependencies: Vec<String>,
    /// `[tool.zkite] backend`, if the project pinned one.
    backend: Option<Backend>,
    /// `[tool.zkite.android] icon`, if the project set one. `None` means the bundled
    /// default (see [`load_icon`]).
    icon: Option<IconSource>,
}

/// A user-supplied launcher icon: `[tool.zkite.android] icon` (phase_2/PLAN.md §3).
///
/// Both halves are kept because they say different things in an error. `declared` is
/// what the user typed and can grep for; `path` is where that actually landed, which
/// is the half that explains a "no such file" they don't expect.
struct IconSource {
    declared: String,
    path: PathBuf,
}

/// Exact artifacts and Android identity produced by this invocation. `zkite run`
/// consumes this instead of guessing filenames or selecting an old APK in dist/.
pub struct BuildOutput {
    pub artifacts: Vec<PathBuf>,
    pub package: String,
    pub activity: String,
}

pub fn run(args: BuildArgs) -> Result<BuildOutput> {
    let t0 = Instant::now();
    // Resolved and *reported* before any work: on a packaged install the thing that
    // goes wrong is silently reading a stale checkout instead of the wheel's own
    // payload, and §23.2 is explicit that a resolver nobody can see the answer of is
    // how the first person to debug that gets no clue why.
    let payload = crate::payload::get()?;
    let repo_root = payload.root();
    println!("  payload: {}", payload.describe());
    let phase0 = repo_root.join("phase_0");

    let mut config = load_project_config(&args.project_dir)?;
    manifest_gen::validate_package(&config.identity.package).map_err(|e| anyhow::anyhow!(e))?;
    // Both zkite-owned namespaces are legal Android package names and install fine on
    // a device — they only fail at *upload*, which is the worst place to find out.
    // Same posture as `assert_native_libs_are_sane`: fail loudly at build, not at Play.
    if let Some(why) = manifest_gen::placeholder_namespace(&config.identity.package) {
        eprintln!(
            "  ! android.package is still {:?} — {why}. \
             Set one under a domain you own before publishing; it can never change after \
             your first release.",
            config.identity.package
        );
    }

    // Read and validate the icon *here*, beside the package check, rather than at the
    // resource-assembly site 80 lines down where it is actually consumed. Everything
    // between the two is expensive — wheel resolution, pyc, stdlib pruning, signing —
    // so a typo'd `icon =` path took a measured 9.2s to report before this moved.
    // Cheap checks first is the same posture as `validate_package` above.
    let icon = load_icon(config.icon.as_ref(), &repo_root)?;

    // --backend beats [tool.zkite] backend beats the default, so a one-off
    // `zkite build apk --backend native` never has to edit (and then remember to
    // un-edit) pyproject.toml.
    let backend = args.backend.or(config.backend).unwrap_or(Backend::DEFAULT);
    let shell = backend.shell(&repo_root);
    config.identity.activity = shell.activity.to_string();

    let abis = resolve_abis(&args.abis, &shell)?;
    println!(
        "building {} for {} on the {} backend: {}",
        config.name,
        format_name(args.format),
        backend.name(),
        abis.join(", ")
    );
    // Before anything expensive, and before anything is copied: does the prebuilt
    // shell we are about to ship verbatim actually come from the source in this
    // payload? `zkite build` compiles none of it (phase_2/PLAN.md §1.1), so this is
    // the only place the question can be asked — and twice it has gone unasked and
    // shipped a binary from a negative-control build (§34.8, and again in §35).
    crate::provenance::verify_shell(&shell.artifacts(), repo_root, shell.build_script)?;

    let work = tempfile::tempdir().context("creating a staging tempdir")?;
    let app_dir = work.path().join("app_src");
    std::fs::create_dir_all(&app_dir)?;

    let python = crate::host_python::find(Some((3, 14)))
        .context("Python 3.14 is required for Android bytecode and stdlib reachability")?;
    println!("  using {} for pyc compilation + stdlib reachability", python.display());
    copy_app_source(&args.project_dir, &app_dir)?;
    resolve_and_stage_dependencies(&config, &app_dir, work.path(), &abis, &python)?;

    let assets = work.path().join("assets");
    std::fs::create_dir_all(&assets)?;
    for notice in runtime_notices(repo_root)? {
        std::fs::write(work.path().join(notice.path), notice.data)?;
    }
    zip_zkite_framework(&repo_root, &assets.join("zkite-framework.zip"))?;
    stage_backend_assets(backend, &repo_root, &shell, &assets)?;

    // `python-x86_64` regardless of the ABI being built, and that is **correct, not a
    // leftover** — §21.6 flagged it as "undocumented and reads like a bug", which is
    // exactly why it is documented here instead of fixed (../../phase_3/PLAN.md §23.4).
    //
    // What goes into `python-stdlib.zip` is the **pure-Python** stdlib, which is
    // architecture-independent: the same `.py` files ship for every ABI. The parts that
    // genuinely are per-architecture — `lib-dynload/*.so` and the `lib*_python.so` set —
    // are staged separately, per ABI, by `stage_backend_assets`/the native-lib staging
    // below, and never come from here.
    //
    // The obvious "fix" (resolve the prefix from the ABI being built) would produce a
    // byte-identical zip from a second copy of the same files, and would **double the
    // payload** a `pip install zkite` wheel has to carry — one 16 MB stdlib per ABI
    // instead of one shared copy (§23.3). Sharing it is what makes the wheel ~19 MB.
    let stdlib_prefix = phase0.join("vendor/python-x86_64/prefix");
    if args.no_prune_stdlib {
        copy_stdlib_unpruned(&stdlib_prefix, &assets.join("python-stdlib.zip"))?;
    } else {
        prune_stdlib(&python, &stdlib_prefix, &app_dir, &repo_root, &assets.join("python-stdlib.zip"))?;
    }

    // Reachability must inspect the source, including installed dependencies,
    // before compile_pyc removes .py files. Otherwise modulefinder sees no app
    // imports and silently drops stdlib modules the app will need on the device.
    pyc_compile(&python, repo_root, &app_dir)?;
    zip_dir(&app_dir, &assets.join("app.zip"))?;

    let keys = load_or_generate_keys(args.keystore.as_deref(), &args.project_dir)?;

    // Resources are ABI-independent; the *manifest* is not, because its `versionCode`
    // now varies per ABI (§17.4 item 2). Built as a closure rather than hoisted, so a
    // future ABI-dependent manifest attribute cannot be added outside the loop the way
    // `versionCode` was.
    let resources = |icon: Vec<u8>| {
        let mut resources = zkite_bundler::launcher_icon::resources(icon);
        resources.push(FileResource {
            subdirectory: "values".into(), name: "strings.xml".into(), resource_id: 0,
            contents: manifest_gen::strings_xml(&config.identity),
        });
        resources
    };

    let out_dir = args.out.clone().unwrap_or_else(|| args.project_dir.join("dist"));
    std::fs::create_dir_all(&out_dir)?;
    let mut artifacts = Vec::new();

    match args.format {
        BuildFormat::Apk => {
            for abi in &abis {
                // **Per ABI, inside the loop.** Play's multiple-APK support requires a
                // distinct version code per APK, and building one manifest outside this
                // loop gave every ABI the same one — rejected at upload, with a
                // confusing message, after a successful build (§17.4 item 2).
                let identity_abi_version = AppIdentity {
                    package: config.identity.package.clone(),
                    label: config.identity.label.clone(),
                    version_code: manifest_gen::abi_version_code(
                        config.identity.version_code,
                        abi,
                    )
                    .map_err(|e| anyhow::anyhow!(e))?,
                    version_name: config.identity.version_name.clone(),
                    min_sdk: config.identity.min_sdk,
                    target_sdk: config.identity.target_sdk,
                    permissions: config.identity.permissions.clone(),
                    activity: config.identity.activity.clone(),
                };
                println!(
                    "  [{abi}] versionCode {} (base {} + per-ABI rank)",
                    identity_abi_version.version_code, config.identity.version_code,
                );
                let package = Package {
                    android_manifest: manifest_gen::android_manifest(&identity_abi_version),
                    resources: resources(icon.clone()),
                };

                let stage = work.path().join(format!("stage-{abi}"));
                stage_common(&assets, &stage, "classes.dex", &shell)?;
                stage_native_libs(&shell, &phase0, abi, &stage)?;
                let extras = zip_extras::read_staging_dir(&stage)?;
                let checked = zkite_bundler::apk::assert_native_libs_are_sane(&extras)
                    .map_err(|e| anyhow::anyhow!(e))?;
                println!("  [{abi}] 16 KB page alignment: {checked} native libraries OK");

                let bytes = zkite_bundler::apk::compile_and_sign_apk_with_extras(
                    &package, extras, &keys,
                )
                .map_err(|e| anyhow::anyhow!("pack+sign failed: {e}"))?;
                let out_path = out_dir.join(format!("{}-{}.apk", config.name, abi));
                std::fs::write(&out_path, &bytes)?;
                println!(
                    "  wrote {} ({:.1} MB) — install with:\n    adb install -r {} && adb shell am start -n {}/{}",
                    out_path.display(),
                    bytes.len() as f64 / 1e6,
                    out_path.display(),
                    config.identity.package,
                    shell.activity,
                );
                artifacts.push(out_path);
            }
        }
        BuildFormat::Aab => {
            // **One artifact, so the base `version_code` is used unmodified** — there
            // is no collision to fix here, which is why §17.4 records that this format
            // was unaffected and is presumably why nobody hit the APK bug. Play
            // generates the per-ABI splits itself and stamps them with the bundle's own
            // code.
            //
            // The hazard that follows from the two formats differing is named rather
            // than papered over: an app that ships APKs at `base*10+rank` and then
            // switches to an AAB at `base` would be uploading a *lower* code, which
            // Play rejects. Bumping `version_code` past the last APK release fixes it,
            // and `zkite build` says so here rather than leaving it to the upload
            // error. See §29's known gaps.
            if abis.len() > 1 {
                eprintln!(
                    "  ! versionCode {} is used as-is for the bundle. If you have \
                     previously uploaded per-ABI *APKs* of this app, those carried \
                     {}-style codes (base x 10 + ABI rank) — bump \
                     android.version_code above the highest one you \
                     published, or Play will reject this as a downgrade.",
                    config.identity.version_code,
                    manifest_gen::abi_version_code(config.identity.version_code, &abis[0])
                        .unwrap_or(config.identity.version_code),
                );
            }
            if config.identity.version_code > manifest_gen::MAX_VERSION_CODE {
                bail!(
                    "version_code {} is above Play's ceiling of {}",
                    config.identity.version_code,
                    manifest_gen::MAX_VERSION_CODE
                );
            }
            let package = Package {
                android_manifest: manifest_gen::android_manifest(&config.identity),
                resources: resources(icon.clone()),
            };
            let stage = work.path().join("stage-aab");
            stage_common(&assets, &stage, "dex/classes.dex", &shell)?;
            for abi in &abis {
                stage_native_libs(&shell, &phase0, abi, &stage)?;
            }
            let extras = zip_extras::read_staging_dir(&stage)?;
            let checked = zkite_bundler::apk::assert_native_libs_are_sane(&extras)
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("  16 KB page alignment: {checked} native libraries OK");

            let bytes = zkite_bundler::aab::compile_and_sign_aab_with_extras(&package, extras, &keys)
                .map_err(|e| anyhow::anyhow!("pack+sign failed: {e}"))?;
            let out_path = out_dir.join(format!("{}.aab", config.name));
            std::fs::write(&out_path, &bytes)?;
            println!("  wrote {} ({:.1} MB)", out_path.display(), bytes.len() as f64 / 1e6);
            artifacts.push(out_path);
        }
    }

    println!("done in {:.2}s", t0.elapsed().as_secs_f64());
    Ok(BuildOutput { artifacts, package: config.identity.package, activity: config.identity.activity })
}

pub fn project_backend(project_dir: &Path, override_backend: Option<Backend>) -> Result<Backend> {
    let config = load_project_config(project_dir)?;
    anyhow::ensure!(project_dir.join("main.py").is_file(), "project has no main.py: {}", project_dir.display());
    Ok(override_backend.or(config.backend).unwrap_or(Backend::DEFAULT))
}

fn format_name(f: BuildFormat) -> &'static str {
    match f {
        BuildFormat::Apk => "apk",
        BuildFormat::Aab => "aab",
    }
}

// ------------------------------------------------------------------ project config

/// App identity belongs to app.json; pyproject is only an optional dependency source.
/// Existing projects without app.json keep their legacy configuration.
pub(crate) fn project_document(project_dir: &Path) -> Result<toml::Value> {
    let pyproject = project_dir.join("pyproject.toml");
    let mut doc: toml::Value = if pyproject.is_file() {
        std::fs::read_to_string(&pyproject)?.parse().context("parsing pyproject.toml")?
    } else { toml::Value::Table(Default::default()) };
    let path = project_dir.join("app.json");
    if path.is_file() {
        let app: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?).context("parsing app.json")?;
        let app = app.as_object().context("app.json must be an object")?;
        for key in app.keys() {
            anyhow::ensure!(["name", "version", "backend", "android", "dependencies"].contains(&key.as_str()), "unknown app.json setting: {key}");
        }
        let mut project = toml::map::Map::new();
        for key in ["name", "version"] {
            if let Some(value) = app.get(key) {
                project.insert(key.into(), toml::Value::try_from(value)?);
            }
        }
        let mut dependencies = doc.get("project").and_then(|p| p.get("dependencies"))
            .map(|v| v.as_array().cloned().context("pyproject.toml dependencies must be an array"))
            .transpose()?.unwrap_or_default();
        if let Some(legacy) = app.get("dependencies") {
            eprintln!("  app.json dependencies are deprecated; move them to pyproject.toml or requirements.txt");
            for value in legacy.as_array().context("app.json dependencies must be an array")? {
                let value = toml::Value::try_from(value)?;
                if !dependencies.contains(&value) { dependencies.push(value); }
            }
        }
        project.insert("dependencies".into(), toml::Value::Array(dependencies));
        let mut zkite = toml::map::Map::new();
        for key in ["backend", "android"] {
            if let Some(value) = app.get(key) { zkite.insert(key.into(), toml::Value::try_from(value)?); }
        }
        doc = toml::Value::Table(toml::map::Map::from_iter([
            ("project".into(), toml::Value::Table(project)),
            ("tool".into(), toml::Value::Table(toml::map::Map::from_iter([("zkite".into(), toml::Value::Table(zkite))]))),
        ]));
    }
    let requirements = project_dir.join("requirements.txt");
    if requirements.is_file() {
        let project = doc.as_table_mut().context("invalid project configuration")?
            .entry("project").or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut().context("project must be a table")?;
        let dependencies = project.entry("dependencies").or_insert_with(|| toml::Value::Array(vec![]))
            .as_array_mut().context("dependencies must be an array")?;
        for line in std::fs::read_to_string(requirements)?.lines() {
            let line = line.split(" #").next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            anyhow::ensure!(!line.starts_with('-') && !line.ends_with('\\'),
                "requirements.txt supports one package requirement per line; put pip options in your installer command");
            dependencies.push(toml::Value::String(line.into()));
        }
    }
    Ok(doc)
}

fn load_project_config(project_dir: &Path) -> Result<ProjectConfig> {
    let doc = project_document(project_dir)?;
    anyhow::ensure!(
        doc.get("tool").and_then(|t| t.get("kite")).is_none(),
        "legacy [tool.kite] configuration: rename it to [tool.zkite] (including [tool.zkite.android]); see Documentation.md for migration steps"
    );

    let project = doc.get("project").context("missing app.json (or legacy pyproject.toml [project])")?;
    let name = project
        .get("name")
        .and_then(|v| v.as_str())
        .context("app.json must contain a string `name` (legacy: pyproject.toml [project].name)")?
        .to_string();
    anyhow::ensure!(
        !name.is_empty() && !name.starts_with('.')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')),
        "[project] name must contain only letters, digits, '.', '_' or '-', and cannot begin with '.'"
    );
    if let Some(version) = project.get("version") {
        anyhow::ensure!(version.as_str().is_some_and(|s| !s.trim().is_empty()), "version must be a nonempty string");
    }
    let version = project.get("version").and_then(|v| v.as_str()).unwrap_or("0.1.0").to_string();
    let dependencies: Vec<String> = match project.get("dependencies") {
        None => Vec::new(),
        Some(value) => value.as_array().context("[project] dependencies must be an array of strings")?
            .iter().map(|value| value.as_str().filter(|s| !s.trim().is_empty())
                .map(str::to_string).context("each dependency must be a nonempty string"))
            .collect::<Result<Vec<_>>>()?,
    }
        .into_iter()
        .filter(|d| {
            let name = wheels_package_name(d).to_ascii_lowercase().replace(['_', '.'], "-");
            !matches!(name.as_str(), "zkite" | "zkite-framework")
        })
        .collect();

    let zkite = doc.get("tool").and_then(|t| t.get("zkite"));
    // `[tool.zkite] backend`, not `[tool.zkite.android] backend` — which renderer an
    // app runs on is not Android-specific (plan1.md §10 phase 4 already promises
    // iOS/desktop off the same renderer), and putting it under the android table
    // would be a name to regret.
    let backend = match zkite.and_then(|k| k.get("backend")) {
        Some(v) => Some(Backend::parse(v.as_str().context(
            "backend must be a string — \"native\" (default) or \"webview\"",
        )?)?),
        None => None,
    };
    let android = zkite.and_then(|k| k.get("android"));
    if let Some(android) = android {
        let table = android.as_table().context("android must be an object")?;
        for key in table.keys() {
            anyhow::ensure!(["package", "label", "version_code", "min_sdk", "target_sdk", "icon", "permissions"].contains(&key.as_str()), "unknown android setting: {key}");
        }
        for key in ["package", "label"] {
            if let Some(value) = table.get(key) {
                anyhow::ensure!(value.as_str().is_some_and(|s| !s.trim().is_empty()), "android.{key} must be a nonempty string");
            }
        }
    }

    let package = android
        .and_then(|a| a.get("package"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        // No warning here: `run()` warns about *any* zkite-owned namespace after
        // resolution, which covers both this fallback and the `com.example.*` that
        // `zkite new` scaffolds. Warning in both places printed it twice.
        .unwrap_or_else(|| manifest_gen::placeholder_package(&name));
    let label = android
        .and_then(|a| a.get("label"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| name.clone());
    let version_code = android_integer(android, "version_code", 1)?;
    let min_sdk = android_integer(android, "min_sdk", DEFAULT_MIN_SDK)?;
    let target_sdk = android_integer(android, "target_sdk", DEFAULT_TARGET_SDK)?;
    anyhow::ensure!(min_sdk >= DEFAULT_MIN_SDK,
        "android min_sdk must be at least {DEFAULT_MIN_SDK}; the native runtime requires Android 8.0+");
    anyhow::ensure!(target_sdk >= min_sdk,
        "android target_sdk must be greater than or equal to min_sdk");
    // Resolved against the project directory, not the cwd: `zkite build` is always run
    // from inside a project today, but the path in pyproject.toml belongs to the
    // project either way, and reading it relative to wherever the shell happens to be
    // is the kind of thing that works until someone adds a `--project` flag.
    // `Path::join` with an absolute path replaces, so an absolute icon= also works.
    let icon = match android.and_then(|a| a.get("icon")) {
        None => None,
        Some(value) => {
            let declared = value.as_str().filter(|s| !s.trim().is_empty())
                .context("android icon must be a nonempty PNG file path")?;
            Some(IconSource { declared: declared.to_string(), path: project_dir.join(declared) })
        },
    };
    // `[tool.zkite.android] permissions = ["INTERNET"]` (phase_3/PLAN.md §34.4). A
    // generated manifest requested none at all and had no way to, so an app that
    // wanted the network failed on the device inside `socket` with nothing naming the
    // manifest. Validated here rather than at pack time: a bad name is the developer's
    // own typo and belongs in a build error that quotes it, not in an AXML failure.
    let permissions = match android.and_then(|a| a.get("permissions")) {
        None => Vec::new(),
        Some(v) => {
            let arr = v.as_array().context(
                "android permissions must be an array of strings, \
                 e.g. permissions = [\"INTERNET\"]",
            )?;
            let mut out = Vec::new();
            for item in arr.iter() {
                let raw = item.as_str().context(
                    "android permissions must be an array of strings, \
                     e.g. permissions = [\"INTERNET\"]",
                )?;
                manifest_gen::validate_permission(raw).map_err(anyhow::Error::msg)?;
                let full = manifest_gen::normalize_permission(raw);
                if !out.contains(&full) {
                    out.push(full);
                }
            }
            out
        }
    };

    Ok(ProjectConfig {
        name,
        identity: AppIdentity {
            package,
            label,
            version_code,
            version_name: version,
            min_sdk,
            target_sdk,
            // Overwritten by `run()` once the backend is resolved — the project
            // config alone can't know it, since `--backend` outranks the file.
            activity: NATIVESHELL_ACTIVITY.to_string(),
            permissions,
        },
        dependencies,
        backend,
        icon,
    })
}

fn android_integer(table: Option<&toml::Value>, key: &str, default: u32) -> Result<u32> {
    match table.and_then(|table| table.get(key)) {
        None => Ok(default),
        Some(value) => value.as_integer().and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .with_context(|| format!("android {key} must be a positive 32-bit integer")),
    }
}

/// Reuse the same validated identity and icon for a project-specific reload shell.
pub(crate) fn preview_identity(project: &Path, backend: Backend) -> Result<(AppIdentity, Vec<u8>)> {
    use sha2::{Digest, Sha256};
    let config = load_project_config(project)?;
    manifest_gen::validate_package(&config.identity.package).map_err(anyhow::Error::msg)?;
    let icon = load_icon(config.icon.as_ref(), crate::payload::root()?)?;
    let mut identity = config.identity;
    // Different checkouts may have different development keys. Keep their previews
    // independent, and never overwrite the standalone application's package/data.
    let root = project.canonicalize()?;
    let digest = Sha256::digest(format!("{}\0{}", root.display(), identity.package).as_bytes());
    identity.package = format!("dev.zkite.preview.p{digest:x}.{}", backend.name());
    identity.label.push_str(" (Dev)");
    identity.version_code = 1;
    identity.activity = match backend {
        Backend::Native => NATIVESHELL_ACTIVITY,
        Backend::WebView => "dev.zkite.devshell.ZKiteActivity",
    }.into();
    if !identity.permissions.iter().any(|p| p == "android.permission.INTERNET") {
        identity.permissions.push("android.permission.INTERNET".into());
    }
    Ok((identity, icon))
}

pub(crate) fn runtime_notices(root: &Path) -> Result<Vec<zip_extras::RawEntry>> {
    ["LICENSE", "THIRD_PARTY_NOTICES.txt"].iter().map(|name| {
        Ok(zip_extras::RawEntry {
            path: format!("assets/zkite-{name}"),
            data: std::fs::read(root.join(name)).with_context(|| format!(
                "missing runtime notice {name}; install the matching zkite-payload release"))?,
        })
    }).collect()
}

fn wheels_package_name(spec: &str) -> String {
    spec.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.')
        .next()
        .unwrap_or(spec)
        .to_string()
}

// -------------------------------------------------------------------- ABI handling

/// An ABI is available for a backend only if *every* library that backend's shell
/// needs is present for it — not just the cdylib. Checking one file was fine while
/// there was one shell with two libs; the native shell adds `libc++_shared.so`, and a
/// half-staged artifacts directory (built before that library was staged, say) would
/// otherwise pass this check and produce an APK that dies at `System.loadLibrary`.
fn resolve_abis(requested: &[String], shell: &Shell) -> Result<Vec<String>> {
    let available: Vec<String> = ["arm64-v8a", "x86_64"]
        .iter()
        .filter(|abi| {
            let dir = shell.artifacts().join(abi);
            shell.libs.iter().all(|lib| dir.join(lib).exists())
        })
        .map(|s| s.to_string())
        .collect();
    if available.is_empty() {
        bail!(
            "no complete prebuilt shell artifacts found under {} — run {} first. \
             Expected, per ABI: {}",
            shell.artifacts().display(),
            shell.build_script,
            shell.libs.join(", ")
        );
    }
    if requested.is_empty() {
        return Ok(available);
    }
    for abi in requested {
        if !available.contains(abi) {
            bail!(
                "no complete prebuilt shell artifacts for ABI {abi:?} (have: {}) — run {}",
                available.join(", "),
                shell.build_script
            );
        }
    }
    Ok(requested.to_vec())
}

fn abi_to_arch(abi: &str) -> Result<&'static str> {
    match abi {
        "arm64-v8a" => Ok("aarch64"),
        "x86_64" => Ok("x86_64"),
        other => bail!("unsupported ABI {other:?}"),
    }
}

// --------------------------------------------------------------------- app staging

/// Bitmap formats `zkite_imagestore` can actually decode (phase_3/PLAN.md §35).
///
/// **An allowlist, not a denylist**, and deliberately short. `copy_app_source` walks
/// the *whole* project directory, which on a real machine contains a `.venv`, a
/// `.git`, editor state and whatever else — so "copy everything that isn't Python"
/// would put megabytes of junk in every APK and would do it silently. Adding a format
/// here means teaching the decoder about it first.
const ASSET_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg"];

/// Stages the app's own source and assets into the directory that becomes
/// `assets/app.zip`.
///
/// # The images half, and a correction to what phase_3/PLAN.md §18.6 said about it
///
/// §18.6 recorded `Image`'s staging problem as "`build_cmd.rs` zips the app directory
/// into `assets/app.zip`, so a `logo.png` next to `main.py` lands **inside a zip**
/// with no path to hand a decoder". Both halves of that turned out to be wrong when
/// §35 went looking for the code:
///
/// - The file never reached the zip at all. This function copied `*.py` and nothing
///   else, so `logo.png` was **dropped at staging** — the zip was never the problem.
/// - And a file that does reach the zip is not stuck there: `ZKiteActivity` unpacks
///   `app.zip` into `appDir` on first launch (`unpackOptionalZipAssetOnce`), on both
///   the native and the WebView shells, so a staged asset lands on the filesystem as
///   an ordinary file with an ordinary path.
///
/// So the fix is this allowlist, not a decoder that can read out of a zip. The
/// original note is corrected in place in §18.6 rather than left to mislead.
///
/// Top-level Python/images and the complete assets/ directory are staged.
fn copy_app_source(project_dir: &Path, app_dir: &Path) -> Result<()> {
    let assets = project_dir.join("assets");
    if assets.is_dir() {
        for entry in walkdir::WalkDir::new(&assets).follow_links(false) {
            let entry = entry?;
            anyhow::ensure!(!entry.file_type().is_symlink(), "assets must not contain symbolic links: {}", entry.path().display());
            let destination = app_dir.join(entry.path().strip_prefix(project_dir)?);
            if entry.file_type().is_dir() { std::fs::create_dir_all(destination)?; }
            else if entry.file_type().is_file() { std::fs::copy(entry.path(), destination)?; }
        }
    }
    for entry in std::fs::read_dir(project_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let is_asset = Path::new(name_str.as_ref())
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ASSET_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        if name_str.ends_with(".py") || is_asset {
            // Only regular files: a *directory* called `img.png` is not a thing
            // anyone means, and `fs::copy` on one fails with an unhelpful OS error.
            if entry.file_type()?.is_file() {
                std::fs::copy(entry.path(), app_dir.join(&name))?;
            }
        }
    }
    if !app_dir.join("main.py").exists() {
        bail!("{} has no main.py", project_dir.display());
    }
    Ok(())
}

/// Resolve complete dependency graphs for every requested Android ABI. Shared
/// assets (including AABs) require identical pure-Python wheels across targets.
fn resolve_and_stage_dependencies(
    config: &ProjectConfig,
    app_dir: &Path,
    work_dir: &Path,
    abis: &[String],
    python: &Path,
) -> Result<()> {
    if config.dependencies.is_empty() {
        return Ok(());
    }
    let mut selected: Option<Vec<PathBuf>> = None;
    for abi in abis {
        let target = AndroidTarget {
            api_level: config.identity.min_sdk,
            abi: abi.replace('-', "_"),
            py_version: PY_VERSION.replace('.', ""),
        };
        let wheels = wheels::resolve_all(python, &target, &config.dependencies, &work_dir.join(format!("wheels-{abi}")))?;
        if let Some(previous) = &selected {
            ensure_same_wheels(previous, &wheels)
                .with_context(|| format!("dependency graph differs for {abi}; shared Android assets require identical pure-Python wheels"))?;
        } else {
            selected = Some(wheels);
        }
    }
    let selected = selected.context("no Android ABI selected for dependency resolution")?;
    for warning in wheels::stale_resolution_warnings(python, &config.dependencies, &selected) {
        eprintln!("  ! {warning}");
    }
    for wheel in selected {
        unpack_wheel(&wheel, app_dir)?;
    }
    Ok(())
}

/// Share the build resolver with the live development loop. Cache only complete
/// installations; failed resolution never replaces a working dependency set.
pub(crate) fn dev_dependencies(project_dir: &Path, abi: &str) -> Result<Option<PathBuf>> {
    use sha2::{Digest, Sha256};
    let doc = project_document(project_dir)?;
    if doc.get("project").and_then(|p| p.get("dependencies")).is_none() {
        return Ok(None);
    }
    let config = load_project_config(project_dir)?;
    if config.dependencies.is_empty() { return Ok(None); }
    let identity = serde_json::to_vec(&("dev-deps-v2-android-markers", PY_VERSION, abi, config.identity.min_sdk, &config.dependencies))?;
    let digest = format!("{:x}", Sha256::digest(identity));
    let cache = project_dir.join(".zkite/dev-dependencies");
    let installed = cache.join(digest);
    if installed.join("complete").is_file() { return Ok(Some(installed.join("packages"))); }
    std::fs::create_dir_all(&cache)?;
    let stage = tempfile::tempdir_in(&cache)?;
    let packages = stage.path().join("packages");
    std::fs::create_dir(&packages)?;
    let python = wheels::find_host_python().context("Python is required to resolve development dependencies")?;
    println!("resolving development dependencies for {abi}…");
    resolve_and_stage_dependencies(&config, &packages, stage.path(), &[abi.to_string()], &python)?;
    std::fs::write(stage.path().join("complete"), b"1")?;
    // Another dev process may have populated the same content-addressed entry.
    if !installed.join("complete").is_file() {
        std::fs::rename(stage.path(), &installed).context("saving development dependencies")?;
    }
    Ok(Some(installed.join("packages")))
}

fn ensure_same_wheels(left: &[PathBuf], right: &[PathBuf]) -> Result<()> {
    use sha2::{Digest, Sha256};
    fn fingerprint(paths: &[PathBuf]) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
        paths.iter().map(|path| {
            let name = path.file_name().context("wheel has no filename")?.to_string_lossy().into_owned();
            Ok((name, Sha256::digest(std::fs::read(path)?).to_vec()))
        }).collect()
    }
    anyhow::ensure!(fingerprint(left)? == fingerprint(right)?, "resolved wheel names or contents differ");
    Ok(())
}

fn unpack_wheel(wheel_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(wheel_path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        // ZIP names always use '/', even when the build host uses '\\'. Reject
        // both Windows drive/UNC paths and Unix traversal on every host, so a wheel
        // cannot write outside the app staging directory.
        if name.starts_with('/')
            || name.contains(['\\', ':', '\0'])
            || name.split('/').any(|part| part == ".." || part == ".")
        {
            bail!("{} contains unsafe wheel entry {name:?}", wheel_path.display());
        }
        let relative = entry.enclosed_name().context("wheel entry escapes its staging directory")?;
        let parts: Vec<&str> = name.split('/').collect();
        let out_path = if parts[0].ends_with(".data") {
            match parts.get(1).copied() {
                Some("purelib" | "platlib") => dest.join(parts[2..].join("/")),
                // Host executable scripts and C headers do not belong in an app.
                Some("scripts" | "headers") => continue,
                Some("data") => bail!("{} contains unsupported install-prefix data {name:?}; move runtime assets into the Python package", wheel_path.display()),
                _ if name.ends_with('/') => continue,
                _ => bail!("{} contains an unknown wheel data scheme: {name:?}", wheel_path.display()),
            }
        } else {
            // Keep dist-info: importlib.metadata.version() and entry points need
            // these files at runtime even though they are not importable modules.
            dest.join(relative)
        };
        if name.ends_with('/') {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&out_path)
            .with_context(|| format!("wheel {} would overwrite an existing app/dependency file {}", wheel_path.display(), out_path.display()))?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

fn pyc_compile(python: &Path, repo_root: &Path, app_dir: &Path) -> Result<()> {
    let script = pybuild_script(repo_root, "compile_pyc.py");
    let status = std::process::Command::new(python).arg(&script).arg(app_dir).status()?;
    if !status.success() {
        bail!("compile_pyc.py failed ({status})");
    }
    Ok(())
}

fn prune_stdlib(
    python: &Path,
    prefix: &Path,
    app_dir: &Path,
    repo_root: &Path,
    out_zip: &Path,
) -> Result<()> {
    let script = pybuild_script(repo_root, "prune_stdlib.py");
    let zkite_py_src = repo_root.join("phase_1/zkite_py/src");
    let status = std::process::Command::new(python)
        .arg(&script)
        .arg(prefix)
        .arg(out_zip)
        .arg(&zkite_py_src)
        .arg(app_dir)
        .status()?;
    if !status.success() {
        bail!("prune_stdlib.py failed ({status})");
    }
    Ok(())
}

/// `phase_2/pybuild/scripts/<name>` under the payload root.
///
/// This was §23.2's second independent anchor — it repeated the `concat!` rather than
/// going through `repo_root()`, which is exactly how four of them accumulated. It now
/// takes the root as an argument so a sixth cannot appear here by inattention.
fn pybuild_script(repo_root: &Path, name: &str) -> PathBuf {
    repo_root.join("phase_2/pybuild/scripts").join(name)
}

fn copy_stdlib_unpruned(prefix: &Path, out_zip: &Path) -> Result<()> {
    // --no-prune-stdlib escape hatch: ship the full stdlib source tree verbatim, no
    // .py compilation either (this path is for debugging a pruning-related boot
    // failure, not for release builds).
    let stdlib_dir = prefix.join(format!("lib/python{PY_VERSION}"));
    let file = std::fs::File::create(out_zip)?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for entry in walkdir::WalkDir::new(&stdlib_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_name() == "lib-dynload" || entry.file_name() == "__pycache__" {
            continue;
        }
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(&prefix)?.to_string_lossy().replace('\\', "/");
            zip.start_file(rel, options)?;
            let mut f = std::fs::File::open(entry.path())?;
            std::io::copy(&mut f, &mut zip)?;
        }
    }
    zip.finish()?;
    Ok(())
}

fn zip_dir(src: &Path, out_zip: &Path) -> Result<()> {
    let file = std::fs::File::create(out_zip)?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for entry in walkdir::WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(src)?.to_string_lossy().replace('\\', "/");
            zip.start_file(rel, options)?;
            let mut f = std::fs::File::open(entry.path())?;
            std::io::copy(&mut f, &mut zip)?;
        }
    }
    zip.finish()?;
    Ok(())
}

fn zip_zkite_framework(repo_root: &Path, out_zip: &Path) -> Result<()> {
    let src = repo_root.join("phase_1/zkite_py/src/zkite");
    let file = std::fs::File::create(out_zip)?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for entry in walkdir::WalkDir::new(&src).into_iter().filter_map(|e| e.ok()) {
        if entry.path().components().any(|c| c.as_os_str() == "__pycache__") {
            continue;
        }
        if entry.file_type().is_file() {
            let rel = format!("zkite/{}", entry.path().strip_prefix(&src)?.to_string_lossy().replace('\\', "/"));
            zip.start_file(rel, options)?;
            let mut f = std::fs::File::open(entry.path())?;
            std::io::copy(&mut f, &mut zip)?;
        }
    }
    zip.start_file("zkite/_build_mode.py", options)?;
    std::io::Write::write_all(&mut zip, b"DEBUG = False\n")?;
    zip.finish()?;
    Ok(())
}

/// The one asset difference between the backends. Each shell reads exactly one of
/// these and would not know what to do with the other, so shipping both would be
/// ~2 MB of dead weight in every APK rather than a harmless belt-and-braces.
fn stage_backend_assets(
    backend: Backend,
    repo_root: &Path,
    shell: &Shell,
    assets: &Path,
) -> Result<()> {
    match backend {
        Backend::WebView => stage_webruntime(repo_root, assets),
        // The native shell unpacks `fonts.zip` into a directory on first launch and
        // hands that directory to `Native.boot` (phase_3/PLAN.md §9.7). Staged
        // verbatim from the shell's own artifacts rather than zipped here from
        // phase_1/webruntime/fonts/, because that directory is gitignored and
        // materialised out of the *host's* /usr/share/fonts — which a user's machine
        // has no reason to have, and `zkite build` may not require.
        Backend::Native => {
            let src = shell.artifacts().join("fonts.zip");
            std::fs::copy(&src, assets.join("fonts.zip")).with_context(|| {
                format!(
                    "reading prebuilt {} — run {} first",
                    src.display(),
                    shell.build_script
                )
            })?;
            Ok(())
        }
    }
}

fn stage_webruntime(repo_root: &Path, assets: &Path) -> Result<()> {
    let src = repo_root.join("phase_1/webruntime");
    let dest = assets.join("webruntime");
    copy_dir(&src, &dest)?;
    std::fs::remove_dir_all(dest.join("test")).ok();
    for entry in std::fs::read_dir(dest.join("vendor"))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".LICENSE") || name.ends_with(".VERSION") {
            std::fs::remove_file(entry.path()).ok();
        }
    }
    // `index.html` loads the six bundled faces by relative path (`fonts/sans.ttf`),
    // and `phase_1/webruntime/fonts/` is gitignored — materialised by
    // `stage_fonts.sh`, absent on a fresh checkout. `copy_dir` therefore ships them
    // when they happen to exist and silently omits them when they don't, and the
    // consequence of omitting them is invisible: the page falls back to the device's
    // own sans-serif (Roboto), which renders fine and diverges from both the native
    // backend and every conformance number (phase_3/PLAN.md §9.7). Say which
    // happened rather than leaving it to whoever next wonders why a device screenshot
    // doesn't match the harness.
    let faces = std::fs::read_dir(dest.join("fonts"))
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "ttf")).count())
        .unwrap_or(0);
    if faces == 0 {
        eprintln!(
            "  ! no bundled fonts staged (phase_1/webruntime/fonts/ is empty or missing) — \
             this app will fall back to the device's own sans-serif, which will not match \
             the native backend or the conformance harness. Run \
             phase_1/webruntime/scripts/stage_fonts.sh to fix."
        );
    } else {
        println!("    staged {faces} bundled font faces into assets/webruntime/fonts/");
    }
    Ok(())
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ native staging

/// Stages the ABI-independent parts of the payload: `classes.dex` (prebuilt, §1.1)
/// and everything already zipped into `assets/`.
///
/// `dex_rel_path` differs by format: an APK wants `classes.dex` at its root, but an
/// AAB's `base` module requires it under `dex/` — `bundletool validate` rejects
/// `classes.dex` at the module root outright ("Module files can be only in
/// pre-defined directories"). `assets/**` and `lib/<abi>/**`, by contrast, land at
/// the same relative path in both formats (`aab.rs` prefixes every extra with
/// `base/` uniformly), which is why only this one path needs to be a parameter.
fn stage_common(
    assets_src: &Path,
    stage: &Path,
    dex_rel_path: &str,
    shell: &Shell,
) -> Result<()> {
    std::fs::create_dir_all(stage.join("assets"))?;
    copy_dir(assets_src, &stage.join("assets"))?;
    let dex_dest = stage.join(dex_rel_path);
    if let Some(parent) = dex_dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Per-shell, not shared: the native shell's dex additionally carries
    // ZKiteSurfaceView, TextSelectionOverlay, vendor/gametextinput's classes and the
    // androidx-core subset they need (phase_3/PLAN.md §7.1/§7.3), so it is ~1 MB
    // against appshell's ~11 KB and the two are not interchangeable.
    let dex_src = shell.artifacts().join("classes.dex");
    std::fs::copy(&dex_src, &dex_dest).with_context(|| {
        format!(
            "reading prebuilt {} — run {}/build_dex.sh first",
            dex_src.display(),
            shell.build_script.rsplit_once('/').map(|(d, _)| d).unwrap_or("scripts")
        )
    })?;
    Ok(())
}

/// Stages `lib/<abi>/*.so`: the prebuilt appshell cdylib + `libpython3.14.so` (both
/// from `phase_2/appshell/scripts/build.sh`'s output) plus every extension module
/// from the vendored Android prefix's `lib-dynload/` — same `.pth`-file-at-runtime
/// pattern phase 0/1 established for Android 10+'s own-lib-dir `dlopen` restriction.
///
/// Extension modules are **renamed** on the way in, `<mod>.cpython-314-<triple>.so`
/// -> `libpymod_<mod>.so`. This is load-bearing, not cosmetic: Android's package
/// installer extracts an APK's `lib/<abi>/` entries into the app's nativeLibraryDir
/// but only those whose basename matches `lib*.so`, silently skipping every other
/// name — no build-time warning, no install-time warning, the file simply is not
/// there at runtime. Measured on a real device against a build of this shape: 3 of
/// 70 staged `.so` files survived installation, which is why `import math` had never
/// worked on any ZKite backend since phase 1. `runtime.py`'s
/// `_ZKiteNativeExtensionFinder` maps the renamed files back to module names at
/// import time. Full account: `phase_3/PLAN.md` §11.6.
fn stage_native_libs(shell: &Shell, phase0: &Path, abi: &str, stage: &Path) -> Result<()> {
    let arch = abi_to_arch(abi)?;
    let lib_dest = stage.join("lib").join(abi);
    std::fs::create_dir_all(&lib_dest)?;

    let artifacts = shell.artifacts().join(abi);
    for name in shell.libs {
        std::fs::copy(artifacts.join(name), lib_dest.join(name))
            .with_context(|| format!("reading prebuilt {name} for {abi}"))?;
    }

    // Shared libraries some extension modules carry a DT_NEEDED on (`_ssl` ->
    // libssl_python.so + libcrypto_python.so, `_hashlib` -> libcrypto_python.so,
    // `_sqlite3` -> libsqlite3_python.so). python.org's Android prefix already
    // names these `lib*_python.so` precisely so the installer will extract them.
    // They were never staged at all, so those three modules would still fail to
    // load — with a dynamic-linker error, not a ModuleNotFoundError — even after
    // the rename below makes the module files themselves present.
    let prefix_lib = phase0.join(format!("vendor/python-{arch}/prefix/lib"));
    if prefix_lib.is_dir() {
        for entry in std::fs::read_dir(&prefix_lib)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("lib") && name.ends_with("_python.so") {
                std::fs::copy(entry.path(), lib_dest.join(entry.file_name()))?;
            }
        }
    }

    let dynload = phase0.join(format!("vendor/python-{arch}/prefix/lib/python{PY_VERSION}/lib-dynload"));
    if dynload.is_dir() {
        for entry in std::fs::read_dir(&dynload)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e == "so") {
                let file_name = entry.file_name();
                let file_name = file_name.to_string_lossy();
                // `math.cpython-314-aarch64-linux-android.so` -> `math`. Module
                // names are `[A-Za-z0-9_]+`, so splitting on the first dot is
                // exact and the staged name round-trips unambiguously.
                let module = file_name.split('.').next().unwrap_or_default();
                if module.is_empty() {
                    continue;
                }
                std::fs::copy(entry.path(), lib_dest.join(format!("libpymod_{module}.so")))?;
            }
        }
    }
    Ok(())
}

// --------------------------------------------------------------------- launcher icon

/// Below this, the launcher upscales and the icon looks visibly soft. Android's
/// xxxhdpi launcher bucket asks for 192x192 (48dp at 4x); phones at 400–560dpi are
/// ordinary. Warn rather than fail — a small icon is ugly, not broken, and refusing
/// a build over it would be the wrong trade.
const MIN_ICON_PX: u32 = 192;

/// The launcher icon that goes into `res/drawable/icon.png`, which the generated
/// manifest references as `android:icon="@drawable/icon"`.
///
/// `[tool.zkite.android] icon` was documented in phase_2/PLAN.md §3 from phase 2 and
/// **implemented in phase 3 §22** — for three phases the key was read nowhere and
/// every ZKite app shipped the same bundled square. See phase_3/PLAN.md §21.1 for how
/// a doc-only feature survived that long.
///
/// Input is a PNG; packaging wraps it in a full-bleed adaptive icon.
fn load_icon(icon: Option<&IconSource>, repo_root: &Path) -> Result<Vec<u8>> {
    let Some(icon) = icon else {
        return Ok(std::fs::read(repo_root.join("phase_2/appshell/app/res/drawable/icon.png"))?);
    };

    let bytes = std::fs::read(&icon.path).with_context(|| {
        format!(
            "reading android icon = {:?}, resolved to {}",
            icon.declared,
            icon.path.display()
        )
    })?;

    // `pack` stores resource bytes verbatim — `FileResource` carries `contents` and
    // the compiler never decodes them (its only note on the subject is a TODO about
    // PNG-crushing). So nothing between here and the device will notice a file that
    // isn't a PNG: it packages, signs, aligns, installs, and then the launcher shows
    // a blank icon with the failure buried in system logs. Check it here, where the
    // path is still in hand.
    let Some((w, h)) = png_dimensions(&bytes) else {
        bail!(
            "android icon = {:?} is not a PNG ({}{}).\n  \
             Android resource packing stores the bytes verbatim — nothing transcodes \
             — so a mislabelled file builds and signs cleanly and then fails to decode \
             on the device. Convert it to PNG.",
            icon.declared,
            icon.path.display(),
            match sniff_format(&bytes) {
                Some(f) => format!(" — looks like {f}"),
                None => String::new(),
            }
        );
    };

    println!("  icon: {} ({w}x{h})", icon.declared);
    if w.min(h) < MIN_ICON_PX {
        eprintln!(
            "  ! icon is {w}x{h} — launchers upscale below {MIN_ICON_PX}x{MIN_ICON_PX} \
             and it will look soft on a high-density screen."
        );
    }
    if w != h {
        eprintln!("  ! icon is {w}x{h}, not square — launchers will letterbox or crop it.");
    }
    Ok(bytes)
}

/// `(width, height)` from a PNG's IHDR, or `None` if this isn't a PNG at all.
///
/// Hand-rolled rather than pulling in an image crate: the layout is fixed by the spec
/// — an 8-byte signature, then IHDR *first*, always — so this is a magic-number check
/// and two big-endian reads, and it is the whole of what a launcher icon needs
/// validated at build time.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

/// Names what the file actually is, when we can tell, so the error says "looks like a
/// JPEG" instead of only "not a PNG". Renaming a `.jpg` is the failure this exists for.
fn sniff_format(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("a JPEG"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("a WebP"),
        [b'G', b'I', b'F', b'8', ..] => Some("a GIF"),
        [b'B', b'M', ..] => Some("a BMP"),
        [0x00, 0x00, 0x01, 0x00, ..] => Some("a Windows .ico"),
        _ if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") => Some("SVG/XML"),
        _ => None,
    }
}

fn load_or_generate_keys(keystore: Option<&Path>, project: &Path) -> Result<Keys> {
    let default = project.join("keystore/release.pem");
    let legacy = project.join("zkite-release.pem");
    let path = if let Some(explicit) = keystore { explicit.to_path_buf() }
        else if default.is_file() { default }
        else if legacy.is_file() { legacy }
        else {
            crate::keygen_cmd::run(&project.join("keystore/release"), "zkite", "android")?;
            default
        };
    keystore::load_pem(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
        .context("loading signing key; use keystore/release.pem or --keystore PATH")
}

// There is deliberately no `repo_root()` here any more. It was one of §23.2's four
// compile-time anchors, and leaving a thin wrapper behind would leave a second name
// for the same thing — which is how the four accumulated. Everything goes through
// `crate::payload`, and `run()` binds its result to a local `repo_root` because that
// is what the ~19 call sites below mean by the word (the payload layout *is* the
// checkout's, by design — see `payload.rs`).

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// A real, minimal PNG of the given size — signature + IHDR + IDAT + IEND, with a
    /// deliberately bogus CRC, since nothing in this path checks one and writing a real
    /// one would only test the test.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 2, 0, 0, 0]);
        v.extend_from_slice(&[0, 0, 0, 0]); // crc
        v
    }

    #[test]
    fn default_signing_keys_are_private_persistent_and_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        load_or_generate_keys(None, dir.path()).unwrap();
        let pem = dir.path().join("keystore/release.pem");
        let original = std::fs::read(&pem).unwrap();
        assert!(dir.path().join("keystore/release.p12").is_file());
        assert_eq!(std::fs::read_to_string(dir.path().join("keystore/.gitignore")).unwrap(), "*\n");
        load_or_generate_keys(None, dir.path()).unwrap();
        assert_eq!(original, std::fs::read(&pem).unwrap());
        assert!(crate::keygen_cmd::run(&dir.path().join("keystore/release"), "test", "test").is_err());
    }

    #[test]
    #[ignore = "downloads real PyPI wheels; run explicitly when checking device dependencies"]
    fn development_resolves_and_caches_requests_with_transitive_packages() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("app.json"),
            r#"{"name":"dependency-check","dependencies":["requests==2.32.5"]}"#).unwrap();
        let packages = dev_dependencies(project.path(), "arm64-v8a").unwrap().unwrap();
        assert!(packages.join("requests/__init__.py").is_file());
        assert!(packages.join("urllib3/__init__.py").is_file());
        assert!(packages.join("certifi/cacert.pem").is_file());
        assert!(packages.join("idna/__init__.py").is_file());
        assert_eq!(Some(packages.clone()), dev_dependencies(project.path(), "arm64-v8a").unwrap());
        let python = wheels::find_host_python().unwrap();
        let status = std::process::Command::new(python).args(["-I", "-S", "-c",
            "import sys; sys.path.insert(0, sys.argv[1]); import requests, importlib.metadata; assert requests.__version__ == '2.32.5'; assert importlib.metadata.version('requests') == '2.32.5'"])
            .arg(&packages).status().unwrap();
        assert!(status.success());
        std::fs::write(project.path().join("app.json"), r#"{"name":"dependency-check","dependencies":[]}"#).unwrap();
        assert!(dev_dependencies(project.path(), "arm64-v8a").unwrap().is_none());
    }

    #[test]
    fn app_json_works_without_a_package_manager_and_overrides_legacy_identity() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.json"), r#"{"name":"newapp","version":"2.0","android":{"label":"New App","package":"org.example.newapp","icon":"assets/logo.png","version_code":2,"permissions":["INTERNET"]}}"#).unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "# pip dependencies\nrequests==2.32.5\n").unwrap();
        let config = load_project_config(dir.path()).unwrap();
        assert_eq!(config.identity.label, "New App");
        assert_eq!(config.identity.version_code, 2);
        assert_eq!(config.dependencies, vec!["requests==2.32.5"]);
        assert_eq!(config.icon.unwrap().path, dir.path().join("assets/logo.png"));
        std::fs::write(dir.path().join("pyproject.toml"), "[project]\nname='unrelated'\ndependencies=['idna']\n[tool.zkite.android]\nlabel='Old'\n").unwrap();
        let config = load_project_config(dir.path()).unwrap();
        assert_eq!(config.name, "newapp");
        assert_eq!(config.identity.label, "New App");
        assert_eq!(config.dependencies, vec!["idna", "requests==2.32.5"]);
    }

    #[test]
    fn malformed_app_json_does_not_fall_back_to_legacy_config() {
        let dir = tempfile::tempdir().unwrap();
        for contents in ["{", r#"{"name":"app","android":{"label":42}}"#, r#"{"name":"app","android":{"version_code":true}}"#, r#"{"name":"app","bakend":"native"}"#] {
            std::fs::write(dir.path().join("app.json"), contents).unwrap();
            assert!(load_project_config(dir.path()).is_err(), "{contents}");
        }
    }

    #[test]
    fn nested_assets_and_release_mode_are_packaged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/assets/images")).unwrap();
        std::fs::create_dir(dir.path().join("dest")).unwrap();
        std::fs::write(dir.path().join("src/main.py"), "pass").unwrap();
        std::fs::write(dir.path().join("src/assets/images/logo.png"), "image").unwrap();
        copy_app_source(&dir.path().join("src"), &dir.path().join("dest")).unwrap();
        assert_eq!(std::fs::read(dir.path().join("dest/assets/images/logo.png")).unwrap(), b"image");
        let zip_path = dir.path().join("framework.zip");
        zip_zkite_framework(crate::payload::root().unwrap(), &zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path).unwrap()).unwrap();
        let mut mode = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("zkite/_build_mode.py").unwrap(), &mut mode).unwrap();
        assert_eq!(mode, "DEBUG = False\n");
    }

    fn project_with_icon(dir: &Path, icon_line: &str) -> ProjectConfig {
        std::fs::write(
            dir.join("pyproject.toml"),
            format!(
                "[project]\nname = \"iconapp\"\nversion = \"0.1.0\"\n\n\
                 [tool.zkite.android]\npackage = \"com.example.iconapp\"\n{icon_line}\n"
            ),
        )
        .unwrap();
        load_project_config(dir).unwrap()
    }

    /// phase_3/PLAN.md §35 — the staging bug behind "an app with a logo shows a blank
    /// rectangle", which was **not** where §18.6 said it was.
    ///
    /// Every assertion here is one that failed before this session: `logo.png` was
    /// simply not copied, so the decoder written in §35 would have had nothing to open
    /// no matter how correct it was.
    ///
    /// The *negative* half carries as much weight as the positive one. This function
    /// walks a real project directory, which on any working machine contains a
    /// `.venv`, a `.git` and editor state — so a fix shaped as "copy everything that
    /// is not Python" would pass the first three assertions and quietly add megabytes
    /// to every APK.
    #[test]
    fn sdk_defaults_and_invalid_config_fail_before_building() {
        let dir = tempfile::tempdir().unwrap();
        let write = |extra: &str| std::fs::write(dir.path().join("pyproject.toml"),
            format!("[project]\nname = \"sample\"\n[tool.zkite.android]\n{extra}")).unwrap();
        write("");
        let config = load_project_config(dir.path()).unwrap();
        assert_eq!((config.identity.min_sdk, config.identity.target_sdk), (26, 36));
        write("min_sdk = 28\ntarget_sdk = 35\nversion_code = 42");
        let config = load_project_config(dir.path()).unwrap();
        assert_eq!((config.identity.min_sdk, config.identity.target_sdk, config.identity.version_code), (28, 35, 42));
        for invalid in ["min_sdk = 25", "min_sdk = 30\ntarget_sdk = 29", "version_code = -1",
                        "version_code = 4294967297", "version_code = 0", "target_sdk = \"36\""] {
            write(invalid);
            assert!(load_project_config(dir.path()).is_err(), "{invalid}");
        }
    }

    #[test]
    fn legacy_configuration_cannot_silently_change_an_apps_identity() {
        let dir = tempfile::tempdir().unwrap();
        for extra in ["", "\n[tool.zkite.android]\npackage = 'org.example.renamed'\n"] {
            std::fs::write(dir.path().join("pyproject.toml"), format!(
                "[project]\nname = 'sample'\n[tool.kite.android]\npackage = 'org.example.original'\n{extra}"
            )).unwrap();
            let error = load_project_config(dir.path()).err().unwrap().to_string();
            assert!(error.contains("legacy [tool.kite]"), "{error}");
            assert!(error.contains("[tool.zkite]"), "{error}");
        }
    }

    #[test]
    fn app_staging_takes_python_and_image_assets_and_nothing_else() {
        let src = tempdir().unwrap();
        let out = tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| std::fs::write(src.path().join(name), bytes).unwrap();

        write("main.py", b"# app");
        write("helper.py", b"# module");
        write("logo.png", &png(64, 64));
        write("photo.jpg", b"\xFF\xD8\xFFnot really a jpeg");
        write("SHOUTY.JPEG", b"\xFF\xD8\xFFuppercase extension");
        // Things that must not be staged.
        write("notes.md", b"# readme");
        write("pyproject.toml", b"[project]");
        write("data.sqlite", b"binary junk");
        write("clip.mp4", b"a 200MB video, in spirit");
        std::fs::create_dir(src.path().join(".venv")).unwrap();
        std::fs::write(src.path().join(".venv/pyvenv.cfg"), b"x").unwrap();
        // A *directory* whose name ends in an asset extension — `fs::copy` on one
        // fails with an OS error that names neither the file nor the cause.
        std::fs::create_dir(src.path().join("sprites.png")).unwrap();

        copy_app_source(src.path(), out.path()).unwrap();

        let staged = |name: &str| out.path().join(name).exists();
        for want in ["main.py", "helper.py", "logo.png", "photo.jpg", "SHOUTY.JPEG"] {
            assert!(staged(want), "{want} should be staged");
        }
        for unwanted in ["notes.md", "pyproject.toml", "data.sqlite", "clip.mp4", ".venv", "sprites.png"] {
            assert!(!staged(unwanted), "{unwanted} must not be staged");
        }
        // The bytes arrive intact, not merely a file of the right name.
        assert_eq!(std::fs::read(out.path().join("logo.png")).unwrap(), png(64, 64));
    }

    /// The pre-existing contract this function has always had, kept explicit now that
    /// it does more than one thing: a project with assets but no `main.py` is still an
    /// error, and the message names the directory.
    #[test]
    fn app_staging_still_requires_a_main_py() {
        let src = tempdir().unwrap();
        let out = tempdir().unwrap();
        std::fs::write(src.path().join("logo.png"), png(8, 8)).unwrap();
        let err = copy_app_source(src.path(), out.path()).unwrap_err().to_string();
        assert!(err.contains("has no main.py"), "unexpected error: {err}");
    }

    #[test]
    fn icon_key_is_read_and_resolved_against_the_project_dir() {
        // The regression this exists for: phase_2/PLAN.md §3 documented this key from
        // phase 2 and no code read it until phase 3 §22 (§21.1).
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/logo.png"), png(512, 512)).unwrap();

        let config = project_with_icon(dir.path(), "icon = \"assets/logo.png\"");
        let icon = config.icon.expect("icon key was not read");
        assert_eq!(icon.declared, "assets/logo.png");
        assert_eq!(icon.path, dir.path().join("assets/logo.png"));

        let bytes = load_icon(Some(&icon), Path::new("/nonexistent-repo-root")).unwrap();
        assert_eq!(png_dimensions(&bytes), Some((512, 512)));
    }

    #[test]
    fn absent_icon_key_falls_back_to_the_bundled_default() {
        let dir = tempdir().unwrap();
        let config = project_with_icon(dir.path(), "");
        assert!(config.icon.is_none());
        // Reads the real bundled resource out of this checkout, so this also pins that
        // the default path still exists.
        let bytes = load_icon(None, crate::payload::root().unwrap()).unwrap();
        assert!(png_dimensions(&bytes).is_some(), "the bundled default is not a PNG");
    }

    #[test]
    fn invalid_icon_configuration_never_silently_uses_the_default() {
        let dir = tempdir().unwrap();
        for value in ["true", "12", "\"\""] {
            std::fs::write(dir.path().join("pyproject.toml"), format!(
                "[project]\nname='app'\n[tool.zkite.android]\nicon={value}\n"
            )).unwrap();
            let error = load_project_config(dir.path()).err().unwrap();
            assert!(error.to_string().contains("icon must be a nonempty PNG file path"));
        }
    }

    #[test]
    fn preview_uses_project_icon_and_label_with_separate_stable_identity() {
        let dir = tempdir().unwrap();
        project_with_icon(dir.path(), "icon='icon.png'\nlabel='My App'");
        std::fs::write(dir.path().join("icon.png"), png(192, 192)).unwrap();
        let (first, icon) = preview_identity(dir.path(), Backend::Native).unwrap();
        assert_eq!(first.label, "My App (Dev)");
        assert!(first.package.starts_with("dev.zkite.preview."));
        assert_eq!(first.activity, NATIVESHELL_ACTIVITY);
        assert!(first.permissions.contains(&"android.permission.INTERNET".to_string()));
        assert_eq!(icon, png(192, 192));
        std::fs::write(dir.path().join("icon.png"), png(512, 512)).unwrap();
        let (second, icon) = preview_identity(dir.path(), Backend::Native).unwrap();
        assert_eq!(first.package, second.package);
        assert_eq!(icon, png(512, 512));
        assert_ne!(first.package, preview_identity(dir.path(), Backend::WebView).unwrap().0.package);
    }

    #[test]
    fn a_missing_icon_errors_with_both_what_was_written_and_where_it_looked() {
        let dir = tempdir().unwrap();
        let config = project_with_icon(dir.path(), "icon = \"assets/logo.png\"");
        let err = load_icon(config.icon.as_ref(), crate::payload::root().unwrap()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("assets/logo.png"), "{msg}");
        assert!(msg.contains(&dir.path().display().to_string()), "{msg}");
    }

    #[test]
    fn a_jpeg_named_png_is_rejected_at_build_and_named_as_a_jpeg() {
        // `pack` stores resource bytes verbatim, so without this check the build is
        // clean and the *device* is where it fails — see `load_icon`'s doc.
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("logo.png"), b"\xFF\xD8\xFF\xE0 not really a png").unwrap();
        let config = project_with_icon(dir.path(), "icon = \"logo.png\"");
        let err = load_icon(config.icon.as_ref(), crate::payload::root().unwrap()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("is not a PNG"), "{msg}");
        assert!(msg.contains("looks like a JPEG"), "{msg}");
    }

    #[test]
    fn png_dimensions_reads_ihdr_and_rejects_non_pngs() {
        assert_eq!(png_dimensions(&png(192, 192)), Some((192, 192)));
        assert_eq!(png_dimensions(&png(1024, 768)), Some((1024, 768)));
        assert_eq!(png_dimensions(b"\x89PNG\r\n\x1a\n"), None); // signature but truncated
        assert_eq!(png_dimensions(b"GIF89a and then some padding bytes"), None);
        assert_eq!(png_dimensions(&[]), None);
    }

    fn test_wheel(path: &Path, entries: &[(&str, &str)]) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn wheel_extraction_rejects_paths_that_escape_on_unix_or_windows() {
        for unsafe_path in ["../escaped.py", "/escaped.py", "C:/escaped.py", "C:escaped.py", "..\\escaped.py", "\\\\host\\share\\escaped.py"] {
            let dir = tempdir().unwrap();
            let wheel = dir.path().join("bad.whl");
            test_wheel(&wheel, &[(unsafe_path, "bad")]);
            let error = unpack_wheel(&wheel, &dir.path().join("app")).unwrap_err();
            assert!(error.to_string().contains("unsafe wheel entry"), "{error}");
            assert!(!dir.path().join("escaped.py").exists());
        }
    }

    #[test]
    fn wheel_extraction_preserves_metadata_and_purelib_and_refuses_overwrites() {
        let dir = tempdir().unwrap();
        let wheel = dir.path().join("test.whl");
        test_wheel(&wheel, &[
            ("example-1.0.data/purelib/example.py", "value = 42"),
            ("example-1.0.dist-info/METADATA", "Name: example\nVersion: 1.0"),
        ]);
        let app = dir.path().join("app");
        unpack_wheel(&wheel, &app).unwrap();
        assert_eq!(std::fs::read_to_string(app.join("example.py")).unwrap(), "value = 42");
        assert!(app.join("example-1.0.dist-info/METADATA").is_file());
        assert!(unpack_wheel(&wheel, &app).unwrap_err().to_string().contains("overwrite"));
    }

    #[test]
    fn shared_assets_require_the_same_wheels_for_every_abi() {
        let left = tempdir().unwrap();
        let right = tempdir().unwrap();
        let l = left.path().join("dep.whl");
        let r = right.path().join("dep.whl");
        std::fs::write(&l, "same").unwrap();
        std::fs::write(&r, "same").unwrap();
        ensure_same_wheels(&[l.clone()], &[r.clone()]).unwrap();
        std::fs::write(&r, "different").unwrap();
        assert!(ensure_same_wheels(&[l], &[r]).is_err());
    }

    #[test]
    fn stored_and_deflated_wheels_both_extract() {
        use std::io::Write;
        for compression in [zip::CompressionMethod::Stored, zip::CompressionMethod::Deflated] {
            let dir = tempdir().unwrap();
            let wheel = dir.path().join("example.whl");
            let mut archive = zip::ZipWriter::new(std::fs::File::create(&wheel).unwrap());
            archive.start_file("example.py", zip::write::SimpleFileOptions::default().compression_method(compression)).unwrap();
            archive.write_all(b"value = 42\n").unwrap();
            archive.finish().unwrap();
            let destination = dir.path().join("app");
            unpack_wheel(&wheel, &destination).unwrap();
            assert_eq!(std::fs::read(destination.join("example.py")).unwrap(), b"value = 42\n");
        }
    }
}
