use super::*;
use crate::production::{self, Equipment, Machine, Robot, RobotStep, StaffRole};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use rapier3d::parry::bounding_volume::BoundingVolume;

const MACHINE_HALF_WIDTH: f32 = 0.55;
const TRANSFER_SPEED: f32 = 1.6;
const TRANSFER_CLEARANCE: f32 = 0.30;
const GRIP_ROTATION_SPEED: f32 = 4.6;

#[derive(Default)]
pub(super) struct Cells {
    equipment: BTreeMap<String, Mechanism>,
    machines: BTreeMap<String, Center>,
    robots: BTreeMap<String, Arm>,
}

struct Mechanism {
    body: Option<RigidBodyHandle>,
    transport: Option<ColliderHandle>,
    position: f32,
}

struct Center {
    config: Machine,
    frame: Isometry<Real>,
    failure_rng: ChaCha8Rng,
    quality_rng: ChaCha8Rng,
    job: Option<Job>,
    fault_tick: Option<u64>,
    repair: u64,
    technician: Option<String>,
    running: bool,
    emergency: bool,
    last_start: bool,
    last_reset: bool,
    cycle_ticks: u64,
    repair_ticks: u64,
}

struct Job {
    parcel: String,
    elapsed: u64,
    fail_at: Option<u64>,
    phase: JobPhase,
}

enum JobPhase {
    Loading(Transfer),
    Machining,
    Unloading { scrap: bool, transfer: Transfer },
    ReadyForRobot { destination: Isometry<Real> },
}

#[derive(Clone, Copy)]
struct Transfer {
    waypoints: [Isometry<Real>; 3],
    next: usize,
}

impl Transfer {
    fn new(from: Isometry<Real>, to: Isometry<Real>) -> Self {
        let height = from.translation.y.max(to.translation.y) + TRANSFER_CLEARANCE;
        let mut lift = from;
        lift.translation.y = height;
        let mut above = to;
        above.translation.y = height;
        Self {
            waypoints: [lift, above, to],
            next: 0,
        }
    }

    fn destination(&self) -> &Isometry<Real> {
        &self.waypoints[2]
    }

    fn next_pose(&mut self, current: Isometry<Real>, speed: f32) -> Option<Isometry<Real>> {
        while let Some(target) = self.waypoints.get(self.next) {
            let delta = target.translation.vector - current.translation.vector;
            let distance = delta.norm();
            let angle = current.rotation.angle_to(&target.rotation);
            if distance <= 0.00001 && angle <= 0.0001 {
                self.next += 1;
                continue;
            }
            let mut next = *target;
            if distance > speed * DT {
                next.translation.vector =
                    current.translation.vector + delta * (speed * DT / distance);
            }
            if angle > GRIP_ROTATION_SPEED * DT {
                next.rotation = current
                    .rotation
                    .slerp(&target.rotation, GRIP_ROTATION_SPEED * DT / angle);
            }
            return Some(next);
        }
        None
    }

    fn advance(&mut self, body: &mut RigidBody) -> bool {
        if let Some(next) = self.next_pose(*body.position(), TRANSFER_SPEED) {
            body.set_next_kinematic_position(next);
            return false;
        }
        true
    }
}

struct Grip {
    parcel: String,
    offset: Isometry<Real>,
}

struct RobotUnload {
    machine: String,
    parcel: String,
    grip_height: f32,
    phase: RobotUnloadPhase,
}

enum RobotUnloadPhase {
    Approach { travel: Transfer, carry: Transfer },
    Carry(Transfer),
}

#[derive(Default)]
struct Arm {
    at: [f32; 3],
    angle: f32,
    cursor: usize,
    wait: u64,
    holding: Option<Grip>,
    unload: Option<RobotUnload>,
    running: bool,
    done: bool,
    last_start: bool,
    last_reset: bool,
}

impl Simulation {
    pub(super) fn cell_insert(&mut self, part: &Part) {
        let base = pose(part.position, part.yaw);
        match &part.kind {
            PartKind::Machine { machine } => {
                self.world.colliders.insert(
                    cuboid([2.0 * MACHINE_HALF_WIDTH, 0.6, 2.0 * MACHINE_HALF_WIDTH])
                        .position(base * Isometry::translation(0.0, -0.3, 0.0)),
                );
                let ports = [machine.intake, machine.outlet, machine.reject];
                let supports = if machine.unload_robot.is_some() {
                    &ports[..1]
                } else {
                    &ports[..]
                };
                for &[x, y, z] in supports {
                    self.world.colliders.insert(
                        cuboid([0.45, 0.038, 0.45])
                            .position(base * Isometry::translation(x, y - 0.024, z))
                            .friction(0.20),
                    );
                }
                self.cells.machines.insert(
                    part.id.clone(),
                    Center {
                        config: machine.clone(),
                        frame: base,
                        job: None,
                        fault_tick: None,
                        repair: 0,
                        technician: None,
                        failure_rng: production::stream(self.project.production.seed, &part.id, 1),
                        quality_rng: production::stream(self.project.production.seed, &part.id, 2),
                        running: false,
                        emergency: false,
                        last_start: false,
                        last_reset: false,
                        cycle_ticks: production::ticks(machine.seconds).unwrap(),
                        repair_ticks: production::ticks(machine.repair_seconds).unwrap(),
                    },
                );
            }
            PartKind::Staff { staff } => {
                let duty = self.staff_on_duty(staff);
                self.publish(&part.id, "on_duty", Value::Bool(duty));
            }
            PartKind::Robot { .. } => {
                self.cells.robots.insert(part.id.clone(), Arm::default());
            }
            PartKind::Equipment {
                equipment, size, ..
            } => {
                use Equipment::*;
                let mut mechanism = Mechanism {
                    body: None,
                    transport: None,
                    position: 0.0,
                };
                match equipment {
                    Roller | Scale | Turntable | ChainTransfer => {
                        let plate = cuboid(*size)
                            .friction(1.2)
                            .active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS);
                        let collider = if *equipment == Turntable {
                            let body = self.world.bodies.insert(
                                RigidBodyBuilder::kinematic_position_based().position(base),
                            );
                            mechanism.body = Some(body);
                            self.world.colliders.insert_with_parent(
                                plate,
                                body,
                                &mut self.world.bodies,
                            )
                        } else {
                            self.world.colliders.insert(plate.position(base))
                        };
                        mechanism.transport = Some(collider);
                        if *equipment == ChainTransfer {
                            let body = self.world.bodies.insert(
                                RigidBodyBuilder::kinematic_position_based().position(
                                    base * Isometry::translation(0.0, size[1] * 0.5 - 0.024, 0.0),
                                ),
                            );
                            self.world.colliders.insert_with_parent(
                                cuboid([size[0] * 0.96, 0.04, size[2] * 0.96])
                                    .friction(1.2)
                                    .active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS),
                                body,
                                &mut self.world.bodies,
                            );
                            mechanism.body = Some(body);
                        }
                    }
                    Fence => {
                        self.world.colliders.insert(cuboid(*size).position(base));
                    }
                    StopBlade | SafetyDoor => {
                        let offset = if *equipment == StopBlade {
                            -size[1]
                        } else {
                            0.0
                        };
                        let body = self.world.bodies.insert(
                            RigidBodyBuilder::kinematic_position_based()
                                .position(base * Isometry::translation(0.0, offset, 0.0)),
                        );
                        self.world.colliders.insert_with_parent(
                            cuboid(*size),
                            body,
                            &mut self.world.bodies,
                        );
                        mechanism.body = Some(body);
                    }
                    _ => {}
                }
                self.cells.equipment.insert(part.id.clone(), mechanism);
            }
            _ => {}
        }
    }

    pub(super) fn cell_tick(&mut self) {
        // Moving equipment, staffing, machine custody, then robot custody.
        let mut cells = std::mem::take(&mut self.cells);
        let parts = self.cell_parts.clone();
        let mut operators = HashSet::new();
        let mut technicians = BTreeMap::new();
        for part in parts.iter() {
            match &part.kind {
                PartKind::Equipment {
                    equipment, size, ..
                } => {
                    let device = cells.equipment.get_mut(&part.id).unwrap();
                    self.move_equipment(part, *equipment, *size, device);
                }
                PartKind::Staff { staff } => {
                    let duty = self.staff_on_duty(staff);
                    self.publish(&part.id, "on_duty", Value::Bool(duty));
                    self.publish(&part.id, "busy", Value::Bool(false));
                    if duty {
                        match staff.role {
                            StaffRole::Operator => {
                                operators.insert(staff.area.clone());
                            }
                            StaffRole::Technician => {
                                technicians.insert(part.id.clone(), staff.area.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let mut assigned = HashSet::new();
        let mut queue: Vec<_> = cells
            .machines
            .iter_mut()
            .filter_map(|(id, center)| center.fault_tick.map(|tick| (tick, id, center)))
            .collect();
        queue.sort_by(|(a, aid, _), (b, bid, _)| (a, aid).cmp(&(b, bid)));
        for (_, _, center) in &mut queue {
            if center
                .technician
                .as_ref()
                .is_some_and(|id| technicians.get(id) == Some(&center.config.area))
            {
                assigned.insert(center.technician.clone().unwrap());
            } else {
                center.technician = None;
            }
        }
        for (_, _, center) in queue {
            if center.technician.is_none() {
                center.technician = technicians
                    .iter()
                    .find(|(id, area)| **area == center.config.area && !assigned.contains(*id))
                    .map(|(id, _)| id.clone());
                if let Some(id) = &center.technician {
                    assigned.insert(id.clone());
                }
            }
        }
        for part in parts.iter() {
            if let Some(center) = cells.machines.get_mut(&part.id) {
                self.tick_center(part, center, operators.contains(&center.config.area));
            }
        }
        for part in parts.iter() {
            if let PartKind::Robot { robot } = &part.kind {
                let arm = cells.robots.get_mut(&part.id).unwrap();
                if cells
                    .machines
                    .values()
                    .any(|center| center.config.unload_robot.as_deref() == Some(part.id.as_str()))
                {
                    self.tick_unload_arm(part, robot, arm, &mut cells.machines);
                } else {
                    self.tick_arm(part, robot, arm);
                }
            }
        }
        for part in parts.iter() {
            if let PartKind::Staff { staff } = &part.kind {
                let busy = assigned.contains(&part.id)
                    || staff.role == StaffRole::Operator
                        && flag(&self.tags, &part.id, "on_duty")
                        && cells.machines.iter().any(|(id, center)| {
                            center.config.area == staff.area
                                && center.running
                                && flag(&self.tags, id, "busy")
                                && !flag(&self.tags, id, "fault")
                                && !flag(&self.tags, id, "interlock")
                        });
                self.publish(&part.id, "busy", Value::Bool(busy));
            }
        }
        self.cells = cells;
    }

    fn staff_on_duty(&self, staff: &production::Staff) -> bool {
        staff.calendar.as_ref().is_some_and(|id| {
            self.project
                .production
                .clock(self.tick)
                .is_some_and(|now| self.project.production.calendars[id].on_duty(now))
        })
    }

    fn move_equipment(
        &mut self,
        part: &Part,
        equipment: Equipment,
        size: [f32; 3],
        device: &mut Mechanism,
    ) {
        use Equipment::*;
        let id = &part.id;
        let base = pose(part.position, part.yaw);
        let direction = |plus, minus| {
            f32::from(u8::from(flag(&self.tags, id, plus)))
                - f32::from(u8::from(flag(&self.tags, id, minus)))
        };
        if let Some(collider) = device.transport {
            let speed = if equipment == Scale { 0.6 } else { 0.45 };
            let mut longitudinal = direction("forward", "reverse") * speed;
            if equipment == Roller
                && !flag(&self.tags, id, "forward")
                && !flag(&self.tags, id, "reverse")
            {
                longitudinal = 0.08 * number(&self.tags, id, "speed_v");
            }
            let mut travel = longitudinal;
            let mut rotation = base.rotation;
            if equipment == Turntable {
                let target = if flag(&self.tags, id, "turn") {
                    90.0
                } else {
                    0.0
                };
                device.position = approach(device.position, target, 0.7_f32.to_degrees() * DT);
                let relative = pose([0.0; 3], device.position);
                rotation *= relative.rotation;
                self.world.bodies[device.body.unwrap()]
                    .set_next_kinematic_position(base * relative);
                self.publish(id, "angle", Value::Float(device.position));
                self.publish(id, "limit_0", Value::Bool(device.position == 0.0));
                self.publish(id, "limit_90", Value::Bool(device.position == 90.0));
                if device.position != 0.0 && device.position != 90.0 {
                    travel = 0.0;
                }
            } else if equipment == ChainTransfer {
                let lateral = direction("right", "left");
                device.position = approach(
                    device.position,
                    if lateral == 0.0 { 0.0 } else { 0.04 },
                    0.15 * DT,
                );
                let chain = device.body.unwrap();
                self.world.bodies[chain].set_next_kinematic_position(
                    base * Isometry::translation(0.0, size[1] * 0.5 - 0.024 + device.position, 0.0),
                );
                let chain_collider = self.world.bodies[chain].colliders()[0];
                self.set_surface(chain_collider, rotation * Vector::z() * lateral * speed);
                if device.position > 0.004 {
                    travel = 0.0;
                }
                self.publish(id, "raised", Value::Bool(device.position == 0.04));
                self.publish(id, "position", Value::Float(device.position));
            }
            self.set_surface(collider, rotation * Vector::x() * travel);
        } else if equipment == StopBlade {
            let target = if flag(&self.tags, id, "raise") {
                size[1]
            } else {
                0.0
            };
            device.position = approach(device.position, target, 0.8 * DT);
            self.world.bodies[device.body.unwrap()].set_next_kinematic_position(
                base * Isometry::translation(0.0, device.position - size[1], 0.0),
            );
            self.publish(id, "position", Value::Float(device.position));
            self.publish(id, "raised", Value::Bool(device.position == size[1]));
        } else if equipment == SafetyDoor {
            let target = if flag(&self.tags, id, "open") {
                90.0
            } else {
                0.0
            };
            let next_angle = approach(device.position, target, 90.0 * DT);
            let next_pose = base * door_transform(size[0], next_angle);
            let leaf = Cuboid::new(Vector::from(size) * 0.5);
            let obstructed = next_angle != device.position && !self.free(&next_pose, &leaf, None);
            if !obstructed {
                device.position = next_angle;
            }
            self.world.bodies[device.body.unwrap()]
                .set_next_kinematic_position(base * door_transform(size[0], device.position));
            self.publish(id, "obstructed", Value::Bool(obstructed));
            self.publish(id, "closed", Value::Bool(device.position == 0.0));
            self.publish(id, "angle", Value::Float(device.position));
        }
    }

    fn set_surface(&mut self, collider: ColliderHandle, velocity: Vector<Real>) {
        let previous = self
            .surfaces
            .velocities
            .insert(collider, velocity)
            .unwrap_or_default();
        if (previous.norm_squared() == 0.0) != (velocity.norm_squared() == 0.0)
            || previous.dot(&velocity) < 0.0
        {
            for parcel in self.parcels.values() {
                if parcel.owner.is_none() {
                    self.world.bodies[parcel.body].wake_up(true);
                }
            }
        }
    }

    pub(super) fn cell_sensors(&mut self) {
        let parts = self.cell_parts.clone();
        for part in parts.iter() {
            let PartKind::Equipment {
                equipment,
                size,
                scale_capacity,
            } = part.kind
            else {
                continue;
            };
            let id = &part.id;
            let base = pose(part.position, part.yaw);
            match equipment {
                Equipment::Capacitive => {
                    let hit = self.beam(&base, 0.2);
                    self.publish(id, "detected", Value::Bool(hit.is_some()));
                    self.publish(id, "distance", Value::Float(hit.unwrap_or(0.2)));
                    self.publish(id, "distance_v", Value::Float(50.0 * hit.unwrap_or(0.2)));
                }
                Equipment::LightArray => {
                    let mut mask = 0u8;
                    for (index, role) in [
                        "beam_1", "beam_2", "beam_3", "beam_4", "beam_5", "beam_6", "beam_7",
                        "beam_8",
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let height = size[1] * (index as f32 / 7.0 - 0.5);
                        let beam = base * Isometry::translation(-size[0] * 0.5, height, 0.0);
                        let blocked = self.beam(&beam, size[0]).is_some();
                        if blocked {
                            mask |= 1 << index;
                        }
                        self.publish(id, role, Value::Bool(blocked));
                    }
                    self.publish(id, "aligned", Value::Bool(true));
                    self.publish(id, "mask", Value::Int(i32::from(mask)));
                    self.publish(id, "value_v", Value::Float(mask.count_ones() as f32 * 1.25));
                }
                Equipment::Scale => {
                    let device = &self.cells.equipment[id];
                    let surface = device.transport.unwrap();
                    let weight = self
                        .parcels
                        .values()
                        .filter(|parcel| {
                            parcel.owner.is_none()
                                && self
                                    .world
                                    .narrow
                                    .contact_pair(surface, parcel.collider)
                                    .is_some_and(|pair| pair.has_any_active_contact)
                        })
                        .map(|parcel| self.world.colliders[parcel.collider].mass())
                        .sum::<f32>()
                        .min(scale_capacity as f32);
                    self.publish(id, "weight_kg", Value::Float(weight));
                    self.publish(
                        id,
                        "weight_v",
                        Value::Float(10.0 * weight / scale_capacity as f32),
                    );
                }
                Equipment::Turntable => {
                    let angle = self.cells.equipment[id].position;
                    let transform = base * pose([0.0; 3], angle);
                    let half = size[0] * 0.42;
                    let sensor_mount = |x| {
                        transform
                            * Isometry::new(
                                vector![x, size[1] * 0.5 + 0.001, 0.0],
                                vector![0.0, 0.0, std::f32::consts::FRAC_PI_2],
                            )
                    };
                    let front = self.beam(&sensor_mount(half), 0.10);
                    let back = self.beam(&sensor_mount(-half), 0.10);
                    self.publish(id, "front", Value::Bool(front.is_some()));
                    self.publish(id, "back", Value::Bool(back.is_some()));
                }
                _ => {}
            }
        }
    }

    fn beam(&self, base: &Isometry<Real>, range: f32) -> Option<f32> {
        let ray = Ray::new(
            Point::from(base.translation.vector),
            base.rotation * Vector::x(),
        );
        self.world
            .queries
            .cast_ray(
                &self.world.bodies,
                &self.world.colliders,
                &ray,
                range,
                true,
                QueryFilter::default()
                    .predicate(&|collider, _| self.parcel_colliders.contains(&collider)),
            )
            .map(|(_, distance)| distance)
    }

    pub(super) fn publish(&mut self, id: &str, role: &str, value: Value) {
        self.tags.get_mut(&tag_id(id, role)).unwrap().value = value;
    }

    pub(super) fn change_count(&mut self, id: &str, role: &str) {
        let state = self.tags.get_mut(&tag_id(id, role)).unwrap();
        if let Value::Int(n) = state.value {
            state.value = Value::Int(n.saturating_add(1));
        }
    }

    fn nearest(&self, at: Isometry<Real>, within: Vector<Real>) -> Option<String> {
        self.parcels
            .iter()
            .filter(|(_, parcel)| parcel.owner.is_none())
            .filter_map(|(id, parcel)| {
                let delta = at.inverse_transform_point(&Point::from(
                    *self.world.bodies[parcel.body].translation(),
                ));
                (0..3)
                    .all(|axis| delta[axis].abs() <= within[axis])
                    .then_some((id, delta.coords.norm_squared()))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id.clone())
    }

    pub(super) fn free(
        &self,
        at: &Isometry<Real>,
        shape: &dyn Shape,
        except: Option<&str>,
    ) -> bool {
        let bounds = shape.compute_aabb(at);
        self.parcels.iter().all(|(id, parcel)| {
            if Some(id.as_str()) == except {
                return true;
            }
            let collider = &self.world.colliders[parcel.collider];
            let other = self.world.bodies[parcel.body].position();
            !bounds.intersects(&collider.shape().compute_aabb(other))
                || !rapier3d::parry::query::intersection_test(at, shape, other, collider.shape())
                    .unwrap_or(true)
        })
    }

    fn hold(&mut self, parcel: &str, owner: &str) {
        let parcel = self.parcels.get_mut(parcel).unwrap();
        parcel.owner = Some(owner.to_owned());
        self.parcel_colliders.remove(&parcel.collider);
        self.world.colliders[parcel.collider].set_enabled(false);
        let body = &mut self.world.bodies[parcel.body];
        body.set_body_type(RigidBodyType::KinematicPositionBased, true);
        body.set_linvel(Vector::zeros(), true);
        body.set_angvel(Vector::zeros(), true);
        body.set_next_kinematic_position(*body.position());
    }

    fn release(&mut self, id: &str) {
        let parcel = self.parcels.get_mut(id).unwrap();
        parcel.owner = None;
        self.parcel_colliders.insert(parcel.collider);
        self.world.colliders[parcel.collider].set_enabled(true);
        let body = &mut self.world.bodies[parcel.body];
        body.set_body_type(RigidBodyType::Dynamic, true);
        body.set_linvel(Vector::zeros(), true);
        body.set_angvel(Vector::zeros(), true);
    }

    fn tick_center(&mut self, part: &Part, center: &mut Center, operator: bool) {
        let id = &part.id;
        let base = center.frame;
        let configuration = &center.config;
        let start = flag(&self.tags, id, "start");
        let reset = flag(&self.tags, id, "reset");
        let emergency = flag(&self.tags, id, "emergency");
        if reset && !center.last_reset && !emergency {
            center.emergency = false;
            center.running = false;
        }
        if emergency {
            center.emergency = true;
        }
        if flag(&self.tags, id, "stop") || center.emergency {
            center.running = false;
        } else if start && !center.last_start && !reset {
            center.running = true;
        }
        center.last_start = start;
        center.last_reset = reset;
        let interlock = configuration
            .door
            .as_ref()
            .is_some_and(|door| !flag(&self.tags, door, "closed"));
        let repairing = center.technician.is_some();
        if center.fault_tick.is_some() && repairing {
            center.repair += 1;
            if center.repair == center.repair_ticks {
                center.fault_tick = None;
                center.technician = None;
            }
        }

        let mut input_error = false;
        let mut blocked = false;
        let active =
            center.running && operator && !interlock && center.fault_tick.is_none() && !repairing;
        if active {
            if let Some(job) = &mut center.job {
                let item = &self.parcels[&job.parcel];
                match &mut job.phase {
                    JobPhase::Loading(travel) => {
                        if travel.advance(&mut self.world.bodies[item.body]) {
                            job.phase = JobPhase::Machining;
                        }
                    }
                    JobPhase::Machining if job.fail_at == Some(job.elapsed) => {
                        center.fault_tick = Some(self.tick);
                        center.repair = 0;
                        job.fail_at = None;
                        self.change_count(id, "failure_count");
                    }
                    JobPhase::Machining => {
                        job.elapsed += 1;
                        if job.elapsed == center.cycle_ticks {
                            let scrap = center.quality_rng.gen_range(0.0..100.0)
                                < configuration.scrap_percent;
                            let product = if scrap {
                                configuration
                                    .scrap_product
                                    .as_ref()
                                    .unwrap_or(&configuration.output)
                            } else {
                                &configuration.output
                            };
                            let port = if scrap {
                                configuration.reject
                            } else {
                                configuration.outlet
                            };
                            let current = *self.world.bodies[item.body].position();
                            let size = self.finish_machining(&job.parcel, product, scrap);
                            let [x, y, z] = port;
                            let destination =
                                base * Isometry::translation(x, y + size[1] * 0.5 + 0.005, z);
                            job.phase = if configuration.unload_robot.is_some() {
                                JobPhase::ReadyForRobot { destination }
                            } else {
                                JobPhase::Unloading {
                                    scrap,
                                    transfer: Transfer::new(current, destination),
                                }
                            };
                        }
                    }
                    JobPhase::Unloading { scrap, transfer } => {
                        let shape = self.world.colliders[item.collider].shape();
                        blocked = !self.free(transfer.destination(), shape, Some(&job.parcel));
                        if !blocked && transfer.advance(&mut self.world.bodies[item.body]) {
                            self.release(&job.parcel);
                            self.change_count(
                                id,
                                if *scrap { "scrap_count" } else { "good_count" },
                            );
                            center.job = None;
                        }
                    }
                    JobPhase::ReadyForRobot { .. } => {}
                }
            } else {
                let [x, y, z] = configuration.intake;
                let at = base * Isometry::translation(x, y + 0.15, z);
                if let Some(parcel) = self.nearest(at, vector![0.28, 0.35, 0.28]) {
                    let item = &self.parcels[&parcel];
                    input_error =
                        item.product.as_deref() != Some(configuration.input.as_str()) || item.scrap;
                    if !input_error {
                        let current = *self.world.bodies[item.body].position();
                        let workholding =
                            base * Isometry::translation(0.0, item.size[1] * 0.5 + 0.05, 0.0);
                        self.hold(&parcel, id);
                        let fail_at = (center.failure_rng.gen_range(0.0..100.0)
                            < configuration.failure_percent)
                            .then(|| center.failure_rng.gen_range(0..center.cycle_ticks));
                        center.job = Some(Job {
                            parcel,
                            elapsed: 0,
                            fail_at,
                            phase: JobPhase::Loading(Transfer::new(current, workholding)),
                        });
                    }
                }
            }
        }
        let mut waiting_unload = false;
        if let Some(job) = &center.job {
            let destination = match &job.phase {
                JobPhase::Unloading { transfer, .. } => Some(transfer.destination()),
                JobPhase::ReadyForRobot { destination } => {
                    waiting_unload = true;
                    Some(destination)
                }
                _ => None,
            };
            if let Some(destination) = destination {
                let item = &self.parcels[&job.parcel];
                blocked = !self.free(
                    destination,
                    self.world.colliders[item.collider].shape(),
                    Some(&job.parcel),
                );
            }
        }
        let progress = center.job.as_ref().map_or(0.0, |job| {
            100.0 * job.elapsed as f32 / center.cycle_ticks as f32
        });
        for (role, state) in [
            ("busy", center.job.is_some()),
            ("fault", center.emergency || center.fault_tick.is_some()),
            ("input_error", input_error),
            ("repairing", repairing && center.fault_tick.is_some()),
            ("waiting_operator", center.running && !operator),
            ("waiting_unload", waiting_unload),
            ("interlock", interlock),
            ("blocked", blocked),
        ] {
            self.publish(id, role, Value::Bool(state));
        }
        self.publish(id, "progress", Value::Float(progress));
        self.publish(
            id,
            "repair_progress",
            Value::Float(100.0 * center.repair as f32 / center.repair_ticks as f32),
        );
    }

    fn finish_machining(&mut self, id: &str, product: &str, scrap: bool) -> [f32; 3] {
        let spec = self
            .project
            .product(product)
            .expect("validated machining recipe");
        let size = spec.size;
        let shape = self.parcel_shape(Some(product), size);
        let item = self.parcels.get_mut(id).unwrap();
        let collider = &mut self.world.colliders[item.collider];
        collider.set_shape(shape);
        collider.set_mass(spec.mass);
        item.size = size;
        item.product = Some(product.to_owned());
        item.scrap = scrap;
        size
    }

    #[cfg(any(feature = "studio", feature = "web"))]
    pub(super) fn machine_grips(&self) -> BTreeMap<String, [f32; 3]> {
        self.cells
            .machines
            .iter()
            .filter_map(|(id, center)| {
                let job = center.job.as_ref()?;
                if !matches!(job.phase, JobPhase::Loading(_) | JobPhase::Unloading { .. }) {
                    return None;
                }
                let item = &self.parcels[&job.parcel];
                let at = self.world.bodies[item.body].position();
                let bounds = self.world.colliders[item.collider].shape().compute_aabb(at);
                Some((
                    id.clone(),
                    [at.translation.x, bounds.maxs.y, at.translation.z],
                ))
            })
            .collect()
    }

    fn tick_unload_arm(
        &mut self,
        part: &Part,
        robot: &Robot,
        arm: &mut Arm,
        machines: &mut BTreeMap<String, Center>,
    ) {
        let id = &part.id;
        let base = pose(part.position, part.yaw);
        let start = flag(&self.tags, id, "start");
        let reset = flag(&self.tags, id, "reset");
        if flag(&self.tags, id, "stop") || reset {
            arm.running = false;
        } else if start && !arm.last_start {
            arm.running = true;
        }
        arm.last_start = start;

        let mut moving = false;
        let mut waiting = false;
        let mut out_of_reach = false;
        if arm.running {
            if arm.unload.is_none() {
                for (machine, center) in machines.iter() {
                    if center.config.unload_robot.as_deref() != Some(id.as_str())
                        || !self.unload_permitted(machine, center)
                    {
                        continue;
                    }
                    let Some(job) = &center.job else { continue };
                    let JobPhase::ReadyForRobot { destination } = job.phase else {
                        continue;
                    };

                    let item = &self.parcels[&job.parcel];
                    let current = *self.world.bodies[item.body].position();
                    let shape = self.world.colliders[item.collider].shape();
                    let bounds = shape.compute_aabb(&current);
                    let grip_height = bounds.maxs.y - current.translation.y;
                    let mut pickup = pose(arm.at, arm.angle);
                    let contact =
                        Point::new(current.translation.x, bounds.maxs.y, current.translation.z);
                    pickup.translation.vector = base.inverse_transform_point(&contact).coords;
                    let travel = Transfer::new(pose(arm.at, arm.angle), pickup);
                    let carry = Transfer::new(current, destination);
                    let reachable = travel
                        .waypoints
                        .iter()
                        .all(|at| within_reach(robot, at.translation.vector))
                        && carry.waypoints.iter().all(|at| {
                            let tool =
                                Point::from(at.translation.vector + Vector::y() * grip_height);
                            within_reach(robot, base.inverse_transform_point(&tool).coords)
                        });
                    if !reachable || !outside_work_zone(shape, center.frame, destination) {
                        out_of_reach = true;
                        continue;
                    }
                    arm.unload = Some(RobotUnload {
                        machine: machine.clone(),
                        parcel: job.parcel.clone(),
                        grip_height,
                        phase: RobotUnloadPhase::Approach { travel, carry },
                    });
                    break;
                }
            }

            if let Some(task) = &mut arm.unload {
                let center = machines.get_mut(&task.machine).unwrap();
                let in_machine = center
                    .job
                    .as_ref()
                    .is_some_and(|job| job.parcel == task.parcel);
                waiting = if in_machine {
                    !self.unload_permitted(&task.machine, center)
                } else {
                    center.emergency || flag(&self.tags, &task.machine, "interlock")
                };
                if !waiting {
                    match &mut task.phase {
                        RobotUnloadPhase::Approach { travel, carry } => {
                            if let Some(next) =
                                travel.next_pose(pose(arm.at, arm.angle), robot.speed)
                            {
                                arm.at = next.translation.vector.into();
                                moving = true;
                            } else {
                                // Ownership changes at contact while the existing body stays in place.
                                self.parcels.get_mut(&task.parcel).unwrap().owner =
                                    Some(id.clone());
                                task.phase = RobotUnloadPhase::Carry(*carry);
                            }
                        }
                        RobotUnloadPhase::Carry(travel) => {
                            let item = &self.parcels[&task.parcel];
                            let current = *self.world.bodies[item.body].position();
                            let shape = self.world.colliders[item.collider].shape();
                            let next = travel.next_pose(current, robot.speed);
                            if in_machine && outside_work_zone(shape, center.frame, current) {
                                center.job = None;
                            }
                            if travel.next >= 2 {
                                waiting =
                                    !self.free(travel.destination(), shape, Some(&task.parcel));
                            }
                            if !waiting {
                                if let Some(next) = next {
                                    let tool = Point::from(
                                        next.translation.vector + Vector::y() * task.grip_height,
                                    );
                                    arm.at = base.inverse_transform_point(&tool).coords.into();
                                    self.world.bodies[item.body].set_next_kinematic_position(next);
                                    moving = true;
                                } else {
                                    let scrap = item.scrap;
                                    self.release(&task.parcel);
                                    self.change_count(
                                        &task.machine,
                                        if scrap { "scrap_count" } else { "good_count" },
                                    );
                                    arm.unload = None;
                                }
                            }
                        }
                    }
                }
            } else {
                waiting = true;
            }
        }
        let holding = arm
            .unload
            .as_ref()
            .is_some_and(|task| matches!(task.phase, RobotUnloadPhase::Carry(_)));
        let step = if holding {
            2
        } else {
            i32::from(arm.unload.is_some())
        };
        self.publish(id, "out_of_reach", Value::Bool(out_of_reach));
        self.publish_arm(id, arm, moving, holding, waiting, step);
    }

    fn unload_permitted(&self, machine: &str, center: &Center) -> bool {
        center.running
            && !flag(&self.tags, machine, "interlock")
            && !flag(&self.tags, machine, "fault")
            && !flag(&self.tags, machine, "waiting_operator")
    }

    fn tick_arm(&mut self, part: &Part, robot: &Robot, arm: &mut Arm) {
        let id = &part.id;
        let base = pose(part.position, part.yaw);
        let start = flag(&self.tags, id, "start");
        let reset = flag(&self.tags, id, "reset");
        if reset && !arm.last_reset {
            // Keep the arm and payload where they were if releasing here is obstructed.
            if self.arm_release(arm) {
                *arm = Arm::default();
            }
        }
        if flag(&self.tags, id, "stop") {
            arm.running = false;
        } else if start && !arm.last_start && !arm.done && !reset {
            arm.running = true;
        }
        arm.last_start = start;
        arm.last_reset = reset;

        let mut moving = false;
        let mut waiting = false;
        if arm.running {
            let step = &robot.program[arm.cursor];
            let advance = match *step {
                RobotStep::Move { at } => {
                    let current = Vector::from(arm.at);
                    let target = Vector::from(at);
                    let delta = target - current;
                    let distance = delta.norm();
                    let budget = robot.speed * DT;
                    moving = distance > 0.0;
                    let reached = distance <= budget;
                    arm.at = if reached {
                        at
                    } else {
                        (current + delta * (budget / distance)).into()
                    };
                    reached
                }
                RobotStep::Rotate { degrees } => {
                    moving = degrees != arm.angle;
                    arm.angle = approach(arm.angle, degrees, 4.6_f32.to_degrees() * DT);
                    arm.angle == degrees
                }
                RobotStep::Wait { seconds } => {
                    waiting = true;
                    arm.wait += 1;
                    arm.wait >= production::ticks(seconds).unwrap()
                }
                RobotStep::WaitSignal => {
                    waiting = !flag(&self.tags, id, "signal");
                    !waiting
                }
                RobotStep::Grab => {
                    if arm.holding.is_none() {
                        let tool = base * pose(arm.at, arm.angle);
                        // Contact with the top of the parcel, in a small suction footprint.
                        let candidate = self
                            .parcels
                            .iter()
                            .filter_map(|(id, parcel)| {
                                if parcel.owner.is_some() {
                                    return None;
                                }
                                let body = &self.world.bodies[parcel.body];
                                let bounds = self.world.colliders[parcel.collider]
                                    .shape()
                                    .compute_aabb(body.position());
                                let p = tool.translation.vector;
                                ((bounds.maxs.y - p.y).abs() <= 0.10
                                    && (bounds.mins.x..=bounds.maxs.x).contains(&p.x)
                                    && (bounds.mins.z..=bounds.maxs.z).contains(&p.z))
                                .then_some(id)
                            })
                            .next()
                            .cloned();
                        if let Some(parcel) = candidate {
                            let item = &self.parcels[&parcel];
                            let offset = tool.inverse() * self.world.bodies[item.body].position();
                            self.hold(&parcel, id);
                            arm.holding = Some(Grip { parcel, offset });
                        }
                    }
                    waiting = arm.holding.is_none();
                    !waiting
                }
                RobotStep::Release => {
                    waiting = !self.arm_release(arm);
                    !waiting
                }
            };
            if advance {
                arm.wait = 0;
                arm.cursor += 1;
                if arm.cursor == robot.program.len() {
                    arm.cursor = 0;
                    if !robot.repeat {
                        arm.running = false;
                        arm.done = true;
                    }
                }
            }
        }
        if let Some(grip) = &arm.holding {
            let body = self.parcels[&grip.parcel].body;
            let tool = base * pose(arm.at, arm.angle);
            self.world.bodies[body].set_next_kinematic_position(tool * grip.offset);
        }
        self.publish_arm(
            id,
            arm,
            moving,
            arm.holding.is_some(),
            waiting,
            if arm.done { 0 } else { (arm.cursor + 1) as i32 },
        );
    }

    fn publish_arm(
        &mut self,
        id: &str,
        arm: &Arm,
        moving: bool,
        holding: bool,
        waiting: bool,
        step: i32,
    ) {
        for (role, value) in [
            ("moving", moving),
            ("holding", holding),
            ("waiting", waiting),
            ("done", arm.done),
        ] {
            self.publish(id, role, Value::Bool(value));
        }
        for (role, value) in [
            ("x", arm.at[0]),
            ("y", arm.at[1]),
            ("z", arm.at[2]),
            ("angle", arm.angle),
        ] {
            self.publish(id, role, Value::Float(value));
        }
        self.publish(id, "step", Value::Int(step));
    }

    fn arm_release(&mut self, arm: &mut Arm) -> bool {
        if let Some(grip) = &arm.holding {
            let id = &grip.parcel;
            let parcel = &self.parcels[id];
            if !self.free(
                self.world.bodies[parcel.body].position(),
                self.world.colliders[parcel.collider].shape(),
                Some(id),
            ) {
                return false;
            }
            self.release(id);
            arm.holding = None;
        }
        true
    }
}

fn within_reach(robot: &Robot, at: Vector<Real>) -> bool {
    at.iter()
        .zip(robot.reach())
        .all(|(coordinate, limit)| (-0.0001..=limit + 0.0001).contains(coordinate))
}

fn outside_work_zone(shape: &dyn Shape, frame: Isometry<Real>, at: Isometry<Real>) -> bool {
    let bounds = shape.compute_aabb(&(frame.inverse() * at));
    bounds.mins.x >= MACHINE_HALF_WIDTH
        || bounds.maxs.x <= -MACHINE_HALF_WIDTH
        || bounds.mins.z >= MACHINE_HALF_WIDTH
        || bounds.maxs.z <= -MACHINE_HALF_WIDTH
}

fn door_transform(width: f32, angle: f32) -> Isometry<Real> {
    let half = width * 0.5;
    Isometry::translation(-half, 0.0, 0.0)
        * pose([0.0; 3], angle)
        * Isometry::translation(half, 0.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value as Json, json};

    fn project(parts: Vec<Json>) -> Project {
        serde_json::from_value(json!({
            "version": 1, "name": "Cell test", "parts": parts,
            "production": {
                "seed": 19, "start": "2026-01-05T07:59:59",
                "calendars": {
                    "operator": {"weekly":[{"days":[1],"start_minute":480,"end_minute":600}]},
                    "tech": {"weekly":[{"days":[1],"start_minute":481,"end_minute":600}]}
                }
            }
        }))
        .unwrap()
    }

    fn center(id: &str, x: f32) -> Json {
        json!({"id":id,"name":id,"position":[x,0.60,0.0],"kind":"machine",
            "machine":{"seconds":0.25,"repair_seconds":0.2,"failure_percent":100,"scrap_percent":100}})
    }

    fn blank(id: &str, x: f32) -> Json {
        json!({"id":id,"name":id,"position":[x - 0.85,0.665,0.0],"kind":"crate",
            "product":"raw-blank","size":[0.25,0.12,0.34],"mass":1.25})
    }

    fn personnel(id: &str, role: &str, calendar: &str) -> Json {
        json!({"id":id,"name":id,"position":[2.0,0.0,2.0],"kind":"staff",
            "staff":{"role":role,"area":"shop","calendar":calendar}})
    }

    #[test]
    fn supervised_cycle_fails_repairs_and_releases_scrap() -> Result<()> {
        let mut sim = Simulation::new(project(vec![
            center("cnc", 0.0),
            blank("blank", 0.0),
            personnel("op", "operator", "operator"),
            personnel("tech", "technician", "tech"),
        ]))?;
        sim.command("cnc.start", Value::Bool(true))?;
        for _ in 0..120 {
            sim.step();
        }
        assert!(flag(&sim.tags, "cnc", "waiting_operator"));
        assert!(!flag(&sim.tags, "cnc", "busy"));
        for _ in 0..180 {
            sim.step();
        }
        assert!(flag(&sim.tags, "cnc", "fault"));
        assert_eq!(sim.tags["cnc.failure_count"].value, Value::Int(1));
        let progress = number(&sim.tags, "cnc", "progress");
        for _ in 0..30 {
            sim.step();
        }
        assert_eq!(number(&sim.tags, "cnc", "progress"), progress);
        assert_eq!(number(&sim.tags, "cnc", "repair_progress"), 0.0);
        sim.tick = 61 * u64::from(HZ);
        for _ in 0..220 {
            sim.step();
        }
        assert!(!flag(&sim.tags, "cnc", "fault"));
        assert_eq!(sim.tags["cnc.failure_count"].value, Value::Int(1));
        assert_eq!(sim.tags["cnc.scrap_count"].value, Value::Int(1));
        assert_eq!(sim.tags["cnc.good_count"].value, Value::Int(0));
        let state = sim.snapshot();
        assert_eq!(state.parcels.len(), 1);
        assert_eq!(state.parcels[0].id, "blank");
        assert!(state.parcels[0].scrap);
        assert_eq!(state.parcels[0].product.as_deref(), Some("machined-base"));
        assert!((state.parcels[0].position[2] - 0.85).abs() < 0.05);
        Ok(())
    }

    #[test]
    fn one_technician_services_only_one_machine_at_a_time() -> Result<()> {
        let mut sim = Simulation::new(project(vec![
            center("a", 0.0),
            center("b", 3.0),
            blank("a-input", 0.0),
            blank("b-input", 3.0),
            personnel("op", "operator", "operator"),
            personnel("tech", "technician", "tech"),
        ]))?;
        sim.tick = 30 * u64::from(HZ);
        sim.command("a.start", Value::Bool(true))?;
        sim.command("b.start", Value::Bool(true))?;
        for _ in 0..180 {
            sim.step();
        }
        assert!(flag(&sim.tags, "a", "fault") && flag(&sim.tags, "b", "fault"));
        sim.tick = 61 * u64::from(HZ);
        sim.step();
        assert_ne!(
            flag(&sim.tags, "a", "repairing"),
            flag(&sim.tags, "b", "repairing")
        );
        assert!(flag(&sim.tags, "tech", "busy"));
        for _ in 0..250 {
            sim.step();
        }
        assert_eq!(sim.tags["a.scrap_count"].value, Value::Int(1));
        assert_eq!(sim.tags["b.scrap_count"].value, Value::Int(1));
        Ok(())
    }

    #[test]
    fn machining_keeps_body_and_waits_for_clear_outlet() -> Result<()> {
        let mut machine = center("cnc", 0.0);
        machine["machine"]["failure_percent"] = json!(0);
        machine["machine"]["scrap_percent"] = json!(0);
        let mut sim = Simulation::new(project(vec![
            machine,
            blank("blank", 0.0),
            json!({"id":"obstruction","name":"Obstruction","position":[0.85,0.65,0.0],"kind":"crate",
                "product":"machined-base","size":[0.24,0.08,0.33],"mass":0.83}),
            personnel("op", "operator", "operator"),
        ]))?;
        sim.tick = u64::from(HZ);
        sim.command("cnc.start", Value::Bool(true))?;
        let body = sim.parcels["blank"].body;
        let collider = sim.parcels["blank"].collider;
        let mut previous = *sim.world.bodies[body].translation();
        let mut saw_loading = false;
        for _ in 0..280 {
            sim.step();
            let item = &sim.parcels["blank"];
            assert_eq!(item.body, body);
            assert_eq!(item.collider, collider);
            let position = *sim.world.bodies[body].translation();
            assert!((position - previous).norm() <= TRANSFER_SPEED * DT + 0.001);
            previous = position;
            saw_loading |= position.x > -0.7 && position.x < -0.2;
        }
        assert!(saw_loading);
        assert!(flag(&sim.tags, "cnc", "blocked"));
        assert!(flag(&sim.tags, "cnc", "busy"));
        assert_eq!(sim.tags["cnc.good_count"].value, Value::Int(0));
        assert_eq!(
            sim.parcels["blank"].product.as_deref(),
            Some("machined-base")
        );
        assert!(sim.world.bodies[body].translation().x.abs() < 0.001);
        assert!((sim.world.colliders[collider].mass() - 0.83).abs() < 0.001);

        sim.remove_parcel("obstruction");
        for _ in 0..24 {
            sim.step();
        }
        assert!(sim.world.bodies[body].translation().y > 0.8);
        sim.command("cnc.stop", Value::Bool(true))?;
        let paused = *sim.world.bodies[body].position();
        for _ in 0..8 {
            sim.step();
        }
        assert_eq!(sim.world.bodies[body].position(), &paused);
        sim.command("cnc.stop", Value::Bool(false))?;
        sim.command("cnc.start", Value::Bool(false))?;
        sim.step();
        sim.command("cnc.start", Value::Bool(true))?;

        let mut saw_unloading = false;
        previous = *sim.world.bodies[body].translation();
        for _ in 0..240 {
            sim.step();
            let position = *sim.world.bodies[body].translation();
            assert!((position - previous).norm() <= TRANSFER_SPEED * DT + 0.001);
            previous = position;
            saw_unloading |= position.x > 0.2 && position.x < 0.7;
        }
        assert!(saw_unloading);
        let item = &sim.parcels["blank"];
        assert_eq!(item.body, body);
        assert_eq!(item.collider, collider);
        assert!(item.owner.is_none());
        assert_eq!(sim.tags["cnc.good_count"].value, Value::Int(1));
        assert!((sim.world.bodies[body].translation().x - 0.85).abs() < 0.02);
        Ok(())
    }

    #[test]
    fn robot_extracts_and_sorts_the_same_part_before_next_cycle() -> Result<()> {
        for scrap in [false, true] {
            let mut machine = center("cnc", 0.0);
            machine["machine"]["unload_robot"] = json!("arm");
            machine["machine"]["failure_percent"] = json!(0);
            machine["machine"]["scrap_percent"] = json!(if scrap { 100 } else { 0 });
            machine["machine"]["outlet"] = json!([1.2, 0.0, 0.0]);
            machine["machine"]["reject"] = json!([1.2, 0.0, 0.46]);
            let outlet_z = if scrap { 0.46 } else { 0.0 };
            let mut sim = Simulation::new(project(vec![
                json!({"id":"arm","name":"Arm","position":[-0.1,0.5,-0.02],"kind":"robot","robot":{}}),
                machine,
                blank("first", 0.0),
                personnel("op", "operator", "operator"),
                json!({"id":"good","name":"Good","position":[1.4,0.49,0.0],"kind":"conveyor",
                    "size":[0.8,0.22,0.4],"speed":0.6}),
                json!({"id":"reject","name":"Reject","position":[1.4,0.49,0.46],"kind":"conveyor",
                    "size":[0.8,0.22,0.4],"speed":0.6}),
                json!({"id":"obstruction","name":"Obstruction","position":[1.2,0.645,outlet_z],
                    "kind":"crate","size":[0.24,0.08,0.33],"mass":0.83}),
            ]))?;
            sim.tick = u64::from(HZ);
            for tag in ["good.enabled", "reject.enabled"] {
                sim.command(tag, Value::Bool(false))?;
            }
            sim.command("cnc.start", Value::Bool(true))?;
            let first = &sim.parcels["first"];
            let (body, collider) = (first.body, first.collider);
            for _ in 0..260 {
                sim.step();
            }
            let spec = sim.project.product("raw-blank").unwrap();
            sim.spawn(
                "next".into(),
                [-0.85, 0.67, 0.0],
                0.0,
                spec.size,
                spec.mass,
                Some("raw-blank".into()),
            );
            let work_pose = *sim.world.bodies[body].position();
            for _ in 0..100 {
                sim.step();
            }
            assert_eq!(sim.world.bodies[body].position(), &work_pose);
            assert!(flag(&sim.tags, "cnc", "waiting_unload"));
            assert_eq!(sim.parcels["first"].owner.as_deref(), Some("cnc"));
            assert!(sim.parcels["next"].owner.is_none());

            sim.command("arm.start", Value::Bool(true))?;
            for _ in 0..180 {
                sim.step();
                if flag(&sim.tags, "arm", "holding") {
                    break;
                }
            }
            assert!(flag(&sim.tags, "arm", "holding"));
            assert_eq!(sim.world.bodies[body].position(), &work_pose);
            assert!(sim.parcels["next"].owner.is_none());
            sim.command("arm.stop", Value::Bool(true))?;
            for _ in 0..12 {
                sim.step();
            }
            assert_eq!(sim.world.bodies[body].position(), &work_pose);
            assert!(sim.parcels["next"].owner.is_none());
            sim.command("arm.stop", Value::Bool(false))?;
            sim.command("arm.start", Value::Bool(false))?;
            sim.step();
            sim.command("arm.start", Value::Bool(true))?;

            let mut previous = *sim.world.bodies[body].translation();
            for _ in 0..150 {
                sim.step();
                let current = *sim.world.bodies[body].position();
                assert!((current.translation.vector - previous).norm() <= 1.5 * DT + 0.001);
                if !outside_work_zone(
                    sim.world.colliders[collider].shape(),
                    sim.cells.machines["cnc"].frame,
                    current,
                ) {
                    assert!(sim.parcels["next"].owner.is_none());
                }
                previous = current.translation.vector;
            }
            let count = if scrap {
                "cnc.scrap_count"
            } else {
                "cnc.good_count"
            };
            assert!(flag(&sim.tags, "arm", "waiting"));
            assert_eq!(sim.parcels["first"].owner.as_deref(), Some("arm"));
            assert_eq!(sim.parcels["next"].owner.as_deref(), Some("cnc"));
            assert_eq!(sim.tags[count].value, Value::Int(0));
            assert!(previous.y > 0.90);
            assert!((previous.x - 1.2).abs() < 0.001);
            assert!((previous.z - outlet_z).abs() < 0.001);

            sim.remove_parcel("obstruction");
            for _ in 0..60 {
                sim.step();
            }
            let first = &sim.parcels["first"];
            assert_eq!((first.body, first.collider), (body, collider));
            assert!(first.owner.is_none());
            assert_eq!(first.scrap, scrap);
            assert_eq!(sim.tags[count].value, Value::Int(1));
            assert!((sim.world.bodies[body].translation().z - outlet_z).abs() < 0.001);
        }
        Ok(())
    }

    #[test]
    fn robot_waits_for_signal_then_transfers_an_item() -> Result<()> {
        let mut sim = Simulation::new(project(vec![
            json!({"id":"arm","name":"Arm","position":[0.0,0.0,0.0],"kind":"robot","robot":{
                "repeat":false,"speed":2.0,"program":[{"op":"wait_signal"},{"op":"move","at":[0.2,0.12,0.2]},
                {"op":"grab"},{"op":"move","at":[0.2,0.65,0.2]},{"op":"move","at":[1.5,0.65,0.2]},
                {"op":"rotate","degrees":90},{"op":"move","at":[1.5,0.2,0.2]},{"op":"release"}]}}),
            json!({"id":"blank","name":"Blank","position":[0.24,0.06,0.2],"yaw":25,"kind":"crate",
                "product":"raw-blank","size":[0.25,0.12,0.34],"mass":1.25}),
        ]))?;
        sim.command("arm.start", Value::Bool(true))?;
        for _ in 0..20 {
            sim.step();
        }
        assert!(flag(&sim.tags, "arm", "waiting"));
        assert!(!flag(&sim.tags, "arm", "holding"));
        sim.command("arm.signal", Value::Bool(true))?;
        let body = sim.parcels["blank"].body;
        let mut saw_grip = false;
        for _ in 0..360 {
            let was_holding = flag(&sim.tags, "arm", "holding");
            let before = *sim.world.bodies[body].position();
            sim.step();
            if !was_holding && flag(&sim.tags, "arm", "holding") {
                saw_grip = true;
                let after = sim.world.bodies[body].position();
                assert!((after.translation.vector - before.translation.vector).norm() < 0.00001);
                assert!(after.rotation.angle_to(&before.rotation) < 0.001);
            }
        }
        assert!(saw_grip);
        assert!(flag(&sim.tags, "arm", "done"));
        assert!(!flag(&sim.tags, "arm", "holding"));
        assert!((sim.snapshot().parcels[0].position[0] - 1.5).abs() < 0.05);
        Ok(())
    }
    #[test]
    fn transport_and_sensor_equipment_reports_physical_state() -> Result<()> {
        let equipment = |id: &str, device: &str, x: f32, y: f32, size: [f32; 3]| {
            json!({"id":id,"name":id,"position":[x,y,0.0],"kind":"equipment",
                "equipment":device,"size":size})
        };
        let cargo = |id: &str, x: f32, y: f32| {
            json!({"id":id,"name":id,"position":[x,y,0.0],"kind":"crate",
                "product":"raw-blank","size":[0.25,0.12,0.34],"mass":1.25})
        };
        let mut sim = Simulation::new(project(vec![
            equipment("scale", "scale", 0.0, 0.30, [1.1, 0.22, 1.1]),
            cargo("weighted", 0.0, 0.47),
            equipment("cap", "capacitive", -0.25, 0.47, [0.10, 0.12, 0.10]),
            equipment("array", "light_array", 0.0, 0.45, [1.0, 0.75, 0.08]),
            equipment("roller", "roller", 2.0, 0.30, [1.1, 0.22, 1.1]),
            cargo("rolling", 2.0, 0.47),
            equipment("turn", "turntable", 4.0, 0.30, [1.1, 0.22, 1.1]),
            equipment("chain", "chain_transfer", 6.0, 0.30, [1.1, 0.22, 1.1]),
            cargo("side", 6.0, 0.47),
            equipment("blade", "stop_blade", 8.0, 0.30, [0.08, 0.12, 0.8]),
            equipment("fence", "fence", 9.0, 0.90, [1.2, 1.8, 0.08]),
            equipment("door", "safety_door", 11.0, 0.90, [1.2, 1.8, 0.08]),
            equipment("tower", "stack_light", 13.0, 0.90, [0.16, 0.6, 0.16]),
        ]))?;
        for id in [
            "roller.forward",
            "turn.turn",
            "chain.right",
            "blade.raise",
            "door.open",
            "tower.green",
        ] {
            sim.command(id, Value::Bool(true))?;
        }
        for _ in 0..65 {
            sim.step();
        }
        assert!(flag(&sim.tags, "cap", "detected"));
        assert_eq!(number(&sim.tags, "scale", "weight_kg"), 1.25);
        assert_eq!(number(&sim.tags, "scale", "weight_v"), 0.625);
        assert!(flag(&sim.tags, "array", "aligned"));
        assert_ne!(sim.tags["array.mask"].value, Value::Int(0));
        assert!(flag(&sim.tags, "chain", "raised"));
        assert!(flag(&sim.tags, "blade", "raised"));
        assert!(!flag(&sim.tags, "door", "closed"));
        let pos = |id: &str| *sim.world.bodies[sim.parcels[id].body].translation();
        assert!(pos("rolling").x > 2.12);
        assert!(pos("side").z > 0.10);
        for _ in 0..240 {
            sim.step();
        }
        assert!(flag(&sim.tags, "turn", "limit_90"));
        sim.command("turn.turn", Value::Bool(false))?;
        sim.command("door.open", Value::Bool(false))?;
        sim.command("chain.left", Value::Bool(true))?;
        for _ in 0..300 {
            sim.step();
        }
        assert!(flag(&sim.tags, "turn", "limit_0"));
        assert!(flag(&sim.tags, "door", "closed"));
        assert!(!flag(&sim.tags, "chain", "raised"));
        Ok(())
    }
}
