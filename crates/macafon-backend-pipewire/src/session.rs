//! Control-loop orchestration of one microphone and mixer graph session.

use crate::graph::{Graph, Port};
use crate::nodes::{enrich_node, hardware, is_physical_input, node};
use crate::{
    Connection, DEFAULT_QUANTUM_FRAMES, Error, MicrophoneHost, Node, PlayerRequest, filter, meter,
    mixer, player, profiler,
};
use pipewire::{self as pw, types::ObjectType};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub(super) fn run_session<H: MicrophoneHost + 'static>(
    mainloop: &pw::main_loop::MainLoopRc,
    host: &Rc<RefCell<H>>,
    waiting: Rc<Cell<bool>>,
) -> Result<(), Error> {
    let connection = Connection::with_mainloop(mainloop.clone(), true)?;
    let processor = host.borrow_mut().start_session()?;
    let filter = Rc::new(filter::Filter::new(
        &connection,
        processor,
        host.borrow().stereo(),
    )?);
    let registry = connection.core.get_registry_rc()?;
    let graph = Rc::new(RefCell::new(Graph::default()));
    let streams = mixer::SharedStreams::default();
    let stream_bindings = Rc::new(RefCell::new(mixer::StreamBindings::new()));
    let metadata_state = Rc::new(RefCell::new(mixer::MetadataState::default()));
    let bound_metadata = Rc::new(RefCell::new(None::<mixer::BoundMetadata>));
    let bind_registry = registry.clone();
    let (bind_streams, bind_bindings) = (streams.clone(), stream_bindings.clone());
    let (bind_metadata_state, bind_metadata) = (metadata_state.clone(), bound_metadata.clone());
    let (forget_streams, forget_bindings) = (streams.clone(), stream_bindings.clone());
    let profiler_global = Rc::new(Cell::new(None));
    let profiler_add = profiler_global.clone();
    let profiler_remove = profiler_global.clone();
    let graph_add = graph.clone();
    let graph_remove = graph.clone();
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ == ObjectType::Profiler {
                profiler_add.set(Some((global.id, global.version)));
            }
            let mut graph = graph_add.borrow_mut();
            if let Some(device) = hardware(global) {
                graph.devices.push(device);
                graph.inputs_changed = true;
            }
            if let Some(node) = node(global) {
                graph.inputs_changed |= is_physical_input(&node);
                graph.outputs_changed |= mixer::is_output_device(&node);
                graph.nodes.push(node);
            }
            if mixer::is_playback_stream(global) {
                if let Some(binding) = mixer::bind_stream(&bind_registry, global, &bind_streams) {
                    bind_bindings.borrow_mut().insert(global.id, binding);
                }
            }
            if global.type_ == ObjectType::Metadata
                && global
                    .props
                    .is_some_and(|p| p.get("metadata.name") == Some("default"))
            {
                *bind_metadata.borrow_mut() =
                    mixer::bind_metadata(&bind_registry, global, &bind_metadata_state);
            }
            if global.type_ == ObjectType::Port {
                if let Some(props) = global.props {
                    if let Some(node_id) = props.get("node.id").and_then(|id| id.parse().ok()) {
                        graph.ports.push(Port {
                            id: global.id,
                            node_id,
                            direction: props.get("port.direction").unwrap_or("").to_owned(),
                            channel: props.get("audio.channel").unwrap_or("").to_owned(),
                        });
                    }
                }
            }
        })
        .global_remove(move |id| {
            if profiler_remove
                .get()
                .is_some_and(|(global, _)| global == id)
            {
                profiler_remove.set(None);
            }
            let mut graph = graph_remove.borrow_mut();
            if graph.link.as_ref().is_some_and(|link| {
                link.output == id
                    || link.input == id
                    || link.output_right == id
                    || link.input_right == id
            }) {
                graph.link = None;
            }
            if let Some((input, output)) = graph
                .nodes
                .iter()
                .find(|node| node.id == id)
                .map(|node| (is_physical_input(node), mixer::is_output_device(node)))
            {
                graph.inputs_changed |= input;
                graph.outputs_changed |= output;
            }
            mixer::forget_stream(&forget_streams, &forget_bindings, id);
            graph.nodes.retain(|n| n.id != id);
            graph.devices.retain(|device| device.id != id);
            graph.ports.retain(|p| p.id != id);
        })
        .register();
    let loop_ = connection.mainloop.clone();
    let core = connection.core.clone();
    let session_filter = filter.clone();
    let graph_rate_hz = filter.graph_rate_hz.clone();
    let failure = connection.failure.clone();
    let host = host.clone();
    let ready = Cell::new(false);
    let reported_latency = Cell::new(host.borrow().latency_samples());

    let connected = Cell::new(None);
    // Dropped with the timer, before the connection.
    let player = RefCell::new(None);
    let meters = RefCell::new(Vec::<(String, meter::Meter)>::new());
    let profiler_available = connection.profiler_available;
    let profiling = RefCell::new(None::<((u32, u32), profiler::Profiler)>);
    let profiler_host = host.clone();
    let mixer_session = Rc::new(RefCell::new(mixer::MixerSession::default()));
    let tick_mixer = mixer_session.clone();
    let requested_quantum = Cell::new(DEFAULT_QUANTUM_FRAMES);
    let (tick_streams, tick_metadata, tick_bound, tick_bindings) = (
        streams.clone(),
        metadata_state.clone(),
        bound_metadata.clone(),
        stream_bindings.clone(),
    );
    let timer = connection.mainloop.loop_().add_timer(move |_| {
        let filter = &session_filter;
        let rate_hz = graph_rate_hz.load(std::sync::atomic::Ordering::Relaxed);
        if rate_hz != 0 && rate_hz != macafon_dsp::SAMPLE_RATE_HZ {
            *failure.borrow_mut() = Some(Error::Message(format!(
                "Macafon requires a 48000 Hz graph, got {rate_hz} Hz"
            )));
            loop_.quit();
            return;
        }
        // SAFETY: the timer runs on the control loop and is dropped before the filter.
        let state = unsafe { pw::sys::pw_filter_get_state(filter.pointer(), std::ptr::null_mut()) };
        if state == pw::sys::pw_filter_state_PW_FILTER_STATE_ERROR {
            *failure.borrow_mut() = Some(Error::Message(
                "Virtual microphone entered an error state".into(),
            ));
            loop_.quit();
            return;
        }
        if matches!(
            state,
            pw::sys::pw_filter_state_PW_FILTER_STATE_PAUSED
                | pw::sys::pw_filter_state_PW_FILTER_STATE_STREAMING
        ) && !ready.replace(true)
        {
            if waiting.replace(false) {
                tracing::info!("PipeWire reconnected; virtual microphone restored");
            } else {
                tracing::info!("Macafon Microphone running; Ctrl+C to stop");
            }
        }
        let mut host = host.borrow_mut();
        let active_profiler = profiler_global.get().filter(|_| {
            profiler_available
                && host.needs_processing_budget()
                && state == pw::sys::pw_filter_state_PW_FILTER_STATE_STREAMING
        });
        if profiling.borrow().as_ref().map(|(global, _)| *global) != active_profiler {
            profiling.replace(None);
            if let Some((id, version)) = active_profiler {
                // SAFETY: the session filter is alive on this control loop.
                let node_id = unsafe { pw::sys::pw_filter_get_node_id(filter.pointer()) };
                let handler = profiler_host.clone();
                match profiler::Profiler::new(
                    registry.clone(),
                    id,
                    version,
                    node_id,
                    move |elapsed_ns, budget_ns| {
                        handler
                            .borrow_mut()
                            .processing_budget(elapsed_ns, budget_ns);
                    },
                ) {
                    Ok(listener) => {
                        profiling.replace(Some(((id, version), listener)));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Cannot monitor microphone processing budget")
                    }
                }
            }
        }
        match host.player_request() {
            Some(PlayerRequest::Start { player: rt, target }) => {
                player.replace(None);
                match player::Player::new(&core, rt, target.as_deref()) {
                    Ok(started) => {
                        player.replace(Some(started));
                    }
                    Err(error) => host.player_failed(error),
                }
            }
            Some(PlayerRequest::Stop) => {
                player.replace(None);
            }
            None => {}
        }
        let quantum = filter
            .graph_quantum
            .load(std::sync::atomic::Ordering::Relaxed);
        let (rate_hz, quantum) = if rate_hz != 0 && quantum != 0 {
            (rate_hz, quantum)
        } else {
            tick_mixer.borrow().clock().unwrap_or((0, 0))
        };
        host.graph_clock(rate_hz, quantum);
        let latency = host.latency_samples();
        if latency != reported_latency.get() {
            match filter.set_latency(latency) {
                Ok(()) => reported_latency.set(latency),
                Err(error) => tracing::warn!(%error, "Cannot report latency"),
            }
        }
        filter.set_keep_processing(host.keep_processing());
        let quantum = host.quantum_frames();
        if requested_quantum.replace(quantum) != quantum {
            filter.request_quantum(quantum);
        }
        filter.set_input_channel(host.input_channel());
        let detected = filter
            .detected_channel
            .load(std::sync::atomic::Ordering::Relaxed);
        host.input_channel_detected(if detected == 2 {
            macafon_dsp::InputChannel::Right
        } else {
            macafon_dsp::InputChannel::Left
        });
        let mut graph = graph.borrow_mut();
        if std::mem::take(&mut graph.inputs_changed) {
            let mut inputs: Vec<Node> = graph
                .nodes
                .iter()
                .filter(|n| is_physical_input(n))
                .cloned()
                .collect();
            for node in &mut inputs {
                enrich_node(node, &graph.devices);
            }
            host.inputs_changed(&inputs);
        }
        let mut meters = meters.borrow_mut();
        if host.device_metering() {
            meters.retain(|(name, _)| {
                graph
                    .nodes
                    .iter()
                    .any(|node| &node.name == name && is_physical_input(node))
            });
            for node in graph.nodes.iter().filter(|node| is_physical_input(node)) {
                if !meters.iter().any(|(name, _)| name == &node.name) {
                    match meter::Meter::new(&core, &node.name) {
                        Ok(meter) => meters.push((node.name.clone(), meter)),
                        Err(error) => tracing::warn!(%error, "Cannot meter physical input"),
                    }
                }
            }
            for (name, meter) in meters.iter_mut() {
                if let Some(source) = graph.nodes.iter().find(|node| &node.name == name) {
                    if let Err(error) = meter.connect_source(&core, source, &graph.ports) {
                        tracing::warn!(%error, "Cannot link input meter");
                    }
                }
                let peak = meter.read_peak();
                host.device_meter(
                    name,
                    if peak > 0.0 {
                        20.0 * peak.log10()
                    } else {
                        -120.0
                    },
                );
            }
        } else {
            meters.clear();
        }
        let input = host.input();
        if graph
            .link
            .as_ref()
            .is_some_and(|link| Some(&link.source) != input.as_ref())
        {
            graph.link = None;
        }
        if let Some(input_name) = &input {
            // SAFETY: the filter remains alive until after the timer is dropped.
            let id = unsafe { pw::sys::pw_filter_get_node_id(filter.pointer()) };
            if let Err(error) = graph.connect_input(&core, input_name, id) {
                *failure.borrow_mut() = Some(error);
                loop_.quit();
            }
        }
        let now_connected = graph.link.is_some();
        if connected.replace(Some(now_connected)) != Some(now_connected)
            && !now_connected
            && input.is_some()
            && ready.get()
        {
            tracing::warn!("Microphone disconnected; producing silence");
        }
        host.input_connected(now_connected);
        tick_mixer.borrow_mut().tick(
            &mut *host,
            &core,
            &mut graph,
            &tick_streams,
            &tick_metadata,
            tick_bound.borrow().as_ref(),
            &tick_bindings,
        );
    });
    timer
        .update_timer(
            Some(Duration::from_millis(50)),
            Some(Duration::from_millis(100)),
        )
        .into_result()
        .map_err(|error| Error::Message(error.to_string()))?;
    connection.mainloop.run();
    drop(timer);
    // Quit or a lost server: undo stream targets before the channels disappear (BG-005).
    let mut mixer_session = mixer_session.borrow_mut();
    // Only on a clean stop: `check` would consume a disconnect that must trigger a reconnect.
    if mixer_session.has_channels() && connection.failure.borrow().is_none() {
        mixer_session.reset_volumes(&stream_bindings);
        mixer_session.release_all(&streams, bound_metadata.borrow().as_ref());
        let _ = connection.roundtrip();
    }
    *mixer_session = mixer::MixerSession::default();
    connection.check()
}
