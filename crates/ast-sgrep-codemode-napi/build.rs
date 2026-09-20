fn main() {
    napi_build::setup();
    // The ffi_* integration tests exercise the Rust side only (Session,
    // call_now) and never call Node FFI — but the test executables still
    // link the napi rlib, whose node symbols have no provider outside a
    // Node host. Leave them unresolved: the cdylib already ships that way
    // on both targets, so this only newly affects test executables.
    // (Windows/MSVC cannot leave symbols unresolved; napi tests need
    // node.lib there. CI's windows-smoke covers the CLI, not napi tests.)
    // Target (not host): build scripts compile for the host triple.
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => {
            println!("cargo:rustc-link-arg=-undefined");
            println!("cargo:rustc-link-arg=dynamic_lookup");
        }
        Ok("linux") => {
            println!("cargo:rustc-link-arg=-Wl,--unresolved-symbols=ignore-all");
        }
        _ => {}
    }
}
