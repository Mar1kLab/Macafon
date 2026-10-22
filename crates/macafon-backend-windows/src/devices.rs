//! Audio endpoint discovery and VB-CABLE detection (MIC-001, WIN-MIC-001, WIN-MIC-009).
//!
//! VB-CABLE is recognised by the hardware ID of the adapter that owns the endpoint, never by
//! the endpoint name: names are localised and users rename them.

use crate::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_IDW, CM_Get_Parent, CM_LOCATE_DEVNODE_PHANTOM,
    CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_HardwareIds, DEVPKEY_Device_Service, DEVPROPTYPE,
};
use windows::Win32::Foundation::{DEVPROPKEY, ERROR_NOT_FOUND, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATEMASK_ALL, EDataFlow, ERole, IMMDevice,
    IMMDeviceEnumerator, IMMEndpoint, IMMNotificationClient, IMMNotificationClient_Impl,
    MMDeviceEnumerator, PKEY_AudioEndpoint_FormFactor, Speakers, eAll, eCapture, eCommunications,
    eConsole, eMultimedia, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
use windows::core::{HRESULT, HSTRING, Interface, PCWSTR, implement};

/// Hardware ID of the free single VB-CABLE (WIN-MIC-001). Other VB-Audio devices (A+B, C+D,
/// Voicemeeter) report different IDs and are never used. To be confirmed on a test machine.
pub const VB_CABLE_HARDWARE_ID: &str = "VBAudioVACWDM";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFlow {
    Capture,
    Render,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointRole {
    Console,
    Multimedia,
    Communications,
}

impl EndpointRole {
    pub const ALL: [Self; 3] = [Self::Console, Self::Multimedia, Self::Communications];

    pub(crate) fn erole(self) -> ERole {
        match self {
            Self::Console => eConsole,
            Self::Multimedia => eMultimedia,
            Self::Communications => eCommunications,
        }
    }
}

impl DataFlow {
    pub(crate) fn edataflow(self) -> EDataFlow {
        match self {
            Self::Capture => eCapture,
            Self::Render => eRender,
        }
    }
}

/// One active endpoint and the PnP adapter behind it.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// MMDevice ID, stable across reboots: `{0.0.1.00000000}.{guid}`.
    pub id: String,
    pub name: String,
    pub flow: DataFlow,
    /// Device instance path of the owning adapter, e.g. `USB\VID_03F0&PID_0592&MI_00\…`.
    pub adapter_instance_id: Option<String>,
    pub hardware_ids: Vec<String>,
    pub service: Option<String>,
    /// `EndpointFormFactor`; tells the stereo `CABLE Input` (Speakers) from `CABLE In 16ch`
    /// (LineLevel) that VB-CABLE 3.3+ adds on the same cable.
    pub form_factor: Option<u32>,
}

impl Endpoint {
    pub fn is_vb_cable(&self) -> bool {
        self.hardware_ids
            .iter()
            .any(|id| id.eq_ignore_ascii_case(VB_CABLE_HARDWARE_ID))
    }

    /// Any VB-Audio device (cable variants, Voicemeeter); hidden from device pickers only when
    /// it is the cable Macafon uses.
    pub fn is_vb_audio(&self) -> bool {
        let vb = |value: &str| value.to_ascii_lowercase().starts_with("vbaudio");
        self.hardware_ids.iter().any(|id| vb(id)) || self.service.as_deref().is_some_and(vb)
    }
}

/// The two ends of VB-CABLE: Macafon renders into `input`; apps record `output`.
#[derive(Debug, Clone)]
pub struct VirtualCable {
    /// `CABLE Input`, a render endpoint.
    pub input: Endpoint,
    /// `CABLE Output`, a capture endpoint — "Macafon Microphone" in the UI (WIN-MIC-003).
    pub output: Endpoint,
}

pub(crate) fn enumerator() -> Result<IMMDeviceEnumerator, Error> {
    // SAFETY: COM is initialised by the caller (ComGuard); standard in-proc activation.
    Ok(unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?)
}

/// All active capture and render endpoints.
pub fn endpoints() -> Result<Vec<Endpoint>, Error> {
    enumerate(DEVICE_STATE_ACTIVE)
}

/// Installed endpoints, including disabled, unplugged and not-present endpoints.
/// Use endpoints() for selection; this inventory is for installation diagnostics only.
pub fn installed_endpoints() -> Result<Vec<Endpoint>, Error> {
    enumerate(DEVICE_STATE(DEVICE_STATEMASK_ALL))
}

fn enumerate(states: DEVICE_STATE) -> Result<Vec<Endpoint>, Error> {
    let enumerator = enumerator()?;
    // SAFETY: plain COM calls on a live enumerator.
    let collection = unsafe { enumerator.EnumAudioEndpoints(eAll, states) }?;
    let count = unsafe { collection.GetCount() }?;
    let mut result = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = unsafe { collection.Item(index) }?;
        result.push(describe(&device)?);
    }
    Ok(result)
}

/// Raised by Windows when an endpoint appears, goes away, is renamed or becomes a default,
/// so callers rescan on change instead of enumerating on a timer. Unregisters on drop; the
/// thread's COM apartment must outlive it.
pub struct EndpointWatch {
    enumerator: IMMDeviceEnumerator,
    callback: IMMNotificationClient,
    changed: Arc<AtomicBool>,
}

impl EndpointWatch {
    pub fn new() -> Result<Self, Error> {
        let enumerator = enumerator()?;
        // The first check reports a change: the initial inventory.
        let changed = Arc::new(AtomicBool::new(true));
        let callback: IMMNotificationClient = EndpointEvents {
            changed: changed.clone(),
        }
        .into();
        // SAFETY: the enumerator and callback are kept until unregistration in Drop.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&callback) }?;
        Ok(Self {
            enumerator,
            callback,
            changed,
        })
    }

    /// Something changed since the last call.
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }
}

impl Drop for EndpointWatch {
    fn drop(&mut self) {
        // SAFETY: the callback registered in new() stays live until this call returns.
        unsafe {
            let _ = self
                .enumerator
                .UnregisterEndpointNotificationCallback(&self.callback);
        }
    }
}

#[implement(IMMNotificationClient)]
struct EndpointEvents {
    changed: Arc<AtomicBool>,
}

impl EndpointEvents {
    fn mark(&self) -> windows::core::Result<()> {
        self.changed.store(true, Ordering::Release);
        Ok(())
    }
}

impl IMMNotificationClient_Impl for EndpointEvents_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        self.mark()
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.mark()
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.mark()
    }
    fn OnDefaultDeviceChanged(
        &self,
        _: EDataFlow,
        _: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.mark()
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, key: &PROPERTYKEY) -> windows::core::Result<()> {
        // Some drivers report other properties many times a second; only these are described.
        if *key == PKEY_Device_FriendlyName || *key == PKEY_AudioEndpoint_FormFactor {
            self.mark()?;
        }
        Ok(())
    }
}

/// The base VB-CABLE, if installed. Needs both of its endpoints active. Of several cable
/// render endpoints the stereo one (form factor Speakers) is preferred.
pub fn find_vb_cable(endpoints: &[Endpoint]) -> Option<VirtualCable> {
    let cable = |flow| {
        endpoints
            .iter()
            .filter(move |endpoint| endpoint.flow == flow && endpoint.is_vb_cable())
    };
    let input = cable(DataFlow::Render)
        .find(|endpoint| endpoint.form_factor == Some(Speakers.0 as u32))
        .or_else(|| cable(DataFlow::Render).next())?;
    Some(VirtualCable {
        input: input.clone(),
        output: cable(DataFlow::Capture).next()?.clone(),
    })
}

pub(crate) fn device_by_id(id: &str) -> Result<IMMDevice, Error> {
    let enumerator = enumerator()?;
    // SAFETY: the HSTRING outlives the call.
    unsafe { enumerator.GetDevice(&HSTRING::from(id)) }
        .map_err(|_| Error::DeviceNotFound(id.to_owned()))
}

pub(crate) fn default_device(flow: DataFlow, role: EndpointRole) -> Result<Option<String>, Error> {
    let enumerator = enumerator()?;
    // SAFETY: plain COM call; "not found" means there is no such default.
    default_endpoint(unsafe { enumerator.GetDefaultAudioEndpoint(flow.edataflow(), role.erole()) })
}

fn default_endpoint(result: windows::core::Result<IMMDevice>) -> Result<Option<String>, Error> {
    match result {
        Ok(device) => Ok(Some(device_id(&device)?)),
        Err(error) if error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn device_id(device: &IMMDevice) -> Result<String, Error> {
    // SAFETY: GetId returns a CoTaskMem string that we copy and free.
    unsafe {
        let raw = device.GetId()?;
        let id = raw.to_string();
        CoTaskMemFree(Some(raw.as_ptr().cast()));
        id.map_err(|error| Error::Format(error.to_string()))
    }
}

fn describe(device: &IMMDevice) -> Result<Endpoint, Error> {
    let id = device_id(device)?;
    // SAFETY: plain COM calls on a live device.
    let flow = unsafe { device.cast::<IMMEndpoint>()?.GetDataFlow() }?;
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }?;
    let name = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
        .map(|value| value.to_string())
        .unwrap_or_default();
    let form_factor = unsafe { store.GetValue(&PKEY_AudioEndpoint_FormFactor) }
        .ok()
        .and_then(|value| u32::try_from(&value).ok());
    let adapter = adapter_devnode(&id);
    Ok(Endpoint {
        flow: if flow == eCapture {
            DataFlow::Capture
        } else {
            DataFlow::Render
        },
        adapter_instance_id: adapter.and_then(devnode_id),
        hardware_ids: adapter
            .and_then(|node| devnode_property(node, &DEVPKEY_Device_HardwareIds))
            .unwrap_or_default(),
        service: adapter
            .and_then(|node| devnode_property(node, &DEVPKEY_Device_Service))
            .and_then(|values| values.into_iter().next()),
        form_factor,
        id,
        name,
    })
}

/// Endpoints are software devnodes `SWD\MMDEVAPI\<id>` whose parent is the audio adapter.
fn adapter_devnode(endpoint_id: &str) -> Option<u32> {
    let path = HSTRING::from(format!("SWD\\MMDEVAPI\\{endpoint_id}"));
    let mut endpoint = 0;
    let mut parent = 0;
    // SAFETY: out-pointers to locals; the HSTRING outlives the call.
    unsafe {
        if CM_Locate_DevNodeW(
            &mut endpoint,
            PCWSTR(path.as_ptr()),
            CM_LOCATE_DEVNODE_PHANTOM,
        ) != CR_SUCCESS
            || CM_Get_Parent(&mut parent, endpoint, 0) != CR_SUCCESS
        {
            return None;
        }
    }
    Some(parent)
}

fn devnode_id(node: u32) -> Option<String> {
    let mut buffer = [0u16; 512];
    // SAFETY: the buffer length is passed via the slice.
    if unsafe { CM_Get_Device_IDW(node, &mut buffer, 0) } != CR_SUCCESS {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// A string or string-list devnode property, as a list.
fn devnode_property(node: u32, key: &DEVPROPKEY) -> Option<Vec<String>> {
    let mut kind = DEVPROPTYPE::default();
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: `size` is the byte length of `buffer`.
    let result = unsafe {
        CM_Get_DevNode_PropertyW(
            node,
            key,
            &mut kind,
            Some(buffer.as_mut_ptr().cast()),
            &mut size,
            0,
        )
    };
    if result != CR_SUCCESS {
        return None;
    }
    let units = (size as usize / 2).min(buffer.len());
    Some(
        buffer[..units]
            .split(|c| *c == 0)
            .filter(|part| !part.is_empty())
            .map(String::from_utf16_lossy)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_query_distinguishes_no_device_from_service_failure() {
        let missing = windows::core::Error::from_hresult(HRESULT::from_win32(ERROR_NOT_FOUND.0));
        assert!(
            default_endpoint(Err(missing))
                .expect("no default")
                .is_none()
        );
        let failure =
            windows::core::Error::from_hresult(windows::Win32::Foundation::E_ACCESSDENIED);
        assert!(default_endpoint(Err(failure)).is_err());
    }

    fn endpoint(id: &str, flow: DataFlow, hardware_id: &str, form_factor: u32) -> Endpoint {
        Endpoint {
            id: id.into(),
            name: format!("localised name {id}"),
            flow,
            adapter_instance_id: Some(r"ROOT\MEDIA\0001".into()),
            hardware_ids: vec![hardware_id.into()],
            service: Some("VBAudioVACMME".into()),
            form_factor: Some(form_factor),
        }
    }

    #[test]
    fn picks_the_stereo_cable_input_by_hardware_id_and_form_factor() {
        let line_level = 2;
        let speakers = Speakers.0 as u32;
        let list = [
            endpoint("mic", DataFlow::Capture, r"USB\VID_03F0", 0),
            endpoint("16ch", DataFlow::Render, "VBAudioVACWDM", line_level),
            endpoint("other-cable", DataFlow::Render, "VBAudioVACAWDM", speakers),
            endpoint("input", DataFlow::Render, "VBAudioVACWDM", speakers),
            endpoint("output", DataFlow::Capture, "VBAudioVACWDM", 0),
        ];
        let cable = find_vb_cable(&list).expect("cable");
        assert_eq!(cable.input.id, "input");
        assert_eq!(cable.output.id, "output");
        assert!(list[2].is_vb_audio() && !list[2].is_vb_cable());
        assert!(
            find_vb_cable(&list[..3]).is_none(),
            "needs CABLE Output as well"
        );
    }
}
