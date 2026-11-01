//! Tool discovery. No process environment mutation: each probe reads the current PATH.
use crate::sounddeck::download_worker::{on_path, probe_version};
use macafon_ipc::{ToolSource, UpdateComponent};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub fn root() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| path.join("Macafon"))
    } else {
        None
    }
}

pub fn managed_directory(component: UpdateComponent) -> Option<PathBuf> {
    let name = match component {
        UpdateComponent::App => return None,
        UpdateComponent::YtDlp => "yt-dlp",
        UpdateComponent::Ffmpeg => "ffmpeg",
    };
    Some(root()?.join("tools").join(name))
}

fn managed(component: UpdateComponent, executable: &str) -> Option<PathBuf> {
    managed_directory(component)
        .map(|path| path.join(executable))
        .filter(|path| path.is_file())
}

fn choose(
    configured: Option<&Path>,
    managed: Option<PathBuf>,
    path: impl FnOnce() -> Option<PathBuf>,
) -> Option<(PathBuf, ToolSource)> {
    // An explicitly configured path remains authoritative even if the executable is broken.
    configured
        .map(|p| (p.to_owned(), ToolSource::Custom))
        .or_else(|| managed.map(|p| (p, ToolSource::Managed)))
        .or_else(|| path().map(|p| (p, ToolSource::Path)))
}

pub fn resolve_downloader(configured: Option<&Path>) -> Option<(PathBuf, ToolSource)> {
    choose(
        configured,
        managed(UpdateComponent::YtDlp, "yt-dlp.exe"),
        || on_path("yt-dlp"),
    )
}

pub fn resolve_ffmpeg() -> Option<(PathBuf, ToolSource)> {
    choose(None, managed(UpdateComponent::Ffmpeg, "ffmpeg.exe"), || {
        on_path("ffmpeg")
    })
}

pub fn managed_ffmpeg_directory() -> Option<PathBuf> {
    managed(UpdateComponent::Ffmpeg, "ffmpeg.exe")?
        .parent()
        .map(Path::to_owned)
}

pub fn scan(state: &mut macafon_ipc::UpdatesState, configured: Option<&Path>, cancel: &AtomicBool) {
    let records: std::collections::BTreeMap<String, String> = root()
        .and_then(|root| std::fs::read(root.join("tools/tools.json")).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    for tool in &mut state.tools {
        let (resolved, args): (_, &[&str]) = match tool.component {
            UpdateComponent::YtDlp => (
                resolve_downloader(configured),
                &["--ignore-config", "--no-plugin-dirs", "--version"],
            ),
            _ => (resolve_ffmpeg(), &["-version"]),
        };
        tool.source = resolved
            .as_ref()
            .map_or(ToolSource::Missing, |(_, source)| *source);
        tool.path = resolved
            .as_ref()
            .map(|(path, _)| path.display().to_string());
        let key = if tool.component == UpdateComponent::YtDlp {
            "yt-dlp"
        } else {
            "ffmpeg"
        };
        tool.installed_version = if tool.source == ToolSource::Managed {
            records.get(key).cloned()
        } else {
            None
        };
        if tool.installed_version.is_none() {
            tool.installed_version =
                resolved.and_then(|(path, _)| probe_version(&path, args, cancel));
        }
        super::availability(tool);
    }
}

#[cfg(not(windows))]
pub fn search_path() -> Option<std::ffi::OsString> {
    std::env::var_os("PATH")
}

#[cfg(windows)]
pub fn search_path() -> Option<std::ffi::OsString> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ,
        RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows::core::{HSTRING, w};
    fn read(key: HKEY, subkey: &str) -> Option<String> {
        let subkey = HSTRING::from(subkey);
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let mut bytes = 0;
        // SAFETY: size-only query with valid constant value name and subkey.
        if unsafe {
            RegGetValueW(
                key,
                &subkey,
                w!("Path"),
                flags,
                None,
                None,
                Some(&mut bytes),
            )
        } != ERROR_SUCCESS
            || bytes > 131072
        {
            return None;
        }
        let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
        // SAFETY: the allocated UTF-16 buffer holds the requested byte count.
        if unsafe {
            RegGetValueW(
                key,
                &subkey,
                w!("Path"),
                flags,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        } != ERROR_SUCCESS
        {
            return None;
        }
        buffer.truncate((bytes as usize / 2).saturating_sub(1));
        Some(String::from_utf16_lossy(&buffer))
    }
    let machine = read(
        HKEY_LOCAL_MACHINE,
        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
    );
    let user = read(HKEY_CURRENT_USER, "Environment");
    if machine.is_none() && user.is_none() {
        return std::env::var_os("PATH");
    }
    let machine = expand_variables(machine.as_deref().unwrap_or(""), "");
    let user = expand_variables(user.as_deref().unwrap_or(""), &machine);
    // Keep process-only additions too, after the freshly read system/user paths.
    Some(
        format!(
            "{machine};{user};{}",
            std::env::var("PATH").unwrap_or_default()
        )
        .into(),
    )
}

#[cfg(any(windows, test))]
fn expand_variables(value: &str, machine_path: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            output.push_str(&rest[start..]);
            return output;
        };
        let variable = &after[..end];
        if variable.eq_ignore_ascii_case("PATH") {
            output.push_str(machine_path);
        } else if let Some((_, value)) =
            std::env::vars().find(|(key, _)| key.eq_ignore_ascii_case(variable))
        {
            output.push_str(&value);
        } else {
            output.push_str(&rest[start..start + end + 2]);
        }
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolution_prefers_custom_then_managed_then_path() {
        let custom = Path::new("custom");
        assert_eq!(
            choose(Some(custom), Some("managed".into()), || panic!(
                "PATH is unnecessary"
            )),
            Some((custom.into(), ToolSource::Custom))
        );
        assert_eq!(
            choose(None, Some("managed".into()), || panic!(
                "PATH is unnecessary"
            )),
            Some(("managed".into(), ToolSource::Managed))
        );
        assert_eq!(
            choose(None, None, || Some("path".into())),
            Some(("path".into(), ToolSource::Path))
        );
        assert_eq!(choose(None, None, || None), None);
        assert_eq!(
            expand_variables("%PATH%;%UNKNOWN_MACAFON_TEST%", "machine"),
            "machine;%UNKNOWN_MACAFON_TEST%"
        );
    }
}
