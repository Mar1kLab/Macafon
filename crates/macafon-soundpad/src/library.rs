//! One-level sound folders. Symlinks/reparse points are rejected to keep IDs inside the root.
use crate::Error;
use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const SUPPORTED_EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac"];
pub const UNPLAYABLE_CATEGORY: &str = "Unplayable";
pub const MAX_FILE_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundEntry {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub category: Option<String>,
    pub size_bytes: u64,
    pub modified_unix_ms: u64,
    pub added_unix_ms: u64,
}

pub fn validate_name(name: &str) -> Result<(), Error> {
    let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if name.eq_ignore_ascii_case(UNPLAYABLE_CATEGORY)
        || name.is_empty()
        || name.starts_with('.')
        || name.ends_with(['.', ' '])
        || reserved
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
    {
        return Err(Error::InvalidName(name.into()));
    }
    Ok(())
}
fn is_supported(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        SUPPORTED_EXTENSIONS
            .iter()
            .any(|ext| s.eq_ignore_ascii_case(ext))
    })
}
fn ordinary_metadata(path: &Path) -> Result<fs::Metadata, Error> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(Error::InvalidId(path.display().to_string()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(Error::InvalidId(path.display().to_string()));
        }
    }
    Ok(meta)
}
fn is_hidden(path: &Path, _meta: &fs::Metadata) -> bool {
    if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if _meta.file_attributes() & (2 | 4) != 0 {
            return true;
        }
    }
    false
}
fn prepare_root(root: &Path) -> Result<PathBuf, Error> {
    fs::create_dir_all(root)?;
    Ok(fs::canonicalize(root)?)
}
fn sound_entry(root: &Path, path: &Path) -> Result<SoundEntry, Error> {
    let meta = ordinary_metadata(path)?;
    sound_entry_with_metadata(root, path, &meta)
}
fn sound_entry_with_metadata(
    root: &Path,
    path: &Path,
    meta: &fs::Metadata,
) -> Result<SoundEntry, Error> {
    if !meta.is_file() {
        return Err(Error::InvalidId(path.display().to_string()));
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| Error::InvalidId(path.display().to_string()))?;
    let id = relative
        .to_str()
        .ok_or_else(|| Error::InvalidId(path.display().to_string()))?
        .replace('\\', "/");
    let category = id.split_once('/').map(|(category, _)| category.to_owned());
    Ok(SoundEntry {
        id,
        path: path.into(),
        category,
        name: path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .into(),
        size_bytes: meta.len(),
        added_unix_ms: meta
            .created()
            .or_else(|_| meta.modified())?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64,
        modified_unix_ms: meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64,
    })
}
pub fn resolve(root: &Path, id: &str) -> Result<PathBuf, Error> {
    let parts: Vec<_> = id.split('/').collect();
    if parts.is_empty()
        || parts.len() > 2
        || parts.iter().enumerate().any(|(index, part)| {
            !(index == 0 && parts.len() == 2 && *part == UNPLAYABLE_CATEGORY)
                && validate_name(part).is_err()
        })
        || !is_supported(Path::new(id))
    {
        return Err(Error::InvalidId(id.into()));
    }
    let root = prepare_root(root)?;
    let mut path = root;
    for part in parts {
        path.push(part);
        ordinary_metadata(&path)?;
    }
    if !path.is_file() {
        return Err(Error::InvalidId(id.into()));
    }
    Ok(path)
}
pub fn categories(root: &Path) -> Result<Vec<String>, Error> {
    let root = prepare_root(root)?;
    let mut names = Vec::new();
    for item in fs::read_dir(root)? {
        let path = item?.path();
        if let Ok(meta) = ordinary_metadata(&path) {
            if meta.is_dir() && !is_hidden(&path, &meta) {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if validate_name(name).is_ok() {
                        names.push(name.into());
                    }
                }
            }
        }
    }
    names.sort_by_key(|n: &String| (n.to_lowercase(), n.clone()));
    Ok(names)
}
pub fn scan(root: &Path) -> Result<Vec<SoundEntry>, Error> {
    scan_library(root).map(|(entries, _)| entries)
}

/// One inventory including empty first-level category folders.
pub fn scan_library(root: &Path) -> Result<(Vec<SoundEntry>, Vec<String>), Error> {
    let root = prepare_root(root)?;
    let mut entries = Vec::new();
    let mut dirs = vec![root.clone()];
    let categories = categories(&root)?;
    dirs.extend(categories.iter().map(|n| root.join(n)));
    for dir in dirs {
        scan_directory(&root, &dir, &mut entries)?;
    }
    entries.sort_by_cached_key(|e| {
        (
            e.category.as_ref().map(|s| s.to_lowercase()),
            e.name.to_lowercase(),
            e.id.clone(),
        )
    });
    Ok((entries, categories))
}
/// A checked inventory used by the daemon's background scan worker.
pub struct PlayableInventory {
    pub entries: Vec<SoundEntry>,
    pub categories: Vec<String>,
    pub problems: Vec<(SoundEntry, String)>,
}
pub fn scan_playable_library(root: &Path) -> Result<PlayableInventory, Error> {
    let (entries, categories) = scan_library(root)?;
    let mut playable = Vec::with_capacity(entries.len());
    for entry in entries {
        match crate::decode::probe_playable(&entry.path) {
            Err(error) if error.is_unplayable() => {
                // A failed move (e.g. a sharing lock) leaves the original visible for a retry.
                if quarantine(root, &entry.id).is_err() {
                    playable.push(entry);
                }
            }
            _ => playable.push(entry),
        }
    }
    let problems = problem_entries(root)?
        .into_iter()
        .map(|entry| {
            let reason = crate::decode::probe_playable(&entry.path)
                .err()
                .filter(Error::is_unplayable)
                .map(|error| error.to_string())
                .unwrap_or_else(|| "Cannot be played".into());
            (entry, reason)
        })
        .collect();
    Ok(PlayableInventory {
        entries: playable,
        categories,
        problems,
    })
}

pub fn problem_entries(root: &Path) -> Result<Vec<SoundEntry>, Error> {
    let root = prepare_root(root)?;
    let directory = root.join(UNPLAYABLE_CATEGORY);
    match ordinary_metadata(&directory) {
        Ok(meta) if meta.is_dir() => {}
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
        _ => return Err(Error::InvalidName(UNPLAYABLE_CATEGORY.into())),
    }
    let mut entries = Vec::new();
    scan_directory(&root, &directory, &mut entries)?;
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(entries)
}

/// Move without overwriting, using the same collision naming as ordinary imports.
pub fn quarantine(root: &Path, id: &str) -> Result<SoundEntry, Error> {
    let source = resolve(root, id)?;
    let root = prepare_root(root)?;
    let directory = root.join(UNPLAYABLE_CATEGORY);
    if source.parent() == Some(directory.as_path()) {
        return sound_entry(&root, &source);
    }
    match fs::create_dir(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    if !ordinary_metadata(&directory)?.is_dir() {
        return Err(Error::InvalidName(UNPLAYABLE_CATEGORY.into()));
    }
    let filename = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::InvalidId(id.into()))?;
    let target = unique_path(&directory, filename);
    rename_no_replace(&source, &target)?;
    sound_entry(&root, &target)
}

pub fn delete_problem_sounds(root: &Path) -> Result<Vec<String>, Error> {
    let mut deleted = Vec::new();
    for entry in problem_entries(root)? {
        delete(root, &entry.id)?;
        deleted.push(entry.id);
    }
    Ok(deleted)
}

fn scan_directory(root: &Path, dir: &Path, entries: &mut Vec<SoundEntry>) -> Result<(), Error> {
    for item in fs::read_dir(dir)? {
        let path = item?.path();
        if let Ok(meta) = ordinary_metadata(&path) {
            if meta.is_file()
                && !is_hidden(&path, &meta)
                && is_supported(&path)
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| validate_name(n).is_ok())
            {
                entries.push(sound_entry_with_metadata(root, &path, &meta)?);
            }
        }
    }
    Ok(())
}
fn category_directory(root: &Path, category: Option<&str>) -> Result<PathBuf, Error> {
    let root = prepare_root(root)?;
    if let Some(name) = category {
        validate_name(name)?;
        let dir = root.join(name);
        match fs::create_dir(&dir) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        if !ordinary_metadata(&dir)?.is_dir() {
            return Err(Error::InvalidName(name.into()));
        }
        Ok(dir)
    } else {
        Ok(root)
    }
}
fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let path = Path::new(filename);
    let stem = path.file_stem().and_then(|n| n.to_str()).unwrap_or("sound");
    let ext = path.extension().and_then(|n| n.to_str()).unwrap_or("");
    let mut target = dir.join(filename);
    let mut number = 2_u64;
    while target.symlink_metadata().is_ok() {
        target = dir.join(format!("{stem} ({number}).{ext}"));
        number += 1;
    }
    target
}
pub fn import(root: &Path, source: &Path, category: Option<&str>) -> Result<SoundEntry, Error> {
    if !is_supported(source) {
        return Err(Error::UnsupportedFormat);
    }
    let meta = fs::metadata(source)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge);
    }
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::InvalidName(source.display().to_string()))?;
    validate_name(name)?;
    let dir = category_directory(root, category)?;
    let target = unique_path(&dir, name);
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)?;
    let copy = (|| {
        use std::io::Read;
        let mut input = fs::File::open(source)?.take(MAX_FILE_BYTES + 1);
        if std::io::copy(&mut input, &mut output)? > MAX_FILE_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(())
    })();
    drop(output);
    if let Err(e) = copy {
        let _ = fs::remove_file(&target);
        return Err(e);
    }
    sound_entry(&prepare_root(root)?, &target)
}
/// Stage uploads on the library's volume so finalization is a rename, including other drives.
pub fn upload_directory(root: &Path) -> Result<PathBuf, Error> {
    let directory = prepare_root(root)?.join(".macafon-uploads");
    match fs::create_dir(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    if !ordinary_metadata(&directory)?.is_dir() {
        return Err(Error::InvalidId(directory.display().to_string()));
    }
    Ok(directory)
}

/// Consume a completed staged upload without copying its payload on the control loop.
pub fn commit_upload(
    root: &Path,
    source: &Path,
    category: Option<&str>,
) -> Result<SoundEntry, Error> {
    let metadata = ordinary_metadata(source)?;
    if !metadata.is_file() || !is_supported(source) {
        return Err(Error::UnsupportedFormat);
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge);
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::InvalidId(source.display().to_string()))?;
    validate_name(name)?;
    let directory = category_directory(root, category)?;
    let target = unique_path(&directory, name);
    rename_no_replace(source, &target)?;
    sound_entry(&prepare_root(root)?, &target)
}

fn rename_no_replace(source: &Path, target: &Path) -> Result<(), Error> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileW(source: *const u16, target: *const u16) -> i32;
        }
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: both pointers reference terminated paths live for this synchronous call.
        if unsafe { MoveFileW(source.as_ptr(), target.as_ptr()) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    {
        if source.is_dir() {
            fs::rename(source, target)?;
        } else {
            fs::hard_link(source, target)?;
            fs::remove_file(source)?;
        }
    }
    Ok(())
}

pub fn rename(root: &Path, id: &str, new_name: &str) -> Result<SoundEntry, Error> {
    validate_name(new_name)?;
    let source = resolve(root, id)?;
    let mut target = source.clone();
    let ext = source
        .extension()
        .and_then(|s| s.to_str())
        .ok_or(Error::UnsupportedFormat)?;
    target.set_file_name(format!("{new_name}.{ext}"));
    if target.symlink_metadata().is_ok() {
        return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists).into());
    }
    rename_no_replace(&source, &target)?;
    sound_entry(&prepare_root(root)?, &target)
}
pub fn move_to_category(
    root: &Path,
    id: &str,
    category: Option<&str>,
) -> Result<SoundEntry, Error> {
    let source = resolve(root, id)?;
    let dir = category_directory(root, category)?;
    if source.parent() == Some(dir.as_path()) {
        return sound_entry(&prepare_root(root)?, &source);
    }
    let filename = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::InvalidId(id.into()))?;
    let target = unique_path(&dir, filename);
    fs::rename(source, &target)?;
    sound_entry(&prepare_root(root)?, &target)
}
pub fn delete(root: &Path, id: &str) -> Result<(), Error> {
    fs::remove_file(resolve(root, id)?)?;
    Ok(())
}
pub fn create_category(root: &Path, name: &str) -> Result<(), Error> {
    category_directory(root, Some(name))?;
    Ok(())
}
pub fn rename_category(root: &Path, from: &str, to: &str) -> Result<(), Error> {
    validate_name(from)?;
    validate_name(to)?;
    let root = prepare_root(root)?;
    let source = root.join(from);
    if !ordinary_metadata(&source)?.is_dir() {
        return Err(Error::InvalidName(from.into()));
    }
    let target = root.join(to);
    if target.symlink_metadata().is_ok() {
        return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists).into());
    }
    rename_no_replace(&source, &target)?;
    Ok(())
}
pub fn delete_category(root: &Path, name: &str) -> Result<Vec<(String, String)>, Error> {
    validate_name(name)?;
    let root = prepare_root(root)?;
    let dir = root.join(name);
    if !ordinary_metadata(&dir)?.is_dir() {
        return Err(Error::InvalidName(name.into()));
    }
    let mut moved = Vec::new();
    let mut sounds = Vec::new();
    scan_directory(&root, &dir, &mut sounds)?;
    sounds.sort_by(|a, b| a.id.cmp(&b.id));
    for sound in sounds {
        let destination = move_to_category(&root, &sound.id, None)?;
        moved.push((sound.id, destination.id));
    }
    if fs::read_dir(&dir)?.next().is_none() {
        fs::remove_dir(dir)?;
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn garbage_moves_to_reserved_folder_and_valid_wav_stays() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("broken.mp3"), b"not an audio stream").unwrap();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&38u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&48000u32.to_le_bytes());
        wav.extend_from_slice(&96000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&2u32.to_le_bytes());
        wav.extend_from_slice(&0i16.to_le_bytes());
        fs::write(root.join("valid.wav"), wav).unwrap();
        let inventory = scan_playable_library(root).unwrap();
        assert_eq!(
            inventory
                .entries
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["valid.wav"]
        );
        assert!(inventory.categories.is_empty());
        assert_eq!(inventory.problems.len(), 1);
        assert_eq!(inventory.problems[0].0.id, "Unplayable/broken.mp3");
        assert!(!inventory.problems[0].1.is_empty());
        assert!(!root.join("broken.mp3").exists());
        assert!(root.join("valid.wav").exists());
        assert!(create_category(root, "unplayable").is_err());
        assert!(rename_category(root, "other", "Unplayable").is_err());
        fs::write(root.join("broken.mp3"), b"another invalid stream").unwrap();
        assert_eq!(scan_playable_library(root).unwrap().problems.len(), 2);
        assert!(root.join("Unplayable/broken (2).mp3").exists());
        delete(root, "Unplayable/broken.mp3").unwrap();
        assert_eq!(
            delete_problem_sounds(root).unwrap(),
            ["Unplayable/broken (2).mp3"]
        );
        assert!(problem_entries(root).unwrap().is_empty());
    }

    #[test]
    fn explorer_folders_include_empty_categories_and_ignore_unmanaged_content() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("root.wav"), b"root").unwrap();
        fs::create_dir_all(root.join("Short sounds/deeper")).unwrap();
        fs::write(root.join("Short sounds/tone.wav"), b"tone").unwrap();
        fs::write(root.join("Short sounds/deeper/ignored.wav"), b"nested").unwrap();
        fs::write(root.join("Short sounds/notes.txt"), b"notes").unwrap();
        fs::write(root.join(".hidden.wav"), b"hidden").unwrap();
        fs::create_dir(root.join("Empty")).unwrap();
        let (entries, categories) = scan_library(root).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(categories, ["Empty", "Short sounds"]);
        fs::rename(root.join("Short sounds"), root.join("Renamed")).unwrap();
        fs::remove_dir(root.join("Empty")).unwrap();
        assert_eq!(super::categories(root).unwrap(), ["Renamed"]);
        assert!(
            scan(root)
                .unwrap()
                .iter()
                .any(|e| e.id == "Renamed/tone.wav")
        );
        delete_category(root, "Renamed").unwrap();
        assert!(root.join("tone.wav").exists());
        // Never destroy unsupported or deeper content to make a category disappear.
        assert!(root.join("Renamed/notes.txt").exists());
        assert!(root.join("Renamed/deeper/ignored.wav").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_hidden_and_system_files_are_not_managed() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        for (name, attributes) in [("hidden.wav", 2), ("system.wav", 4)] {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .attributes(attributes)
                .open(temp.path().join(name))
                .unwrap();
        }
        assert!(scan(temp.path()).unwrap().is_empty());
        assert!(temp.path().join("system.wav").exists());
    }

    #[test]
    fn staged_uploads_finalize_in_the_category_without_overwriting_files() {
        let temp = tempfile::tempdir().unwrap();
        let stage = upload_directory(temp.path()).unwrap().join("upload");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("tone.wav"), b"first").unwrap();
        commit_upload(temp.path(), &stage.join("tone.wav"), Some("Short sounds")).unwrap();
        fs::write(stage.join("tone.wav"), b"second").unwrap();
        let second =
            commit_upload(temp.path(), &stage.join("tone.wav"), Some("Short sounds")).unwrap();
        assert_eq!(second.id, "Short sounds/tone (2).wav");
        assert!(!stage.join("tone.wav").exists());
        assert_eq!(
            fs::read(temp.path().join("Short sounds/tone.wav")).unwrap(),
            b"first"
        );
        assert_eq!(fs::read(second.path).unwrap(), b"second");
    }

    #[test]
    fn moves_never_replace_a_destination_created_after_collision_selection() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.wav");
        let target = temp.path().join("target.wav");
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"destination").unwrap();
        assert!(rename_no_replace(&source, &target).is_err());
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(target).unwrap(), b"destination");
    }

    #[test]
    fn deleting_category_reports_collision_renames() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("Memes")).unwrap();
        fs::write(temp.path().join("beep.wav"), b"root").unwrap();
        fs::write(temp.path().join("Memes/beep.wav"), b"category").unwrap();
        assert_eq!(
            delete_category(temp.path(), "Memes").unwrap(),
            vec![("Memes/beep.wav".into(), "beep (2).wav".into())]
        );
        assert_eq!(fs::read(temp.path().join("beep.wav")).unwrap(), b"root");
        assert_eq!(
            fs::read(temp.path().join("beep (2).wav")).unwrap(),
            b"category"
        );
    }
    #[test]
    fn folder_operations_and_validation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        assert!(scan(&root).unwrap().is_empty());
        let source = temp.path().join("Beep.WAV");
        fs::write(&source, b"audio").unwrap();
        let first = import(&root, &source, Some("Memes")).unwrap();
        assert!(source.exists());
        assert_eq!(
            import(&root, &source, Some("Memes")).unwrap().id,
            "Memes/Beep (2).WAV"
        );
        let renamed = rename(&root, &first.id, "Airhorn").unwrap();
        let moved = move_to_category(&root, &renamed.id, None).unwrap();
        assert_eq!(moved.id, "Airhorn.WAV");
        create_category(&root, "Empty").unwrap();
        rename_category(&root, "Empty", "aaa").unwrap();
        fs::create_dir_all(root.join("Memes/deep")).unwrap();
        fs::write(root.join("Memes/deep/no.wav"), b"").unwrap();
        fs::write(root.join(".hidden.wav"), b"").unwrap();
        fs::write(root.join("Memes/notes.txt"), b"").unwrap();
        assert_eq!(scan(&root).unwrap().len(), 2);
        assert_eq!(scan(&root).unwrap()[0].id, moved.id);
        assert_eq!(categories(&root).unwrap(), vec!["aaa", "Memes"]);
        delete_category(&root, "Memes").unwrap();
        assert!(root.join("Memes/notes.txt").exists());
        assert!(root.join("Beep (2).WAV").exists());
        delete(&root, &moved.id).unwrap();
        for id in [
            "../outside.wav",
            "/abs.wav",
            "a/b/c.wav",
            "a\\b.wav",
            "C:/x.wav",
            "./x.wav",
            "x.txt",
        ] {
            assert!(resolve(&root, id).is_err(), "{id}");
        }
        for name in [
            "", ".", "..", "a/", "a\\b", "CON", "nul.txt", "COM1", "LPT9", "x.", "x ", "a:b",
            "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "a\nb",
        ] {
            assert!(create_category(&root, name).is_err(), "{name}");
        }
    }
}
