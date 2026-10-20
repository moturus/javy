#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
: "${RUSTUP_TOOLCHAIN:?set RUSTUP_TOOLCHAIN to the selected Motor Rust toolchain}"
export RUSTUP_TOOLCHAIN
cargo build --locked --manifest-path "$root/motor-tests/Cargo.toml" \
    --release --target x86_64-unknown-motor -j "${JOBS:-2}"
