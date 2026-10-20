# Motor OS port

This branch builds the `javy` JavaScript compiler and the `wasmi` core-Wasm
runner. Wasmi executes the compiler/plugin and Wizer instrumentation; Binaryen
still optimizes static output. Compiler VM, snapshot, Walrus and Binaryen
lifetimes are separated to limit peak memory. Motor memory reservations are
owned by Wasmi memories and released with their store. The C++ TLS bridge stays
local to Javy.

This is branch `motor-9.1.0` of `moturus/javy`, based on upstream
`04a467bc776b72450e660274929e0eafc8558c19`. Its Motor dependencies are git
dependencies pinned by the checked-in lockfiles, never sibling checkouts:

- `wasmi`, `wasmi_core`, `wasmi_ir`: `moturus/wasmi` branch `motor-1.1.0`,
  based on `8273dfb09d493971b7bb12fe614d740cdc857175`.
- `wasmtime-wizer`: `moturus/wasmtime` branch `motor-48.0.1`, based on
  `7bac2c2775808aaec5d4aa5627a5e447b51102cf`; Wizer stays in that workspace.
- `moto-rt`, `moto-sys`: `moturus/motor-os` branch `main`, one revision for
  both. The lockfiles may pin one ahead of `main`
  (`cargo update -p moto-rt --precise <rev>`); `cargo update -p moto-rt` moves
  them to the branch head together.

Use the checked-in lockfiles. The Motor OS tree builds the revision of this
branch named in its `src/build-addon-sources.sh`.

Prepare the default plugin once, from the upstream Javy 9.1.0 release:

```sh
mkdir -p target/motor-inputs
curl -fL https://github.com/bytecodealliance/javy/releases/download/v9.1.0/plugin.wasm.gz \
  -o target/motor-inputs/plugin.wasm.gz
printf '%s  %s\n' dc237a6fb9c7e58423456a12fc3c4e7a97d9d1eeb91b89ca73db76b78ae95e83 \
  target/motor-inputs/plugin.wasm.gz | sha256sum -c -
gzip -dc target/motor-inputs/plugin.wasm.gz > target/motor-inputs/plugin.wasm
./motor-build.sh
```

The build script additionally verifies the decompressed plugin digest. It needs
`RUSTUP_TOOLCHAIN` set to the Motor Rust toolchain and `MOTOR_SYSROOT` to its
assembly's sysroot (the `sysroot` directory beside the `images` directory that
Motor OS's `src/resolve-toolchain-assembly.sh --resolve` prints); Motor OS's
`src/build-motor-os.sh` exports both. `JAVY_DEFAULT_PLUGIN` can select another
copy of this same verified input; `CARGO_TARGET_DIR` selects disk-backed output.
The build does not download the plugin. `JOBS` defaults to two.

Run `./motor-test.sh` (same `RUSTUP_TOOLCHAIN`) to build the native memory/TLS
test crate; Wasmi's own regressions run in its checkout with its `motor-test.sh`. On Motor, run `motor-javy-tests backing` and `motor-javy-tests tls`
with `MOTOR_OS_CAPS=0`; its tests use native reservations and thread destructors.
Run `javy build input.js -o output.wasm`, then `wasmi output.wasm`. Dynamic output
uses `javy build -C dynamic=y -C plugin=plugin.wasm` and `wasmi output.wasm --plugin plugin.wasm`.
Use `MOTOR_OS_CAPS=0x200` for compilation and a scratch directory writable
by role None (`chmod rwxrwxrwx PATH` in the Motor shell).
Tool processes require role None with only the required filesystem/network bits.

Brotli remains upstream. Its use of `f32::log2` can select different compressed
source encodings with Rust's bundled libm and Linux glibc. The prototype's
Motor-only f64 workaround is deliberately excluded: valid execution does not
require it, and it is not a general cross-platform determinism solution.
Byte-for-byte comparison with the Linux release for compressed-source artifacts
remains a separate requirement to resolve. No Motor OS or toolchain math change
is part of this branch.
