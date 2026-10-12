//! Input ports and owned links in a single graph session.

use crate::nodes::{Hardware, is_physical_input};
use crate::{Error, Node};
use pipewire::{self as pw, properties::properties};

#[derive(Clone)]
pub(crate) struct Port {
    pub(crate) id: u32,
    pub(crate) node_id: u32,
    pub(crate) direction: String,
    pub(crate) channel: String,
}

pub(crate) struct InputLink {
    pub(crate) source: String,
    pub(crate) output: u32,
    pub(crate) input: u32,
    pub(crate) output_right: u32,
    pub(crate) input_right: u32,
    pub(crate) _links: Vec<pw::link::Link>,
}

#[derive(Default)]
pub(crate) struct Graph {
    pub(crate) nodes: Vec<Node>,
    pub(crate) devices: Vec<Hardware>,
    pub(crate) ports: Vec<Port>,
    pub(crate) link: Option<InputLink>,
    pub(crate) inputs_changed: bool,
    pub(crate) outputs_changed: bool,
}

impl Graph {
    pub(crate) fn connect_input(
        &mut self,
        core: &pw::core::CoreRc,
        input_name: &str,
        filter_id: u32,
    ) -> Result<(), Error> {
        if self.link.is_some() {
            return Ok(());
        }
        // Refuse our virtual sources and monitor nodes: an accidental feedback loop is unsafe.
        let Some(source) = self
            .nodes
            .iter()
            .find(|n| n.name == input_name && is_physical_input(n))
        else {
            return Ok(());
        };
        let output = self
            .ports
            .iter()
            .filter(|p| p.node_id == source.id && p.direction == "out")
            .min_by_key(|p| match p.channel.as_str() {
                "MONO" => 0,
                "FL" => 1,
                _ => 2,
            });
        let input = self
            .ports
            .iter()
            .find(|p| p.node_id == filter_id && p.direction == "in" && p.channel == "FL");
        if let (Some(output), Some(input)) = (output, input) {
            let output_right = self
                .ports
                .iter()
                .find(|p| p.node_id == source.id && p.direction == "out" && p.channel == "FR")
                .unwrap_or(output);
            let Some(input_right) = self
                .ports
                .iter()
                .find(|p| p.node_id == filter_id && p.direction == "in" && p.channel == "FR")
            else {
                return Ok(());
            };
            let mut links = Vec::with_capacity(2);
            for (out, inp) in [(output, input), (output_right, input_right)] {
                links.push(core.create_object::<pw::link::Link>(
                    "link-factory",
                    &properties! {
                        "link.output.node" => source.id.to_string(),
                        "link.output.port" => out.id.to_string(),
                        "link.input.node" => filter_id.to_string(),
                        "link.input.port" => inp.id.to_string(),
                        "object.linger" => "false",
                        "link.passive" => "true",
                    },
                )?);
            }
            tracing::info!(source = %source.description, "Microphone connected");
            self.link = Some(InputLink {
                source: input_name.to_owned(),
                output: output.id,
                input: input.id,
                output_right: output_right.id,
                input_right: input_right.id,
                _links: links,
            });
        }
        Ok(())
    }
}
