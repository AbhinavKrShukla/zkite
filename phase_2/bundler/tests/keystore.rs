//! `zkite keygen`'s output, checked the way phase 0 checked `pack`'s output: against
//! the real Android tools it replaces, not just against itself.

use zkite_bundler::keystore::{generate, KeygenOptions};

#[test]
fn generated_pem_round_trips_through_load_pem() {
    let dir = tempfile::tempdir().unwrap();
    let out = generate(&KeygenOptions::default()).expect("keygen");
    let pem_path = dir.path().join("keystore.pem");
    std::fs::write(&pem_path, &out.combined_pem).unwrap();

    let keys = zkite_bundler::keystore::load_pem(&pem_path).expect("load_pem");
    assert!(!keys.certificate.is_empty());
}

#[test]
fn pkcs12_output_is_readable_by_openssl() {
    // openssl is a system tool used only to *verify* the output here, mirroring
    // phase 0's "check pack's output with apksigner/zipalign/aapt2" pattern — zkite
    // itself never shells out to openssl at build or keygen time.
    //
    // Finding: `p12::PFX::new` encrypts the cert bag with RC2-40-CBC and the key bag
    // with 3DES — the traditional PKCS#12 algorithms every JDK `keytool`/`jarsigner`
    // reads natively (confirmed below in `pkcs12_output_is_readable_by_keytool`, the
    // interop check that actually matters for a Play Console upload key). OpenSSL
    // 3.0 moved both algorithms behind the `legacy` provider by default, so a plain
    // `openssl pkcs12 -in ...` fails with "unsupported... Algorithm (RC2-40-CBC)"
    // even though the file is well-formed PKCS#12 — hence `-legacy` below.
    let openssl = std::process::Command::new("openssl").arg("version").output();
    if openssl.is_err() {
        eprintln!("openssl not on PATH, skipping PKCS#12 cross-check");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let opts = KeygenOptions { pkcs12_password: "testpass123", ..KeygenOptions::default() };
    let out = generate(&opts).expect("keygen");
    let p12_path = dir.path().join("keystore.p12");
    std::fs::write(&p12_path, &out.pkcs12).unwrap();

    let result = std::process::Command::new("openssl")
        .args([
            "pkcs12",
            "-legacy",
            "-in",
            p12_path.to_str().unwrap(),
            "-noout",
            "-passin",
            "pass:testpass123",
        ])
        .output()
        .expect("run openssl");

    assert!(
        result.status.success(),
        "openssl could not read the generated .p12:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn distinct_keygen_calls_produce_distinct_keys() {
    let a = generate(&KeygenOptions::default()).expect("keygen a");
    let b = generate(&KeygenOptions::default()).expect("keygen b");
    assert_ne!(a.combined_pem, b.combined_pem);
}

#[test]
fn pkcs12_output_is_readable_by_keytool() {
    // keytool is the tool Play Console / Android tooling docs actually assume for
    // PKCS#12 upload keys (see ../PLAN.md notes) — check interop against it
    // directly rather than only against openssl, whose 3.x defaults dropped the
    // legacy RC2/3DES algorithms Java's own PKCS#12 implementation still expects.
    let keytool = std::process::Command::new("keytool").arg("-help").output();
    if keytool.is_err() {
        eprintln!("keytool not on PATH, skipping PKCS#12 cross-check");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let opts = KeygenOptions { pkcs12_password: "testpass123", ..KeygenOptions::default() };
    let out = generate(&opts).expect("keygen");
    let p12_path = dir.path().join("keystore.p12");
    std::fs::write(&p12_path, &out.pkcs12).unwrap();

    let result = std::process::Command::new("keytool")
        .args([
            "-list",
            "-v",
            "-keystore",
            p12_path.to_str().unwrap(),
            "-storetype",
            "PKCS12",
            "-storepass",
            "testpass123",
        ])
        .output()
        .expect("run keytool");

    assert!(
        result.status.success(),
        "keytool could not read the generated .p12:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let listing = String::from_utf8_lossy(&result.stdout);
    assert!(listing.contains("zkite"), "expected alias 'zkite' in keytool listing:\n{listing}");
}
