// Compiles vendor/gametextinput's C++ source directly into this crate's cdylib,
// instead of linking the AAR's prebuilt `libgame-text-input.a` — see
// vendor/gametextinput/VERSION for what's vendored and why, and ../PLAN.md §5 (IME)
// for the vendoring decision itself (phase 0's README already scoped this: "the C++
// file is small enough to fold into the existing cc-crate/NDK cross-compile in
// build.rs").
//
// Compiling from source rather than linking the prebuilt `.a` keeps this crate's build
// self-contained in the same way every other native piece here is (no per-ABI archive
// selection logic to get wrong) and sidesteps having to match the prebuilt archive's
// own NDK/STL version against this workspace's — genuinely small (~400 lines, one
// file), so "just compile it" is simpler than "link someone else's static lib
// correctly."
//
// `cc` picks up the same per-target `CXX_<target_with_underscores>` env var
// scripts/build.sh now sets (mirroring the `CC_<target>` it already set for `pyo3-ffi`)
// — see that script's own comment for why the manual toolchain setup exists at all
// instead of `cargo-ndk`.
fn main() {
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let include = format!("{root}/vendor/gametextinput/include");
    let src = format!("{include}/game-text-input/gametextinput.cpp");
    println!("cargo:rerun-if-changed={src}");
    cc::Build::new()
        .cpp(true)
        .file(&src)
        .include(&include)
        // Matches the NDK's own default (libc++), and gametextinput.cpp uses
        // <mutex>/<memory>/<vector> — nothing exotic enough to need a newer standard.
        .flag_if_supported("-std=c++17")
        .compile("gametextinput");
}
