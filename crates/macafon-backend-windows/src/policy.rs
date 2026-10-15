//! System default endpoints (MIC-012, WIN-CH-007) through the undocumented `IPolicyConfig`,
//! as EarTrumpet and SoundSwitch do. No administrator rights are needed.

use crate::Error;
use crate::devices::{DataFlow, EndpointRole};
use ffi::IPolicyConfig;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::{GUID, HSTRING, PCWSTR};

/// `CPolicyConfigClient`.
const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

#[allow(non_snake_case, dead_code)]
mod ffi {
    use std::ffi::c_void;
    use windows::Win32::Media::Audio::ERole;
    use windows::core::{HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, interface};

    // Windows 7 and later layout; only SetDefaultEndpoint is called, the rest keep the vtable
    // order. Parameters we never pass are opaque pointers.
    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        fn GetDeviceFormat(
            &self,
            device: PCWSTR,
            default: i32,
            format: *mut *mut c_void,
        ) -> HRESULT;
        fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(
            &self,
            device: PCWSTR,
            endpoint: *mut c_void,
            mix: *mut c_void,
        ) -> HRESULT;
        fn GetProcessingPeriod(
            &self,
            device: PCWSTR,
            default: i32,
            default_period: *mut i64,
            min_period: *mut i64,
        ) -> HRESULT;
        fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
        fn GetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn GetPropertyValue(
            &self,
            device: PCWSTR,
            fx_store: i32,
            key: *const c_void,
            value: *mut c_void,
        ) -> HRESULT;
        fn SetPropertyValue(
            &self,
            device: PCWSTR,
            fx_store: i32,
            key: *const c_void,
            value: *mut c_void,
        ) -> HRESULT;
        pub fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
    }
}

/// Current default endpoint IDs for each role, in `EndpointRole::ALL` order.
pub fn defaults(flow: DataFlow) -> Result<[Option<String>; 3], Error> {
    let [console, multimedia, communications] = EndpointRole::ALL;
    Ok([
        crate::devices::default_device(flow, console)?,
        crate::devices::default_device(flow, multimedia)?,
        crate::devices::default_device(flow, communications)?,
    ])
}

/// Make `device_id` the default for `role`. Callers journal the previous default first and
/// restore it on quit (WIN-MIC-005, WIN-CH-008).
pub fn set_default(device_id: &str, role: EndpointRole) -> Result<(), Error> {
    // SAFETY: COM is initialised by the caller; the HSTRING outlives the call.
    unsafe {
        let policy: IPolicyConfig = CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;
        let id = HSTRING::from(device_id);
        policy
            .SetDefaultEndpoint(PCWSTR(id.as_ptr()), role.erole())
            .ok()?;
    }
    Ok(())
}
