//! Windows loader-facing exports for the D3D12.X HLE registry.
//!
//! This DLL does not map or modify guest images. Wine resolves imports in the
//! usual way. Resolution is lazy, on the first call rather than in DllMain.
//!
//! Non-Windows builds intentionally expose no D3D12.X symbols; build this
//! crate for a Windows target to produce `d3d12_xs.dll`.

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::sync::LazyLock;

    use victory_gpu::device::VirtualMemory;
    use victory_gpu::object::{HRESULT, Iid, Object};
    use victory_gpu::root::{Desc, VersionedDesc};
    use victory_hle::{HleResolver, HostPolicy};
    use victory_loader::{ImportResolver, Provenance, Resolution};
    use victory_pe::ImportSymbol;

    type Configure = extern "system" fn(*mut VirtualMemory) -> HRESULT;
    type CreateDevice = extern "system" fn(*const c_void, *const u32, *const Iid, *mut *const Object) -> HRESULT;
    type Serialize = extern "system" fn(*const Desc, u32, *mut *mut c_void, *mut *mut c_void) -> HRESULT;
    type SerializeVersioned = extern "system" fn(*const VersionedDesc, *mut *mut c_void, *mut *mut c_void) -> HRESULT;
    type Deserialize = extern "system" fn(*const u8, usize, *const Iid, *mut *mut c_void) -> HRESULT;

    struct Exports {
        configure: Configure,
        create_device: CreateDevice,
        serialize: Serialize,
        serialize_versioned: SerializeVersioned,
        deserialize: Deserialize,
    }

    static EXPORTS: LazyLock<Exports> = LazyLock::new(|| {
        let resolver = HleResolver::new(HostPolicy::none());
        let resolve = |name: &str| {
            let symbol = ImportSymbol::Name { hint: 0, name: name.into() };
            match resolver.resolve("d3d12_xs.dll", &symbol) {
                Resolution::Found { address, provenance: Provenance::Hle } if address != 0 => address,
                _ => panic!("missing D3D12.X HLE registry export {name}"),
            }
        };
        // SAFETY: each registry entry names the HLE function with the
        // corresponding explicit ABI above. These addresses live for the
        // lifetime of this DLL. In particular, create_device is the HLE
        // entry, which installs its allocator and page mapper.
        unsafe {
            Exports {
                configure: std::mem::transmute::<usize, Configure>(resolve("D3D12ConfigureVirtualMemoryX")),
                create_device: std::mem::transmute::<usize, CreateDevice>(resolve("D3D12XboxCreateDevice")),
                serialize: std::mem::transmute::<usize, Serialize>(resolve("D3D12SerializeRootSignature")),
                serialize_versioned: std::mem::transmute::<usize, SerializeVersioned>(resolve("D3D12SerializeVersionedRootSignature")),
                deserialize: std::mem::transmute::<usize, Deserialize>(resolve("D3D12CreateRootSignatureDeserializer")),
            }
        }
    });

    fn exports() -> &'static Exports {
        &EXPORTS
    }

    // A PE data export must contain the IID itself, not a pointer to it.
    // Keep normal GUID alignment while initializing from the HLE's IID.
    #[repr(C, align(4))]
    pub struct AlignedIid(Iid);

    #[unsafe(no_mangle)]
    pub static IID_ID3D12Device: AlignedIid = AlignedIid(victory_gpu::device::IID);

    #[unsafe(no_mangle)]
    pub extern "system" fn D3D12ConfigureVirtualMemoryX(config: *mut VirtualMemory) -> HRESULT {
        (exports().configure)(config)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn D3D12XboxCreateDevice(adapter: *const c_void, parameters: *const u32, iid: *const Iid, out: *mut *const Object) -> HRESULT {
        (exports().create_device)(adapter, parameters, iid, out)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn D3D12SerializeRootSignature(desc: *const Desc, version: u32, blob: *mut *mut c_void, error: *mut *mut c_void) -> HRESULT {
        (exports().serialize)(desc, version, blob, error)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn D3D12SerializeVersionedRootSignature(desc: *const VersionedDesc, blob: *mut *mut c_void, error: *mut *mut c_void) -> HRESULT {
        (exports().serialize_versioned)(desc, blob, error)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn D3D12CreateRootSignatureDeserializer(blob: *const u8, size: usize, iid: *const Iid, out: *mut *mut c_void) -> HRESULT {
        (exports().deserialize)(blob, size, iid, out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn exports_resolve_and_iid_matches_registry_data() {
            let _ = exports();
            let symbol = ImportSymbol::Name { hint: 0, name: "IID_ID3D12Device".into() };
            let Resolution::Found { address, provenance: Provenance::Hle } =
                HleResolver::new(HostPolicy::none()).resolve("d3d12_xs.dll", &symbol)
            else {
                panic!("missing IID HLE registry export");
            };
            // SAFETY: the registry's IID entry points to a static Iid.
            assert_eq!(IID_ID3D12Device.0, unsafe { *(address as *const Iid) });
        }
    }
}
