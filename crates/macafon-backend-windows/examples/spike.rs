//! M3.0 spike (PRD §19.1): endpoint and VB-CABLE discovery, default devices, audio sessions,
//! and the WASAPI capture → chain → render path with latency and clock-drift figures.
//!
//! `cargo run --release -p macafon-backend-windows --example spike -- devices`

#[cfg(not(windows))]
fn main() {
    eprintln!("the Windows spike only runs on Windows");
}

#[cfg(windows)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    spike::main()
}

#[cfg(windows)]
mod spike {
    use anyhow::{Context, bail};
    use clap::{Parser, Subcommand};
    use macafon_backend_windows::devices::VB_CABLE_HARDWARE_ID;
    use macafon_backend_windows::{
        ComGuard, DataFlow, Endpoint, EndpointRole, MicrophoneConfig, MicrophonePipeline,
        endpoints, find_vb_cable, policy, probe, sessions,
    };
    use macafon_dsp::{Preset, Telemetry};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[derive(Parser)]
    struct Args {
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Subcommand)]
    enum Command {
        /// Active endpoints with adapter hardware IDs; marks VB-CABLE.
        Devices,
        /// List physical outputs and show priority selection; writes are opt-in.
        Outputs {
            /// Ordered endpoint IDs (empty means follow system default).
            #[arg(long)]
            priority: Vec<String>,
            #[arg(long)]
            apply: bool,
            #[arg(long, default_value_t = 3)]
            restore_after_s: u64,
            #[arg(long, default_value = ".artifacts/output-state")]
            state_dir: std::path::PathBuf,
        },
        /// Read cable, format, writer and engine-period facts without changing settings.
        Doctor {
            /// Physical microphone ID or part of its name; otherwise use capture default.
            #[arg(long)]
            input: Option<String>,
        },
        /// Default capture and render endpoints per role.
        Defaults,
        /// Make an endpoint the default for all roles, then restore the previous defaults.
        SetDefault {
            /// Endpoint ID or part of its name.
            device: String,
            #[arg(long, default_value_t = 3)]
            restore_after_s: u64,
        },
        /// Application audio sessions on all render endpoints.
        Sessions,
        /// Watch app groups and peaks, optionally set an executable's group fader.
        Mixer {
            #[arg(long)]
            exe: Option<String>,
            #[arg(long, default_value_t = 50.0)]
            volume: f32,
            #[arg(long)]
            mute: bool,
            #[arg(long, default_value_t = 15)]
            seconds: u64,
            #[arg(long, default_value = ".artifacts/mixer-state")]
            state_dir: std::path::PathBuf,
        },
        /// Capture → chain → render. Reports latency components and clock drift every second.
        Run(RunOptions),
        /// Measure the delay between two capture endpoints (sanity check of the probe).
        Probe {
            direct: String,
            processed: String,
            #[arg(long, default_value_t = 2.0)]
            seconds: f64,
        },
    }

    #[derive(clap::Args)]
    struct RunOptions {
        /// Capture endpoint ID or part of its name; default: system default microphone.
        #[arg(long)]
        input: Option<String>,
        /// Render endpoint ID or part of its name; default: VB-CABLE `CABLE Input`.
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value = "raw")]
        preset: String,
        #[arg(long, default_value_t = 20)]
        seconds: u64,
        /// Latency kept above the minimum safe level (render buffer + one period).
        #[arg(long, default_value_t = 2.0)]
        margin_ms: f64,
        /// Requested WASAPI buffer per stream; 0 = engine default.
        #[arg(long, default_value_t = 0.0)]
        stream_buffer_ms: f64,
        /// Render at the smallest shared-mode period (IAudioClient3).
        #[arg(long)]
        low_latency: bool,
        /// Measure the added latency against the microphone every N seconds (VB-CABLE only).
        #[arg(long, default_value_t = 0)]
        probe_every_s: u64,
        /// Switch the input to this endpoint (ID or part of its name) after `--switch-after-s`.
        #[arg(long)]
        switch_input: Option<String>,
        #[arg(long, default_value_t = 10)]
        switch_after_s: u64,
    }

    pub fn main() -> anyhow::Result<()> {
        let args = Args::parse();
        let _com = ComGuard::new()?;
        match args.command {
            Command::Devices => devices(),
            Command::Outputs {
                priority,
                apply,
                restore_after_s,
                state_dir,
            } => outputs(&priority, apply, restore_after_s, &state_dir),
            Command::Doctor { input } => doctor(input.as_deref()),
            Command::Defaults => defaults(),
            Command::SetDefault {
                device,
                restore_after_s,
            } => set_default(&device, restore_after_s),
            Command::Sessions => list_sessions(),
            Command::Mixer {
                exe,
                volume,
                mute,
                seconds,
                state_dir,
            } => {
                use macafon_backend_windows::mixer::{MixSettings, SessionWatcher};
                let watcher = SessionWatcher::start(&state_dir)?;
                if let Some(exe) = exe {
                    watcher.set_app(
                        &exe,
                        MixSettings {
                            channel_pct: volume,
                            channel_muted: mute,
                            ..MixSettings::default()
                        },
                    )?;
                }
                let until = Instant::now() + Duration::from_secs(seconds);
                while Instant::now() < until {
                    while let Some(event) = watcher.try_event() {
                        println!("{event:?}");
                    }
                    for app in watcher.apps()? {
                        println!("{app:?}");
                    }
                    for peak in watcher.output_peaks()? {
                        println!("{peak:?}");
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                watcher.restore()?;
                while let Some(event) = watcher.try_event() {
                    println!("{event:?}");
                }
                println!("conditional session restoration completed");
                Ok(())
            }
            Command::Run(options) => run(&options),
            Command::Probe {
                direct,
                processed,
                seconds,
            } => {
                let list = endpoints()?;
                let direct = pick(&list, &direct, Some(DataFlow::Capture))?;
                let processed = pick(&list, &processed, Some(DataFlow::Capture))?;
                let m = probe::measure_added_latency(&direct.id, &processed.id, seconds)?;
                println!(
                    "delay {:.2} ms (correlation {:.3})",
                    m.added_latency_ms, m.correlation
                );
                Ok(())
            }
        }
    }

    fn outputs(
        priority: &[String],
        apply: bool,
        seconds: u64,
        state_dir: &std::path::Path,
    ) -> anyhow::Result<()> {
        use macafon_backend_windows::{
            journal::{self, SystemDefaults},
            output::{self, OutputPolicy},
        };
        let list = endpoints()?;
        for e in list
            .iter()
            .filter(|e| e.flow == DataFlow::Render && !e.is_vb_cable())
        {
            println!("{}  {}", e.id, e.name);
        }
        let before = policy::defaults(DataFlow::Render)?;
        println!("current defaults: {before:?}");
        println!(
            "priority choice: {:?}",
            output::priority_choice(priority, &list, before[0].as_deref())
        );
        if apply {
            let path = journal::journal_path(state_dir, DataFlow::Render);
            let mut defaults = SystemDefaults;
            // Recover an interrupted prior run before taking a fresh snapshot.
            journal::restore(&mut defaults, &path)?;
            let before = policy::defaults(DataFlow::Render)?;
            let result = OutputPolicy::default().scan(priority, &list, &mut defaults, &path);
            println!("policy: {result:?}");
            if result.is_ok() {
                std::thread::sleep(Duration::from_secs(seconds));
            }
            // Also restore after a partially successful set.
            journal::restore(&mut defaults, &path)?;
            result?;
            let after = policy::defaults(DataFlow::Render)?;
            println!("restored defaults: {after:?}");
            if before != after {
                bail!("defaults differ after conditional restoration (possibly a manual change)");
            }
        }
        Ok(())
    }

    fn doctor(input: Option<&str>) -> anyhow::Result<()> {
        use macafon_backend_windows::diagnostics;
        let list = endpoints()?;
        let microphone = match input {
            Some(query) => Some(pick(&list, query, Some(DataFlow::Capture))?.id),
            None => policy::defaults(DataFlow::Capture)?[0]
                .clone()
                .filter(|id| list.iter().any(|e| e.id == *id && !e.is_vb_audio())),
        };
        let facts = diagnostics::collect(microphone.as_deref(), std::process::id())?;
        println!(
            "cable installed: {}; registered: {}; active: {}; other VB-Audio: {}",
            facts.cable.installed,
            facts.cable.registered_endpoints.len(),
            facts.cable.active_endpoints.len(),
            facts.cable.other_vb_audio.len()
        );
        for e in &facts.cable.registered_endpoints {
            println!("  cable {:?}: {} ({})", e.flow, e.name, e.id);
        }
        for e in &facts.cable.other_vb_audio {
            println!("  ignored VB-Audio: {} ({})", e.name, e.id);
        }
        for e in facts.endpoints {
            println!("{} ({})", e.endpoint.name, e.endpoint.id);
            for (label, format) in [("mix", e.mix_format), ("device", e.device_format)] {
                match format {
                    Ok(f) => println!(
                        "  {label}: {} Hz, {} ch, {} bits; resampling: {}",
                        f.rate_hz,
                        f.channels,
                        f.bits,
                        f.needs_resampling()
                    ),
                    Err(error) => println!("  {label}: unavailable ({error})"),
                }
            }
            match e.engine_periods {
                Ok(p) => {
                    let ms = |frames| f64::from(frames) * 1000.0 / f64::from(p.mix_rate_hz);
                    println!(
                        "  engine periods: default {:.2} ms, min {:.2} ms, max {:.2} ms, step {:.2} ms",
                        ms(p.default_frames),
                        ms(p.min_frames),
                        ms(p.max_frames),
                        ms(p.fundamental_frames)
                    );
                }
                Err(error) => println!("  engine periods: unavailable ({error})"),
            }
        }
        println!(
            "render defaults: {:?}; cable roles: {:?}",
            facts.render_defaults, facts.cable_default_roles
        );
        println!("foreign writers: {}", facts.foreign_writers.len());
        for s in facts.foreign_writers {
            println!(
                "  pid {} exe {:?} endpoint {} active {} muted {} peak {:.4}",
                s.process_id, s.executable, s.endpoint_id, s.active, s.muted, s.peak
            );
        }
        println!(
            "{}\n{}",
            facts.format_settings, facts.latency_recommendation
        );
        Ok(())
    }

    fn devices() -> anyhow::Result<()> {
        let list = endpoints()?;
        for endpoint in &list {
            let tag = if endpoint.is_vb_cable() {
                " [VB-CABLE]"
            } else if endpoint.is_vb_audio() {
                " [other VB-Audio]"
            } else {
                ""
            };
            println!("{:?}  {}{tag}", endpoint.flow, endpoint.name);
            println!("    id        {}", endpoint.id);
            println!(
                "    adapter   {}",
                endpoint.adapter_instance_id.as_deref().unwrap_or("?")
            );
            println!("    hw ids    {}", endpoint.hardware_ids.join(", "));
            println!(
                "    service   {}",
                endpoint.service.as_deref().unwrap_or("?")
            );
            match macafon_backend_windows::wasapi::engine_periods(&endpoint.id) {
                Ok(p) => {
                    let ms = |frames: u32| f64::from(frames) * 1000.0 / f64::from(p.mix_rate_hz);
                    println!(
                        "    mix       {} Hz, {} ch; engine period default {:.2} ms, min {:.2} ms, step {:.2} ms",
                        p.mix_rate_hz,
                        p.mix_channels,
                        ms(p.default_frames),
                        ms(p.min_frames),
                        ms(p.fundamental_frames),
                    );
                }
                Err(error) => println!("    mix       ? ({error})"),
            }
        }
        match find_vb_cable(&list) {
            Some(cable) => println!(
                "\nVB-CABLE found: input '{}', output '{}'",
                cable.input.name, cable.output.name
            ),
            None => println!("\nVB-CABLE not found (hardware ID {VB_CABLE_HARDWARE_ID})"),
        }
        Ok(())
    }

    fn defaults() -> anyhow::Result<()> {
        let list = endpoints()?;
        for flow in [DataFlow::Capture, DataFlow::Render] {
            for (role, id) in EndpointRole::ALL.iter().zip(policy::defaults(flow)?) {
                println!("{flow:?} {role:?}: {}", describe(&list, id.as_deref()));
            }
        }
        Ok(())
    }

    fn set_default(device: &str, restore_after_s: u64) -> anyhow::Result<()> {
        let list = endpoints()?;
        let endpoint = pick(&list, device, None)?;
        let previous = policy::defaults(endpoint.flow)?;
        println!("setting '{}' as default {:?}", endpoint.name, endpoint.flow);
        for role in EndpointRole::ALL {
            policy::set_default(&endpoint.id, role)?;
        }
        for (role, id) in EndpointRole::ALL
            .iter()
            .zip(policy::defaults(endpoint.flow)?)
        {
            println!("  now {role:?}: {}", describe(&list, id.as_deref()));
        }
        std::thread::sleep(Duration::from_secs(restore_after_s));
        for (role, id) in EndpointRole::ALL.iter().zip(&previous) {
            if let Some(id) = id {
                policy::set_default(id, *role)?;
            }
        }
        for (role, id) in EndpointRole::ALL
            .iter()
            .zip(policy::defaults(endpoint.flow)?)
        {
            println!("  restored {role:?}: {}", describe(&list, id.as_deref()));
        }
        if policy::defaults(endpoint.flow)? != previous {
            bail!("defaults were not restored");
        }
        Ok(())
    }

    fn list_sessions() -> anyhow::Result<()> {
        let list = endpoints()?;
        for session in sessions::sessions()? {
            let exe = session
                .executable
                .as_deref()
                .and_then(|path| path.rsplit('\\').next())
                .unwrap_or(if session.system_sounds {
                    "<system sounds>"
                } else {
                    "?"
                });
            println!(
                "{:<28} pid {:>6} vol {:>4.0}% {}{} peak {:.3}  on {}  '{}'",
                exe,
                session.process_id,
                session.volume * 100.0,
                if session.muted { "muted " } else { "" },
                if session.active { "active" } else { "idle" },
                session.peak,
                describe(&list, Some(&session.endpoint_id)),
                session.display_name,
            );
        }
        Ok(())
    }

    fn run(options: &RunOptions) -> anyhow::Result<()> {
        let list = endpoints()?;
        let input = match options.input.as_deref() {
            Some(query) => pick(&list, query, Some(DataFlow::Capture))?,
            None => {
                let [console, ..] = policy::defaults(DataFlow::Capture)?;
                let id = console.context("no default microphone")?;
                list.iter()
                    .find(|endpoint| endpoint.id == id)
                    .cloned()
                    .context("default microphone is not active")?
            }
        };
        if input.is_vb_cable() {
            bail!("the input is VB-CABLE itself; pick a physical microphone with --input");
        }
        let output = match options.output.as_deref() {
            Some(query) => pick(&list, query, Some(DataFlow::Render))?,
            None => {
                find_vb_cable(&list)
                    .context("VB-CABLE is not installed; pass --output to test on another device")?
                    .input
            }
        };
        let preset = &options.preset;
        let settings = Preset::from_id(preset)
            .with_context(|| format!("unknown preset {preset}"))?
            .settings();
        let telemetry = Arc::new(Telemetry::default());
        telemetry.set_enabled(true);
        let (mut control, processor) = macafon_engine::microphone(&settings, telemetry.clone())?;
        println!(
            "input  '{}'\noutput '{}'\npreset {preset}",
            input.name, output.name
        );
        let pipeline = MicrophonePipeline::start(
            &MicrophoneConfig {
                input_id: Some(input.id.clone()),
                output_id: Some(output.id.clone()),
                latency_margin_ms: options.margin_ms,
                stream_buffer_ms: options.stream_buffer_ms,
                low_latency_render: options.low_latency,
            },
            processor,
        )?;
        let report = pipeline.report();
        println!(
            "capture: buffer {:.2} ms, period {:.2} ms, engine latency {:.2} ms",
            report.capture_buffer_ms, report.capture_period_ms, report.capture_latency_ms
        );
        println!(
            "render:  buffer {:.2} ms, period {:.2} ms, engine latency {:.2} ms",
            report.render_buffer_ms, report.render_period_ms, report.render_latency_ms
        );
        println!("chain:   {:.2} ms", report.chain_latency_ms);
        let probe_target = find_vb_cable(&list)
            .filter(|cable| cable.input.id == output.id)
            .map(|cable| cable.output.id);
        if options.probe_every_s > 0 && probe_target.is_none() {
            bail!("--probe-every-s needs VB-CABLE as the output");
        }
        let deadline = Instant::now() + Duration::from_secs(options.seconds);
        let mut switch_input = options.switch_input.clone();
        let mut next_probe = Instant::now() + Duration::from_secs(options.probe_every_s);
        while Instant::now() < deadline && pipeline.is_running() {
            std::thread::sleep(Duration::from_secs(1));
            control.collect_garbage();
            let meters = telemetry.take();
            let r = pipeline.report();
            println!(
                "{:>5.0}s in {:>6.1} dBFS out {:>6.1} dBFS | level {:>5.2} ms (buffer min {:>5.2}), render queue {:>5.2} ms, max packet {:>5.2}/{:>5.2} ms, \
                 est. {:>5.1} ms | drift {:>+5} ppm (rates {:.1}/{:.1} Hz) | underruns {} dropped {} skipped {} gaps {} | in {} out {} reopened {}",
                r.elapsed_s,
                meters.input_peak_dbfs,
                meters.output_peak_dbfs,
                r.drift_level_ms,
                r.drift_buffer_min_ms.unwrap_or(0.0),
                r.render_padding_ms,
                r.capture_max_packet_ms,
                r.render_max_request_ms,
                r.estimated_latency_ms(),
                r.drift_ratio_ppm,
                r.capture_rate_hz,
                r.render_rate_hz,
                r.underruns,
                r.dropped_frames,
                r.skipped_frames,
                r.discontinuities,
                if r.input_connected { "up" } else { "DOWN" },
                if r.output_connected { "up" } else { "DOWN" },
                r.reconnects,
            );
            if let Some(query) = switch_input
                .as_deref()
                .filter(|_| r.elapsed_s >= options.switch_after_s as f64)
            {
                let target = pick(&list, query, Some(DataFlow::Capture))
                    .map(|endpoint| endpoint.id)
                    .unwrap_or_else(|_| query.to_owned());
                println!("       switching input to {target}");
                pipeline.set_input(Some(target));
                switch_input = None;
            }
            if let Some(cable_output) = probe_target
                .as_deref()
                .filter(|_| options.probe_every_s > 0 && Instant::now() >= next_probe)
            {
                match probe::measure_added_latency(&input.id, cable_output, 2.0) {
                    Ok(m) => println!(
                        "       probe: added latency {:.2} ms (correlation {:.3}, mic {:.1} dBFS RMS)",
                        m.added_latency_ms, m.correlation, m.direct_rms_dbfs
                    ),
                    Err(error) => println!("       probe failed: {error}"),
                }
                next_probe = Instant::now() + Duration::from_secs(options.probe_every_s);
            }
        }
        pipeline.stop()?;
        Ok(())
    }

    fn pick(list: &[Endpoint], query: &str, flow: Option<DataFlow>) -> anyhow::Result<Endpoint> {
        let query_lower = query.to_lowercase();
        let matches: Vec<_> = list
            .iter()
            .filter(|endpoint| flow.is_none_or(|flow| endpoint.flow == flow))
            .filter(|endpoint| {
                endpoint.id == query || endpoint.name.to_lowercase().contains(&query_lower)
            })
            .collect();
        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!("no active endpoint matches '{query}'"),
            many => bail!(
                "'{query}' is ambiguous: {}",
                many.iter()
                    .map(|endpoint| endpoint.name.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        }
    }

    fn describe(list: &[Endpoint], id: Option<&str>) -> String {
        match id {
            None => "-".into(),
            Some(id) => list
                .iter()
                .find(|endpoint| endpoint.id == id)
                .map_or_else(|| id.to_owned(), |endpoint| endpoint.name.clone()),
        }
    }
}
