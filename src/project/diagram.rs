use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Deserializer, Serialize, de};

use super::{Direction, Pipe, Project, Tag};

pub const COORDINATE_LIMIT: f32 = 1_000_000.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ProcessDiagram {
    #[serde(skip_serializing_if = "is_default")]
    pub kind: DiagramKind,
    #[serde(skip_serializing_if = "is_default")]
    pub notation: InstrumentNotation,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub nodes: BTreeMap<String, DiagramNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<DiagramRoute>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub instruments: BTreeMap<String, Instrument>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub control_links: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct SavedDiagram {
    kind: DiagramKind,
    notation: InstrumentNotation,
    nodes: BTreeMap<String, DiagramNode>,
    routes: Vec<DiagramRoute>,
    instruments: BTreeMap<String, SavedInstrument>,
    control_links: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedInstrument {
    function: InstrumentFunction,
    number: u32,
    #[serde(default)]
    location: InstrumentLocation,
    #[serde(default)]
    output_tag: Option<String>,
}

impl<'de> Deserialize<'de> for ProcessDiagram {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let saved = SavedDiagram::deserialize(deserializer)?;
        let mut control_links = saved.control_links;
        let mut instruments = BTreeMap::new();
        for (id, instrument) in saved.instruments {
            if let Some(output) = instrument.output_tag {
                if instrument.function != InstrumentFunction::Control
                    || control_links.get(&id).is_some_and(|tag| tag != &output)
                {
                    return Err(de::Error::custom(format!(
                        "conflicting legacy control link {id}"
                    )));
                }
                control_links.insert(id.clone(), output);
            }
            instruments.insert(
                id,
                Instrument {
                    function: instrument.function,
                    number: instrument.number,
                    location: instrument.location,
                },
            );
        }
        Ok(Self {
            kind: saved.kind,
            notation: saved.notation,
            nodes: saved.nodes,
            routes: saved.routes,
            instruments,
            control_links,
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagramVariant {
    pub name: String,
    pub sheet: ProcessDiagram,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagramKind {
    #[default]
    Functional,
    IsoBlock,
    IsoPfd,
    IsoPid,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentNotation {
    #[default]
    Gost,
    Isa,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentLocation {
    #[default]
    Field,
    Panel,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiagramNode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagramRoute {
    pub pipe: PipeEnds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bend: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub data: LineDetails,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LineDetails {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reference: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub medium: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub design_flow_l_s: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub design_temperature_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nominal_size_mm: Option<u32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub piping_class: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub material: String,
    #[serde(skip_serializing_if = "is_default")]
    pub secondary: bool,
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Instrument {
    pub function: InstrumentFunction,
    pub number: u32,
    #[serde(skip_serializing_if = "is_default")]
    pub location: InstrumentLocation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentFunction {
    Indication,
    Transmission,
    Control,
    IndicatingControl,
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
        is_default(&self.kind)
            && is_default(&self.notation)
            && self.nodes.is_empty()
            && self.routes.is_empty()
            && self.instruments.is_empty()
            && self.control_links.is_empty()
    }

    #[cfg(any(feature = "studio", feature = "web", test))]
    pub fn changes_from(&self, source: &Self) -> Vec<LinkChange> {
        self.control_links
            .keys()
            .chain(source.control_links.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|input| {
                let before = self.control_links.get(input);
                let after = source.control_links.get(input);
                (before != after).then(|| LinkChange {
                    input: input.clone(),
                    before: before.cloned(),
                    after: after.cloned(),
                })
            })
            .collect()
    }

    #[cfg(any(feature = "studio", feature = "web", test))]
    pub fn apply_links(&mut self, changes: impl IntoIterator<Item = LinkChange>) {
        for change in changes {
            if self.control_links.get(&change.input) != change.before.as_ref() {
                continue;
            }
            if let Some(output) = change.after {
                self.control_links.insert(change.input, output);
            } else {
                self.control_links.remove(&change.input);
            }
        }
    }

    #[cfg(any(feature = "studio", feature = "web"))]
    pub fn update_route(&mut self, pipe: &PipeEnds, change: impl FnOnce(&mut DiagramRoute)) {
        if let Some(index) = self.routes.iter().position(|route| &route.pipe == pipe) {
            change(&mut self.routes[index]);
            if self.routes[index].bend.is_none() && is_default(&self.routes[index].data) {
                self.routes.remove(index);
            }
        } else {
            let mut route = DiagramRoute {
                pipe: pipe.clone(),
                bend: None,
                data: LineDetails::default(),
            };
            change(&mut route);
            if route.bend.is_some() || !is_default(&route.data) {
                self.routes.push(route);
            }
        }
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
        self.instruments.retain(|tag, _| !tag.starts_with(&prefix));
        self.control_links
            .retain(|input, output| !input.starts_with(&prefix) && !output.starts_with(&prefix));
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
                        .is_none_or(|label| !label.trim().is_empty())
                    && node
                        .reference
                        .as_ref()
                        .is_none_or(|reference| !reference.trim().is_empty()),
                "invalid diagram node {id}"
            );
        }
        let pipes: BTreeSet<_> = project.pipes.iter().map(PipeEnds::from).collect();
        let mut routed = BTreeSet::new();
        for route in &self.routes {
            ensure!(
                pipes.contains(&route.pipe)
                    && routed.insert(&route.pipe)
                    && route.bend.is_none_or(valid_position)
                    && route
                        .data
                        .design_flow_l_s
                        .is_none_or(|flow| flow.is_finite() && flow >= 0.0)
                    && route.data.design_temperature_c.is_none_or(f32::is_finite)
                    && route.data.nominal_size_mm != Some(0),
                "invalid diagram route {} → {}",
                route.pipe.from,
                route.pipe.to
            );
        }
        let is_water_tag = |id: &str, direction| {
            id.split_once('.')
                .is_some_and(|(part, _)| water_parts.contains(part))
                && tags.get(id).is_some_and(|tag| tag.direction == direction)
        };
        let is_measurement =
            |id| Quantity::for_tag(id).is_some() && is_water_tag(id, Direction::Sensor);
        for (id, instrument) in &self.instruments {
            ensure!(
                is_measurement(id.as_str()) && instrument.number > 0,
                "invalid diagram instrument {id}"
            );
        }
        for (id, output) in &self.control_links {
            ensure!(
                is_measurement(id.as_str()) && is_water_tag(output, Direction::Actuator),
                "invalid diagram control link {id} → {output}"
            );
        }
        Ok(())
    }
}

#[cfg(any(feature = "studio", feature = "web", test))]
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LinkChange {
    pub input: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
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
                reference: None,
            },
        );
        project.process_diagram.routes.push(DiagramRoute {
            pipe,
            bend: Some([40.0, 50.0]),
            data: LineDetails::default(),
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
        assert!(
            !reopened
                .process_diagram
                .control_links
                .contains_key("tank.level_pct")
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
            .control_links
            .insert("tank.level_pct".into(), "valve.flow_l_s".into());
        assert!(
            project
                .validate()
                .unwrap_err()
                .to_string()
                .contains("invalid diagram control link")
        );
        Ok(())
    }

    #[test]
    fn reads_legacy_loop_outputs_as_independent_links() -> Result<()> {
        let legacy = r#"{"instruments":{"tank.level_pct":{"function":"control","number":1,"output_tag":"valve.opening_pct"}}}"#;
        let sheet: ProcessDiagram = serde_json::from_str(legacy)?;
        assert_eq!(sheet.control_links["tank.level_pct"], "valve.opening_pct");
        assert_eq!(sheet.instruments["tank.level_pct"].number, 1);
        let saved = serde_json::to_string(&sheet)?;
        assert!(!saved.contains("output_tag"));
        assert_eq!(serde_json::from_str::<ProcessDiagram>(&saved)?, sheet);
        Ok(())
    }

    #[test]
    fn variants_sync_only_checked_control_links() -> Result<()> {
        let mut project: Project =
            serde_json::from_str(include_str!("../../examples/heated-tank.json"))?;
        let mut source = project.process_diagram.clone();
        source
            .control_links
            .insert("tank.temperature_c".into(), "tank.heater_pct".into());
        let mut destination = source.clone();
        destination.kind = DiagramKind::IsoPid;
        destination.notation = InstrumentNotation::Isa;
        destination
            .instruments
            .get_mut("tank.level_pct")
            .unwrap()
            .number = 101;
        destination
            .instruments
            .get_mut("tank.level_pct")
            .unwrap()
            .function = InstrumentFunction::Indication;
        destination.nodes.insert(
            "tank".into(),
            DiagramNode {
                position: Some([4.0, 12.0]),
                label: Some("Reservoir".into()),
                reference: Some("V-101".into()),
            },
        );
        destination.routes.push(DiagramRoute {
            pipe: PipeEnds::from(&project.pipes[0]),
            bend: Some([3.0, 8.0]),
            data: LineDetails {
                reference: "W-01".into(),
                nominal_size_mm: Some(50),
                ..Default::default()
            },
        });
        destination
            .control_links
            .insert("tank.level_pct".into(), "pump.target_l_s".into());
        destination.control_links.remove("tank.temperature_c");
        destination
            .control_links
            .insert("valve.flow_l_s".into(), "valve.opening_pct".into());
        let initial = destination.clone();

        let changes = destination.changes_from(&source);
        assert_eq!(changes.len(), 3);
        destination.apply_links(changes.into_iter().filter(|change| change.before.is_none()));
        assert_eq!(
            destination.control_links["tank.temperature_c"],
            "tank.heater_pct"
        );
        assert_eq!(
            destination.control_links["tank.level_pct"],
            "pump.target_l_s"
        );
        assert!(destination.control_links.contains_key("valve.flow_l_s"));

        let approved = destination.changes_from(&source);
        destination.apply_links(approved);
        assert_eq!(destination.control_links, source.control_links);
        assert_eq!(destination.instruments, initial.instruments);
        assert_eq!(destination.nodes, initial.nodes);
        assert_eq!(destination.routes, initial.routes);
        project.process_diagram = source;
        project.diagram_variants.insert(
            "pid".into(),
            DiagramVariant {
                name: "P&ID".into(),
                sheet: destination.clone(),
            },
        );
        project.validate()?;
        let mut reopened: Project = serde_json::from_str(&serde_json::to_string(&project)?)?;
        assert_eq!(reopened.diagram_variants["pid"].sheet, destination);
        reopened.remove_part("valve");
        assert!(
            !reopened.diagram_variants["pid"]
                .sheet
                .control_links
                .contains_key("tank.level_pct")
        );
        assert!(
            !reopened
                .process_diagram
                .control_links
                .contains_key("tank.level_pct")
        );
        reopened.validate()?;
        Ok(())
    }
}
