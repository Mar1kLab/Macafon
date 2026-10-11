//! Reversible system-default ownership, persisted before changing PipeWire metadata.
use crate::{Connection, Error};
use pipewire::{metadata::Metadata, types::ObjectType};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
};

const KEYS: [&str; 2] = ["default.configured.audio.source", "default.audio.source"];
const SOURCE: &str = "macafon_mic";
const VALUE: &str = r#"{"name":"macafon_mic"}"#;
type Properties = BTreeMap<String, Option<(String, String)>>;

/// Stable user-local journal shared by the daemon and the uninstall/recovery CLI.
pub fn default_source_journal() -> Result<PathBuf, Error> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or_else(|| Error::Message("HOME or XDG_STATE_HOME is required".into()))?;
    Ok(base.join("macafon/default-source.json"))
}

struct Defaults {
    _listener: pipewire::metadata::MetadataListener,
    metadata: Metadata,
    values: Rc<RefCell<Properties>>,
    connection: Connection,
    source_available: bool,
    cookie: u32,
}

impl Defaults {
    fn connect() -> Result<Self, Error> {
        let connection = Connection::new()?;
        let cookie = Rc::new(Cell::new(None));
        let observed_cookie = cookie.clone();
        let _core = connection
            .core
            .add_listener_local()
            .info(move |info| {
                observed_cookie.set(Some(info.cookie()));
            })
            .register();
        let registry = connection.core.get_registry()?;
        let found = Rc::new(RefCell::new(None));
        let collected = found.clone();
        let source = Rc::new(Cell::new(false));
        let source_found = source.clone();
        let _listener = registry
            .add_listener_local()
            .global(move |global| {
                if global.type_ == ObjectType::Node
                    && global
                        .props
                        .is_some_and(|props| props.get("node.name") == Some(SOURCE))
                {
                    source_found.set(true);
                }
                if global.type_ == ObjectType::Metadata
                    && global
                        .props
                        .is_some_and(|props| props.get("metadata.name") == Some("default"))
                {
                    *collected.borrow_mut() = Some(global.id);
                }
            })
            .register();
        connection.roundtrip()?;
        let id = (*found.borrow())
            .ok_or_else(|| Error::Message("PipeWire default metadata is unavailable".into()))?;
        let global = pipewire::registry::GlobalObject {
            id,
            permissions: pipewire::permissions::PermissionFlags::all(),
            type_: ObjectType::Metadata,
            version: 3,
            props: None::<&pipewire::spa::utils::dict::DictRef>,
        };
        let metadata: Metadata = registry.bind(&global)?;
        let values = Rc::new(RefCell::new(
            KEYS.into_iter()
                .map(|key| (key.to_owned(), None))
                .collect::<Properties>(),
        ));
        let collected = values.clone();
        let listener = metadata
            .add_listener_local()
            .property(move |subject, key, type_, value| {
                if subject == 0 {
                    let mut values = collected.borrow_mut();
                    if let Some(key) = key.filter(|key| KEYS.contains(key)) {
                        values.insert(
                            key.to_owned(),
                            value.map(|value| {
                                (
                                    type_.unwrap_or("Spa:String:JSON").to_owned(),
                                    value.to_owned(),
                                )
                            }),
                        );
                    } else if key.is_none() {
                        values.values_mut().for_each(|value| *value = None);
                    }
                }
                0
            })
            .register();
        connection.roundtrip()?;
        drop(_listener);
        drop(registry);
        drop(_core);
        Ok(Self {
            connection,
            metadata,
            values,
            _listener: listener,
            source_available: source.get(),
            cookie: cookie
                .get()
                .ok_or_else(|| Error::Message("PipeWire server identity is unavailable".into()))?,
        })
    }
    fn set(&self, key: &str, value: Option<&(String, String)>) -> Result<(), Error> {
        if value.is_some_and(|(kind, value)| kind.contains('\0') || value.contains('\0')) {
            return Err(Error::Message("Invalid saved default metadata".into()));
        }
        self.metadata.set_property(
            0,
            key,
            value.map(|(kind, _)| kind.as_str()),
            value.map(|(_, value)| value.as_str()),
        );
        self.connection.roundtrip()
    }
}

fn io_error(error: impl std::fmt::Display) -> Error {
    Error::Message(format!("Default-source journal: {error}"))
}
fn owns(value: &Option<(String, String)>) -> bool {
    value
        .as_ref()
        .and_then(|(_, value)| serde_json::from_str::<serde_json::Value>(value).ok())
        .is_some_and(|value| value.get("name").and_then(serde_json::Value::as_str) == Some(SOURCE))
}

/// Call only after explicit confirmation. Never forget the previous values on repeated calls.
pub fn set_default_source(path: &Path) -> Result<(), Error> {
    let defaults = Defaults::connect()?;
    if !defaults.source_available {
        return Err(Error::Message("Macafon Microphone is unavailable".into()));
    }
    if path.exists() {
        let saved = read_journal(path)?;
        if KEYS.iter().all(|key| owns(&defaults.values.borrow()[*key])) {
            return Ok(());
        }
        restore(&defaults, &saved)?;
        fs::remove_file(path).map_err(io_error)?;
    }
    let parent = path.parent().ok_or_else(|| io_error("missing parent"))?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(io_error)?;
    let previous = serde_json::to_vec(
        &serde_json::json!({"cookie": defaults.cookie, "properties": &*defaults.values.borrow()}),
    )
    .map_err(io_error)?;
    persist_journal(path, &previous)?;
    let value = ("Spa:String:JSON".to_owned(), VALUE.to_owned());
    for key in KEYS {
        defaults.set(key, Some(&value))?;
    }
    Ok(())
}

fn persist_journal(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(io_error)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        // Linking publishes a complete durable file atomically and refuses an existing journal.
        fs::hard_link(&temporary, path).map_err(io_error)?;
        let parent = path.parent().ok_or_else(|| io_error("missing parent"))?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(io_error)
    })();
    let _ = fs::remove_file(temporary);
    result
}

struct Journal {
    cookie: u32,
    properties: Properties,
}
fn read_journal(path: &Path) -> Result<Journal, Error> {
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(path).map_err(io_error)?).map_err(io_error)?;
    let cookie = document
        .get("cookie")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| io_error("invalid server identity"))?;
    let properties: Properties = serde_json::from_value(
        document
            .get("properties")
            .cloned()
            .ok_or_else(|| io_error("missing properties"))?,
    )
    .map_err(io_error)?;
    if properties.len() != KEYS.len() || !KEYS.iter().all(|key| properties.contains_key(*key)) {
        return Err(io_error("invalid keys"));
    }
    Ok(Journal { cookie, properties })
}
fn restore(defaults: &Defaults, saved: &Journal) -> Result<(), Error> {
    for key in KEYS {
        // Compare ownership immediately before each write; a later user choice wins.
        defaults.connection.roundtrip()?;
        let current = defaults.values.borrow()[key].clone();
        // A new server can have empty metadata after a restart. A missing value on the
        // same server is a user's explicit reset and must remain untouched.
        if owns(&current) || (defaults.cookie != saved.cookie && current.is_none()) {
            defaults.set(key, saved.properties[key].as_ref())?;
        }
    }
    Ok(())
}

/// Restore after Quit, daemon crash recovery or uninstall; keep the journal if PipeWire is down.
pub fn restore_default_source(path: &Path) -> Result<(), Error> {
    if !path.exists() {
        return Ok(());
    }
    let saved = read_journal(path)?;
    let defaults = Defaults::connect()?;
    restore(&defaults, &saved)?;
    fs::remove_file(path).map_err(io_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_uses_node_name_not_substring() {
        assert!(owns(&Some(("Spa:String:JSON".into(), VALUE.into()))));
        assert!(!owns(&Some((
            "Spa:String:JSON".into(),
            r#"{"name":"other","description":"macafon_mic"}"#.into()
        ))));
        assert!(!owns(&None));
    }
}
