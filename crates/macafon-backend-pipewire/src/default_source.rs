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

type Properties = BTreeMap<String, Option<(String, String)>>;

/// Which system default Macafon takes over: the input (MIC-012) or the output (OUT-005).
#[derive(Clone, Copy)]
struct Kind {
    keys: [&'static str; 2],
    file: &'static str,
    /// Node names Macafon owns for this default.
    owned: fn(&str) -> bool,
}

const SOURCE: Kind = Kind {
    keys: ["default.configured.audio.source", "default.audio.source"],
    file: "macafon/default-source.json",
    owned: |name| name == "macafon_mic",
};

const SINK: Kind = Kind {
    keys: ["default.configured.audio.sink", "default.audio.sink"],
    file: "macafon/default-sink.json",
    owned: |name| name.starts_with(crate::CHANNEL_PREFIX),
};

fn journal(kind: Kind) -> Result<PathBuf, Error> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or_else(|| Error::Message("HOME or XDG_STATE_HOME is required".into()))?;
    Ok(base.join(kind.file))
}

/// Stable user-local journal shared by the daemon and the uninstall/recovery CLI.
pub fn default_source_journal() -> Result<PathBuf, Error> {
    journal(SOURCE)
}

/// The output counterpart of [`default_source_journal`] (OUT-005).
pub fn default_sink_journal() -> Result<PathBuf, Error> {
    journal(SINK)
}

struct Defaults {
    kind: Kind,
    _listener: pipewire::metadata::MetadataListener,
    metadata: Metadata,
    values: Rc<RefCell<Properties>>,
    connection: Connection,
    node_available: bool,
    cookie: u32,
}

impl Defaults {
    fn connect(kind: Kind, node: &str) -> Result<Self, Error> {
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
        let wanted = node.to_owned();
        let _listener = registry
            .add_listener_local()
            .global(move |global| {
                if global.type_ == ObjectType::Node
                    && global
                        .props
                        .is_some_and(|props| props.get("node.name") == Some(wanted.as_str()))
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
            kind.keys
                .into_iter()
                .map(|key| (key.to_owned(), None))
                .collect::<Properties>(),
        ));
        let collected = values.clone();
        let listener = metadata
            .add_listener_local()
            .property(move |subject, key, type_, value| {
                if subject == 0 {
                    let mut values = collected.borrow_mut();
                    if let Some(key) = key.filter(|key| kind.keys.contains(key)) {
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
            kind,
            connection,
            metadata,
            values,
            _listener: listener,
            node_available: source.get(),
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
fn value_name(value: &Option<(String, String)>) -> Option<String> {
    value
        .as_ref()
        .and_then(|(_, value)| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value.get("name")?.as_str().map(str::to_owned))
}

fn owns(kind: Kind, value: &Option<(String, String)>) -> bool {
    value_name(value).is_some_and(|name| (kind.owned)(&name))
}

/// Call only after explicit confirmation. Never forget the previous values on repeated calls.
pub fn set_default_source(path: &Path) -> Result<(), Error> {
    set_default(
        SOURCE,
        path,
        "macafon_mic",
        "Macafon Microphone is unavailable",
    )
}

/// OUT-005: make channel `node` the default output; the original default is journaled once
/// and survives switching between channels.
pub fn set_default_sink(path: &Path, node: &str) -> Result<(), Error> {
    set_default(
        SINK,
        path,
        node,
        "The default channel is not in the graph yet",
    )
}

/// The output that was the default before Macafon took over, from the journal.
pub fn previous_default_sink(path: &Path) -> Option<String> {
    let saved = read_journal(SINK, path).ok()?;
    SINK.keys
        .iter()
        .find_map(|key| value_name(&saved.properties[*key]))
}

pub fn restore_default_sink(path: &Path) -> Result<(), Error> {
    restore_default(SINK, path)
}

fn set_default(kind: Kind, path: &Path, node: &str, missing: &str) -> Result<(), Error> {
    let defaults = Defaults::connect(kind, node)?;
    if !defaults.node_available {
        return Err(Error::Message(missing.into()));
    }
    let value = (
        "Spa:String:JSON".to_owned(),
        serde_json::json!({ "name": node }).to_string(),
    );
    if path.exists() {
        let saved = read_journal(kind, path)?;
        let current = defaults.values.borrow().clone();
        if kind.keys.iter().all(|key| owns(kind, &current[*key])) {
            // Ours already (maybe another channel): switch without touching the journal.
            for key in kind.keys {
                if value_name(&current[key]).as_deref() != Some(node) {
                    defaults.set(key, Some(&value))?;
                }
            }
            return Ok(());
        }
        if defaults.cookie != saved.cookie {
            // A restarted server starts without our values; the journal still holds the
            // user's choice from before Macafon, so keep it and only adopt the new server.
            let renewed = serde_json::to_vec(
                &serde_json::json!({"cookie": defaults.cookie, "properties": saved.properties}),
            )
            .map_err(io_error)?;
            fs::remove_file(path).map_err(io_error)?;
            persist_journal(path, &renewed)?;
            for key in kind.keys {
                defaults.set(key, Some(&value))?;
            }
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
    for key in kind.keys {
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
fn read_journal(kind: Kind, path: &Path) -> Result<Journal, Error> {
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
    if properties.len() != kind.keys.len()
        || !kind.keys.iter().all(|key| properties.contains_key(*key))
    {
        return Err(io_error("invalid keys"));
    }
    Ok(Journal { cookie, properties })
}
fn restore(defaults: &Defaults, saved: &Journal) -> Result<(), Error> {
    let kind = defaults.kind;
    for key in kind.keys {
        // Compare ownership immediately before each write; a later user choice wins.
        defaults.connection.roundtrip()?;
        let current = defaults.values.borrow()[key].clone();
        // A new server can have empty metadata after a restart. A missing value on the
        // same server is a user's explicit reset and must remain untouched.
        if owns(kind, &current) || (defaults.cookie != saved.cookie && current.is_none()) {
            defaults.set(key, saved.properties[key].as_ref())?;
        }
    }
    Ok(())
}

/// Restore after Quit, daemon crash recovery or uninstall; keep the journal if PipeWire is down.
pub fn restore_default_source(path: &Path) -> Result<(), Error> {
    restore_default(SOURCE, path)
}

fn restore_default(kind: Kind, path: &Path) -> Result<(), Error> {
    if !path.exists() {
        return Ok(());
    }
    let saved = read_journal(kind, path)?;
    let defaults = Defaults::connect(kind, "")?;
    restore(&defaults, &saved)?;
    fs::remove_file(path).map_err(io_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_uses_node_name_not_substring() {
        let json = |value: &str| Some(("Spa:String:JSON".to_owned(), value.to_owned()));
        assert!(owns(SOURCE, &json(r#"{"name":"macafon_mic"}"#)));
        assert!(!owns(
            SOURCE,
            &json(r#"{"name":"other","description":"macafon_mic"}"#)
        ));
        assert!(!owns(SOURCE, &None));
        assert!(owns(SINK, &json(r#"{"name":"macafon_ch_voice"}"#)));
        assert!(!owns(SINK, &json(r#"{"name":"macafon_mic"}"#)));
    }
}
