//! Physical input level probes: exist only while the device picker is subscribed.

use crate::Error;
use crate::filter::audio_format;
use pipewire::{self as pw, properties::properties};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use std::{ffi::c_void, ptr::NonNull};

const SAMPLE_BYTES: usize = std::mem::size_of::<f32>();

struct RtState {
    stream: *mut pw::sys::pw_stream,
    peak: Arc<AtomicU32>,
}

pub(crate) struct Meter {
    pointer: NonNull<pw::sys::pw_stream>,
    peak: Arc<AtomicU32>,
    links: Vec<pw::link::Link>,
    _state: Box<RtState>,
    _events: Box<pw::sys::pw_stream_events>,
    _hook: Box<pw::spa::sys::spa_hook>,
}

impl Meter {
    /// Connect a raw input probe to this physical node only.
    pub fn new(core: &pw::core::CoreRc, target: &str) -> Result<Self, Error> {
        let mut props = properties! {
            "node.name" => "macafon_input_meter",
            "node.description" => "Macafon input meter",
            "application.name" => "Macafon",
            "media.type" => "Audio",
            "media.category" => "Capture",
            "media.role" => "Test",
            "node.rate" => "1/48000",
            "node.autoconnect" => "false",
            "node.latency" => "256/48000",
            "audio.channels" => "2",
            "audio.position" => "[ FL FR ]",
            "adapter.auto-port-config" => "{ mode=dsp position=preserve }",
        };
        props.insert("target.object", target);
        props.insert("node.dont-reconnect", "true");
        // SAFETY: core is alive; properties ownership is transferred to PipeWire.
        let pointer = unsafe {
            pw::sys::pw_stream_new(
                core.as_raw_ptr(),
                c"Macafon input meter".as_ptr(),
                props.into_raw(),
            )
        };
        let pointer = NonNull::new(pointer)
            .ok_or_else(|| Error::Message("Cannot create pw_stream".into()))?;
        let peak = Arc::new(AtomicU32::new(0));
        let mut state = Box::new(RtState {
            stream: pointer.as_ptr(),
            peak: peak.clone(),
        });
        // SAFETY: pw_stream_events holds integers and optional function pointers; zero is valid.
        let mut events: Box<pw::sys::pw_stream_events> = Box::new(unsafe { std::mem::zeroed() });
        events.version = pw::sys::PW_VERSION_STREAM_EVENTS;
        events.process = Some(process);
        // SAFETY: a zeroed hook is the documented initial listener state.
        let mut hook: Box<pw::spa::sys::spa_hook> = Box::new(unsafe { std::mem::zeroed() });
        // SAFETY: state, events and hook have stable heap addresses and outlive the stream;
        // destroying the stream stops callbacks before they are freed.
        unsafe {
            pw::sys::pw_stream_add_listener(
                pointer.as_ptr(),
                &mut *hook,
                &*events,
                (&mut *state as *mut RtState).cast(),
            );
        }
        let player = Self {
            peak,
            links: Vec::new(),
            pointer,
            _state: state,
            _events: events,
            _hook: hook,
        };
        let format = audio_format(pw::spa::sys::SPA_AUDIO_FORMAT_F32, true)?;
        let pod = pw::spa::pod::Pod::from_bytes(&format)
            .ok_or_else(|| Error::Message("Invalid input meter format pod".into()))?;
        let mut params = [pod.as_raw_ptr().cast_const()];
        // SAFETY: the stream and listener are set up; the pod outlives the call.
        let result = unsafe {
            pw::sys::pw_stream_connect(
                player.pointer.as_ptr(),
                pw::spa::sys::SPA_DIRECTION_INPUT,
                pw::sys::PW_ID_ANY,
                pw::sys::pw_stream_flags_PW_STREAM_FLAG_MAP_BUFFERS
                    | pw::sys::pw_stream_flags_PW_STREAM_FLAG_RT_PROCESS,
                params.as_mut_ptr(),
                params.len() as u32,
            )
        };
        if result < 0 {
            return Err(Error::Message(format!(
                "Cannot connect input meter stream: {result}"
            )));
        }
        Ok(player)
    }
}

impl Meter {
    pub fn connect_source(
        &mut self,
        core: &pw::core::CoreRc,
        source: &crate::Node,
        ports: &[crate::Port],
    ) -> Result<(), Error> {
        if !self.links.is_empty() {
            return Ok(());
        }
        // SAFETY: the stream remains alive throughout this control-loop call.
        let node_id = unsafe { pw::sys::pw_stream_get_node_id(self.pointer.as_ptr()) };
        let outputs: Vec<_> = ports
            .iter()
            .filter(|port| port.node_id == source.id && port.direction == "out")
            .collect();
        let inputs: Vec<_> = ports
            .iter()
            .filter(|port| port.node_id == node_id && port.direction == "in")
            .collect();
        if outputs.is_empty() || inputs.is_empty() {
            return Ok(());
        }
        for input in inputs {
            let output = outputs
                .iter()
                .find(|port| port.channel == input.channel)
                .copied()
                .unwrap_or(outputs[0]);
            self.links.push(core.create_object::<pw::link::Link>(
                "link-factory",
                &properties! {
                    "link.output.node" => source.id.to_string(),
                    "link.output.port" => output.id.to_string(),
                    "link.input.node" => node_id.to_string(),
                    "link.input.port" => input.id.to_string(),
                    "object.linger" => "false",
                    "link.passive" => "false",
                },
            )?);
        }
        Ok(())
    }

    pub fn read_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        // SAFETY: uniquely owned stream; destruction stops callbacks before state is dropped.
        unsafe { pw::sys::pw_stream_destroy(self.pointer.as_ptr()) };
    }
}

unsafe extern "C" fn process(data: *mut c_void) {
    assert_no_alloc::assert_no_alloc(|| {
        // SAFETY: PipeWire passes the RtState registered with this listener.
        unsafe { process_inner(data) }
    });
}

unsafe fn process_inner(data: *mut c_void) {
    if data.is_null() {
        return;
    }
    // SAFETY: data is the RtState of a live stream; PipeWire serializes process callbacks
    // on the data loop, so this is the only reference.
    let state = unsafe { &mut *data.cast::<RtState>() };
    // SAFETY: dequeue/queue are RT-safe and the stream outlives its callbacks.
    let buffer = unsafe { pw::sys::pw_stream_dequeue_buffer(state.stream) };
    if buffer.is_null() {
        return;
    }
    // SAFETY: a dequeued buffer stays ours until queued; MAP_BUFFERS maps its data and
    // PipeWire guarantees maxsize bytes behind it.
    unsafe {
        let spa = (*buffer).buffer;
        if !spa.is_null() && (*spa).n_datas > 0 {
            let first = &mut *(*spa).datas;
            if !first.data.is_null() && !first.chunk.is_null() {
                let chunk = &*first.chunk;
                let offset = (chunk.offset as usize).min(first.maxsize as usize);
                let bytes = (chunk.size as usize).min(first.maxsize as usize - offset);
                let samples = std::slice::from_raw_parts(
                    first.data.cast::<u8>().add(offset).cast::<f32>(),
                    bytes / SAMPLE_BYTES,
                );
                let peak = samples
                    .iter()
                    .copied()
                    .filter(|sample| sample.is_finite())
                    .map(f32::abs)
                    .fold(0.0_f32, f32::max);
                state.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
            }
        }
        pw::sys::pw_stream_queue_buffer(state.stream, buffer);
    }
}
