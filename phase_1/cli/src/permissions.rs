//! Infer permissions without executing app code; keep app.json as the visible source
//! of truth. Only permissions recorded as inferred may be removed automatically.
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const ANALYZER: &str = include_str!("../python/infer_permissions.py");
#[derive(Deserialize)]
struct Analysis {
    permissions: Vec<String>,
    reasons: BTreeMap<String, Vec<String>>,
    warnings: Vec<String>,
}
fn canonical(value: &str) -> String {
    if value.contains('.') { value.into() } else { format!("android.permission.{value}") }
}
fn strings(value: Option<&Value>, key: &str) -> Result<Vec<String>> {
    match value {
        None => Ok(vec![]),
        Some(value) => value.as_array().with_context(|| format!("android.{key} must be an array"))?
            .iter().map(|v| v.as_str().map(str::to_owned).with_context(|| format!("android.{key} must contain strings"))).collect(),
    }
}
pub fn sync(project: &Path) -> Result<()> {
    let path = project.join("app.json");
    let original = if path.is_file() { Some(std::fs::read(&path)?) } else { None };
    let mut app: Value = if let Some(ref bytes) = original {
        serde_json::from_slice(bytes).context("parsing app.json before permission inference")?
    } else {
        // Preserve legacy identity and keep dependencies in the existing pyproject.
        let doc = crate::build_cmd::project_document(project)?;
        let mut app = json!({"name":doc["project"]["name"].as_str().context("project name missing")?});
        if let Some(version) = doc.get("project").and_then(|p| p.get("version")) { app["version"] = serde_json::to_value(version)?; }
        if let Some(zkite) = doc.get("tool").and_then(|t| t.get("zkite")) {
            for key in ["android", "backend"] {
                if let Some(value) = zkite.get(key) { app[key] = serde_json::to_value(value)?; }
            }
        }
        app
    };
    anyhow::ensure!(app.is_object(), "app.json must be an object");
    if app.get("android").is_none() { app["android"] = json!({}); }
    let android = app.get_mut("android").unwrap().as_object_mut().context("android must be an object")?;
    let existing = strings(android.get("permissions"), "permissions")?;
    let previous = strings(android.get("inferred_permissions"), "inferred_permissions")?;
    let capabilities = strings(android.get("capabilities"), "capabilities")?;
    let mut files = BTreeMap::new();
    let ignore = [".git", ".zkite", ".venv", "venv", "__pycache__", "node_modules", "dist", "build", "target", "keystore"];
    for entry in walkdir::WalkDir::new(project).follow_links(false).into_iter().filter_entry(|e| {
        e.depth() == 0 || (!e.file_name().to_string_lossy().starts_with('.') && !ignore.contains(&e.file_name().to_string_lossy().as_ref()))
    }) {
        let entry = entry?;
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|e| e == "py") {
            let relative = entry.path().strip_prefix(project)?.to_string_lossy().replace('\\', "/");
            files.insert(relative, std::fs::read_to_string(entry.path()).with_context(|| format!("reading {}",entry.path().display()))?);
        }
    }
    let python = crate::host_python::find(Some((3,14)))?;
    let mut child = Command::new(python).args(["-I", "-S", "-c", ANALYZER])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let input = serde_json::to_vec(&json!({"files": files,"capabilities": capabilities}))?;
    child.stdin.take().context("analyzer stdin unavailable")?.write_all(&input)?;
    let output = child.wait_with_output()?;
    anyhow::ensure!(output.status.success(), "permission analysis failed: {}", String::from_utf8_lossy(&output.stderr));
    let analysis: Analysis = serde_json::from_slice(&output.stdout).context("invalid permission analysis output")?;
    let old_auto: BTreeSet<_> = previous.iter().map(|p| canonical(p)).collect();
    let mut seen = BTreeSet::new();
    let manual: Vec<_> = existing.iter().filter(|p| !old_auto.contains(&canonical(p)) && seen.insert(canonical(p))).cloned().collect();
    let manual_set: BTreeSet<_> = manual.iter().map(|p| canonical(p)).collect();
    let inferred: Vec<_> = analysis.permissions.iter().filter(|p| !manual_set.contains(&canonical(p))).cloned().collect();
    let mut merged = manual;
    merged.extend(inferred.iter().cloned());
    for permission in &merged { zkite_bundler::manifest_gen::validate_permission(permission).map_err(anyhow::Error::msg)?; }
    if !merged.is_empty() || android.contains_key("permissions") { android.insert("permissions".into(),json!(merged)); }
    if !inferred.is_empty() || android.contains_key("inferred_permissions") { android.insert("inferred_permissions".into(),json!(inferred)); }
    let changed = original.as_ref().map(|bytes| serde_json::from_slice::<Value>(bytes).ok().as_ref() != Some(&app)).unwrap_or(true);
    if changed {
        let mut staged = tempfile::NamedTempFile::new_in(project)?;
        if path.exists() { staged.as_file().set_permissions(std::fs::metadata(&path)?.permissions())?; }
        serde_json::to_writer_pretty(&mut staged, &app)?; staged.write_all(b"\n")?; staged.as_file().sync_all()?;
        let actual = std::fs::read(&path).ok();
        anyhow::ensure!(actual == original, "app.json changed during permission analysis; retry the build");
        staged.persist(&path).context("saving inferred permissions in app.json")?;
        println!("  updated app.json permissions from Python usage ({} inferred)", inferred.len());
    }
    for (permission,reasons) in &analysis.reasons {
        println!("  permission {permission}: {}", reasons.join(", "));
    }
    for warning in &analysis.warnings { eprintln!("  permission analysis: {warning}"); }
    let directory = project.join(".zkite"); std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join("permissions.json"), serde_json::to_vec_pretty(&json!({
        "permissions": merged.iter().map(|p| canonical(p)).collect::<Vec<_>>(),
        "inferred": inferred, "reasons": analysis.reasons, "warnings": analysis.warnings,
        "scope": "project Python sources and explicit capabilities; dynamic/external code requires explicit permissions"
    }))?)?;
    Ok(())
}

/// Validate bytes from the signed archive, not the pre-compilation XML.
pub fn verify_archive(bytes: &[u8], expected: &[String], bundle: bool) -> Result<()> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let path = if bundle { "base/manifest/AndroidManifest.xml" } else { "AndroidManifest.xml" };
    let mut manifest = Vec::new();
    zip.by_name(path)?.take(4 * 1024 * 1024).read_to_end(&mut manifest)?;
    let actual = if bundle {
        zkite_bundler::manifest_verify::aab_permissions(&manifest)
    } else {
        zkite_bundler::manifest_verify::apk_permissions(&manifest)
    }.map_err(anyhow::Error::msg).context("verifying compiled manifest permissions")?;
    let wanted: BTreeSet<_> = expected.iter().map(|p| canonical(p)).collect();
    anyhow::ensure!(actual == wanted, "packaged manifest permission mismatch: expected {wanted:?}, found {actual:?}");
    println!("  verified {} permissions in packaged {path}", actual.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inference_is_idempotent_and_removes_only_owned_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app.json");
        std::fs::write(&app,r#"{"name":"scan-test","android":{"permissions":["INTERNET"]}}"#).unwrap();
        let source = dir.path().join("main.py");
        std::fs::write(&source,"from zkite import audio\nasync def capture():\n return await audio.record()\n").unwrap();
        sync(dir.path()).unwrap();
        let first = std::fs::read(&app).unwrap();
        let value: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(value["android"]["inferred_permissions"],json!(["RECORD_AUDIO"]));
        assert_eq!(value["android"]["permissions"],json!(["INTERNET","RECORD_AUDIO"]));
        sync(dir.path()).unwrap();
        assert_eq!(std::fs::read(&app).unwrap(),first);
        std::fs::write(&source,"from zkite import Text\ndef view(): return Text('', 'hello')\n").unwrap();
        sync(dir.path()).unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(&app).unwrap()).unwrap();
        assert_eq!(value["android"]["permissions"],json!(["INTERNET"]));
        assert_eq!(value["android"]["inferred_permissions"],json!([]));
    }
    #[test]
    fn syntax_failure_does_not_rewrite_configuration() {
        let dir=tempfile::tempdir().unwrap();
        let content=b"{\"name\":\"invalid-source\"}\n";
        std::fs::write(dir.path().join("app.json"),content).unwrap();
        std::fs::write(dir.path().join("main.py"),"def broken(").unwrap();
        assert!(sync(dir.path()).is_err());
        assert_eq!(std::fs::read(dir.path().join("app.json")).unwrap(),content);
    }
}
