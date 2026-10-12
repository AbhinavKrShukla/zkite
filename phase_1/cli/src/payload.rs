//! Where the build payload lives — the one resolver every non-project path goes
//! through (`../../phase_3/PLAN.md` §23.2, §23.7 step 1).
//!
//! **The problem this exists for.** `zkite build` reads a lot that is not the user's
//! project: both shells' prebuilt artifacts, the vendored Python prefixes, `zkite_py`'s
//! source, `webruntime`, the pybuild scripts, the vendored `pip.pyz`, the default
//! launcher icon. Until this module, every one of those resolved against a
//! `concat!(env!("CARGO_MANIFEST_DIR"), "/../..")` baked into the binary at compile
//! time — in **five** independent places (§23.2 found four; §24 added the fifth on
//! purpose and routed it this way as the pattern to copy). So the binary worked on
//! exactly one machine on earth, and a second *Linux* box failed identically to a Mac.
//!
//! **The layout is deliberately the checkout's own.** A payload root contains
//! `phase_0/vendor/…`, `phase_1/zkite_py/src`, `phase_3/gpu_harness/artifacts`, and so
//! on, exactly as this repository lays them out. That is not laziness — it is what
//! makes the compile-time checkout a *valid payload root by construction*, so the
//! dev-mode fallback needs no special-casing anywhere and cannot drift from the
//! shipped layout. Every existing `root.join("phase_1/…")` call site is unchanged.
//!
//! **The fight, per §23.7 step 1, is not the code.** It is that the checkout entry
//! must stay last and stay a fallback *forever*, and that every future site must come
//! through here rather than adding a sixth anchor. Hence: one function, `pub(crate)`
//! visibility on the root only, and `zkite doctor` reporting which root it resolved —
//! because the failure this design introduces is a wheel silently finding a stale
//! checkout instead of its own payload, and that is unfalsifiable without the report.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The compile-time checkout. **Last** in every search order, forever (§23.7 step 1).
const CHECKOUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The environment override, first in the search order.
pub const ENV_VAR: &str = "ZKITE_PAYLOAD_ROOT";

/// The directory name looked for beside the installed binary.
const BESIDE_BINARY_DIR: &str = "zkite-payload";

/// Files a payload must carry whatever backend or ABI it was staged for.
///
/// A *probe*, not a stamp file, and that is the point: both of these are load-bearing
/// inputs to every single `zkite build` (`zkite_py`'s source goes into
/// `zkite-framework.zip`; `prune_stdlib.py` produces `python-stdlib.zip`), so a
/// directory that has them is a payload and a directory that does not would have
/// failed later anyway — just 80 lines and one wheel-resolution later, which is the
/// §22.1 "validate beside the cheap checks" posture. A marker file someone has to
/// remember to write is the thing that goes stale (§18.7's `icons.rs`).
const MARKERS: [&str; 2] = [
    "phase_1/zkite_py/src/zkite/__init__.py",
    "phase_2/pybuild/scripts/prune_stdlib.py",
];

/// Which entry in the search order answered. Carried rather than discarded because
/// `zkite doctor` has to be able to *say* it (§23.2's closing line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `ZKITE_PAYLOAD_ROOT` was set and valid.
    Env,
    /// A `zkite-payload/` directory found relative to the running binary.
    BesideBinary,
    /// The compile-time checkout — dev mode, and the entry that must stay last.
    Checkout,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Env => "ZKITE_PAYLOAD_ROOT",
            Origin::BesideBinary => "beside the zkite binary",
            Origin::Checkout => "this checkout (dev mode — not a packaged install)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Payload {
    root: PathBuf,
    origin: Origin,
}

impl Payload {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// One line naming both the root and how it was found, for `zkite doctor` and for
    /// the header `zkite build` prints.
    pub fn describe(&self) -> String {
        format!("{} ({})", self.root.display(), self.origin.label())
    }
}

/// Does this directory look like a payload root? See [`MARKERS`].
fn looks_like_payload(dir: &Path) -> bool {
    MARKERS.iter().all(|m| dir.join(m).exists())
}

/// Tidy a resolved root for *reading*. The compile-time checkout arrives as
/// `…/phase_1/cli/../..`, which is correct and unreadable — and this path's whole
/// purpose is being read by someone working out why a wheel found the wrong payload.
/// Falls back to the raw path if canonicalisation fails, since a slightly ugly real
/// answer beats an error.
fn tidy(dir: PathBuf) -> PathBuf {
    dir.canonicalize().unwrap_or(dir)
}

/// Candidate payload directories relative to the running binary, in order.
///
/// Both entries are deliberately **independent of the Python version**, because a
/// `site-packages` path is not — and the distribution unit (`zkite` vs `zkite-cli`,
/// §23.8) is an open owner decision, so committing to a `site-packages` layout now
/// would be guessing at a shape nobody has chosen. See §29's known gaps.
fn beside_binary_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        // A portable unpacked tree: `zkite` and `zkite-payload/` side by side.
        exe_dir.join(BESIDE_BINARY_DIR),
        // A wheel using the `.data/scripts/` + `.data/data/` scheme installs the
        // script into `<prefix>/bin` and the data into `<prefix>/`, so the payload
        // lands one level up from the binary. Version-independent, unlike
        // `site-packages`.
        exe_dir
            .parent()
            .map(|p| p.join(BESIDE_BINARY_DIR))
            .unwrap_or_else(|| exe_dir.join(BESIDE_BINARY_DIR)),
    ]
}

/// The pure, injectable core of the search — everything that decides *which* root
/// wins, with no environment and no `current_exe()` in it.
///
/// Split out for the same reason `slider_value_at` lives in `zkite_renderer` rather
/// than `gpu_harness` (§26.3/§27.3): the arithmetic goes on the testable side of the
/// boundary and what is left outside is plumbing. Here the untestable part is
/// reading two OS-level facts.
fn resolve_from(
    env: Option<&Path>,
    exe_dir: Option<&Path>,
    checkout: &Path,
) -> Result<Payload, String> {
    // An explicit override is an assertion, not a hint: someone who sets it means it,
    // so a bad value is a loud error rather than a silent fall-through to a checkout
    // that happens to be on this machine. That silent fall-through is exactly the
    // failure mode `wheels.rs:76` had (§23.2: "the quiet one — it degrades rather
    // than fails"), and reproducing it in the resolver meant to fix it would be
    // absurd.
    if let Some(dir) = env {
        if !looks_like_payload(dir) {
            return Err(format!(
                "{ENV_VAR}={} does not look like a zkite payload — expected to find {} \
                 under it.\n  Unset it to fall back to the payload beside the binary, or \
                 point it at a directory laid out like a zkite checkout.",
                dir.display(),
                MARKERS.join(" and "),
            ));
        }
        return Ok(Payload { root: tidy(dir.to_path_buf()), origin: Origin::Env });
    }

    if let Some(exe_dir) = exe_dir {
        for candidate in beside_binary_candidates(exe_dir) {
            if looks_like_payload(&candidate) {
                return Ok(Payload { root: tidy(candidate), origin: Origin::BesideBinary });
            }
        }
    }

    if looks_like_payload(checkout) {
        return Ok(Payload { root: tidy(checkout.to_path_buf()), origin: Origin::Checkout });
    }

    Err(format!(
        "no zkite build payload found. Tried, in order:\n  \
         1. ${ENV_VAR} (not set)\n  \
         2. {BESIDE_BINARY_DIR}/ beside the zkite binary{}\n  \
         3. the compile-time checkout at {}\n\
         A payload is a directory containing {}. Nothing packages one into a \
         distributable yet — see phase_3/PLAN.md §23.",
        exe_dir.map(|d| format!(" ({})", d.display())).unwrap_or_default(),
        checkout.display(),
        MARKERS.join(" and "),
    ))
}

/// The resolved payload for this process. Cached, because the answer cannot change
/// mid-run and because several commands ask independently (`wheels::pip_command` has
/// no root threaded into it at all).
pub fn get() -> anyhow::Result<&'static Payload> {
    static RESOLVED: OnceLock<Result<Payload, String>> = OnceLock::new();
    RESOLVED
        .get_or_init(|| {
            let env = std::env::var_os(ENV_VAR).map(PathBuf::from);
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf));
            resolve_from(env.as_deref(), exe_dir.as_deref(), Path::new(CHECKOUT))
        })
        .as_ref()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// The payload root. The one function that replaces all five compile-time anchors.
pub fn root() -> anyhow::Result<&'static Path> {
    Ok(get()?.root())
}

// ---------------------------------------------------------------- the staged set
//
// §29.7 left this gap open in as many words: "`stage_payload.sh` derives its staged
// set by mirroring `build_cmd.rs` by hand, so if a staging site is added and not
// mirrored here, an off-checkout build breaks and no test catches it — the same class
// as §18.7's ungated `gen_icons.py`. A check belongs in the CI that does not exist."
//
// A check is the weaker answer. There is no second list to check *against* any more:
// `stage_payload.sh` asks `zkite payload-manifest` what to copy, so the script cannot
// drift from the binary that will later read the result — it has nothing of its own to
// drift with. What a check could still catch is a *new* read site in `build_cmd.rs`
// that never gets added here, and that is what `the_manifest_covers_every_payload_path_
// build_cmd_reads` does, by reading the source (`payload_manifest_lint`).

/// One thing a payload must carry. `Glob` exists for exactly one case — the
/// `lib*_python.so` set beside each Python prefix — and naming it beats either
/// copying the whole `lib/` directory (which drags in a 5.6 MB duplicate
/// `libpython3.14.so` the shell's own `artifacts/` already provides) or teaching the
/// shell script a second list of basenames.
pub enum Entry {
    File(String),
    Dir(String),
    Glob { dir: String, pattern: String },
}

impl Entry {
    /// The line format `stage_payload.sh` reads: kind, path, optional pattern, all
    /// tab-separated. Deliberately not JSON — the consumer is `while read` in bash,
    /// and none of these paths can contain a tab.
    fn line(&self) -> String {
        match self {
            Entry::File(p) => format!("file\t{p}"),
            Entry::Dir(p) => format!("dir\t{p}"),
            Entry::Glob { dir, pattern } => format!("glob\t{dir}\t{pattern}"),
        }
    }

    /// The path this entry is rooted at — what a completeness check resolves.
    pub fn path(&self) -> &str {
        match self {
            Entry::File(p) | Entry::Dir(p) => p,
            Entry::Glob { dir, .. } => dir,
        }
    }
}

pub const PY_VERSION: &str = "3.14";

/// The pure-Python stdlib is read from the **x86_64** prefix for every ABI, and that
/// is correct rather than a leftover (§23.4, and the long comment at `build_cmd.rs`'s
/// `stdlib_prefix`): those files are architecture-independent, and shipping one copy
/// instead of one per ABI is most of what keeps a wheel at ~19 MB.
pub const STDLIB_PREFIX_ARCH: &str = "x86_64";

pub fn abi_to_arch(abi: &str) -> anyhow::Result<&'static str> {
    match abi {
        "arm64-v8a" => Ok("aarch64"),
        "x86_64" => Ok("x86_64"),
        other => anyhow::bail!("unsupported ABI {other:?}"),
    }
}

/// Everything a payload must carry for `zkite build` on `backend` for `abis`.
///
/// The one place that knows. `stage_payload.sh` prints this and copies it; the
/// completeness test resolves it; a release pipeline would do the same. Adding a read
/// site to `build_cmd.rs` means adding it here, and the lint test says so by name.
pub fn staged_paths(backend: &str, abis: &[String], include_devshell: bool) -> anyhow::Result<Vec<Entry>> {
    let mut out = vec![
        // The two markers, first: `looks_like_payload` probes for these, so a payload
        // missing one is refused by the resolver rather than failing 80 lines into a
        // build.
        Entry::Dir("phase_1/zkite_py/src".into()),
        Entry::Dir("phase_2/pybuild/scripts".into()),
        Entry::File("phase_2/vendor/pip/pip.pyz".into()),
        Entry::File("phase_2/appshell/app/res/drawable/icon.png".into()),
        Entry::File("LICENSE".into()),
        Entry::File("THIRD_PARTY_NOTICES.txt".into()),
        Entry::Dir(format!(
            "phase_0/vendor/python-{STDLIB_PREFIX_ARCH}/prefix/lib/python{PY_VERSION}"
        )),
    ];

    for abi in abis {
        let arch = abi_to_arch(abi)?;
        let lib = format!("phase_0/vendor/python-{arch}/prefix/lib");
        out.push(Entry::Dir(format!("{lib}/python{PY_VERSION}/lib-dynload")));
        out.push(Entry::Glob { dir: lib, pattern: "lib*_python.so".into() });
    }

    let (shell_dir, libs): (&str, &[&str]) = match backend {
        "native" => (
            "phase_3/gpu_harness",
            &["libzkitegpuharness.so", "libpython3.14.so", "libc++_shared.so"],
        ),
        "webview" => ("phase_2/appshell", &["libzkiteappshell.so", "libpython3.14.so"]),
        other => anyhow::bail!("unknown backend {other:?}"),
    };
    out.push(Entry::File(format!("{shell_dir}/artifacts/classes.dex")));
    // Which build wrote those artifacts, so a packaged payload can still say (§34.8).
    out.push(Entry::File(format!("{shell_dir}/artifacts/provenance.json")));
    if backend == "native" {
        out.push(Entry::File(format!("{shell_dir}/artifacts/fonts.zip")));
    } else {
        // `stage_webruntime` prunes `test/` itself, so the whole directory is correct
        // here and the 19 MB of `node_modules` under it never reaches a build.
        out.push(Entry::Dir("phase_1/webruntime".into()));
    }
    for abi in abis {
        for lib in libs {
            out.push(Entry::File(format!("{shell_dir}/artifacts/{abi}/{lib}")));
        }
    }
    if include_devshell {
        // `zkite dev`'s own lookup is one entry in the same search order (§24.2). 39 MB
        // per ABI, and §24.3 recommends download-on-demand for it, so it is opt-in.
        let (devshell_dir, stem) = if backend == "native" {
            ("phase_3/gpu_harness", "zkite-native-devshell")
        } else {
            ("phase_1/devshell", "zkite-devshell")
        };
        for abi in abis {
            out.push(Entry::File(format!("{devshell_dir}/artifacts/{stem}-{abi}.apk")));
        }
    }
    Ok(out)
}

pub fn print_manifest(backend: &str, abis: &[String], include_devshell: bool) -> anyhow::Result<()> {
    for entry in staged_paths(backend, abis, include_devshell)? {
        println!("{}", entry.line());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{tempdir, TempDir};

    /// A directory that really does satisfy [`looks_like_payload`] — real files at the
    /// real relative paths, not a stubbed predicate. Same call §26.5 made in building
    /// its dispatch test through the real widgets: the marker check is half of what is
    /// under test, so faking it would test the test.
    fn staged_payload() -> TempDir {
        let dir = tempdir().unwrap();
        for marker in MARKERS {
            let path = dir.path().join(marker);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"# staged by a test\n").unwrap();
        }
        dir
    }

    #[test]
    fn the_env_override_wins_over_everything_else() {
        let env = staged_payload();
        let checkout = staged_payload();
        let beside = staged_payload();
        let resolved =
            resolve_from(Some(env.path()), Some(beside.path()), checkout.path()).unwrap();
        assert_eq!(resolved.root(), tidy(env.path().to_path_buf()));
        assert_eq!(resolved.origin(), Origin::Env);
    }

    #[test]
    fn beside_the_binary_wins_over_the_checkout() {
        // The regression this exists for is the whole point of §23.7 step 1: a
        // packaged install that finds a stale checkout instead of its own payload.
        let checkout = staged_payload();
        let venv = tempdir().unwrap();
        let bin = venv.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let payload = bin.join(BESIDE_BINARY_DIR);
        for marker in MARKERS {
            let path = payload.join(marker);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
        }

        let resolved = resolve_from(None, Some(&bin), checkout.path()).unwrap();
        assert_eq!(resolved.root(), tidy(payload.clone()));
        assert_eq!(resolved.origin(), Origin::BesideBinary);
    }

    #[test]
    fn a_payload_one_level_above_the_binary_is_found_too() {
        // The `.data/data/` wheel scheme: script in `<prefix>/bin`, data in
        // `<prefix>/`.
        let checkout = staged_payload();
        let prefix = staged_payload(); // doubles as `<prefix>/zkite-payload`'s parent
        let payload = prefix.path().join(BESIDE_BINARY_DIR);
        for marker in MARKERS {
            let path = payload.join(marker);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
        }
        let bin = prefix.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();

        let resolved = resolve_from(None, Some(&bin), checkout.path()).unwrap();
        assert_eq!(resolved.root(), tidy(payload.clone()));
        assert_eq!(resolved.origin(), Origin::BesideBinary);
    }

    #[test]
    fn the_checkout_is_the_last_resort_and_is_reported_as_such() {
        let checkout = staged_payload();
        let empty = tempdir().unwrap();
        let resolved = resolve_from(None, Some(empty.path()), checkout.path()).unwrap();
        assert_eq!(resolved.root(), tidy(checkout.path().to_path_buf()));
        assert_eq!(resolved.origin(), Origin::Checkout);
        assert!(
            resolved.describe().contains("dev mode"),
            "the checkout fallback must say so — a wheel silently using a checkout is \
             the failure this design introduces: {}",
            resolved.describe()
        );
    }

    #[test]
    fn an_incomplete_directory_is_not_a_payload() {
        // Half-staged is the realistic failure (an interrupted download, a partial
        // copy), and it must not win the search: `check_shell_artifacts` already
        // takes this posture for a half-staged artifacts dir.
        let dir = tempdir().unwrap();
        let one = dir.path().join(MARKERS[0]);
        std::fs::create_dir_all(one.parent().unwrap()).unwrap();
        std::fs::write(&one, b"x").unwrap();
        assert!(!looks_like_payload(dir.path()));

        let checkout = staged_payload();
        let resolved = resolve_from(None, Some(dir.path()), checkout.path()).unwrap();
        assert_eq!(resolved.origin(), Origin::Checkout);
    }

    #[test]
    fn a_bad_env_override_is_a_loud_error_not_a_silent_fallback() {
        // §23.2's "quiet one": `wheels.rs:76` degraded instead of failing, so on a
        // second machine wheel resolution changed behaviour without saying so. An
        // override that fell through to a checkout would do the same thing one layer
        // up.
        let checkout = staged_payload();
        let bogus = tempdir().unwrap();
        let err = resolve_from(Some(bogus.path()), None, checkout.path()).unwrap_err();
        assert!(err.contains(ENV_VAR), "{err}");
        assert!(err.contains(&bogus.path().display().to_string()), "{err}");
    }

    #[test]
    fn no_payload_anywhere_names_all_three_candidates() {
        let nothing = tempdir().unwrap();
        let err = resolve_from(None, Some(nothing.path()), nothing.path()).unwrap_err();
        for expected in [ENV_VAR, BESIDE_BINARY_DIR, "phase_3/PLAN.md §23"] {
            assert!(err.contains(expected), "error must name {expected}: {err}");
        }
    }

    /// Every payload-relative path literal in a source file, as the lint sees them.
    ///
    /// A deliberately small parser: it looks for the handful of spellings the codebase
    /// actually uses to reach into the payload, takes the string literal that follows,
    /// and truncates a `format!` at its first `{`. It cannot see a path assembled from
    /// a variable — which is the lint's known blind spot, recorded in §37's known gaps
    /// rather than papered over. What it does catch is the realistic regression: a new
    /// `repo_root.join("phase_N/…")` added to a build step and never staged.
    fn payload_path_literals(src: &str) -> Vec<String> {
        // (spelling, whether the literal is relative to `phase_0` rather than the root)
        const PATTERNS: [(&str, bool); 8] = [
            ("repo_root.join(\"", false),
            ("repo_root.join(format!(\"", false),
            ("root.join(\"", false),
            ("root().ok()?.join(\"", false),
            ("root()?.join(\"", false),
            ("root().unwrap().join(\"", false),
            ("phase0.join(\"", true),
            ("phase0.join(format!(\"", true),
        ];
        let mut found = Vec::new();
        for (pattern, under_phase0) in PATTERNS {
            let mut rest = src;
            while let Some(at) = rest.find(pattern) {
                rest = &rest[at + pattern.len()..];
                let Some(end) = rest.find('"') else { continue };
                let mut literal = rest[..end].to_string();
                if let Some(brace) = literal.find('{') {
                    literal.truncate(brace);
                }
                let literal = literal.trim_end_matches('/').to_string();
                if literal.is_empty() {
                    continue;
                }
                found.push(if under_phase0 { format!("phase_0/{literal}") } else { literal });
            }
        }
        found.sort();
        found.dedup();
        found
    }

    /// The union of every manifest, so the lint is not backend- or ABI-specific.
    fn every_manifest_path() -> Vec<String> {
        let abis: Vec<String> = vec!["arm64-v8a".into(), "x86_64".into()];
        let mut all = Vec::new();
        for backend in ["native", "webview"] {
            for entry in staged_paths(backend, &abis, true).unwrap() {
                all.push(entry.path().to_string());
            }
        }
        all
    }

    #[test]
    fn the_manifest_covers_every_payload_path_build_cmd_reads() {
        // §29.7's residual gap, closed from the other end. `stage_payload.sh` no
        // longer has a list to keep in sync — it asks for this one — so what is left
        // to catch is a *read* site that never became a *staged* site. That failure
        // does not show up on this machine at all: the checkout is a valid payload
        // root, so the build keeps working and only a stranger's wheel breaks.
        let manifest = every_manifest_path();
        let mut uncovered = Vec::new();
        for source in [
            include_str!("build_cmd.rs"),
            include_str!("dev_cmd.rs"),
            include_str!("wheels.rs"),
        ] {
            for literal in payload_path_literals(source) {
                // Covered either way round: `phase_1/zkite_py/src/zkite` sits under the
                // staged `phase_1/zkite_py/src`, and the staged
                // `phase_2/appshell/artifacts/classes.dex` sits under the read
                // `phase_2/appshell`.
                let covered = manifest
                    .iter()
                    .any(|m| m.starts_with(&literal) || literal.starts_with(m.as_str()));
                if !covered {
                    uncovered.push(literal);
                }
            }
        }
        assert!(
            uncovered.is_empty(),
            "these paths are read from the payload root but nothing stages them, so \
             `zkite build` works on this checkout and fails on an installed wheel \
             (phase_3/PLAN.md §29.7). Add them to `staged_paths`: {uncovered:?}"
        );
    }

    #[test]
    fn the_lint_actually_finds_a_path_that_is_not_staged() {
        // The control. The test above passes trivially if the parser returns nothing —
        // which is precisely how a lint rots — so this pins that it reads real
        // spellings out of real source and that an unstaged one is not covered.
        let literals = payload_path_literals(include_str!("build_cmd.rs"));
        assert!(
            literals.iter().any(|l| l == "phase_1/zkite_py/src"),
            "the parser must find the spellings build_cmd.rs really uses: {literals:?}"
        );
        assert!(
            literals.len() >= 8,
            "build_cmd.rs reads more than a couple of payload paths: {literals:?}"
        );

        let invented = "repo_root.join(\"phase_9/not_staged_anywhere\")";
        let manifest = every_manifest_path();
        let found = payload_path_literals(invented);
        assert_eq!(found, vec!["phase_9/not_staged_anywhere".to_string()]);
        assert!(
            !manifest.iter().any(|m| m.starts_with(&found[0]) || found[0].starts_with(m.as_str())),
            "an unstaged path must read as uncovered, or the lint asserts nothing"
        );
    }

    #[test]
    fn every_manifest_entry_resolves_in_this_checkout() {
        // The checkout is a payload root by construction (see the test below), so
        // every entry must be findable here. This is what catches a renamed or typo'd
        // path in `staged_paths` itself — the list is only useful if it is true.
        let root = Path::new(CHECKOUT);
        let abis: Vec<String> = vec!["arm64-v8a".into(), "x86_64".into()];
        for backend in ["native", "webview"] {
            for entry in staged_paths(backend, &abis, false).unwrap() {
                match &entry {
                    // Shell artifacts are build outputs, gitignored, and absent on a
                    // fresh clone — `resolve_abis` already fails loudly for those with
                    // a "run the build script" message, so asserting on them here
                    // would just make the test suite depend on having run an NDK.
                    e if e.path().contains("/artifacts/") => {}
                    Entry::File(p) => {
                        assert!(root.join(p).is_file(), "manifest names a missing file: {p}")
                    }
                    Entry::Dir(p) => {
                        assert!(root.join(p).is_dir(), "manifest names a missing directory: {p}")
                    }
                    Entry::Glob { dir, pattern } => {
                        assert!(root.join(dir).is_dir(), "manifest names a missing directory: {dir}");
                        // The `*` is medial (`lib*_python.so`), so this is a
                        // prefix/suffix test, not a `contains`. Getting that wrong is
                        // how this test first "passed" the manifest and then failed
                        // on a directory that does hold three matching files.
                        let (prefix, suffix) =
                            pattern.split_once('*').expect("the one pattern kind used here");
                        let hits = std::fs::read_dir(root.join(dir))
                            .unwrap()
                            .filter_map(|e| e.ok())
                            .filter(|e| {
                                let n = e.file_name().to_string_lossy().to_string();
                                n.starts_with(prefix) && n.ends_with(suffix)
                            })
                            .count();
                        assert!(hits > 0, "glob {pattern} in {dir} matches nothing");
                    }
                }
            }
        }
    }

    #[test]
    fn this_checkout_is_itself_a_valid_payload_root() {
        // The property the whole design rests on: the compile-time checkout satisfies
        // the same probe a packaged payload does, so the fallback needs no
        // special-casing. If someone moves `zkite_py`'s source or renames
        // `prune_stdlib.py`, this fails here rather than on a stranger's machine.
        assert!(
            looks_like_payload(Path::new(CHECKOUT)),
            "{CHECKOUT} should satisfy the payload probe"
        );
    }
}
