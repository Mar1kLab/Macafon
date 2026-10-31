//! CLI adapters use argv directly, a fixed locale and bounded execution time; never a shell.
use crate::{Backend, DisplayError, Inventory, Plan};
use macafon_ipc::{DisplayIdentity, DisplayLayout, DisplayMode, DisplayMonitor, DisplayRotation};
use serde_json::Value;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(2);
fn rejected(message: impl Into<String>) -> DisplayError {
    DisplayError::Rejected(message.into())
}

pub(crate) fn run(program: &str, args: &[String]) -> Result<String, DisplayError> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| rejected("stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| rejected("stderr unavailable"))?;
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(4 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take(64 * 1024)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(DisplayError::Unavailable(format!(
                    "{program}: {}",
                    result
                        .err()
                        .map_or("command timed out".into(), |e| e.to_string())
                )));
            }
        }
    };
    let stdout = out.join().map_err(|_| rejected("stdout reader failed"))??;
    let stderr = err.join().map_err(|_| rejected("stderr reader failed"))??;
    if !status?.success() {
        return Err(rejected(format!(
            "{program}: {}",
            String::from_utf8_lossy(&stderr).trim()
        )));
    }
    String::from_utf8(stdout).map_err(|e| rejected(e.to_string()))
}
fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).into()).collect()
}
pub(crate) fn inventory(backend: Backend) -> Result<Inventory, DisplayError> {
    match backend {
        Backend::Kscreen => {
            let mut inventory = parse_kscreen(&run("kscreen-doctor", &strings(&["-j"]))?)?;
            inventory.mirror_supported = kscreen_mirroring();
            Ok(inventory)
        }
        Backend::WlrRandr => parse_wlr(&run("wlr-randr", &strings(&["--json"]))?),
        Backend::Xrandr => parse_xrandr(&run("xrandr", &strings(&["--verbose"]))?),
        Backend::Mutter => unreachable!("Mutter uses D-Bus"),
    }
}
pub(crate) fn kscreen_mirroring() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        run("kscreen-doctor", &strings(&["--help"])).is_ok_and(|help| help.contains(".mirror."))
    })
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn integer(value: &Value, key: &str) -> Result<i32, DisplayError> {
    value[key]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| rejected(format!("invalid {key}")))
}
fn mode(value: &Value, kscreen: bool) -> Result<DisplayMode, DisplayError> {
    let size = if kscreen { &value["size"] } else { value };
    let width = integer(size, "width")?;
    let height = integer(size, "height")?;
    let hz = value[if kscreen { "refreshRate" } else { "refresh" }]
        .as_f64()
        .unwrap_or(0.0);
    if width <= 0 || height <= 0 || !hz.is_finite() || hz <= 0.0 || hz > 10_000.0 {
        return Err(rejected("invalid display mode"));
    }
    Ok(DisplayMode {
        width: width as u32,
        height: height as u32,
        refresh_mhz: (hz * 1000.0).round() as u32,
    })
}
fn new_monitor(path: &str) -> DisplayMonitor {
    DisplayMonitor {
        identity: DisplayIdentity {
            edid: String::new(),
            serial: None,
            path: path.into(),
        },
        name: path.into(),
        built_in: path.starts_with("eDP") || path.starts_with("LVDS") || path.starts_with("DSI"),
        layout: None,
        modes: Vec::new(),
        hdr_supported: false,
    }
}
fn layout(
    mode: DisplayMode,
    x: i32,
    y: i32,
    rotation: DisplayRotation,
    primary: bool,
    hdr: bool,
) -> DisplayLayout {
    DisplayLayout {
        mode,
        x,
        y,
        rotation,
        primary,
        hdr,
        mirror_of: None,
    }
}
fn sorted(inventory: &mut Inventory) {
    for (monitor, ids) in inventory.monitors.iter_mut().zip(&mut inventory.mode_ids) {
        let mut modes: Vec<_> = monitor.modes.drain(..).zip(ids.drain(..)).collect();
        modes.sort_by_key(|(m, _)| {
            std::cmp::Reverse((u64::from(m.width) * u64::from(m.height), m.refresh_mhz))
        });
        (monitor.modes, *ids) = modes.into_iter().unzip();
    }
}
fn scale(value: &Value) -> f64 {
    value
        .as_f64()
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(1.0)
}

pub(crate) fn parse_kscreen(input: &str) -> Result<Inventory, DisplayError> {
    let root: Value = serde_json::from_str(input).map_err(|e| rejected(e.to_string()))?;
    let outputs = root["outputs"]
        .as_array()
        .ok_or_else(|| rejected("missing outputs"))?;
    let mut result = Inventory {
        monitors: Vec::new(),
        mode_ids: Vec::new(),
        scales: Vec::new(),
        preferred: Vec::new(),
        mirror_supported: false,
    };
    let primary = outputs
        .iter()
        .filter(|o| o["connected"] == true && o["enabled"] != false)
        .filter_map(|o| {
            o["priority"]
                .as_u64()
                .filter(|p| *p > 0)
                .map(|p| (p, text(o, "name")))
        })
        .min_by_key(|(p, _)| *p)
        .map(|(_, n)| n);
    for output in outputs.iter().filter(|o| o["connected"] == true) {
        let name = text(output, "name");
        if name.is_empty() {
            return Err(rejected("output name is missing"));
        }
        let mut monitor = new_monitor(name);
        let modes = output["modes"]
            .as_array()
            .ok_or_else(|| rejected("missing modes"))?;
        let mut ids = Vec::new();
        for entry in modes {
            monitor.modes.push(mode(entry, true)?);
            ids.push(text(entry, "id").to_owned());
        }
        let preferred = output["preferredModes"]
            .as_array()
            .and_then(|modes| modes.first())
            .and_then(Value::as_str)
            .and_then(|id| ids.iter().position(|candidate| candidate == id))
            .map(|i| monitor.modes[i]);
        result.preferred.push(preferred);
        monitor.hdr_supported = output["hdr"].is_boolean();
        if output["enabled"] != false {
            let i = ids
                .iter()
                .position(|id| id == text(output, "currentModeId"))
                .ok_or_else(|| rejected("current mode not found"))?;
            let rotation = match output["rotation"].as_u64().unwrap_or(1) {
                2 => DisplayRotation::Rotate90,
                4 => DisplayRotation::Rotate180,
                8 => DisplayRotation::Rotate270,
                _ => DisplayRotation::None,
            };
            monitor.layout = Some(layout(
                monitor.modes[i],
                integer(&output["pos"], "x")?,
                integer(&output["pos"], "y")?,
                rotation,
                output["primary"] == true || primary == Some(name),
                output["hdr"] == true,
            ));
            if let Some(source) = output["replicationSource"].as_i64().filter(|i| *i > 0)
                && let Some(other) = outputs.iter().find(|o| o["id"].as_i64() == Some(source))
            {
                monitor.layout.as_mut().expect("layout set above").mirror_of =
                    Some(text(other, "name").into());
            }
        }
        result.monitors.push(monitor);
        result.mode_ids.push(ids);
        result.scales.push(scale(&output["scale"]));
    }
    sorted(&mut result);
    Ok(result)
}
pub(crate) fn parse_wlr(input: &str) -> Result<Inventory, DisplayError> {
    let root: Value = serde_json::from_str(input).map_err(|e| rejected(e.to_string()))?;
    let outputs = root
        .as_array()
        .ok_or_else(|| rejected("expected output array"))?;
    let mut result = Inventory {
        monitors: Vec::new(),
        mode_ids: Vec::new(),
        scales: Vec::new(),
        preferred: Vec::new(),
        mirror_supported: false,
    };
    for output in outputs {
        let name = text(output, "name");
        if name.is_empty() {
            return Err(rejected("missing output name"));
        }
        let mut monitor = new_monitor(name);
        monitor.identity.edid =
            if text(output, "make").is_empty() || text(output, "model").is_empty() {
                String::new()
            } else {
                format!("{}:{}", text(output, "make"), text(output, "model"))
            };
        monitor.identity.serial = output["serial"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if !text(output, "description").is_empty() {
            monitor.name = text(output, "description").into();
        }
        let modes = output["modes"]
            .as_array()
            .ok_or_else(|| rejected("missing modes"))?;
        for entry in modes {
            monitor.modes.push(mode(entry, false)?);
        }
        result.preferred.push(
            modes
                .iter()
                .position(|m| m["preferred"] == true)
                .map(|i| monitor.modes[i]),
        );
        if output["enabled"] == true {
            let current = modes
                .iter()
                .position(|m| m["current"] == true)
                .ok_or_else(|| rejected("current mode not found"))?;
            let rotation = match text(output, "transform") {
                "90" => DisplayRotation::Rotate90,
                "180" => DisplayRotation::Rotate180,
                "270" => DisplayRotation::Rotate270,
                _ => DisplayRotation::None,
            };
            monitor.layout = Some(layout(
                monitor.modes[current],
                integer(&output["position"], "x")?,
                integer(&output["position"], "y")?,
                rotation,
                false,
                false,
            ));
        }
        result
            .mode_ids
            .push(vec![String::new(); monitor.modes.len()]);
        result.scales.push(scale(&output["scale"]));
        result.monitors.push(monitor);
    }
    sorted(&mut result);
    Ok(result)
}

pub(crate) fn parse_xrandr(input: &str) -> Result<Inventory, DisplayError> {
    let mut result = Inventory {
        monitors: Vec::new(),
        mode_ids: Vec::new(),
        scales: Vec::new(),
        preferred: Vec::new(),
        mirror_supported: false,
    };
    let lines: Vec<_> = input.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let words: Vec<_> = line.split_whitespace().collect();
        if line.starts_with(char::is_whitespace) || words.get(1) != Some(&"connected") {
            index += 1;
            continue;
        }
        let mut monitor = new_monitor(words[0]);
        let mut ids = Vec::new();
        let primary = words.contains(&"primary");
        let geometry = words.iter().find_map(|word| geometry(word));
        let rotation = line
            .split(" (0x")
            .nth(1)
            .and_then(|s| s.split(')').nth(1))
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("normal");
        let rotation = match rotation {
            "left" => DisplayRotation::Rotate90,
            "inverted" => DisplayRotation::Rotate180,
            "right" => DisplayRotation::Rotate270,
            _ => DisplayRotation::None,
        };
        index += 1;
        let mut current = None;
        let mut preferred = None;
        while index < lines.len() && lines[index].starts_with(char::is_whitespace) {
            let line = lines[index].trim();
            if line == "EDID:" {
                let mut hex = String::new();
                let mut j = index + 1;
                while j < lines.len() {
                    let part = lines[j].trim();
                    if part.len() != 32 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
                        break;
                    }
                    hex.push_str(part);
                    j += 1;
                }
                let bytes: Vec<_> = (0..hex.len())
                    .step_by(2)
                    .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                    .collect();
                if let Some((id, name)) = crate::identity::edid(&bytes, words[0]) {
                    monitor.identity = id;
                    if let Some(name) = name {
                        monitor.name = name;
                    }
                }
            }
            // Verbose modelines: name (XID) clock ... followed by h: and v: timing rows.
            let tokens: Vec<_> = line.split_whitespace().collect();
            if tokens.get(1).is_some_and(|s| s.starts_with("(0x")) && index + 2 < lines.len() {
                let h: Vec<_> = lines[index + 1].split_whitespace().collect();
                let v: Vec<_> = lines[index + 2].split_whitespace().collect();
                let field = |tokens: &[&str], key: &str| {
                    tokens
                        .iter()
                        .position(|s| *s == key)
                        .and_then(|i| tokens.get(i + 1))
                        .and_then(|s| s.parse::<u32>().ok())
                };
                if let (Some(width), Some(height), Some(hz)) = (
                    field(&h, "width"),
                    field(&v, "height"),
                    v.iter()
                        .position(|s| *s == "clock")
                        .and_then(|i| v.get(i + 1))
                        .and_then(|s| s.trim_end_matches("Hz").parse::<f64>().ok()),
                ) {
                    let mode = DisplayMode {
                        width,
                        height,
                        refresh_mhz: (hz * 1000.0).round() as u32,
                    };
                    if width > 0 && height > 0 && hz.is_finite() && hz > 0.0 {
                        if tokens.contains(&"+preferred") {
                            preferred = Some(mode);
                        }
                        if tokens.contains(&"*current") {
                            current = Some(mode);
                        }
                        monitor.modes.push(mode);
                        ids.push(tokens[0].to_owned());
                    }
                }
            }
            index += 1;
        }
        if let Some((x, y)) = geometry {
            let mode =
                current.ok_or_else(|| rejected("active xrandr output has no current mode"))?;
            monitor.layout = Some(layout(mode, x, y, rotation, primary, false));
        }
        result.monitors.push(monitor);
        result.mode_ids.push(ids);
        result.scales.push(1.0);
        result.preferred.push(preferred);
    }
    // XRandR clones are outputs at the same desktop origin and with the same dimensions.
    let mut sources: Vec<(usize, String, DisplayMode, i32, i32)> = Vec::new();
    let mut order: Vec<_> = (0..result.monitors.len()).collect();
    order.sort_by_key(|i| {
        !result.monitors[*i]
            .layout
            .as_ref()
            .is_some_and(|l| l.primary)
    });
    for i in order {
        let path = result.monitors[i].identity.path.clone();
        if let Some(l) = &mut result.monitors[i].layout {
            if let Some((_, source, ..)) = sources.iter().find(|(_, _, m, x, y)| {
                *x == l.x && *y == l.y && m.width == l.mode.width && m.height == l.mode.height
            }) {
                l.mirror_of = Some(source.clone());
                l.primary = false;
            } else {
                sources.push((i, path, l.mode, l.x, l.y));
            }
        }
    }
    sorted(&mut result);
    Ok(result)
}
fn geometry(word: &str) -> Option<(i32, i32)> {
    let split = word.find(['+', '-'])?;
    let size = &word[..split];
    let (w, h) = size.split_once('x')?;
    w.parse::<u32>().ok()?;
    h.parse::<u32>().ok()?;
    let rest = &word[split..];
    let next = rest[1..].find(['+', '-'])? + 1;
    Some((rest[..next].parse().ok()?, rest[next..].parse().ok()?))
}

pub(crate) fn arguments(
    backend: Backend,
    inventory: &Inventory,
    plan: &Plan,
) -> Result<Vec<String>, DisplayError> {
    let mut args = Vec::new();
    // KScreen / wlroots use nonnegative logical coordinates; shift the whole desktop together.
    let min_x = plan
        .layouts
        .iter()
        .flatten()
        .map(|l| l.x)
        .min()
        .unwrap_or(0);
    let min_y = plan
        .layouts
        .iter()
        .flatten()
        .map(|l| l.y)
        .min()
        .unwrap_or(0);
    for (i, (monitor, layout)) in inventory.monitors.iter().zip(&plan.layouts).enumerate() {
        let path = &monitor.identity.path;
        if backend == Backend::Kscreen
            && (path.contains('.') || path.chars().any(char::is_whitespace))
        {
            return Err(rejected("unsupported KScreen connector name"));
        }
        if backend != Backend::Kscreen {
            args.extend(["--output".into(), path.clone()]);
        }
        let Some(layout) = layout else {
            args.push(if backend == Backend::Kscreen {
                format!("output.{path}.disable")
            } else {
                "--off".into()
            });
            continue;
        };
        let mode_index = monitor
            .modes
            .iter()
            .position(|m| *m == layout.mode)
            .ok_or_else(|| rejected("mode disappeared"))?;
        let x = layout
            .x
            .checked_sub(min_x)
            .ok_or_else(|| rejected("position overflow"))?;
        let y = layout
            .y
            .checked_sub(min_y)
            .ok_or_else(|| rejected("position overflow"))?;
        let hz = format!("{:.3}", f64::from(layout.mode.refresh_mhz) / 1000.0);
        let rotation = match layout.rotation {
            DisplayRotation::None => 0,
            DisplayRotation::Rotate90 => 1,
            DisplayRotation::Rotate180 => 2,
            DisplayRotation::Rotate270 => 3,
        };
        match backend {
            Backend::Kscreen => {
                args.extend([
                    format!("output.{path}.enable"),
                    format!("output.{path}.mode.{}", inventory.mode_ids[i][mode_index]),
                    format!("output.{path}.position.{x},{y}"),
                    format!(
                        "output.{path}.rotation.{}",
                        ["normal", "left", "inverted", "right"][rotation]
                    ),
                ]);
                args.push(format!("output.{path}.scale.{}", inventory.scales[i]));
                if layout.primary {
                    args.push(format!("output.{path}.primary"));
                }
                if inventory.mirror_supported {
                    args.push(format!(
                        "output.{path}.mirror.{}",
                        layout.mirror_of.as_deref().unwrap_or("none")
                    ));
                }
                if monitor.hdr_supported {
                    args.push(format!(
                        "output.{path}.hdr.{}",
                        if layout.hdr { "enable" } else { "disable" }
                    ));
                }
            }
            Backend::WlrRandr => {
                args.extend([
                    "--on".into(),
                    "--mode".into(),
                    format!("{}x{}@{hz}Hz", layout.mode.width, layout.mode.height),
                    "--pos".into(),
                    format!("{x},{y}"),
                    "--transform".into(),
                    ["normal", "90", "180", "270"][rotation].into(),
                    "--scale".into(),
                    inventory.scales[i].to_string(),
                ]);
            }
            Backend::Xrandr => {
                args.extend([
                    "--mode".into(),
                    inventory.mode_ids[i][mode_index].clone(),
                    "--rate".into(),
                    hz,
                    "--pos".into(),
                    format!("{x}x{y}"),
                    "--rotate".into(),
                    ["normal", "left", "inverted", "right"][rotation].into(),
                ]);
                if layout.primary {
                    args.push("--primary".into());
                }
                if let Some(source) = &layout.mirror_of {
                    args.extend(["--same-as".into(), source.clone()]);
                }
            }
            Backend::Mutter => unreachable!("Mutter uses D-Bus"),
        }
    }
    Ok(args)
}
pub(crate) fn apply(
    backend: Backend,
    inventory: &Inventory,
    plan: &Plan,
    verify: bool,
) -> Result<(), DisplayError> {
    let mut args = arguments(backend, inventory, plan)?;
    let program = match backend {
        Backend::Kscreen => "kscreen-doctor",
        Backend::WlrRandr => "wlr-randr",
        Backend::Xrandr => "xrandr",
        Backend::Mutter => unreachable!("Mutter uses D-Bus"),
    };
    if verify {
        match backend {
            Backend::Kscreen => return Ok(()),
            Backend::WlrRandr => args.push("--dryrun".into()),
            Backend::Xrandr => args.push("--dryrun".into()),
            _ => {}
        }
    }
    run(program, &args).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) const KSCREEN: &str = r#"{"outputs":[
      {"id":1,"name":"eDP-1","connected":true,"enabled":true,"priority":2,"pos":{"x":0,"y":0},"rotation":1,"scale":1.25,"currentModeId":"7","modes":[{"id":"7","size":{"width":1920,"height":1080},"refreshRate":59.94}],"preferredModes":["7"]},
      {"id":2,"name":"DP-1","connected":true,"enabled":true,"priority":1,"pos":{"x":1536,"y":0},"rotation":2,"scale":1.0,"hdr":true,"currentModeId":"9","modes":[{"id":"9","size":{"width":2560,"height":1440},"refreshRate":144.0}],"preferredModes":["9"]},
      {"id":3,"name":"HDMI-A-1","connected":true,"enabled":false,"priority":0,"modes":[{"id":"3","size":{"width":1920,"height":1080},"refreshRate":60.0}]},
      {"id":4,"name":"DP-2","connected":false}
    ]}"#;
    pub(super) const WLR: &str = r#"[
      {"name":"DP-1","description":"Dell Monitor","make":"Dell Inc.","model":"U2720Q","serial":"1234","enabled":true,"modes":[{"width":1920,"height":1080,"refresh":59.94,"preferred":true,"current":true},{"width":2560,"height":1440,"refresh":60.0,"preferred":false,"current":false}],"position":{"x":0,"y":0},"transform":"270","scale":1.25},
      {"name":"HDMI-A-1","make":null,"model":null,"serial":null,"enabled":false,"modes":[{"width":1920,"height":1080,"refresh":60.0,"preferred":true,"current":false}]}
    ]"#;
    const XRANDR: &str = include_str!("fixtures/xrandr-verbose.txt");

    fn plan_for(inventory: &Inventory) -> Plan {
        crate::plan(&crate::as_profile(&inventory.monitors), inventory).expect("profile plan")
    }
    #[test]
    fn kscreen_json_and_apply_arguments_preserve_ids_hdr_and_rotation() {
        let inventory = parse_kscreen(KSCREEN).expect("KScreen JSON");
        assert_eq!(inventory.monitors.len(), 3);
        assert!(inventory.monitors[0].built_in);
        assert_eq!(inventory.monitors[0].modes[0].refresh_mhz, 59940);
        assert!(
            inventory.monitors[1]
                .layout
                .as_ref()
                .expect("active")
                .primary
        );
        assert_eq!(
            inventory.monitors[1]
                .layout
                .as_ref()
                .expect("active")
                .rotation,
            DisplayRotation::Rotate90
        );
        assert!(inventory.monitors[2].layout.is_none());
        let args = arguments(Backend::Kscreen, &inventory, &plan_for(&inventory)).expect("argv");
        for expected in [
            "output.eDP-1.mode.7",
            "output.DP-1.rotation.left",
            "output.DP-1.hdr.enable",
            "output.DP-1.primary",
            "output.HDMI-A-1.disable",
            "output.DP-1.position.1536,0",
        ] {
            assert!(
                args.iter().any(|a| a == expected),
                "missing {expected}: {args:?}"
            );
        }
        assert!(!args.iter().any(|a| a.starts_with("output.eDP-1.hdr")));
    }
    #[test]
    fn wlr_json_uses_hz_preserves_scale_and_emits_no_unsupported_primary_hdr() {
        let inventory = parse_wlr(WLR).expect("wlr JSON");
        let monitor = &inventory.monitors[0];
        assert_eq!(monitor.modes[0].width, 2560);
        let l = monitor.layout.as_ref().expect("active");
        assert_eq!(l.mode.refresh_mhz, 59940);
        assert_eq!(l.rotation, DisplayRotation::Rotate270);
        assert_eq!(monitor.identity.serial.as_deref(), Some("1234"));
        let args = arguments(Backend::WlrRandr, &inventory, &plan_for(&inventory)).expect("argv");
        assert_eq!(
            args,
            strings(&[
                "--output",
                "DP-1",
                "--on",
                "--mode",
                "1920x1080@59.940Hz",
                "--pos",
                "0,0",
                "--transform",
                "270",
                "--scale",
                "1.25",
                "--output",
                "HDMI-A-1",
                "--off"
            ])
        );
    }
    #[test]
    fn xrandr_verbose_reads_rotated_physical_mode_inactive_outputs_and_refresh() {
        let inventory = parse_xrandr(XRANDR).expect("verbose xrandr");
        assert_eq!(inventory.monitors.len(), 2);
        let monitor = &inventory.monitors[0];
        assert_eq!(monitor.modes.len(), 2);
        assert_eq!(monitor.modes[1].refresh_mhz, 59940);
        let l = monitor.layout.as_ref().expect("active");
        assert_eq!(l.mode.width, 1920);
        assert_eq!(l.rotation, DisplayRotation::Rotate90);
        assert!(inventory.monitors[1].layout.is_none());
        let args = arguments(Backend::Xrandr, &inventory, &plan_for(&inventory)).expect("argv");
        assert_eq!(
            args,
            strings(&[
                "--output",
                "DP-1",
                "--mode",
                "1920x1080",
                "--rate",
                "60.000",
                "--pos",
                "0x0",
                "--rotate",
                "left",
                "--primary",
                "--output",
                "HDMI-1",
                "--off"
            ])
        );
        assert_eq!(geometry("1920x1080-1920+50"), Some((-1920, 50)));
    }
    #[test]
    fn xrandr_verbose_edid_and_clone_target_are_captured() {
        let mut bytes = [0u8; 128];
        bytes[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
        bytes[8..10].copy_from_slice(&0x1e6du16.to_be_bytes());
        bytes[10..12].copy_from_slice(&0x5b7fu16.to_le_bytes());
        bytes[127] = 0u8.wrapping_sub(bytes[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b)));
        let edid = bytes
            .chunks(16)
            .map(|row| {
                format!(
                    "\t\t{}\n",
                    row.iter().map(|b| format!("{b:02x}")).collect::<String>()
                )
            })
            .collect::<String>();
        let input = XRANDR.replace("\tIdentifier: 0x42\n", &format!("\tEDID:\n{edid}"));
        assert_eq!(
            parse_xrandr(&input).expect("EDID inventory").monitors[0]
                .identity
                .edid,
            "GSM5B7F"
        );
        let mut inventory = parse_xrandr(XRANDR).expect("inventory");
        inventory.monitors[1].layout = inventory.monitors[0].layout.clone();
        let child = inventory.monitors[1].layout.as_mut().expect("clone");
        child.primary = false;
        child.mirror_of = Some("DP-1".into());
        let args =
            arguments(Backend::Xrandr, &inventory, &plan_for(&inventory)).expect("clone arguments");
        assert!(args.windows(2).any(|a| a == ["--same-as", "DP-1"]));
    }
    #[test]
    fn validation_rejects_all_off_missing_modes_and_mirror_cycles() {
        let inventory = parse_kscreen(KSCREEN).expect("inventory");
        let mut profile = crate::as_profile(&inventory.monitors);
        for p in &mut profile {
            p.layout = None;
        }
        assert!(matches!(
            crate::plan(&profile, &inventory),
            Err(DisplayError::NothingToTurnOn)
        ));
        let mut profile = crate::as_profile(&inventory.monitors);
        profile[0].layout.as_mut().expect("layout").mode.width = 9999;
        assert!(matches!(
            crate::plan(&profile, &inventory),
            Err(DisplayError::Rejected(_))
        ));
        let mut profile = crate::as_profile(&inventory.monitors);
        profile[0].layout.as_mut().expect("layout").mirror_of = Some("DP-1".into());
        profile[1].layout.as_mut().expect("layout").mirror_of = Some("eDP-1".into());
        assert!(matches!(
            crate::plan(&profile, &inventory),
            Err(DisplayError::Rejected(_))
        ));
        assert!(parse_kscreen("{}").is_err());
        assert!(parse_wlr("{}").is_err());
        assert!(parse_xrandr("DP-1 connected 1920x1080+0+0\n").is_err());
    }
}

#[cfg(test)]
mod desktop_safety_tests {
    use super::*;
    #[test]
    fn all_on_uses_preferred_modes_and_logical_widths() {
        let inventory = parse_wlr(super::tests::WLR).expect("inventory");
        let profile = crate::all_on_profile(&inventory).expect("all on");
        let first = profile[0].layout.as_ref().expect("first output");
        let second = profile[1].layout.as_ref().expect("second output");
        assert_eq!(first.mode.width, 1920); // Preferred, even though 2560 sorts first.
        assert_eq!(second.x, 864); // Rotated 1080px / 1.25 scale.
        assert!(first.primary);
        assert!(!second.primary);
        assert!(profile.iter().all(|m| m.layout.is_some()));
    }
    #[test]
    fn kscreen_mirror_arguments_follow_resolved_connector_names() {
        let mut inventory = parse_kscreen(super::tests::KSCREEN).expect("inventory");
        inventory.mirror_supported = true;
        inventory.monitors[0].identity.serial = Some("panel".into());
        inventory.monitors[0].identity.edid = "GSM1234".into();
        let mut profile = crate::as_profile(&inventory.monitors);
        profile[0].identity.path = "old-port".into();
        profile[0].layout.as_mut().expect("source").primary = true;
        let child = profile[1].layout.as_mut().expect("child");
        child.primary = false;
        child.mirror_of = Some("old-port".into());
        let plan = crate::plan(&profile, &inventory).expect("resolve and project mirror");
        let args = arguments(Backend::Kscreen, &inventory, &plan).expect("argv");
        assert!(args.contains(&"output.DP-1.mirror.eDP-1".to_owned()));
        assert!(args.contains(&"output.eDP-1.mirror.none".to_owned()));
        inventory.mirror_supported = false;
        let args = arguments(Backend::Kscreen, &inventory, &plan).expect("older KScreen argv");
        assert!(!args.iter().any(|a| a.contains(".mirror.")));
    }
}
