//! "Start with system" (AS-001, AS-002): the packaged `systemd --user` unit when it is
//! installed and the desktop session reaches `graphical-session.target`, otherwise an XDG
//! autostart entry. `MACAFON_NO_SYSTEMD=1` forces the XDG entry.

use macafon_ipc::{AutostartMethod, AutostartState};
use std::path::{Path, PathBuf};
use std::process::Command;

const UNIT: &str = "macafond.service";
const DESKTOP_FILE: &str = "macafond.desktop";

fn systemctl(args: &[&str]) -> Option<std::process::Output> {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .ok()
}

fn succeeds(args: &[&str]) -> bool {
    systemctl(args).is_some_and(|output| output.status.success())
}

fn systemd_usable() -> bool {
    std::env::var_os("MACAFON_NO_SYSTEMD").is_none_or(|value| value.is_empty() || value == "0")
        && succeeds(&["cat", UNIT])
        && succeeds(&["is-active", "--quiet", "graphical-session.target"])
}

fn systemd_enabled() -> bool {
    systemctl(&["is-enabled", UNIT])
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "enabled")
}

fn desktop_path() -> Result<PathBuf, String> {
    macafon_config::config_dir()
        .map(|macafon| macafon.with_file_name("autostart").join(DESKTOP_FILE))
        .map_err(|error| error.to_string())
}

/// Quote an argument for a desktop entry `Exec` key (Desktop Entry Specification).
fn exec_argument(argument: &str) -> String {
    if argument
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
    {
        return argument.to_owned();
    }
    let mut quoted = String::from("\"");
    for c in argument.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    // `%` is a field code everywhere in Exec, even inside quotes.
    quoted.replace('%', "%%")
}

fn desktop_entry(executable: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Macafon\n\
         Comment=Macafon audio daemon: virtual microphone and tray icon\n\
         Exec={} run\n\
         Icon=audio-input-microphone\n\
         Terminal=false\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_argument(&executable.to_string_lossy())
    )
}

pub fn state() -> AutostartState {
    let method = if systemd_usable() {
        AutostartMethod::Systemd
    } else {
        AutostartMethod::XdgAutostart
    };
    let desktop = desktop_path().is_ok_and(|path| path.is_file());
    let enabled = desktop || (method == AutostartMethod::Systemd && systemd_enabled());
    AutostartState { enabled, method }
}

pub fn set(enabled: bool) -> Result<AutostartState, String> {
    let systemd = systemd_usable();
    let desktop = desktop_path()?;
    if enabled && systemd {
        run(&["enable", UNIT])?;
    } else if enabled {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        if let Some(directory) = desktop.parent() {
            std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        }
        std::fs::write(&desktop, desktop_entry(&executable))
            .map_err(|error| format!("{}: {error}", desktop.display()))?;
    }
    // Only one mechanism stays active.
    if (!enabled || !systemd) && systemd_enabled() {
        run(&["disable", UNIT])?;
    }
    if (!enabled || systemd) && desktop.exists() {
        std::fs::remove_file(&desktop)
            .map_err(|error| format!("{}: {error}", desktop.display()))?;
    }
    Ok(state())
}

fn run(args: &[&str]) -> Result<(), String> {
    let output = systemctl(args).ok_or("systemctl is not available")?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "systemctl --user {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_arguments_follow_the_desktop_entry_quoting_rules() {
        assert_eq!(exec_argument("/usr/bin/macafond"), "/usr/bin/macafond");
        assert_eq!(
            exec_argument("/home/a b/$x\"100%"),
            "\"/home/a b/\\$x\\\"100%%\""
        );
        assert!(
            desktop_entry(Path::new("/opt/mic eq/macafond"))
                .contains("Exec=\"/opt/mic eq/macafond\" run\n")
        );
    }
}
