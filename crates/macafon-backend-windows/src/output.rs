//! Synchronous render-default policy. Call off the audio thread, with one journal owner.
//! Explicit per-application endpoint selections are never changed.
use crate::{
    DataFlow, Endpoint, EndpointRole, Error,
    journal::{self, Defaults},
};
use std::path::Path;

/// First available non-cable render endpoint in the user's order, only if different from
/// the current default. An empty list leaves Windows in control without any writes.
/// All base VB-CABLE endpoints are excluded, including the extra 16-channel endpoint.
pub fn priority_choice<'a>(
    priority: &[String],
    active: &'a [Endpoint],
    current: Option<&str>,
) -> Option<&'a str> {
    preferred(priority, active).filter(|id| Some(*id) != current)
}

fn preferred<'a>(priority: &[String], active: &'a [Endpoint]) -> Option<&'a str> {
    priority.iter().find_map(|id| {
        active
            .iter()
            .find(|e| e.id == *id && e.flow == DataFlow::Render && !e.is_vb_cable())
            .map(|e| e.id.as_str())
    })
}

/// Result of a device scan; manual changes include all three current roles.
#[derive(Debug, PartialEq, Eq)]
pub enum OutputChange {
    Unchanged,
    Applied(String),
    ManualChange([Option<String>; 3]),
}

/// Tracks the preferred endpoint and observed defaults across scans.
/// A change of any role while the previous preferred device remains present is considered
/// manual. Automation pauses until the winning priority endpoint changes (or reset is
/// called after an explicit configuration change). Unplugging the preferred device permits
/// fallback even if Windows has already selected a different default. Changes between scans
/// that leave identical IDs cannot be distinguished from no change.
#[derive(Debug, Default)]
pub struct OutputPolicy {
    winner: Option<String>,
    observed: Option<[Option<String>; 3]>,
    paused: bool,
}

impl OutputPolicy {
    /// Enter Follow system default mode: conditionally restore pre-Macafon defaults,
    /// then forget priority observations. Continue scanning with an empty priority list.
    /// A later manual Windows choice wins over the journal, as it does on Quit.
    pub fn follow_system_default(
        &mut self,
        defaults: &mut impl Defaults,
        path: &Path,
    ) -> Result<(), Error> {
        journal::restore(defaults, path)?;
        self.reset();
        Ok(())
    }
    /// Forget observations after an explicit output/priority configuration change.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Apply a selection to all roles through the restore journal, only when needed.
    /// Call journal::restore on Quit and before creating this policy on the next start.
    pub fn scan(
        &mut self,
        priority: &[String],
        active: &[Endpoint],
        defaults: &mut impl Defaults,
        path: &Path,
    ) -> Result<OutputChange, Error> {
        let mut current = [None, None, None];
        for (i, role) in EndpointRole::ALL.into_iter().enumerate() {
            current[i] = defaults.get(DataFlow::Render, role)?;
        }
        let winner = preferred(priority, active).map(str::to_owned);
        let old_present = self.winner.as_ref().is_some_and(|id| {
            active
                .iter()
                .any(|e| e.id == *id && e.flow == DataFlow::Render && !e.is_vb_cable())
        });
        if old_present && self.observed.as_ref().is_some_and(|old| old != &current) {
            self.winner = winner;
            self.observed = Some(current.clone());
            self.paused = true;
            return Ok(OutputChange::ManualChange(current));
        }
        if self.winner != winner {
            self.paused = false;
        }
        self.winner = winner.clone();
        self.observed = Some(current.clone());
        let Some(id) = winner.filter(|_| !self.paused) else {
            return Ok(OutputChange::Unchanged);
        };
        if current.iter().all(|v| v.as_deref() == Some(&id)) {
            return Ok(OutputChange::Unchanged);
        }
        // On partial failure retry on the next scan, rather than interpreting our own write
        // as a manual override. The journal remains available for recovery.
        if let Err(error) = journal::set_default(defaults, path, DataFlow::Render, &id) {
            self.observed = None;
            return Err(error);
        }
        self.observed = Some(std::array::from_fn(|_| Some(id.clone())));
        Ok(OutputChange::Applied(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(crate) fn endpoint(id: &str, flow: DataFlow, cable: bool) -> Endpoint {
        Endpoint {
            id: id.into(),
            name: id.into(),
            flow,
            adapter_instance_id: None,
            hardware_ids: if cable {
                vec![crate::devices::VB_CABLE_HARDWARE_ID.into()]
            } else {
                vec![]
            },
            service: None,
            form_factor: None,
        }
    }
    struct Fake {
        roles: [Option<String>; 3],
        writes: usize,
    }
    impl Defaults for Fake {
        fn get(&self, _: DataFlow, role: EndpointRole) -> Result<Option<String>, Error> {
            Ok(self.roles[EndpointRole::ALL
                .iter()
                .position(|r| *r == role)
                .expect("test operation succeeds")]
            .clone())
        }
        fn set(&mut self, id: &str, role: EndpointRole) -> Result<(), Error> {
            self.roles[EndpointRole::ALL
                .iter()
                .position(|r| *r == role)
                .expect("test operation succeeds")] = Some(id.into());
            self.writes += 1;
            Ok(())
        }
    }
    #[test]
    fn priority_excludes_cable_capture_and_missing_devices() {
        let active = vec![
            endpoint("cable", DataFlow::Render, true),
            endpoint("mic", DataFlow::Capture, false),
            endpoint("speakers", DataFlow::Render, false),
        ];
        let order = ["missing", "cable", "mic", "speakers"].map(str::to_owned);
        assert_eq!(
            priority_choice(&order, &active, Some("other")),
            Some("speakers")
        );
        assert_eq!(priority_choice(&order, &active, Some("speakers")), None);
        assert_eq!(priority_choice(&order[..3], &active, Some("other")), None);
        assert_eq!(priority_choice(&[], &active, Some("other")), None);
    }
    #[test]
    fn hotplug_and_manual_override_are_journaled_without_fighting_the_user() {
        let path = std::env::temp_dir().join(format!("macafon-output-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut fake = Fake {
            roles: [
                Some("original".into()),
                Some("original".into()),
                Some("headset".into()),
            ],
            writes: 0,
        };
        let active = [
            endpoint("dac", DataFlow::Render, false),
            endpoint("speakers", DataFlow::Render, false),
        ];
        let order = ["dac".into(), "speakers".into()];
        let mut policy = OutputPolicy::default();
        assert_eq!(
            policy
                .scan(&order, &active, &mut fake, &path)
                .expect("test operation succeeds"),
            OutputChange::Applied("dac".into())
        );
        policy
            .scan(&order, &active, &mut fake, &path)
            .expect("test operation succeeds");
        assert_eq!(fake.writes, 3);
        fake.roles[2] = Some("manual".into());
        assert!(matches!(
            policy
                .scan(&order, &active, &mut fake, &path)
                .expect("test operation succeeds"),
            OutputChange::ManualChange(_)
        ));
        policy
            .scan(&order, &active, &mut fake, &path)
            .expect("test operation succeeds");
        assert_eq!(fake.writes, 3);
        assert_eq!(
            policy
                .scan(&order, &active[1..], &mut fake, &path)
                .expect("test operation succeeds"),
            OutputChange::Applied("speakers".into())
        );
        assert_eq!(
            policy
                .scan(&order, &active, &mut fake, &path)
                .expect("test operation succeeds"),
            OutputChange::Applied("dac".into())
        );
        fake.roles[2] = Some("user-final".into());
        policy
            .follow_system_default(&mut fake, &path)
            .expect("follow original default");
        let writes = fake.writes;
        policy
            .scan(&[], &active, &mut fake, &path)
            .expect("follow scan");
        assert_eq!(fake.writes, writes);
        assert_eq!(
            fake.roles,
            [
                Some("original".into()),
                Some("original".into()),
                Some("user-final".into())
            ]
        );
    }
}
