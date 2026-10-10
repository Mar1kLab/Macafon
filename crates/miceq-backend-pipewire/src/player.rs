//! Playback stream of the A/B test (AB-002): mono 48 kHz into the listening device.

use crate::Error;
use crate::filter::mono_format;
use miceq_engine::AbRt;
use pipewire::{self as pw, properties::properties};
use std::{ffi::c_void, ptr::NonNull};

const SAMPLE_BYTES: usize = std::mem::size_of::<f32>();

struct RtState {
    stream: *mut pw::sys::pw_stream,
    player: AbRt,
}

pub(crate) struct Player {
    pointer: NonNull<pw::sys::pw_stream>,
    _state: Box<RtState>,
    _events: Box<pw::sys::pw_stream_events>,
    _hook: Box<pw::spa::sys::spa_hook>,
}

impl Player {
    /// Create and connect the stream; `target` is a sink `node.name`, `None` for the default.
    pub fn new(core: &pw::core::CoreRc, player: AbRt, target: Option<&str>) -> Result<Self, Error> {
        let mut props = properties! {
            "node.name" => "miceq_ab",
            "node.description" => "micEQ A/B test",
            "application.name" => "micEQ",
            "media.type" => "Audio",
            "media.category" => "Playback",
            "media.role" => "Test",
            "node.rate" => "1/48000",
            "node.latency" => "256/48000",
        };
        if let Some(target) = target {
            props.insert("target.object", target);
        }
        // SAFETY: core is alive; properties ownership is transferred to PipeWire.
        let pointer = unsafe {
            pw::sys::pw_stream_new(
                core.as_raw_ptr(),
                c"micEQ A/B test".as_ptr(),
                props.into_raw(),
            )
        };
        let pointer = NonNull::new(pointer)
            .ok_or_else(|| Error::Message("Cannot create pw_stream".into()))?;
        let mut state = Box::new(RtState {
            stream: pointer.as_ptr(),
            player,
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
            pointer,
            _state: state,
            _events: events,
            _hook: hook,
        };
        let format = mono_format(pw::spa::sys::SPA_AUDIO_FORMAT_F32)?;
        let pod = pw::spa::pod::Pod::from_bytes(&format)
            .ok_or_else(|| Error::Message("Invalid playback format pod".into()))?;
        let mut params = [pod.as_raw_ptr().cast_const()];
        // SAFETY: the stream and listener are set up; the pod outlives the call.
        let result = unsafe {
            pw::sys::pw_stream_connect(
                player.pointer.as_ptr(),
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
                "Cannot connect A/B stream: {result}"
            )));
        }
        Ok(player)
    }
}

impl Drop for Player {
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
                let capacity = first.maxsize as usize / SAMPLE_BYTES;
                let requested = (*buffer).requested as usize;
                let frames = if requested == 0 {
                    capacity
                } else {
                    requested.min(capacity)
                };
                let samples = std::slice::from_raw_parts_mut(first.data.cast::<f32>(), frames);
                state.player.process(samples);
                let chunk = &mut *first.chunk;
                chunk.offset = 0;
                chunk.stride = SAMPLE_BYTES as i32;
                chunk.size = (frames * SAMPLE_BYTES) as u32;
            }
        }
        pw::sys::pw_stream_queue_buffer(state.stream, buffer);
    }
}
