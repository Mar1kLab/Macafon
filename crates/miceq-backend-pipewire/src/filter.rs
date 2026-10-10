use crate::{Connection, Error};
use miceq_engine::MicRt;
use pipewire::{self as pw, properties::properties};
use std::{
    ffi::c_void,
    ptr::{self, NonNull},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering},
    },
};

/// Unlinked output ports have no buffer; the chain then runs on this scratch space.
const SCRATCH_SAMPLES: usize = 8192;

/// Separate port storage keeps graph notifications independent of mutable DSP state.
struct PortState {
    output: bool,
    io: AtomicPtr<pw::spa::sys::spa_io_buffers>,
}

struct RtState {
    input: *mut c_void,
    output: *mut c_void,
    output_right: *mut c_void,
    processor: MicRt,
    graph_rate_hz: Arc<AtomicU32>,
    keep_processing: Arc<AtomicBool>,
    scratch: Box<[f32]>,
}

struct ControlState {
    filter: *mut pw::sys::pw_filter,
    stereo: bool,
}

pub(crate) struct Filter {
    pointer: NonNull<pw::sys::pw_filter>,
    state: Box<RtState>,
    _events: Box<pw::sys::pw_filter_events>,
    hook: Box<pw::spa::sys::spa_hook>,
    _control_events: Box<pw::sys::pw_filter_events>,
    control_hook: Box<pw::spa::sys::spa_hook>,
    control_state: Box<ControlState>,
    pub graph_rate_hz: Arc<AtomicU32>,
    keep_processing: Arc<AtomicBool>,
}

impl Filter {
    pub fn new(connection: &Connection, processor: MicRt, stereo: bool) -> Result<Self, Error> {
        let graph_rate_hz = Arc::new(AtomicU32::new(0));
        let keep_processing = Arc::new(AtomicBool::new(false));
        let state = Box::new(RtState {
            input: ptr::null_mut(),
            output: ptr::null_mut(),
            output_right: ptr::null_mut(),
            processor,
            graph_rate_hz: graph_rate_hz.clone(),
            keep_processing: keep_processing.clone(),
            scratch: vec![0.0; SCRATCH_SAMPLES].into_boxed_slice(),
        });
        // SAFETY: pw_filter_events is a C struct of integers and optional function pointers;
        // zero is a valid value for each field.
        let mut events: Box<pw::sys::pw_filter_events> = Box::new(unsafe { std::mem::zeroed() });
        events.version = pw::sys::PW_VERSION_FILTER_EVENTS;
        events.process = Some(process);
        let props = properties! {
            "node.name" => "miceq_mic",
            "node.description" => "micEQ Microphone",
            "media.type" => "Audio",
            "media.category" => "Filter",
            "media.role" => "Communication",
            "media.class" => "Audio/Source/Virtual",
            "node.virtual" => "true",
            "node.autoconnect" => "false",
            "node.want-driver" => "true",
            "node.pause-on-idle" => "false",
            "node.rate" => "1/48000",
            "node.latency" => "128/48000",
            "audio.rate" => "48000",
            "audio.channels" => if stereo { "2" } else { "1" },
            "audio.position" => if stereo { "[ FL FR ]" } else { "[ MONO ]" },
        };
        // SAFETY: core is alive; properties ownership is transferred to PipeWire.
        let pointer = unsafe {
            pw::sys::pw_filter_new(
                connection.core.as_raw_ptr(),
                c"micEQ Microphone".as_ptr(),
                props.into_raw(),
            )
        };
        let pointer = NonNull::new(pointer)
            .ok_or_else(|| Error::Message("Cannot create pw_filter".into()))?;
        // SAFETY: a zeroed spa_hook is the documented initial state for listener registration.
        let hook = Box::new(unsafe { std::mem::zeroed() });
        // SAFETY: the listener structs contain only integers and nullable pointers.
        let mut control_events: Box<pw::sys::pw_filter_events> =
            Box::new(unsafe { std::mem::zeroed() });
        control_events.version = pw::sys::PW_VERSION_FILTER_EVENTS;
        control_events.param_changed = Some(param_changed);
        control_events.io_changed = Some(io_changed);
        control_events.add_buffer = Some(add_buffer);
        // SAFETY: a zeroed hook is the documented initial listener state.
        let control_hook = Box::new(unsafe { std::mem::zeroed() });
        let mut filter = Self {
            pointer,
            state,
            _events: events,
            hook,
            graph_rate_hz,
            keep_processing,
            _control_events: control_events,
            control_hook,
            control_state: Box::new(ControlState {
                filter: pointer.as_ptr(),
                stereo,
            }),
        };
        // SAFETY: callback state/events have stable addresses and outlive the filter;
        // destroying the filter joins processing before their memory is freed.
        unsafe {
            pw::sys::pw_filter_add_listener(
                filter.pointer(),
                &mut *filter.hook,
                &*filter._events,
                (&mut *filter.state as *mut RtState).cast(),
            );
        }
        // SAFETY: this separate control listener accesses only immutable control state;
        // it never aliases the mutable DSP state used by the RT callback.
        unsafe {
            pw::sys::pw_filter_add_listener(
                filter.pointer(),
                &mut *filter.control_hook,
                &*filter._control_events,
                (&mut *filter.control_state as *mut ControlState).cast(),
            );
        }
        filter.state.input = filter.add_port(pw::sys::PW_DIRECTION_INPUT, "input_MONO", "MONO")?;
        filter.state.output = filter.add_port(
            pw::sys::PW_DIRECTION_OUTPUT,
            if stereo { "output_FL" } else { "output_MONO" },
            if stereo { "FL" } else { "MONO" },
        )?;
        if stereo {
            filter.state.output_right =
                filter.add_port(pw::sys::PW_DIRECTION_OUTPUT, "output_FR", "FR")?;
        }
        filter.connect()?;
        Ok(filter)
    }

    pub fn pointer(&self) -> *mut pw::sys::pw_filter {
        self.pointer.as_ptr()
    }

    fn add_port(&self, direction: u32, name: &str, channel: &str) -> Result<*mut c_void, Error> {
        let props = properties! {
            "format.dsp" => "32 bit float mono audio",
            "port.name" => name,
            "audio.channel" => channel,
            "port.group" => if direction == pw::sys::PW_DIRECTION_OUTPUT { "output" } else { "input" },
        };
        // SAFETY: the filter is alive and properties ownership transfers to PipeWire.
        let port = unsafe {
            pw::sys::pw_filter_add_port(
                self.pointer(),
                direction,
                pw::sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
                std::mem::size_of::<PortState>(),
                props.into_raw(),
                ptr::null_mut(),
                0,
            )
        };
        if port.is_null() {
            Err(Error::Message("Cannot create filter port".into()))
        } else {
            // SAFETY: PipeWire allocated suitably aligned user storage of this size;
            // graph IO/buffer notifications begin only after connect().
            unsafe {
                port.cast::<PortState>().write(PortState {
                    output: direction == pw::sys::PW_DIRECTION_OUTPUT,
                    io: AtomicPtr::new(ptr::null_mut()),
                });
            }
            Ok(port)
        }
    }

    /// Run the chain even when no application records the virtual microphone: for
    /// calibration, recording and meters. Off otherwise, so an unused microphone costs
    /// nothing (NFR-R7).
    pub fn set_keep_processing(&self, keep: bool) {
        self.keep_processing.store(keep, Ordering::Relaxed);
    }

    /// Report a new algorithmic latency to the graph (NFR-L4). Control loop only.
    pub fn set_latency(&self, samples: u32) -> Result<(), Error> {
        let bytes = latency_pod(samples)?;
        let pod = pw::spa::pod::Pod::from_bytes(&bytes)
            .ok_or_else(|| Error::Message("Invalid process latency pod".into()))?;
        let mut params = [pod.as_raw_ptr().cast_const()];
        // SAFETY: the filter is alive and this runs on the control loop; the pod outlives
        // the synchronous call.
        let result = unsafe {
            pw::sys::pw_filter_update_params(
                self.pointer(),
                ptr::null_mut(),
                params.as_mut_ptr(),
                params.len() as u32,
            )
        };
        if result < 0 {
            Err(Error::Message(format!("Cannot update latency: {result}")))
        } else {
            Ok(())
        }
    }

    fn connect(&self) -> Result<(), Error> {
        use pw::spa::{pod::Pod, sys};
        let bytes = latency_pod(self.state.processor.latency_samples())?;
        let pod = Pod::from_bytes(&bytes)
            .ok_or_else(|| Error::Message("Invalid process latency pod".into()))?;
        // WirePlumber discovers devices through node EnumFormat, while our ports
        // retain their fixed DSP format. Advertise the selected graph format.
        let format_bytes = audio_format(sys::SPA_AUDIO_FORMAT_F32P, self.control_state.stereo)?;
        let format_pod = Pod::from_bytes(&format_bytes)
            .ok_or_else(|| Error::Message("Invalid microphone format pod".into()))?;
        let mut params = [
            pod.as_raw_ptr().cast_const(),
            format_pod.as_raw_ptr().cast_const(),
        ];
        // SAFETY: all ports and callback storage are prepared before enabling RT processing.
        let result = unsafe {
            pw::sys::pw_filter_connect(
                self.pointer(),
                pw::sys::pw_filter_flags_PW_FILTER_FLAG_RT_PROCESS,
                params.as_mut_ptr(),
                params.len() as u32,
            )
        };
        if result < 0 {
            Err(Error::Message(format!(
                "Cannot connect pw_filter: {result}"
            )))
        } else {
            Ok(())
        }
    }
}

/// `EnumFormat` for mono 48 kHz audio in the given SPA sample format.
pub(crate) fn mono_format(sample_format: u32) -> Result<Vec<u8>, Error> {
    audio_format(sample_format, false)
}

fn channel_positions(stereo: bool) -> Vec<pw::spa::utils::Id> {
    use pw::spa::{sys, utils::Id};
    if stereo {
        vec![Id(sys::SPA_AUDIO_CHANNEL_FL), Id(sys::SPA_AUDIO_CHANNEL_FR)]
    } else {
        vec![Id(sys::SPA_AUDIO_CHANNEL_MONO)]
    }
}

fn audio_format(sample_format: u32, stereo: bool) -> Result<Vec<u8>, Error> {
    use pw::spa::{
        pod::{Object, Property, PropertyFlags, Value, serialize::PodSerializer},
        sys,
    };
    let format = Value::Object(Object {
        type_: sys::SPA_TYPE_OBJECT_Format,
        id: sys::SPA_PARAM_EnumFormat,
        properties: [
            (
                sys::SPA_FORMAT_mediaType,
                Value::Id(pw::spa::utils::Id(sys::SPA_MEDIA_TYPE_audio)),
            ),
            (
                sys::SPA_FORMAT_mediaSubtype,
                Value::Id(pw::spa::utils::Id(sys::SPA_MEDIA_SUBTYPE_raw)),
            ),
            (
                sys::SPA_FORMAT_AUDIO_format,
                Value::Id(pw::spa::utils::Id(sample_format)),
            ),
            (
                sys::SPA_FORMAT_AUDIO_rate,
                Value::Int(miceq_dsp::SAMPLE_RATE_HZ as i32),
            ),
            (
                sys::SPA_FORMAT_AUDIO_channels,
                Value::Int(if stereo { 2 } else { 1 }),
            ),
            (
                sys::SPA_FORMAT_AUDIO_position,
                Value::ValueArray(pw::spa::pod::ValueArray::Id(channel_positions(stereo))),
            ),
        ]
        .into_iter()
        .map(|(key, value)| Property {
            key,
            value,
            flags: PropertyFlags::empty(),
        })
        .collect(),
    });
    Ok(
        PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &format)
            .map_err(|error| Error::Message(format!("Cannot serialize audio format: {error:?}")))?
            .0
            .into_inner(),
    )
}

fn latency_pod(samples: u32) -> Result<Vec<u8>, Error> {
    use pw::spa::{
        pod::{Object, Property, PropertyFlags, Value, serialize::PodSerializer},
        sys,
    };
    let latency = Value::Object(Object {
        type_: sys::SPA_TYPE_OBJECT_ParamProcessLatency,
        id: sys::SPA_PARAM_ProcessLatency,
        properties: vec![Property {
            key: sys::SPA_PARAM_PROCESS_LATENCY_rate,
            flags: PropertyFlags::empty(),
            value: Value::Int(samples as i32),
        }],
    });
    Ok(
        PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &latency)
            .map_err(|error| {
                Error::Message(format!("Cannot serialize process latency: {error:?}"))
            })?
            .0
            .into_inner(),
    )
}

impl Drop for Filter {
    fn drop(&mut self) {
        // SAFETY: uniquely owned filter; destruction stops callbacks before dropping state.
        unsafe {
            pw::sys::pw_filter_destroy(self.pointer());
        }
    }
}

unsafe extern "C" fn process(data: *mut c_void, position: *mut pw::spa::sys::spa_io_position) {
    assert_no_alloc::assert_no_alloc(|| {
        // SAFETY: PipeWire supplies callback arguments as described by process_inner.
        unsafe {
            process_inner(data, position);
        }
    });
}

unsafe fn process_inner(data: *mut c_void, position: *mut pw::spa::sys::spa_io_position) {
    if data.is_null() || position.is_null() {
        return;
    }
    // SAFETY: PipeWire serializes this callback on its data loop; data points to the
    // prepared RtState and position is valid for the duration of this invocation.
    let (state, position) = unsafe { (&mut *data.cast::<RtState>(), &*position) };
    state
        .graph_rate_hz
        .store(position.clock.rate.denom, Ordering::Relaxed);
    let Ok(count) = u32::try_from(position.clock.duration) else {
        return;
    };
    if count == 0 {
        return;
    }
    // SAFETY: ports are alive; PipeWire supplies negotiated DSP buffers for the graph quantum.
    let (input, left, right) = unsafe {
        (
            pw::sys::pw_filter_get_dsp_buffer(state.input, count).cast::<f32>(),
            pw::sys::pw_filter_get_dsp_buffer(state.output, count).cast::<f32>(),
            if state.output_right.is_null() {
                ptr::null_mut()
            } else {
                pw::sys::pw_filter_get_dsp_buffer(state.output_right, count).cast::<f32>()
            },
        )
    };
    let supported_rate =
        position.clock.rate.num == 1 && position.clock.rate.denom == miceq_dsp::SAMPLE_RATE_HZ;
    if left.is_null() && right.is_null() {
        // Nothing records the virtual microphone, so the graph gives us no output buffer.
        // SAFETY: a linked input provides count readable samples; scratch is prepared.
        unsafe {
            process_unlinked(
                &mut state.processor,
                input,
                count as usize,
                &mut state.scratch,
                state.keep_processing.load(Ordering::Relaxed) && supported_rate,
            );
        }
        return;
    }
    // SAFETY: linked ports provide count readable/writable samples. The helper handles
    // absent ports and possible buffer aliasing without creating overlapping slices.
    unsafe {
        process_outputs(
            &mut state.processor,
            input,
            left,
            right,
            count as usize,
            supported_rate,
        );
    }
}

/// Keep meters and capture running without output buffers, only at the supported graph rate.
unsafe fn process_unlinked(
    processor: &mut MicRt,
    input: *const f32,
    count: usize,
    scratch: &mut [f32],
    keep_processing: bool,
) {
    if input.is_null() || !keep_processing || scratch.is_empty() {
        return;
    }
    // SAFETY: callers provide count readable samples on a non-null input pointer.
    let input = unsafe { std::slice::from_raw_parts(input, count) };
    for chunk in input.chunks(scratch.len()) {
        let output = &mut scratch[..chunk.len()];
        output.copy_from_slice(chunk);
        processor.process(output);
    }
}

/// Process once into whichever output is linked, then duplicate into the other channel.
unsafe fn process_outputs(
    processor: &mut MicRt,
    input: *const f32,
    left: *mut f32,
    right: *mut f32,
    count: usize,
    supported_rate: bool,
) {
    let (primary, secondary) = if left.is_null() {
        (right, left)
    } else {
        (left, right)
    };
    if primary.is_null() {
        return;
    }
    // SAFETY: callers provide count samples on each non-null pointer; ptr::copy permits
    // input/output aliasing, and the mutable slice ends before copying to the second port.
    unsafe {
        if input.is_null() {
            ptr::write_bytes(primary, 0, count);
        } else {
            ptr::copy(input, primary, count);
        }
        let output = std::slice::from_raw_parts_mut(primary, count);
        if supported_rate {
            processor.process(output);
        }
        if !supported_rate || input.is_null() {
            output.fill(0.0);
        }
        if !secondary.is_null() {
            ptr::copy(primary, secondary, count);
        }
    }
}

/// Buffer replacement seeds the output free queue with every new buffer. Leaving
/// an old IO buffer ID in place would recycle that ID a second time on the next
/// callback, so two dequeues could return the same memory before async readers finish.
unsafe fn reset_output_io(state: &PortState) {
    let io = state.io.load(Ordering::Acquire);
    if state.output && !io.is_null() {
        // SAFETY: IO/buffer notifications run while graph processing is quiescent.
        // The current IO area belongs to this live port; old buffer IDs are invalid
        // once its buffer set is replaced. DSP state is never accessed here.
        unsafe {
            (*io).buffer_id = pw::spa::sys::SPA_ID_INVALID;
            (*io).status = pw::spa::sys::SPA_STATUS_NEED_DATA as i32;
        }
    }
}

unsafe extern "C" fn io_changed(
    _data: *mut c_void,
    port: *mut c_void,
    id: u32,
    area: *mut c_void,
    size: u32,
) {
    if port.is_null() || id != pw::spa::sys::SPA_IO_Buffers {
        return;
    }
    let io = if size as usize >= std::mem::size_of::<pw::spa::sys::spa_io_buffers>() {
        area.cast()
    } else {
        ptr::null_mut()
    };
    // SAFETY: live port user storage is initialized before connecting the graph.
    let state = unsafe { &*port.cast::<PortState>() };
    state.io.store(io, Ordering::Release);
    // SAFETY: a replaced IO area must request fresh output from this port.
    unsafe {
        reset_output_io(state);
    }
}

unsafe extern "C" fn add_buffer(
    _data: *mut c_void,
    port: *mut c_void,
    _buffer: *mut pw::sys::pw_buffer,
) {
    if port.is_null() {
        return;
    }
    // SAFETY: buffer notifications use the live initialized port storage and run
    // while the graph is quiescent, before these buffers can reach process().
    unsafe {
        reset_output_io(&*port.cast::<PortState>());
    }
}

// PipeWire invokes parameter notifications on the control loop, never in process().
unsafe extern "C" fn param_changed(
    data: *mut c_void,
    port: *mut c_void,
    id: u32,
    param: *const pw::spa::sys::spa_pod,
) {
    if data.is_null() || !port.is_null() || id != pw::spa::sys::SPA_PARAM_Format {
        return;
    }
    use pw::spa::pod::{
        Object, Pod, Property, PropertyFlags, Value, ValueArray, serialize::PodSerializer,
    };
    // SAFETY: the control listener holds this immutable state until filter destruction.
    let state = unsafe { &*data.cast::<ControlState>() };
    // pw_filter has fixed DSP ports and no node PortConfig parameter. Record
    // the negotiated Format and notify Props after it, allowing WirePlumber
    // to finish its adapter activation using the ports already present.
    let props = Value::Object(Object {
        type_: pw::spa::sys::SPA_TYPE_OBJECT_Props,
        id: pw::spa::sys::SPA_PARAM_Props,
        properties: vec![Property {
            key: pw::spa::sys::SPA_PROP_channelMap,
            flags: PropertyFlags::empty(),
            value: Value::ValueArray(ValueArray::Id(channel_positions(state.stereo))),
        }],
    });
    let Ok((bytes, _)) = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &props) else {
        return;
    };
    let bytes = bytes.into_inner();
    let Some(pod) = Pod::from_bytes(&bytes) else {
        return;
    };
    let mut params = [param, pod.as_raw_ptr().cast_const()];
    // SAFETY: the control state's filter pointer is live, and the pod remains
    // valid during this synchronous control-loop call. No DSP state is accessed.
    unsafe {
        pw::sys::pw_filter_update_params(
            state.filter,
            ptr::null_mut(),
            params.as_mut_ptr(),
            params.len() as u32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::{AllocDisabler, assert_no_alloc};
    use miceq_dsp::ChainSettings;

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    #[test]
    fn stereo_duplicates_one_processed_signal_with_partial_links_and_silence() {
        let (mut control, mut processor) =
            miceq_engine::microphone(&ChainSettings::raw(), Arc::default()).expect("valid chain");
        let mut settings = ChainSettings::raw();
        settings.gain.params.gain_db = 6.0;
        control.apply(&settings).expect("queued gain");
        control.start_capture();
        let input = [0.1; 1024];
        let mut left = [0.0; 1024];
        let mut right = [0.0; 1024];
        // SAFETY: all non-null pointers refer to distinct arrays of 1024 samples.
        assert_no_alloc(|| unsafe {
            process_outputs(
                &mut processor,
                input.as_ptr(),
                left.as_mut_ptr(),
                right.as_mut_ptr(),
                input.len(),
                true,
            );
        });
        let mut capture = Vec::new();
        control.read_capture(&mut capture).expect("captured");
        assert_eq!(capture, input);
        assert_eq!(left, right);
        assert!((left[1023] - 0.1 * 10f32.powf(0.3)).abs() < 1e-5);

        for right_only in [true, false] {
            left.fill(-1.0);
            right.fill(-1.0);
            let (left_port, right_port) = if right_only {
                (ptr::null_mut(), right.as_mut_ptr())
            } else {
                (left.as_mut_ptr(), ptr::null_mut())
            };
            // SAFETY: linked output pointers each refer to 1024 writable samples.
            assert_no_alloc(|| unsafe {
                process_outputs(
                    &mut processor,
                    input.as_ptr(),
                    left_port,
                    right_port,
                    input.len(),
                    true,
                );
            });
            let output = if right_only { &right } else { &left };
            assert!((output[1023] - 0.1 * 10f32.powf(0.3)).abs() < 1e-5);
        }
        for (source, supported_rate) in [(ptr::null(), true), (input.as_ptr(), false)] {
            // SAFETY: outputs contain 1024 samples, and source is absent or points to input.
            assert_no_alloc(|| unsafe {
                process_outputs(
                    &mut processor,
                    source,
                    left.as_mut_ptr(),
                    right.as_mut_ptr(),
                    input.len(),
                    supported_rate,
                );
            });
            assert_eq!(left, [0.0; 1024]);
            assert_eq!(right, left);
        }
        control.start_capture();
        let mut scratch = [0.0; 128];
        // SAFETY: input contains 1024 readable samples; scratch is preallocated.
        assert_no_alloc(|| unsafe {
            process_outputs(
                &mut processor,
                input.as_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                input.len(),
                true,
            );
            process_unlinked(
                &mut processor,
                input.as_ptr(),
                input.len(),
                &mut scratch,
                false,
            );
        });
        capture.clear();
        control
            .read_capture(&mut capture)
            .expect("capture inactive");
        assert!(capture.is_empty());
        // SAFETY: input contains 1024 readable samples; scratch is preallocated.
        assert_no_alloc(|| unsafe {
            process_unlinked(
                &mut processor,
                input.as_ptr(),
                input.len(),
                &mut scratch,
                true,
            );
        });
        control
            .read_capture(&mut capture)
            .expect("capture without consumers");
        assert_eq!(capture, input);
        settings.muted = true;
        control.apply(&settings).expect("queued mute");
        // SAFETY: all non-null pointers refer to distinct arrays of 1024 samples.
        assert_no_alloc(|| unsafe {
            process_outputs(
                &mut processor,
                input.as_ptr(),
                left.as_mut_ptr(),
                right.as_mut_ptr(),
                input.len(),
                true,
            );
        });
        assert_eq!(left[1023], 0.0);
        assert_eq!(right, left);
    }
}
