mod cell;
mod feed;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use rapier3d::prelude::*;
use serde::Serialize;

use crate::{
    production::{self, Emitter},
    project::{Direction, Part, PartKind, Project, Tag, Value},
};

pub const HZ: u32 = 120;
pub const DT: f32 = 1.0 / HZ as f32;
const MAX_PARCELS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Good,
    Stale,
}

#[derive(Clone, Debug, Serialize)]
pub struct TagState {
    #[serde(flatten)]
    pub tag: Tag,
    pub value: Value,
    pub quality: Quality,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParcelPose {
    pub id: String,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub size: [f32; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub scrap: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub project: String,
    pub running: bool,
    pub tick: u64,
    pub elapsed_seconds: f64,
    pub calendar_time: Option<chrono::NaiveDateTime>,
    pub tags: BTreeMap<String, TagState>,
    pub parcels: Vec<ParcelPose>,
    pub drivers: BTreeMap<String, String>,
    #[cfg(any(feature = "studio", feature = "web"))]
    #[serde(skip)]
    pub belt_travel: BTreeMap<String, f32>,
    #[cfg(any(feature = "studio", feature = "web"))]
    #[serde(skip)]
    pub machine_grips: BTreeMap<String, [f32; 3]>,
}

struct Belt {
    collider: ColliderHandle,
    axis: Vector<Real>,
    speed: f32,
    #[cfg(any(feature = "studio", feature = "web"))]
    travel: f32,
}

struct Pusher {
    joint: ImpulseJointHandle,
    carriage: RigidBodyHandle,
    commanded: f32,
    origin: Vector<Real>,
    axis: Vector<Real>,
}

struct Parcel {
    body: RigidBodyHandle,
    collider: ColliderHandle,
    size: [f32; 3],
    product: Option<String>,
    owner: Option<String>,
    scrap: bool,
}

struct RecipeRuntime {
    selector: Emitter,
    rng: rand_chacha::ChaCha8Rng,
    next_tick: u64,
    envelope: [f32; 3],
}

#[derive(Default)]
struct Surfaces {
    velocities: HashMap<ColliderHandle, Vector<Real>>,
}

impl PhysicsHooks for Surfaces {
    fn modify_solver_contacts(&self, ctx: &mut ContactModificationContext<'_>) {
        let first = self
            .velocities
            .get(&ctx.collider1)
            .copied()
            .unwrap_or_default();
        let second = self
            .velocities
            .get(&ctx.collider2)
            .copied()
            .unwrap_or_default();
        let relative_velocity = first - second;
        for contact in &mut *ctx.solver_contacts {
            contact.tangent_velocity = relative_velocity;
        }
    }
}

pub struct Simulation {
    pub project: Project,
    pub running: bool,
    pub tick: u64,
    tags: BTreeMap<String, TagState>,
    pub drivers: BTreeMap<String, String>,
    world: World,
    surfaces: Surfaces,
    belts: BTreeMap<String, Belt>,
    pushers: BTreeMap<String, Pusher>,
    parcels: BTreeMap<String, Parcel>,
    product_colliders: BTreeMap<String, SharedShape>,
    parcel_colliders: HashSet<ColliderHandle>,
    emissions: BTreeMap<String, f32>,
    recipes: BTreeMap<String, RecipeRuntime>,
    random_feeds: BTreeMap<String, feed::RandomFeed>,
    cells: cell::Cells,
    cell_parts: Arc<[Part]>,
    serial: u64,
    initial: BTreeMap<String, Value>,
}

struct World {
    pipeline: PhysicsPipeline,
    parameters: IntegrationParameters,
    islands: IslandManager,
    broad: BroadPhaseMultiSap,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    joints: ImpulseJointSet,
    multibody: MultibodyJointSet,
    ccd: CCDSolver,
    queries: QueryPipeline,
}

impl World {
    fn new() -> Self {
        let mut world = Self {
            pipeline: PhysicsPipeline::new(),
            parameters: IntegrationParameters {
                dt: DT,
                ..Default::default()
            },
            islands: IslandManager::new(),
            broad: BroadPhaseMultiSap::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            joints: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            queries: QueryPipeline::new(),
        };
        world.colliders.insert(
            ColliderBuilder::cuboid(1000.0, 0.15, 1000.0).translation(vector![0.0, -0.15, 0.0]),
        );
        world
    }

    fn step(&mut self, hooks: &Surfaces) {
        self.pipeline.step(
            &vector![0.0, -10.0, 0.0],
            &self.parameters,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.joints,
            &mut self.multibody,
            &mut self.ccd,
            Some(&mut self.queries),
            hooks,
            &(),
        );
    }
}

impl Simulation {
    pub fn new(project: Project) -> Result<Self> {
        project.validate()?;
        let mut initial = BTreeMap::new();
        let mut tags = BTreeMap::new();
        for (id, tag) in project.tags() {
            let value = tag.check(
                project
                    .initial_commands
                    .get(&id)
                    .copied()
                    .unwrap_or(tag.default),
            )?;
            initial.insert(id.clone(), value);
            tags.insert(
                id,
                TagState {
                    tag,
                    value,
                    quality: Quality::Good,
                },
            );
        }
        let mut product_colliders = BTreeMap::new();
        let mut used_products = std::collections::BTreeSet::new();
        for part in &project.parts {
            used_products.extend(part.product.as_deref());
            match &part.kind {
                PartKind::Emitter {
                    emission: Some(production::Emission::Counts { rows, .. }),
                    ..
                } => {
                    used_products.extend(rows.iter().filter_map(|row| row.product.as_deref()));
                }
                PartKind::Emitter {
                    emission: Some(production::Emission::Probability { rows }),
                    ..
                } => {
                    used_products.extend(rows.iter().filter_map(|row| row.product.as_deref()));
                }
                PartKind::Machine { machine } => {
                    used_products.insert(machine.input.as_str());
                    used_products.insert(machine.output.as_str());
                    used_products.extend(machine.scrap_product.as_deref());
                }
                _ => {}
            }
        }
        for id in used_products {
            if let Some(mesh) = project.product(id).and_then(|spec| spec.mesh) {
                let shape = mesh.collision_shape().with_context(|| {
                    format!("product {id} has no three-dimensional collision hull")
                })?;
                product_colliders.insert(id.to_owned(), shape);
            }
        }
        let mut cell_parts: Vec<_> = project
            .parts
            .iter()
            .filter(|part| {
                matches!(
                    part.kind,
                    PartKind::Equipment { .. }
                        | PartKind::Staff { .. }
                        | PartKind::Machine { .. }
                        | PartKind::Robot { .. }
                )
            })
            .cloned()
            .collect();
        cell_parts.sort_by(|a, b| a.id.cmp(&b.id));
        let mut simulation = Self {
            project,
            running: false,
            tick: 0,
            tags,
            drivers: BTreeMap::new(),
            world: World::new(),
            surfaces: Surfaces::default(),
            belts: BTreeMap::new(),
            pushers: BTreeMap::new(),
            parcels: BTreeMap::new(),
            product_colliders,
            parcel_colliders: HashSet::new(),
            emissions: BTreeMap::new(),
            recipes: BTreeMap::new(),
            random_feeds: BTreeMap::new(),
            cells: cell::Cells::default(),
            cell_parts: cell_parts.into(),
            serial: 0,
            initial,
        };
        for part in simulation.project.parts.clone() {
            simulation.insert_part(&part);
        }
        if !simulation.random_feeds.is_empty() {
            simulation.world.queries.update(&simulation.world.colliders);
        }
        Ok(simulation)
    }

    fn insert_part(&mut self, part: &Part) {
        let isometry = pose(part.position, part.yaw);
        match part.kind {
            PartKind::Conveyor { size, .. } => {
                let collider = self.world.colliders.insert(
                    cuboid(size)
                        .position(isometry)
                        .friction(1.2)
                        .active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS),
                );
                self.belts.insert(
                    part.id.clone(),
                    Belt {
                        collider,
                        axis: isometry.rotation * Vector::x(),
                        speed: 0.0,
                        #[cfg(any(feature = "studio", feature = "web"))]
                        travel: 0.0,
                    },
                );
            }
            PartKind::Barrier { size } => {
                self.world.colliders.insert(cuboid(size).position(isometry));
            }
            PartKind::FlowGuide {
                length,
                height,
                inlet,
                outlet,
            } => {
                for rail in production::guide_rails(length, height, inlet, outlet) {
                    self.world.colliders.insert(
                        cuboid(rail.size)
                            .position(isometry * pose(rail.position, rail.yaw))
                            .friction(0.07)
                            .friction_combine_rule(CoefficientCombineRule::Min),
                    );
                }
            }
            PartKind::Crate { size, mass } => self.spawn(
                part.id.clone(),
                part.position,
                part.yaw,
                size,
                mass,
                part.product.clone(),
            ),
            PartKind::Pusher { stroke, .. } => {
                let base = self
                    .world
                    .bodies
                    .insert(RigidBodyBuilder::fixed().position(isometry));
                let carriage = self.world.bodies.insert(
                    RigidBodyBuilder::dynamic()
                        .position(isometry)
                        .ccd_enabled(true),
                );
                self.world.colliders.insert_with_parent(
                    cuboid([0.65, 0.32, 0.10]).mass(4.0).friction(0.25),
                    carriage,
                    &mut self.world.bodies,
                );
                let slider = PrismaticJointBuilder::new(Vector::z_axis())
                    .limits([0.0, stroke])
                    .motor_position(0.0, 2200.0, 125.0)
                    .motor_max_force(8000.0);
                let joint = self.world.joints.insert(base, carriage, slider, true);
                self.pushers.insert(
                    part.id.clone(),
                    Pusher {
                        joint,
                        carriage,
                        commanded: 0.0,
                        origin: isometry.translation.vector,
                        axis: isometry.rotation * Vector::z(),
                    },
                );
            }
            PartKind::Emitter {
                ref emission,
                ref random_spawn_belt,
                interval,
                ..
            } => {
                if let Some(belt) = random_spawn_belt {
                    let belt = self
                        .project
                        .parts
                        .iter()
                        .find(|part| &part.id == belt)
                        .unwrap();
                    self.random_feeds.insert(
                        part.id.clone(),
                        feed::RandomFeed::new(self.project.production.seed, part, belt, interval),
                    );
                }
                if let Some(config) = emission {
                    let selector = Emitter::new(config).unwrap();
                    let mut envelope = [0.02_f32; 3];
                    for product in selector.products() {
                        let dimensions = self.project.product(product).unwrap().size;
                        for axis in 0..3 {
                            envelope[axis] = envelope[axis].max(dimensions[axis]);
                        }
                    }
                    self.recipes.insert(
                        part.id.clone(),
                        RecipeRuntime {
                            selector,
                            rng: production::stream(self.project.production.seed, &part.id, 0),
                            next_tick: 0,
                            envelope,
                        },
                    );
                }
                self.emissions.insert(part.id.clone(), 0.0);
            }
            PartKind::Equipment { .. }
            | PartKind::Machine { .. }
            | PartKind::Robot { .. }
            | PartKind::Staff { .. } => self.cell_insert(part),
            PartKind::Sensor { .. } | PartKind::Remover { .. } => {}
        }
    }

    fn spawn(
        &mut self,
        id: String,
        position: [f32; 3],
        yaw: f32,
        size: [f32; 3],
        mass: f32,
        product: Option<String>,
    ) {
        let spec = product.as_deref().and_then(|id| self.project.product(id));
        let (size, mass) = spec.map_or((size, mass), |spec| (spec.size, spec.mass));
        let collider = ColliderBuilder::new(self.parcel_shape(product.as_deref(), size));
        let body = self.world.bodies.insert(
            RigidBodyBuilder::dynamic()
                .position(pose(position, yaw))
                .ccd_enabled(true),
        );
        let collider = self.world.colliders.insert_with_parent(
            collider.mass(mass).friction(0.85),
            body,
            &mut self.world.bodies,
        );
        self.parcels.insert(
            id,
            Parcel {
                body,
                collider,
                size,
                product,
                owner: None,
                scrap: false,
            },
        );
        self.parcel_colliders.insert(collider);
    }

    fn parcel_shape(&self, product: Option<&str>, [x, y, z]: [f32; 3]) -> SharedShape {
        product
            .and_then(|id| self.product_colliders.get(id))
            .cloned()
            .unwrap_or_else(|| SharedShape::cuboid(x * 0.5, y * 0.5, z * 0.5))
    }

    fn checked_command(&self, id: &str, value: Value) -> Result<Value> {
        let state = self
            .tags
            .get(id)
            .with_context(|| format!("unknown tag {id}"))?;
        ensure!(
            state.tag.direction == Direction::Actuator,
            "{id} is read only"
        );
        state.tag.check(value)
    }

    pub fn command(&mut self, id: &str, value: Value) -> Result<()> {
        let value = self.checked_command(id, value)?;
        let state = self.tags.get_mut(id).unwrap();
        state.value = value;
        state.quality = Quality::Good;
        Ok(())
    }

    pub fn command_batch(&mut self, mut values: Vec<(String, Value)>) -> Result<()> {
        for (id, value) in &mut values {
            *value = self.checked_command(id, *value)?;
        }
        for (id, value) in values {
            let state = self.tags.get_mut(&id).unwrap();
            state.value = value;
            state.quality = Quality::Good;
        }
        Ok(())
    }

    pub fn stale(&mut self, id: &str, reset: bool) {
        if let Some(state) = self.tags.get_mut(id) {
            if reset {
                state.value = self.initial[id];
            }
            state.quality = Quality::Stale;
        }
    }

    #[cfg(any(feature = "mcp", feature = "studio"))]
    pub fn reset(&mut self) -> Result<()> {
        let drivers = self.drivers.clone();
        let stale: Vec<_> = self
            .tags
            .values()
            .filter(|state| state.quality == Quality::Stale)
            .map(|state| state.tag.id.clone())
            .collect();
        *self = Self::new(self.project.clone())?;
        self.drivers = drivers;
        for id in stale {
            self.stale(&id, false);
        }
        Ok(())
    }

    pub fn step(&mut self) {
        if !self.cell_parts.is_empty() {
            self.cell_tick();
        }
        let mut sensors = Vec::new();
        let mut to_spawn = Vec::new();
        let mut moving_changed = false;
        for part in &self.project.parts {
            let id = &part.id;
            match part.kind {
                PartKind::Conveyor { .. } => {
                    let target = if flag(&self.tags, id, "enabled") {
                        number(&self.tags, id, "target_speed")
                    } else {
                        0.0
                    };
                    let belt = self.belts.get_mut(id).unwrap();
                    let previous = belt.speed;
                    belt.speed = approach(previous, target, 4.0 * DT);
                    #[cfg(any(feature = "studio", feature = "web"))]
                    {
                        belt.travel = (belt.travel + belt.speed * DT).rem_euclid(0.25);
                    }
                    moving_changed |= (previous == 0.0) != (belt.speed == 0.0)
                        || previous.signum() != belt.speed.signum();
                    self.surfaces
                        .velocities
                        .insert(belt.collider, belt.axis * belt.speed);
                    sensors.push((tag_id(id, "speed"), Value::Float(belt.speed)));
                }
                PartKind::Pusher { stroke, speed } => {
                    let pusher = self.pushers.get_mut(id).unwrap();
                    let target = if flag(&self.tags, id, "extend") {
                        stroke
                    } else {
                        0.0
                    };
                    let requested = approach(pusher.commanded, target, speed * DT);
                    if requested != pusher.commanded {
                        pusher.commanded = requested;
                        self.world.bodies[pusher.carriage].wake_up(true);
                        self.world
                            .joints
                            .get_mut(pusher.joint, true)
                            .unwrap()
                            .data
                            .set_motor_position(JointAxis::LinX, requested, 2200.0, 125.0);
                    }
                }
                PartKind::Emitter {
                    interval,
                    size,
                    mass,
                    ref emission,
                    ref random_spawn_belt,
                } => {
                    if random_spawn_belt.is_some() {
                        continue;
                    }
                    if emission.is_some() {
                        let enabled = flag(&self.tags, id, "enabled");
                        let state = &self.recipes[id];
                        if !enabled
                            || self.tick < state.next_tick
                            || self.parcels.len() + to_spawn.len() >= MAX_PARCELS
                        {
                            continue;
                        }
                        // Each attempted draw uses the largest footprint in the recipe.
                        let at = pose(part.position, part.yaw);
                        let shape = Cuboid::new(Vector::from(state.envelope) * 0.55);
                        if !self.free(&at, &shape, None)
                            || to_spawn.iter().any(|(_, p, y, s, _, _)| {
                                rapier3d::parry::query::intersection_test(
                                    &at,
                                    &shape,
                                    &pose(*p, *y),
                                    &Cuboid::new(Vector::from(*s) * 0.5),
                                )
                                .unwrap_or(true)
                            })
                        {
                            continue;
                        }
                        let state = self.recipes.get_mut(id).unwrap();
                        state.next_tick = self
                            .tick
                            .saturating_add(production::ticks(f64::from(interval)).unwrap());
                        let outcome = state.selector.next(
                            &mut state.rng,
                            self.tick as f64 / f64::from(HZ),
                            self.project.production.clock(self.tick),
                        );
                        sensors.push((tag_id(id, "recipe_error"), Value::Bool(outcome.is_err())));
                        match outcome {
                            Ok(Some(product)) => {
                                let spec = self.project.product(&product).unwrap();
                                self.serial += 1;
                                to_spawn.push((
                                    format!("{id}#{}", self.serial),
                                    part.position,
                                    part.yaw,
                                    spec.size,
                                    spec.mass,
                                    Some(product),
                                ));
                                sensors.push((
                                    tag_id(id, "count"),
                                    Value::Int(count(&self.tags, id).saturating_add(1)),
                                ));
                            }
                            Ok(None) => {
                                let n = if let Value::Int(n) =
                                    self.tags[&tag_id(id, "skipped")].value
                                {
                                    n
                                } else {
                                    0
                                };
                                sensors
                                    .push((tag_id(id, "skipped"), Value::Int(n.saturating_add(1))));
                            }
                            Err(error) => {
                                if !flag(&self.tags, id, "recipe_error") {
                                    log::warn!("emitter {id}: {error}");
                                }
                            }
                        }
                        continue;
                    }
                    let effective_size = part
                        .product
                        .as_deref()
                        .and_then(|id| self.project.product(id))
                        .map_or(size, |spec| spec.size);
                    let timer = self.emissions.get_mut(id).unwrap();
                    if flag(&self.tags, id, "enabled") {
                        *timer -= DT;
                        let occupied = self.parcels.values().any(|parcel| {
                            if parcel.owner.is_some() {
                                return false;
                            }
                            let offset = self.world.bodies[parcel.body].translation()
                                - Vector::from(part.position);
                            offset.norm_squared() < effective_size[0].max(effective_size[2]).powi(2)
                        });
                        if *timer <= 0.0
                            && !occupied
                            && self.parcels.len() + to_spawn.len() < MAX_PARCELS
                        {
                            *timer = interval;
                            self.serial += 1;
                            to_spawn.push((
                                format!("{}#{}", id, self.serial),
                                part.position,
                                part.yaw,
                                size,
                                mass,
                                part.product.clone(),
                            ));
                            sensors.push((
                                tag_id(id, "count"),
                                Value::Int(count(&self.tags, id).saturating_add(1)),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        if moving_changed {
            for parcel in self.parcels.values() {
                self.world.bodies[parcel.body].wake_up(true);
            }
        }
        self.step_random_feeds(&mut to_spawn);
        for (id, position, yaw, size, mass, product) in to_spawn {
            self.spawn(id, position, yaw, size, mass, product);
        }

        self.world.step(&self.surfaces);
        self.tick += 1;
        if !self.cell_parts.is_empty() {
            self.cell_sensors();
        }

        let mut removed = BTreeMap::new();
        for part in &self.project.parts {
            let id = &part.id;
            match part.kind {
                PartKind::Pusher { stroke, .. } => {
                    let pusher = &self.pushers[id];
                    let travel = (self.world.bodies[pusher.carriage].translation() - pusher.origin)
                        .dot(&pusher.axis)
                        .clamp(0.0, stroke);
                    sensors.push((tag_id(id, "position"), Value::Float(travel)));
                    sensors.push((
                        tag_id(id, "front"),
                        Value::Bool(travel >= stroke - 0.02_f32.min(stroke * 0.2)),
                    ));
                    sensors.push((
                        tag_id(id, "back"),
                        Value::Bool(travel <= 0.05_f32.min(stroke * 0.2)),
                    ));
                }
                PartKind::Sensor { range } => {
                    let isometry = pose(part.position, part.yaw);
                    let ray = Ray::new(Point::from(part.position), isometry.rotation * Vector::x());
                    let hit = self.world.queries.cast_ray(
                        &self.world.bodies,
                        &self.world.colliders,
                        &ray,
                        range,
                        true,
                        QueryFilter::default()
                            .predicate(&|collider, _| self.parcel_colliders.contains(&collider)),
                    );
                    sensors.push((tag_id(id, "detected"), Value::Bool(hit.is_some())));
                    sensors.push((
                        tag_id(id, "distance"),
                        Value::Float(hit.map_or(range, |(_, distance)| distance)),
                    ));
                }
                PartKind::Remover { size } if flag(&self.tags, id, "enabled") => {
                    let inverse = pose(part.position, part.yaw).inverse();
                    let mut current = count(&self.tags, id);
                    for (name, parcel) in &self.parcels {
                        if parcel.owner.is_some() || removed.contains_key(name) {
                            continue;
                        }
                        let local =
                            inverse * Point::from(*self.world.bodies[parcel.body].translation());
                        if (0..3).all(|axis| local[axis].abs() <= size[axis] * 0.5) {
                            removed.insert(name.clone(), ());
                            current = current.saturating_add(1);
                        }
                    }
                    sensors.push((tag_id(id, "count"), Value::Int(current)));
                }
                _ => {}
            }
        }
        for (id, ()) in removed {
            self.remove_parcel(&id);
        }
        for (id, value) in sensors {
            self.tags.get_mut(&id).unwrap().value = value;
        }
    }

    fn remove_parcel(&mut self, id: &str) {
        let parcel = self.parcels.remove(id).unwrap();
        self.parcel_colliders.remove(&parcel.collider);
        self.world.bodies.remove(
            parcel.body,
            &mut self.world.islands,
            &mut self.world.colliders,
            &mut self.world.joints,
            &mut self.world.multibody,
            true,
        );
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut tags = self.tags.clone();
        tags.get_mut("sim.running").unwrap().value = Value::Bool(self.running);
        let elapsed_seconds = self.tick as f64 / HZ as f64;
        tags.get_mut("sim.time").unwrap().value = Value::Float(elapsed_seconds as f32);
        Snapshot {
            project: self.project.name.clone(),
            running: self.running,
            tick: self.tick,
            elapsed_seconds,
            calendar_time: self.project.production.clock(self.tick),
            tags,
            parcels: self
                .parcels
                .iter()
                .map(|(id, parcel)| {
                    let body = &self.world.bodies[parcel.body];
                    let q = body.rotation().quaternion();
                    ParcelPose {
                        id: id.clone(),
                        position: (*body.translation()).into(),
                        rotation: [q.i, q.j, q.k, q.w],
                        size: parcel.size,
                        product: parcel.product.clone(),
                        scrap: parcel.scrap,
                    }
                })
                .collect(),
            drivers: self.drivers.clone(),
            #[cfg(any(feature = "studio", feature = "web"))]
            machine_grips: self.machine_grips(),
            #[cfg(any(feature = "studio", feature = "web"))]
            belt_travel: self
                .belts
                .iter()
                .map(|(id, belt)| (id.clone(), belt.travel))
                .collect(),
        }
    }
}

fn pose(position: [f32; 3], yaw: f32) -> Isometry<Real> {
    Isometry::new(Vector::from(position), vector![0.0, yaw.to_radians(), 0.0])
}

fn cuboid([x, y, z]: [f32; 3]) -> ColliderBuilder {
    ColliderBuilder::cuboid(x * 0.5, y * 0.5, z * 0.5)
}

fn approach(value: f32, target: f32, max_change: f32) -> f32 {
    let delta = target - value;
    if delta.abs() <= max_change {
        target
    } else {
        value + delta.signum() * max_change
    }
}

fn tag_id(part: &str, role: &str) -> String {
    format!("{part}.{role}")
}

fn flag(tags: &BTreeMap<String, TagState>, id: &str, role: &str) -> bool {
    matches!(tags[&tag_id(id, role)].value, Value::Bool(true))
}

fn number(tags: &BTreeMap<String, TagState>, id: &str, role: &str) -> f32 {
    match tags[&tag_id(id, role)].value {
        Value::Float(value) => value,
        _ => unreachable!("float tag"),
    }
}

fn count(tags: &BTreeMap<String, TagState>, id: &str) -> i32 {
    match tags[&tag_id(id, "count")].value {
        Value::Int(value) => value,
        _ => unreachable!("count tag"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conveyor_carries_parcel_to_detector_and_exit() -> Result<()> {
        let mut simulation = Simulation::new(Project::demo())?;
        simulation.command("supply.enabled", Value::Bool(false))?;
        let mut seen = false;
        for _ in 0..(HZ * 8) {
            simulation.step();
            seen |= simulation.tags["scanner.detected"].value == Value::Bool(true);
        }
        assert!(seen, "parcel crossed the ray");
        assert_eq!(simulation.tags["exit.count"].value, Value::Int(1));
        assert_eq!(simulation.tags["side_exit.count"].value, Value::Int(0));
        assert!(simulation.parcels.is_empty());
        Ok(())
    }

    #[test]
    fn pusher_reaches_both_limits() -> Result<()> {
        let mut simulation = Simulation::new(Project::demo())?;
        simulation.command("supply.enabled", Value::Bool(false))?;
        simulation.command("diverter.extend", Value::Bool(true))?;
        for _ in 0..(HZ * 2) {
            simulation.step();
        }
        assert_eq!(simulation.tags["diverter.front"].value, Value::Bool(true));
        simulation.command("diverter.extend", Value::Bool(false))?;
        for _ in 0..(HZ * 2) {
            simulation.step();
        }
        assert_eq!(simulation.tags["diverter.back"].value, Value::Bool(true));
        Ok(())
    }
}
