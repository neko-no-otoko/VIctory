//! Limited native guest services for x86-64 macOS.
//!
//! This is not the Windows HLE registry or a complete Xbox runtime.

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod macos {
    use victory_loader::{ImportResolver, Resolution};
    use victory_pe::ImportSymbol;

    /// Compose the native XG and XSystem providers without substituting a
    /// fallback for imports that neither provider implements.
    pub struct NativeResolver;

    impl ImportResolver for NativeResolver {
        fn resolve(&self, dll: &str, symbol: &ImportSymbol) -> Resolution {
            match victory_xg::NativeResolver.resolve(dll, symbol) {
                Resolution::Missing => victory_runtime::NativeResolver.resolve(dll, symbol),
                found => found,
            }
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub use macos::NativeResolver;
