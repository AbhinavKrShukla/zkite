#!/usr/bin/env bash
# Run inside quay.io/pypa/manylinux_2_28_x86_64 by release-candidate.yml.
# --platform only labels a wheel: the container provides the actual ABI baseline.
set -euo pipefail
export PATH="/opt/python/cp314-cp314/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
python --version
dnf install -y protobuf-compiler
curl --proto '=https' --tlsv1.2 --fail -sSf https://sh.rustup.rs -o /tmp/zkite-rustup.sh
sh /tmp/zkite-rustup.sh -y --profile minimal --default-toolchain stable
export PATH="$HOME/.cargo/bin:$PATH"
python scripts/fetch_pack.py
cargo build --release --locked --manifest-path phase_1/cli/Cargo.toml
python phase_1/cli/scripts/build_wheels.py \
  --payload candidate-payload --out candidate-wheels \
  --platform manylinux_2_28_x86_64 \
  --version "${ZKITE_CANDIDATE_VERSION:?candidate version required}" --verify
