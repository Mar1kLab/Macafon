//! Applications behind playback streams (PRD §7.4): the grouping key (APP-002), a readable
//! name and icon (APP-003), and the built-in classification of known apps (APP-006).

pub mod icons;

/// Where a newly seen application goes before the user moves it (APP-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Voice,
    Music,
    /// Everything else, and system sounds (APP-011): the default channel.
    Other,
}

/// Key of the pseudo-application that collects event and notification sounds (APP-011).
pub const SYSTEM_SOUNDS_KEY: &str = "system:sounds";

/// Stream properties used for identification; borrowed from the PipeWire node info.
#[derive(Debug, Default, Clone, Copy)]
pub struct StreamProps<'a> {
    pub binary: Option<&'a str>,
    pub flatpak_id: Option<&'a str>,
    pub app_id: Option<&'a str>,
    pub name: Option<&'a str>,
    pub icon_name: Option<&'a str>,
    pub media_role: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    /// Stable across restarts: `bin:discord`, `app:com.spotify.Client`, `name:Firefox`.
    pub key: String,
    pub name: String,
    /// Freedesktop icon name, if any is known.
    pub icon: Option<String>,
}

/// Interpreters and runtimes that host many different applications: their binary says
/// nothing about the app, so the next key source is used instead.
const GENERIC_BINARIES: &[&str] = &[
    "electron",
    "python",
    "python3",
    "java",
    "node",
    "wine",
    "wine64",
    "wine-preloader",
    "wine64-preloader",
    "mono",
    "bwrap",
    "sh",
    "bash",
];

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn generic(binary: &str) -> bool {
    let base = binary.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    GENERIC_BINARIES.contains(&base.to_ascii_lowercase().as_str())
}

/// APP-002: process binary → Flatpak/application id → application name.
pub fn identify(props: &StreamProps) -> Option<AppIdentity> {
    if matches!(
        non_empty(props.media_role),
        Some("event" | "notification" | "Notification")
    ) {
        return Some(AppIdentity {
            key: SYSTEM_SOUNDS_KEY.to_owned(),
            name: "System sounds".to_owned(),
            icon: Some("preferences-system-notifications".to_owned()),
        });
    }
    let name = non_empty(props.name);
    let binary = non_empty(props.binary)
        .map(|binary| binary.rsplit('/').next().unwrap_or(binary))
        .filter(|binary| !generic(binary));
    let id = non_empty(props.flatpak_id).or(non_empty(props.app_id));
    let key = if let Some(binary) = binary {
        format!("bin:{}", binary.to_ascii_lowercase())
    } else if let Some(id) = id {
        format!("app:{id}")
    } else {
        format!("name:{}", name?)
    };
    let display = name
        .map(str::to_owned)
        .or_else(|| id.map(|id| id.rsplit('.').next().unwrap_or(id).to_owned()))
        .or_else(|| binary.map(title_case))
        .unwrap_or_else(|| key.clone());
    let icon = non_empty(props.icon_name)
        .or(id)
        .or(binary)
        .map(str::to_owned);
    Some(AppIdentity {
        key,
        name: display,
        icon,
    })
}

fn title_case(binary: &str) -> String {
    let mut chars = binary.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Lower-case fragments matched against the key and name; the first match wins.
const KNOWN: &[(&str, Category)] = &[
    ("discord", Category::Voice),
    ("vesktop", Category::Voice),
    ("vencord", Category::Voice),
    ("telegram", Category::Voice),
    ("teamspeak", Category::Voice),
    ("ts3client", Category::Voice),
    ("mumble", Category::Voice),
    ("zoom", Category::Voice),
    ("slack", Category::Voice),
    ("teams", Category::Voice),
    ("element", Category::Voice),
    ("im.riot", Category::Voice),
    ("spotify", Category::Music),
    ("cider", Category::Music),
    ("yandex-music", Category::Music),
    ("yandexmusic", Category::Music),
    ("yandex music", Category::Music),
    ("tidal", Category::Music),
    ("rhythmbox", Category::Music),
    ("strawberry", Category::Music),
    ("amberol", Category::Music),
    ("deezer", Category::Music),
];

/// APP-006: the channel category of a known app on first sight; `Other` otherwise.
pub fn classify(app: &AppIdentity) -> Category {
    if app.key == SYSTEM_SOUNDS_KEY {
        return Category::Other;
    }
    let key = app.key.to_ascii_lowercase();
    let name = app.name.to_ascii_lowercase();
    KNOWN
        .iter()
        .find(|(fragment, _)| key.contains(fragment) || name.contains(fragment))
        .map_or(Category::Other, |(_, category)| *category)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_app_002_priority_and_known_apps_are_classified() {
        let discord = identify(&StreamProps {
            binary: Some("Discord"),
            name: Some("WEBRTC VoiceEngine"),
            ..Default::default()
        })
        .expect("identified");
        assert_eq!(discord.key, "bin:discord");
        assert_eq!(classify(&discord), Category::Voice);
        // Electron hides the app behind its runtime; the Flatpak id identifies it.
        let spotify = identify(&StreamProps {
            binary: Some("/usr/lib/electron25"),
            flatpak_id: Some("com.spotify.Client"),
            name: Some("Spotify"),
            ..Default::default()
        })
        .expect("identified");
        assert_eq!(spotify.key, "app:com.spotify.Client");
        assert_eq!(classify(&spotify), Category::Music);
        let firefox = identify(&StreamProps {
            binary: Some("firefox"),
            name: Some("Firefox"),
            icon_name: Some("firefox"),
            ..Default::default()
        })
        .expect("identified");
        assert_eq!(
            (firefox.key.as_str(), firefox.name.as_str()),
            ("bin:firefox", "Firefox")
        );
        assert_eq!(classify(&firefox), Category::Other);
        let game = identify(&StreamProps {
            name: Some("Cyberpunk 2077"),
            binary: Some("wine64-preloader"),
            ..Default::default()
        })
        .expect("identified");
        assert_eq!(game.key, "name:Cyberpunk 2077");
        let bell = identify(&StreamProps {
            binary: Some("gnome-shell"),
            media_role: Some("event"),
            ..Default::default()
        })
        .expect("identified");
        assert_eq!(bell.key, SYSTEM_SOUNDS_KEY);
        assert_eq!(classify(&bell), Category::Other);
        assert_eq!(identify(&StreamProps::default()), None);
    }
}
