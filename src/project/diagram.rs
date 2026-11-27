use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{Direction, Pipe, Project, Tag};

pub const COORDINATE_LIMIT: f32 = 1_000_000.0;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProcessDiagram {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub nodes: BTreeMap<String, DiagramNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<DiagramRoute>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub instruments: BTreeMap<String, Instrument>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiagramNode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagramRoute {
    pub pipe: PipeEnds,
    pub bend: [f32; 2],
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipeEnds {
    pub from: String,
    pub from_port: u8,
    pub to: String,
    pub to_port: u8,
}

impl From<&Pipe> for PipeEnds {
    fn from(pipe: &Pipe) -> Self {
        Self {
            from: pipe.from.clone(),
            from_port: pipe.from_port,
            to: pipe.to.clone(),
            to_port: pipe.to_port,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Instrument {
    pub function: InstrumentFunction,
    pub number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tag: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentFunction {
    Indication,
    Transmission,
    Control,
    Alarm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Quantity {
    Flow,
    Level,
    Pressure,
    Temperature,
}

impl Quantity {
    pub fn for_tag(id: &str) -> Option<Self> {
        let (_, role) = id.split_once('.')?;
        match role {
            "flow_l_s" | "inflow_l_s" | "outflow_l_s" | "outlet_1_l_s" | "outlet_2_l_s" => {
                Some(Self::Flow)
            }
            "level_pct" => Some(Self::Level),
            "pressure_kpa" => Some(Self::Pressure),
            "temperature_c" | "liquid_temp_c" | "outlet_2_temperature_c" => Some(Self::Temperature),
            _ => None,
        }
    }
}

impl ProcessDiagram {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.routes.is_empty() && self.instruments.is_empty()
    }

    #[cfg(any(feature = "studio", feature = "web", test))]
    pub(super) fn retain_routes(&mut self, pipes: &[Pipe]) {
        let connected: BTreeSet<_> = pipes.iter().map(PipeEnds::from).collect();
        self.routes.retain(|route| connected.contains(&route.pipe));
    }

    #[cfg(any(feature = "studio", feature = "web", test))]
    pub(super) fn remove_part(&mut self, id: &str, pipes: &[Pipe]) {
        self.nodes.remove(id);
        self.retain_routes(pipes);
        let prefix = format!("{id}.");
        self.instruments.retain(|tag, instrument| {
            if instrument
                .output_tag
                .as_ref()
                .is_some_and(|output| output.starts_with(&prefix))
            {
                instrument.output_tag = None;
            }
            !tag.starts_with(&prefix)
        });
    }

    pub(super) fn validate(&self, project: &Project, tags: &BTreeMap<String, Tag>) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let water_parts: BTreeSet<_> = project
            .parts
            .iter()
            .filter(|part| part.kind.has_water_inlet() || part.kind.has_water_outlet())
            .map(|part| part.id.as_str())
            .collect();
        for (id, node) in &self.nodes {
            ensure!(
                water_parts.contains(id.as_str()),
                "unknown diagram node {id}"
            );
            ensure!(
                node.position.is_none_or(valid_position)
                    && node
                        .label
                        .as_ref()
                        .is_none_or(|label| !label.trim().is_empty()),
                "invalid diagram node {id}"
            );
        }
        let pipes: BTreeSet<_> = project.pipes.iter().map(PipeEnds::from).collect();
        let mut routed = BTreeSet::new();
        for route in &self.routes {
            ensure!(
                pipes.contains(&route.pipe)
                    && routed.insert(&route.pipe)
                    && valid_position(route.bend),
                "invalid diagram route {} → {}",
                route.pipe.from,
                route.pipe.to
            );
        }
        for (id, instrument) in &self.instruments {
            let is_water_tag = |id: &str, direction| {
                id.split_once('.')
                    .is_some_and(|(part, _)| water_parts.contains(part))
                    && tags.get(id).is_some_and(|tag| tag.direction == direction)
            };
            ensure!(
                Quantity::for_tag(id).is_some()
                    && is_water_tag(id, Direction::Sensor)
                    && instrument.number > 0,
                "invalid diagram instrument {id}"
            );
            if let Some(output) = &instrument.output_tag {
                ensure!(
                    instrument.function == InstrumentFunction::Control
                        && is_water_tag(output, Direction::Actuator),
                    "invalid diagram control output {id} → {output}"
                );
            }
        }
        Ok(())
    }
}

fn valid_position(position: [f32; 2]) -> bool {
    position
        .into_iter()
        .all(|value| (-COORDINATE_LIMIT..=COORDINATE_LIMIT).contains(&value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagram_round_trips_and_cleans_up_removed_references() -> Result<()> {
        let mut project: Project =
            serde_json::from_str(include_str!("../../examples/heated-tank.json"))?;
        let saved_positions = project
            .parts
            .iter()
            .map(|part| part.position)
            .collect::<Vec<_>>();
        let pipe = PipeEnds::from(&project.pipes[0]);
        project.process_diagram.nodes.insert(
            "tank".into(),
            DiagramNode {
                position: Some([20.0, 30.0]),
                label: Some("Бак".into()),
            },
        );
        project.process_diagram.routes.push(DiagramRoute {
            pipe,
            bend: [40.0, 50.0],
        });
        project.validate()?;

        let mut reopened: Project = serde_json::from_str(&serde_json::to_string(&project)?)?;
        assert_eq!(reopened.process_diagram, project.process_diagram);
        assert_eq!(
            reopened
                .parts
                .iter()
                .map(|part| part.position)
                .collect::<Vec<_>>(),
            saved_positions
        );

        let mut pipes = reopened.pipes.clone();
        pipes.remove(0);
        reopened.replace_pipes(pipes);
        assert!(reopened.process_diagram.routes.is_empty());
        assert!(reopened.remove_part("valve"));
        assert_eq!(
            reopened.process_diagram.instruments["tank.level_pct"].output_tag,
            None
        );
        assert!(reopened.remove_part("tank"));
        assert!(reopened.process_diagram.is_empty());
        reopened.validate()?;
        assert!(
            serde_json::to_value(reopened)?
                .get("process_diagram")
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn diagram_rejects_a_sensor_as_the_control_output() -> Result<()> {
        let mut project: Project =
            serde_json::from_str(include_str!("../../examples/heated-tank.json"))?;
        project
            .process_diagram
            .instruments
            .get_mut("tank.level_pct")
            .unwrap()
            .output_tag = Some("valve.flow_l_s".into());
        assert!(
            project
                .validate()
                .unwrap_err()
                .to_string()
                .contains("invalid diagram control output")
        );
        Ok(())
    }
}
