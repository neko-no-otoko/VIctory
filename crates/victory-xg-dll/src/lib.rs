//! Windows loader-facing exports for the shared XG texture implementation.
//!
//! Wine resolves these named exports normally. Other XG names are not exported;
//! PE ordinals for the named exports are linker-assigned, not Xbox ordinals.
//! Non-Windows builds intentionally expose no XG symbols.

#[cfg(windows)]
mod windows {
    use victory_xg::{ResourceDesc, computer::{self, Computer}};

    #[unsafe(no_mangle)]
    pub extern "system" fn XGCreateTextureComputer(desc: *const ResourceDesc, out: *mut *mut Computer) -> i32 {
        computer::XGCreateTextureComputer(desc, out)
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn XGSuggestSwizzleMode(desc: *const ResourceDesc, flags: u32, out: *mut u32) -> i32 {
        computer::XGSuggestSwizzleMode(desc, flags, out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::ptr;

        #[test]
        fn exports_forward_errors_and_computer_lifetime() {
            let desc = ResourceDesc {
                dimension: 3, alignment: 0, width: 256, height: 256,
                depth_or_array: 1, mips: 0, format: 98, samples: 1,
                quality: 0, layout: 0x10a, flags: 0,
            };
            let mut mode = 0;
            assert_eq!(XGSuggestSwizzleMode(&desc, 0, &mut mode), 0);
            assert_eq!(mode, 0x105);
            assert_eq!(XGSuggestSwizzleMode(ptr::null(), 0, &mut mode), 0x8007_0057u32 as i32);
            assert_eq!(XGSuggestSwizzleMode(&desc, 0, ptr::null_mut()), 0x8007_0057u32 as i32);

            let mut computer = ptr::null_mut();
            assert_eq!(XGCreateTextureComputer(ptr::null(), &mut computer), 0x8007_0057u32 as i32);
            assert!(computer.is_null());
            assert_eq!(XGCreateTextureComputer(&desc, &mut computer), 0);
            assert!(!computer.is_null());
            type Ref = extern "system" fn(*mut Computer) -> u32;
            // SAFETY: the returned object starts with its live XG vtable, whose
            // first two slots are AddRef and Release with this ABI.
            unsafe {
                let vtable = *(computer as *const *const Ref);
                let add_ref = *vtable;
                let release = *vtable.add(1);
                assert_eq!(add_ref(computer), 2);
                assert_eq!(release(computer), 1);
                assert_eq!(release(computer), 0);
            }
        }
    }
}
