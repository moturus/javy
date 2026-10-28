#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# The Motor OS build exports the selected toolchain and its sysroot
# (src/build-motor-os.sh). Stand-alone: RUSTUP_TOOLCHAIN is the channel in the
# Motor OS checkout's rust-toolchain.toml, and MOTOR_SYSROOT is
# $(rustc --print sysroot)/lib/rustlib/x86_64-unknown-motor/sysroot.
: "${RUSTUP_TOOLCHAIN:?set RUSTUP_TOOLCHAIN to the selected Motor Rust toolchain}"
: "${MOTOR_SYSROOT:?set MOTOR_SYSROOT to the Motor toolchain assembly sysroot}"
export RUSTUP_TOOLCHAIN
export CC_x86_64_unknown_motor=$MOTOR_SYSROOT/bin/motor-clang
export CXX_x86_64_unknown_motor=$MOTOR_SYSROOT/bin/motor-clang++
export CXXSTDLIB_x86_64_unknown_motor=c++
export CARGO_TARGET_X86_64_UNKNOWN_MOTOR_LINKER=$MOTOR_SYSROOT/bin/motor-clang++
export CARGO_TARGET_X86_64_UNKNOWN_MOTOR_RUSTFLAGS='-C link-self-contained=no -C default-linker-libraries=yes -C link-arg=-lc++'
export JAVY_DEFAULT_PLUGIN=${JAVY_DEFAULT_PLUGIN:-$root/target/motor-inputs/plugin.wasm}
printf '180230f9346dc4b7d7139791280c9f4da09b2292eef751a3d35ae80154d88350  %s\n' "$JAVY_DEFAULT_PLUGIN" | sha256sum -c -
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target/motor}
cargo build --locked --manifest-path "$root/Cargo.toml" --release \
    --target x86_64-unknown-motor -p javy-cli -p javy-motor-engine --bin javy --bin wasmi -j "${JOBS:-2}"
