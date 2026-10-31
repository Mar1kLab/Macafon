//! Mutter's versioned DisplayConfig tuples. Unknown a{sv} properties remain optional.
//! Source contract: GNOME/mutter data/dbus-interfaces/org.gnome.Mutter.DisplayConfig.xml.
use crate::{Applied, DisplayError, Inventory, Plan};
use macafon_ipc::{
    DisplayIdentity, DisplayLayout, DisplayMode, DisplayMonitor, DisplayRotation, ProfileMonitor,
};
use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::OwnedValue;

pub(crate) const DESTINATION: &str = "org.gnome.Mutter.DisplayConfig";
pub(crate) const PATH: &str = "/org/gnome/Mutter/DisplayConfig";
type Props = HashMap<String, OwnedValue>;
type Spec = (String, String, String, String);
type Mode = (String, i32, i32, f64, f64, Vec<f64>, Props);
type Physical = (Spec, Vec<Mode>, Props);
type Logical = (i32, i32, f64, u32, bool, Vec<Spec>, Props);
type RawState = (u32, Vec<Physical>, Vec<Logical>, Props);
type MonitorConfig = (String, String, Props);
type LogicalConfig = (i32, i32, f64, u32, bool, Vec<MonitorConfig>);

pub(crate) struct State {
    raw: RawState,
}
pub(crate) fn connection() -> Result<zbus::blocking::Connection, DisplayError> {
    Ok(zbus::blocking::connection::Builder::session()?
        .method_timeout(Duration::from_secs(2))
        .build()?)
}
pub(crate) fn current() -> Result<State, DisplayError> {
    let bus = connection()?;
    let reply = bus.call_method(
        Some(DESTINATION),
        PATH,
        Some(DESTINATION),
        "GetCurrentState",
        &(),
    )?;
    Ok(State {
        raw: reply.body().deserialize()?,
    })
}
fn boolean(props: &Props, key: &str) -> bool {
    props
        .get(key)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}
fn unsigned(props: &Props, key: &str) -> Option<u32> {
    props.get(key).and_then(|v| u32::try_from(v).ok())
}
fn display_mode(mode: &Mode) -> Result<DisplayMode, DisplayError> {
    if mode.1 <= 0 || mode.2 <= 0 || !mode.3.is_finite() || mode.3 <= 0.0 || mode.3 > 10_000.0 {
        return Err(DisplayError::Rejected("invalid Mutter mode".into()));
    }
    Ok(DisplayMode {
        width: mode.1 as u32,
        height: mode.2 as u32,
        refresh_mhz: (mode.3 * 1000.0).round() as u32,
    })
}
impl State {
    pub(crate) fn inventory(&self) -> Result<Inventory, DisplayError> {
        let mut result = Inventory {
            monitors: Vec::new(),
            mode_ids: Vec::new(),
            scales: Vec::new(),
            preferred: Vec::new(),
            mirror_supported: true,
        };
        for (spec, modes, props) in &self.raw.1 {
            let logical = self
                .raw
                .2
                .iter()
                .find(|l| l.5.iter().any(|s| s.0 == spec.0));
            let current = modes.iter().find(|mode| boolean(&mode.6, "is-current"));
            let hdr_supported = props
                .get("supported-color-modes")
                .and_then(|v| <Vec<u32>>::try_from(v.try_clone().ok()?).ok())
                .is_some_and(|modes| modes.contains(&1));
            result.preferred.push(
                modes
                    .iter()
                    .find(|m| boolean(&m.6, "is-preferred"))
                    .map(display_mode)
                    .transpose()?,
            );
            let layout = match (logical, current) {
                (Some(logical), Some(mode)) => Some(DisplayLayout {
                    x: logical.0,
                    y: logical.1,
                    mode: display_mode(mode)?,
                    rotation: match logical.3 % 4 {
                        1 => DisplayRotation::Rotate90,
                        2 => DisplayRotation::Rotate180,
                        3 => DisplayRotation::Rotate270,
                        _ => DisplayRotation::None,
                    },
                    primary: logical.4 && logical.5.first().is_some_and(|s| s.0 == spec.0),
                    mirror_of: logical
                        .5
                        .first()
                        .filter(|s| s.0 != spec.0)
                        .map(|s| s.0.clone()),
                    hdr: unsigned(props, "color-mode") == Some(1),
                }),
                (Some(_), None) => {
                    return Err(DisplayError::Rejected(
                        "Mutter active monitor has no current mode".into(),
                    ));
                }
                _ => None,
            };
            let mut ordered: Vec<_> = modes.iter().collect();
            ordered.sort_by_key(|m| {
                std::cmp::Reverse((i64::from(m.1) * i64::from(m.2), (m.3 * 1000.0) as u32))
            });
            result
                .mode_ids
                .push(ordered.iter().map(|m| m.0.clone()).collect());
            result.scales.push(logical.map_or_else(
                || {
                    modes
                        .iter()
                        .find(|m| boolean(&m.6, "is-preferred"))
                        .map_or(1.0, |m| m.4)
                },
                |l| l.2,
            ));
            result.monitors.push(DisplayMonitor {
                identity: DisplayIdentity {
                    path: spec.0.clone(),
                    edid: if spec.1.is_empty() || spec.2.is_empty() {
                        String::new()
                    } else {
                        format!("{}:{}", spec.1, spec.2)
                    },
                    serial: (!spec.3.is_empty()).then(|| spec.3.clone()),
                },
                name: props
                    .get("display-name")
                    .and_then(|v| <&str>::try_from(v).ok())
                    .unwrap_or(&spec.0)
                    .into(),
                built_in: boolean(props, "is-builtin"),
                layout,
                modes: ordered
                    .iter()
                    .map(|m| display_mode(m))
                    .collect::<Result<_, _>>()?,
                hdr_supported,
            });
        }
        if boolean(&self.raw.3, "global-scale-required") {
            let scale = self
                .raw
                .2
                .first()
                .map(|l| l.2)
                .or_else(|| result.scales.first().copied())
                .unwrap_or(1.0);
            result.scales.fill(scale);
        }
        crate::identity::enrich(&mut result.monitors);
        Ok(result)
    }
    fn properties(&self) -> Props {
        let mut props = Props::new();
        if boolean(&self.raw.3, "supports-changing-layout-mode")
            && let Some(mode) = unsigned(&self.raw.3, "layout-mode")
        {
            props.insert("layout-mode".into(), mode.into());
        }
        props
    }
    fn raw_config(&self) -> Result<Vec<LogicalConfig>, DisplayError> {
        self.raw
            .2
            .iter()
            .map(|l| {
                let monitors =
                    l.5.iter()
                        .map(|spec| {
                            let (_, modes, properties) =
                                self.raw.1.iter().find(|p| p.0.0 == spec.0).ok_or_else(|| {
                                    DisplayError::Rejected("missing Mutter monitor".into())
                                })?;
                            let mode = modes
                                .iter()
                                .find(|m| boolean(&m.6, "is-current"))
                                .ok_or_else(|| {
                                    DisplayError::Rejected("missing Mutter current mode".into())
                                })?;
                            let mut props = Props::new();
                            if let Some(color) = unsigned(properties, "color-mode") {
                                props.insert("color-mode".into(), color.into());
                            }
                            if properties.contains_key("is-underscanning") {
                                props.insert(
                                    "underscanning".into(),
                                    boolean(properties, "is-underscanning").into(),
                                );
                            }
                            Ok((spec.0.clone(), mode.0.clone(), props))
                        })
                        .collect::<Result<Vec<_>, DisplayError>>()?;
                Ok((l.0, l.1, l.2, l.3, l.4, monitors))
            })
            .collect()
    }
}
fn send(
    serial: u32,
    method: u32,
    logical: Vec<LogicalConfig>,
    props: Props,
) -> Result<(), DisplayError> {
    connection()?
        .call_method(
            Some(DESTINATION),
            PATH,
            Some(DESTINATION),
            "ApplyMonitorsConfig",
            &(serial, method, logical, props),
        )
        .map_err(|e| DisplayError::Rejected(e.to_string()))?;
    Ok(())
}
fn configuration(
    state: &State,
    inventory: &Inventory,
    plan: &Plan,
) -> Result<Vec<LogicalConfig>, DisplayError> {
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
    plan.groups
        .iter()
        .map(|group| {
            let source = group[0];
            let l = plan.layouts[source].as_ref().expect("enabled mirror group");
            let scale = inventory.scales[source];
            let monitors = group
                .iter()
                .map(|i| {
                    let m = &inventory.monitors[*i];
                    let l = plan.layouts[*i].as_ref().expect("enabled group member");
                    let index = m
                        .modes
                        .iter()
                        .position(|mode| *mode == l.mode)
                        .ok_or_else(|| DisplayError::Rejected("mode disappeared".into()))?;
                    let id = &inventory.mode_ids[*i][index];
                    let (_, modes, props) = &state.raw.1[*i];
                    let mode = modes
                        .iter()
                        .find(|mode| mode.0 == *id)
                        .ok_or_else(|| DisplayError::Rejected("mode disappeared".into()))?;
                    if !mode.5.iter().any(|s| (*s - scale).abs() < 0.001) {
                        return Err(DisplayError::Rejected(
                            "mode does not support the existing monitor scale".into(),
                        ));
                    }
                    let mut config = Props::new();
                    if m.hdr_supported {
                        config.insert("color-mode".into(), u32::from(l.hdr).into());
                    } else if let Some(color) = unsigned(props, "color-mode") {
                        config.insert("color-mode".into(), color.into());
                    }
                    if props.contains_key("is-underscanning") {
                        config.insert(
                            "underscanning".into(),
                            boolean(props, "is-underscanning").into(),
                        );
                    }
                    Ok((m.identity.path.clone(), id.clone(), config))
                })
                .collect::<Result<_, DisplayError>>()?;
            let rotation = match l.rotation {
                DisplayRotation::None => 0,
                DisplayRotation::Rotate90 => 1,
                DisplayRotation::Rotate180 => 2,
                DisplayRotation::Rotate270 => 3,
            };
            Ok((
                l.x.checked_sub(min_x)
                    .ok_or_else(|| DisplayError::Rejected("position overflow".into()))?,
                l.y.checked_sub(min_y)
                    .ok_or_else(|| DisplayError::Rejected("position overflow".into()))?,
                scale,
                rotation,
                l.primary,
                monitors,
            ))
        })
        .collect()
}
pub(crate) fn configure(profile: &[ProfileMonitor], verify: bool) -> Result<Applied, DisplayError> {
    let state = current()?;
    let inventory = state.inventory()?;
    let plan = crate::plan(profile, &inventory)?;
    if !verify {
        send(
            state.raw.0,
            0,
            configuration(&state, &inventory, &plan)?,
            state.properties(),
        )?;
    }
    send(
        state.raw.0,
        if verify { 0 } else { 1 },
        configuration(&state, &inventory, &plan)?,
        state.properties(),
    )?;
    Ok(Applied {
        missing: plan.missing,
    })
}
fn restore_configuration(
    saved: &State,
    current: &State,
    profile: &[ProfileMonitor],
    scales: &[f64],
) -> Result<Vec<LogicalConfig>, DisplayError> {
    let raw = saved.raw_config()?;
    let complete = raw.iter().flat_map(|l| &l.5).all(|(port, id, _)| {
        current
            .raw
            .1
            .iter()
            .any(|(spec, modes, _)| spec.0 == *port && modes.iter().any(|m| m.0 == *id))
    });
    if complete {
        return Ok(raw);
    }
    // Hotplug or a changed mode id must not prevent restoring the surviving displays.
    let mut inventory = current.inventory()?;
    let resolved = crate::logic::resolve(profile, &inventory.monitors);
    for (i, resolved) in resolved.iter().enumerate() {
        if let Some(index) = resolved {
            inventory.scales[*index] = scales[i];
        }
    }
    let plan = crate::plan(profile, &inventory)?;
    configuration(current, &inventory, &plan)
}
pub(crate) fn restore(
    saved: &State,
    profile: &[ProfileMonitor],
    scales: &[f64],
) -> Result<(), DisplayError> {
    let current = current()?;
    // Use a fresh serial. Preserve the exact raw packet while its connectors/modes still exist.
    send(
        current.raw.0,
        0,
        restore_configuration(saved, &current, profile, scales)?,
        saved.properties(),
    )?;
    send(
        current.raw.0,
        1,
        restore_configuration(saved, &current, profile, scales)?,
        saved.properties(),
    )
}

pub(crate) fn keep() -> Result<(), DisplayError> {
    let state = current()?;
    // GNOME Shell may request its own confirmation before writing monitors.xml.
    send(state.raw.0, 2, state.raw_config()?, state.properties())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_mutter_variant_body_and_builds_scaled_mirror_configuration() {
        let spec = |port: &str| (port.into(), "DEL".into(), "U2720Q".into(), port.into());
        let modes = || {
            vec![(
                "1920x1080@59.940".into(),
                1920,
                1080,
                59.94,
                1.25,
                vec![1.0, 1.25, 2.0],
                HashMap::from([("is-current".into(), true.into())]),
            )]
        };
        let raw: RawState = (
            7,
            vec![
                (
                    spec("DP-1"),
                    modes(),
                    HashMap::from([
                        (
                            "supported-color-modes".into(),
                            OwnedValue::try_from(zbus::zvariant::Value::from(vec![0u32, 1]))
                                .expect("color modes"),
                        ),
                        ("color-mode".into(), 1u32.into()),
                    ]),
                ),
                (spec("HDMI-1"), modes(), Props::new()),
            ],
            vec![(
                0,
                0,
                1.25,
                1,
                true,
                vec![spec("DP-1"), spec("HDMI-1")],
                Props::new(),
            )],
            Props::new(),
        );
        let message = zbus::Message::method_call("/org/macafon/Test", "Fixture")
            .expect("message")
            .build(&raw)
            .expect("serialize fixture");
        let state = State {
            raw: message
                .body()
                .deserialize()
                .expect("GetCurrentState signature"),
        };
        let inventory = state.inventory().expect("inventory");
        assert_eq!(inventory.monitors[0].modes[0].refresh_mhz, 59940);
        assert!(inventory.monitors[0].hdr_supported);
        assert!(inventory.monitors[0].layout.as_ref().expect("layout").hdr);
        assert_eq!(
            inventory.monitors[1]
                .layout
                .as_ref()
                .expect("mirror")
                .mirror_of
                .as_deref(),
            Some("DP-1")
        );
        let plan = crate::plan(&crate::as_profile(&inventory.monitors), &inventory).expect("plan");
        let config = configuration(&state, &inventory, &plan).expect("config");
        assert_eq!(config.len(), 1);
        assert_eq!(config[0].2, 1.25);
        assert_eq!(config[0].3, 1);
        assert_eq!(config[0].5.len(), 2);
        assert_eq!(state.raw_config().expect("raw restore")[0].2, 1.25);
        let mut after_unplug = State {
            raw: message.body().deserialize().expect("fixture again"),
        };
        after_unplug.raw.1.remove(0);
        after_unplug.raw.2[0].5.remove(0);
        after_unplug.raw.2[0].2 = 2.0;
        let restored = restore_configuration(
            &state,
            &after_unplug,
            &crate::as_profile(&inventory.monitors),
            &inventory.scales,
        )
        .expect("restore surviving mirror");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].2, 1.25);
        assert_eq!(restored[0].5.len(), 1);
        assert_eq!(restored[0].5[0].0, "HDMI-1");
        assert!(restored[0].4);
    }
}
