#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
export RUSTUP_TOOLCHAIN=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$root/../motor-os/rust-toolchain.toml")
"$root/../wasmi/motor-test.sh"
cargo build --locked --manifest-path "$root/motor-tests/Cargo.toml" \
    --release --target x86_64-unknown-motor -j "${JOBS:-2}"
