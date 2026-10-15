//! Read-only Windows Doctor facts, collected synchronously with COM initialised by caller.
//! Engine periods are not measured end-to-end latency; the cable control panel is not read.
use crate::{
    DataFlow, Endpoint, EndpointRole, Error, devices,
    journal::{self, Defaults},
    sessions::{self, Session},
    wasapi::{self, EnginePeriods},
};
use std::path::Path;
use windows::Win32::{
    Media::Audio::{IAudioClient, PKEY_AudioEngine_DeviceFormat, WAVEFORMATEX},
    System::{
        Com::{CLSCTX_ALL, CoTaskMemFree, STGM_READ},
        Variant::VT_BLOB,
    },
};

/// Guidance from docs/development/windows-m3.md: default cable buffering added roughly
/// 58–90 ms; low-latency render still measured 82–88 ms overall. No tested numeric Max
/// Latency is available yet, so this is a tuning recommendation, not a latency guarantee.
pub const CABLE_LATENCY_RECOMMENDATION: &str = "Reduce Max Latency in VBCABLE_ControlPanel, then repeat the latency probe and check for dropouts. Default cable buffering contributed about 58–90 ms; no validated Max Latency value is available yet. WASAPI engine periods do not measure total latency.";
/// Windows location for changing the shared/device format.
pub const FORMAT_SETTINGS: &str = "Windows Sound settings > More sound settings > Playback/Recording > device Properties > Advanced > Default Format: select 48000 Hz.";

/// Shared mix or configured device format; bits denotes the sample container width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub rate_hz: u32,
    pub channels: u16,
    pub bits: u16,
}
impl AudioFormat {
    /// Whether the 48 kHz DSP engine needs sample-rate conversion for this format.
    pub fn needs_resampling(self) -> bool {
        self.rate_hz != macafon_dsp::SAMPLE_RATE_HZ
    }
}
impl From<WAVEFORMATEX> for AudioFormat {
    fn from(f: WAVEFORMATEX) -> Self {
        Self {
            rate_hz: f.nSamplesPerSec,
            channels: f.nChannels,
            bits: f.wBitsPerSample,
        }
    }
}

/// Per-endpoint read errors remain facts so one unavailable property does not hide others.
#[derive(Debug)]
pub struct EndpointFacts {
    pub endpoint: Endpoint,
    pub mix_format: Result<AudioFormat, Error>,
    pub device_format: Result<AudioFormat, Error>,
    pub engine_periods: Result<EnginePeriods, Error>,
}

/// Installed means the base cable has registered endpoints, even if none are active.
/// Other VB-Audio products are reported separately and never used as the base cable.
#[derive(Debug)]
pub struct CableState {
    pub installed: bool,
    pub registered_endpoints: Vec<Endpoint>,
    pub active_endpoints: Vec<Endpoint>,
    pub other_vb_audio: Vec<Endpoint>,
}

/// Pure inventory classification; names are deliberately not used for detection.
pub fn classify_cable(installed: &[Endpoint], active: &[Endpoint]) -> CableState {
    let registered_endpoints: Vec<_> = installed
        .iter()
        .filter(|e| e.is_vb_cable())
        .cloned()
        .collect();
    CableState {
        installed: !registered_endpoints.is_empty(),
        registered_endpoints,
        active_endpoints: active.iter().filter(|e| e.is_vb_cable()).cloned().collect(),
        other_vb_audio: installed
            .iter()
            .filter(|e| e.is_vb_audio() && !e.is_vb_cable())
            .cloned()
            .collect(),
    }
}

/// Active sessions on any base cable render endpoint except our PID count as foreign.
/// Muted/zero-peak sessions still own streams and may resume writing; idle sessions do not.
/// PID 0 (system sounds/unknown ownership) is conservatively foreign.
pub fn foreign_writers<'a>(
    sessions: &'a [Session],
    active: &[Endpoint],
    our_pid: u32,
) -> Vec<&'a Session> {
    sessions
        .iter()
        .filter(|s| {
            s.active
                && (s.process_id == 0 || s.process_id != our_pid)
                && active
                    .iter()
                    .any(|e| e.id == s.endpoint_id && e.flow == DataFlow::Render && e.is_vb_cable())
        })
        .collect()
}

/// Roles pointing at any base cable render endpoint, including its 16-channel variant.
pub fn cable_default_roles(
    defaults: &[Option<String>; 3],
    inventory: &[Endpoint],
) -> Vec<EndpointRole> {
    EndpointRole::ALL
        .into_iter()
        .zip(defaults)
        .filter_map(|(role, id)| {
            inventory
                .iter()
                .any(|e| {
                    Some(&e.id) == id.as_ref() && e.flow == DataFlow::Render && e.is_vb_cable()
                })
                .then_some(role)
        })
        .collect()
}

/// Snapshot for daemon checks, without a running pipeline or a latency probe.
#[derive(Debug)]
pub struct DoctorFacts {
    pub cable: CableState,
    /// Selected CABLE Input/Output (when active) and the requested physical microphone.
    pub endpoints: Vec<EndpointFacts>,
    pub foreign_writers: Vec<Session>,
    pub render_defaults: [Option<String>; 3],
    pub cable_default_roles: Vec<EndpointRole>,
    pub latency_recommendation: &'static str,
    pub format_settings: &'static str,
}

/// Collect independent properties without starting streams or changing Windows settings.
/// A requested microphone must be active, capture, and not a VB-Audio virtual device.
pub fn collect(microphone_id: Option<&str>, our_pid: u32) -> Result<DoctorFacts, Error> {
    let active = devices::endpoints()?;
    let installed = devices::installed_endpoints()?;
    let mut selected = Vec::new();
    if let Some(cable) = devices::find_vb_cable(&active) {
        selected.extend([cable.input, cable.output]);
    } else {
        selected.extend(active.iter().filter(|e| e.is_vb_cable()).cloned());
    }
    if let Some(id) = microphone_id {
        selected.push(
            active
                .iter()
                .find(|e| e.id == id && e.flow == DataFlow::Capture && !e.is_vb_audio())
                .cloned()
                .ok_or_else(|| Error::DeviceNotFound(id.into()))?,
        );
    }
    let render_defaults = crate::policy::defaults(DataFlow::Render)?;
    Ok(DoctorFacts {
        cable: classify_cable(&installed, &active),
        endpoints: selected
            .into_iter()
            .map(|endpoint| EndpointFacts {
                mix_format: mix_format(&endpoint.id),
                device_format: device_format(&endpoint.id),
                engine_periods: wasapi::engine_periods(&endpoint.id),
                endpoint,
            })
            .collect(),
        foreign_writers: foreign_writers(&sessions::sessions()?, &active, our_pid)
            .into_iter()
            .cloned()
            .collect(),
        cable_default_roles: cable_default_roles(&render_defaults, &installed),
        render_defaults,
        latency_recommendation: CABLE_LATENCY_RECOMMENDATION,
        format_settings: FORMAT_SETTINGS,
    })
}

fn mix_format(id: &str) -> Result<AudioFormat, Error> {
    let device = devices::device_by_id(id)?;
    // SAFETY: COM is initialised; GetMixFormat owns a valid WAVEFORMATEX allocation,
    // copied before CoTaskMemFree. No borrowed pointer escapes.
    unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let raw = client.GetMixFormat()?;
        let format = AudioFormat::from(raw.read_unaligned());
        CoTaskMemFree(Some(raw.cast()));
        Ok(format)
    }
}

fn device_format(id: &str) -> Result<AudioFormat, Error> {
    let device = devices::device_by_id(id)?;
    // SAFETY: the property variant owns the blob until it drops; type, size and pointer
    // are checked before reading an unaligned WAVEFORMATEX header. No pointer escapes.
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ)?;
        let value = store.GetValue(&PKEY_AudioEngine_DeviceFormat)?;
        let inner = &value.Anonymous.Anonymous;
        if inner.vt != VT_BLOB {
            return Err(Error::Format("device format is not a blob".into()));
        }
        let blob = inner.Anonymous.blob;
        if blob.pBlobData.is_null() || (blob.cbSize as usize) < std::mem::size_of::<WAVEFORMATEX>()
        {
            return Err(Error::Format("device format blob is truncated".into()));
        }
        Ok(AudioFormat::from(
            blob.pBlobData.cast::<WAVEFORMATEX>().read_unaligned(),
        ))
    }
}

/// One-shot correction: prefer an active non-VB-Audio journaled original (role order),
/// otherwise the first active render endpoint on a recognised physical bus. Unknown/root
/// software adapters are not automatic fallbacks. No change if no role uses
/// the base cable. All writes use the default journal; caller restores on Quit/start.
/// Returns the chosen ID, or None when no correction was needed.
pub fn fix_cable_default(
    defaults: &mut impl Defaults,
    path: &Path,
    active: &[Endpoint],
) -> Result<Option<String>, Error> {
    let mut current = [None, None, None];
    for (i, role) in EndpointRole::ALL.into_iter().enumerate() {
        current[i] = defaults.get(DataFlow::Render, role)?;
    }
    if cable_default_roles(&current, active).is_empty() {
        return Ok(None);
    }
    let physical: Vec<_> = active
        .iter()
        .filter(|e| e.flow == DataFlow::Render && !e.is_vb_audio())
        .collect();
    let chosen = EndpointRole::ALL
        .into_iter()
        .filter_map(|role| journal::previous(path, DataFlow::Render, role))
        .find(|id| physical.iter().any(|e| e.id == *id))
        .or_else(|| {
            physical
                .iter()
                .find(|e| physical_bus(e))
                .map(|e| e.id.clone())
        })
        .ok_or_else(|| {
            Error::DeviceNotFound("no physical render endpoint for cable default correction".into())
        })?;
    journal::set_default(defaults, path, DataFlow::Render, &chosen)?;
    Ok(Some(chosen))
}

fn physical_bus(endpoint: &Endpoint) -> bool {
    // Conservative fallback only: explicitly journaled originals may be software mixers.
    let known_bus = |id: &str| {
        let id = id.to_ascii_uppercase();
        [
            "USB\\",
            "HDAUDIO\\",
            "INTELAUDIO\\",
            "PCI\\",
            "BTHENUM\\",
            "BTHHFENUM\\",
            "BTHA2DP\\",
            "ACPI\\",
        ]
        .iter()
        .any(|prefix| id.starts_with(prefix))
    };
    endpoint
        .adapter_instance_id
        .as_deref()
        .is_some_and(known_bus)
        || endpoint.hardware_ids.iter().any(|id| known_bus(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn endpoint(id: &str, flow: DataFlow, hw: &str) -> Endpoint {
        Endpoint {
            id: id.into(),
            name: "renamed".into(),
            flow,
            adapter_instance_id: None,
            hardware_ids: vec![hw.into()],
            service: None,
            form_factor: None,
        }
    }
    #[test]
    fn classifies_inactive_cable_other_products_and_format_flags() {
        let inventory = [
            endpoint("cable", DataFlow::Render, devices::VB_CABLE_HARDWARE_ID),
            endpoint("other", DataFlow::Render, "VBAudioVACAWDM"),
        ];
        let state = classify_cable(&inventory, &[]);
        assert!(state.installed);
        assert!(state.active_endpoints.is_empty());
        assert_eq!(state.other_vb_audio.len(), 1);
        assert!(!classify_cable(&inventory[1..], &[]).installed);
        for (rate_hz, expected) in [(48_000, false), (44_100, true), (96_000, true)] {
            assert_eq!(
                AudioFormat {
                    rate_hz,
                    channels: 2,
                    bits: 32
                }
                .needs_resampling(),
                expected
            );
        }
        assert_eq!(
            cable_default_roles(
                &[None, Some("cable".into()), Some("other".into())],
                &inventory
            ),
            vec![EndpointRole::Multimedia]
        );
    }
    #[test]
    fn foreign_sessions_require_active_cable_stream_but_not_an_audible_peak() {
        let active = [endpoint(
            "cable",
            DataFlow::Render,
            devices::VB_CABLE_HARDWARE_ID,
        )];
        let session = |pid, active, endpoint: &str| Session {
            endpoint_id: endpoint.into(),
            process_id: pid,
            executable: None,
            display_name: String::new(),
            system_sounds: pid == 0,
            active,
            volume: 0.0,
            muted: true,
            peak: 0.0,
        };
        let list = [
            session(42, true, "cable"),
            session(7, true, "cable"),
            session(0, true, "cable"),
            session(9, false, "cable"),
            session(9, true, "speakers"),
        ];
        assert_eq!(
            foreign_writers(&list, &active, 42)
                .iter()
                .map(|s| s.process_id)
                .collect::<Vec<_>>(),
            vec![7, 0]
        );
    }
    #[test]
    fn correction_prefers_journaled_physical_original_and_is_restorable() {
        struct Fake([Option<String>; 3]);
        fn index(role: EndpointRole) -> usize {
            match role {
                EndpointRole::Console => 0,
                EndpointRole::Multimedia => 1,
                EndpointRole::Communications => 2,
            }
        }
        impl Defaults for Fake {
            fn get(&self, _: DataFlow, role: EndpointRole) -> Result<Option<String>, Error> {
                Ok(self.0[index(role)].clone())
            }
            fn set(&mut self, id: &str, role: EndpointRole) -> Result<(), Error> {
                self.0[index(role)] = Some(id.into());
                Ok(())
            }
        }
        let path = std::env::temp_dir().join(format!("macafon-doctor-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let active = [
            endpoint("virtual", DataFlow::Render, "VBAudioVACAWDM"),
            endpoint("software", DataFlow::Render, r"ROOT\SoftwareMixer"),
            endpoint("first", DataFlow::Render, r"USB\VID_1234"),
            endpoint("original", DataFlow::Render, r"USB\VID_1234"),
            endpoint("cable", DataFlow::Render, devices::VB_CABLE_HARDWARE_ID),
        ];
        let mut fake = Fake(std::array::from_fn(|_| Some("original".into())));
        journal::set_default(&mut fake, &path, DataFlow::Render, "cable").expect("set cable");
        assert_eq!(
            fix_cable_default(&mut fake, &path, &active).expect("fix"),
            Some("original".into())
        );
        assert_eq!(
            fix_cable_default(&mut fake, &path, &active).expect("no-op"),
            None
        );
        journal::restore(&mut fake, &path).expect("restore");
        fake.0 = std::array::from_fn(|_| Some("cable".into()));
        assert_eq!(
            fix_cable_default(&mut fake, &path, &active).expect("fallback"),
            Some("first".into())
        );
        journal::restore(&mut fake, &path).expect("restore baseline");
        assert_eq!(fake.0, std::array::from_fn(|_| Some("cable".into())));
        assert!(fix_cable_default(&mut fake, &path, &active[4..]).is_err());
    }
}
