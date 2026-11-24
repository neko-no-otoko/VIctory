fn main() {
    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => println!("cargo:rustc-link-arg-cdylib=/DELAYLOAD:victory-probe-delayed.dll"),
        Ok("gnu") => {
            println!("cargo:rustc-link-arg-cdylib=-Wl,--delayload=victory-probe-delayed.dll");
        }
        other => panic!("unsupported target environment for delay loading: {other:?}"),
    }
    println!("cargo:rustc-link-lib=delayimp");
}
