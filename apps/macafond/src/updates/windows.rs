//! Per-user installation. The verified archive is streamed into fixed output filenames.
use super::{Component, cancelled, tools, version};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

pub fn replace_file(from: &Path, to: &Path) -> Result<()> {
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from = windows::core::HSTRING::from(from.as_os_str());
    let to = windows::core::HSTRING::from(to.as_os_str());
    // SAFETY: both path strings outlive this synchronous rename on the same volume.
    unsafe {
        MoveFileExW(
            &from,
            &to,
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .context("cannot replace update file")
}

pub fn check_space(path: &Path, bytes: u64) -> Result<()> {
    let path = windows::core::HSTRING::from(path.as_os_str());
    let mut available = 0;
    // SAFETY: path and output pointer remain valid throughout the synchronous query.
    unsafe {
        windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            &path,
            Some(&mut available),
            None,
            None,
        )
    }
    .context("cannot query free disk space")?;
    ensure!(
        available >= bytes.saturating_add(bytes / 10),
        "not enough free disk space"
    );
    Ok(())
}

pub(super) fn reject_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(ancestor) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "linked update folders are unsupported"
            );
        }
    }
    Ok(())
}

pub fn cleanup_installers() -> Result<()> {
    let Some(root) = tools::root() else {
        return Ok(());
    };
    let directory = root.join("updates");
    reject_links(&directory)?;
    if !directory.exists() {
        return Ok(());
    }
    let running = version(env!("CARGO_PKG_VERSION")).context("invalid running version")?;
    for entry in fs::read_dir(directory).context("cannot list staged updates")? {
        let entry = entry.context("cannot list staged updates")?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let obsolete = name
            .strip_suffix(".part")
            .unwrap_or(name)
            .strip_prefix("Macafon_")
            .and_then(|name| name.strip_suffix("_x64-setup.exe"))
            .and_then(version)
            .is_some_and(|value| value <= running);
        if obsolete && entry.file_type()?.is_file() {
            fs::remove_file(entry.path()).context("cannot remove obsolete installer")?;
        }
    }
    Ok(())
}

pub fn launch_installer(path: &Path) -> Result<()> {
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP; the daemon follows its normal quit path.
    Command::new(path)
        .args(["/P", "/UPDATE", "/R"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x00000008 | 0x00000200)
        .spawn()
        .context("cannot start Macafon installer")?;
    Ok(())
}

fn records(directory: &Path) -> Result<BTreeMap<String, String>> {
    let path = directory.join("tools.json");
    reject_links(&path)?;
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_slice(&fs::read(path).context("cannot read managed tool versions")?)
        .context("invalid managed tool versions")
}
fn save_records(directory: &Path, records: &BTreeMap<String, String>) -> Result<()> {
    let path = directory.join("tools.json");
    let temporary = path.with_extension("json.part");
    let result = (|| {
        reject_links(&path)?;
        reject_links(&temporary)?;
        let mut file = File::create(&temporary).context("cannot save managed tool versions")?;
        serde_json::to_writer(&mut file, records).context("cannot save managed tool versions")?;
        file.sync_all()
            .context("cannot flush managed tool versions")?;
        drop(file);
        replace_file(&temporary, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
fn key(component: Component) -> Result<&'static str> {
    match component {
        Component::YtDlp => Ok("yt-dlp"),
        Component::Ffmpeg => Ok("ffmpeg"),
        Component::App => anyhow::bail!("the app is not a managed tool"),
    }
}

pub fn install_tool(
    component: Component,
    downloaded: &Path,
    tag: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let directory = tools::root()
        .context("LOCALAPPDATA is unavailable")?
        .join("tools");
    install_tool_in(&directory, component, downloaded, tag, cancel)
}

pub(super) fn install_tool_in(
    directory: &Path,
    component: Component,
    downloaded: &Path,
    tag: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let name = key(component)?;
    let target = directory.join(name);
    let staging = target.with_extension("staging");
    let backup = target.with_extension("previous");
    reject_links(&target)?;
    reject_links(&staging)?;
    reject_links(&backup)?;
    if staging.exists() {
        fs::remove_dir_all(&staging).context("cannot clean tool staging folder")?;
    }
    fs::create_dir_all(&staging).context("cannot create tool staging folder")?;
    let result = (|| {
        if component == Component::YtDlp {
            fs::copy(downloaded, staging.join("yt-dlp.exe")).context("cannot stage yt-dlp")?;
        } else {
            extract_ffmpeg(downloaded, &staging, cancel)?;
        }
        cancelled(cancel)?;
        let mut versions = records(directory)?;
        versions.insert(name.into(), tag.into());
        if backup.exists() {
            fs::remove_dir_all(&backup).context("cannot clean previous managed tool")?;
        }
        let had_previous = target.exists();
        if had_previous {
            fs::rename(&target, &backup).context("cannot move previous managed tool")?;
        }
        if fs::rename(&staging, &target).is_err() {
            if had_previous {
                let _ = fs::rename(&backup, &target);
            }
            anyhow::bail!("cannot replace managed tool");
        }
        if let Err(error) = save_records(directory, &versions) {
            let _ = fs::remove_dir_all(&target);
            if had_previous {
                let _ = fs::rename(&backup, &target);
            }
            return Err(error);
        }
        // Replacement and its version record are committed together; cancellation was checked
        // immediately before this short rename transaction.
        if had_previous {
            let _ = fs::remove_dir_all(&backup);
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

pub fn remove_tool(component: Component, cancel: &AtomicBool) -> Result<()> {
    let directory = tools::root()
        .context("LOCALAPPDATA is unavailable")?
        .join("tools");
    remove_tool_in(&directory, component, cancel)
}

fn remove_tool_in(directory: &Path, component: Component, cancel: &AtomicBool) -> Result<()> {
    let name = key(component)?;
    let target = directory.join(name);
    reject_links(&target)?;
    let mut versions = records(directory)?;
    versions.remove(name);
    cancelled(cancel)?;
    if !target.exists() {
        if directory.exists() {
            save_records(directory, &versions)?;
        }
        return Ok(());
    }
    let removed = target.with_extension("removed");
    reject_links(&removed)?;
    if removed.exists() {
        fs::remove_dir_all(&removed).context("cannot clean removed tool")?;
    }
    fs::rename(&target, &removed).context("cannot remove managed tool")?;
    if let Err(error) = save_records(directory, &versions) {
        let _ = fs::rename(&removed, &target);
        return Err(error);
    }
    fs::remove_dir_all(&removed).context("cannot delete removed tool")
}

fn tar(
    archive: &Path,
    args: &[&std::ffi::OsStr],
    output: &Path,
    limit: u64,
    cancel: &AtomicBool,
) -> Result<()> {
    let executable = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .context("SystemRoot is unavailable")?
        .join("System32/tar.exe");
    let file = File::create(output).context("cannot create extracted file")?;
    let mut child = Command::new(executable)
        .arg(args[0])
        .arg(archive)
        .args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .context("Windows tar.exe is unavailable")?;
    let started = Instant::now();
    let result = (|| {
        loop {
            cancelled(cancel)?;
            ensure!(
                started.elapsed() < Duration::from_secs(120),
                "ffmpeg extraction timed out"
            );
            ensure!(
                fs::metadata(output)?.len() <= limit,
                "extracted file is too large"
            );
            if let Some(status) = child.try_wait()? {
                ensure!(status.success(), "cannot extract ffmpeg archive");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

fn extract_ffmpeg(archive: &Path, staging: &Path, cancel: &AtomicBool) -> Result<()> {
    use std::ffi::OsStr;
    let listing = staging.join("members.txt");
    tar(archive, &[OsStr::new("-tf")], &listing, 1024 * 1024, cancel)?;
    let members = fs::read_to_string(&listing).context("invalid ffmpeg archive member names")?;
    fs::remove_file(&listing)?;
    for name in ["ffmpeg.exe", "ffprobe.exe"] {
        let suffix = format!("/bin/{name}");
        let matches: Vec<_> = members
            .lines()
            .filter(|member| member.ends_with(&suffix))
            .collect();
        ensure!(
            matches.len() == 1,
            "ffmpeg executable is missing or ambiguous in archive"
        );
        let member = matches[0];
        ensure!(
            !member.starts_with('-')
                && !member.contains(['\\', '\r', '\0'])
                && member.split('/').all(|part| !matches!(part, ".." | "")),
            "invalid ffmpeg archive member"
        );
        // -O writes the selected member to stdout. No archive path is ever extracted to disk;
        // stdout is redirected to a fixed filename in our freshly created staging directory.
        tar(
            archive,
            &[OsStr::new("-xOf"), OsStr::new(member)],
            &staging.join(name),
            300 * 1024 * 1024,
            cancel,
        )?;
        ensure!(
            fs::metadata(staging.join(name))?.len() > 0,
            "empty ffmpeg executable"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_replacement_rolls_back_and_removal_keeps_other_copies() {
        let root =
            std::env::temp_dir().join(format!("macafon-tool-transaction-{}", std::process::id()));
        fs::create_dir_all(&root).expect("fixture directory");
        let downloaded = root.join("downloaded");
        let directory = root.join("tools");
        fs::write(root.join("custom.exe"), b"custom").expect("custom tool");
        fs::write(&downloaded, b"first").expect("fixture");
        let cancel = AtomicBool::new(false);
        install_tool_in(&directory, Component::YtDlp, &downloaded, "first", &cancel)
            .expect("first install");
        fs::write(&downloaded, b"second").expect("fixture");
        // A failed metadata commit must restore the existing executable and recorded tag.
        fs::create_dir(directory.join("tools.json.part")).expect("block metadata commit");
        assert!(
            install_tool_in(&directory, Component::YtDlp, &downloaded, "second", &cancel).is_err()
        );
        assert_eq!(
            fs::read(directory.join("yt-dlp/yt-dlp.exe")).expect("previous executable"),
            b"first"
        );
        assert_eq!(records(&directory).expect("records")["yt-dlp"], "first");
        fs::remove_dir(directory.join("tools.json.part")).expect("unblock metadata");
        install_tool_in(&directory, Component::YtDlp, &downloaded, "second", &cancel)
            .expect("replace");
        assert_eq!(records(&directory).expect("records")["yt-dlp"], "second");
        assert!(
            install_tool_in(
                &directory,
                Component::YtDlp,
                &downloaded,
                "cancelled",
                &AtomicBool::new(true)
            )
            .is_err()
        );
        assert_eq!(records(&directory).expect("records")["yt-dlp"], "second");
        remove_tool_in(&directory, Component::YtDlp, &cancel).expect("remove managed copy");
        assert!(!directory.join("yt-dlp").exists());
        assert_eq!(
            fs::read(root.join("custom.exe")).expect("custom retained"),
            b"custom"
        );
        fs::remove_dir_all(root).expect("cleanup fixture");
    }
    #[test]
    fn tar_streams_only_the_selected_binaries() {
        let directory =
            std::env::temp_dir().join(format!("macafon-updates-tar-{}", std::process::id()));
        fs::create_dir_all(directory.join("build/bin")).expect("fixture directory");
        fs::write(directory.join("build/bin/ffmpeg.exe"), b"ffmpeg").expect("fixture");
        fs::write(directory.join("build/bin/ffprobe.exe"), b"ffprobe").expect("fixture");
        fs::write(directory.join("build/bin/ignored.exe"), b"ignored").expect("fixture");
        let archive = directory.join("archive.zip");
        let tar = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
            .join("System32/tar.exe");
        assert!(
            Command::new(tar)
                .arg("-a")
                .arg("-cf")
                .arg(&archive)
                .arg("-C")
                .arg(&directory)
                .arg("build")
                .status()
                .expect("tar")
                .success()
        );
        let staging = directory.join("staging");
        fs::create_dir(&staging).expect("staging");
        extract_ffmpeg(&archive, &staging, &AtomicBool::new(false)).expect("extract");
        assert_eq!(
            fs::read(staging.join("ffmpeg.exe")).expect("ffmpeg"),
            b"ffmpeg"
        );
        assert_eq!(fs::read_dir(staging).expect("members").count(), 2);
        fs::remove_dir_all(directory).expect("cleanup fixture");
    }
}
