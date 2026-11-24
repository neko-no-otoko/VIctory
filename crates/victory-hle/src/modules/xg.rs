//! XG texture layout API, implemented in the shared XG crate.
pub use victory_xg::computer::*;
use crate::{HleModule, hle_export};

pub(crate) fn module() -> HleModule {
    HleModule {
        name: "xg",
        dlls: &["xg_xs.dll"],
        exports: vec![hle_export!(XGCreateTextureComputer), hle_export!(XGSuggestSwizzleMode)],
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn exports_both_xg_functions() {
        assert_eq!(super::module().exports.len(), 2);
    }
}
