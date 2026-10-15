//! Application audio sessions on render endpoints (WIN-CH-001, WIN-CH-002, WIN-CH-004).
//! Read-only diagnostic snapshot. Live app groups and control are in [`crate::mixer`].

use crate::Error;
use crate::devices::{DataFlow, device_id, enumerator};
use windows::Win32::Foundation::{CloseHandle, S_OK};
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2,
    ISimpleAudioVolume,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{Interface, PWSTR};

#[derive(Debug, Clone)]
pub struct Session {
    pub endpoint_id: String,
    pub process_id: u32,
    /// Full executable path; grouping key is its file name (APP-002).
    pub executable: Option<String>,
    pub display_name: String,
    pub system_sounds: bool,
    pub active: bool,
    pub volume: f32,
    pub muted: bool,
    pub peak: f32,
}

/// All sessions on all active render endpoints.
pub fn sessions() -> Result<Vec<Session>, Error> {
    let enumerator = enumerator()?;
    let mut result = Vec::new();
    // SAFETY: plain COM calls on live objects; strings returned by COM are freed after copy.
    unsafe {
        let devices =
            enumerator.EnumAudioEndpoints(DataFlow::Render.edataflow(), DEVICE_STATE_ACTIVE)?;
        for index in 0..devices.GetCount()? {
            let device = devices.Item(index)?;
            let endpoint_id = device_id(&device)?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
            let list = manager.GetSessionEnumerator()?;
            for session_index in 0..list.GetCount()? {
                let control = list.GetSession(session_index)?;
                let control: IAudioSessionControl2 = control.cast()?;
                let volume: ISimpleAudioVolume = control.cast()?;
                let meter: IAudioMeterInformation = control.cast()?;
                let process_id = control.GetProcessId().unwrap_or(0);
                let display_name = match control.GetDisplayName() {
                    Ok(raw) => {
                        let name = raw.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(raw.as_ptr().cast()));
                        name
                    }
                    Err(_) => String::new(),
                };
                result.push(Session {
                    endpoint_id: endpoint_id.clone(),
                    process_id,
                    executable: executable(process_id),
                    display_name,
                    system_sounds: control.IsSystemSoundsSession() == S_OK,
                    active: control.GetState()? == AudioSessionStateActive,
                    volume: volume.GetMasterVolume()?,
                    muted: volume.GetMute()?.as_bool(),
                    peak: meter.GetPeakValue().unwrap_or(0.0),
                });
            }
        }
    }
    Ok(result)
}

pub(crate) fn executable(process_id: u32) -> Option<String> {
    if process_id == 0 {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut size = buffer.len() as u32;
    // SAFETY: the handle is closed below; `size` is the buffer length in characters.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()?;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(process);
        result.ok()?;
    }
    Some(String::from_utf16_lossy(&buffer[..size as usize]))
}
