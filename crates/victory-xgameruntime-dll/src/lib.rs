//! Windows loader-facing exports for the XGameRuntime HLE registry.
//!
//! Wine resolves these named exports normally. The linker assigns PE ordinals;
//! they are not mappings to documented Xbox ordinals. Unknown names are absent.
//! Non-Windows builds intentionally expose no XGameRuntime symbols.

#[cfg(windows)]
#[allow(non_snake_case)]
mod windows {
    use std::ffi::{c_char, c_void};

    use victory_hle::modules::xgameruntime::{self as runtime, Guid, HRESULT};

    #[unsafe(no_mangle)]
    pub extern "system" fn XGameRuntimeInitialize() -> HRESULT {
        runtime::XGameRuntimeInitialize()
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XGameRuntimeUninitialize() {
        runtime::XGameRuntimeUninitialize()
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn InitializeApiImpl(version: u64, build: u64) -> HRESULT {
        runtime::InitializeApiImpl(version, build)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn InitializeApiImplEx(version: u64, build: u64, flags: u64) -> HRESULT {
        runtime::InitializeApiImplEx(version, build, flags)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn InitializeApiImplEx2(
        version: u64,
        build: u64,
        flags: u64,
        options: *const c_void,
    ) -> HRESULT {
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
