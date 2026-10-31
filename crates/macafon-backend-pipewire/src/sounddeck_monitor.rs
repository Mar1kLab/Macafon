//! SoundDeck monitor (PRD §7.6): a stereo playback stream that plays the SoundDeck into the
//! user's headphones. On the default output it is an ordinary app named "SoundDeck", so the
//! mixer routes it through a channel with its EQ; a chosen output device pins it there.

use crate::Error;
use crate::filter::audio_format;
use macafon_backend::SoundDeckTap;
use pipewire::{self as pw, properties::properties};
use std::{ffi::c_void, ptr::NonNull};

/// `node.name` of the monitor stream; the mixer shows it as its own app (SND-002).
pub const SOUNDDECK_NODE: &str = macafon_backend::SOUNDDECK_MONITOR_NODE;
/// App name the monitor stream is identified by in the mixer.
pub const SOUNDDECK_APP: &str = "SoundDeck";

const CHANNELS: usize = 2;
const FRAME_BYTES: usize = CHANNELS * std::mem::size_of::<f32>();

struct RtState {
    stream: *mut pw::sys::pw_stream,
    tap: SoundDeckTap,
}

pub(crate) struct Monitor {
    pointer: NonNull<pw::sys::pw_stream>,
    /// The output it was asked for: `None` follows the default output.
    pub target: Option<String>,
    _state: Box<RtState>,
    _events: Box<pw::sys::pw_stream_events>,
    _hook: Box<pw::spa::sys::spa_hook>,
}

impl Monitor {
    /// Create and connect the stream; `target` is a sink `node.name`, `None` for the default.
    pub fn new(
        core: &pw::core::CoreRc,
        tap: SoundDeckTap,
        target: Option<String>,
    ) -> Result<Self, Error> {
        let mut props = properties! {
            "node.name" => SOUNDDECK_NODE,
            "node.description" => "Macafon SoundDeck",
            "application.name" => SOUNDDECK_APP,
            "media.name" => "SoundDeck",
            "media.type" => "Audio",
            "media.category" => "Playback",
            "media.role" => "Music",
            "node.rate" => "1/48000",
            "node.latency" => "256/48000",
        };
        if let Some(target) = &target {
            // A chosen device wins over channel routing (the mixer skips unmovable streams).
            props.insert("target.object", target.as_str());
            props.insert("node.dont-move", "true");
        }
        // SAFETY: core is alive; properties ownership is transferred to PipeWire.
        let pointer = unsafe {
            pw::sys::pw_stream_new(
                core.as_raw_ptr(),
                c"Macafon SoundDeck".as_ptr(),
                props.into_raw(),
            )
        };
        let pointer = NonNull::new(pointer)
            .ok_or_else(|| Error::Message("Cannot create pw_stream".into()))?;
        let mut state = Box::new(RtState {
            stream: pointer.as_ptr(),
            tap,
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
        let monitor = Self {
            pointer,
            target,
            _state: state,
            _events: events,
            _hook: hook,
        };
        let format = audio_format(pw::spa::sys::SPA_AUDIO_FORMAT_F32, true)?;
        let pod = pw::spa::pod::Pod::from_bytes(&format)
            .ok_or_else(|| Error::Message("Invalid SoundDeck format pod".into()))?;
        let mut params = [pod.as_raw_ptr().cast_const()];
        // SAFETY: the stream and listener are set up; the pod outlives the call.
        let result = unsafe {
            pw::sys::pw_stream_connect(
                monitor.pointer.as_ptr(),
                pw::spa::sys::SPA_DIRECTION_OUTPUT,
                pw::sys::PW_ID_ANY,
                pw::sys::pw_stream_flags_PW_STREAM_FLAG_AUTOCONNECT
                    | pw::sys::pw_stream_flags_PW_STREAM_FLAG_MAP_BUFFERS
                    | pw::sys::pw_stream_flags_PW_STREAM_FLAG_RT_PROCESS,
                params.as_mut_ptr(),
                params.len() as u32,
            )
        };
        if result < 0 {
            return Err(Error::Message(format!(
                "Cannot connect the SoundDeck monitor: {result}"
            )));
        }
        Ok(monitor)
    }

    /// The stream failed or was disconnected by the server; the control loop reopens it.
    pub fn failed(&self) -> bool {
        // SAFETY: the stream is alive while self is.
        let state =
            unsafe { pw::sys::pw_stream_get_state(self.pointer.as_ptr(), std::ptr::null_mut()) };
        matches!(
            state,
            pw::sys::pw_stream_state_PW_STREAM_STATE_ERROR
                | pw::sys::pw_stream_state_PW_STREAM_STATE_UNCONNECTED
        )
    }
}

impl Drop for Monitor {
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
                let capacity = first.maxsize as usize / FRAME_BYTES;
                let requested = (*buffer).requested as usize;
                let frames = if requested == 0 {
                    capacity
                } else {
                    requested.min(capacity)
                };
                let samples =
                    std::slice::from_raw_parts_mut(first.data.cast::<f32>(), frames * CHANNELS);
                samples.fill(0.0);
                state.tap.mix(samples, CHANNELS);
                let chunk = &mut *first.chunk;
                chunk.offset = 0;
                chunk.stride = FRAME_BYTES as i32;
                chunk.size = (frames * FRAME_BYTES) as u32;
            }
        }
        pw::sys::pw_stream_queue_buffer(state.stream, buffer);
    }
}
