#[cfg(windows)]
pub fn scheduling(efficiency: bool, normal_priority: bool) -> anyhow::Result<()> {
    use windows_sys::Win32::System::Threading::*;
    // SAFETY: the pseudo handle is valid for the current process. The structure
    // and its byte size match ProcessPowerThrottling's documented ABI.
    unsafe {
        let process = GetCurrentProcess();
        anyhow::ensure!(
            SetPriorityClass(
                process,
                if normal_priority {
                    NORMAL_PRIORITY_CLASS
                } else {
                    BELOW_NORMAL_PRIORITY_CLASS
                }
            ) != 0,
            "SetPriorityClass failed: {}",
            std::io::Error::last_os_error()
        );
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: if efficiency {
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            } else {
                0
            },
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

#[cfg(not(any(windows, target_os = "linux")))]
pub fn scheduling(_efficiency: bool, _normal_priority: bool) -> anyhow::Result<()> {
    Ok(())
}
#[cfg(not(any(windows, target_os = "linux")))]
pub fn ram_bytes() -> u64 {
    0
}
pub fn thread_budget(requested: u16, logical: u16, efficiency: bool) -> u16 {
    let threads = requested.clamp(1, logical.max(1));
    if efficiency {
        (threads / 2).max(1)
    } else {
        threads
    }
}

#[cfg(target_os = "linux")]
pub fn scheduling(_efficiency: bool, normal_priority: bool) -> anyhow::Result<()> {
    if !normal_priority {
        // SAFETY: changes only this worker's scheduling; no privilege is needed to lower it.
        anyhow::ensure!(
            unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 10) } == 0,
            "cannot lower worker priority: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn ram_bytes() -> u64 {
    let resident = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .unwrap_or(0);
    // SAFETY: sysconf is a read-only query with a defined constant.
    let page_bytes = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    resident.saturating_mul(page_bytes.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::thread_budget;

    #[test]
    fn efficiency_reduces_the_validated_budget_without_zero_threads() {
        assert_eq!(thread_budget(7, 20, false), 7);
        assert_eq!(thread_budget(7, 20, true), 3);
        assert_eq!(thread_budget(u16::MAX, 20, true), 10);
        assert_eq!(thread_budget(0, 20, true), 1);
        assert_eq!(thread_budget(1, 1, true), 1);
    }
}
