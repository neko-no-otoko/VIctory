//! Windows loader-facing exports for the XGameRuntime HLE registry.
//!
//! Wine resolves these named exports normally. The linker assigns PE ordinals;
//! they are not mappings to documented Xbox ordinals. Unknown names are absent.
//! Non-Windows builds intentionally expose no XGameRuntime symbols.

#[cfg(windows)]
#[allow(non_snake_case)]
mod windows {
    use std::ffi::{c_char, c_void};
    use std::path::Path;
    use std::sync::LazyLock;

    use victory_hle::modules::xasync::E_INVALIDARG;
    use victory_hle::runtime::{self as title_runtime, Title};

    use victory_hle::modules::xgameruntime::{self as runtime, Guid, HRESULT};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut c_void) -> i32;
    }

    // Runtime objects and their methods may outlive a guest's FreeLibrary.
    fn pin_for_objects() -> Result<(), HRESULT> {
        static PIN: LazyLock<Result<(), HRESULT>> = LazyLock::new(|| {
            let mut module = std::ptr::null_mut();
            // FROM_ADDRESS interprets the function address as an address in
            // this module; PIN keeps its tables and code for process lifetime.
            let ok = unsafe { GetModuleHandleExW(0x4 | 0x1, pin_for_objects as *const () as *const u16, &mut module) };
            if ok == 0 { Err(0x8000_4005_u32 as i32) } else { Ok(()) }
        });
        *PIN
    }

    fn configured_title(root: Option<&Path>, executable: &Path) -> Result<Title, HRESULT> {
        if root.is_some_and(|root| !root.is_absolute() || !root.is_dir()) {
            return Err(E_INVALIDARG);
        }
        let dir = executable.parent().ok_or(E_INVALIDARG)?;
        let manifest = std::fs::read_to_string(dir.join("MicrosoftGame.config"))
            .unwrap_or_default();
        let mut title = Title::from_manifest(&manifest);
        if let Some(root) = root { title.storage = root.join("storage"); }
        Ok(title)
    }

    fn configured_drive(value: &std::ffi::OsStr, title: Option<&Title>) -> Result<u8, HRESULT> {
        let bytes = value.to_str().ok_or(E_INVALIDARG)?.as_bytes();
        if bytes.len() != 1 {
            return Err(E_INVALIDARG);
        }
        let drive = bytes[0].to_ascii_uppercase();
        if !(b'D'..=b'Y').contains(&drive) || matches!(drive, b'G' | b'T') {
            return Err(E_INVALIDARG);
        }
        let title = title.ok_or(E_INVALIDARG)?;
        if !title.storage.is_dir() || !Path::new(&format!("{}:\\", char::from(drive))).is_dir() {
            return Err(E_INVALIDARG);
        }
        Ok(drive)
    }

    static INITIALIZED: LazyLock<Result<(), HRESULT>> = LazyLock::new(|| {
        let executable = std::env::current_exe().map_err(|_| E_INVALIDARG)?;
        let root = std::env::var_os("VICTORY_WINE_TITLE_DATA");
        let title = configured_title(root.as_deref().map(Path::new), &executable)?;
        let drive = std::env::var_os("VICTORY_WINE_PLS_DRIVE")
            .map(|value| configured_drive(&value, Some(&title)))
            .transpose()?;

        // A null name asks Windows for the process image, not this DLL.
        let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
        if base != 0 {
            title_runtime::set_main_image(base);
            title_runtime::set_image_path(base, &executable);
        }
        title_runtime::set_wine_title(Some(title), drive);
        Ok(())
    });

    fn initialize_title() -> Result<(), HRESULT> {
        *INITIALIZED
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XGameRuntimeInitialize() -> HRESULT {
        if let Err(hr) = initialize_title() {
            return hr;
        }
        runtime::XGameRuntimeInitialize()
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XGameRuntimeUninitialize() {
        runtime::XGameRuntimeUninitialize()
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn InitializeApiImpl(version: u64, build: u64) -> HRESULT {
        if let Err(hr) = initialize_title() {
            return hr;
        }
        runtime::InitializeApiImpl(version, build)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn InitializeApiImplEx(version: u64, build: u64, flags: u64) -> HRESULT {
        if let Err(hr) = initialize_title() {
            return hr;
        }
        runtime::InitializeApiImplEx(version, build, flags)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn InitializeApiImplEx2(
        version: u64,
        build: u64,
        flags: u64,
        options: *const c_void,
    ) -> HRESULT {
        if let Err(hr) = initialize_title() {
            return hr;
        }
        runtime::InitializeApiImplEx2(version, build, flags, options)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn UninitializeApiImpl() {
        runtime::UninitializeApiImpl()
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn QueryApiImpl(
        provider: *const Guid,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> HRESULT {
        if let Err(hr) = initialize_title() {
            return hr;
        }
        if let Err(hr) = pin_for_objects() {
            return hr;
        }
        runtime::QueryApiImpl(provider, iid, out)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XErrorReport(hr: HRESULT, message: *const c_char) {
        runtime::XErrorReport(hr, message)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::ptr;
        use victory_hle::modules::{xasync, xsystem, xuser};

        #[test]
        fn configured_root_uses_adjacent_manifest_without_creating_storage() {
            let dir = std::env::temp_dir().join(format!(
                "victory-wine-title-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&dir).unwrap();
            let exe = dir.join("game.exe");
            let default = configured_title(Some(&dir), &exe).unwrap();
            assert_eq!(default.id, 0);
            assert_eq!(default.storage, dir.join("storage"));
            assert_eq!(default.saves(), dir.join("saves"));
            assert!(!default.storage.exists());
            let no_manifest = configured_title(None, &exe).unwrap();
            assert_eq!(no_manifest.id, 0);
            assert!(no_manifest.storage.as_os_str().is_empty());
            std::fs::write(dir.join("MicrosoftGame.config"), "<Game><TitleId>639AA8DD</TitleId></Game>").unwrap();
            assert_eq!(configured_title(Some(&dir), &exe).unwrap().id, 0x639A_A8DD);
            let no_root = configured_title(None, &exe).unwrap();
            assert_eq!(no_root.id, 0x639A_A8DD);
            assert!(no_root.storage.as_os_str().is_empty());
            assert_eq!(configured_title(Some(Path::new("relative")), &exe), Err(E_INVALIDARG));
            assert_eq!(configured_title(Some(&dir.join("missing")), &exe), Err(E_INVALIDARG));
            assert_eq!(configured_title(Some(&dir.join("MicrosoftGame.config")), &exe), Err(E_INVALIDARG));
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn drive_requires_valid_letter_and_existing_title_storage() {
            use std::ffi::OsStr;

            for value in ["", "C", "G", "g", "T", "t", "Z", "PP", "1", "é"] {
                assert_eq!(configured_drive(OsStr::new(value), None), Err(E_INVALIDARG));
            }
            assert_eq!(configured_drive(OsStr::new("P"), None), Err(E_INVALIDARG));
            let title = Title {
                storage: std::env::temp_dir().join(format!(
                    "victory-missing-storage-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                )),
                ..Default::default()
            };
            assert_eq!(configured_drive(OsStr::new("p"), Some(&title)), Err(E_INVALIDARG));
        }

        #[test]
        fn forwards_runtime_entry_points() {
            assert_eq!(XGameRuntimeInitialize(), runtime::S_OK);
            assert_eq!(InitializeApiImpl(1, 2), runtime::S_OK);
            assert_eq!(InitializeApiImplEx(1, 2, 3), runtime::S_OK);
            assert_eq!(InitializeApiImplEx2(1, 2, 3, ptr::null()), runtime::S_OK);
            XErrorReport(runtime::NOT_SUPPORTED, ptr::null());
            UninitializeApiImpl();
            XGameRuntimeUninitialize();
        }

        #[test]
        fn query_returns_the_shared_provider_objects() {
            for (provider, iid) in [
                (xsystem::PROVIDER, xsystem::IID),
                (xuser::PROVIDER, xuser::IID),
                (xasync::PROVIDER, xasync::PROVIDER),
            ] {
                let mut actual = ptr::null_mut();
                let mut expected = ptr::null_mut();
                assert_eq!(QueryApiImpl(&provider, &iid, &mut actual), runtime::S_OK);
                assert_eq!(runtime::QueryApiImpl(&provider, &iid, &mut expected), runtime::S_OK);
                assert!(!actual.is_null());
                assert_eq!(actual, expected);
            }

            let unknown = Guid::new(0, 0, 0, [0; 8]);
            let mut out = ptr::without_provenance_mut::<c_void>(1);
            assert_eq!(QueryApiImpl(&unknown, &unknown, &mut out), runtime::NOT_SUPPORTED);
            assert_eq!(out, ptr::without_provenance_mut::<c_void>(1));
        }
    }
}
