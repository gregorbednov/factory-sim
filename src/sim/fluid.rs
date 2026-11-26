use std::collections::{BTreeMap, BTreeSet};

use rapier3d::na::{DMatrix, DVector};

use crate::project::{Part, PartKind, Project, water_distance, water_pipe_points};

const HEAT_CAPACITY: f64 = 4180.0;
const KPA_PER_M: f64 = 9.80665;

#[derive(Default, Clone, Copy)]
pub(super) struct Flow {
    pub rate_l_s: f64,
    pub temperature_c: f64,
    pub dirt_mg_l: f64,
}

impl Flow {
    pub fn mix(&mut self, incoming: Flow) {
        let total = self.rate_l_s + incoming.rate_l_s;
        if total > 0.0 {
            let share = incoming.rate_l_s / total;
            self.temperature_c += share * (incoming.temperature_c - self.temperature_c);
            self.dirt_mg_l += share * (incoming.dirt_mg_l - self.dirt_mg_l);
            self.rate_l_s = total;
        }
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct PortFlows {
    pub inlet: Flow,
    pub outlet: Flow,
    pub second_outlet: Flow,
    pub pressure_kpa: f64,
    pub pressure_drop_kpa: f64,
    pub power_w: f64,
}

#[derive(Clone, Copy)]
pub(super) struct Storage {
    pub volume_l: f64,
    pub capacity_l: f64,
    pub temperature_c: f64,
    pub dirt_mg_l: f64,
}

#[derive(Default)]
struct Service {
    load_mg: f64,
    failures: i32,
    reset_held: bool,
}

#[derive(Clone)]
struct InlineDevice {
    id: String,
    lane: u8,
    elevation: f64,
    preceding_pipe: f64,
    kind: PartKind,
}

#[derive(Clone, Copy)]
enum Boundary {
    Source {
        head: f64,
        supply: Flow,
    },
    Tank {
        bottom: f64,
        depth: f64,
        inlet_height: f64,
    },
    Sink {
        head: f64,
        max_rate: f64,
        container: bool,
    },
    Junction,
}

impl Boundary {
    fn from_part(part: &Part) -> Option<Self> {
        let head = f64::from(part.position[1]);
        Some(match part.kind {
            PartKind::WaterSource {
                size,
                max_flow_l_s,
                temperature_c,
                dirt_mg_l,
            } => Self::Source {
                head: head + 0.35 * f64::from(size[1]),
                supply: Flow {
                    rate_l_s: f64::from(max_flow_l_s),
                    temperature_c: f64::from(temperature_c),
                    dirt_mg_l: f64::from(dirt_mg_l),
                },
            },
            PartKind::HeatedTank(tank) if tank.networked => Self::Tank {
                bottom: head - 0.5 * f64::from(tank.size[1]),
                depth: 0.8 * f64::from(tank.size[1]),
                inlet_height: part.water_port(false).unwrap().0[1],
            },
            PartKind::WaterDrain { size, max_flow_l_s } => Self::Sink {
                head: head - 0.5 * f64::from(size[1]),
                max_rate: f64::from(max_flow_l_s),
                container: false,
            },
            PartKind::WaterFiller { max_flow_l_s, .. } => Self::Sink {
                head,
                max_rate: f64::from(max_flow_l_s),
                container: true,
            },
            PartKind::WaterJunction { .. } => Self::Junction,
            _ => return None,
        })
    }
}

struct Node {
    id: String,
    boundary: Boundary,
    incoming: Vec<usize>,
    outgoing: Vec<usize>,
}

struct Segment {
    from: usize,
    from_port: u8,
    to: usize,
    devices: Vec<InlineDevice>,
    pipe_resistance: f64,
    final_pipe: f64,
}

#[derive(Clone, Copy)]
struct Curve {
    boost: f64,
    resistance: f64,
    limit: f64,
}

impl Curve {
    fn flow(self, head_delta: f64) -> f64 {
        ((head_delta + self.boost).max(0.0) / self.resistance)
            .sqrt()
            .min(self.limit)
    }
}

pub(super) struct Network {
    boundaries: Vec<Node>,
    segments: Vec<Segment>,
    services: BTreeMap<String, Service>,
    exchangers: BTreeMap<String, f64>,
}

impl Network {
    pub fn new(project: &Project) -> Self {
        let parts: BTreeMap<_, _> = project
            .parts
            .iter()
            .map(|part| (part.id.as_str(), part))
            .collect();
        let pipes: BTreeMap<_, _> = project
            .pipes
            .iter()
            .map(|p| ((p.from.as_str(), p.from_port), p))
            .collect();
        let mut boundaries = Vec::new();
        let mut indexes = BTreeMap::new();
        // The project validates the directed graph as acyclic. Build its parts in topological order.
        let mut indegree: BTreeMap<_, _> = parts.keys().map(|&id| (id, 0)).collect();
        for pipe in &project.pipes {
            *indegree.get_mut(pipe.to.as_str()).unwrap() += 1;
        }
        let mut ready: BTreeSet<_> = indegree
            .iter()
            .filter_map(|(&id, &n)| (n == 0).then_some(id))
            .collect();
        while let Some(id) = ready.pop_first() {
            if let Some(boundary) = Boundary::from_part(parts[id]) {
                indexes.insert(id, boundaries.len());
                boundaries.push(Node {
                    id: id.to_owned(),
                    boundary,
                    incoming: Vec::new(),
                    outgoing: Vec::new(),
                });
            }
            for port in 0..parts[id].kind.water_port_count(true) {
                if let Some(pipe) = pipes.get(&(id, port)) {
                    let remaining = indegree.get_mut(pipe.to.as_str()).unwrap();
                    *remaining -= 1;
                    if *remaining == 0 {
                        ready.insert(&pipe.to);
                    }
                }
            }
        }
        let mut segments = Vec::new();
        for from in 0..boundaries.len() {
            let id = boundaries[from].id.clone();
            for from_port in 0..parts[id.as_str()].kind.water_port_count(true) {
                let mut next = pipes.get(&(id.as_str(), from_port)).copied();
                let mut devices = Vec::new();
                let mut pipe_resistance = 0.0;
                while let Some(pipe) = next {
                    let part = parts[pipe.to.as_str()];
                    let points = water_pipe_points(
                        parts[pipe.from.as_str()],
                        pipe.from_port,
                        part,
                        pipe.to_port,
                    )
                    .unwrap();
                    let length = points
                        .windows(2)
                        .map(|leg| water_distance(leg[0], leg[1]))
                        .sum::<f64>();
                    let resistance = f64::from(pipe.resistance_per_m) * length.max(0.001);
                    pipe_resistance += resistance;
                    if let Some(&to) = indexes.get(pipe.to.as_str()) {
                        boundaries[from].outgoing.push(segments.len());
                        boundaries[to].incoming.push(segments.len());
                        segments.push(Segment {
                            from,
                            from_port,
                            to,
                            devices,
                            pipe_resistance,
                            final_pipe: resistance,
                        });
                        break;
                    }
                    devices.push(InlineDevice {
                        id: part.id.clone(),
                        lane: pipe.to_port,
                        elevation: f64::from(part.position[1]),
                        preceding_pipe: resistance,
                        kind: part.kind.clone(),
                    });
                    next = pipes.get(&(part.id.as_str(), pipe.to_port)).copied();
                }
            }
        }
        let services = project
            .parts
            .iter()
            .filter(|p| {
                matches!(
                    p.kind,
                    PartKind::WaterPump { .. } | PartKind::WaterFilter { .. }
                )
            })
            .map(|p| (p.id.clone(), Service::default()))
            .collect();
        let exchangers = project
            .parts
            .iter()
            .filter_map(|p| match p.kind {
                PartKind::WaterHeatExchanger {
                    conductance_w_k, ..
                } => Some((p.id.clone(), f64::from(conductance_w_k))),
                _ => None,
            })
            .collect();
        Self {
            boundaries,
            segments,
            services,
            exchangers,
        }
    }

    #[cfg(any(feature = "studio", feature = "web"))]
    pub fn reconnect(&mut self, project: &Project) {
        let mut network = Self::new(project);
        for (id, service) in &mut network.services {
            if let Some(previous) = self.services.remove(id) {
                *service = previous;
            }
        }
        *self = network;
    }

    fn device_curve(
        device: &InlineDevice,
        services: &BTreeMap<String, Service>,
        command: &impl Fn(&str, &str) -> f64,
    ) -> Curve {
        let mut curve = Curve {
            boost: 0.0,
            resistance: 0.0,
            limit: f64::INFINITY,
        };
        let id = &device.id;
        match device.kind {
            PartKind::WaterPump {
                max_flow_l_s,
                shutoff_head_m,
                dirt_limit_mg,
                ..
            } => {
                let target = command(id, "target_l_s");
                if target == 0.0 || services[id].load_mg >= f64::from(dirt_limit_mg) {
                    curve.limit = 0.0;
                } else {
                    let rated = f64::from(max_flow_l_s);
                    curve.boost = f64::from(shutoff_head_m) * (target / rated).powi(2);
                    curve.resistance = f64::from(shutoff_head_m) / rated.powi(2);
                }
            }
            PartKind::WaterValve {
                max_flow_l_s,
                open_resistance,
                ..
            } => {
                let opening = command(id, "opening_pct") / 100.0;
                if opening == 0.0 {
                    curve.limit = 0.0;
                } else {
                    curve.limit = f64::from(max_flow_l_s);
                    curve.resistance = f64::from(open_resistance) / opening.powi(2);
                }
            }
            PartKind::WaterFilter {
                clean_resistance,
                dirty_resistance,
                capacity_mg,
                ..
            } => {
                curve.resistance = f64::from(clean_resistance)
                    + f64::from(dirty_resistance - clean_resistance)
                        * (services[id].load_mg / f64::from(capacity_mg))
            }
            PartKind::WaterHeatExchanger { resistance, .. } => {
                curve.resistance = f64::from(resistance)
            }
            _ => {}
        }
        curve
    }

    fn calculate_rates(
        &self,
        curves: &[Curve],
        in_heads: &mut [f64],
        out_heads: &mut [f64],
    ) -> Vec<f64> {
        let span = curves.iter().map(|c| c.boost).sum::<f64>() + 1.0;
        let lower = in_heads
            .iter()
            .chain(out_heads.iter())
            .copied()
            .fold(0.0, f64::min)
            - span;
        let upper = in_heads
            .iter()
            .chain(out_heads.iter())
            .copied()
            .fold(0.0, f64::max)
            + span;
        let junctions: Vec<_> = self
            .boundaries
            .iter()
            .enumerate()
            .filter_map(|(i, node)| matches!(node.boundary, Boundary::Junction).then_some(i))
            .collect();
        let indexes: BTreeMap<_, _> = junctions
            .iter()
            .enumerate()
            .map(|(row, &i)| (i, row))
            .collect();
        let mut rates = vec![0.0; self.segments.len()];
        for _ in 0..64 {
            for (index, node) in self.boundaries.iter().enumerate() {
                if !matches!(node.boundary, Boundary::Junction) {
                    continue;
                }
                let (mut low, mut high) = (lower, upper);
                for _ in 0..52 {
                    let mid = (low + high) * 0.5;
                    let inlet: f64 = node
                        .incoming
                        .iter()
                        .map(|&e| curves[e].flow(out_heads[self.segments[e].from] - mid))
                        .sum();
                    let outlet: f64 = node
                        .outgoing
                        .iter()
                        .map(|&e| curves[e].flow(mid - in_heads[self.segments[e].to]))
                        .sum();
                    if inlet > outlet {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                in_heads[index] = (low + high) * 0.5;
                out_heads[index] = in_heads[index];
            }
            for (i, segment) in self.segments.iter().enumerate() {
                rates[i] = curves[i].flow(out_heads[segment.from] - in_heads[segment.to]);
            }
            let imbalance = |rates: &[f64]| {
                DVector::from_iterator(
                    junctions.len(),
                    junctions.iter().map(|&i| {
                        let node = &self.boundaries[i];
                        sum_rates(&node.outgoing, rates) - sum_rates(&node.incoming, rates)
                    }),
                )
            };
            let residual = imbalance(&rates);
            if residual.amax() <= 1e-8 {
                break;
            }
            if junctions.len() > 64 {
                continue;
            }
            // A joint Newton step avoids slow convergence when two junctions are
            // coupled by parallel, low-resistance branches or a capped outlet.
            let mut jacobian = DMatrix::identity(junctions.len(), junctions.len()) * 1e-10;
            for (i, segment) in self.segments.iter().enumerate() {
                let rate = rates[i];
                let curve = curves[i];
                if rate <= 0.0 || rate >= curve.limit {
                    continue;
                }
                let slope = 1.0 / (2.0 * curve.resistance * rate);
                let from = indexes.get(&segment.from).copied();
                let to = indexes.get(&segment.to).copied();
                if let Some(row) = from {
                    jacobian[(row, row)] += slope;
                }
                if let Some(row) = to {
                    jacobian[(row, row)] += slope;
                }
                if let (Some(a), Some(b)) = (from, to) {
                    jacobian[(a, b)] -= slope;
                    jacobian[(b, a)] -= slope;
                }
            }
            let Some(direction) = jacobian.lu().solve(&(-&residual)) else {
                continue;
            };
            let original: Vec<_> = junctions.iter().map(|&i| in_heads[i]).collect();
            let mut trial = vec![0.0; rates.len()];
            let mut accepted = false;
            for attempt in 0..10 {
                let fraction = 0.5_f64.powi(attempt);
                for (row, &i) in junctions.iter().enumerate() {
                    let head = (original[row] + fraction * direction[row]).clamp(lower, upper);
                    in_heads[i] = head;
                    out_heads[i] = head;
                }
                for (i, segment) in self.segments.iter().enumerate() {
                    trial[i] = curves[i].flow(out_heads[segment.from] - in_heads[segment.to]);
                }
                if imbalance(&trial).norm_squared() < residual.norm_squared() {
                    rates.copy_from_slice(&trial);
                    accepted = true;
                    break;
                }
            }
            if !accepted {
                for (row, &i) in junctions.iter().enumerate() {
                    in_heads[i] = original[row];
                    out_heads[i] = original[row];
                }
            }
        }
        // Back off only imbalanced components if the fixed iteration budget was exhausted.
        let mut pending: Vec<_> = self
            .boundaries
            .iter()
            .enumerate()
            .filter_map(|(i, node)| {
                (matches!(node.boundary, Boundary::Junction)
                    && (sum_rates(&node.incoming, &rates) - sum_rates(&node.outgoing, &rates))
                        .abs()
                        > 1e-8)
                    .then_some(i)
            })
            .collect();
        while let Some(index) = pending.pop() {
            let node = &self.boundaries[index];
            for &e in node.incoming.iter().chain(&node.outgoing) {
                if rates[e] == 0.0 {
                    continue;
                }
                rates[e] = 0.0;
                let segment = &self.segments[e];
                let other = if segment.from == index {
                    segment.to
                } else {
                    segment.from
                };
                if matches!(self.boundaries[other].boundary, Boundary::Junction) {
                    pending.push(other);
                }
            }
        }
        rates
    }

    pub fn solve(
        &mut self,
        dt: f64,
        command: impl Fn(&str, &str) -> f64,
        storage: impl Fn(&str) -> Storage,
        filler_limit: impl Fn(&str) -> f64,
    ) -> BTreeMap<String, PortFlows> {
        if dt > 0.0 {
            for (id, service) in &mut self.services {
                let reset = command(id, "reset_fault") > 0.0 || command(id, "replace") > 0.0;
                if reset && !service.reset_held {
                    service.load_mg = 0.0;
                }
                service.reset_held = reset;
            }
        }
        let mut in_heads = Vec::new();
        let mut out_heads = Vec::new();
        for node in &self.boundaries {
            let (in_head, out_head) = match node.boundary {
                Boundary::Source { head, .. } | Boundary::Sink { head, .. } => (head, head),
                Boundary::Tank {
                    bottom,
                    depth,
                    inlet_height,
                } => {
                    let tank = storage(&node.id);
                    let surface = bottom + depth * (tank.volume_l / tank.capacity_l);
                    (surface.max(inlet_height), surface)
                }
                Boundary::Junction => (0.0, 0.0),
            };
            in_heads.push(in_head);
            out_heads.push(out_head);
        }

        let mut curves: Vec<_> = self
            .segments
            .iter()
            .map(|s| {
                let source_cap = match self.boundaries[s.from].boundary {
                    Boundary::Source { supply, .. } => supply.rate_l_s,
                    _ => f64::INFINITY,
                };
                let sink_cap = match self.boundaries[s.to].boundary {
                    Boundary::Sink {
                        max_rate,
                        container,
                        ..
                    } if container => max_rate.min(filler_limit(&self.boundaries[s.to].id)),
                    Boundary::Sink { max_rate, .. } => max_rate,
                    _ => f64::INFINITY,
                };
                let mut curve = Curve {
                    boost: 0.0,
                    resistance: s.pipe_resistance,
                    limit: source_cap.min(sink_cap),
                };
                for device in &s.devices {
                    let local = Self::device_curve(device, &self.services, &command);
                    curve.boost += local.boost;
                    curve.resistance += local.resistance;
                    curve.limit = curve.limit.min(local.limit);
                }
                curve
            })
            .collect();

        let mut rates = Vec::new();
        for _ in 0..(self.boundaries.len() * 2 + 32) {
            rates = self.calculate_rates(&curves, &mut in_heads, &mut out_heads);
            let mut changed = false;
            for node in &self.boundaries {
                if !matches!(node.boundary, Boundary::Tank { .. }) {
                    continue;
                }
                let tank = storage(&node.id);
                let inlet = sum_rates(&node.incoming, &rates);
                let outlet = sum_rates(&node.outgoing, &rates);
                for (edges, actual, limit) in [
                    (
                        &node.outgoing,
                        outlet,
                        inlet + volume_rate(tank.volume_l, dt),
                    ),
                    (
                        &node.incoming,
                        inlet,
                        outlet + volume_rate(tank.capacity_l - tank.volume_l, dt),
                    ),
                ] {
                    if let Some(&e) = edges.first()
                        && actual > limit + 1e-12
                    {
                        curves[e].limit = limit;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
            // A finite budget also handles poorly conditioned multi-junction networks.
            rates.fill(0.0);
        }
        let mut transfer = BTreeMap::new();
        if !self.exchangers.is_empty() {
            let mut inputs = BTreeMap::new();
            self.transport(
                dt,
                &rates,
                &out_heads,
                &command,
                &storage,
                &BTreeMap::new(),
                &mut inputs,
                false,
            );
            for (id, &ua) in &self.exchangers {
                let [a, b] = inputs.get(id).copied().unwrap_or_default();
                let c_min = HEAT_CAPACITY * a.rate_l_s.min(b.rate_l_s);
                if c_min > 0.0 {
                    let c_max = HEAT_CAPACITY * a.rate_l_s.max(b.rate_l_s);
                    let ratio = c_min / c_max;
                    let ntu = ua / c_min;
                    let effectiveness = if (1.0 - ratio).abs() < 1e-8 {
                        ntu / (1.0 + ntu)
                    } else {
                        let term = (-ntu * (1.0 - ratio)).exp();
                        (1.0 - term) / (1.0 - ratio * term)
                    };
                    transfer.insert(
                        id.clone(),
                        effectiveness * c_min * (a.temperature_c - b.temperature_c),
                    );
                }
            }
        }
        self.transport(
            dt,
            &rates,
            &out_heads,
            &command,
            &storage,
            &transfer,
            &mut BTreeMap::new(),
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn transport(
        &mut self,
        dt: f64,
        rates: &[f64],
        out_heads: &[f64],
        command: &impl Fn(&str, &str) -> f64,
        storage: &impl Fn(&str) -> Storage,
        transfer: &BTreeMap<String, f64>,
        exchanger_inputs: &mut BTreeMap<String, [Flow; 2]>,
        publish: bool,
    ) -> BTreeMap<String, PortFlows> {
        let mut ports = BTreeMap::<String, PortFlows>::new();
        for (s, &rate_l_s) in self.segments.iter().zip(rates) {
            let origin = &self.boundaries[s.from];
            let origin_ports = ports.entry(origin.id.clone()).or_default();
            let contents = match origin.boundary {
                Boundary::Source { supply, .. } => supply,
                Boundary::Tank { .. } => {
                    let tank = storage(&origin.id);
                    if tank.volume_l > 0.0 || origin_ports.inlet.rate_l_s == 0.0 {
                        Flow {
                            temperature_c: tank.temperature_c,
                            dirt_mg_l: tank.dirt_mg_l,
                            ..Flow::default()
                        }
                    } else {
                        origin_ports.inlet
                    }
                }
                _ => origin_ports.inlet,
            };
            let mut flow = Flow {
                rate_l_s,
                ..contents
            };
            if s.from_port == 0 {
                origin_ports.outlet = flow;
            } else {
                origin_ports.second_outlet = flow;
            }
            let mut head = out_heads[s.from];
            for device in &s.devices {
                head -= device.preceding_pipe * rate_l_s.powi(2);
                let local = Self::device_curve(device, &self.services, command);
                let drop = local.resistance * rate_l_s.powi(2);
                let entry = ports.entry(device.id.clone()).or_default();
                if device.lane == 0 {
                    entry.inlet = flow;
                    entry.pressure_kpa = (head - device.elevation) * KPA_PER_M;
                    entry.pressure_drop_kpa = drop * KPA_PER_M;
                }
                let power = match device.kind {
                    PartKind::WaterFilter {
                        capacity_mg,
                        efficiency_pct,
                        ..
                    } if rate_l_s > 0.0 => {
                        let state = self.services.get_mut(&device.id).unwrap();
                        let throughput = dt * rate_l_s;
                        let requested = flow.dirt_mg_l * (f64::from(efficiency_pct) / 100.0);
                        let capacity = (f64::from(capacity_mg) - state.load_mg).max(0.0);
                        let captured = requested.min(volume_rate(capacity, throughput));
                        flow.dirt_mg_l -= captured;
                        if publish {
                            state.load_mg += captured * throughput;
                        }
                        0.0
                    }
                    PartKind::WaterPump { dirt_limit_mg, .. } if publish && rate_l_s > 0.0 => {
                        let state = self.services.get_mut(&device.id).unwrap();
                        let limit = f64::from(dirt_limit_mg);
                        let previous = state.load_mg;
                        state.load_mg = (previous + rate_l_s * dt * flow.dirt_mg_l).min(limit);
                        if previous < limit && state.load_mg >= limit {
                            state.failures = state.failures.saturating_add(1);
                        }
                        0.0
                    }
                    PartKind::WaterHeater { max_power_w, .. } if rate_l_s > 0.0 => {
                        let requested =
                            f64::from(max_power_w) * command(&device.id, "power_pct") / 100.0;
                        // This single-phase water model limits heat delivery at 95 °C.
                        let heating = requested
                            .min((95.0 - flow.temperature_c).max(0.0) * HEAT_CAPACITY * rate_l_s);
                        flow.temperature_c += heating / (HEAT_CAPACITY * rate_l_s);
                        heating
                    }
                    PartKind::WaterHeatExchanger { .. } => {
                        if !publish {
                            exchanger_inputs.entry(device.id.clone()).or_default()
                                [device.lane as usize] = flow;
                        }
                        let heat = transfer.get(&device.id).copied().unwrap_or(0.0);
                        if rate_l_s > 0.0 {
                            let into_lane = if device.lane == 0 { -heat } else { heat };
                            flow.temperature_c += into_lane / (HEAT_CAPACITY * rate_l_s);
                        }
                        heat
                    }
                    _ => 0.0,
                };
                entry.power_w = power;
                if device.lane == 0 {
                    entry.outlet = flow;
                } else {
                    entry.second_outlet = flow;
                }
                head += local.boost - drop;
            }
            let to = &self.boundaries[s.to].id;
            let destination = ports.entry(to.clone()).or_default();
            destination.inlet.mix(flow);
            destination.pressure_kpa =
                (head - s.final_pipe * rate_l_s.powi(2) - out_heads[s.to]) * KPA_PER_M;
        }
        ports
    }

    pub fn service_values(&self, id: &str) -> (f64, i32) {
        self.services
            .get(id)
            .map_or((0.0, 0), |s| (s.load_mg, s.failures))
    }
}

fn sum_rates(edges: &[usize], rates: &[f64]) -> f64 {
    edges.iter().map(|&i| rates[i]).sum()
}

pub(super) fn volume_rate(volume_l: f64, dt: f64) -> f64 {
    if dt > 0.0 {
        volume_l / dt
    } else if volume_l > 0.0 {
        f64::INFINITY
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pump_and_system_curves_balance_at_the_calculated_flow() {
        let curve = Curve {
            boost: 1.0,
            resistance: 10.0,
            limit: 1.0,
        };
        assert!((curve.flow(-0.5) - (0.5_f64 / 10.0).sqrt()).abs() < 1e-12);
        assert_eq!(curve.flow(-1.1), 0.0);
    }
}
