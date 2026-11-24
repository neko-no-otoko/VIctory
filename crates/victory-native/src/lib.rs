//! Limited native guest services for x86-64 macOS.
//!
//! This is not the Windows HLE registry or a complete Xbox runtime.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod files;
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub use files::FileScope;


#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod macos {
    use victory_loader::{ImportResolver, Resolution};
    use victory_pe::ImportSymbol;

    /// Compose the limited native XG, XSystem and D3D12.X providers. Imports
    /// outside those providers remain missing.
    pub struct NativeResolver;

    impl ImportResolver for NativeResolver {
        fn resolve(&self, dll: &str, symbol: &ImportSymbol) -> Resolution {
            if let Some(address) = crate::files::resolve(dll, symbol) {
                return Resolution::Found { address, provenance: victory_loader::Provenance::Hle };
            }
            if dll.eq_ignore_ascii_case("d3d12_xs.dll")
                && matches!(symbol, ImportSymbol::Name { name, .. } if name == "D3D12XboxCreateDevice")
            {
                return Resolution::Found {
                    address: victory_gpu::device::native_create as *const () as usize,
                    provenance: victory_loader::Provenance::Hle,
                };
            }
            match victory_xg::NativeResolver.resolve(dll, symbol) {
                Resolution::Missing => victory_runtime::NativeResolver.resolve(dll, symbol),
                found => found,
            }
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub use macos::NativeResolver;
