//! `zkite keygen` — plan1.md §7: "Wire in `rcgen` + `p12` so `zkite keygen` produces
//! both forms and `zkite build --keystore` accepts what developers already have.
//! Don't make people install `keytool` at the last mile."
//!
//! Two output files from one key: a combined PEM (what `pack_sign::crypto_keys::Keys`
//! reads — `apk.rs`/`aab.rs` consume this directly) and a PKCS#12 `.p12` (the format
//! Play Console's upload-key flow and most existing Android tooling expect, so a
//! `zkite`-generated key interoperates with `keytool`/`jarsigner`-based workflows
//! without the user ever running either).
//!
//! Key generation itself goes through the pure-Rust `rsa` crate, not `rcgen`'s own
//! key generation — `rcgen` 0.14's RSA generation only works with the `aws-lc-rs`
//! backend, an extra native dependency this crate doesn't otherwise need. `rcgen` is
//! used only to self-sign a certificate around an already-generated RSA key (via
//! `KeyPair::from_pem`), the same split `pack_sign::crypto_keys::Keys::
//! generate_random_testing_keys` already uses for its ephemeral test-key path — this
//! just gives the certificate a real, configurable subject and prints the result to
//! disk instead of discarding it after one build.

use rand::rngs::OsRng;
use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use rsa::RsaPrivateKey;

use pack_common::Result;
use pack_sign::crypto_keys::Keys;

pub struct GeneratedKeystore {
    /// Combined PEM (`CERTIFICATE` + `PRIVATE KEY`) — what `--keystore` reads.
    pub combined_pem: String,
    /// PKCS#12, password-protected — for interop with `keytool`/`jarsigner`/Play.
    pub pkcs12: Vec<u8>,
}

pub struct KeygenOptions<'a> {
    pub common_name: &'a str,
    pub rsa_bits: usize,
    pub pkcs12_password: &'a str,
    pub pkcs12_alias: &'a str,
}

impl Default for KeygenOptions<'_> {
    fn default() -> Self {
        KeygenOptions {
            common_name: "zkite",
            rsa_bits: 2048,
            pkcs12_password: "android",
            pkcs12_alias: "zkite",
        }
    }
}

pub fn generate(opts: &KeygenOptions) -> std::result::Result<GeneratedKeystore, String> {
    let private_key = RsaPrivateKey::new(&mut OsRng, opts.rsa_bits)
        .map_err(|e| format!("generating RSA-{} key: {e}", opts.rsa_bits))?;
    let private_key_pkcs8_pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| format!("encoding private key: {e}"))?
        .to_string();
    let private_key_pkcs8_der = private_key
        .to_pkcs8_der()
        .map_err(|e| format!("encoding private key: {e}"))?;

    let key_pair = rcgen::KeyPair::from_pem(&private_key_pkcs8_pem)
        .map_err(|e| format!("loading generated key into rcgen: {e}"))?;

    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, opts.common_name);
    let mut params = rcgen::CertificateParams::new(vec![])
        .map_err(|e| format!("building certificate params: {e}"))?;
    params.distinguished_name = dn;

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("self-signing certificate: {e}"))?;
    let cert_der = cert.der().to_vec();
    let cert_pem = cert.pem();

    let combined_pem = format!("{cert_pem}{private_key_pkcs8_pem}");

    // Sanity check: what we just wrote must be exactly what `Keys::from_combined_pem_string`
    // (used by apk.rs/aab.rs at build time) can parse back — fail loudly here, at
    // keygen time, rather than confusingly at the next `zkite build`.
    Keys::from_combined_pem_string(&combined_pem)
        .map_err(|e| format!("generated keystore failed self-check: {e}"))?;

    let pfx = p12::PFX::new(
        &cert_der,
        private_key_pkcs8_der.as_bytes(),
        None,
        opts.pkcs12_password,
        opts.pkcs12_alias,
    )
    .ok_or_else(|| "building PKCS#12 container failed".to_string())?;

    Ok(GeneratedKeystore { combined_pem, pkcs12: pfx.to_der() })
}

/// Load a combined PEM from disk — the shape `zkite build --keystore <file>` expects.
pub fn load_pem(path: &std::path::Path) -> Result<Keys> {
    let text = std::fs::read_to_string(path)?;
    Keys::from_combined_pem_string(&text)
}
