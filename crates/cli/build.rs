fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("motor") {
        println!("cargo:rustc-link-arg=-Wl,--wrap=__emutls_get_address");
        println!("cargo:rustc-link-arg=-Wl,--wrap=__cxa_thread_atexit");
    }
    println!("cargo:rerun-if-env-changed=JAVY_DEFAULT_PLUGIN");
    let source = std::env::var_os("JAVY_DEFAULT_PLUGIN")
        .expect("set JAVY_DEFAULT_PLUGIN to the prepared, digest-verified plugin");
    println!(
        "cargo:rerun-if-changed={}",
        std::path::Path::new(&source).display()
    );
    let target = std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("plugin.wasm");
    std::fs::copy(source, target).expect("copy prepared default plugin");
}
