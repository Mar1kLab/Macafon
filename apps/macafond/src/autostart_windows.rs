//! Opt-in, per-user daemon autostart. Reading the state never changes the registry.

use macafon_ipc::{AutostartMethod, AutostartState};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyW, RegDeleteKeyValueW,
    RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("Macafon");

/// Windows paths cannot contain quotes. Always quote the executable, including spaces,
/// shell metacharacters and non-ASCII characters; Run values are not shell commands.
fn command_line(executable: &Path) -> Result<Vec<u16>, String> {
    let path: Vec<u16> = executable.as_os_str().encode_wide().collect();
    if path.is_empty() || path.iter().any(|unit| matches!(*unit, 0 | 34)) {
        return Err("autostart executable path is empty or contains a quote/NUL".into());
    }
    Ok(std::iter::once(34)
        .chain(path)
        .chain("\" run".encode_utf16())
        .chain(std::iter::once(0))
        .collect())
}

pub fn state() -> AutostartState {
    let mut bytes = 0;
    // SAFETY: predefined HKCU handle, static NUL-terminated names and writable size.
    // A size-only query does not allocate a buffer or modify the registry.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        )
    };
    if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND {
        tracing::warn!(error = %result.to_hresult(), "Cannot read Windows autostart state");
    }
    AutostartState {
        enabled: result == ERROR_SUCCESS && bytes > 2,
        method: AutostartMethod::WindowsRun,
    }
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: this handle was opened by RegCreateKeyW and is closed exactly once.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

pub fn set(enabled: bool) -> Result<AutostartState, String> {
    if enabled {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let command = command_line(&executable)?;
        let mut handle = HKEY::default();
        // SAFETY: static NUL-terminated key name and writable output handle. Only HKCU
        // is accessed; creating the Run key requires no administrator privileges.
        unsafe { RegCreateKeyW(HKEY_CURRENT_USER, RUN_KEY, &mut handle) }
            .ok()
            .map_err(|error| error.to_string())?;
        let key = Key(handle);
        // SAFETY: key is open; command is a NUL-terminated UTF-16 buffer, with its byte
        // size including the terminator. The buffer lives until the call returns.
        unsafe {
            RegSetKeyValueW(
                key.0,
                None,
                VALUE,
                REG_SZ.0,
                Some(command.as_ptr().cast()),
                u32::try_from(command.len() * 2).map_err(|error| error.to_string())?,
            )
        }
        .ok()
        .map_err(|error| error.to_string())?;
    } else {
        // SAFETY: predefined HKCU handle and static names; deletes only the Macafon value.
        let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) };
        if result != ERROR_FILE_NOT_FOUND {
            result.ok().map_err(|error| error.to_string())?;
        }
    }
    Ok(state())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_command_quotes_the_executable_and_preserves_windows_paths() {
        for path in [
            r"C:\Macafon\macafond.exe",
            r"C:\Program Files\Macafon\macafond.exe",
            r"C:\Users\Тест & 100%\macafond.exe",
            r"\\server\share name\macafond.exe",
        ] {
            let command = command_line(Path::new(path)).expect("valid executable path");
            assert_eq!(command.last(), Some(&0));
            assert_eq!(
                String::from_utf16(&command[..command.len() - 1]).expect("UTF-16"),
                format!("\"{path}\" run")
            );
        }
        assert!(command_line(Path::new("bad\"path.exe")).is_err());
        assert!(command_line(Path::new("bad\0path.exe")).is_err());
        assert!(command_line(Path::new("")).is_err());
    }
}
