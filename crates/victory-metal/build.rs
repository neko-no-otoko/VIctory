fn main() {
    println!("cargo:rerun-if-changed=src/metal_render.m");
    println!("cargo:rerun-if-changed=src/metal_compute.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/metal_render.m")
            .file("src/metal_compute.m")
            .flag("-fobjc-arc")
            .compile("victory_metal_render");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=Metal");
    }
}
