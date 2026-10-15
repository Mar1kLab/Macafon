//! Event-driven shared-mode WASAPI streams in the engine format (float32, 48 kHz).

use crate::Error;
use crate::devices::device_by_id;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient,
    IAudioClient3, WAVEFORMATEX,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
    WaitForSingleObject,
};
use windows::core::w;

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
pub(crate) const HNS_PER_SECOND: f64 = 10_000_000.0;

/// Shared-mode engine periods a device supports at its mix format (IAudioClient3), in frames.
#[derive(Debug, Clone, Copy)]
pub struct EnginePeriods {
    pub mix_rate_hz: u32,
    pub mix_channels: u16,
    pub default_frames: u32,
    pub fundamental_frames: u32,
    pub min_frames: u32,
    pub max_frames: u32,
}

pub fn engine_periods(device_id: &str) -> Result<EnginePeriods, Error> {
    let device = device_by_id(device_id)?;
    // SAFETY: COM is initialised by the caller; the mix format is freed after use.
    unsafe {
        let client: IAudioClient3 = device.Activate(CLSCTX_ALL, None)?;
        let format = client.GetMixFormat()?;
        let (mut default, mut fundamental, mut min, mut max) = (0, 0, 0, 0);
        let result = client.GetSharedModeEnginePeriod(
            format,
            &mut default,
            &mut fundamental,
            &mut min,
            &mut max,
        );
        let periods = EnginePeriods {
            mix_rate_hz: (*format).nSamplesPerSec,
            mix_channels: (*format).nChannels,
            default_frames: default,
            fundamental_frames: fundamental,
            min_frames: min,
            max_frames: max,
        };
        CoTaskMemFree(Some(format.cast()));
        result?;
        Ok(periods)
    }
}

pub(crate) struct Stream {
    pub client: IAudioClient,
    event: HANDLE,
    pub channels: u16,
    pub buffer_frames: u32,
    /// Engine period of the device, in 100 ns units.
    pub period_hns: i64,
    /// Stream latency reported by the audio engine, in 100 ns units.
    pub latency_hns: i64,
}

impl Stream {
    /// Open `device_id` with `channels` float32 channels at 48 kHz; Windows converts rate and
    /// channel layout from the device mix format. `buffer_hns` 0 picks the engine default.
    pub fn open(device_id: &str, channels: u16, buffer_hns: i64) -> Result<Self, Error> {
        let device = device_by_id(device_id)?;
        let rate = macafon_dsp::SAMPLE_RATE_HZ;
        let block_align = channels * 4;
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
            nChannels: channels,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * u32::from(block_align),
            nBlockAlign: block_align,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        // SAFETY: COM is initialised on this thread; `format` outlives Initialize; the event
        // handle is owned by the returned Stream and closed in Drop.
        unsafe {
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let mut period_hns = 0;
            client.GetDevicePeriod(Some(&mut period_hns), None)?;
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                buffer_hns,
                0,
                &format,
                None,
            )?;
            let event = CreateEventW(None, false, false, None)?;
            let stream = Self {
                buffer_frames: client.GetBufferSize()?,
                latency_hns: client.GetStreamLatency()?,
                client,
                event,
                channels,
                period_hns,
            };
            stream.client.SetEventHandle(stream.event)?;
            Ok(stream)
        }
    }

    /// Wait for the next device period; false on timeout.
    pub fn wait(&self, timeout_ms: u32) -> bool {
        // SAFETY: the event handle is valid for the lifetime of `self`.
        unsafe { WaitForSingleObject(self.event, timeout_ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: stopping an initialised client and closing our own event handle.
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

/// MMCSS "Pro Audio" registration for the current thread, reverted on drop.
pub(crate) struct ProAudioThread(HANDLE);

impl ProAudioThread {
    pub fn register() -> Option<Self> {
        let mut task_index = 0;
        // SAFETY: registers the calling thread; the handle is reverted in Drop.
        unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) }
            .ok()
            .map(Self)
    }
}

impl Drop for ProAudioThread {
    fn drop(&mut self) {
        // SAFETY: the handle came from AvSetMmThreadCharacteristicsW on this thread.
        unsafe {
            let _ = AvRevertMmThreadCharacteristics(self.0);
        }
    }
}
