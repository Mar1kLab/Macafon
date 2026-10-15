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
    }

    pub fn main() -> anyhow::Result<()> {
        let args = Args::parse();
        let _com = ComGuard::new()?;
        match args.command {
            Command::Devices => devices(),
            Command::Defaults => defaults(),
            Command::SetDefault {
                device,
                restore_after_s,
            } => set_default(&device, restore_after_s),
            Command::Sessions => list_sessions(),
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
                input_id: input.id.clone(),
                output_id: output.id.clone(),
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
        let mut next_probe = Instant::now() + Duration::from_secs(options.probe_every_s);
        while Instant::now() < deadline && pipeline.is_running() {
            std::thread::sleep(Duration::from_secs(1));
            control.collect_garbage();
            let meters = telemetry.take();
            let r = pipeline.report();
            println!(
                "{:>5.0}s in {:>6.1} dBFS out {:>6.1} dBFS | level {:>5.2} ms (buffer min {:>5.2}), render queue {:>5.2} ms, max packet {:>5.2}/{:>5.2} ms, \
                 est. {:>5.1} ms | drift {:>+5} ppm (rates {:.1}/{:.1} Hz) | underruns {} dropped {} skipped {} gaps {}",
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
            );
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
