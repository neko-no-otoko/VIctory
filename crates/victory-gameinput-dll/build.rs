use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=GameInput.def");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let def = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("GameInput.def");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-cdylib=/DEF:{}", def.display());
    } else {
        // The GNU Windows linker driver accepts a module-definition file as input.
        println!("cargo:rustc-link-arg-cdylib={}", def.display());
    }
}
