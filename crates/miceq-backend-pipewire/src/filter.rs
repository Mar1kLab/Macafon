use crate::{Connection, Error};
use miceq_engine::MicRt;
use pipewire::{self as pw, properties::properties};
use std::{
    ffi::c_void,
    ptr::{self, NonNull},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

struct RtState {
    input: *mut c_void,
    output: *mut c_void,
    processor: MicRt,
    graph_rate_hz: Arc<AtomicU32>,
}

pub(crate) struct Filter {
    pointer: NonNull<pw::sys::pw_filter>,
    state: Box<RtState>,
    _events: Box<pw::sys::pw_filter_events>,
    hook: Box<pw::spa::sys::spa_hook>,
    _control_events: Box<pw::sys::pw_filter_events>,
    control_hook: Box<pw::spa::sys::spa_hook>,
    pub graph_rate_hz: Arc<AtomicU32>,
}

impl Filter {
    pub fn new(connection: &Connection, processor: MicRt) -> Result<Self, Error> {
        let graph_rate_hz = Arc::new(AtomicU32::new(0));
        let state = Box::new(RtState {
            input: ptr::null_mut(),
            output: ptr::null_mut(),
            processor,
            graph_rate_hz: graph_rate_hz.clone(),
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
            "audio.channels" => "1",
            "audio.position" => "[ MONO ]",
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
        // SAFETY: a zeroed hook is the documented initial listener state.
        let control_hook = Box::new(unsafe { std::mem::zeroed() });
        let mut filter = Self {
            pointer,
            state,
            _events: events,
            hook,
            graph_rate_hz,
            _control_events: control_events,
            control_hook,
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
        // SAFETY: this separate control listener accesses only the filter pointer;
        // it never aliases the mutable DSP state used by the RT callback.
        unsafe {
            pw::sys::pw_filter_add_listener(
                filter.pointer(),
                &mut *filter.control_hook,
                &*filter._control_events,
                filter.pointer().cast(),
            );
        }
        filter.state.input = filter.add_port(pw::sys::PW_DIRECTION_INPUT, "input_MONO")?;
        filter.state.output = filter.add_port(pw::sys::PW_DIRECTION_OUTPUT, "output_MONO")?;
        filter.connect()?;
        Ok(filter)
    }

    pub fn pointer(&self) -> *mut pw::sys::pw_filter {
        self.pointer.as_ptr()
    }

    fn add_port(&self, direction: u32, name: &str) -> Result<*mut c_void, Error> {
        let props = properties! {
            "format.dsp" => "32 bit float mono audio",
            "port.name" => name,
            "audio.channel" => "MONO",
        };
        // SAFETY: filter is alive; properties ownership is transferred; no format pods.
        let port = unsafe {
            pw::sys::pw_filter_add_port(
                self.pointer(),
                direction,
                pw::sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
                1,
                props.into_raw(),
                ptr::null_mut(),
                0,
            )
        };
        if port.is_null() {
            Err(Error::Message("Cannot create filter port".into()))
        } else {
            Ok(port)
        }
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
        use pw::spa::{
            pod::{Object, Pod, Property, PropertyFlags, Value, serialize::PodSerializer},
            sys,
        };
        let bytes = latency_pod(self.state.processor.latency_samples())?;
        let pod = Pod::from_bytes(&bytes)
            .ok_or_else(|| Error::Message("Invalid process latency pod".into()))?;
        // WirePlumber discovers devices through node EnumFormat, while our ports
        // retain their fixed DSP format. Advertise the actual mono graph format.
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
                    Value::Id(pw::spa::utils::Id(sys::SPA_AUDIO_FORMAT_F32P)),
                ),
                (
                    sys::SPA_FORMAT_AUDIO_rate,
                    Value::Int(miceq_dsp::SAMPLE_RATE_HZ as i32),
                ),
                (sys::SPA_FORMAT_AUDIO_channels, Value::Int(1)),
                (
                    sys::SPA_FORMAT_AUDIO_position,
                    Value::ValueArray(pw::spa::pod::ValueArray::Id(vec![pw::spa::utils::Id(
                        sys::SPA_AUDIO_CHANNEL_MONO,
                    )])),
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
        let format_bytes = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &format)
            .map_err(|error| {
                Error::Message(format!("Cannot serialize microphone format: {error:?}"))
            })?
            .0
            .into_inner();
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
    // SAFETY: ports are alive, and get_dsp_buffer is RT-safe. PipeWire checks capacity.
    let (input, output) = unsafe {
        (
            pw::sys::pw_filter_get_dsp_buffer(state.input, count).cast::<f32>(),
            pw::sys::pw_filter_get_dsp_buffer(state.output, count).cast::<f32>(),
        )
    };
    if output.is_null() {
        return;
    }
    // SAFETY: PipeWire provides count writable f32 samples; input can be absent or alias
    // output, so copy with ptr::copy before creating the mutable output slice.
    let output = unsafe {
        if input.is_null() {
            ptr::write_bytes(output, 0, count as usize);
        } else {
            ptr::copy(input, output, count as usize);
        }
        std::slice::from_raw_parts_mut(output, count as usize)
    };
    if position.clock.rate.num != 1 || position.clock.rate.denom != miceq_dsp::SAMPLE_RATE_HZ {
        output.fill(0.0);
        return;
    }
    state.processor.process(output);
    if input.is_null() {
        // A disconnected physical microphone must immediately produce digital silence.
        output.fill(0.0);
    }
}

// PipeWire invokes parameter notifications on the control loop, never in process().
unsafe extern "C" fn param_changed(
    data: *mut c_void,
    port: *mut c_void,
    id: u32,
    param: *const pw::spa::sys::spa_pod,
) {
    if !port.is_null() || id != pw::spa::sys::SPA_PARAM_Format {
        return;
    }
    use pw::spa::pod::{
        Object, Pod, Property, PropertyFlags, Value, ValueArray, serialize::PodSerializer,
    };
    // pw_filter has fixed DSP ports and no node PortConfig parameter. Record
    // the negotiated Format and notify Props after it, allowing WirePlumber
    // to finish its adapter activation using the ports already present.
    let props = Value::Object(Object {
        type_: pw::spa::sys::SPA_TYPE_OBJECT_Props,
        id: pw::spa::sys::SPA_PARAM_Props,
        properties: vec![Property {
            key: pw::spa::sys::SPA_PROP_channelMap,
            flags: PropertyFlags::empty(),
            value: Value::ValueArray(ValueArray::Id(vec![pw::spa::utils::Id(
                pw::spa::sys::SPA_AUDIO_CHANNEL_MONO,
            )])),
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
    // SAFETY: the listener's data is its live filter pointer, and the pod remains
    // valid during this synchronous control-loop call. No DSP state is accessed.
    unsafe {
        pw::sys::pw_filter_update_params(
            data.cast(),
            ptr::null_mut(),
            params.as_mut_ptr(),
            params.len() as u32,
        );
    }
}
