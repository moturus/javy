#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
motor=$(cd -- "$root/../motor-os" && pwd)
export RUSTUP_TOOLCHAIN=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$motor/rust-toolchain.toml")
assembly_images=$("$motor/src/resolve-toolchain-assembly.sh" --resolve)
assembly_sysroot=${assembly_images%/images}/sysroot
export CC_x86_64_unknown_motor=$assembly_sysroot/bin/motor-clang
export CXX_x86_64_unknown_motor=$assembly_sysroot/bin/motor-clang++
export CXXSTDLIB_x86_64_unknown_motor=c++
export CARGO_TARGET_X86_64_UNKNOWN_MOTOR_LINKER=$assembly_sysroot/bin/motor-clang++
export CARGO_TARGET_X86_64_UNKNOWN_MOTOR_RUSTFLAGS='-C link-self-contained=no -C default-linker-libraries=yes -C link-arg=-lc++'
export JAVY_DEFAULT_PLUGIN=${JAVY_DEFAULT_PLUGIN:-$root/target/motor-inputs/plugin.wasm}
printf '180230f9346dc4b7d7139791280c9f4da09b2292eef751a3d35ae80154d88350  %s\n' "$JAVY_DEFAULT_PLUGIN" | sha256sum -c -
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target/motor}
cargo build --locked --manifest-path "$root/Cargo.toml" --release \
    --target x86_64-unknown-motor -p javy-cli -p javy-motor-engine --bin javy --bin wasmi -j "${JOBS:-2}"
