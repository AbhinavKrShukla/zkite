//! Does the prebuilt shell we are about to ship match the source it claims?
//! (`../../phase_3/PLAN.md` §34.8.)
//!
//! **The problem this exists for.** `zkite build apk` compiles nothing. It copies
//! `libzkitegpuharness.so`, `libpython3.14.so`, `libc++_shared.so`, `classes.dex` and
//! `fonts.zip` verbatim out of `<shell>/artifacts/`, and until this module there was
//! no build id, no hash and no warning anywhere in that path. Twice that shipped the
//! wrong binary:
//!
//! * **§34.8** — a negative-control build with the frame sink disabled was the last
//!   thing to write `phase_3/gpu_harness/artifacts/`, so the app under test had no
//!   push path and the symptom read as a network hang. `strings` could not tell the
//!   two shells apart; the only reliable tell was the `.so`'s mtime against the build
//!   log.
//! * **§35** — the same trap, avoided only by byte-comparing `classes.dex` against a
//!   freshly built one by hand.
//!
//! **So the check is not on the artifact's bytes.** Both incidents produced a
//! perfectly well-formed artifact; what was false was *which source it came from*.
//! The build scripts now write `artifacts/provenance.json` recording a digest over
//! exactly the source inputs that fed them (`phase_1/cli/scripts/artifact_provenance.py`),
//! and this module re-derives that digest from the payload and compares.
//!
//! **The three answers are deliberately different**, because they are different
//! statements and §34.8's whole lesson is that collapsing them into silence is what
//! ships the wrong app:
//!
//! | situation | meaning | what `zkite build` does |
//! |---|---|---|
//! | digests differ | the claim is **false** — stale artifact | **fails**, naming the rebuild command |
//! | sources absent | a packaged payload; nothing to compare | reports what the record says |
//! | no record | nothing claims anything | warns, and says which artifact is unverified |
//!
//! **The fail-open path is the middle row, and §31.6 is explicit that it cannot be
//! tested by observing that it did not fire** — so it has its own test with the
//! sources really removed, asserting it both stays silent about failure *and* still
//! prints the recorded identity. A payload that says nothing about its own shell is
//! how §34.8 happened in the first place.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The escape hatch, for the one legitimate case: deliberately building an app
/// against a shell you have just patched and do not intend to rebuild. Named rather
/// than implicit, so it shows up in a shell history when someone asks how a control
/// build got shipped.
pub const SKIP_ENV: &str = "ZKITE_SKIP_SHELL_PROVENANCE";

const RECORD_NAME: &str = "provenance.json";

/// The record layout this build of `zkite` understands. A record from the future is an
/// error rather than a best-effort read: serde would silently accept one whose fields
/// had been *redefined* and hand back a confident wrong answer, and a confident wrong
/// answer about which build an artifact came from is the entire subject of §34.8.
const SCHEMA: u32 = 1;

/// Matched against whole path *components*, so `build/` (cargo and javac scratch) is
/// excluded while `build.rs` — a real input — is not. Must stay identical to
/// `artifact_provenance.py`'s `EXCLUDED_COMPONENTS`.
const EXCLUDED_COMPONENTS: [&str; 8] = [
    "target",
    "build",
    "artifacts",
    "__pycache__",
    ".git",
    "node_modules",
    ".mypy_cache",
    ".pytest_cache",
];
const EXCLUDED_SUFFIXES: [&str; 2] = [".pyc", ".pyo"];

#[derive(Debug, Deserialize)]
pub struct Record {
    pub schema: u32,
    pub shell: String,
    #[serde(default)]
    pub groups: BTreeMap<String, Group>,
}

#[derive(Debug, Deserialize)]
pub struct Group {
    pub built_at: String,
    pub rebuild: String,
    pub source_hash: String,
    pub source_inputs: Vec<String>,
    /// `false` means the recorded inputs do not fully determine the artifact, so a
    /// matching digest is *not* a verification. `fonts.zip` is the case: its faces
    /// come from the host's `/usr/share/fonts`, which no digest over this repository
    /// can see.
    #[serde(default = "yes")]
    pub source_complete: bool,
    #[serde(default)]
    pub git_head: Option<String>,
    #[serde(default)]
    pub git_dirty: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Recorded and re-derived digests agree, over a complete input set.
    Matches,
    /// They disagree: this artifact was built from different source than is on disk.
    Stale { recorded: String, current: String, rebuild: String, built_at: String },
    /// The record's inputs are not in this payload — the ordinary shape of a
    /// packaged install, which carries `artifacts/` and no sources. Reported rather
    /// than verified, and never an error.
    SourcesAbsent { found: usize, of: usize, built_at: String, source_hash: String },
    /// The record exists and says so itself: a match here would not mean anything.
    NotDeterminedBySource { built_at: String },
}

impl Verdict {
    pub fn is_stale(&self) -> bool {
        matches!(self, Verdict::Stale { .. })
    }
}

/// Read a shell's record. `Ok(None)` means there is no record; a malformed one is an
/// error rather than a `None`, because "unreadable" and "absent" want different
/// answers and quietly treating a corrupt record as absent is the silence this
/// module exists to remove.
pub fn read(artifacts_dir: &Path) -> anyhow::Result<Option<Record>> {
    let path = artifacts_dir.join(RECORD_NAME);
    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow::anyhow!("reading {}: {e}", path.display())),
    };
    let record: Record = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("{} is not a readable provenance record: {e}", path.display()))?;
    if record.schema != SCHEMA {
        anyhow::bail!(
            "{} is a schema-{} provenance record and this zkite understands schema {SCHEMA} \
             — rebuild the shell with the matching checkout's build scripts",
            path.display(),
            record.schema
        );
    }
    Ok(Some(record))
}

/// `df6a798 (dirty)` — the half of the record that answers "which build is this?"
/// directly. §34.8 answered that question with the `.so`'s mtime against a build log,
/// which is the only tool that worked and is not one anybody should need.
fn built_from(group: &Group) -> String {
    match (&group.git_head, group.git_dirty) {
        (Some(head), true) => format!(", git {} (dirty tree)", &head[..7.min(head.len())]),
        (Some(head), false) => format!(", git {}", &head[..7.min(head.len())]),
        (None, _) => String::new(),
    }
}

/// Check one group of a record against the sources under `payload_root`.
pub fn check_group(payload_root: &Path, group: &Group) -> Verdict {
    if !group.source_complete {
        return Verdict::NotDeterminedBySource { built_at: group.built_at.clone() };
    }
    let present = group
        .source_inputs
        .iter()
        .filter(|rel| payload_root.join(rel).exists())
        .count();
    if present != group.source_inputs.len() {
        return Verdict::SourcesAbsent {
            found: present,
            of: group.source_inputs.len(),
            built_at: group.built_at.clone(),
            source_hash: group.source_hash.clone(),
        };
    }
    let current = source_digest(payload_root, &group.source_inputs);
    if current == group.source_hash {
        Verdict::Matches
    } else {
        Verdict::Stale {
            recorded: group.source_hash.clone(),
            current,
            rebuild: group.rebuild.clone(),
            built_at: group.built_at.clone(),
        }
    }
}

/// The canonical stream's sha256 — the Rust half of the digest
/// `phase_1/cli/scripts/artifact_provenance.py` writes. The two must agree byte for
/// byte, so the definition lives in one place and is quoted in both:
///
/// > collect every regular file under every input path, keyed by its repo-relative
/// > path, dropping the excluded components;
/// > sort by that path's raw bytes;
/// > for each: `path` · NUL · `len(bytes)` as decimal ASCII · NUL · `bytes` · NUL;
/// > sha256 over the whole stream, lowercase hex, full 64 characters.
///
/// The length field is not redundant with the NUL terminators: every input set here
/// contains files that can hold NUL bytes (`.jar`, `.ttf`, `.png`), so without it two
/// different trees can collide by construction.
///
/// Pinned against the Python implementation by a test that runs it, rather than by
/// the two docstrings agreeing — which is the improvement on §31.6's arrangement,
/// where `runtime.py` and `dev_cmd.rs` keep one digest in step by prose alone.
pub fn source_digest(root: &Path, inputs: &[String]) -> String {
    use sha2::{Digest, Sha256};

    // Keyed by the repo-relative path's raw bytes: overlapping inputs contribute each
    // file exactly once, and `BTreeMap`'s ordering *is* the sort the stream needs, so
    // the digest cannot depend on the order inputs were passed in.
    let mut files: BTreeMap<Vec<u8>, PathBuf> = BTreeMap::new();
    for rel_input in inputs {
        collect_into(root, rel_input, &mut files);
    }

    let mut h = Sha256::new();
    for (rel, abs) in &files {
        let Ok(data) = std::fs::read(abs) else { continue };
        h.update(rel);
        h.update(b"\0");
        h.update(data.len().to_string().as_bytes());
        h.update(b"\0");
        h.update(&data);
        h.update(b"\0");
    }
    format!("{:x}", h.finalize())
}

fn excluded(rel: &str) -> bool {
    rel.split('/').any(|c| EXCLUDED_COMPONENTS.contains(&c))
        || EXCLUDED_SUFFIXES.iter().any(|s| rel.ends_with(s))
}

fn collect_into(root: &Path, rel_input: &str, out: &mut BTreeMap<Vec<u8>, PathBuf>) {
    let abs_input = root.join(rel_input);
    // Symlinks are skipped rather than followed, here and in the Python writer.
    // Following one would make the digest depend on something outside the tree.
    let Ok(meta) = std::fs::symlink_metadata(&abs_input) else { return };
    if meta.is_symlink() {
        return;
    }
    if meta.is_file() {
        if !excluded(rel_input) {
            out.insert(rel_input.as_bytes().to_vec(), abs_input);
        }
        return;
    }
    if !meta.is_dir() {
        return;
    }
    for entry in walkdir::WalkDir::new(&abs_input)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.file_name()
                .to_str()
                .map(|n| !EXCLUDED_COMPONENTS.contains(&n))
                .unwrap_or(false)
        })
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if !excluded(&rel) {
            out.insert(rel.into_bytes(), entry.path().to_path_buf());
        }
    }
}

/// Check every group of one shell and print what each says, failing on any stale
/// one. The single entry point `build_cmd` calls.
///
/// Returns an error only for [`Verdict::Stale`]. Everything else is reported: the
/// point is that an unverifiable artifact says so out loud rather than looking the
/// same as a verified one.
pub fn verify_shell(
    artifacts_dir: &Path,
    payload_root: &Path,
    build_script: &str,
) -> anyhow::Result<()> {
    if std::env::var_os(SKIP_ENV).is_some() {
        eprintln!("  ! {SKIP_ENV} is set — not checking that {} matches its source", artifacts_dir.display());
        return Ok(());
    }

    let Some(record) = read(artifacts_dir)? else {
        eprintln!(
            "  ! no {RECORD_NAME} under {} — nothing records which build produced these \
             artifacts, so `zkite build` cannot tell a current shell from one left behind \
             by an experiment (phase_3/PLAN.md §34.8). Run {build_script} to write one.",
            artifacts_dir.display()
        );
        return Ok(());
    };

    let mut stale = Vec::new();
    for (name, group) in &record.groups {
        match check_group(payload_root, group) {
            Verdict::Matches => {
                println!(
                    "  shell {}/{name}: matches source (built {}{})",
                    record.shell,
                    group.built_at,
                    built_from(group)
                );
            }
            Verdict::NotDeterminedBySource { built_at } => {
                println!(
                    "  shell {}/{name}: built {built_at}{} — not determined by repo source, \
                     not verified",
                    record.shell,
                    built_from(group)
                );
            }
            Verdict::SourcesAbsent { found, of, built_at, source_hash } => {
                println!(
                    "  shell {}/{name}: built {built_at}{} from source {} \
                     (packaged payload — {found}/{of} inputs present, not re-derived)",
                    record.shell,
                    built_from(group),
                    &source_hash[..12.min(source_hash.len())],
                );
            }
            Verdict::Stale { recorded, current, rebuild, built_at } => {
                stale.push(format!(
                    "  {}/{name}: built {built_at}{} from source {}, but this tree is {}\n     \
                     rebuild with: {rebuild}",
                    record.shell,
                    built_from(group),
                    &recorded[..12.min(recorded.len())],
                    &current[..12.min(current.len())],
                ));
            }
        }
    }

    if !stale.is_empty() {
        anyhow::bail!(
            "the prebuilt {} shell does not match the source in this payload — \
             `zkite build` copies these artifacts verbatim, so this would ship a binary \
             built from something else (phase_3/PLAN.md §34.8):\n{}\n  \
             Set {SKIP_ENV}=1 to build anyway.",
            record.shell,
            stale.join("\n")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::{tempdir, TempDir};

    /// The writer script, so a test can run the real thing rather than a re-statement
    /// of it. Same call `phase_2/bundler`'s suite makes in shelling out to real
    /// `apksigner`/`keytool`: the artefact under test is the agreement between two
    /// programs, and a mock of one side tests nothing.
    fn writer() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/artifact_provenance.py")
    }

    fn python_digest(root: &Path, inputs: &[&str]) -> String {
        let mut cmd = Command::new(crate::host_python::find(None).expect("a host Python is required"));
        cmd.arg(writer()).arg("digest").arg("--root").arg(root);
        for i in inputs {
            cmd.arg("--input").arg(i);
        }
        let out = cmd.output().expect(
            "python3 must be on PATH: this test pins the Rust digest against the writer \
             that produces it, and a mocked writer would pin nothing",
        );
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap().to_string()
    }

    /// A tree with the shapes the real input sets have: nested directories, a binary
    /// file containing NUL bytes, an excluded `target/`, an excluded `__pycache__`,
    /// and `build.rs` sitting next to a `build/` that must be dropped.
    fn fixture() -> TempDir {
        let dir = tempdir().unwrap();
        let w = |rel: &str, body: &[u8]| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        w("crate/src/lib.rs", b"pub fn a() {}\n");
        w("crate/src/nested/deep.rs", b"// deep\n");
        w("crate/Cargo.toml", b"[package]\nname = \"c\"\n");
        w("crate/build.rs", b"fn main() {}\n");
        w("crate/blob.bin", b"\x00\x01\x00\x02NUL inside\x00");
        w("crate/target/debug/junk.rlib", b"should not be hashed");
        w("crate/build/scratch.class", b"should not be hashed");
        w("crate/src/__pycache__/x.cpython-314.pyc", b"should not be hashed");
        w("crate/src/stray.pyc", b"should not be hashed");
        dir
    }

    #[test]
    fn the_rust_and_python_digests_agree_over_a_real_tree() {
        let dir = fixture();
        let inputs = ["crate/src", "crate/Cargo.toml", "crate/build.rs", "crate/blob.bin"];
        let owned: Vec<String> = inputs.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            source_digest(dir.path(), &owned),
            python_digest(dir.path(), &inputs),
            "the two implementations of the canonical stream have drifted"
        );
    }

    #[test]
    fn a_single_byte_of_source_moves_both_digests_together() {
        // The negative control for the test above: a comparison of two constants that
        // never change would pass just as happily if neither implementation read
        // anything at all.
        let dir = fixture();
        let inputs = ["crate/src", "crate/Cargo.toml"];
        let owned: Vec<String> = inputs.iter().map(|s| s.to_string()).collect();
        let before_rs = source_digest(dir.path(), &owned);
        let before_py = python_digest(dir.path(), &inputs);

        std::fs::write(dir.path().join("crate/src/lib.rs"), b"pub fn a() {} //\n").unwrap();
        let after_rs = source_digest(dir.path(), &owned);
        let after_py = python_digest(dir.path(), &inputs);

        assert_ne!(before_rs, after_rs, "one byte of source must change the digest");
        assert_eq!(after_rs, after_py, "and both implementations must still agree");
        assert_ne!(before_py, after_py);
    }

    #[test]
    fn build_output_directories_are_excluded_but_build_rs_is_not() {
        // `build.rs` is a real input and `build/` is javac scratch. Component-wise
        // matching is what separates them, and getting it wrong is silent in both
        // directions: hashing `target/` makes every check fail, dropping `build.rs`
        // makes a real source change invisible.
        let dir = fixture();
        let all: Vec<String> = vec!["crate".to_string()];
        let with_junk = source_digest(dir.path(), &all);

        std::fs::write(dir.path().join("crate/target/debug/junk.rlib"), b"different").unwrap();
        std::fs::write(dir.path().join("crate/build/scratch.class"), b"different").unwrap();
        std::fs::write(dir.path().join("crate/src/stray.pyc"), b"different").unwrap();
        assert_eq!(with_junk, source_digest(dir.path(), &all), "excluded paths must not count");

        std::fs::write(dir.path().join("crate/build.rs"), b"fn main() { /* x */ }\n").unwrap();
        assert_ne!(with_junk, source_digest(dir.path(), &all), "build.rs must count");
    }

    fn group(inputs: &[&str], hash: &str) -> Group {
        Group {
            built_at: "2026-08-10T00:00:00Z".into(),
            rebuild: "phase_3/gpu_harness/scripts/build.sh arm64-v8a".into(),
            source_hash: hash.into(),
            source_inputs: inputs.iter().map(|s| s.to_string()).collect(),
            source_complete: true,
            git_head: None,
            git_dirty: false,
        }
    }

    #[test]
    fn a_source_edit_after_the_build_is_reported_as_stale() {
        // §34.8's exact shape: the artifact is fine and came from the wrong build.
        let dir = fixture();
        let inputs = ["crate/src"];
        let g = group(&inputs, &source_digest(dir.path(), &["crate/src".to_string()]));
        assert_eq!(check_group(dir.path(), &g), Verdict::Matches);

        std::fs::write(dir.path().join("crate/src/lib.rs"), b"pub fn a() { panic!() }\n").unwrap();
        let verdict = check_group(dir.path(), &g);
        assert!(verdict.is_stale(), "{verdict:?}");
        match verdict {
            Verdict::Stale { rebuild, .. } => {
                assert!(rebuild.contains("build.sh"), "the verdict must carry the fix: {rebuild}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_payload_without_sources_reports_rather_than_failing_or_going_quiet() {
        // §31.6: a fail-open path cannot be tested by observing that it did not fire.
        // A packaged payload carries `artifacts/` and no `src/`, so there is nothing
        // to re-derive — but going silent there is exactly how §34.8 happened, so the
        // verdict has to still carry the recorded identity.
        let empty = tempdir().unwrap();
        let g = group(&["crate/src", "crate/Cargo.toml"], "deadbeef");
        match check_group(empty.path(), &g) {
            Verdict::SourcesAbsent { found, of, source_hash, .. } => {
                assert_eq!((found, of), (0, 2));
                assert_eq!(source_hash, "deadbeef", "it must still say what it was built from");
            }
            other => panic!("a payload with no sources must not be an error: {other:?}"),
        }
    }

    #[test]
    fn a_half_staged_payload_is_not_mistaken_for_a_verified_one() {
        // The realistic corruption (an interrupted copy) must not silently hash the
        // subset that survived and call it a match — `payload::an_incomplete_directory_
        // is_not_a_payload` takes the same posture one layer up.
        let dir = fixture();
        let g = group(&["crate/src", "crate/missing.rs"], "whatever");
        match check_group(dir.path(), &g) {
            Verdict::SourcesAbsent { found, of, .. } => assert_eq!((found, of), (1, 2)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_artifact_that_says_it_is_not_determined_by_source_is_never_called_verified() {
        // fonts.zip: its faces come from the host's /usr/share/fonts. A digest over
        // this repository that happened to match would be saying nothing.
        let dir = fixture();
        let mut g = group(&["crate/src"], &source_digest(dir.path(), &["crate/src".to_string()]));
        g.source_complete = false;
        assert!(matches!(check_group(dir.path(), &g), Verdict::NotDeterminedBySource { .. }));
    }

    #[test]
    fn a_missing_record_is_distinguishable_from_an_unreadable_one() {
        let dir = tempdir().unwrap();
        assert!(read(dir.path()).unwrap().is_none(), "absent is None, not an error");
        std::fs::write(dir.path().join(RECORD_NAME), b"{not json").unwrap();
        let err = read(dir.path()).unwrap_err().to_string();
        assert!(err.contains("provenance record"), "{err}");
    }

    #[test]
    fn the_writer_produces_a_record_this_module_can_read_and_verify() {
        // End to end against the real script: write a record, read it back with
        // serde, and check it. A hand-written JSON fixture would drift from the
        // writer silently, which is the failure this whole module is about.
        let dir = fixture();
        let artifacts = dir.path().join("artifacts");
        std::fs::create_dir_all(&artifacts).unwrap();
        std::fs::write(artifacts.join("libthing.so"), b"pretend cdylib").unwrap();

        // The writer resolves inputs against the *repo*, so point it at a real one.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        let out = Command::new(crate::host_python::find(None).expect("a host Python is required"))
            .arg(writer())
            .args(["record", "--shell", "native", "--group", "testgroup"])
            .arg("--artifacts")
            .arg(&artifacts)
            .args(["--extra", "phase_1/cli/scripts"])
            .args(["--rebuild", "run the build script"])
            .args(["--built-by", "a test"])
            .args(["--file", "libthing.so"])
            .output()
            .expect("python3 must be on PATH");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

        let record = read(&artifacts).unwrap().expect("the writer must have written one");
        assert_eq!(record.shell, "native");
        let g = record.groups.get("testgroup").expect("the group it was told to write");
        assert_eq!(check_group(&repo, g), Verdict::Matches, "the writer and reader must agree");

        // The control has to be a tree where the inputs *are* present and differ —
        // against one where they are simply absent the verdict is `SourcesAbsent`,
        // which is not the same claim and would have passed an `is_stale()` check for
        // the wrong reason.
        let impostor = tempdir().unwrap();
        let scripts = impostor.path().join("phase_1/cli/scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(scripts.join("artifact_provenance.py"), b"# not the real one\n").unwrap();
        assert!(
            check_group(impostor.path(), g).is_stale(),
            "a present-but-different source tree must read as stale"
        );
    }
}
