use std::marker::PhantomData;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

/// COM initialised (MTA) for the current thread until dropped. Not `Send`: apartments are
/// per thread.
pub struct ComGuard(PhantomData<*const ()>);

impl ComGuard {
    pub fn new() -> Result<Self, crate::Error> {
        // SAFETY: paired with CoUninitialize in Drop on the same thread (the guard is !Send).
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        Ok(Self(PhantomData))
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: matches the successful CoInitializeEx in `new`.
        unsafe { CoUninitialize() };
    }
}
