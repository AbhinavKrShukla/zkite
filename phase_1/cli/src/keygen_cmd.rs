//! `zkite keygen` — plan1.md §7: "zkite keygen produces both forms and `zkite build
//! --keystore` accepts what developers already have." See
//! ../../phase_2/bundler/src/keystore.rs for the actual generation logic; this is
//! just the CLI-facing write-two-files wrapper.

use std::path::Path;

use anyhow::{Context, Result};
use std::io::Write;
use zkite_bundler::keystore::{generate, KeygenOptions};

pub fn run(out: &Path, common_name: &str, password: &str) -> Result<()> {
    let opts = KeygenOptions {
        common_name,
        pkcs12_password: password,
        pkcs12_alias: common_name,
        ..KeygenOptions::default()
    };
    let generated = generate(&opts).map_err(|e| anyhow::anyhow!(e))?;

    let pem_path = with_extension(out, "pem");
    let p12_path = with_extension(out, "p12");
    anyhow::ensure!(!pem_path.exists() && !p12_path.exists(),
        "refusing to replace an existing signing key at {} or {}", pem_path.display(), p12_path.display());
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
        if parent.file_name().is_some_and(|name| name == "keystore") {
            let ignore = parent.join(".gitignore");
            if !ignore.exists() { std::fs::write(ignore, "*\n")?; }
        }
    }
    let write_private = |path: &Path, bytes: &[u8]| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path).with_context(|| format!("creating {}", path.display()))?.write_all(bytes)?;
        Ok(())
    };
    write_private(&pem_path, generated.combined_pem.as_bytes())?;
    write_private(&p12_path, &generated.pkcs12)?;

    println!("wrote {}", pem_path.display());
    println!("wrote {} (password: {password})", p12_path.display());
    println!();
    println!("  zkite build apk --keystore {}", pem_path.display());
    println!();
    println!(
        "Keep both files private and back them up — losing them means you can never \
         publish an update under the same app identity again. The .p12 uses the \
         password above; change it before sharing this with anyone."
    );
    Ok(())
}

fn with_extension(base: &Path, ext: &str) -> std::path::PathBuf {
    let mut p = base.to_path_buf();
    p.set_extension(ext);
    p
}
