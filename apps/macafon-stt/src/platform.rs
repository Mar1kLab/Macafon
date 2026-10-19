#[cfg(windows)]
pub fn low_impact() -> anyhow::Result<()> {
    use windows_sys::Win32::System::Threading::*;
    // SAFETY: the pseudo handle is valid for the current process. The structure
    // and its byte size match ProcessPowerThrottling's documented ABI.
    unsafe {
        let process = GetCurrentProcess();
        anyhow::ensure!(
            SetPriorityClass(process, BELOW_NORMAL_PRIORITY_CLASS) != 0,
            "SetPriorityClass failed: {}",
            std::io::Error::last_os_error()
        );
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        };
        anyhow::ensure!(
            SetProcessInformation(
                process,
                ProcessPowerThrottling,
                (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
                std::mem::size_of_val(&state) as u32
            ) != 0,
            "SetProcessInformation failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

#[cfg(windows)]
pub fn ram_bytes() -> u64 {
    use windows_sys::Win32::System::{ProcessStatus::*, Threading::GetCurrentProcess};
    // SAFETY: zero initializes this POD output structure; the API writes only
    // within the provided structure size and uses a valid current-process handle.
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        counters.cb = std::mem::size_of_val(&counters) as u32;
        if GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ) != 0
        {
            counters.WorkingSetSize as u64
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
pub fn low_impact() -> anyhow::Result<()> {
    Ok(())
}
#[cfg(not(windows))]
pub fn ram_bytes() -> u64 {
    0
}
