//! `org.freedesktop.portal.GlobalShortcuts` (HK-003, Wayland and portal-capable desktops).
//! The desktop confirms the shortcuts and may assign other keys; it reports what it chose.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

pub type Results = HashMap<String, OwnedValue>;

pub struct Portal {
    connection: Connection,
    session: Option<OwnedObjectPath>,
}

/// One shortcut of a bind request: id, description, preferred trigger.
pub struct Shortcut {
    pub id: String,
    pub description: String,
    pub trigger: String,
}

fn token() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    format!("macafon{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

impl Portal {
    /// `None` when the session bus has no portal with GlobalShortcuts.
    pub fn connect() -> Option<Self> {
        let connection = Connection::session().ok()?;
        // Newer portals identify host apps by a registered id (GNOME 48); older ones ignore it.
        if let Ok(registry) = Proxy::new(
            &connection,
            DESTINATION,
            PATH,
            "org.freedesktop.host.portal.Registry",
        ) {
            let options: HashMap<&str, Value> = HashMap::new();
            let _: Result<(), _> = registry.call("Register", &("macafon", options));
        }
        let proxy = Proxy::new(&connection, DESTINATION, PATH, INTERFACE).ok()?;
        let version: u32 = proxy.get_property("version").ok()?;
        tracing::info!(version, "GlobalShortcuts portal available");
        Some(Self {
            connection,
            session: None,
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    fn proxy(&self) -> zbus::Result<Proxy<'static>> {
        Proxy::new(&self.connection, DESTINATION, PATH, INTERFACE)
    }

    /// Call a request-style method and wait for its `Response` (the user may see a dialog).
    fn request<B>(&self, method: &str, body: impl FnOnce(String) -> B) -> Result<Results, String>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let token = token();
        let sender = self
            .connection
            .unique_name()
            .ok_or("no unique bus name")?
            .trim_start_matches(':')
            .replace('.', "_");
        let path = format!("{PATH}/request/{sender}/{token}");
        let request = Proxy::new(
            &self.connection,
            DESTINATION,
            path,
            "org.freedesktop.portal.Request",
        )
        .map_err(|error| error.to_string())?;
        // Subscribe before calling, or a fast portal answers before we listen.
        let mut responses = request
            .receive_signal("Response")
            .map_err(|error| error.to_string())?;
        let _: OwnedObjectPath = self
            .proxy()
            .and_then(|proxy| proxy.call(method, &body(token)))
            .map_err(|error| error.to_string())?;
        let message = responses.next().ok_or("portal closed the request")?;
        let (code, results): (u32, Results) = message
            .body()
            .deserialize()
            .map_err(|error| error.to_string())?;
        match code {
            0 => Ok(results),
            1 => Err("cancelled by the user".into()),
            _ => Err("the portal refused".into()),
        }
    }

    /// Replace all shortcuts: a new session each time, so removed ones disappear.
    /// Returns the trigger the desktop assigned to each id.
    pub fn bind(&mut self, shortcuts: &[Shortcut]) -> Result<HashMap<String, String>, String> {
        self.close();
        let results = self.request("CreateSession", |token| {
            let mut options: HashMap<&str, Value> = HashMap::new();
            options.insert("handle_token", Value::from(token));
            options.insert("session_handle_token", Value::from("macafon"));
            (options,)
        })?;
        let session: String = results
            .get("session_handle")
            .and_then(|value| String::try_from(value.clone()).ok())
            .ok_or("no session handle")?;
        let session = OwnedObjectPath::try_from(session).map_err(|error| error.to_string())?;
        self.session = Some(session.clone());
        if shortcuts.is_empty() {
            return Ok(HashMap::new());
        }
        let list: Vec<(String, HashMap<&str, Value>)> = shortcuts
            .iter()
            .map(|shortcut| {
                let mut props: HashMap<&str, Value> = HashMap::new();
                props.insert("description", Value::from(shortcut.description.clone()));
                props.insert("preferred_trigger", Value::from(shortcut.trigger.clone()));
                (shortcut.id.clone(), props)
            })
            .collect();
        let results = self.request("BindShortcuts", |token| {
            let mut options: HashMap<&str, Value> = HashMap::new();
            options.insert("handle_token", Value::from(token));
            (session.clone(), list, "", options)
        })?;
        Ok(triggers(results.get("shortcuts")))
    }

    fn close(&mut self) {
        if let Some(session) = self.session.take() {
            if let Ok(proxy) = Proxy::new(
                &self.connection,
                DESTINATION,
                session,
                "org.freedesktop.portal.Session",
            ) {
                let _: Result<(), _> = proxy.call("Close", &());
            }
        }
    }
}

impl Drop for Portal {
    fn drop(&mut self) {
        self.close();
    }
}

/// `a(sa{sv})` → id → `trigger_description`.
pub fn triggers(value: Option<&OwnedValue>) -> HashMap<String, String> {
    let mut found = HashMap::new();
    let Some(Value::Array(array)) = value.map(|value| &**value) else {
        return found;
    };
    for item in array.iter() {
        let Value::Structure(structure) = item else {
            continue;
        };
        let fields = structure.fields();
        let (Some(Value::Str(id)), Some(Value::Dict(props))) = (fields.first(), fields.get(1))
        else {
            continue;
        };
        if let Ok(Some(description)) = props.get::<&str, String>(&"trigger_description") {
            found.insert(id.to_string(), description);
        }
    }
    found
}
