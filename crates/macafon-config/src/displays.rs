//! `[displays]`: saved monitor layouts (PRD §8.5).

use macafon_ipc::DisplayProfile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaysConfig {
    /// In the user's order.
    pub profiles: Vec<DisplayProfile>,
    /// DISP-004: profile id → the monitor sets (`monitor_set_key`) it was kept on; applying it
    /// on one of them again needs no confirmation.
    pub confirmed: BTreeMap<String, Vec<String>>,
}

impl DisplaysConfig {
    pub fn profile(&self, id: &str) -> Option<&DisplayProfile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    /// A new id from the name: `work`, `work-2`, …
    pub fn next_id(&self, name: &str) -> String {
        let mut base: String = name
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if base.is_empty() {
            base = "profile".into();
        }
        let mut id = base.clone();
        let mut n = 2;
        while self.profile(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }

    pub fn is_confirmed(&self, id: &str, set_key: &str) -> bool {
        self.confirmed
            .get(id)
            .is_some_and(|sets| sets.iter().any(|set| set == set_key))
    }
}

/// The set of connected monitors, order-independent: sorted EDID ids (with serials) joined.
pub fn monitor_set_key<'a>(
    monitors: impl IntoIterator<Item = &'a macafon_ipc::DisplayIdentity>,
) -> String {
    let mut ids: Vec<String> = monitors
        .into_iter()
        .map(|identity| match &identity.serial {
            Some(serial) => format!("{}:{serial}", identity.edid),
            None => identity.edid.clone(),
        })
        .collect();
    ids.sort();
    ids.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use macafon_ipc::DisplayIdentity;

    fn identity(edid: &str, serial: Option<&str>, path: &str) -> DisplayIdentity {
        DisplayIdentity {
            edid: edid.into(),
            serial: serial.map(Into::into),
            path: path.into(),
        }
    }

    #[test]
    fn ids_come_from_names_and_stay_unique() {
        let mut config = DisplaysConfig::default();
        assert_eq!(config.next_id("  Work: 3 screens "), "work-3-screens");
        assert_eq!(config.next_id("!!!"), "profile");
        config.profiles.push(DisplayProfile {
            id: "work".into(),
            name: "Work".into(),
            icon: "monitor".into(),
            monitors: Vec::new(),
            output_device: None,
        });
        assert_eq!(config.next_id("Work"), "work-2");
    }

    #[test]
    fn monitor_sets_ignore_order_and_ports() {
        let a = identity("GSM5B7F", Some("123"), r"\\?\DISPLAY#GSM5B7F#1");
        let b = identity("DEL4123", None, r"\\?\DISPLAY#DEL4123#2");
        let b_other_port = identity("DEL4123", None, r"\\?\DISPLAY#DEL4123#7");
        assert_eq!(
            monitor_set_key([&a, &b]),
            monitor_set_key([&b_other_port, &a])
        );
        assert_ne!(monitor_set_key([&a]), monitor_set_key([&a, &b]));
    }
}
