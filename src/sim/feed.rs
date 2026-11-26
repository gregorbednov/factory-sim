use super::*;
use rand::Rng;

pub(super) type Spawn = (String, [f32; 3], f32, [f32; 3], f32, Option<String>);

struct Payload {
    size: [f32; 3],
    mass: f32,
    product: Option<String>,
}

pub(super) struct RandomFeed {
    frame: Isometry<Real>,
    belt_yaw: f32,
    deck: [f32; 3],
    station: f32,
    interval: u64,
    next_tick: u64,
    rng: rand_chacha::ChaCha8Rng,
    pending: Option<Payload>,
}

impl RandomFeed {
    pub(super) fn new(seed: u64, source: &Part, belt: &Part, interval: f32) -> Self {
        let frame = pose(belt.position, belt.yaw);
        Self {
            frame,
            belt_yaw: belt.yaw,
            deck: belt.kind.feed_surface().unwrap(),
            station: frame
                .inverse_transform_point(&Point::from(source.position))
                .x,
            interval: production::ticks(f64::from(interval)).unwrap(),
            next_tick: 0,
            rng: production::stream(seed, &source.id, 3),
            pending: None,
        }
    }

    fn placement(&mut self, sim: &Simulation, queued: &[Spawn]) -> Option<([f32; 3], f32)> {
        let payload = self.pending.as_ref()?;
        let shape = sim.parcel_shape(payload.product.as_deref(), payload.size);
        let edge = 0.005;
        let half_length = 0.5 * self.deck[0] - edge;
        let half_width = 0.5 * self.deck[2] - edge;
        for _ in 0..16 {
            let yaw = self.rng.gen_range(0.0..360.0);
            let bounds = shape.compute_aabb(&pose([0.0; 3], yaw));
            let x_min = -half_length - bounds.mins.x;
            let x_max = half_length - bounds.maxs.x;
            let z_min = -half_width - bounds.mins.z;
            let z_max = half_width - bounds.maxs.z;
            if x_min > x_max || z_min > z_max {
                continue;
            }

            let local = Point::new(
                self.station.clamp(x_min, x_max),
                self.deck[1] * 0.5 - bounds.mins.y + 0.015,
                self.rng.gen_range(z_min..=z_max),
            );
            let at = self.frame.transform_point(&local).coords.into();
            let yaw = ((self.belt_yaw % 360.0) + yaw).rem_euclid(360.0);
            let transform = pose(at, yaw);
            if sim.free(&transform, shape.as_ref(), None)
                && sim
                    .world
                    .queries
                    .intersection_with_shape(
                        &sim.world.bodies,
                        &sim.world.colliders,
                        &transform,
                        shape.as_ref(),
                        QueryFilter::exclude_dynamic(),
                    )
                    .is_none()
                && queued.iter().all(|(_, other, angle, size, _, product)| {
                    let other_shape = sim.parcel_shape(product.as_deref(), *size);
                    !rapier3d::parry::query::intersection_test(
                        &transform,
                        shape.as_ref(),
                        &pose(*other, *angle),
                        other_shape.as_ref(),
                    )
                    .unwrap_or(true)
                })
            {
                return Some((at, yaw));
            }
        }
        None
    }
}

impl Simulation {
    pub(super) fn step_random_feeds(&mut self, queued: &mut Vec<Spawn>) {
        let mut feeds = std::mem::take(&mut self.random_feeds);
        for (id, feed) in &mut feeds {
            if !flag(&self.tags, id, "enabled") {
                self.publish(id, "spawn_blocked", Value::Bool(false));
                continue;
            }
            if self.tick < feed.next_tick || self.parcels.len() + queued.len() >= MAX_PARCELS {
                continue;
            }

            if feed.pending.is_none() {
                feed.pending = self.next_random_payload(id);
                if feed.pending.is_none() {
                    feed.next_tick = self.tick.saturating_add(feed.interval);
                    self.publish(id, "spawn_blocked", Value::Bool(false));
                    continue;
                }
            }
            let Some((at, yaw)) = feed.placement(self, queued) else {
                // Keep the chosen recipe result through retries while the line is occupied.
                feed.next_tick = self.tick.saturating_add(6);
                self.publish(id, "spawn_blocked", Value::Bool(true));
                continue;
            };
            let payload = feed.pending.take().unwrap();
            feed.next_tick = self.tick.saturating_add(feed.interval);
            self.serial += 1;
            queued.push((
                format!("{id}#{}", self.serial),
                at,
                yaw,
                payload.size,
                payload.mass,
                payload.product,
            ));
            self.publish(id, "spawn_blocked", Value::Bool(false));
            self.change_count(id, "count");
        }
        self.random_feeds = feeds;
    }

    fn next_random_payload(&mut self, id: &str) -> Option<Payload> {
        let part = self
            .project
            .parts
            .iter()
            .find(|part| part.id == id)
            .unwrap();
        let PartKind::Emitter { size, mass, .. } = part.kind else {
            unreachable!("random feed is created only for an emitter");
        };
        let product = if let Some(recipe) = self.recipes.get_mut(id) {
            let outcome = recipe.selector.next(
                &mut recipe.rng,
                self.tick as f64 / f64::from(HZ),
                self.project.production.clock(self.tick),
            );
            match outcome {
                Ok(Some(product)) => {
                    self.publish(id, "recipe_error", Value::Bool(false));
                    Some(product)
                }
                Ok(None) => {
                    self.publish(id, "recipe_error", Value::Bool(false));
                    self.change_count(id, "skipped");
                    return None;
                }
                Err(error) => {
                    if !flag(&self.tags, id, "recipe_error") {
                        log::warn!("emitter {id}: {error}");
                    }
                    self.publish(id, "recipe_error", Value::Bool(true));
                    return None;
                }
            }
        } else {
            part.product.clone()
        };
        let (size, mass) = product
            .as_deref()
            .and_then(|id| self.project.product(id))
            .map_or((size, mass), |spec| (spec.size, spec.mass));
        Some(Payload {
            size,
            mass,
            product,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Project {
        serde_json::from_str(include_str!("../../examples/flow-alignment.json")).unwrap()
    }

    #[test]
    fn random_feed_is_upright_within_rotated_belt_and_replays_after_reset() -> Result<()> {
        let mut project = example();
        project.remove_part("guide");
        project.remove_part("exit");
        let belt = &mut project.parts[0];
        belt.position = [1.0, 0.59, 2.0];
        belt.yaw = -90.0;
        let frame = pose(belt.position, belt.yaw);
        let source = &mut project.parts[1];
        source.position = frame
            .transform_point(&Point::new(-2.7, 0.6, 0.4))
            .coords
            .into();
        if let PartKind::Emitter { interval, .. } = &mut source.kind {
            *interval = 0.05;
        }

        let mut sim = Simulation::new(project)?;
        let mut positions = Vec::new();
        for index in 1..=8 {
            for _ in 0..15 {
                sim.step();
                if !sim.parcels.is_empty() {
                    break;
                }
            }
            let id = format!("source#{index}");
            let parcel = &sim.parcels[&id];
            let local = frame.inverse() * sim.world.bodies[parcel.body].position();
            let bounds = sim.world.colliders[parcel.collider]
                .shape()
                .compute_aabb(&local);
            assert!((local.translation.x + 2.7).abs() < 0.001);
            assert!((local.rotation * Vector::y() - Vector::y()).norm() < 0.0001);
            assert!(bounds.mins.z >= -0.496 && bounds.maxs.z <= 0.496);
            assert!((bounds.mins.y - 0.125).abs() < 0.002);
            positions.push(local);
            sim.remove_parcel(&id);
        }
        assert!(positions.iter().any(|at| at.translation.z < -0.1));
        assert!(positions.iter().any(|at| at.translation.z > 0.1));
        assert!(positions[0].rotation.angle_to(&positions[1].rotation) > 0.05);

        sim.reset()?;
        sim.step();
        let body = sim.parcels["source#1"].body;
        assert_eq!(
            frame.inverse() * sim.world.bodies[body].position(),
            positions[0]
        );
        Ok(())
    }

    #[test]
    fn blocked_random_feed_keeps_the_selected_recipe_product() -> Result<()> {
        let mut project = example();
        project.remove_part("guide");
        project.remove_part("exit");
        project
            .initial_commands
            .insert("line.enabled".into(), Value::Bool(false));
        if let PartKind::Emitter { emission, .. } = &mut project.parts[1].kind {
            *emission = Some(production::Emission::Counts {
                rows: vec![
                    production::CountRow {
                        product: Some("raw-blank".into()),
                        count: 1,
                    },
                    production::CountRow {
                        product: Some("machined-base".into()),
                        count: 1,
                    },
                ],
                skip: None,
            });
        }
        let mut clear = Simulation::new(project.clone())?;
        clear.step();
        let first = clear.parcels["source#1"].product.clone();
        project.parts.push(Part {
            id: "obstruction".into(),
            name: "Obstruction".into(),
            position: [-2.9, 0.86, 0.0],
            yaw: 0.0,
            product: None,
            kind: PartKind::Crate {
                size: [0.8, 0.3, 0.98],
                mass: 2.0,
            },
        });
        let mut sim = Simulation::new(project)?;
        for _ in 0..30 {
            sim.step();
        }
        assert!(flag(&sim.tags, "source", "spawn_blocked"));
        assert_eq!(count(&sim.tags, "source"), 0);
        sim.remove_parcel("obstruction");
        for _ in 0..12 {
            sim.step();
        }
        assert!(!flag(&sim.tags, "source", "spawn_blocked"));
        assert_eq!(sim.parcels["source#1"].product, first);
        Ok(())
    }

    #[test]
    fn passive_rails_physically_center_and_turn_an_off_axis_product() -> Result<()> {
        let mut project = example();
        project.remove_part("exit");
        project.initial_commands.remove("source.enabled");
        project.parts[1].kind = PartKind::Crate {
            size: [0.38, 0.13, 0.22],
            mass: 1.0,
        };
        project.parts[1].position = [-2.9, 0.78, -0.30];
        project.parts[1].yaw = 23.0;
        if let PartKind::FlowGuide { outlet, .. } = &mut project.parts[2].kind {
            *outlet = 0.28;
        }
        let mut guided = Simulation::new(project.clone())?;
        project.remove_part("guide");
        let mut unguided = Simulation::new(project)?;
        for _ in 0..800 {
            guided.step();
            unguided.step();
        }
        let p = &guided.world.bodies[guided.parcels["source"].body];
        assert!(p.translation().x > 1.72, "{:?}", p.position());
        assert!(p.translation().z.abs() < 0.05, "{:?}", p.position());
        assert!((p.rotation() * Vector::x()).x.abs() > 0.99);
        let p = &unguided.world.bodies[unguided.parcels["source"].body];
        assert!((p.translation().z + 0.30).abs() < 0.03);
        Ok(())
    }
}
