//! Macaron's 16-byte Savitech output commands. No settings are read or initialised.
//! Register layout verified against the official devices.ibasso.cn console (2026-10-17).
use crate::{ComGuard, DataFlow, Error, devices};
use std::mem::size_of;
use std::time::{Duration, Instant};
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::HumanInterfaceDevice::*;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{HSTRING, PCWSTR, PWSTR};

const VOLUME_TABLE: [u8; 101] = [
    255, 155, 150, 145, 140, 135, 130, 125, 120, 115, 110, 109, 108, 107, 106, 105, 104, 103, 102,
    101, 100, 99, 98, 97, 96, 95, 94, 93, 92, 91, 90, 88, 86, 84, 82, 80, 78, 76, 74, 72, 70, 68,
    66, 64, 62, 60, 58, 56, 54, 52, 50, 49, 48, 47, 46, 45, 44, 43, 42, 41, 40, 39, 38, 37, 36, 35,
    34, 33, 32, 31, 30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11,
    10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
];

#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub path: String,
    pub output: Option<String>,
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this object uniquely owns a successfully opened handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {
    fn drop(&mut self) {
        // SAFETY: unique ownership of a SetupAPI device set.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

fn physical_id(mut node: u32) -> Option<String> {
    for _ in 0..12 {
        let mut buffer = [0u16; 512];
        // SAFETY: fixed output buffer; walking only live PnP nodes.
        unsafe {
            if CM_Get_Device_IDW(node, &mut buffer, 0) != CR_SUCCESS {
                return None;
            }
            let len = buffer.iter().position(|v| *v == 0).unwrap_or(buffer.len());
            let id = String::from_utf16_lossy(&buffer[..len]).to_ascii_lowercase();
            if id.starts_with("usb\\vid_262a&pid_1a0b\\") {
                return Some(id);
            }
            let mut parent = 0;
            if CM_Get_Parent(&mut parent, node, 0) != CR_SUCCESS {
                return None;
            }
            node = parent;
        }
    }
    None
}

fn endpoint_physical_id(id: &str) -> Option<String> {
    let mut text: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
    let mut node = 0;
    // SAFETY: writable, NUL-terminated instance ID and valid output pointer.
    if unsafe {
        CM_Locate_DevNodeW(
            &mut node,
            PWSTR(text.as_mut_ptr()),
            CM_LOCATE_DEVNODE_NORMAL,
        )
    } != CR_SUCCESS
    {
        return None;
    }
    physical_id(node)
}

pub fn discover() -> Result<Vec<Device>, Error> {
    let _com = ComGuard::new()?;
    let outputs = devices::endpoints()?;
    // SAFETY: standard HID interface enumeration; the RAII wrapper frees the set.
    let guid = unsafe { HidD_GetHidGuid() };
    let set = DeviceSet(unsafe {
        SetupDiGetClassDevsW(
            Some(&guid),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }?);
    let mut result = Vec::new();
    for index in 0.. {
        let mut interface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: all pointers refer to live, correctly sized structures.
        unsafe {
            if SetupDiEnumDeviceInterfaces(set.0, None, &guid, index, &mut interface).is_err() {
                break;
            }
            let mut bytes = 0;
            let _ = SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &interface,
                None,
                0,
                Some(&mut bytes),
                None,
            );
            if bytes < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 {
                continue;
            }
            // u64 storage provides the alignment required by the variable-length structure.
            let mut storage = vec![0u64; (bytes as usize).div_ceil(8)];
            let detail = storage
                .as_mut_ptr()
                .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            let mut info = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &interface,
                Some(detail),
                bytes,
                None,
                Some(&mut info),
            )?;
            let path = PCWSTR((*detail).DevicePath.as_ptr())
                .to_string()
                .map_err(|e| Error::Format(e.to_string()))?;
            if !path.to_ascii_lowercase().contains("vid_262a&pid_1a0b") {
                continue;
            }
            let Some(id) = physical_id(info.DevInst) else {
                continue;
            };
            let output = outputs
                .iter()
                .find(|endpoint| {
                    endpoint.flow == DataFlow::Render
                        && endpoint
                            .adapter_instance_id
                            .as_deref()
                            .and_then(endpoint_physical_id)
                            .as_deref()
                            == Some(&id)
                })
                .map(|e| e.id.clone());
            if !result.iter().any(|d: &Device| d.id == id) {
                result.push(Device { id, path, output });
            }
        }
    }
    Ok(result)
}

pub fn register(command: u8, chip: u8, page: u8, offset: u8, value: u8) -> [u8; 16] {
    let mut packet = [0; 16];
    packet[..4].copy_from_slice(&[command, 0x11, 0x88, chip]);
    packet[6..12].copy_from_slice(&[5, page, 0, offset, 0, value]);
    packet
}

pub fn volume_reports(volume: u8, balance: i8) -> Option<Vec<[u8; 16]>> {
    if volume > 100 || !(-10..=10).contains(&balance) {
        return None;
    }
    let base = VOLUME_TABLE[volume as usize];
    let left = base.saturating_sub((-balance).max(0) as u8);
    let right = base.saturating_sub(balance.max(0) as u8);
    let mut reports = Vec::with_capacity(10);
    for (chip, value, commands) in [
        (0x60, left, [1, 2, 9, 10, 19]),
        (0x62, right, [3, 4, 11, 12, 20]),
    ] {
        reports.push(register(commands[0], chip, 9, 1, value));
        reports.push(register(commands[1], chip, 9, 2, value));
        reports.push(register(commands[2], chip, 7, 0, value));
        reports.push(register(commands[3], chip, 7, 1, value));
        let mut memory = [0; 16];
        memory[..8].copy_from_slice(&[
            commands[4],
            0x11,
            0xa0,
            0xa2,
            0,
            if chip == 0x60 { 16 } else { 17 },
            1,
            value,
        ]);
        reports.push(memory);
    }
    Some(reports)
}

pub fn setting_reports(
    gain: Option<u8>,
    filter: Option<u8>,
    output: Option<u8>,
) -> Option<Vec<[u8; 16]>> {
    let mut reports = Vec::new();
    for (index, values, command, page) in [
        (gain, &[0, 32, 49][..], 21, 8),
        (filter, &[66, 130, 2, 194, 34][..], 17, 9),
        (output, &[28, 30][..], 23, 11),
    ] {
        if let Some(index) = index {
            let value = *values.get(index as usize)?;
            reports.push(register(command, 0x60, page, 0, value));
            reports.push(register(command + 1, 0x62, page, 0, value));
        }
    }
    Some(reports)
}

fn io_error(message: &str) -> Error {
    Error::Thread(message.into())
}

/// Passive hardware-volume notifications, also sent while another application plays audio.
pub struct Monitor {
    file: Handle,
}

impl Monitor {
    pub fn open(path: &str) -> Result<Self, Error> {
        // SAFETY: read-only shared handle; opening never writes or initialises the DAC.
        let file = Handle(unsafe {
            CreateFileW(
                &HSTRING::from(path),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }?);
        Ok(Self { file })
    }

    pub fn poll(&self) -> Result<Option<(u8, i8)>, Error> {
        let mut input = [0; 33];
        for _ in 0..32 {
            match transfer(self.file.0, &mut input, false, 5) {
                Ok(33) => {
                    if let Some(value) = parse_volume_notification(&input) {
                        return Ok(Some(value));
                    }
                }
                Ok(_) => {}
                Err(Error::Thread(message)) if message == "Macaron USB operation timed out" => {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }
}

/// Full Windows report, including report ID 0. Bytes 9/10 are channel attenuations.
pub fn parse_volume_notification(input: &[u8]) -> Option<(u8, i8)> {
    if input.len() != 33 || input[..9] != [0, 0, 0, 0, 0, 0xfe, 1, 0, 1] {
        return None;
    }
    let left = input[9];
    let right = input[10];
    let base = left.max(right);
    let balance = i16::from(left) - i16::from(right);
    if !(-10..=10).contains(&balance) {
        return None;
    }
    let volume = VOLUME_TABLE.iter().position(|value| *value == base)? as u8;
    Some((volume, balance as i8))
}

pub fn send(path: &str, reports: &[[u8; 16]]) -> Result<(), Error> {
    // SAFETY: device path is NUL-terminated; file is shared with the audio driver.
    let file = Handle(unsafe {
        CreateFileW(
            &HSTRING::from(path),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
    }?);
    let mut attributes = HIDD_ATTRIBUTES {
        Size: size_of::<HIDD_ATTRIBUTES>() as u32,
        ..Default::default()
    };
    let mut preparsed = PHIDP_PREPARSED_DATA::default();
    let mut caps = HIDP_CAPS::default();
    // SAFETY: valid HID handle and correctly sized output structures. Preparsed data is freed.
    unsafe {
        if !HidD_GetAttributes(file.0, &mut attributes)
            || attributes.VendorID != 0x262a
            || attributes.ProductID != 0x1a0b
        {
            return Err(io_error("Not a Macaron"));
        }
        if !HidD_GetPreparsedData(file.0, &mut preparsed) {
            return Err(io_error("Cannot read HID descriptor"));
        }
        let status = HidP_GetCaps(preparsed, &mut caps);
        HidD_FreePreparsedData(preparsed);
        if status.is_err() || caps.OutputReportByteLength != 17 || caps.InputReportByteLength != 33
        {
            return Err(io_error("Unsupported Macaron HID reports"));
        }
        HidD_FlushQueue(file.0);
    }
    let batch_deadline = Instant::now() + Duration::from_secs(2);
    for report in reports {
        let mut output = [0; 17];
        output[1..].copy_from_slice(report);
        let remaining = batch_deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u32;
        if remaining == 0 {
            return Err(io_error("Macaron command batch timed out"));
        }
        transfer(file.0, &mut output, true, remaining.min(500))?;
        let deadline = (Instant::now() + Duration::from_millis(750)).min(batch_deadline);
        loop {
            let mut input = [0; 33];
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u32;
            if remaining == 0 {
                return Err(io_error("Macaron acknowledgement timed out"));
            }
            let count = transfer(file.0, &mut input, false, remaining)?;
            // Family-B responses have command ID at payload offset 6 (after report ID 0).
            if count == 33 && input[0] == 0 && input[7] == report[0] {
                break;
            }
        }
    }
    Ok(())
}

fn transfer(file: HANDLE, bytes: &mut [u8], write: bool, timeout_ms: u32) -> Result<u32, Error> {
    // SAFETY: event and overlapped buffers remain live until completion or cancellation.
    unsafe {
        let event = Handle(CreateEventW(None, true, false, None)?);
        let mut overlap = OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        };
        let started = if write {
            WriteFile(file, Some(bytes), None, Some(&mut overlap))
        } else {
            ReadFile(file, Some(bytes), None, Some(&mut overlap))
        };
        if let Err(error) = started {
            if error.code() != ERROR_IO_PENDING.to_hresult() {
                return Err(error.into());
            }
        }
        if WaitForSingleObject(event.0, timeout_ms) != WAIT_OBJECT_0 {
            let _ = CancelIoEx(file, Some(&overlap));
            let mut count = 0;
            let _ = GetOverlappedResult(file, &overlap, &mut count, true);
            return Err(io_error("Macaron USB operation timed out"));
        }
        let mut count = 0;
        GetOverlappedResult(file, &overlap, &mut count, false)?;
        if write && count != bytes.len() as u32 {
            return Err(io_error("Incomplete Macaron USB write"));
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_buttons_decode_captured_attenuation_reports() {
        let mut report = [0; 33];
        report[..11].copy_from_slice(&[0, 0, 0, 0, 0, 254, 1, 0, 1, 19, 19]);
        assert_eq!(parse_volume_notification(&report), Some((81, 0)));
        report[9] = 20;
        report[10] = 17;
        assert_eq!(parse_volume_notification(&report), Some((80, 3)));
        report[5] = 0;
        assert_eq!(
            parse_volume_notification(&report),
            None,
            "command acknowledgements are not volume events"
        );
    }
    #[test]
    fn console_packets_preserve_channels_and_validate_ranges() {
        let packets = volume_reports(80, 0).expect("valid volume");
        assert_eq!(
            &packets[0][..12],
            &[1, 17, 136, 96, 0, 0, 5, 9, 0, 1, 0, 20]
        );
        assert_eq!(&packets[4][..8], &[19, 17, 160, 162, 0, 16, 1, 20]);
        assert_eq!(packets[5][3], 98);
        assert_eq!(volume_reports(80, 10).expect("right")[5][11], 10);
        assert_eq!(volume_reports(80, -10).expect("left")[0][11], 10);
        assert!(volume_reports(101, 0).is_none());
        assert!(volume_reports(80, 11).is_none());
        let modes = setting_reports(Some(1), Some(4), Some(1)).expect("modes");
        assert_eq!(
            modes.iter().map(|p| p[11]).collect::<Vec<_>>(),
            [32, 32, 34, 34, 30, 30]
        );
        assert!(setting_reports(Some(3), None, None).is_none());
    }
}
