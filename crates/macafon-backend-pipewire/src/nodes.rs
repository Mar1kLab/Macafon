//! Physical device discovery and stable hardware identity.

use crate::{Connection, Error};
use pipewire::{self as pw, types::ObjectType};
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Clone)]
pub struct Node {
    pub id: u32,
    pub serial: String,
    pub name: String,
    pub description: String,
    pub media_class: String,
    pub virtual_node: bool,
    pub device_id: Option<u32>,
    pub stable_id: String,
    pub kind: &'static str,
    pub form_factor: String,
}

pub(crate) fn node(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
) -> Option<Node> {
    if global.type_ != ObjectType::Node {
        return None;
    }
    let props = global.props?;
    Some(Node {
        id: global.id,
        serial: props.get("object.serial")?.to_owned(),
        name: props.get("node.name")?.to_owned(),
        description: props
            .get("node.description")
            .or_else(|| props.get("application.name"))
            .or_else(|| props.get("node.name"))?
            .to_owned(),
        media_class: props.get("media.class").unwrap_or("").to_owned(),
        virtual_node: props.get("node.virtual") == Some("true"),
        device_id: props.get("device.id").and_then(|id| id.parse().ok()),
        stable_id: stable_device_id(
            props.get("device.serial"),
            props
                .get("device.bus-path")
                .or_else(|| props.get("device.bus-id")),
            props.get("node.name")?,
        ),
        form_factor: props.get("device.form-factor").unwrap_or("").to_owned(),
        kind: device_kind(
            props.get("device.bus").unwrap_or(""),
            props.get("device.form-factor").unwrap_or(""),
            props.get("node.name")?,
        ),
    })
}

fn stable_device_id(serial: Option<&str>, bus_path: Option<&str>, name: &str) -> String {
    format!(
        "{}:{name}",
        serial
            .filter(|s| !s.is_empty())
            .or(bus_path)
            .unwrap_or(name)
    )
}

fn device_kind(bus: &str, form_factor: &str, name: &str) -> &'static str {
    if bus == "bluetooth" || name.starts_with("bluez_") {
        "bluetooth"
    } else if matches!(form_factor, "speaker" | "speakers") {
        "speakers"
    } else if matches!(form_factor, "headset" | "headphone") {
        "headset"
    } else if bus == "usb" || name.contains(".usb-") {
        "usb"
    } else if matches!(bus, "pci" | "platform") || form_factor == "internal" {
        "builtin"
    } else {
        "unknown"
    }
}

#[derive(Clone)]
pub(crate) struct Hardware {
    pub(crate) id: u32,
    serial: Option<String>,
    bus_path: Option<String>,
    bus: String,
    form_factor: String,
}

pub(crate) fn hardware(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
) -> Option<Hardware> {
    if global.type_ != ObjectType::Device {
        return None;
    }
    let props = global.props?;
    Some(Hardware {
        id: global.id,
        serial: props.get("device.serial").map(str::to_owned),
        bus_path: props
            .get("device.bus-path")
            .or_else(|| props.get("device.bus-id"))
            .map(str::to_owned),
        bus: props.get("device.bus").unwrap_or("").to_owned(),
        form_factor: props.get("device.form-factor").unwrap_or("").to_owned(),
    })
}

pub(crate) fn enrich_node(node: &mut Node, devices: &[Hardware]) {
    if let Some(device) = devices
        .iter()
        .find(|device| Some(device.id) == node.device_id)
    {
        if device.serial.is_some() || device.bus_path.is_some() {
            node.stable_id = stable_device_id(
                device.serial.as_deref(),
                device.bus_path.as_deref(),
                &node.name,
            );
        }
        node.kind = device_kind(&device.bus, &device.form_factor, &node.name);
        if !device.form_factor.is_empty() {
            node.form_factor = device.form_factor.clone();
        }
    }
}

pub fn list_nodes() -> Result<Vec<Node>, Error> {
    let connection = Connection::new()?;
    let registry = connection.core.get_registry()?;
    let nodes = Rc::new(RefCell::new(Vec::new()));
    let collected = nodes.clone();
    let devices = Rc::new(RefCell::new(Vec::new()));
    let hardware_devices = devices.clone();
    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            if let Some(device) = hardware(global) {
                hardware_devices.borrow_mut().push(device);
            }
            if let Some(node) = node(global) {
                collected.borrow_mut().push(node);
            }
        })
        .register();
    connection.roundtrip()?;
    let mut result = nodes.borrow().clone();
    for node in &mut result {
        enrich_node(node, &devices.borrow());
    }
    result.sort_by_key(|node| node.id);
    Ok(result)
}

pub(crate) fn is_physical_input(node: &Node) -> bool {
    node.media_class == "Audio/Source"
        && !node.virtual_node
        && !node.name.starts_with("macafon_")
        && !node.name.ends_with(".monitor")
}

#[cfg(test)]
mod input_identity_tests {
    use super::*;
    #[test]
    fn device_identity_and_kind_use_hardware_properties() {
        assert_eq!(
            stable_device_id(Some("serial123"), Some("usb-1"), "mic"),
            "serial123:mic"
        );
        assert_eq!(stable_device_id(None, Some("usb-1"), "mic"), "usb-1:mic");
        assert_eq!(device_kind("usb", "headset", "mic"), "headset");
        assert_eq!(device_kind("bluetooth", "headset", "mic"), "bluetooth");
        assert_eq!(device_kind("pci", "", "mic"), "builtin");
    }
}
