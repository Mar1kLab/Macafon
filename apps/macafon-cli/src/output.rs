//! Human-readable and JSON presentation of daemon state.

use anyhow::Result;
use macafon_ipc::{AbState, CalibrationReport, MicState, MixerState};

pub(super) fn print_ab(state: &AbState, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(state)?);
        return Ok(());
    }
    let clip = state.clip_ms.map_or("none".to_owned(), |ms| {
        format!("{:.1} s", ms as f32 / 1000.0)
    });
    println!("clip:     {clip}");
    println!(
        "playing:  {}{}",
        state.playing,
        state
            .device
            .as_deref()
            .map_or(String::new(), |device| format!(" → {device}"))
    );
    println!("side:     {:?}", state.side);
    println!("position: {:.1} s", state.position_ms as f32 / 1000.0);
    println!("match:    {}", state.loudness_match);
    Ok(())
}

pub(super) fn print_report(report: &CalibrationReport) {
    println!("noise floor:  {:.1} dBFS", report.noise_floor_dbfs);
    println!(
        "speech:       {:.1} dBFS RMS, peak {:.1} dBFS",
        report.speech_rms_dbfs, report.speech_peak_dbfs
    );
    println!(
        "SNR:          {:.1} dB → {:.1} dB ({:?} noise)",
        report.snr_db, report.snr_after_db, report.noise_level
    );
    println!("gain:         {:+.1} dB", report.gain_db);
    let hum = if report.low_frequency_noise {
        " (low-frequency noise)"
    } else {
        ""
    };
    println!("high-pass:    {:.0} Hz{hum}", report.high_pass_freq_hz);
    println!("noise supp.:  {:.0}%", report.noise_intensity_pct);
    println!(
        "expander:     {:.0} dBFS threshold",
        report.expander_threshold_db
    );
    if report.suggest_studio {
        println!("tip:          the room is noisy; try Studio noise suppression");
    }
}

pub(super) fn print_mixer(state: &MixerState, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(state)?);
        return Ok(());
    }
    println!(
        "mixer: {}  output: {}  master: {:.0}%{}",
        if state.enabled { "on" } else { "off" },
        state.output.as_deref().unwrap_or("-"),
        state.master_volume_pct,
        if state.master_muted { " (muted)" } else { "" }
    );
    for channel in &state.channels {
        println!(
            "  {}{}\t{}\t{:.0}%{}",
            channel.id,
            if channel.id == state.default_channel {
                "*"
            } else {
                ""
            },
            channel.name,
            channel.settings.volume_pct,
            if channel.settings.muted {
                " (muted)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

pub(super) fn print_state(state: &MicState) {
    let latency_ms = state.latency_samples as f32 * 1000.0 / 48_000.0;
    let device = state.device.as_deref().unwrap_or("none");
    let input = match (
        state.device.is_some(),
        state.input_connected && state.active_device == state.device,
    ) {
        (false, _) => "",
        (true, true) => " (connected)",
        (true, false) => " (disconnected)",
    };
    let modified = if state.preset_modified {
        " (modified)"
    } else {
        ""
    };
    println!(
        "preset:   {}{modified}",
        state.preset.as_deref().unwrap_or("custom")
    );
    println!("input:    {device}{input}");
    println!(
        "fallback: {}",
        state.fallback_device.as_deref().unwrap_or("none")
    );
    println!(
        "active:   {}",
        state.active_device.as_deref().unwrap_or("none")
    );
    println!("channel:  {:?}", state.input_channel);
    println!("muted:    {}", state.settings.muted);
    println!("format:   {}", if state.stereo { "stereo" } else { "mono" });
    println!("bypass:   {}", state.settings.bypass);
    println!("gain:     {:+.1} dB", state.settings.gain.params.gain_db);
    println!("latency:  {latency_ms:.2} ms");
}
