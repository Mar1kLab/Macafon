//! Event-driven shared-mode WASAPI streams in the engine format (float32, 48 kHz).

use crate::Error;
use crate::devices::device_by_id;
use assert_no_alloc::assert_no_alloc;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient,
    IAudioClient3, IAudioRenderClient, IAudioSessionManager2, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
    WaitForSingleObject,
};
use windows::core::{GUID, Interface, w};

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID =
    GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
pub(crate) const HNS_PER_SECOND: f64 = 10_000_000.0;
pub(crate) const SOUNDPAD_SESSION: GUID = GUID::from_u128(0xf68b887a_4c27_47c6_9b66_e792fe401d75);

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

/// Default shared-mode period of a device, in 100 ns units, without opening a stream.
pub fn device_period_hns(device_id: &str) -> Result<i64, Error> {
    let device = device_by_id(device_id)?;
    // SAFETY: COM is initialised by the caller; GetDevicePeriod needs no initialised client.
    unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let mut period = 0;
        client.GetDevicePeriod(Some(&mut period), None)?;
        Ok(period)
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
    /// Open `device_id` in the shared-mode engine format at its smallest period
    /// (IAudioClient3), for render paths where the period dominates latency. The device mix
    /// format must be 48 kHz float32; `channels` is the mix format's channel count.
    pub fn open_low_latency(device_id: &str) -> Result<Self, Error> {
        let device = device_by_id(device_id)?;
        // SAFETY: COM is initialised on this thread; the mix format stays alive until it is
        // freed after initialisation; the event handle is owned by the returned Stream.
        unsafe {
            let client: IAudioClient3 = device.Activate(CLSCTX_ALL, None)?;
            let format = client.GetMixFormat()?;
            let (rate, channels, bits) = (
                (*format).nSamplesPerSec,
                (*format).nChannels,
                (*format).wBitsPerSample,
            );
            let float = (*format).wFormatTag == WAVE_FORMAT_IEEE_FLOAT
                || ((*format).wFormatTag == WAVE_FORMAT_EXTENSIBLE
                    && std::ptr::addr_of!((*format.cast::<WAVEFORMATEXTENSIBLE>()).SubFormat)
                        .read_unaligned()
                        == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT);
            let result = if rate != macafon_dsp::SAMPLE_RATE_HZ || bits != 32 || !float {
                Err(Error::Format(format!(
                    "low-latency mode needs a 48 kHz float mix format, device uses {rate} Hz {bits}-bit"
                )))
            } else {
                let (mut default, mut fundamental, mut min, mut max) = (0, 0, 0, 0);
                client
                    .GetSharedModeEnginePeriod(
                        format,
                        &mut default,
                        &mut fundamental,
                        &mut min,
                        &mut max,
                    )
                    .and_then(|()| {
                        client.InitializeSharedAudioStream(
                            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                            min,
                            format,
                            None,
                        )
                    })
                    .map(|()| min)
                    .map_err(Error::from)
            };
            CoTaskMemFree(Some(format.cast()));
            let period_frames = result?;
            let client: IAudioClient = client.cast()?;
            Self::from_initialized_client(
                client,
                channels,
                (f64::from(period_frames) * HNS_PER_SECOND / f64::from(rate)) as i64,
            )
        }
    }

    /// Open `device_id` with `channels` float32 channels at 48 kHz; Windows converts rate and
    /// channel layout from the device mix format. `buffer_hns` 0 picks the engine default.
    pub fn open(device_id: &str, channels: u16, buffer_hns: i64) -> Result<Self, Error> {
        Self::open_session(device_id, channels, buffer_hns, None)
    }

    fn open_session(
        device_id: &str,
        channels: u16,
        buffer_hns: i64,
        session: Option<&GUID>,
    ) -> Result<Self, Error> {
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
                session.map(std::ptr::from_ref),
            )?;
            Self::from_initialized_client(client, channels, period_hns)
        }
    }

    /// Attach an owned event after either shared-mode initialization path succeeds.
    fn from_initialized_client(
        client: IAudioClient,
        channels: u16,
        period_hns: i64,
    ) -> Result<Self, Error> {
        // SAFETY: both callers initialize the client on this COM thread. Once constructed,
        // Stream owns the event and closes it even if SetEventHandle fails.
        unsafe {
            let buffer_frames = client.GetBufferSize()?;
            let latency_hns = client.GetStreamLatency()?;
            let event = CreateEventW(None, false, false, None)?;
            let stream = Self {
                buffer_frames,
                latency_hns,
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

/// Stereo render stream shared by A/B playback and the soundpad monitor.
/// The caller initializes COM and retains its guard for this stream's lifetime.
pub(crate) struct RenderStream {
    client: IAudioRenderClient,
    stream: Stream,
}

impl RenderStream {
    /// False once the device was invalidated (unplugged, disabled); event delivery may stop
    /// then, so callers probe on wait timeouts.
    pub fn device_alive(&self) -> bool {
        // SAFETY: the audio client stays initialised for the stream's lifetime.
        unsafe { self.stream.client.GetCurrentPadding() }.is_ok()
    }

    pub fn open(device_id: &str) -> Result<Self, Error> {
        let stream = Stream::open(device_id, 2, 0)?;
        Self::start(stream)
    }

    pub(crate) fn open_soundpad(device_id: &str) -> Result<Self, Error> {
        let device = device_by_id(device_id)?;
        // SAFETY: COM is initialized on the opening thread. Create and label the session
        // before Initialize publishes its stream, so discovery cannot see an untagged one.
        let session = unsafe {
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
            let session = manager.GetAudioSessionControl(Some(&SOUNDPAD_SESSION), 0)?;
            session.SetGroupingParam(&SOUNDPAD_SESSION, std::ptr::null())?;
            session.SetDisplayName(w!("Soundpad"), std::ptr::null())?;
            session
        };
        let stream = Stream::open_session(device_id, 2, 0, Some(&SOUNDPAD_SESSION))?;
        let result = Self::start(stream);
        drop(session);
        result
    }

    fn start(stream: Stream) -> Result<Self, Error> {
        // SAFETY: the client was initialized by Stream::open; prefill with silence before Start.
        let client = unsafe {
            let client: IAudioRenderClient = stream.client.GetService()?;
            client.GetBuffer(stream.buffer_frames)?;
            client.ReleaseBuffer(stream.buffer_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
            stream.client.Start()?;
            client
        };
        Ok(Self { stream, client })
    }

    pub fn wait(&self, timeout_ms: u32) -> bool {
        self.stream.wait(timeout_ms)
    }

    /// Fill available frames, capped for callers with fixed-size scratch buffers.
    /// The callback runs under the same allocation guard for both playback paths.
    pub fn render_available(
        &self,
        max_frames: u32,
        render: impl FnOnce(&mut [f32], usize),
    ) -> windows::core::Result<()> {
        // SAFETY: the render client belongs to this float32 stream. GetBuffer returns room
        // for `frames` interleaved frames, released immediately after the callback fills it.
        unsafe {
            let padding = self.stream.client.GetCurrentPadding()?;
            let frames = self
                .stream
                .buffer_frames
                .min(max_frames)
                .saturating_sub(padding);
            if frames == 0 {
                return Ok(());
            }
            let channels = usize::from(self.stream.channels);
            let data = self.client.GetBuffer(frames)?;
            let output =
                std::slice::from_raw_parts_mut(data.cast::<f32>(), frames as usize * channels);
            assert_no_alloc(|| render(output, channels));
            self.client.ReleaseBuffer(frames, 0)
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
