//! One output channel as a PipeWire node (PRD §12.1): a `pw_filter` with
//! `media.class = Audio/Sink`, stereo input ports that applications link to, the channel chain
//! processed in place in the same graph cycle, and stereo output ports that Macafon links to
//! the real output device itself (see ADR-0003).

use crate::Error;
use crate::filter::{
    ControlState, PortState, add_buffer, audio_format, io_changed, latency_pod, param_changed,
};
use macafon_engine::ChannelRt;
use pipewire::{self as pw, properties::properties};
use std::{
    ffi::{CString, c_void},
    ptr::{self, NonNull},
    sync::{
        Arc,
        atomic::{AtomicPtr, AtomicU32, Ordering},
    },
};

/// Larger quanta are processed in chunks of this many samples.
const SCRATCH_SAMPLES: usize = 8192;

struct RtState {
    input_left: *mut c_void,
    input_right: *mut c_void,
    output_left: *mut c_void,
    output_right: *mut c_void,
    processor: ChannelRt,
    scratch_left: Box<[f32]>,
    scratch_right: Box<[f32]>,
    /// Graph rate and quantum of the last cycle, for diagnostics.
    clock: Arc<[AtomicU32; 2]>,
}

pub(crate) struct ChannelNode {
    pointer: NonNull<pw::sys::pw_filter>,
    state: Box<RtState>,
    _events: Box<pw::sys::pw_filter_events>,
    hook: Box<pw::spa::sys::spa_hook>,
    _control_events: Box<pw::sys::pw_filter_events>,
    control_hook: Box<pw::spa::sys::spa_hook>,
    control_state: Box<ChannelControlState>,
    pub id: String,
    reported_latency: u32,
    clock: Arc<[AtomicU32; 2]>,
}

/// `node.name` of a channel: `macafon_ch_<id>` (OUT-004 excludes this prefix).
pub fn channel_node_name(id: &str) -> String {
    format!("{CHANNEL_PREFIX}{id}")
}

pub const CHANNEL_PREFIX: &str = "macafon_ch_";

impl ChannelNode {
    pub fn new(
        core: &pw::core::CoreRc,
        id: &str,
        description: &str,
        processor: ChannelRt,
    ) -> Result<Self, Error> {
        let node_name = channel_node_name(id);
        let latency = processor.latency_samples();
        let clock: Arc<[AtomicU32; 2]> = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
        let state = Box::new(RtState {
            clock: clock.clone(),
            input_left: ptr::null_mut(),
            input_right: ptr::null_mut(),
            output_left: ptr::null_mut(),
            output_right: ptr::null_mut(),
            processor,
            scratch_left: vec![0.0; SCRATCH_SAMPLES].into_boxed_slice(),
            scratch_right: vec![0.0; SCRATCH_SAMPLES].into_boxed_slice(),
        });
        // SAFETY: pw_filter_events is a C struct of integers and optional function pointers;
        // zero is a valid value for each field.
        let mut events: Box<pw::sys::pw_filter_events> = Box::new(unsafe { std::mem::zeroed() });
        events.version = pw::sys::PW_VERSION_FILTER_EVENTS;
        events.process = Some(process);
        let props = properties! {
            "node.name" => node_name.as_str(),
            "node.description" => description,
            "media.type" => "Audio",
            "media.category" => "Filter",
            "media.class" => "Audio/Sink",
            "node.virtual" => "true",
            "node.autoconnect" => "false",
            "node.want-driver" => "true",
            "node.pause-on-idle" => "false",
            "node.rate" => "1/48000",
            "node.latency" => "128/48000",
            "audio.rate" => "48000",
            "audio.channels" => "2",
            "audio.position" => "[ FL FR ]",
            "device.icon-name" => "audio-speakers",
        };
        let name = CString::new(description).unwrap_or_else(|_| c"Macafon channel".to_owned());
        // SAFETY: core is alive; properties ownership is transferred to PipeWire.
        let pointer =
            unsafe { pw::sys::pw_filter_new(core.as_raw_ptr(), name.as_ptr(), props.into_raw()) };
        let pointer = NonNull::new(pointer)
            .ok_or_else(|| Error::Message("Cannot create channel pw_filter".into()))?;
        // SAFETY: zeroed hooks and listener structs are the documented initial states.
        let hook = Box::new(unsafe { std::mem::zeroed() });
        // SAFETY: as above, integers and nullable function pointers only.
        let mut control_events: Box<pw::sys::pw_filter_events> =
            Box::new(unsafe { std::mem::zeroed() });
        control_events.version = pw::sys::PW_VERSION_FILTER_EVENTS;
        control_events.param_changed = Some(channel_param_changed);
        control_events.io_changed = Some(io_changed);
        control_events.add_buffer = Some(add_buffer);
        // SAFETY: a zeroed hook is the documented initial listener state.
        let control_hook = Box::new(unsafe { std::mem::zeroed() });
        let mut node = Self {
            pointer,
            state,
            _events: events,
            hook,
            _control_events: control_events,
            control_hook,
            control_state: Box::new(ChannelControlState {
                common: ControlState {
                    filter: pointer.as_ptr(),
                    stereo: true,
                    fixed_volume: false,
                },
                volume_pct: 100.0,
                muted: false,
                pending: false,
            }),
            id: id.to_owned(),
            reported_latency: latency,
            clock,
        };
        // SAFETY: callback state and events have stable heap addresses that outlive the
        // filter; destroying the filter joins processing before they are freed.
        unsafe {
            pw::sys::pw_filter_add_listener(
                node.pointer(),
                &mut *node.hook,
                &*node._events,
                (&mut *node.state as *mut RtState).cast(),
            );
            pw::sys::pw_filter_add_listener(
                node.pointer(),
                &mut *node.control_hook,
                &*node._control_events,
                (&mut *node.control_state as *mut ChannelControlState).cast(),
            );
        }
        node.state.input_left = node.add_port(pw::sys::PW_DIRECTION_INPUT, "playback_FL", "FL")?;
        node.state.input_right = node.add_port(pw::sys::PW_DIRECTION_INPUT, "playback_FR", "FR")?;
        node.state.output_left = node.add_port(pw::sys::PW_DIRECTION_OUTPUT, "output_FL", "FL")?;
        node.state.output_right = node.add_port(pw::sys::PW_DIRECTION_OUTPUT, "output_FR", "FR")?;
        publish_volume(&node.control_state)?;
        node.connect(latency)?;
        Ok(node)
    }

    /// Control loop only: consume desktop requests before publishing the accepted state.
    pub fn take_volume_change(&mut self) -> Option<(f32, bool)> {
        let state = &mut self.control_state;
        std::mem::take(&mut state.pending).then_some((state.volume_pct, state.muted))
    }

    pub fn set_volume(&mut self, volume_pct: f32, muted: bool) -> Result<(), Error> {
        let state = &mut self.control_state;
        if (state.volume_pct - volume_pct).abs() < 0.0001 && state.muted == muted {
            return Ok(());
        }
        state.volume_pct = volume_pct;
        state.muted = muted;
        publish_volume(state)
    }

    /// `(rate, quantum)` of the last processed cycle; zeros before the first one.
    pub fn clock(&self) -> (u32, u32) {
        (
            self.clock[0].load(Ordering::Relaxed),
            self.clock[1].load(Ordering::Relaxed),
        )
    }

    pub fn pointer(&self) -> *mut pw::sys::pw_filter {
        self.pointer.as_ptr()
    }

    /// PipeWire global id, once the node is registered.
    pub fn node_id(&self) -> u32 {
        // SAFETY: the filter is alive on this control loop.
        unsafe { pw::sys::pw_filter_get_node_id(self.pointer()) }
    }

    fn add_port(&self, direction: u32, name: &str, channel: &str) -> Result<*mut c_void, Error> {
        let props = properties! {
            "format.dsp" => "32 bit float mono audio",
            "port.name" => name,
            "audio.channel" => channel,
            "port.group" => if direction == pw::sys::PW_DIRECTION_OUTPUT { "output" } else { "playback" },
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
            return Err(Error::Message("Cannot create channel port".into()));
        }
        // SAFETY: PipeWire allocated aligned user storage of this size; graph notifications
        // begin only after connect().
        unsafe {
            port.cast::<PortState>().write(PortState {
                output: direction == pw::sys::PW_DIRECTION_OUTPUT,
                io: AtomicPtr::new(ptr::null_mut()),
            });
        }
        Ok(port)
    }

    fn connect(&self, latency: u32) -> Result<(), Error> {
        use pw::spa::{pod::Pod, sys};
        let latency = latency_pod(latency)?;
        let latency = Pod::from_bytes(&latency)
            .ok_or_else(|| Error::Message("Invalid process latency pod".into()))?;
        let format = audio_format(sys::SPA_AUDIO_FORMAT_F32P, true)?;
        let format = Pod::from_bytes(&format)
            .ok_or_else(|| Error::Message("Invalid channel format pod".into()))?;
        let mut params = [
            latency.as_raw_ptr().cast_const(),
            format.as_raw_ptr().cast_const(),
        ];
        // SAFETY: ports and callback storage are prepared before RT processing starts.
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
                "Cannot connect channel pw_filter: {result}"
            )))
        } else {
            Ok(())
        }
    }

    /// Report the chain latency to the graph (NFR-L4). Control loop only.
    pub fn set_latency(&mut self, samples: u32) -> Result<(), Error> {
        if samples == self.reported_latency {
            return Ok(());
        }
        let bytes = latency_pod(samples)?;
        let pod = pw::spa::pod::Pod::from_bytes(&bytes)
            .ok_or_else(|| Error::Message("Invalid process latency pod".into()))?;
        let mut params = [pod.as_raw_ptr().cast_const()];
        // SAFETY: the filter is alive on the control loop; the pod outlives the call.
        let result = unsafe {
            pw::sys::pw_filter_update_params(
                self.pointer(),
                ptr::null_mut(),
                params.as_mut_ptr(),
                params.len() as u32,
            )
        };
        if result < 0 {
            return Err(Error::Message(format!("Cannot update latency: {result}")));
        }
        self.reported_latency = samples;
        Ok(())
    }

    /// Rename the device apps see (CH-002). Control loop only.
    pub fn set_description(&self, description: &str) {
        let props = properties! { "node.description" => description };
        // SAFETY: the filter is alive; PipeWire copies the dictionary during the call.
        unsafe {
            pw::sys::pw_filter_update_properties(
                self.pointer(),
                ptr::null_mut(),
                props.dict().as_raw_ptr(),
            );
        }
    }
}

// This state belongs exclusively to PipeWire's control loop, never the RT callback.
struct ChannelControlState {
    common: ControlState,
    volume_pct: f32,
    muted: bool,
    pending: bool,
}

fn volume_props(volume_pct: f32, muted: bool) -> Result<Vec<u8>, Error> {
    use pw::spa::pod::{
        Object, Property, PropertyFlags, Value, ValueArray, serialize::PodSerializer,
    };
    use pw::spa::sys;
    let gain = macafon_dsp::channel::volume_gain(volume_pct);
    let properties = [
        (sys::SPA_PROP_volume, Value::Float(1.0)),
        (sys::SPA_PROP_mute, Value::Bool(muted)),
        (
            sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(vec![gain; 2])),
        ),
        (
            sys::SPA_PROP_softVolumes,
            Value::ValueArray(ValueArray::Float(vec![1.0; 2])),
        ),
        (
            sys::SPA_PROP_channelMap,
            Value::ValueArray(ValueArray::Id(vec![
                pw::spa::utils::Id(sys::SPA_AUDIO_CHANNEL_FL),
                pw::spa::utils::Id(sys::SPA_AUDIO_CHANNEL_FR),
            ])),
        ),
    ]
    .into_iter()
    .map(|(key, value)| Property {
        key,
        flags: PropertyFlags::empty(),
        value,
    })
    .collect();
    PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(Object {
            type_: sys::SPA_TYPE_OBJECT_Props,
            id: sys::SPA_PARAM_Props,
            properties,
        }),
    )
    .map(|(bytes, _)| bytes.into_inner())
    .map_err(|error| Error::Message(format!("Cannot serialize channel volume: {error:?}")))
}

fn publish_volume(state: &ChannelControlState) -> Result<(), Error> {
    let bytes = volume_props(state.volume_pct, state.muted)?;
    let pod = pw::spa::pod::Pod::from_bytes(&bytes)
        .ok_or_else(|| Error::Message("Invalid channel volume pod".into()))?;
    let mut params = [pod.as_raw_ptr().cast_const()];
    // SAFETY: control-loop-owned filter and pod remain live throughout this call.
    let result = unsafe {
        pw::sys::pw_filter_update_params(
            state.common.filter,
            ptr::null_mut(),
            params.as_mut_ptr(),
            1,
        )
    };
    if result < 0 {
        return Err(Error::Message(format!(
            "Cannot publish channel volume: {result}"
        )));
    }
    Ok(())
}

unsafe extern "C" fn channel_param_changed(
    data: *mut c_void,
    port: *mut c_void,
    id: u32,
    param: *const pw::spa::sys::spa_pod,
) {
    use pw::spa::{
        pod::{Pod, Value, ValueArray, deserialize::PodDeserializer},
        sys,
    };
    if data.is_null() {
        return;
    }
    // SAFETY: listener storage lives until filter destruction; callbacks run on the control loop.
    let state = unsafe { &mut *data.cast::<ChannelControlState>() };
    if id != sys::SPA_PARAM_Props || !port.is_null() {
        // SAFETY: delegate with the common listener state and original PipeWire arguments.
        unsafe {
            param_changed(
                (&mut state.common as *mut ControlState).cast(),
                port,
                id,
                param,
            )
        };
        // Format activation also publishes the channel map; restore the complete volume props.
        if port.is_null() && id == sys::SPA_PARAM_Format {
            let _ = publish_volume(state);
        }
        return;
    }
    if param.is_null() {
        return;
    }
    // SAFETY: PipeWire supplies a valid pod for the duration of this callback.
    let pod = unsafe { Pod::from_raw(param) };
    let Ok((_, Value::Object(object))) = PodDeserializer::deserialize_any_from(pod.as_bytes())
    else {
        return;
    };
    let mut gain = None;
    let mut factor = None;
    let mut muted = state.muted;
    for property in object.properties {
        match (property.key, property.value) {
            (sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(values))) => {
                // Macafon has one stereo fader: retain the loudest channel, no balance control.
                gain = values
                    .into_iter()
                    .filter(|v| v.is_finite())
                    .reduce(f32::max);
            }
            (sys::SPA_PROP_volume, Value::Float(value)) if value.is_finite() => {
                factor = Some(value)
            }
            (sys::SPA_PROP_mute, Value::Bool(value)) => muted = value,
            _ => {}
        }
    }
    // channelVolumes is linear amplitude; desktop percentages and our fader are cubic.
    let volume_pct = gain
        .map(|gain| gain * factor.unwrap_or(1.0))
        .or(factor)
        .map(|g| g.clamp(0.0, 1.0).cbrt() * 100.0)
        .unwrap_or(state.volume_pct);
    if (volume_pct - state.volume_pct).abs() > 0.001 || muted != state.muted {
        state.volume_pct = volume_pct;
        state.muted = muted;
        state.pending = true;
    }
    // Normalize balance, over-amplification and scalar volume without applying gain twice.
    let _ = publish_volume(state);
}

impl Drop for ChannelNode {
    fn drop(&mut self) {
        // SAFETY: uniquely owned filter; destruction stops callbacks before state is dropped.
        unsafe {
            pw::sys::pw_filter_destroy(self.pointer());
        }
    }
}

unsafe extern "C" fn process(data: *mut c_void, position: *mut pw::spa::sys::spa_io_position) {
    assert_no_alloc::assert_no_alloc(|| {
        // SAFETY: PipeWire supplies the callback arguments described in process_inner.
        unsafe { process_inner(data, position) }
    });
}

unsafe fn process_inner(data: *mut c_void, position: *mut pw::spa::sys::spa_io_position) {
    if data.is_null() || position.is_null() {
        return;
    }
    // SAFETY: PipeWire serializes this callback on its data loop; data is the prepared
    // RtState and position is valid during this invocation.
    let (state, position) = unsafe { (&mut *data.cast::<RtState>(), &*position) };
    let Ok(count) = usize::try_from(position.clock.duration) else {
        return;
    };
    if count == 0 || count > SCRATCH_SAMPLES {
        return;
    }
    state.clock[0].store(position.clock.rate.denom, Ordering::Relaxed);
    state.clock[1].store(count as u32, Ordering::Relaxed);
    let supported_rate =
        position.clock.rate.num == 1 && position.clock.rate.denom == macafon_dsp::SAMPLE_RATE_HZ;
    // SAFETY: ports are alive; PipeWire supplies negotiated DSP buffers for this quantum,
    // or null for unlinked ports.
    let buffer = |port: *mut c_void| unsafe {
        pw::sys::pw_filter_get_dsp_buffer(port, count as u32).cast::<f32>()
    };
    let (in_l, in_r) = (buffer(state.input_left), buffer(state.input_right));
    let (out_l, out_r) = (buffer(state.output_left), buffer(state.output_right));
    if out_l.is_null() && out_r.is_null() {
        // Not linked to an output device: nothing to produce, nothing to spend (NFR-R7).
        return;
    }
    // Unlinked outputs process into scratch so the chain always sees both channels.
    let out_l = if out_l.is_null() {
        state.scratch_left.as_mut_ptr()
    } else {
        out_l
    };
    let out_r = if out_r.is_null() {
        state.scratch_right.as_mut_ptr()
    } else {
        out_r
    };
    // SAFETY: each non-null pointer refers to `count` samples of a distinct port buffer or a
    // scratch buffer of at least `count` samples; ptr::copy allows input/output aliasing.
    unsafe {
        copy_or_silence(in_l, out_l, count);
        copy_or_silence(if in_r.is_null() { in_l } else { in_r }, out_r, count);
        let left = std::slice::from_raw_parts_mut(out_l, count);
        let right = std::slice::from_raw_parts_mut(out_r, count);
        if supported_rate {
            state.processor.process(left, right);
        } else {
            left.fill(0.0);
            right.fill(0.0);
        }
    }
}

/// SAFETY: `output` has `count` writable samples; `input` is null or has `count` readable ones.
unsafe fn copy_or_silence(input: *const f32, output: *mut f32, count: usize) {
    // SAFETY: guaranteed by the caller.
    unsafe {
        if input.is_null() {
            ptr::write_bytes(output, 0, count);
        } else {
            ptr::copy(input, output, count);
        }
    }
}
