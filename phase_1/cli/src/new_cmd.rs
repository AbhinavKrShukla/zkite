//! `zkite new <name>` — plan1.md §0: `zkite new myapp && cd myapp` should be the whole
//! setup. App settings are independent of the Python package manager.
//!
//! Templates are embedded at compile time from `../../templates/new_project/` (kept
//! as real, syntax-highlightable files on disk rather than Rust string literals) so
//! there's exactly one place to edit them. `zkite.tailwind.json`'s widget list must
//! stay in sync with `zkite_py/src/zkite/tailwind_config.py::WIDGET_NAMES` — there's no
//! automated check for that yet (cross-language, low value until the widget set
//! moves again), just this comment.

use anyhow::{bail, Context, Result};
use zkite_bundler::manifest_gen::sanitize_package_segment;
#[cfg(test)]
use zkite_bundler::manifest_gen::validate_package;
use std::fs;
use std::path::Path;

const MAIN_PY: &str = include_str!("../../templates/new_project/main.py");
const APP_JSON: &str = include_str!("../../templates/new_project/app.json");
const TAILWIND_CONFIG: &str = include_str!("../../templates/new_project/zkite.tailwind.json");
const GITIGNORE: &str = include_str!("../../templates/new_project/.gitignore");

pub fn run(name: &str, in_dir: &Path) -> Result<()> {
    run_at(name, in_dir, false)
}

pub fn run_at(name: &str, in_dir: &Path, current: bool) -> Result<()> {
    validate_project_name(name)?;
    let project_dir = if current { in_dir.to_path_buf() } else { in_dir.join(name) };
    if !current && project_dir.exists() {
        bail!(
            "{} already exists — `zkite new` only creates fresh projects",
            project_dir.display()
        );
    }
    for file in ["main.py", "app.json", "zkite.tailwind.json", ".gitignore", "assets"] {
        if project_dir.join(file).symlink_metadata().is_ok() {
            bail!("{} already exists — refusing to overwrite it", project_dir.join(file).display());
        }
    }
    fs::create_dir_all(&project_dir)
        .with_context(|| format!("creating {}", project_dir.display()))?;

    write(&project_dir, "main.py", MAIN_PY)?;
    write(
        &project_dir,
        "app.json",
        // Two substitutions, not one: the project name goes in verbatim, but the
        // package segment has to be sanitized — `zkite new my-app` scaffolding
        // `com.example.my-app` would fail `validate_package` on the first build.
        &APP_JSON
            .replace("__ZKITE_PACKAGE_SUFFIX__", &sanitize_package_segment(name))
            .replace("__ZKITE_PROJECT_NAME__", name),
    )?;
    write(&project_dir, "zkite.tailwind.json", TAILWIND_CONFIG)?;
    write(&project_dir, ".gitignore", GITIGNORE)?;
    fs::create_dir(project_dir.join("assets"))?;
    write(&project_dir, "assets/.gitkeep", "")?;

    Ok(())
}

fn write(project_dir: &Path, filename: &str, contents: &str) -> Result<()> {
    let path = project_dir.join(filename);
    fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))
}

fn validate_project_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("project name cannot be empty");
    }
    let valid = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid || name.starts_with(|c: char| c.is_ascii_digit()) {
        bail!(
            "'{name}' isn't a valid project name — use letters, digits, '_' or '-', \
             not starting with a digit (it becomes a Python package name)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scaffold_and_current_directory_are_safe() {
        let dir = tempdir().unwrap();
        run_at("my-cool_App", dir.path(), true).unwrap();
        let config: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.path().join("app.json")).unwrap()).unwrap();
        assert_eq!(config["name"], "my-cool_App");
        assert_eq!(config["android"]["package"], "com.example.my_cool_app");
        assert!(validate_package(config["android"]["package"].as_str().unwrap()).is_ok());
        assert!(dir.path().join("assets").is_dir());
        assert!(!dir.path().join("pyproject.toml").exists());
        assert!(run_at("other", dir.path(), true).is_err());
    }

    #[test]
    fn conflict_does_not_partially_scaffold() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "existing").unwrap();
        assert!(run_at("app", dir.path(), true).is_err());
        assert!(!dir.path().join("main.py").exists());
        assert_eq!(fs::read_to_string(dir.path().join(".gitignore")).unwrap(), "existing");
    }

    #[test]
    fn main_py_defines_view_and_uses_class_string_first_order() {
        let dir = tempdir().unwrap();
        run("myapp", dir.path()).unwrap();
        let main_py = fs::read_to_string(dir.path().join("myapp/main.py")).unwrap();
        assert!(main_py.contains("def view(s: Welcome):"));
        // class string first (see ../../PLAN.md §3's finding), not content first
        assert!(main_py.contains("Text(\"w-full text-3xl"));
        assert!(main_py.contains("Button(\"bg-orange-600"));
    }

    #[test]
    fn refuses_to_overwrite_an_existing_directory() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("myapp")).unwrap();
        let err = run("myapp", dir.path()).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn rejects_invalid_project_names() {
        let dir = tempdir().unwrap();
        assert!(run("123bad", dir.path()).is_err());
        assert!(run("has space", dir.path()).is_err());
        assert!(run("", dir.path()).is_err());
    }

    #[test]
    fn accepts_hyphens_and_underscores() {
        let dir = tempdir().unwrap();
        assert!(run("my-cool_app", dir.path()).is_ok());
    }
}
