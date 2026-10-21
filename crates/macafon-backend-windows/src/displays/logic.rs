//! Profile operations independent of Windows handles and driver identifiers.
use macafon_ipc::{
    DisplayIdentity, DisplayLayout, DisplayMode, DisplayMonitor, DisplayRotation, ProfileMonitor,
};

/// Resolve exact paths first across the entire set, then unambiguous EDID identities.
/// Never let an EDID fallback steal a monitor with an exact path match later in the profile.
pub(super) fn resolve(
    profile: &[ProfileMonitor],
    monitors: &[DisplayMonitor],
) -> Vec<Option<usize>> {
    let mut result = vec![None; profile.len()];
    let mut used = vec![false; monitors.len()];
    for pass in 0..3 {
        for (i, saved) in profile.iter().enumerate() {
            if result[i].is_some() {
                continue;
            }
            let candidates: Vec<_> = monitors
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    let a = &saved.identity;
                    let b = &m.identity;
                    match pass {
                        0 => !a.path.is_empty() && a.path.eq_ignore_ascii_case(&b.path),
                        1 => {
                            !a.edid.is_empty()
                                && a.edid == b.edid
                                && a.serial.is_some()
                                && a.serial == b.serial
                        }
                        _ => !a.edid.is_empty() && a.edid == b.edid,
                    }
                })
                .map(|(j, _)| j)
                .collect();
            if let [j] = candidates.as_slice() {
                if !used[*j] {
                    result[i] = Some(*j);
                    used[*j] = true;
                }
            }
        }
    }
    result
}

pub(super) fn normalize(layouts: &mut [Option<DisplayLayout>]) -> Result<(), &'static str> {
    if layouts.iter().all(Option::is_none) {
        return Ok(());
    }
    let primary_index = layouts
        .iter()
        .position(|l| {
            l.as_ref()
                .is_some_and(|l| l.primary && l.mirror_of.is_none())
        })
        .or_else(|| {
            layouts
                .iter()
                .position(|l| l.as_ref().is_some_and(|l| l.mirror_of.is_none()))
        })
        .ok_or("a mirror needs an enabled source monitor")?;
    let primary = layouts[primary_index]
        .as_ref()
        .ok_or("primary monitor is off")?;
    let origin = (primary.x, primary.y);
    for (i, layout) in layouts.iter_mut().enumerate() {
        if let Some(l) = layout {
            l.x =
                l.x.checked_sub(origin.0)
                    .ok_or("display position overflow")?;
            l.y =
                l.y.checked_sub(origin.1)
                    .ok_or("display position overflow")?;
            l.primary = i == primary_index;
        }
    }
    Ok(())
}

pub(super) struct Connected {
    pub layouts: Vec<Option<DisplayLayout>>,
    pub groups: Vec<Vec<usize>>,
}

/// Project mirror groups onto connected monitors. If the original mirror source is
/// missing, a surviving member becomes the group's source, retaining its target mode.
pub(super) fn connected(
    profile: &[ProfileMonitor],
    resolved: &[Option<usize>],
) -> Result<Connected, &'static str> {
    let mut layouts: Vec<_> = profile
        .iter()
        .enumerate()
        .map(|(i, p)| resolved[i].and(p.layout.clone()))
        .collect();
    let mut roots = vec![None; profile.len()];
    for (i, layout) in layouts.iter().enumerate() {
        if layout.is_none() {
            continue;
        }
        let mut root = i;
        for depth in 0..=profile.len() {
            if depth == profile.len() {
                return Err("cyclic monitor mirror references");
            }
            let l = profile[root]
                .layout
                .as_ref()
                .ok_or("mirror source monitor is off")?;
            let Some(path) = &l.mirror_of else {
                break;
            };
            root = profile
                .iter()
                .position(|p| p.identity.path.eq_ignore_ascii_case(path))
                .ok_or("unknown mirror source monitor")?;
        }
        roots[i] = Some(root);
    }
    let mut groups = Vec::new();
    for root in 0..profile.len() {
        let members: Vec<_> = (0..profile.len())
            .filter(|j| roots[*j] == Some(root))
            .collect();
        if members.is_empty() {
            continue;
        }
        let source = if members.contains(&root) {
            root
        } else {
            members[0]
        };
        let mut group = vec![source];
        group.extend(members.into_iter().filter(|i| *i != source));
        let primary = profile[root].layout.as_ref().is_some_and(|l| l.primary);
        for i in &group {
            if let Some(l) = &mut layouts[*i] {
                l.mirror_of = (*i != source).then(|| profile[source].identity.path.clone());
                l.primary = *i == source && primary;
            }
        }
        groups.push(group);
    }
    normalize(&mut layouts)?;
    groups.sort_by_key(|g| !layouts[g[0]].as_ref().is_some_and(|l| l.primary));
    Ok(Connected { layouts, groups })
}

pub(super) fn mode_matches(a: DisplayMode, b: DisplayMode) -> bool {
    a.width == b.width && a.height == b.height && a.refresh_mhz.abs_diff(b.refresh_mhz) <= 500
}

fn mirror_target(path: &Option<String>, identities: &[DisplayIdentity]) -> Option<usize> {
    path.as_ref().and_then(|p| {
        identities
            .iter()
            .position(|id| id.path.eq_ignore_ascii_case(p))
    })
}

pub fn matches(profile: &[ProfileMonitor], monitors: &[DisplayMonitor]) -> bool {
    let resolved = resolve(profile, monitors);
    let Ok(Connected {
        layouts: wanted, ..
    }) = connected(profile, &resolved)
    else {
        return false;
    };
    let mut actual: Vec<_> = monitors.iter().map(|m| m.layout.clone()).collect();
    if normalize(&mut actual).is_err() {
        return false;
    }
    let saved_ids: Vec<_> = profile.iter().map(|p| p.identity.clone()).collect();
    let live_ids: Vec<_> = monitors.iter().map(|m| m.identity.clone()).collect();
    for (j, monitor) in monitors.iter().enumerate() {
        let i = resolved.iter().position(|r| *r == Some(j));
        let expected = i.and_then(|i| wanted[i].as_ref());
        match (expected, actual[j].as_ref()) {
            (None, None) => {}
            (Some(a), Some(b)) => {
                let saved_mirror =
                    mirror_target(&a.mirror_of, &saved_ids).and_then(|i| resolved[i]);
                let live_mirror = mirror_target(&b.mirror_of, &live_ids);
                // Invalid mirror references cannot silently match an extended monitor.
                if a.mirror_of.is_some() != b.mirror_of.is_some()
                    || saved_mirror != live_mirror
                    || a.x != b.x
                    || a.y != b.y
                    || a.primary != b.primary
                    || a.rotation != b.rotation
                    || !mode_matches(a.mode, b.mode)
                    || (monitor.hdr_supported && a.hdr != b.hdr)
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

pub fn suggestions(
    monitors: &[DisplayMonitor],
) -> Vec<(String, &'static str, Vec<ProfileMonitor>)> {
    let make = |m: &DisplayMonitor| -> Option<DisplayLayout> {
        let mode = m
            .layout
            .as_ref()
            .map(|l| l.mode)
            .or_else(|| m.modes.first().copied())?;
        Some(DisplayLayout {
            x: 0,
            y: 0,
            mode,
            rotation: m
                .layout
                .as_ref()
                .map_or(DisplayRotation::None, |l| l.rotation),
            primary: true,
            mirror_of: None,
            hdr: m.layout.as_ref().is_some_and(|l| l.hdr),
        })
    };
    let mut result = Vec::new();
    if monitors.len() >= 2 {
        let primary = monitors
            .iter()
            .position(|m| m.layout.as_ref().is_some_and(|l| l.primary))
            .unwrap_or(0);
        let mut order: Vec<_> = (0..monitors.len()).collect();
        order.sort_by_key(|i| monitors[*i].layout.as_ref().map_or(i32::MAX, |l| l.x));
        let mut x = 0i32;
        let mut profile: Vec<_> = monitors
            .iter()
            .map(|m| ProfileMonitor {
                identity: m.identity.clone(),
                name: m.name.clone(),
                layout: make(m),
            })
            .collect();
        for i in order {
            if let Some(l) = &mut profile[i].layout {
                l.primary = i == primary;
                l.x = x;
                let width = if matches!(
                    l.rotation,
                    DisplayRotation::Rotate90 | DisplayRotation::Rotate270
                ) {
                    l.mode.height
                } else {
                    l.mode.width
                };
                x = x.saturating_add(i32::try_from(width).unwrap_or(i32::MAX));
            }
        }
        let mut layouts: Vec<_> = profile.iter().map(|p| p.layout.clone()).collect();
        if normalize(&mut layouts).is_ok() {
            for (p, l) in profile.iter_mut().zip(layouts) {
                p.layout = l;
            }
            result.push(("All monitors".into(), "monitors", profile));
        }
    }
    for (i, m) in monitors.iter().enumerate() {
        let profile = monitors
            .iter()
            .enumerate()
            .map(|(j, other)| ProfileMonitor {
                identity: other.identity.clone(),
                name: other.name.clone(),
                layout: if i == j { make(other) } else { None },
            })
            .collect();
        result.push((
            m.name.clone(),
            if m.built_in { "laptop" } else { "monitor" },
            profile,
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(path: &str, serial: Option<&str>) -> DisplayMonitor {
        DisplayMonitor {
            identity: DisplayIdentity {
                edid: "GSM1234".into(),
                serial: serial.map(str::to_string),
                path: path.into(),
            },
            name: path.into(),
            built_in: false,
            layout: Some(DisplayLayout {
                x: 0,
                y: 0,
                mode: DisplayMode {
                    width: 1920,
                    height: 1080,
                    refresh_mhz: 60000,
                },
                rotation: DisplayRotation::None,
                primary: true,
                mirror_of: None,
                hdr: false,
            }),
            modes: vec![DisplayMode {
                width: 1920,
                height: 1080,
                refresh_mhz: 60000,
            }],
            hdr_supported: false,
        }
    }
    fn profile(ms: &[DisplayMonitor]) -> Vec<ProfileMonitor> {
        ms.iter()
            .map(|m| ProfileMonitor {
                identity: m.identity.clone(),
                name: m.name.clone(),
                layout: m.layout.clone(),
            })
            .collect()
    }
    #[test]
    fn displays_identity_resolution_table() {
        for (saved, live, expected) in [
            (
                vec![monitor("a", None), monitor("b", None)],
                vec![monitor("b", None), monitor("a", None)],
                vec![Some(1), Some(0)],
            ),
            (
                vec![monitor("old", Some("1")), monitor("b", Some("2"))],
                vec![monitor("new", Some("1")), monitor("b", Some("2"))],
                vec![Some(0), Some(1)],
            ),
            (
                vec![monitor("old", None)],
                vec![monitor("a", None), monitor("b", None)],
                vec![None],
            ),
            (
                vec![monitor("old", None)],
                vec![monitor("new", None)],
                vec![Some(0)],
            ),
            (vec![monitor("old", None)], vec![], vec![None]),
            (
                vec![monitor("old", None), monitor("b", None)],
                vec![monitor("b", None)],
                vec![None, Some(0)],
            ),
        ] {
            assert_eq!(resolve(&profile(&saved), &live), expected);
        }
    }
    #[test]
    fn displays_matches_normalization_missing_refresh_and_mirrors() {
        let mut live = vec![monitor("a", None), monitor("b", None)];
        live[1].layout.as_mut().expect("active test layout").primary = false;
        live[1]
            .layout
            .as_mut()
            .expect("active test layout")
            .mirror_of = Some("a".into());
        let mut saved = profile(&live);
        for p in &mut saved {
            let l = p.layout.as_mut().expect("active test layout");
            l.x += 200;
            l.y -= 100;
        }
        saved.push(ProfileMonitor {
            identity: monitor("missing", Some("x")).identity,
            name: "Missing".into(),
            layout: None,
        });
        for (refresh_mhz, expected) in [
            (59499, false),
            (59500, true),
            (60000, true),
            (60500, true),
            (60501, false),
        ] {
            saved[0]
                .layout
                .as_mut()
                .expect("active test layout")
                .mode
                .refresh_mhz = refresh_mhz;
            assert_eq!(matches(&saved, &live), expected);
        }
        saved[0]
            .layout
            .as_mut()
            .expect("active test layout")
            .mode
            .refresh_mhz = 60000;
        saved[1]
            .layout
            .as_mut()
            .expect("active test layout")
            .mirror_of = None;
        assert!(!matches(&saved, &live));
        saved[1].layout = None;
        assert!(!matches(&saved, &live));
    }
    #[test]
    fn displays_suggestions_keep_primary_and_extend() {
        let mut live = vec![monitor("a", None), monitor("b", None)];
        live[0].layout.as_mut().expect("active test layout").primary = false;
        live[0].layout.as_mut().expect("active test layout").x = -1920;
        let suggestions = suggestions(&live);
        assert_eq!(suggestions.len(), 3);
        assert_eq!(
            suggestions[0].2[0]
                .layout
                .as_ref()
                .expect("active test layout")
                .x,
            -1920
        );
        assert!(
            suggestions[0].2[1]
                .layout
                .as_ref()
                .expect("active test layout")
                .primary
        );
        assert!(suggestions[1].2[1].layout.is_none());
    }

    #[test]
    fn displays_missing_mirror_source_and_changed_ports() {
        let mut saved_monitors = vec![monitor("a", Some("1")), monitor("b", Some("2"))];
        let child = saved_monitors[1]
            .layout
            .as_mut()
            .expect("active test layout");
        child.primary = false;
        child.mirror_of = Some("a".into());
        let saved = profile(&saved_monitors);
        // Both ports changed, but mirror references still refer to saved paths.
        let mut live = saved_monitors.clone();
        live[0].identity.path = "new-a".into();
        live[1].identity.path = "new-b".into();
        live[1]
            .layout
            .as_mut()
            .expect("active test layout")
            .mirror_of = Some("new-a".into());
        assert!(matches(&saved, &live));
        // Losing the original source must not disable the remaining screen.
        live.remove(0);
        let child = live[0].layout.as_mut().expect("active test layout");
        child.primary = true;
        child.mirror_of = None;
        assert!(matches(&saved, &live));
        let projected = connected(&saved, &resolve(&saved, &live)).expect("connected mirror group");
        assert_eq!(projected.groups, vec![vec![1]]);
        assert!(
            projected.layouts[1]
                .as_ref()
                .expect("surviving mirror")
                .primary
        );
        let mut cyclic = saved.clone();
        cyclic[0]
            .layout
            .as_mut()
            .expect("active test layout")
            .mirror_of = Some("b".into());
        assert!(connected(&cyclic, &resolve(&cyclic, &live)).is_err());
    }

    #[test]
    fn displays_matches_hdr_and_other_connected_monitors() {
        let mut live = vec![monitor("a", None)];
        let mut saved = profile(&live);
        saved[0].layout.as_mut().expect("active test layout").hdr = true;
        assert!(matches(&saved, &live)); // Ignore HDR on unsupported displays.
        live[0].hdr_supported = true;
        assert!(!matches(&saved, &live));
        live[0].layout.as_mut().expect("active test layout").hdr = true;
        assert!(matches(&saved, &live));
        live.push(monitor("b", None));
        assert!(!matches(&saved, &live));
        live[1].layout = None;
        assert!(matches(&saved, &live));
    }
}
