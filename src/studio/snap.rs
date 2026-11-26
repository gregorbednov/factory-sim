use glam::{Quat, Vec3};

use crate::project::{Part, PartKind, Project, Value};

const SNAP_RADIUS: f32 = 0.28;
const HEIGHT_TOLERANCE: f32 = 0.08;
const ANGLE_TOLERANCE: f32 = 12.0;
const HALF_GAP: f32 = 0.005;

pub(super) struct Placement {
    pub position: Vec3,
    pub yaw: f32,
}

pub(super) struct Snap {
    pub placement: Placement,
    pub marker: Vec3,
    pub distance: f32,
}

struct Port {
    local: Vec3,
    flow: Vec3,
    input: bool,
}

struct Deck {
    size: Vec3,
    flow: Vec3,
}

impl Deck {
    fn new(project: &Project, part: &Part) -> Option<Self> {
        let size = Vec3::from(part.kind.feed_surface()?);
        let command = |role| project.initial_commands.get(&format!("{}.{role}", part.id));
        let reverse = match part.kind {
            PartKind::Conveyor { .. } => {
                matches!(command("target_speed"), Some(Value::Float(speed)) if *speed < 0.0)
            }
            _ => {
                matches!(command("reverse"), Some(Value::Bool(true)))
                    && !matches!(command("forward"), Some(Value::Bool(true)))
            }
        };
        let flow = if reverse { Vec3::NEG_X } else { Vec3::X };
        Some(Self { size, flow })
    }

    fn ports(&self) -> Vec<Port> {
        [true, false]
            .into_iter()
            .map(|input| {
                let sign = if input { -1.0 } else { 1.0 };
                Port {
                    local: Vec3::Y * (self.size.y * 0.5)
                        + self.flow * sign * (self.size.x * 0.5 + HALF_GAP),
                    flow: self.flow,
                    input,
                }
            })
            .collect()
    }
}

fn ports(part: &Part, deck: Option<&Deck>) -> Vec<Port> {
    if let Some(deck) = deck {
        return deck.ports();
    }
    let PartKind::Machine { machine } = &part.kind else {
        return Vec::new();
    };
    [
        (machine.intake, true),
        (machine.outlet, false),
        (machine.reject, false),
    ]
    .into_iter()
    .filter(|(_, input)| *input || machine.unload_robot.is_none())
    .filter_map(|(at, input)| {
        let local = Vec3::from(at);
        let outward = Vec3::new(local.x, 0.0, local.z).try_normalize()?;
        let pad_edge = (outward.x.abs() + outward.z.abs()) * 0.225;
        Some(Port {
            local: local + outward * (pad_edge + HALF_GAP),
            flow: if input { -outward } else { outward },
            input,
        })
    })
    .collect()
}

fn rotation(yaw: f32) -> Quat {
    Quat::from_rotation_y(yaw.to_radians())
}

fn align_yaw(moving: Vec3, fixed: Vec3, yaw: f32) -> Option<f32> {
    let angle = |axis: Vec3| (-axis.z).atan2(axis.x).to_degrees();
    let delta = (angle(fixed) - angle(rotation(yaw) * moving) + 180.0).rem_euclid(360.0) - 180.0;
    (delta.abs() <= ANGLE_TOLERANCE).then_some(yaw + delta)
}

fn consider(best: &mut Option<Snap>, raw: &Placement, mut position: Vec3, yaw: f32, marker: Vec3) {
    position.y = raw.position.y;
    let distance = (position - raw.position).length();
    if distance <= SNAP_RADIUS
        && best
            .as_ref()
            .is_none_or(|candidate| distance < candidate.distance)
        && position.x.abs() <= 1000.0
        && position.z.abs() <= 1000.0
    {
        *best = Some(Snap {
            placement: Placement { position, yaw },
            marker,
            distance,
        });
    }
}

// From the equipment's current location, find a point along the belt that suits its role.
fn mount(
    part: &Part,
    position: Vec3,
    yaw: f32,
    belt: &Part,
    belt_placement: &Placement,
    deck: &Deck,
) -> Option<(Vec3, f32, Vec3)> {
    let belt_rotation = rotation(belt_placement.yaw);
    let local = belt_rotation.inverse() * (position - belt_placement.position);
    let deck_y = deck.size.y * 0.5;
    let half_length = deck.size.x * 0.5;
    let (x, aligned) = match &part.kind {
        PartKind::FlowGuide {
            length,
            height,
            inlet,
            ..
        } if *length <= deck.size.x
            && *inlet <= deck.size.z + 0.05
            && (local.y - height * 0.5 - deck_y).abs() <= HEIGHT_TOLERANCE =>
        {
            let travel = half_length - length * 0.5;
            (
                local.x.clamp(-travel, travel),
                align_yaw(Vec3::X, belt_rotation * deck.flow, yaw)?,
            )
        }
        PartKind::Emitter {
            random_spawn_belt, ..
        } if random_spawn_belt.as_ref().is_none_or(|id| *id == belt.id)
            && (0.02..=0.60).contains(&(local.y - deck_y)) =>
        {
            let margin = half_length.min(0.12);
            (
                local.x.clamp(-half_length + margin, half_length - margin),
                yaw,
            )
        }
        PartKind::Remover { size } if (local.y - deck_y - 0.08).abs() <= size[1] * 0.5 => {
            ((half_length - half_length.min(0.10)) * deck.flow.x, yaw)
        }
        _ => return None,
    };
    let at = |y| belt_placement.position + belt_rotation * Vec3::new(x, y, 0.0);
    Some((at(local.y), aligned, at(deck_y)))
}

pub(super) fn find(project: &Project, moving: &Part, raw: &Placement) -> Option<Snap> {
    let moving_deck = Deck::new(project, moving);
    let moving_ports = ports(moving, moving_deck.as_ref());
    let mut links = None;
    let mut mounts = None;
    for other in &project.parts {
        if other.id == moving.id {
            continue;
        }
        let other_deck = Deck::new(project, other);
        let fixed = Placement {
            position: Vec3::from(other.position),
            yaw: other.yaw,
        };
        let fixed_rotation = rotation(fixed.yaw);

        if moving_deck.is_some() || other_deck.is_some() {
            for port in ports(other, other_deck.as_ref()) {
                let marker = fixed.position + fixed_rotation * port.local;
                for own in moving_ports.iter().filter(|own| own.input != port.input) {
                    if (raw.position.y + own.local.y - marker.y).abs() > HEIGHT_TOLERANCE {
                        continue;
                    }
                    if let Some(yaw) = align_yaw(own.flow, fixed_rotation * port.flow, raw.yaw) {
                        consider(
                            &mut links,
                            raw,
                            marker - rotation(yaw) * own.local,
                            yaw,
                            marker,
                        );
                    }
                }
            }
        }

        if let Some(deck) = &other_deck
            && let Some((position, yaw, mark)) =
                mount(moving, raw.position, raw.yaw, other, &fixed, deck)
        {
            consider(&mut mounts, raw, position, yaw, mark);
        }
        if let Some(deck) = &moving_deck
            && let Some((position, yaw, mark)) = mount(other, fixed.position, fixed.yaw, moving, raw, deck)
            // Moving the belt can center an accessory, while its authored orientation stays fixed.
            && (yaw - fixed.yaw).abs() <= 0.01
        {
            let correction = fixed.position - position;
            consider(
                &mut mounts,
                raw,
                raw.position + correction,
                raw.yaw,
                mark + correction,
            );
        }
    }
    links.or(mounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        let mut project = Project::demo();
        project.parts.clear();
        project.initial_commands.clear();
        project
    }

    fn belt(id: &str, position: [f32; 3], yaw: f32) -> Part {
        Part {
            id: id.into(),
            name: id.into(),
            position,
            yaw,
            product: None,
            kind: PartKind::Conveyor {
                size: [2.0, 0.22, 1.0],
                speed: 1.0,
            },
        }
    }

    #[test]
    fn snaps_aligned_belts_at_deck_ends_and_preserves_height() {
        let mut project = project();
        let moving = belt("moving", [0.0; 3], 6.0);
        project.parts.push(belt("fixed", [0.0, 0.59, 0.0], 0.0));
        let mut raw = Placement {
            position: Vec3::new(2.14, 0.56, 0.09),
            yaw: 6.0,
        };
        let aligned = find(&project, &moving, &raw).unwrap();
        assert!(
            aligned
                .placement
                .position
                .distance(Vec3::new(2.01, 0.56, 0.0))
                < 0.001
        );
        assert!(aligned.placement.yaw.abs() < 0.001);

        raw.position.y = 0.8;
        assert!(find(&project, &moving, &raw).is_none());
        raw.position.y = 0.56;
        raw.yaw = 180.0;
        assert!(find(&project, &moving, &raw).is_none());
        project
            .initial_commands
            .insert("moving.target_speed".into(), Value::Float(-0.5));
        assert!(find(&project, &moving, &raw).is_some());
    }

    #[test]
    fn guide_snaps_to_a_rotated_deck_and_remains_above_it() {
        let mut project = project();
        project.parts.push(belt("fixed", [2.0, 0.59, -1.0], 90.0));
        let moving = Part {
            kind: PartKind::FlowGuide {
                length: 1.5,
                height: 0.18,
                inlet: 0.84,
                outlet: 0.44,
            },
            ..belt("guide", [0.0; 3], 85.0)
        };
        let raw = Placement {
            position: Vec3::new(2.16, 0.80, -0.80),
            yaw: 85.0,
        };
        let aligned = find(&project, &moving, &raw).unwrap();
        assert!(
            aligned
                .placement
                .position
                .distance(Vec3::new(2.0, 0.80, -0.80))
                < 0.001
        );
        assert!((aligned.placement.yaw - 90.0).abs() < 0.001);
    }
}
