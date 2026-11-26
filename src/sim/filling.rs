use std::collections::{BTreeMap, BTreeSet};

use rapier3d::prelude::{Point, Vector};
use serde::Serialize;

use super::{
    Simulation, flag,
    fluid::{Flow, PortFlows, volume_rate},
    number, pose, tag_id,
};
use crate::project::{PartKind, Value};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Contents {
    pub capacity_l: f32,
    pub volume_l: f64,
    pub temperature_c: f64,
    pub dirt_mg_l: f64,
}

impl Contents {
    pub(super) fn new(capacity_l: f32) -> Self {
        Self {
            capacity_l,
            volume_l: 0.0,
            temperature_c: 0.0,
            dirt_mg_l: 0.0,
        }
    }

    fn add(&mut self, amount_l: f64, flow: Flow) {
        let next = (self.volume_l + amount_l).min(f64::from(self.capacity_l));
        if next > self.volume_l {
            let share = (next - self.volume_l) / next;
            self.temperature_c += share * (flow.temperature_c - self.temperature_c);
            self.dirt_mg_l += share * (flow.dirt_mg_l - self.dirt_mg_l);
            self.volume_l = next;
        }
    }
}

pub(super) struct Batch {
    parcel: Option<String>,
    target_l: f64,
    pub limit_l_s: f64,
}

impl Simulation {
    pub(super) fn filler_batches(&self, dt: f64) -> BTreeMap<String, Batch> {
        let mut reserved = BTreeSet::new();
        let mut batches = BTreeMap::new();
        for part in &self.project.parts {
            let PartKind::WaterFiller { size, .. } = part.kind else {
                continue;
            };
            let inverse = pose(part.position, part.yaw).inverse();
            let selected = self
                .parcels
                .iter()
                .filter_map(|(id, parcel)| {
                    let contents = parcel.liquid?;
                    if parcel.owner.is_some() || reserved.contains(id) || parcel.scrap {
                        return None;
                    }
                    let local =
                        inverse * Point::from(*self.world.bodies[parcel.body].translation());
                    let rim = local.y + parcel.size[1] * 0.5;
                    let upright = (self.world.bodies[parcel.body].rotation() * Vector::y()).y > 0.9;
                    (upright
                        && local.x.abs() <= size[0] * 0.4
                        && local.z.abs() <= size[2] * 0.4
                        && (-size[1] * 1.5..=0.0).contains(&rim))
                    .then_some((id, contents, local.x.powi(2) + local.z.powi(2)))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2));
            let mut batch = Batch {
                parcel: None,
                limit_l_s: 0.0,
                target_l: 0.0,
            };
            if let Some((id, contents, _)) = selected {
                batch.parcel = Some(id.clone());
                batch.target_l =
                    f64::from(number(&self.tags, &part.id, "target_l").min(contents.capacity_l));
                if flag(&self.tags, &part.id, "fill") {
                    batch.limit_l_s =
                        volume_rate((batch.target_l - contents.volume_l).max(0.0), dt);
                    reserved.insert(id.clone());
                }
            }
            batches.insert(part.id.clone(), batch);
        }
        batches
    }

    pub(super) fn apply_filling(
        &mut self,
        batches: BTreeMap<String, Batch>,
        flows: &BTreeMap<String, PortFlows>,
        dt: f64,
        sensors: &mut Vec<(String, Value)>,
    ) {
        for (id, batch) in batches {
            let mut volume_l = 0.0;
            if let Some(parcel) = batch
                .parcel
                .as_ref()
                .and_then(|id| self.parcels.get_mut(id))
            {
                let contents = parcel.liquid.as_mut().unwrap();
                let flow = flows.get(&id).map_or(Flow::default(), |ports| ports.inlet);
                let previous = contents.volume_l;
                contents.add(
                    (flow.rate_l_s * dt).min((batch.target_l - previous).max(0.0)),
                    flow,
                );
                volume_l = contents.volume_l;
                if volume_l > previous {
                    self.world.colliders[parcel.collider]
                        .set_mass(parcel.dry_mass + volume_l as f32);
                }
            }
            let present = batch.parcel.is_some();
            sensors.extend([
                (tag_id(&id, "container_present"), Value::Bool(present)),
                (tag_id(&id, "filled_l"), Value::Float(volume_l as f32)),
                (
                    tag_id(&id, "dose_done"),
                    Value::Bool(
                        present && batch.target_l > 0.0 && volume_l + 1e-9 >= batch.target_l,
                    ),
                ),
            ]);
        }
    }
}
