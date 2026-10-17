//! One-level sound folders. Symlinks/reparse points are rejected to keep IDs inside the root.
use crate::Error;
use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const SUPPORTED_EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac"];
pub const MAX_FILE_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundEntry {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub category: Option<String>,
    pub size_bytes: u64,
    pub modified_unix_ms: u64,
}

fn validate_name(name: &str) -> Result<(), Error> {
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
    if name.is_empty()
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
        if _meta.file_attributes() & 2 != 0 {
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
        || parts.iter().any(|p| validate_name(p).is_err())
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
                    names.push(name.into());
                }
            }
        }
    }
    names.sort_by_key(|n: &String| (n.to_lowercase(), n.clone()));
    Ok(names)
}
pub fn scan(root: &Path) -> Result<Vec<SoundEntry>, Error> {
    let root = prepare_root(root)?;
    let mut entries = Vec::new();
    let mut dirs = vec![root.clone()];
    dirs.extend(categories(&root)?.iter().map(|n| root.join(n)));
    for dir in dirs {
        for item in fs::read_dir(dir)? {
            let path = item?.path();
            if let Ok(meta) = ordinary_metadata(&path) {
                if meta.is_file() && !is_hidden(&path, &meta) && is_supported(&path) {
                    entries.push(sound_entry(&root, &path)?);
                }
            }
        }
    }
    entries.sort_by_key(|e| {
        (
            e.category.as_ref().map(|s| s.to_lowercase()),
            e.name.to_lowercase(),
            e.id.clone(),
        )
    });
    Ok(entries)
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
    fs::rename(source, &target)?;
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
    fs::rename(source, target)?;
    Ok(())
}
pub fn delete_category(root: &Path, name: &str) -> Result<(), Error> {
    validate_name(name)?;
    let root = prepare_root(root)?;
    let dir = root.join(name);
    if !ordinary_metadata(&dir)?.is_dir() {
        return Err(Error::InvalidName(name.into()));
    }
    for sound in scan(&root)?
        .into_iter()
        .filter(|e| e.category.as_deref() == Some(name))
    {
        move_to_category(&root, &sound.id, None)?;
    }
    if fs::read_dir(&dir)?.next().is_none() {
        fs::remove_dir(dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
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
