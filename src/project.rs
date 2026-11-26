use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::SocketAddr,
    path::Path,
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::product::{BOXES, Product, ProductSpec};

use crate::production::{self, Emission, Equipment, Machine, Production, Robot, Staff};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub production: Production,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pipes: Vec<Pipe>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub products: BTreeMap<String, Product>,
    #[serde(default)]
    pub tag_names: BTreeMap<String, String>,
    #[serde(default)]
    pub initial_commands: BTreeMap<String, Value>,
    #[serde(default)]
    pub drivers: Vec<Driver>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pipe {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "first_port")]
    pub from_port: u8,
    #[serde(default, skip_serializing_if = "first_port")]
    pub to_port: u8,
    #[serde(default = "default_pipe_resistance")]
    pub resistance_per_m: f32,
}

impl Pipe {
    #[cfg(any(feature = "studio", test))]
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            from_port: 0,
            to_port: 0,
            resistance_per_m: default_pipe_resistance(),
        }
    }
}

fn first_port(port: &u8) -> bool {
    *port == 0
}

const fn default_pipe_resistance() -> f32 {
    3.0
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Part {
    pub id: String,
    pub name: String,
    pub position: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(flatten)]
    pub kind: PartKind,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PartKind {
    Conveyor {
        size: [f32; 3],
        speed: f32,
    },
    Pusher {
        stroke: f32,
        speed: f32,
    },
    Sensor {
        range: f32,
    },
    Emitter {
        interval: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emission: Option<Emission>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        random_spawn_belt: Option<String>,
        size: [f32; 3],
        mass: f32,
    },
    Remover {
        size: [f32; 3],
    },
    Crate {
        size: [f32; 3],
        mass: f32,
    },
    Barrier {
        size: [f32; 3],
    },
    FlowGuide {
        length: f32,
        height: f32,
        inlet: f32,
        outlet: f32,
    },
    Equipment {
        equipment: Equipment,
        size: [f32; 3],
        #[serde(default = "default_scale_capacity")]
        scale_capacity: u32,
    },
    Machine {
        machine: Machine,
    },
    Robot {
        robot: Robot,
    },
    Staff {
        staff: Staff,
    },
    HeatedTank(HeatedTank),
    WaterSource {
        size: [f32; 3],
        temperature_c: f32,
        max_flow_l_s: f32,
        #[serde(default)]
        dirt_mg_l: f32,
    },
    WaterPump {
        size: [f32; 3],
        max_flow_l_s: f32,
        #[serde(default = "default_pump_head")]
        shutoff_head_m: f32,
        #[serde(default = "default_pump_dirt_limit")]
        dirt_limit_mg: f32,
    },
    WaterValve {
        size: [f32; 3],
        max_flow_l_s: f32,
        #[serde(default = "default_open_valve_resistance")]
        open_resistance: f32,
    },
    WaterDrain {
        size: [f32; 3],
        #[serde(default = "default_drain_flow")]
        max_flow_l_s: f32,
    },
    WaterJunction {
        size: [f32; 3],
        mixing: bool,
    },
    WaterInstrument {
        size: [f32; 3],
        measurement: WaterMeasurement,
    },
    WaterFilter {
        size: [f32; 3],
        clean_resistance: f32,
        dirty_resistance: f32,
        capacity_mg: f32,
        efficiency_pct: f32,
    },
    WaterHeater {
        size: [f32; 3],
        max_power_w: f32,
    },
    WaterHeatExchanger {
        size: [f32; 3],
        conductance_w_k: f32,
        resistance: f32,
    },
    WaterFiller {
        size: [f32; 3],
        max_flow_l_s: f32,
        dose_l: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaterMeasurement {
    Flow,
    Temperature,
    Pressure,
}

pub const fn default_pump_dirt_limit() -> f32 {
    250.0
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeatedTank {
    pub size: [f32; 3],
    pub volume_l: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_volume_l: Option<f32>,
    pub initial_temperature_c: f32,
    pub ambient_temperature_c: f32,
    pub heat_loss_w_per_k: f32,
    pub heater_max_w: f32,
    #[serde(default)]
    pub networked: bool,
    #[serde(default)]
    pub feed_temperature_c: f32,
    #[serde(default)]
    pub feed_max_l_s: f32,
    #[serde(default)]
    pub initial_dirt_mg_l: f32,
}

#[cfg(any(feature = "studio", feature = "web"))]
impl Default for HeatedTank {
    fn default() -> Self {
        Self {
            size: [0.6, 1.2, 0.6],
            volume_l: 40.0,
            initial_volume_l: Some(20.0),
            initial_temperature_c: 20.0,
            ambient_temperature_c: 20.0,
            heat_loss_w_per_k: 8.0,
            heater_max_w: 2000.0,
            networked: true,
            feed_temperature_c: 0.0,
            feed_max_l_s: 0.0,
            initial_dirt_mg_l: 0.0,
        }
    }
}

impl HeatedTank {
    pub fn starting_volume_l(&self) -> f32 {
        if self.networked {
            self.initial_volume_l.unwrap_or(self.volume_l)
        } else {
            self.volume_l
        }
    }
}

pub const fn default_pump_head() -> f32 {
    4.0
}

pub const fn default_open_valve_resistance() -> f32 {
    16.0
}

pub const fn default_drain_flow() -> f32 {
    0.03
}

impl Part {
    pub fn water_port(&self, outlet: bool) -> Option<([f64; 3], [f64; 3])> {
        self.water_port_at(outlet, 0)
    }

    pub fn water_port_at(&self, outlet: bool, port: u8) -> Option<([f64; 3], [f64; 3])> {
        if port >= self.kind.water_port_count(outlet) {
            return None;
        }
        let side = if outlet { 1.0 } else { -1.0 };
        let mut z = 0.0;
        let (x, y, direction_x, direction_y) = match self.kind {
            PartKind::WaterSource {
                size: [x, y, _], ..
            } if outlet => (0.45 * x, -0.25 * y, 1.0, 0.0),
            PartKind::WaterPump { size: [x, ..], .. } if outlet => (0.43 * x, 0.0, 1.0, 0.0),
            PartKind::WaterPump { size: [x, ..], .. } => (-0.44 * x, 0.0, -1.0, 0.0),
            PartKind::WaterValve {
                size: [x, y, _], ..
            } if outlet => (0.49 * x, -0.08 * y, 1.0, 0.0),
            PartKind::WaterValve {
                size: [x, y, _], ..
            } => (-0.49 * x, -0.08 * y, -1.0, 0.0),
            PartKind::HeatedTank(tank) if tank.networked && outlet => {
                (0.5 * tank.size[0], -0.31 * tank.size[1], 1.0, 0.0)
            }
            PartKind::HeatedTank(tank) if tank.networked => {
                (-0.5 * tank.size[0], 0.16 * tank.size[1], -1.0, 0.0)
            }
            PartKind::WaterDrain {
                size: [_, y, _], ..
            } if !outlet => (0.0, 0.5 * y, 0.0, 1.0),
            PartKind::WaterJunction {
                size: [x, _, width],
                ..
            } => {
                if self.kind.water_port_count(outlet) == 2 {
                    z = if port == 0 {
                        -0.32 * width
                    } else {
                        0.32 * width
                    };
                }
                (side * x * 0.5, 0.0, f64::from(side), 0.0)
            }
            PartKind::WaterHeatExchanger {
                size: [x, _, width],
                ..
            } => {
                z = if port == 0 {
                    -0.30 * width
                } else {
                    0.30 * width
                };
                (side * x * 0.5, 0.0, f64::from(side), 0.0)
            }
            PartKind::WaterInstrument { size: [x, ..], .. }
            | PartKind::WaterFilter { size: [x, ..], .. }
            | PartKind::WaterHeater { size: [x, ..], .. } => {
                (side * x * 0.5, 0.0, f64::from(side), 0.0)
            }
            PartKind::WaterFiller {
                size: [x, y, _], ..
            } => (-0.5 * x, 0.25 * y, -1.0, 0.0),
            _ => return None,
        };
        let (sin, cos) = f64::from(self.yaw).to_radians().sin_cos();
        let [px, py, pz] = self.position.map(f64::from);
        Some((
            [
                px + cos * f64::from(x) + sin * f64::from(z),
                py + f64::from(y),
                pz - sin * f64::from(x) + cos * f64::from(z),
            ],
            [cos * direction_x, direction_y, -sin * direction_x],
        ))
    }
}

pub fn water_pipe_points(
    from: &Part,
    from_port: u8,
    to: &Part,
    to_port: u8,
) -> Option<[[f64; 3]; 4]> {
    let (outlet, out_dir) = from.water_port_at(true, from_port)?;
    let (inlet, in_dir) = to.water_port_at(false, to_port)?;
    let reach = (water_distance(outlet, inlet) * 0.25).min(0.16);
    let extend = |point: [f64; 3], dir: [f64; 3]| {
        std::array::from_fn(|index| point[index] + dir[index] * reach)
    };
    Some([
        outlet,
        extend(outlet, out_dir),
        extend(inlet, in_dir),
        inlet,
    ])
}

pub fn water_distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}

impl PartKind {
    pub fn water_port_count(&self, outlet: bool) -> u8 {
        match self {
            Self::WaterSource { .. } => u8::from(outlet),
            Self::WaterDrain { .. } | Self::WaterFiller { .. } => u8::from(!outlet),
            Self::WaterJunction { mixing, .. } => {
                if *mixing == outlet {
                    1
                } else {
                    2
                }
            }
            Self::WaterHeatExchanger { .. } => 2,
            Self::HeatedTank(tank) => u8::from(tank.networked),
            Self::WaterPump { .. }
            | Self::WaterValve { .. }
            | Self::WaterInstrument { .. }
            | Self::WaterFilter { .. }
            | Self::WaterHeater { .. } => 1,
            _ => 0,
        }
    }

    pub fn has_water_inlet(&self) -> bool {
        self.water_port_count(false) > 0
    }
    pub fn has_water_outlet(&self) -> bool {
        self.water_port_count(true) > 0
    }

    pub fn water_size(&self) -> Option<[f32; 3]> {
        match *self {
            Self::WaterSource { size, .. }
            | Self::WaterPump { size, .. }
            | Self::WaterValve { size, .. }
            | Self::WaterDrain { size, .. }
            | Self::WaterJunction { size, .. }
            | Self::WaterInstrument { size, .. }
            | Self::WaterFilter { size, .. }
            | Self::WaterHeater { size, .. }
            | Self::WaterHeatExchanger { size, .. }
            | Self::WaterFiller { size, .. } => Some(size),
            Self::HeatedTank(tank) => Some(tank.size),
            _ => None,
        }
    }

    pub fn feed_surface(&self) -> Option<[f32; 3]> {
        match *self {
            Self::Conveyor { size, .. }
            | Self::Equipment {
                equipment: Equipment::Roller | Equipment::Scale,
                size,
                ..
            } => Some(size),
            _ => None,
        }
    }
}

fn default_scale_capacity() -> u32 {
    20
}

#[cfg(any(feature = "studio", feature = "web"))]
impl PartKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Conveyor { .. } => "Conveyor",
            Self::Pusher { .. } => "Pusher",
            Self::Sensor { .. } => "Photoelectric sensor",
            Self::Emitter { .. } => "Emitter",
            Self::Remover { .. } => "Remover",
            Self::Crate { .. } => "Crate",
            Self::Barrier { .. } => "Guard",
            Self::FlowGuide { .. } => "Passive flow guide",
            Self::Equipment { equipment, .. } => equipment.label(),
            Self::Machine { machine } => {
                match machine.assembly.as_ref().map(|assembly| assembly.mode) {
                    Some(production::AssemblyMode::Manual) => "Manual assembly table",
                    Some(production::AssemblyMode::Automatic) => "Automatic assembly table",
                    None => "Machining center",
                }
            }
            Self::Robot { robot } if robot.two_axis => "Two-axis Pick & Place",
            Self::Robot { .. } => "XYZ Pick & Place",
            Self::Staff { staff } if staff.role == production::StaffRole::Technician => {
                "Technician"
            }
            Self::Staff { .. } => "Operator",
            Self::HeatedTank(_) => "Heated tank",
            Self::WaterSource { .. } => "Water source",
            Self::WaterPump { .. } => "Water pump",
            Self::WaterValve { .. } => "Control valve",
            Self::WaterDrain { .. } => "Water drain",
            Self::WaterJunction { mixing: false, .. } => "Water splitter",
            Self::WaterJunction { mixing: true, .. } => "Water mixer",
            Self::WaterInstrument { measurement, .. } => match measurement {
                WaterMeasurement::Flow => "Water flow meter",
                WaterMeasurement::Temperature => "Water thermometer",
                WaterMeasurement::Pressure => "Water pressure gauge",
            },
            Self::WaterFilter { .. } => "Water filter",
            Self::WaterHeater { .. } => "Inline water heater",
            Self::WaterHeatExchanger { .. } => "Water heat exchanger",
            Self::WaterFiller { .. } => "Liquid filler",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Float(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Sensor,
    Actuator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Bool,
    Float,
    Int,
}

#[derive(Clone, Debug, Serialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub direction: Direction,
    pub value_type: ValueType,
    pub default: Value,
    pub range: Option<[f32; 2]>,
}

impl Tag {
    pub fn check(&self, value: Value) -> Result<Value> {
        match (self.value_type, value) {
            (ValueType::Bool, Value::Bool(_)) | (ValueType::Int, Value::Int(_)) => Ok(value),
            (ValueType::Float, Value::Float(value)) if value.is_finite() => Ok(Value::Float(
                self.range.map_or(value, |[lo, hi]| value.clamp(lo, hi)),
            )),
            (ValueType::Float, Value::Int(value)) => self.check(Value::Float(value as f32)),
            _ => bail!("tag {} expects {:?}", self.id, self.value_type),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Driver {
    pub id: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default = "poll_ms")]
    pub poll_ms: u64,
    #[serde(default)]
    pub on_disconnect: Disconnect,
    #[serde(flatten)]
    pub protocol: Protocol,
}

fn enabled() -> bool {
    true
}
fn poll_ms() -> u64 {
    33
}
fn unit_id() -> u8 {
    1
}
fn scale() -> f32 {
    100.0
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Disconnect {
    #[default]
    Hold,
    Reset,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "protocol", rename_all = "snake_case", deny_unknown_fields)]
pub enum Protocol {
    ModbusClient {
        address: SocketAddr,
        #[serde(default = "unit_id")]
        unit_id: u8,
        bindings: Vec<ModbusBinding>,
    },
    ModbusServer {
        address: SocketAddr,
        #[serde(default = "unit_id")]
        unit_id: u8,
        bindings: Vec<ModbusBinding>,
    },
    OpcUaClient {
        endpoint: String,
        #[serde(default)]
        security: OpcSecurity,
        bindings: Vec<OpcBinding>,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpcSecurity {
    #[default]
    None,
    SignAndEncrypt {
        pki_dir: std::path::PathBuf,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpcBinding {
    pub tag: String,
    pub node: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Coil,
    DiscreteInput,
    HoldingRegister,
    InputRegister,
}

impl Area {
    pub fn bit(self) -> bool {
        matches!(self, Self::Coil | Self::DiscreteInput)
    }

    pub fn writable(self) -> bool {
        matches!(self, Self::Coil | Self::HoldingRegister)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModbusBinding {
    pub tag: String,
    pub area: Area,
    pub address: u16,
    #[serde(default = "scale")]
    pub scale: f32,
}

impl Driver {
    pub fn mapped_tags(&self) -> Vec<&str> {
        match &self.protocol {
            Protocol::ModbusClient { bindings, .. } | Protocol::ModbusServer { bindings, .. } => {
                bindings
                    .iter()
                    .map(|binding| binding.tag.as_str())
                    .collect()
            }
            Protocol::OpcUaClient { bindings, .. } => bindings
                .iter()
                .map(|binding| binding.tag.as_str())
                .collect(),
        }
    }
}

impl Project {
    pub fn product(&self, id: &str) -> Option<ProductSpec<'_>> {
        self.products.get(id).map(Product::spec).or_else(|| {
            BOXES
                .iter()
                .find(|(key, _)| *key == id)
                .map(|(_, spec)| *spec)
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let project: Self =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        project.validate()?;
        Ok(project)
    }

    #[cfg(feature = "studio")]
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        use std::io::Write;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("create temporary file in {}", parent.display()))?;
        serde_json::to_writer_pretty(&mut temporary, self)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(path)
            .with_context(|| format!("save {}", path.display()))?;
        Ok(())
    }

    #[cfg(any(feature = "studio", test))]
    pub fn demo() -> Self {
        let project: Self = serde_json::from_str(include_str!("../examples/sorting.json"))
            .expect("bundled project parses");
        project
    }

    #[cfg(any(feature = "studio", test))]
    pub fn remove_part(&mut self, id: &str) -> bool {
        let Some(index) = self.parts.iter().position(|part| part.id == id) else {
            return false;
        };
        self.parts.remove(index);
        self.pipes.retain(|pipe| pipe.from != id && pipe.to != id);
        for part in &mut self.parts {
            if let PartKind::Emitter {
                random_spawn_belt, ..
            } = &mut part.kind
                && random_spawn_belt.as_deref() == Some(id)
            {
                *random_spawn_belt = None;
            }
            if let PartKind::Machine { machine } = &mut part.kind {
                if machine.door.as_deref() == Some(id) {
                    machine.door = None;
                }
                if machine.unload_robot.as_deref() == Some(id) {
                    machine.unload_robot = None;
                }
            }
        }
        let prefix = format!("{id}.");
        self.tag_names.retain(|tag, _| !tag.starts_with(&prefix));
        self.initial_commands
            .retain(|tag, _| !tag.starts_with(&prefix));
        for driver in &mut self.drivers {
            match &mut driver.protocol {
                Protocol::ModbusClient { bindings, .. }
                | Protocol::ModbusServer { bindings, .. } => {
                    bindings.retain(|binding| !binding.tag.starts_with(&prefix));
                }
                Protocol::OpcUaClient { bindings, .. } => {
                    bindings.retain(|binding| !binding.tag.starts_with(&prefix));
                }
            }
        }
        true
    }

    fn check_pipe(&self, pipe: &Pipe, existing: &[Pipe]) -> Result<()> {
        let (from, to) = (pipe.from.as_str(), pipe.to.as_str());
        ensure!(from != to, "a water pipe cannot connect {from} to itself");
        let sizes = |id, outlet| {
            self.parts
                .iter()
                .find(|part| part.id == id)
                .map_or(0, |part| part.kind.water_port_count(outlet))
        };
        ensure!(
            pipe.from_port < sizes(from, true),
            "{from} has no water outlet {}",
            u16::from(pipe.from_port) + 1
        );
        ensure!(
            pipe.to_port < sizes(to, false),
            "{to} has no water inlet {}",
            u16::from(pipe.to_port) + 1
        );
        ensure!(
            !existing.iter().any(
                |other| (other.from == from && other.from_port == pipe.from_port)
                    || (other.to == to && other.to_port == pipe.to_port)
            ),
            "water port already connected in {from} → {to}"
        );
        let mut pending = vec![to];
        let mut visited = BTreeSet::new();
        while let Some(node) = pending.pop() {
            ensure!(
                node != from,
                "water pipe {from} → {to} would create a cycle"
            );
            if visited.insert(node) {
                pending.extend(
                    existing
                        .iter()
                        .filter(|p| p.from == node)
                        .map(|p| p.to.as_str()),
                );
            }
        }
        Ok(())
    }

    #[cfg(feature = "studio")]
    pub fn can_connect_pipe(&self, pipe: &Pipe) -> bool {
        self.check_pipe(pipe, &self.pipes).is_ok()
    }

    pub fn tags(&self) -> BTreeMap<String, Tag> {
        let mut result = BTreeMap::new();
        let mut add = |id: &str, name: &str, direction, default, range| {
            let value_type = match default {
                Value::Bool(_) => ValueType::Bool,
                Value::Int(_) => ValueType::Int,
                Value::Float(_) => ValueType::Float,
            };
            result.insert(
                id.to_owned(),
                Tag {
                    id: id.to_owned(),
                    name: self
                        .tag_names
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| name.to_owned()),
                    direction,
                    value_type,
                    default,
                    range,
                },
            );
        };
        add(
            "sim.running",
            "Simulation running",
            Direction::Sensor,
            Value::Bool(false),
            None,
        );
        add(
            "sim.time",
            "Elapsed simulation time",
            Direction::Sensor,
            Value::Float(0.0),
            None,
        );
        for part in &self.parts {
            let mut role = |id, name, dir, value, range| {
                add(
                    &format!("{}.{id}", part.id),
                    &format!("{} · {name}", part.name),
                    dir,
                    value,
                    range,
                )
            };
            use Direction::{Actuator, Sensor};
            use Value::{Bool, Float, Int};
            match part.kind {
                PartKind::Conveyor { speed, .. } => {
                    role("enabled", "Enable", Actuator, Bool(true), None);
                    role(
                        "target_speed",
                        "Target speed",
                        Actuator,
                        Float(speed),
                        Some([-speed, speed]),
                    );
                    role(
                        "speed",
                        "Belt speed",
                        Sensor,
                        Float(0.0),
                        Some([-speed, speed]),
                    );
                }
                PartKind::Pusher { stroke, .. } => {
                    role("extend", "Extend", Actuator, Bool(false), None);
                    role("front", "Front limit", Sensor, Bool(false), None);
                    role("back", "Back limit", Sensor, Bool(true), None);
                    role(
                        "position",
                        "Stroke",
                        Sensor,
                        Float(0.0),
                        Some([0.0, stroke]),
                    );
                }
                PartKind::Sensor { range } => {
                    role("detected", "Detected", Sensor, Bool(false), None);
                    role(
                        "distance",
                        "Distance",
                        Sensor,
                        Float(range),
                        Some([0.0, range]),
                    );
                }
                PartKind::Emitter { ref emission, .. } => {
                    role("enabled", "Enable", Actuator, Bool(false), None);
                    role("count", "Emitted", Sensor, Int(0), None);
                    role("spawn_blocked", "Spawn blocked", Sensor, Bool(false), None);
                    if emission.is_some() {
                        role("skipped", "Skipped", Sensor, Int(0), None);
                        role(
                            "recipe_error",
                            "Invalid probability",
                            Sensor,
                            Bool(false),
                            None,
                        );
                    }
                }
                PartKind::Remover { .. } => {
                    role("enabled", "Enable", Actuator, Bool(true), None);
                    role("count", "Removed", Sensor, Int(0), None);
                }
                PartKind::Equipment {
                    equipment,
                    scale_capacity,
                    ..
                } => {
                    use Equipment::*;
                    let (commands, readings): (&[&str], &[&str]) = match equipment {
                        Roller | Scale => (&["forward", "reverse"], &[]),
                        StopBlade => (&["raise"], &["raised"]),
                        Capacitive => (&[], &["detected"]),
                        LightArray => (&[], &["aligned"]),
                        StackLight => (&["red", "yellow", "green"], &[]),
                        Fence | FreeRoller | Chute => (&[], &[]),
                        SafetyDoor => (&["open"], &["closed", "obstructed"]),
                        Turntable => (
                            &["forward", "reverse", "turn"],
                            &["front", "back", "limit_0", "limit_90"],
                        ),
                        ChainTransfer => (&["forward", "reverse", "left", "right"], &["raised"]),
                    };
                    for &command in commands {
                        role(command, command, Actuator, Bool(false), None);
                    }
                    for &reading in readings {
                        role(
                            reading,
                            reading,
                            Sensor,
                            Bool(matches!(reading, "closed" | "limit_0")),
                            None,
                        );
                    }
                    match equipment {
                        Roller => role(
                            "speed_v",
                            "Analog speed (V)",
                            Actuator,
                            Float(0.0),
                            Some([-10.0, 10.0]),
                        ),
                        Scale => {
                            role(
                                "weight_kg",
                                "Weight (kg)",
                                Sensor,
                                Float(0.0),
                                Some([0.0, scale_capacity as f32]),
                            );
                            role(
                                "weight_v",
                                "Weight (V)",
                                Sensor,
                                Float(0.0),
                                Some([0.0, 10.0]),
                            );
                        }
                        Capacitive => {
                            role(
                                "distance",
                                "Distance (m)",
                                Sensor,
                                Float(0.2),
                                Some([0.0, 0.2]),
                            );
                            role(
                                "distance_v",
                                "Distance (V)",
                                Sensor,
                                Float(10.0),
                                Some([0.0, 10.0]),
                            );
                        }
                        LightArray => {
                            for beam in [
                                "beam_1", "beam_2", "beam_3", "beam_4", "beam_5", "beam_6",
                                "beam_7", "beam_8",
                            ] {
                                role(beam, beam, Sensor, Bool(false), None);
                            }
                            role("mask", "Blocked beams", Sensor, Int(0), None);
                            role(
                                "value_v",
                                "Blocked beams (V)",
                                Sensor,
                                Float(0.0),
                                Some([0.0, 10.0]),
                            );
                        }
                        Turntable => role(
                            "angle",
                            "Table angle (°)",
                            Sensor,
                            Float(0.0),
                            Some([0.0, 90.0]),
                        ),
                        StopBlade | ChainTransfer => {
                            role("position", "Lift (m)", Sensor, Float(0.0), None)
                        }
                        SafetyDoor => role(
                            "angle",
                            "Door angle (°)",
                            Sensor,
                            Float(0.0),
                            Some([0.0, 90.0]),
                        ),
                        _ => {}
                    }
                }
                PartKind::Machine { ref machine } => {
                    for name in ["start", "stop", "reset", "emergency"] {
                        role(name, name, Actuator, Bool(false), None);
                    }
                    for name in [
                        "busy",
                        "fault",
                        "input_error",
                        "repairing",
                        "waiting_operator",
                        "waiting_unload",
                        "blocked",
                        "interlock",
                    ] {
                        role(name, name, Sensor, Bool(false), None);
                    }
                    for name in ["good_count", "scrap_count", "failure_count"] {
                        role(name, name, Sensor, Int(0), None);
                    }
                    if let Some(assembly) = &machine.assembly {
                        role(
                            "waiting_material",
                            "Waiting for components",
                            Sensor,
                            Bool(false),
                            None,
                        );
                        role(
                            "material_count",
                            "Components collected",
                            Sensor,
                            Int(0),
                            None,
                        );
                        let required = assembly.bom.iter().fold(0_i32, |sum, row| {
                            sum.saturating_add(i32::try_from(row.count).unwrap_or(i32::MAX))
                        });
                        role(
                            "required_count",
                            "Components required",
                            Sensor,
                            Int(required),
                            None,
                        );
                    }
                    role(
                        "progress",
                        if machine.assembly.is_some() {
                            "Assembly progress (%)"
                        } else {
                            "Machining progress (%)"
                        },
                        Sensor,
                        Float(0.0),
                        Some([0.0, 100.0]),
                    );
                    role(
                        "repair_progress",
                        "Repair progress (%)",
                        Sensor,
                        Float(0.0),
                        Some([0.0, 100.0]),
                    );
                }
                PartKind::Robot { .. } => {
                    for name in ["start", "stop", "reset", "signal"] {
                        role(name, name, Actuator, Bool(false), None);
                    }
                    for name in ["moving", "holding", "waiting", "done"] {
                        role(name, name, Sensor, Bool(false), None);
                    }
                    role(
                        "out_of_reach",
                        "Unload route unavailable",
                        Sensor,
                        Bool(false),
                        None,
                    );
                    role("step", "Program step", Sensor, Int(0), None);
                    for name in ["x", "y", "z", "angle"] {
                        role(name, name, Sensor, Float(0.0), None);
                    }
                }
                PartKind::Staff { .. } => {
                    role("on_duty", "On duty", Sensor, Bool(false), None);
                    role("busy", "Busy", Sensor, Bool(false), None);
                }
                PartKind::HeatedTank(tank) => {
                    role("mixer_on", "Mixer on", Actuator, Bool(false), None);
                    role(
                        "heater_pct",
                        "Heater command (%)",
                        Actuator,
                        Float(0.0),
                        Some([0.0, 100.0]),
                    );
                    if !tank.networked {
                        role(
                            "feed_l_s",
                            "Water feed (L/s)",
                            Actuator,
                            Float(0.0),
                            Some([0.0, tank.feed_max_l_s]),
                        );
                    }
                    role(
                        "temperature_c",
                        "Water temperature (°C)",
                        Sensor,
                        Float(tank.initial_temperature_c),
                        None,
                    );
                    role(
                        "heater_actual_w",
                        "Heater output (W)",
                        Sensor,
                        Float(0.0),
                        Some([0.0, tank.heater_max_w]),
                    );
                    role(
                        "heater_blocked",
                        "Heater blocked",
                        Sensor,
                        Bool(false),
                        None,
                    );
                    role(
                        "level_pct",
                        "Water level (%)",
                        Sensor,
                        Float(tank.starting_volume_l() / tank.volume_l * 100.0),
                        Some([0.0, 100.0]),
                    );
                    if tank.networked {
                        role("inflow_l_s", "Inlet flow (L/s)", Sensor, Float(0.0), None);
                        role("outflow_l_s", "Outlet flow (L/s)", Sensor, Float(0.0), None);
                    }
                }
                PartKind::WaterPump { max_flow_l_s, .. } => {
                    role(
                        "target_l_s",
                        "Pump flow at zero head (L/s)",
                        Actuator,
                        Float(0.0),
                        Some([0.0, max_flow_l_s]),
                    );
                    role(
                        "reset_fault",
                        "Reset pump fault",
                        Actuator,
                        Bool(false),
                        None,
                    );
                    role("failed", "Pump failed", Sensor, Bool(false), None);
                    role("wear_pct", "Pump wear (%)", Sensor, Float(0.0), None);
                    role("failures", "Pump failures", Sensor, Int(0), None);
                }
                PartKind::WaterValve { .. } => {
                    role(
                        "opening_pct",
                        "Valve opening (%)",
                        Actuator,
                        Float(0.0),
                        Some([0.0, 100.0]),
                    );
                }
                PartKind::WaterInstrument { measurement, .. } => match measurement {
                    WaterMeasurement::Flow => {}
                    WaterMeasurement::Temperature => role(
                        "temperature_c",
                        "Water temperature (°C)",
                        Sensor,
                        Float(0.0),
                        None,
                    ),
                    WaterMeasurement::Pressure => role(
                        "pressure_kpa",
                        "Water pressure (kPa)",
                        Sensor,
                        Float(0.0),
                        None,
                    ),
                },
                PartKind::WaterFilter { .. } => {
                    role(
                        "replace",
                        "Replace filter cartridge",
                        Actuator,
                        Bool(false),
                        None,
                    );
                    role(
                        "loading_pct",
                        "Filter loading (%)",
                        Sensor,
                        Float(0.0),
                        None,
                    );
                    role(
                        "pressure_drop_kpa",
                        "Pressure drop (kPa)",
                        Sensor,
                        Float(0.0),
                        None,
                    );
                    role(
                        "inlet_dirt_mg_l",
                        "Inlet dirt (mg/L)",
                        Sensor,
                        Float(0.0),
                        None,
                    );
                }
                PartKind::WaterHeater { max_power_w, .. } => {
                    role(
                        "power_pct",
                        "Heater command (%)",
                        Actuator,
                        Float(0.0),
                        Some([0.0, 100.0]),
                    );
                    role(
                        "power_w",
                        "Heater output (W)",
                        Sensor,
                        Float(0.0),
                        Some([0.0, max_power_w]),
                    );
                }
                PartKind::WaterHeatExchanger { .. } => {
                    role(
                        "heat_w",
                        "Heat to second circuit (W)",
                        Sensor,
                        Float(0.0),
                        None,
                    );
                    role(
                        "outlet_2_temperature_c",
                        "Second outlet temperature (°C)",
                        Sensor,
                        Float(0.0),
                        None,
                    );
                    role(
                        "outlet_2_dirt_mg_l",
                        "Second outlet dirt (mg/L)",
                        Sensor,
                        Float(0.0),
                        None,
                    );
                }
                PartKind::WaterFiller { dose_l, .. } => {
                    role("fill", "Fill container", Actuator, Bool(false), None);
                    role(
                        "target_l",
                        "Target dose (L)",
                        Actuator,
                        Float(dose_l),
                        Some([0.0, f32::MAX]),
                    );
                    role(
                        "container_present",
                        "Container present",
                        Sensor,
                        Bool(false),
                        None,
                    );
                    role(
                        "filled_l",
                        "Liquid in container (L)",
                        Sensor,
                        Float(0.0),
                        None,
                    );
                    role("dose_done", "Dose complete", Sensor, Bool(false), None);
                }
                PartKind::Crate { .. }
                | PartKind::Barrier { .. }
                | PartKind::FlowGuide { .. }
                | PartKind::WaterJunction { .. }
                | PartKind::WaterSource { .. }
                | PartKind::WaterDrain { .. } => {}
            }
            if part.kind.has_water_inlet() || part.kind.has_water_outlet() {
                role("flow_l_s", "Actual flow (L/s)", Sensor, Float(0.0), None);
                role("dirt_mg_l", "Water dirt (mg/L)", Sensor, Float(0.0), None);
                role(
                    "liquid_temp_c",
                    "Stream temperature (°C)",
                    Sensor,
                    Float(0.0),
                    None,
                );
            }
            if part.kind.water_port_count(true) > 1 {
                role(
                    "outlet_1_l_s",
                    "First outlet flow (L/s)",
                    Sensor,
                    Float(0.0),
                    None,
                );
                role(
                    "outlet_2_l_s",
                    "Second outlet flow (L/s)",
                    Sensor,
                    Float(0.0),
                    None,
                );
            }
        }
        result
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == SCHEMA_VERSION,
            "unsupported project version {}",
            self.version
        );
        ensure!(!self.name.trim().is_empty(), "project name cannot be empty");
        for (id, product) in &self.products {
            ensure!(
                valid_id(id) && !BOXES.iter().any(|(key, _)| *key == id),
                "invalid or reserved product id {id}"
            );
            product
                .validate()
                .with_context(|| format!("invalid product {id}"))?;
        }
        self.production.validate()?;
        let mut ids = BTreeSet::new();
        for part in &self.parts {
            ensure!(
                valid_id(&part.id) && ids.insert(&part.id),
                "invalid or duplicate part id {}",
                part.id
            );
            ensure!(!part.name.trim().is_empty(), "empty name for {}", part.id);
            ensure!(
                part.position
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 1000.0)
                    && part.yaw.is_finite(),
                "invalid transform for {}",
                part.id
            );
            if let Some(id) = &part.product {
                ensure!(
                    matches!(part.kind, PartKind::Crate { .. } | PartKind::Emitter { .. })
                        && self.product(id).is_some(),
                    "invalid product {id} for {}",
                    part.id
                );
            }
            let positive = |values: &[f32]| {
                values
                    .iter()
                    .all(|v| v.is_finite() && *v > 0.0 && *v <= 1000.0)
            };
            let ok = match part.kind {
                PartKind::Conveyor { size, speed } => positive(&size) && positive(&[speed]),
                PartKind::Pusher { stroke, speed } => positive(&[stroke, speed]),
                PartKind::Sensor { range } => positive(&[range]),
                PartKind::Emitter {
                    interval,
                    size,
                    mass,
                    ref emission,
                    ref random_spawn_belt,
                } => {
                    ensure!(
                        random_spawn_belt.as_ref().is_none_or(|id| self
                            .parts
                            .iter()
                            .any(|belt| &belt.id == id && belt.kind.feed_surface().is_some())),
                        "invalid spawn conveyor for {}",
                        part.id
                    );
                    if let Some(emission) = emission {
                        production::Emitter::new(emission)
                            .with_context(|| format!("emitter {}", part.id))?;
                        let products: Vec<_> = match emission {
                            Emission::Counts { rows, .. } => rows
                                .iter()
                                .filter_map(|row| row.product.as_deref())
                                .collect(),
                            Emission::Probability { rows } => rows
                                .iter()
                                .filter_map(|row| row.product.as_deref())
                                .collect(),
                        };
                        for id in products {
                            ensure!(
                                self.product(id).is_some(),
                                "{} has unknown emission product {id}",
                                part.id
                            );
                        }
                    }
                    positive(&[interval, mass]) && positive(&size)
                }
                PartKind::Crate { size, mass } => positive(&[mass]) && positive(&size),
                PartKind::Remover { size } | PartKind::Barrier { size } => positive(&size),
                PartKind::HeatedTank(tank) => {
                    positive(&tank.size)
                        && [tank.volume_l, tank.heater_max_w]
                            .into_iter()
                            .all(|value| value.is_finite() && value > 0.0)
                        && [tank.heat_loss_w_per_k, tank.feed_max_l_s]
                            .into_iter()
                            .all(|value| value.is_finite() && value >= 0.0)
                        && tank
                            .initial_volume_l
                            .is_none_or(|volume| (0.0..=tank.volume_l).contains(&volume))
                        && (0.0..100.0).contains(&tank.initial_temperature_c)
                        && (0.0..100.0).contains(&tank.ambient_temperature_c)
                        && (0.0..100.0).contains(&tank.feed_temperature_c)
                        && tank.initial_dirt_mg_l.is_finite()
                        && tank.initial_dirt_mg_l >= 0.0
                }
                PartKind::WaterSource {
                    size,
                    temperature_c,
                    max_flow_l_s,
                    dirt_mg_l,
                } => {
                    positive(&size)
                        && (0.0..100.0).contains(&temperature_c)
                        && dirt_mg_l.is_finite()
                        && dirt_mg_l >= 0.0
                        && max_flow_l_s.is_finite()
                        && max_flow_l_s >= 0.0
                }
                PartKind::WaterPump {
                    size,
                    max_flow_l_s,
                    shutoff_head_m,
                    dirt_limit_mg,
                } => {
                    positive(&size)
                        && max_flow_l_s.is_finite()
                        && max_flow_l_s >= 0.0
                        && shutoff_head_m.is_finite()
                        && shutoff_head_m > 0.0
                        && dirt_limit_mg.is_finite()
                        && dirt_limit_mg > 0.0
                }
                PartKind::WaterValve {
                    size,
                    max_flow_l_s,
                    open_resistance,
                } => {
                    positive(&size)
                        && max_flow_l_s.is_finite()
                        && max_flow_l_s >= 0.0
                        && open_resistance.is_finite()
                        && open_resistance >= 0.0
                }
                PartKind::WaterDrain { size, max_flow_l_s } => {
                    positive(&size) && max_flow_l_s.is_finite() && max_flow_l_s >= 0.0
                }
                PartKind::WaterJunction { size, .. } | PartKind::WaterInstrument { size, .. } => {
                    positive(&size)
                }
                PartKind::WaterFilter {
                    size,
                    clean_resistance,
                    dirty_resistance,
                    capacity_mg,
                    efficiency_pct,
                } => {
                    positive(&size)
                        && clean_resistance.is_finite()
                        && clean_resistance >= 0.0
                        && dirty_resistance.is_finite()
                        && dirty_resistance >= clean_resistance
                        && capacity_mg.is_finite()
                        && capacity_mg > 0.0
                        && (0.0..=100.0).contains(&efficiency_pct)
                }
                PartKind::WaterHeater { size, max_power_w } => {
                    positive(&size) && max_power_w.is_finite() && max_power_w > 0.0
                }
                PartKind::WaterHeatExchanger {
                    size,
                    conductance_w_k,
                    resistance,
                } => {
                    positive(&size)
                        && conductance_w_k.is_finite()
                        && conductance_w_k > 0.0
                        && resistance.is_finite()
                        && resistance >= 0.0
                }
                PartKind::WaterFiller {
                    size,
                    max_flow_l_s,
                    dose_l,
                } => {
                    positive(&size)
                        && max_flow_l_s.is_finite()
                        && max_flow_l_s > 0.0
                        && dose_l.is_finite()
                        && dose_l > 0.0
                }
                PartKind::FlowGuide {
                    length,
                    height,
                    inlet,
                    outlet,
                } => {
                    positive(&[length, height, inlet, outlet]) && length >= 0.02 && inlet >= outlet
                }
                PartKind::Equipment {
                    equipment,
                    size,
                    scale_capacity,
                } => {
                    positive(&size)
                        && (equipment != Equipment::Scale || matches!(scale_capacity, 20 | 100))
                        && (equipment != Equipment::LightArray || size[0] <= 1.5)
                }
                PartKind::Robot { ref robot } => {
                    robot
                        .validate()
                        .with_context(|| format!("robot {}", part.id))?;
                    true
                }
                PartKind::Staff { ref staff } => {
                    !staff.area.trim().is_empty()
                        && staff
                            .calendar
                            .as_ref()
                            .is_none_or(|id| self.production.calendars.contains_key(id))
                }
                PartKind::Machine { ref machine } => {
                    production::ticks(machine.seconds)
                        .with_context(|| format!("machining time for {}", part.id))?;
                    production::ticks(machine.repair_seconds)
                        .with_context(|| format!("repair time for {}", part.id))?;
                    if let Some(assembly) = &machine.assembly {
                        let mut products = std::collections::BTreeSet::new();
                        let total: u64 = assembly.bom.iter().map(|row| u64::from(row.count)).sum();
                        ensure!(
                            (1..=u64::from(production::MAX_ASSEMBLY_COMPONENTS)).contains(&total)
                                && assembly.bom.iter().all(|row| row.count > 0
                                    && self.product(&row.product).is_some()
                                    && products.insert(&row.product)),
                            "invalid bill of materials for {}: expected unique known products and 1 to {} components",
                            part.id,
                            production::MAX_ASSEMBLY_COMPONENTS
                        );
                    }
                    ensure!(
                        (machine.assembly.is_some() || self.product(&machine.input).is_some())
                            && self.product(&machine.output).is_some()
                            && machine
                                .scrap_product
                                .as_ref()
                                .is_none_or(|id| self.product(id).is_some()),
                        "unknown recipe product for {}",
                        part.id
                    );
                    ensure!(
                        machine
                            .door
                            .as_ref()
                            .is_none_or(|id| self.parts.iter().any(|door| &door.id == id
                                && matches!(
                                    door.kind,
                                    PartKind::Equipment {
                                        equipment: Equipment::SafetyDoor,
                                        ..
                                    }
                                ))),
                        "invalid safety door for {}",
                        part.id
                    );
                    ensure!(
                        machine.unload_robot.as_ref().is_none_or(|id| self
                            .parts
                            .iter()
                            .any(|robot| &robot.id == id
                                && matches!(robot.kind, PartKind::Robot { .. }))),
                        "invalid unload robot for {}",
                        part.id
                    );
                    !machine.area.trim().is_empty()
                        && [machine.scrap_percent, machine.failure_percent]
                            .iter()
                            .all(|p| (0.0..=100.0).contains(p))
                        && [machine.intake, machine.outlet, machine.reject]
                            .into_iter()
                            .all(production::position)
                        && machine.outlet != machine.reject
                        && machine.intake != machine.outlet
                        && machine.intake != machine.reject
                }
            };
            ensure!(
                ok,
                "invalid dimensions or operating parameters for {}",
                part.id
            );
        }
        for (index, pipe) in self.pipes.iter().enumerate() {
            self.check_pipe(pipe, &self.pipes[..index])?;
            ensure!(
                pipe.resistance_per_m.is_finite() && pipe.resistance_per_m > 0.0,
                "invalid pipe resistance for {} → {}",
                pipe.from,
                pipe.to
            );
        }
        let tags = self.tags();
        for (id, name) in &self.tag_names {
            ensure!(
                tags.contains_key(id) && !name.trim().is_empty(),
                "invalid tag name for {id}"
            );
        }
        for (id, value) in &self.initial_commands {
            let tag = tags
                .get(id)
                .with_context(|| format!("unknown initial command {id}"))?;
            ensure!(tag.direction == Direction::Actuator, "{id} is a sensor");
            tag.check(*value)?;
        }
        let mut driver_ids = BTreeSet::new();
        let mut actuator_owners = BTreeSet::new();
        let mut listeners = BTreeSet::new();
        for driver in &self.drivers {
            ensure!(
                valid_id(&driver.id) && driver_ids.insert(&driver.id),
                "invalid or duplicate driver id {}",
                driver.id
            );
            ensure!(
                (5..=60_000).contains(&driver.poll_ms),
                "invalid poll period for {}",
                driver.id
            );
            let mut mapped = BTreeSet::new();
            for id in driver.mapped_tags() {
                let tag = tags
                    .get(id)
                    .with_context(|| format!("{}: unknown tag {id}", driver.id))?;
                ensure!(mapped.insert(id), "{}: duplicate tag {id}", driver.id);
                if driver.enabled && tag.direction == Direction::Actuator {
                    ensure!(
                        actuator_owners.insert(id),
                        "multiple enabled drivers own actuator {id}"
                    );
                }
            }
            match &driver.protocol {
                Protocol::ModbusClient {
                    address, bindings, ..
                }
                | Protocol::ModbusServer {
                    address, bindings, ..
                } => {
                    let server = matches!(driver.protocol, Protocol::ModbusServer { .. });
                    ensure!(address.port() > 0, "{}: port cannot be zero", driver.id);
                    if server && driver.enabled {
                        ensure!(
                            listeners.insert(address),
                            "duplicate server address {address}"
                        );
                    }
                    let mut locations = BTreeSet::new();
                    for binding in bindings {
                        let tag = &tags[&binding.tag];
                        ensure!(
                            binding.area.bit() == (tag.value_type == ValueType::Bool),
                            "{}: area incompatible with {}",
                            driver.id,
                            tag.id
                        );
                        let must_write = (server && tag.direction == Direction::Actuator)
                            || (!server && tag.direction == Direction::Sensor);
                        ensure!(
                            !must_write || binding.area.writable(),
                            "{}: read-only area for {}",
                            driver.id,
                            tag.id
                        );
                        ensure!(
                            binding.scale.is_finite() && binding.scale > 0.0,
                            "{}: invalid scale",
                            tag.id
                        );
                        ensure!(
                            locations.insert((binding.area, binding.address)),
                            "{}: duplicate Modbus address",
                            driver.id
                        );
                    }
                }
                Protocol::OpcUaClient {
                    endpoint, bindings, ..
                } => {
                    ensure!(
                        endpoint
                            .strip_prefix("opc.tcp://")
                            .is_some_and(|s| !s.trim().is_empty()),
                        "{}: expected opc.tcp endpoint",
                        driver.id
                    );
                    let mut nodes = BTreeSet::new();
                    for binding in bindings {
                        ensure!(
                            nodes.insert(&binding.node),
                            "{}: duplicate OPC UA node",
                            driver.id
                        );
                        #[cfg(feature = "opcua")]
                        binding
                            .node
                            .parse::<opcua::types::NodeId>()
                            .map_err(|err| {
                                anyhow::anyhow!(
                                    "{}: invalid NodeId {}: {err:?}",
                                    driver.id,
                                    binding.node
                                )
                            })?;
                        #[cfg(not(feature = "opcua"))]
                        ensure!(
                            !driver.enabled,
                            "{}: OPC UA support was omitted at build time",
                            driver.id
                        );
                    }
                    #[cfg(not(feature = "opcua"))]
                    ensure!(
                        !driver.enabled,
                        "{}: OPC UA support was omitted at build time",
                        driver.id
                    );
                }
            }
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        && id != "sim"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_a_part_removes_only_its_tag_configuration() -> Result<()> {
        let mut project = Project::demo();
        let mut other = project
            .parts
            .iter()
            .find(|part| part.id == "main")
            .unwrap()
            .clone();
        other.id = "main-backup".into();
        project.parts.push(other);
        project
            .tag_names
            .insert("main.speed".into(), "Speed".into());
        project
            .initial_commands
            .insert("main.enabled".into(), Value::Bool(false));
        project
            .initial_commands
            .insert("main-backup.enabled".into(), Value::Bool(false));

        assert!(project.remove_part("main"));
        assert!(!project.remove_part("main"));
        assert!(!project.tags().keys().any(|tag| tag.starts_with("main.")));
        assert!(!project.tag_names.contains_key("main.speed"));
        assert!(!project.initial_commands.contains_key("main.enabled"));
        assert!(project.initial_commands.contains_key("main-backup.enabled"));
        for driver in &project.drivers {
            assert!(driver.mapped_tags().contains(&"diverter.extend"));
            assert!(
                !driver
                    .mapped_tags()
                    .iter()
                    .any(|tag| tag.starts_with("main."))
            );
        }
        project.validate()
    }
}
