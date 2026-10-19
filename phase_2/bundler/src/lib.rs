//! Phase 2's packing library — see `../PLAN.md` §1.1 for why this exists as a library
//! and not a subprocess: `zkite build` runs on the *user's* machine, so the packing
//! logic that phase 0/1 ran via `cargo run --manifest-path spike2_pack_apk` has to be
//! compiled straight into the `zkite` binary instead.
//!
//! Every byte of low-level format knowledge (AXML, ARSC, protobuf resource tables,
//! zip alignment, v1/v2/v3 signing) still comes from `pack`
//! (`phase_0/vendor/pack`), same as `spike2_pack_apk`. This crate is the "extra
//! files" splice — `classes.dex`, `lib/<abi>/*.so`, `assets/**` — generalized from
//! spike 2's `apk.rs` and extended to also cover AAB (`aab.rs`), which has the
//! identical gap for the identical reason (`../PLAN.md` §1.6).

pub mod apk;
pub mod keystore;
pub mod manifest_gen;
pub mod zip_extras;

#[cfg(feature = "aab")]
pub mod aab;

pub use pack_common::{PackError, Result};

pub mod launcher_icon;

pub mod manifest_verify;
