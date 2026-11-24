fn main() {
    println!("cargo:rerun-if-changed=src/metal_render.m");
    println!("cargo:rerun-if-changed=src/metal_compute.m");
    println!("cargo:rerun-if-changed=src/metal_present.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/metal_render.m")
            .file("src/metal_compute.m")
            .file("src/metal_present.m")
            .flag("-fobjc-arc")
            .compile("victory_metal_render");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
    }
}
