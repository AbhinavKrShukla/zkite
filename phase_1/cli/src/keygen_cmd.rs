//! `zkite keygen` — plan1.md §7: "zkite keygen produces both forms and `zkite build
//! --keystore` accepts what developers already have." See
//! ../../phase_2/bundler/src/keystore.rs for the actual generation logic; this is
//! just the CLI-facing write-two-files wrapper.

use std::path::Path;

use anyhow::{Context, Result};
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
    std::fs::write(&pem_path, &generated.combined_pem)
        .with_context(|| format!("writing {}", pem_path.display()))?;
    std::fs::write(&p12_path, &generated.pkcs12)
        .with_context(|| format!("writing {}", p12_path.display()))?;

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
