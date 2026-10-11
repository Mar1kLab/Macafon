//! App icons (APP-003): a freedesktop icon name or app id becomes an image file, found through
//! `.desktop` files (`Icon=`) and the icon themes, and is handed to the UI as a `data:` URL —
//! the WebView cannot read files outside its bundle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Larger files are not icons worth sending over IPC.
const MAX_ICON_BYTES: u64 = 512 * 1024;
/// A missing icon is looked up again after this long: the app may have been installed since.
const MISS_TTL: Duration = Duration::from_secs(60);

const SIZES: [&str; 8] = [
    "scalable", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "512x512",
];
/// Themes like Breeze put the size below the context: `apps/48/firefox.svg`.
const SIZES_INVERTED: [&str; 5] = ["scalable", "64", "48", "32", "22"];
const CONTEXTS: [&str; 3] = ["apps", "status", "devices"];

pub struct IconResolver {
    data_dirs: Vec<PathBuf>,
    /// Lower-case desktop id, binary or WM class → `Icon=` value. Built on first use.
    desktop: Option<HashMap<String, String>>,
    cache: HashMap<String, (Option<String>, Instant)>,
}

impl IconResolver {
    /// `$XDG_DATA_HOME`, `$XDG_DATA_DIRS` and the Flatpak exports.
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut dirs = Vec::new();
        match std::env::var_os("XDG_DATA_HOME") {
            Some(dir) if !dir.is_empty() => dirs.push(PathBuf::from(dir)),
            _ => dirs.extend(home.as_ref().map(|home| home.join(".local/share"))),
        }
        let system = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
        dirs.extend(system.split(':').map(PathBuf::from));
        dirs.extend(
            home.as_ref()
                .map(|home| home.join(".local/share/flatpak/exports/share")),
        );
        dirs.push(PathBuf::from("/var/lib/flatpak/exports/share"));
        let mut unique = Vec::new();
        for dir in dirs {
            if !unique.contains(&dir) {
                unique.push(dir);
            }
        }
        Self::new(unique)
    }

    pub fn new(data_dirs: Vec<PathBuf>) -> Self {
        Self {
            data_dirs,
            desktop: None,
            cache: HashMap::new(),
        }
    }

    /// `data:image/…;base64,…` for an icon name, app id or binary; cached.
    pub fn data_url(&mut self, name: &str) -> Option<String> {
        if let Some((url, at)) = self.cache.get(name)
            && (url.is_some() || at.elapsed() < MISS_TTL)
        {
            return url.clone();
        }
        let url = self.find(name).and_then(|path| encode(&path));
        self.cache
            .insert(name.to_owned(), (url.clone(), Instant::now()));
        url
    }

    /// The image file for `name`, if any.
    pub fn find(&mut self, name: &str) -> Option<PathBuf> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let direct = Path::new(name);
        if direct.is_absolute() {
            return direct.is_file().then(|| direct.to_owned());
        }
        let icon = self
            .desktop_index()
            .get(&name.to_ascii_lowercase())
            .cloned();
        if let Some(icon) = &icon {
            let path = Path::new(icon);
            if path.is_absolute() {
                return path.is_file().then(|| path.to_owned());
            }
            if let Some(found) = self.themed(icon) {
                return Some(found);
            }
        }
        self.themed(name)
            .or_else(|| self.themed(&name.to_ascii_lowercase()))
    }

    fn desktop_index(&mut self) -> &HashMap<String, String> {
        let dirs = &self.data_dirs;
        self.desktop.get_or_insert_with(|| {
            let mut index = HashMap::new();
            // Earlier dirs win: the user's own entries shadow the system ones.
            for dir in dirs.iter().rev() {
                for file in desktop_files(&dir.join("applications")) {
                    let Ok(text) = std::fs::read_to_string(&file) else {
                        continue;
                    };
                    let Some(entry) = parse_desktop(&text) else {
                        continue;
                    };
                    let stem = file
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or_default();
                    for key in [Some(stem), entry.exec.as_deref(), entry.wm_class.as_deref()]
                        .into_iter()
                        .flatten()
                        .filter(|key| !key.is_empty())
                    {
                        index.insert(key.to_ascii_lowercase(), entry.icon.clone());
                    }
                }
            }
            index
        })
    }

    /// hicolor first (where apps install their icons), then every other theme, then pixmaps.
    fn themed(&self, icon: &str) -> Option<PathBuf> {
        if icon.contains('/') {
            return None;
        }
        let file = |dir: &Path| {
            ["svg", "png"]
                .iter()
                .map(|ext| dir.join(format!("{icon}.{ext}")))
                .find(|path| path.is_file())
        };
        for data in &self.data_dirs {
            let hicolor = data.join("icons/hicolor");
            for size in SIZES {
                if let Some(found) = file(&hicolor.join(size).join("apps")) {
                    return Some(found);
                }
            }
        }
        for data in &self.data_dirs {
            let Ok(themes) = std::fs::read_dir(data.join("icons")) else {
                continue;
            };
            let mut themes: Vec<PathBuf> = themes
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| !path.ends_with("hicolor"))
                .collect();
            themes.sort();
            for theme in themes {
                for context in CONTEXTS {
                    for size in SIZES {
                        if let Some(found) = file(&theme.join(size).join(context)) {
                            return Some(found);
                        }
                    }
                    for size in SIZES_INVERTED {
                        if let Some(found) = file(&theme.join(context).join(size)) {
                            return Some(found);
                        }
                    }
                }
            }
        }
        self.data_dirs
            .iter()
            .find_map(|data| file(&data.join("pixmaps")))
    }
}

struct DesktopEntry {
    icon: String,
    /// Basename of the executable in `Exec=`.
    exec: Option<String>,
    wm_class: Option<String>,
}

fn desktop_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // One level of vendor subdirectories (`applications/kde4/`).
            if let Ok(inner) = std::fs::read_dir(&path) {
                files.extend(
                    inner
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|path| is_desktop(path)),
                );
            }
        } else if is_desktop(&path) {
            files.push(path);
        }
    }
    files
}

fn is_desktop(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "desktop")
}

fn parse_desktop(text: &str) -> Option<DesktopEntry> {
    let mut in_entry = false;
    let (mut icon, mut exec, mut wm_class) = (None, None, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Icon" => icon = Some(value.trim().to_owned()),
            "Exec" => exec = exec_binary(value),
            "StartupWMClass" => wm_class = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    Some(DesktopEntry {
        icon: icon.filter(|icon| !icon.is_empty())?,
        exec,
        wm_class,
    })
}

/// `env FOO=1 /opt/discord/Discord %U` → `Discord`.
fn exec_binary(exec: &str) -> Option<String> {
    exec.split_whitespace()
        .map(|word| word.trim_matches('"'))
        .find(|word| *word != "env" && !word.contains('='))
        .map(|word| word.rsplit('/').next().unwrap_or(word).to_owned())
}

fn encode(path: &Path) -> Option<String> {
    let mime = match path.extension()?.to_str()? {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        _ => return None,
    };
    if std::fs::metadata(path).ok()?.len() > MAX_ICON_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!("data:{mime};base64,{}", base64(&bytes)))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, byte)| n | u32::from(*byte) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_files_and_themes_resolve_binaries_to_icons() {
        let root = std::env::temp_dir().join(format!("macafon-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("applications");
        let hicolor = root.join("icons/hicolor/48x48/apps");
        let breeze = root.join("icons/breeze/apps/48");
        for dir in [&apps, &hicolor, &breeze] {
            std::fs::create_dir_all(dir).expect("temp dir");
        }
        std::fs::write(
            apps.join("com.discordapp.Discord.desktop"),
            "[Desktop Entry]\nName=Discord\nExec=env BAMF=1 /opt/discord/Discord %U\nIcon=discord-app\n\
             [Desktop Action New]\nIcon=wrong\n",
        )
        .expect("desktop file");
        std::fs::write(hicolor.join("discord-app.png"), [0x89, b'P', b'N', b'G']).expect("png");
        std::fs::write(
            breeze.join("preferences-system-notifications.svg"),
            "<svg/>",
        )
        .expect("svg");

        let mut resolver = IconResolver::new(vec![root.clone()]);
        // The stream's binary finds the desktop entry by its Exec line.
        assert_eq!(
            resolver.data_url("Discord").as_deref(),
            Some("data:image/png;base64,iVBORw==")
        );
        assert_eq!(
            resolver.find("com.discordapp.Discord"),
            Some(hicolor.join("discord-app.png"))
        );
        assert_eq!(
            resolver.find("preferences-system-notifications"),
            Some(breeze.join("preferences-system-notifications.svg"))
        );
        assert_eq!(resolver.data_url("nothing-here"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
