//! Updates of Macafon itself and of its optional SoundDeck tools (PRD §8.6).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum UpdateComponent {
    App,
    YtDlp,
    Ffmpeg,
}

/// Where the copy in use comes from (UPD-006). Always `bundled` for the app itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    #[default]
    Missing,
    /// Installed by Macafon into its tools folder; the only kind Macafon updates or removes.
    Managed,
    /// `sounddeck.downloader_path` (yt-dlp only).
    Custom,
    Path,
    /// The running Macafon.
    Bundled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum UpdateStage {
    #[default]
    Idle,
    Checking,
    Downloading,
    Verifying,
    /// Unpacking (ffmpeg) or replacing the files.
    Installing,
    /// The app installer is downloaded and verified; `updates.install` with `app` runs it.
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ComponentUpdate {
    pub component: UpdateComponent,
    /// Macafon can download and install this component here (Windows); otherwise the UI only
    /// shows versions and, for the app, opens the release page (UPD-004, UPD-005).
    pub installable: bool,
    pub source: ToolSource,
    /// Path of the copy in use, if any.
    pub path: Option<String>,
    /// The running app version, `yt-dlp --version`, or the tag recorded for a managed ffmpeg.
    pub installed_version: Option<String>,
    /// From the last successful check; `None` before any check.
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub release_url: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub download_bytes: Option<u64>,
    pub stage: UpdateStage,
    pub progress_pct: Option<f32>,
    /// Short reason of the last failure; cleared by the next action.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdatesState {
    /// Opt-in automatic check, at most once per 24 h (UPD-002); off by default.
    pub check_automatically: bool,
    /// Unix seconds of the last finished check.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_checked_unix: Option<u64>,
    pub app: ComponentUpdate,
    /// yt-dlp, then ffmpeg.
    pub tools: Vec<ComponentUpdate>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, RequestEnvelope};

    #[test]
    fn update_requests_and_event_use_the_public_wire_names() {
        for (method, params) in [
            ("updates.get", serde_json::json!(null)),
            ("updates.check", serde_json::json!(null)),
            ("updates.install", serde_json::json!({"component":"yt_dlp"})),
            ("updates.cancel", serde_json::json!({"component":"app"})),
            ("updates.remove", serde_json::json!({"component":"ffmpeg"})),
            (
                "updates.set_check_automatically",
                serde_json::json!({"enabled":true}),
            ),
            (
                "updates.open_release",
                serde_json::json!({"component":"app"}),
            ),
        ] {
            let request = serde_json::json!({"id":1,"method":method,"params":params});
            let parsed: RequestEnvelope = serde_json::from_value(request).expect("update request");
            let encoded = serde_json::to_value(&parsed).expect("encode request");
            assert_eq!(encoded["method"], method);
        }
        let component = ComponentUpdate {
            component: UpdateComponent::YtDlp,
            installable: true,
            source: ToolSource::Managed,
            path: None,
            installed_version: Some("2026.10.01".into()),
            latest_version: Some("2026.10.20".into()),
            update_available: true,
            release_url: None,
            download_bytes: Some(18_000_000),
            stage: UpdateStage::Downloading,
            progress_pct: Some(12.5),
            error: None,
        };
        let state = UpdatesState {
            check_automatically: false,
            last_checked_unix: None,
            app: ComponentUpdate {
                component: UpdateComponent::App,
                source: ToolSource::Bundled,
                ..component.clone()
            },
            tools: vec![component],
        };
        let event = Event::UpdatesChanged(state);
        let encoded = serde_json::to_value(&event).expect("updates event");
        assert_eq!(encoded["event"], "updates.changed");
        assert_eq!(encoded["data"]["tools"][0]["component"], "yt_dlp");
        assert_eq!(encoded["data"]["tools"][0]["stage"], "downloading");
        assert_eq!(
            serde_json::from_value::<Event>(encoded).expect("event round trip"),
            event
        );
    }
}
