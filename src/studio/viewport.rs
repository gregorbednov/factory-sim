use std::{collections::BTreeMap, sync::Arc};

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use glam::{Mat4, Quat, Vec3};

use super::{
    geometry::cylinder_mesh,
    i18n::Language,
    render::{self, GraphicsQuality, Instance, MeshBatch},
    snap,
};

use crate::{
    product::{Mesh, ProductSpec},
    production::{Equipment, StaffRole},
    project::{Part, PartKind, Pipe, Project, Value, WaterMeasurement, water_pipe_points},
    sim::Snapshot,
};

#[path = "thumbnail.rs"]
mod thumbnail;

pub(super) fn part_thumbnail(
    project: &Project,
    part: &Part,
    snapshot: &Snapshot,
) -> egui::ColorImage {
    let mut scene = Scene::default();
    scene.part(project, part, snapshot, false, true);
    thumbnail::render(&scene)
}

const STEEL: Color32 = Color32::from_rgb(151, 169, 178);
const ALUMINUM: Color32 = Color32::from_rgb(193, 199, 202);
const DRIVE: Color32 = Color32::from_rgb(54, 89, 113);
const GUARD: Color32 = Color32::from_rgb(226, 176, 68);
const DARK: Color32 = Color32::from_rgb(49, 59, 64);
const ACTIVE: Color32 = Color32::from_rgb(109, 221, 189);
const SELECT: Color32 = Color32::from_rgb(246, 190, 110);
const WATER_PIPE: Color32 = Color32::from_rgb(75, 144, 222);
const PIPE_DIAMETER_M: f32 = 0.065;
const FLOW_MARKER: Color32 = Color32::from_rgb(133, 234, 250);

pub struct Camera {
    yaw: f32,
    pitch: f32,
    zoom: f32,
    target: Vec3,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.43,
            pitch: 0.66,
            zoom: 1.0,
            target: Vec3::new(0.0, 0.35, 1.0),
        }
    }
}

#[derive(Clone, Copy)]
struct View {
    right: Vec3,
    up: Vec3,
    eye: Vec3,
    center: Pos2,
    target: Vec3,
    scale: f32,
    size: Vec2,
}

impl View {
    fn new(rect: Rect, camera: &Camera) -> Self {
        let (sin, cos) = camera.yaw.sin_cos();
        let (v, h) = camera.pitch.sin_cos();
        let eye = Vec3::new(sin * h, v, cos * h);
        let right = Vec3::Y.cross(eye).normalize();
        let up = eye.cross(right);
        Self {
            right,
            up,
            eye,
            center: rect.center(),
            size: rect.size(),
            target: camera.target,
            scale: (rect.width() / 12.5).min(rect.height() / 9.5).max(8.0) * camera.zoom,
        }
    }

    fn project(self, point: Vec3) -> Pos2 {
        let offset = point - self.target;
        self.center + Vec2::new(self.right.dot(offset), -self.up.dot(offset)) * self.scale
    }

    fn matrix(self) -> Mat4 {
        let horizontal = self.size.x / (2.0 * self.scale);
        let vertical = self.size.y / (2.0 * self.scale);
        Mat4::orthographic_rh(-horizontal, horizontal, -vertical, vertical, 10.0, 4000.0)
            * Mat4::look_at_rh(self.target + self.eye * 1600.0, self.target, Vec3::Y)
    }

    fn ground_delta(self, delta: Vec2) -> Vec3 {
        let up = Vec3::new(self.up.x, 0.0, self.up.z);
        self.right * (delta.x / self.scale) - up * (delta.y / (self.scale * up.length_squared()))
    }
}

const METAL: [f32; 4] = [0.33, 0.64, 0.0, 0.0];
const RUBBER: [f32; 4] = [0.79, 0.03, 2.0, 0.0];
const CONCRETE: [f32; 4] = [0.72, 0.0, 1.0, 0.0];
const CARDBOARD: [f32; 4] = [0.86, 0.0, 3.0, 0.0];
const POLYMER: [f32; 4] = [0.45, 0.0, 0.0, 0.0];

#[derive(Default)]
struct Scene<'a> {
    instances: Vec<Instance>,
    cylinders: Vec<Instance>,
    windows: Vec<Instance>,
    meshes: BTreeMap<&'a str, (&'a Arc<Mesh>, Vec<Instance>)>,
}

impl<'a> Scene<'a> {
    fn block(&mut self, center: Vec3, size: Vec3, rotation: Quat, color: Color32, highlight: bool) {
        let material = if color == ACTIVE {
            [0.31, 0.0, 0.0, 0.75]
        } else if color == STEEL {
            METAL
        } else {
            POLYMER
        };
        self.surface(center, size, rotation, color, material, 0.0, highlight);
    }

    #[allow(clippy::too_many_arguments)]
    fn surface(
        &mut self,
        center: Vec3,
        size: Vec3,
        rotation: Quat,
        color: Color32,
        material: [f32; 4],
        phase: f32,
        highlight: bool,
    ) {
        self.instances.push(Self::instance(
            center, size, rotation, color, material, phase, highlight,
        ));
    }

    fn instance(
        center: Vec3,
        size: Vec3,
        rotation: Quat,
        color: Color32,
        material: [f32; 4],
        phase: f32,
        highlight: bool,
    ) -> Instance {
        let linear_color = |color: Color32| {
            let channel = |v| (f32::from(v) / 255.0).powf(2.2);
            Vec3::new(channel(color.r()), channel(color.g()), channel(color.b()))
        };
        let mut color = linear_color(color);
        if highlight {
            color = color.lerp(linear_color(SELECT), 0.54);
        }
        Instance {
            center: center.extend(0.0).to_array(),
            axis_x: (rotation * Vec3::X * size.x).extend(0.0).to_array(),
            axis_y: (rotation * Vec3::Y * size.y).extend(0.0).to_array(),
            axis_z: (rotation * Vec3::Z * size.z).extend(0.0).to_array(),
            color: color.extend(phase).to_array(),
            surface: material,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn cylinder(
        &mut self,
        center: Vec3,
        size: Vec3,
        rotation: Quat,
        color: Color32,
        material: [f32; 4],
        highlight: bool,
    ) {
        self.cylinders.push(Self::instance(
            center, size, rotation, color, material, 0.0, highlight,
        ));
    }

    fn window(&mut self, center: Vec3, size: Vec3, rotation: Quat, highlight: bool) {
        self.windows.push(Self::instance(
            center,
            size,
            rotation,
            Color32::from_rgb(108, 163, 177),
            POLYMER,
            0.0,
            highlight,
        ));
    }

    fn parcel(
        &mut self,
        position: Vec3,
        size: Vec3,
        rotation: Quat,
        product: Option<(&'a str, ProductSpec<'a>)>,
        selected: bool,
    ) {
        if let Some((id, spec)) = product {
            let [r, g, b] = spec.color;
            let color = Color32::from_rgb(r, g, b);
            if let Some(mesh) = spec.mesh {
                self.meshes
                    .entry(id)
                    .or_insert_with(|| (mesh, Vec::new()))
                    .1
                    .push(Self::instance(
                        position,
                        Vec3::ONE,
                        rotation,
                        color,
                        POLYMER,
                        0.0,
                        selected,
                    ));
                return;
            }
            if spec.liquid_capacity_l.is_some() {
                let bottle = size.x < 0.1;
                if bottle {
                    self.cylinder(
                        position - rotation * Vec3::Y * size.y * 0.1,
                        size * Vec3::new(1.0, 0.8, 1.0),
                        rotation,
                        color,
                        POLYMER,
                        selected,
                    );
                } else {
                    self.block(
                        position - rotation * Vec3::Y * size.y * 0.09,
                        size * Vec3::new(1.0, 0.82, 1.0),
                        rotation,
                        color,
                        selected,
                    );
                }
                self.cylinder(
                    position + rotation * Vec3::Y * size.y * 0.39,
                    size * Vec3::new(0.43, 0.22, 0.43),
                    rotation,
                    color,
                    POLYMER,
                    selected,
                );
                return;
            }
            if !spec.cardboard {
                self.block(
                    position - rotation * Vec3::Y * (size.y * 0.045),
                    size * Vec3::new(0.94, 0.91, 0.94),
                    rotation,
                    color,
                    selected,
                );
                for (y, band) in [(-0.43, 0.14), (0.44, 0.12)] {
                    self.block(
                        position + rotation * Vec3::Y * (size.y * y),
                        size * Vec3::new(1.0, band, 1.0),
                        rotation,
                        color.gamma_multiply(0.82),
                        selected,
                    );
                }
                for sign in [-1.0, 1.0] {
                    self.block(
                        position + rotation * (size * Vec3::new(0.0, 0.26, sign * 0.476)),
                        size * Vec3::new(0.36, 0.13, 0.016),
                        rotation,
                        DARK,
                        selected,
                    );
                }
                return;
            }
        }
        let color = product.map_or([190, 143, 93], |(_, spec)| spec.color);
        self.surface(
            position,
            size,
            rotation,
            Color32::from_rgb(color[0], color[1], color[2]),
            CARDBOARD,
            0.0,
            selected,
        );
        self.block(
            position + rotation * Vec3::Y * (size.y * 0.5 + 0.0025),
            Vec3::new(size.x * 0.20, 0.004, size.z * 0.995),
            rotation,
            Color32::from_rgb(226, 201, 151),
            false,
        );
        self.block(
            position + rotation * Vec3::new(size.x * 0.21, size.y * 0.5 + 0.003, size.z * 0.20),
            Vec3::new(size.x * 0.25, 0.005, size.z * 0.28),
            rotation,
            Color32::from_rgb(231, 226, 214),
            false,
        );
    }

    fn water_ports(&mut self, part: &Part, highlight: bool) {
        for outlet in [false, true] {
            for index in 0..part.kind.water_port_count(outlet) {
                let (at, outward) = part.water_port_at(outlet, index).unwrap();
                self.cylinder(
                    Vec3::from(at.map(|value| value as f32)),
                    Vec3::new(0.105, 0.045, 0.105),
                    Quat::from_rotation_arc(Vec3::Y, Vec3::from(outward.map(|value| value as f32))),
                    if outlet { WATER_PIPE } else { STEEL },
                    METAL,
                    highlight,
                );
            }
        }
    }

    fn water_pipe(&mut self, pipe: &Pipe, from: &Part, to: &Part, snapshot: &Snapshot) {
        let Some(points) = water_pipe_points(from, pipe.from_port, to, pipe.to_port) else {
            return;
        };
        let points = points.map(|point| Vec3::from(point.map(|value| value as f32)));
        let length: f32 = points.windows(2).map(|leg| leg[0].distance(leg[1])).sum();
        let volume_l = snapshot
            .water_outlet_volume_l
            .get(&(from.id.clone(), pipe.from_port))
            .copied()
            .unwrap_or(0.0);
        let mut markers = PipeMarkers::new(
            length,
            PIPE_DIAMETER_M,
            if from.kind.water_port_count(true) > 1 {
                water_number(
                    from,
                    snapshot,
                    if pipe.from_port == 0 {
                        "outlet_1_l_s"
                    } else {
                        "outlet_2_l_s"
                    },
                )
            } else {
                water_flow(from, snapshot)
            },
            volume_l,
        );
        for leg in points.windows(2) {
            let delta = leg[1] - leg[0];
            let Some(direction) = delta.try_normalize() else {
                continue;
            };
            let length = delta.length();
            let angle = Quat::from_rotation_arc(Vec3::Y, direction);
            self.cylinder(
                (leg[0] + leg[1]) * 0.5,
                Vec3::new(PIPE_DIAMETER_M, length + 0.01, PIPE_DIAMETER_M),
                angle,
                WATER_PIPE,
                POLYMER,
                false,
            );
            if let Some(markers) = &mut markers {
                while markers.next_m < length {
                    self.cylinder(
                        leg[0] + direction * markers.next_m,
                        markers.size,
                        angle,
                        FLOW_MARKER,
                        POLYMER,
                        false,
                    );
                    markers.next_m += markers.spacing_m;
                }
                markers.next_m -= length;
            }
        }
    }

    fn part(
        &mut self,
        project: &'a Project,
        part: &'a Part,
        snapshot: &Snapshot,
        highlight: bool,
        preview: bool,
    ) {
        let product = part
            .product
            .as_deref()
            .and_then(|id| project.product(id).map(|spec| (id, spec)));
        let center = Vec3::from(part.position);
        let rotation = Quat::from_rotation_y(part.yaw.to_radians());
        let box_at = |scene: &mut Self, offset, size, color| {
            scene.block(
                center + rotation * Vec3::from(offset),
                Vec3::from(size),
                rotation,
                color,
                highlight,
            );
        };
        let cylinder_at = |scene: &mut Self, offset, size, tilt, color, material| {
            scene.cylinder(
                center + rotation * Vec3::from(offset),
                Vec3::from(size),
                rotation * tilt,
                color,
                material,
                highlight,
            );
        };
        let along_x = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
        let along_z = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        match part.kind {
            PartKind::Conveyor {
                size: [length, height, width],
                ..
            } => {
                let top = height * 0.5;
                let frame = height.min(0.24);
                let pulley = frame.min(length * 0.28);
                let beam = (width * 0.09).min(0.085);
                for sign in [-1.0, 1.0] {
                    let z = sign * (width - beam) * 0.5;
                    box_at(
                        self,
                        [0.0, top - frame * 0.53, z],
                        [length, frame * 0.96, beam],
                        STEEL,
                    );
                    box_at(
                        self,
                        [0.0, top - frame * 0.56, z + sign * beam * 0.46],
                        [length * 0.95, frame * 0.16, beam * 0.12],
                        DARK,
                    );
                    let leg_height = center.y + top - frame;
                    if leg_height > 0.035 {
                        let post = width.min(length).clamp(0.025, 0.075);
                        for x in [-length * 0.34, length * 0.34] {
                            box_at(
                                self,
                                [x, top - frame - leg_height * 0.5, z * 0.88],
                                [post, leg_height, post],
                                STEEL,
                            );
                            box_at(
                                self,
                                [x, 0.013 - center.y, z * 0.88],
                                [post * 2.3, 0.026, post * 2.0],
                                DARK,
                            );
                        }
                        box_at(
                            self,
                            [0.0, top - frame - leg_height * 0.46, z * 0.88],
                            [length * 0.68, post * 0.60, post * 0.60],
                            STEEL,
                        );
                    }
                }
                let drive_x = (length - pulley) * 0.5;
                for x in [-drive_x, drive_x] {
                    cylinder_at(
                        self,
                        [x, top - pulley * 0.5, 0.0],
                        [pulley, width, pulley],
                        along_z,
                        DARK,
                        POLYMER,
                    );
                }
                let motor = height.clamp(0.08, 0.24);
                box_at(
                    self,
                    [drive_x, top - frame * 0.6, width * 0.5],
                    [motor, motor, motor * 0.70],
                    DRIVE,
                );
                cylinder_at(
                    self,
                    [drive_x, top - frame * 0.60, width * 0.5 + motor * 0.72],
                    [motor, motor * 1.3, motor],
                    along_z,
                    DRIVE,
                    METAL,
                );
                for offset in [0.35, 0.58, 0.81, 1.04] {
                    cylinder_at(
                        self,
                        [drive_x, top - frame * 0.60, width * 0.5 + motor * offset],
                        [motor * 1.1, motor * 0.08, motor * 1.1],
                        along_z,
                        DRIVE,
                        POLYMER,
                    );
                }
                let phase = if preview {
                    0.0
                } else {
                    snapshot.belt_travel.get(&part.id).copied().unwrap_or(0.0)
                };
                let belt_thickness = 0.004_f32.min(pulley * 0.20);
                for y in [
                    top - belt_thickness * 0.5,
                    top - pulley + belt_thickness * 0.5,
                ] {
                    self.surface(
                        center + rotation * Vec3::new(0.0, y, 0.0),
                        Vec3::new(length - pulley, belt_thickness, width - beam * 1.8),
                        rotation,
                        DARK,
                        RUBBER,
                        phase,
                        highlight,
                    );
                }
            }
            PartKind::Pusher { .. } => {
                let p = if preview {
                    0.0
                } else {
                    float(snapshot, &part.id, "position")
                };
                box_at(self, [0.0, -0.045, -0.23], [0.46, 0.09, 0.40], DARK);
                cylinder_at(
                    self,
                    [0.0, 0.0, -0.24],
                    [0.18, 0.37, 0.18],
                    along_z,
                    ALUMINUM,
                    METAL,
                );
                for z in [-0.42, -0.07] {
                    box_at(self, [0.0, 0.0, z], [0.20, 0.20, 0.045], DRIVE);
                }
                for (x, diameter) in [(-0.16, 0.042), (0.0, 0.071), (0.16, 0.042)] {
                    cylinder_at(
                        self,
                        [x, 0.0, p * 0.5 - 0.045],
                        [diameter, p + 0.09, diameter],
                        along_z,
                        ALUMINUM,
                        METAL,
                    );
                }
                box_at(self, [0.0, 0.0, p - 0.006], [0.65, 0.32, 0.088], GUARD);
                box_at(self, [0.0, 0.0, p + 0.044], [0.61, 0.28, 0.012], DARK);
                if center.y > 0.13 {
                    box_at(
                        self,
                        [0.0, (0.09 - center.y) * 0.5, -0.21],
                        [0.12, center.y - 0.09, 0.12],
                        STEEL,
                    );
                }
            }
            PartKind::Sensor { range } => {
                let detected = !preview && flag(snapshot, &part.id, "detected");
                let distance = if detected {
                    float(snapshot, &part.id, "distance")
                } else {
                    range
                };
                let housing = Color32::from_rgb(218, 119, 44);
                box_at(self, [-0.018, 0.0, 0.0], [0.12, 0.105, 0.083], housing);
                box_at(self, [0.041, 0.0, 0.0], [0.019, 0.096, 0.075], DARK);
                for y in [-0.021, 0.021] {
                    cylinder_at(
                        self,
                        [0.052, y, 0.0],
                        [0.025, 0.007, 0.025],
                        along_x,
                        Color32::from_rgb(97, 137, 156),
                        [0.16, 0.35, 0.0, 0.0],
                    );
                }
                box_at(
                    self,
                    [-0.03, 0.055, 0.0],
                    [0.026, 0.004, 0.019],
                    if detected { ACTIVE } else { DARK },
                );
                if center.y > 0.08 {
                    box_at(
                        self,
                        [-0.045, (-center.y - 0.052) * 0.5, 0.0],
                        [0.035, center.y - 0.052, 0.035],
                        STEEL,
                    );
                    box_at(
                        self,
                        [-0.045, 0.01 - center.y, 0.0],
                        [0.10, 0.02, 0.10],
                        DARK,
                    );
                }
                if highlight && distance > 0.056 {
                    let start = 0.056;
                    box_at(
                        self,
                        [(distance + start) * 0.5, 0.0, 0.0],
                        [distance - start, 0.006, 0.006],
                        if detected { SELECT } else { ACTIVE },
                    );
                }
            }
            PartKind::Emitter { size, .. } => {
                let [_, height, width] = product.map_or(size, |(_, spec)| spec.size);
                let span = crate::production::emitter_opening(width);
                let clearance = height * 0.5 + 0.13;
                for side in [-1.0, 1.0] {
                    let z = side * span * 0.5;
                    box_at(self, [0.0, clearance - 0.33, z], [0.09, 0.66, 0.085], STEEL);
                    box_at(self, [0.0, clearance - 0.66, z], [0.18, 0.035, 0.13], DARK);
                }
                box_at(
                    self,
                    [0.0, clearance, 0.0],
                    [0.16, 0.10, span + 0.09],
                    DRIVE,
                );
                box_at(
                    self,
                    [0.0, clearance - 0.054, 0.0],
                    [0.145, 0.014, span * 0.77],
                    DARK,
                );
                let blocked = !preview
                    && (flag(snapshot, &part.id, "spawn_blocked")
                        || flag(snapshot, &part.id, "recipe_error"));
                let enabled = !preview && flag(snapshot, &part.id, "enabled");
                box_at(
                    self,
                    [0.083, clearance, span * 0.34],
                    [0.012, 0.02, 0.068],
                    if blocked {
                        GUARD
                    } else if enabled {
                        ACTIVE
                    } else {
                        DARK
                    },
                );
            }
            PartKind::Remover {
                size: [length, height, width],
            } => {
                let post = width.min(length).clamp(0.024, 0.075);
                let hood = (height * 0.17).clamp(0.04, 0.13);
                for sign in [-1.0, 1.0] {
                    box_at(
                        self,
                        [0.0, 0.0, sign * width * 0.5],
                        [post, height, post],
                        STEEL,
                    );
                    box_at(
                        self,
                        [0.0, -height * 0.47, sign * width * 0.5],
                        [post * 2.1, height * 0.06, post * 1.7],
                        DARK,
                    );
                }
                box_at(
                    self,
                    [0.0, (height + hood) * 0.5, 0.0],
                    [length, hood, width + post],
                    DRIVE,
                );
                box_at(
                    self,
                    [length * 0.5, (height + hood) * 0.5, width * 0.30],
                    [post * 0.22, hood * 0.34, post],
                    if !preview && flag(snapshot, &part.id, "enabled") {
                        ACTIVE
                    } else {
                        DARK
                    },
                );
            }
            PartKind::Equipment {
                equipment,
                size: [length, height, width],
                ..
            } => {
                use Equipment::*;
                let reading = |role| {
                    if preview {
                        0.0
                    } else {
                        float(snapshot, &part.id, role)
                    }
                };
                let active = |role| !preview && flag(snapshot, &part.id, role);
                let yellow = Color32::from_rgb(226, 176, 68);
                let red = Color32::from_rgb(218, 70, 55);
                match equipment {
                    FreeRoller => {
                        cylinder_at(
                            self,
                            [0.0; 3],
                            [length, width, height],
                            along_z,
                            ALUMINUM,
                            METAL,
                        );
                        for side in [-1.0, 1.0] {
                            box_at(
                                self,
                                [0.0, -height * 0.28, side * width * 0.49],
                                [length * 0.94, height * 0.45, 0.065],
                                DRIVE,
                            );
                        }
                    }
                    Chute => {
                        let tilt =
                            rotation * Quat::from_rotation_z(-crate::production::CHUTE_SLOPE);
                        self.block(
                            center,
                            Vec3::new(length, height, width),
                            tilt,
                            ALUMINUM,
                            highlight,
                        );
                        for (at, size) in crate::production::chute_rails([length, height, width]) {
                            self.block(
                                center + tilt * Vec3::from(at),
                                Vec3::from(size),
                                tilt,
                                STEEL,
                                highlight,
                            );
                        }
                        for x in [-0.36 * length, 0.36 * length] {
                            let deck = center + tilt * Vec3::new(x, -height * 0.5, 0.0);
                            let leg = deck.y.max(0.02);
                            for side in [-1.0, 1.0] {
                                self.block(
                                    deck - Vec3::Y * leg * 0.5
                                        + rotation * Vec3::Z * side * width * 0.36,
                                    Vec3::new(0.075, leg, 0.075),
                                    rotation,
                                    DRIVE,
                                    highlight,
                                );
                            }
                        }
                    }
                    Roller | Scale | Turntable | ChainTransfer => {
                        let top = height * 0.5;
                        let post = (width * 0.07).min(0.075);
                        let leg_height = center.y - top;
                        if equipment == Turntable {
                            let base = width.min(length) * 0.72;
                            cylinder_at(
                                self,
                                [0.0, -top - 0.045, 0.0],
                                [base, 0.09, base],
                                Quat::IDENTITY,
                                DARK,
                                METAL,
                            );
                            if leg_height > 0.09 {
                                cylinder_at(
                                    self,
                                    [0.0, -(center.y + top) * 0.5 - 0.045, 0.0],
                                    [base * 0.58, leg_height - 0.09, base * 0.58],
                                    Quat::IDENTITY,
                                    DRIVE,
                                    POLYMER,
                                );
                            }
                        } else if leg_height > 0.04 {
                            for x in [-length * 0.35, length * 0.35] {
                                for z in [-width * 0.38, width * 0.38] {
                                    box_at(
                                        self,
                                        [x, -top - leg_height * 0.5, z],
                                        [post, leg_height, post],
                                        STEEL,
                                    );
                                    box_at(
                                        self,
                                        [x, 0.012 - center.y, z],
                                        [post * 2.1, 0.024, post * 2.1],
                                        DARK,
                                    );
                                }
                                box_at(
                                    self,
                                    [x, -top - leg_height * 0.38, 0.0],
                                    [post, post * 0.65, width * 0.76],
                                    STEEL,
                                );
                            }
                        }
                        let deck = rotation * Quat::from_rotation_y(reading("angle").to_radians());
                        let mut block = |offset, size, color| {
                            self.block(
                                center + deck * Vec3::from(offset),
                                Vec3::from(size),
                                deck,
                                color,
                                highlight,
                            );
                        };
                        let beam = (width * 0.08).min(0.085);
                        for sign in [-1.0, 1.0] {
                            block(
                                [0.0, -height * 0.13, sign * (width - beam) * 0.5],
                                [length, height * 0.74, beam],
                                STEEL,
                            );
                            block(
                                [0.0, -height * 0.17, sign * width * 0.496],
                                [length * 0.95, height * 0.12, beam * 0.09],
                                DARK,
                            );
                        }
                        for x in [-length * 0.37, length * 0.37] {
                            block(
                                [x, -height * 0.36, 0.0],
                                [length * 0.08, height * 0.24, width - beam],
                                DRIVE,
                            );
                        }
                        if equipment == Scale {
                            self.surface(
                                center + deck * Vec3::Y * (top - 0.013),
                                Vec3::new(length * 0.98, 0.026, width - beam * 1.8),
                                deck,
                                DARK,
                                POLYMER,
                                0.0,
                                highlight,
                            );
                        } else {
                            let count = ((length / 0.11).ceil() as usize).clamp(3, 40);
                            let pitch = length / count as f32;
                            let diameter = (pitch * 0.82).min(height * 0.78);
                            for index in 0..count {
                                let x = pitch * (index as f32 + 0.5) - length * 0.5;
                                self.cylinder(
                                    center + deck * Vec3::new(x, top - diameter * 0.5, 0.0),
                                    Vec3::new(diameter, width - beam * 1.45, diameter),
                                    deck * along_z,
                                    ALUMINUM,
                                    METAL,
                                    highlight,
                                );
                            }
                        }
                        if equipment == ChainTransfer {
                            for x in [-0.35 * length, 0.0, 0.35 * length] {
                                self.block(
                                    center
                                        + rotation
                                            * Vec3::new(
                                                x,
                                                height * 0.5 + reading("position") - 0.012,
                                                0.0,
                                            ),
                                    Vec3::new(length * 0.054, 0.02, width * 0.98),
                                    rotation,
                                    DARK,
                                    highlight,
                                );
                            }
                        }
                        if equipment == Scale {
                            box_at(self, [0.0, 0.07, width * 0.54], [0.23, 0.16, 0.09], DRIVE);
                            box_at(
                                self,
                                [0.0, 0.087, width * 0.589],
                                [0.155, 0.054, 0.01],
                                if reading("weight_kg") > 0.0 {
                                    ACTIVE
                                } else {
                                    DARK
                                },
                            );
                        }
                    }
                    StopBlade => {
                        box_at(
                            self,
                            [0.0, -height - 0.025, 0.0],
                            [length * 1.35, 0.08, width],
                            DARK,
                        );
                        box_at(
                            self,
                            [0.0, reading("position") - height, 0.0],
                            [length, height, width],
                            yellow,
                        );
                    }
                    Capacitive => {
                        cylinder_at(
                            self,
                            [0.0; 3],
                            [height, length, width],
                            along_x,
                            ALUMINUM,
                            METAL,
                        );
                        for x in [-0.27, 0.23] {
                            cylinder_at(
                                self,
                                [length * x, 0.0, 0.0],
                                [height * 1.12, length * 0.13, width * 1.12],
                                along_x,
                                STEEL,
                                METAL,
                            );
                        }
                        cylinder_at(
                            self,
                            [length * 0.47, 0.0, 0.0],
                            [height * 0.91, length * 0.08, width * 0.91],
                            along_x,
                            if active("detected") { ACTIVE } else { DRIVE },
                            [0.33, 0.0, 0.0, f32::from(active("detected")) * 0.65],
                        );
                    }
                    LightArray => {
                        let housing = (length * 0.11).min(0.075);
                        for sign in [-1.0, 1.0] {
                            let x = sign * (length + housing) * 0.5;
                            box_at(self, [x, 0.0, 0.0], [housing, height + 0.1, width], GUARD);
                            box_at(
                                self,
                                [sign * length * 0.5, 0.0, 0.0],
                                [housing * 0.18, height + 0.055, width * 0.72],
                                DARK,
                            );
                        }
                        let mask = match snapshot
                            .tags
                            .get(&format!("{}.mask", part.id))
                            .map(|tag| tag.value)
                        {
                            Some(Value::Int(bits)) if !preview => bits,
                            _ => 0,
                        };
                        for i in 0..8 {
                            if highlight || (mask & (1 << i)) != 0 {
                                box_at(
                                    self,
                                    [0.0, height * (i as f32 / 7.0 - 0.5), 0.0],
                                    [length, 0.007, 0.007],
                                    if (mask & (1 << i)) != 0 { red } else { ACTIVE },
                                );
                            }
                        }
                    }
                    StackLight => {
                        cylinder_at(
                            self,
                            [0.0, -height * 0.45, 0.0],
                            [length * 1.12, height * 0.10, width * 1.12],
                            Quat::IDENTITY,
                            DARK,
                            POLYMER,
                        );
                        cylinder_at(
                            self,
                            [0.0, -height * 0.19, 0.0],
                            [length * 0.32, height * 0.43, width * 0.32],
                            Quat::IDENTITY,
                            ALUMINUM,
                            METAL,
                        );
                        for (index, (role, color)) in
                            [("green", ACTIVE), ("yellow", yellow), ("red", red)]
                                .into_iter()
                                .enumerate()
                        {
                            let y = height * (0.10 + 0.15 * index as f32);
                            let on = active(role);
                            cylinder_at(
                                self,
                                [0.0, y, 0.0],
                                [length, height * 0.138, width],
                                Quat::IDENTITY,
                                if on {
                                    color
                                } else {
                                    color.gamma_multiply(0.50)
                                },
                                [0.24, 0.0, 0.0, if on { 1.1 } else { 0.0 }],
                            );
                            cylinder_at(
                                self,
                                [0.0, y + height * 0.074, 0.0],
                                [length * 1.035, height * 0.018, width * 1.035],
                                Quat::IDENTITY,
                                DARK,
                                POLYMER,
                            );
                        }
                    }
                    Fence | SafetyDoor => {
                        let angle = if equipment == SafetyDoor {
                            reading("angle")
                        } else {
                            0.0
                        };
                        let leaf = rotation * Quat::from_rotation_y(angle.to_radians());
                        let origin =
                            center + (leaf * Vec3::X - rotation * Vec3::X) * (length * 0.5);
                        for sign in [-1.0, 1.0] {
                            self.block(
                                origin + leaf * Vec3::new(sign * length * 0.48, 0.0, 0.0),
                                Vec3::new(0.055, height, width),
                                leaf,
                                yellow,
                                highlight,
                            );
                            self.block(
                                origin + Vec3::Y * sign * height * 0.48,
                                Vec3::new(length, 0.055, width),
                                leaf,
                                yellow,
                                highlight,
                            );
                        }
                        let mut x = -length * 0.4;
                        let count = ((length / 0.09) as usize).clamp(3, 30);
                        for _ in 0..=count {
                            self.block(
                                origin + leaf * Vec3::X * x,
                                Vec3::new(0.013, height * 0.91, 0.018),
                                leaf,
                                DARK,
                                highlight,
                            );
                            x += length * 0.8 / count as f32;
                        }
                        let rows = ((height / 0.12) as usize).clamp(3, 22);
                        for row in 1..rows {
                            let y = height * (row as f32 / rows as f32 - 0.5);
                            self.block(
                                origin + Vec3::Y * y,
                                Vec3::new(length * 0.96, 0.012, 0.016),
                                leaf,
                                DARK,
                                highlight,
                            );
                        }
                        if equipment == SafetyDoor {
                            self.block(
                                origin + leaf * Vec3::new(length * 0.37, 0.0, -width * 0.55),
                                Vec3::new(0.12, 0.06, 0.085),
                                leaf,
                                STEEL,
                                highlight,
                            );
                        }
                    }
                }
            }
            PartKind::Machine { ref machine } => {
                let manual = machine.assembly.is_some() && machine.requires_operator();
                if machine.assembly.is_some() {
                    let frame = if manual { DRIVE } else { STEEL };
                    box_at(self, [0.0, -0.045, 0.0], [1.10, 0.09, 1.10], STEEL);
                    for x in [-0.47, 0.47] {
                        for z in [-0.47, 0.47] {
                            box_at(self, [x, -0.345, z], [0.09, 0.51, 0.09], frame);
                        }
                        box_at(self, [x, -0.47, 0.0], [0.07, 0.06, 1.02], frame);
                    }
                    if manual {
                        box_at(self, [0.0, 0.16, 0.49], [1.10, 0.32, 0.08], DRIVE);
                        for x in [-0.24, 0.0, 0.24] {
                            box_at(self, [x, 0.049, 0.37], [0.20, 0.10, 0.13], GUARD);
                        }
                    } else {
                        for x in [-0.49, 0.49] {
                            for z in [-0.49, 0.49] {
                                box_at(self, [x, 0.47, z], [0.065, 0.94, 0.065], frame);
                            }
                            box_at(self, [x, 0.92, 0.0], [0.10, 0.08, 1.08], frame);
                        }
                        box_at(self, [0.0, 0.92, 0.0], [1.10, 0.12, 0.15], DRIVE);
                        box_at(self, [0.0, 0.84, 0.0], [0.22, 0.085, 0.20], STEEL);
                    }
                    let lamp = if !preview && flag(snapshot, &part.id, "fault") {
                        Color32::from_rgb(225, 66, 54)
                    } else if !preview && flag(snapshot, &part.id, "busy") {
                        ACTIVE
                    } else {
                        GUARD
                    };
                    box_at(self, [0.44, 0.19, -0.48], [0.18, 0.28, 0.075], DARK);
                    box_at(self, [0.44, 0.24, -0.52], [0.11, 0.10, 0.015], lamp);
                } else {
                    let body = Color32::from_rgb(164, 183, 186);
                    box_at(self, [0.0, -0.30, 0.0], [1.10, 0.60, 1.10], DARK);
                    box_at(self, [0.0, -0.30, -0.558], [0.97, 0.45, 0.023], body);
                    for y in [-0.45, -0.405, -0.36, -0.315] {
                        box_at(self, [0.28, y, -0.572], [0.25, 0.012, 0.009], DARK);
                    }
                    box_at(self, [-0.35, -0.18, -0.575], [0.075, 0.075, 0.025], DRIVE);
                    for x in [-0.50, 0.50] {
                        for z in [-0.50, 0.50] {
                            box_at(self, [x, 0.42, z], [0.12, 0.84, 0.12], body);
                        }
                        // Lower openings leave the transfer path between the worktable and the conveyors.
                        box_at(self, [x, 0.69, 0.0], [0.09, 0.35, 0.93], body);
                    }

                    // The rear service window also keeps the work area visible from the default view.
                    box_at(self, [0.0, 0.39, 0.525], [0.93, 0.075, 0.062], body);
                    box_at(self, [0.0, 0.80, 0.525], [0.98, 0.10, 0.062], body);
                    box_at(self, [0.0, 0.357, 0.525], [0.93, 0.05, 0.066], DARK);
                    self.window(
                        center + rotation * Vec3::new(0.0, 0.586, 0.525),
                        Vec3::new(0.895, 0.33, 0.006),
                        rotation,
                        highlight,
                    );
                    if machine.unload_robot.is_none() {
                        box_at(self, [0.0, 0.91, 0.0], [1.10, 0.09, 1.10], body);
                    } else {
                        // The overhead opening leaves the gantry access to the spindle.
                        box_at(self, [0.0, 0.87, -0.49], [1.10, 0.13, 0.18], body);
                    }

                    let angle = if preview {
                        0.0
                    } else {
                        machine
                            .door
                            .as_deref()
                            .map_or(0.0, |id| float(snapshot, id, "angle"))
                    };
                    let leaf = rotation * Quat::from_rotation_y(angle.to_radians());
                    let hinge = center + rotation * Vec3::new(-0.445, 0.0, -0.55);
                    let mut door_bar = |offset, size, color| {
                        self.block(
                            hinge + leaf * Vec3::from(offset),
                            Vec3::from(size),
                            leaf,
                            color,
                            highlight,
                        );
                    };
                    door_bar([0.44, 0.115, 0.0], [0.88, 0.23, 0.064], body);
                    door_bar([0.44, 0.78, 0.0], [0.88, 0.13, 0.064], body);
                    for x in [0.04, 0.84] {
                        door_bar([x, 0.47, 0.0], [0.08, 0.55, 0.064], DRIVE);
                    }
                    door_bar([0.76, 0.40, -0.072], [0.03, 0.15, 0.050], DARK);
                    for y in [0.335, 0.465] {
                        door_bar([0.76, y, -0.046], [0.03, 0.028, 0.046], DARK);
                    }
                    self.window(
                        hinge + leaf * Vec3::new(0.44, 0.4725, 0.0),
                        Vec3::new(0.74, 0.495, 0.006),
                        leaf,
                        highlight,
                    );

                    box_at(self, [0.52, 0.59, -0.624], [0.21, 0.26, 0.095], DARK);
                    box_at(self, [0.52, 0.638, -0.676], [0.165, 0.102, 0.009], DRIVE);
                    for (x, color) in [
                        (0.47, ACTIVE),
                        (0.52, GUARD),
                        (0.57, Color32::from_rgb(210, 61, 49)),
                    ] {
                        cylinder_at(
                            self,
                            [x, 0.51, -0.678],
                            [0.024, 0.012, 0.024],
                            along_z,
                            color,
                            POLYMER,
                        );
                    }
                    cylinder_at(
                        self,
                        [0.11, 0.69, -0.14],
                        [0.24, 0.30, 0.24],
                        Quat::IDENTITY,
                        DRIVE,
                        METAL,
                    );
                    let progress = if preview {
                        0.0
                    } else {
                        float(snapshot, &part.id, "progress") / 100.0
                    };
                    let gripping = !preview && snapshot.machine_grips.contains_key(&part.id);
                    let spindle =
                        if gripping || (!preview && flag(snapshot, &part.id, "waiting_unload")) {
                            0.69
                        } else {
                            0.49 - 0.08 * (progress * 25.0).sin()
                        };
                    cylinder_at(
                        self,
                        [0.11, spindle, -0.14],
                        [0.075, 0.28, 0.075],
                        Quat::IDENTITY,
                        ALUMINUM,
                        METAL,
                    );
                    let status = if !preview
                        && (flag(snapshot, &part.id, "fault")
                            || flag(snapshot, &part.id, "input_error"))
                    {
                        Color32::from_rgb(225, 66, 54)
                    } else if !preview && flag(snapshot, &part.id, "busy") {
                        SELECT
                    } else {
                        ACTIVE
                    };
                    let beacon_z = -0.49;
                    cylinder_at(
                        self,
                        [0.39, 0.97, beacon_z],
                        [0.13, 0.035, 0.13],
                        Quat::IDENTITY,
                        DARK,
                        POLYMER,
                    );
                    cylinder_at(
                        self,
                        [0.39, 1.03, beacon_z],
                        [0.10, 0.095, 0.10],
                        Quat::IDENTITY,
                        status,
                        [0.27, 0.0, 0.0, 0.68],
                    );
                }
                let [x, y, z] = machine.intake;
                box_at(self, [x, y - 0.024, z], [0.45, 0.038, 0.45], STEEL);
                for ([x, y, z], color) in [
                    (machine.outlet, ACTIVE),
                    (machine.reject, Color32::from_rgb(195, 86, 65)),
                ] {
                    if machine.unload_robot.is_none() {
                        box_at(self, [x, y - 0.024, z], [0.45, 0.038, 0.45], color);
                    } else if highlight || preview {
                        box_at(self, [x, y + 0.006, z], [0.15, 0.012, 0.15], color);
                    }
                }
                if !manual
                    && !preview
                    && let Some(grip) = snapshot.machine_grips.get(&part.id)
                {
                    let grip = Vec3::from(*grip);
                    let lift = ((center.y + 0.94) - grip.y).max(0.05);
                    self.block(
                        grip + Vec3::Y * (lift * 0.5),
                        Vec3::new(0.055, lift, 0.055),
                        rotation,
                        STEEL,
                        highlight,
                    );
                    self.block(
                        grip + Vec3::Y * 0.026,
                        Vec3::new(0.18, 0.052, 0.14),
                        rotation,
                        SELECT,
                        highlight,
                    );
                }
            }
            PartKind::Robot { ref robot } => {
                let [x, y, z] = robot.reach();
                let [px, py, pz] = if preview {
                    [0.0; 3]
                } else {
                    ["x", "y", "z"].map(|axis| float(snapshot, &part.id, axis))
                };
                let rail = y + 0.18;
                let span = z.max(0.12);
                for pos in [-0.08, x + 0.08] {
                    box_at(self, [pos, rail * 0.5, -0.09], [0.085, rail, 0.10], STEEL);
                }
                box_at(self, [x * 0.5, rail, -0.09], [x + 0.22, 0.12, 0.12], STEEL);
                box_at(
                    self,
                    [x * 0.5, rail + 0.012, -0.026],
                    [x + 0.20, 0.018, 0.013],
                    DARK,
                );
                box_at(self, [px, rail + 0.06, -0.09], [0.22, 0.13, 0.17], DRIVE);
                box_at(self, [px, rail + 0.046, pz], [0.17, 0.11, 0.16], DRIVE);
                box_at(
                    self,
                    [px, rail, span * 0.5 - 0.04],
                    [0.13, 0.11, span + 0.20],
                    SELECT,
                );
                for offset in [-0.042, 0.042] {
                    cylinder_at(
                        self,
                        [px + offset, (rail + py) * 0.5, pz],
                        [0.031, (rail - py).max(0.025), 0.031],
                        Quat::IDENTITY,
                        ALUMINUM,
                        METAL,
                    );
                }
                let angle = if preview {
                    0.0
                } else {
                    float(snapshot, &part.id, "angle")
                };
                let grip = center + rotation * Vec3::new(px, py, pz);
                let tool = rotation * Quat::from_rotation_y(angle.to_radians());
                let holding = !preview && flag(snapshot, &part.id, "holding");
                self.block(
                    grip,
                    Vec3::new(0.17, 0.045, 0.15),
                    tool,
                    if holding { ACTIVE } else { GUARD },
                    highlight,
                );
                for sign in [-1.0, 1.0] {
                    let jaw = if holding { 0.056 } else { 0.075 };
                    self.block(
                        grip + tool * Vec3::new(sign * jaw, -0.044, 0.0),
                        Vec3::new(0.026, 0.058, 0.09),
                        tool,
                        DARK,
                        highlight,
                    );
                }
            }
            PartKind::Staff { ref staff } => {
                let duty = !preview && flag(snapshot, &part.id, "on_duty");
                let body = if duty {
                    match staff.role {
                        StaffRole::Operator => Color32::from_rgb(63, 153, 184),
                        StaffRole::Technician => Color32::from_rgb(217, 167, 55),
                    }
                } else {
                    Color32::from_rgb(82, 91, 96)
                };
                for x in [-0.11, 0.11] {
                    box_at(self, [x, 0.45, 0.0], [0.15, 0.86, 0.17], DARK);
                }
                for x in [-0.25, 0.25] {
                    box_at(self, [x, 1.08, 0.0], [0.12, 0.53, 0.14], body);
                }
                box_at(self, [0.0, 1.12, 0.0], [0.38, 0.56, 0.22], body);
                box_at(
                    self,
                    [0.0, 1.52, 0.0],
                    [0.22, 0.25, 0.22],
                    Color32::from_rgb(190, 143, 113),
                );
                box_at(
                    self,
                    [0.0, 1.66, 0.0],
                    [0.29, 0.11, 0.28],
                    if duty { SELECT } else { STEEL },
                );
            }
            PartKind::FlowGuide {
                length,
                height,
                inlet,
                outlet,
            } => {
                for rail in crate::production::guide_rails(length, height, inlet, outlet) {
                    self.block(
                        center + rotation * Vec3::from(rail.position),
                        Vec3::from(rail.size),
                        rotation * Quat::from_rotation_y(rail.yaw.to_radians()),
                        Color32::from_rgb(226, 176, 68),
                        highlight,
                    );
                }
            }
            PartKind::Barrier {
                size: [length, height, width],
            } => {
                let plinth = height * 0.14;
                box_at(
                    self,
                    [0.0, plinth * 0.5, 0.0],
                    [length * 0.99, height - plinth, width * 0.93],
                    STEEL,
                );
                box_at(
                    self,
                    [0.0, (plinth - height) * 0.5, 0.0],
                    [length, plinth, width],
                    DARK,
                );
                box_at(
                    self,
                    [0.0, height * 0.46, 0.0],
                    [length, height * 0.08, width],
                    GUARD,
                );
            }
            PartKind::WaterSource {
                size: [x, y, z], ..
            } => {
                cylinder_at(
                    self,
                    [0.0, -0.035 * y, 0.0],
                    [0.94 * x, 0.86 * y, 0.94 * z],
                    Quat::IDENTITY,
                    WATER_PIPE,
                    POLYMER,
                );
                cylinder_at(
                    self,
                    [0.0, 0.42 * y, 0.0],
                    [x, 0.06 * y, z],
                    Quat::IDENTITY,
                    STEEL,
                    METAL,
                );
                cylinder_at(
                    self,
                    [0.0, 0.475 * y, 0.0],
                    [0.24 * x, 0.05 * y, 0.24 * z],
                    Quat::IDENTITY,
                    DARK,
                    METAL,
                );
                box_at(self, [0.0, -0.48 * y, 0.0], [x, 0.04 * y, z], DARK);
            }
            PartKind::WaterPump {
                size: [x, y, z], ..
            } => {
                box_at(self, [0.0, -0.43 * y, 0.0], [x, 0.14 * y, z], DARK);
                cylinder_at(
                    self,
                    [-0.15 * x, 0.0, 0.0],
                    [0.74 * z, 0.53 * x, 0.59 * y],
                    along_x,
                    DRIVE,
                    METAL,
                );
                cylinder_at(
                    self,
                    [0.23 * x, 0.0, 0.0],
                    [0.86 * z, 0.32 * x, 0.72 * y],
                    along_x,
                    GUARD,
                    METAL,
                );
                box_at(
                    self,
                    [0.18 * x, 0.42 * y, 0.0],
                    [0.21 * x, 0.16 * y, 0.25 * z],
                    if snapshot
                        .tags
                        .get(&format!("{}.failed", part.id))
                        .is_some_and(|state| state.value == Value::Bool(true))
                    {
                        Color32::from_rgb(221, 72, 48)
                    } else if snapshot.running && water_flow(part, snapshot) > 0.0 {
                        ACTIVE
                    } else {
                        DARK
                    },
                );
            }
            PartKind::WaterValve {
                size: [x, y, z], ..
            } => {
                let opening = match snapshot
                    .tags
                    .get(&format!("{}.opening_pct", part.id))
                    .map(|state| state.value)
                {
                    Some(Value::Float(opening)) => opening / 100.0,
                    _ => 0.0,
                };
                let brass = Color32::from_rgb(184, 132, 58);
                cylinder_at(
                    self,
                    [0.0, -0.08 * y, 0.0],
                    [0.82 * z, 0.62 * x, 0.64 * y],
                    along_x,
                    brass,
                    METAL,
                );
                for sign in [-1.0, 1.0] {
                    cylinder_at(
                        self,
                        [sign * 0.36 * x, -0.08 * y, 0.0],
                        [0.65 * z, 0.24 * x, 0.54 * y],
                        along_x,
                        STEEL,
                        METAL,
                    );
                }
                box_at(
                    self,
                    [0.0, 0.25 * y, 0.0],
                    [0.30 * x, 0.23 * y, 0.33 * z],
                    DARK,
                );
                self.block(
                    center + rotation * Vec3::new(0.0, 0.435 * y, 0.0),
                    Vec3::new(0.75 * x.min(z), 0.13 * y, 0.19 * z),
                    rotation * Quat::from_rotation_y((1.0 - opening) * std::f32::consts::FRAC_PI_2),
                    Color32::from_rgb(211, 75, 54),
                    false,
                );
            }
            PartKind::WaterDrain {
                size: [x, y, z], ..
            } => {
                cylinder_at(
                    self,
                    [0.0, -0.10 * y, 0.0],
                    [x, 0.80 * y, z],
                    Quat::IDENTITY,
                    STEEL,
                    METAL,
                );
                cylinder_at(
                    self,
                    [0.0, 0.32 * y, 0.0],
                    [0.78 * x, 0.10 * y, 0.78 * z],
                    Quat::IDENTITY,
                    DARK,
                    POLYMER,
                );
            }
            PartKind::WaterJunction {
                size: [x, y, z],
                mixing,
            } => {
                box_at(self, [0.0, 0.0, 0.0], [x, y * 0.63, z], WATER_PIPE);
                box_at(
                    self,
                    [0.0, 0.37 * y, 0.0],
                    [x * 0.63, y * 0.12, z * 0.72],
                    if mixing { GUARD } else { STEEL },
                );
            }
            PartKind::WaterInstrument {
                size: [x, y, z],
                measurement,
            } => {
                cylinder_at(
                    self,
                    [0.0, 0.0, 0.0],
                    [z, x, y * 0.45],
                    along_x,
                    STEEL,
                    METAL,
                );
                let dial = match measurement {
                    WaterMeasurement::Flow => WATER_PIPE,
                    WaterMeasurement::Temperature => GUARD,
                    WaterMeasurement::Pressure => ACTIVE,
                };
                cylinder_at(
                    self,
                    [0.0, 0.28 * y, 0.0],
                    [x * 0.67, y * 0.35, z * 0.8],
                    Quat::IDENTITY,
                    dial,
                    METAL,
                );
            }
            PartKind::WaterFilter {
                size: [x, y, z], ..
            } => {
                cylinder_at(
                    self,
                    [0.0, 0.0, 0.0],
                    [x * 0.8, y * 0.9, z],
                    Quat::IDENTITY,
                    DRIVE,
                    METAL,
                );
                box_at(self, [0.0, 0.06 * y, 0.0], [x, y * 0.23, z], STEEL);
                let loading = water_number(part, snapshot, "loading_pct");
                box_at(
                    self,
                    [0.0, 0.27 * y, 0.51 * z],
                    [x * 0.24, y * 0.13, z * 0.08],
                    if loading > 90.0 { GUARD } else { ACTIVE },
                );
            }
            PartKind::WaterHeater {
                size: [x, y, z], ..
            } => {
                box_at(self, [0.0, 0.0, 0.0], [x, y, z], STEEL);
                for across in [-0.24, 0.0, 0.24] {
                    box_at(
                        self,
                        [across * x, 0.51 * y, 0.0],
                        [x * 0.10, y * 0.08, z * 0.75],
                        GUARD,
                    );
                }
            }
            PartKind::WaterHeatExchanger {
                size: [x, y, z], ..
            } => {
                for left in [-0.34, -0.17, 0.0, 0.17, 0.34] {
                    box_at(self, [x * left, 0.0, 0.0], [x * 0.12, y, z], STEEL);
                }
                box_at(self, [-0.46 * x, 0.0, 0.0], [x * 0.08, y * 0.86, z], DRIVE);
                box_at(self, [0.46 * x, 0.0, 0.0], [x * 0.08, y * 0.86, z], DRIVE);
            }
            PartKind::WaterFiller {
                size: [x, y, z], ..
            } => {
                box_at(self, [0.0, 0.25 * y, 0.0], [x, y * 0.35, z * 0.65], STEEL);
                box_at(
                    self,
                    [0.0, -0.5 * y, -0.45 * z],
                    [x * 0.2, y * 1.7, z * 0.13],
                    DRIVE,
                );
                cylinder_at(
                    self,
                    [0.0, 0.04 * y, 0.0],
                    [x * 0.23, y * 0.28, z * 0.23],
                    Quat::IDENTITY,
                    DARK,
                    METAL,
                );
                if water_flow(part, snapshot) > 0.0 {
                    cylinder_at(
                        self,
                        [0.0, -0.28 * y, 0.0],
                        [0.014, 0.46 * y, 0.014],
                        Quat::IDENTITY,
                        FLOW_MARKER,
                        POLYMER,
                    );
                }
            }
            PartKind::HeatedTank(tank) => {
                let [x, y, z] = tank.size;
                box_at(self, [0.0, -0.08 * y, 0.0], [x, 0.72 * y, z], STEEL);
                box_at(
                    self,
                    [0.0, -0.47 * y, 0.0],
                    [0.86 * x, 0.06 * y, 0.86 * z],
                    DARK,
                );
                box_at(
                    self,
                    [0.0, 0.29 * y, 0.0],
                    [0.98 * x, 0.06 * y, 0.98 * z],
                    DARK,
                );
                box_at(
                    self,
                    [0.0, 0.41 * y, 0.0],
                    [0.38 * x, 0.18 * y, 0.38 * z],
                    STEEL,
                );
                self.block(
                    center + rotation * Vec3::new(0.0, -0.05 * y, 0.505 * z),
                    Vec3::new(0.18 * x, 0.40 * y, 0.015 * z),
                    rotation,
                    DARK,
                    false,
                );

                let initial_level = tank.starting_volume_l() / tank.volume_l;
                let level = if preview {
                    initial_level
                } else {
                    match snapshot
                        .tags
                        .get(&format!("{}.level_pct", part.id))
                        .map(|state| state.value)
                    {
                        Some(Value::Float(level)) => level / 100.0,
                        _ => initial_level,
                    }
                }
                .clamp(0.0, 1.0);
                if level > 0.0 {
                    let height = 0.36 * y * level;
                    self.block(
                        center + rotation * Vec3::new(0.0, -0.23 * y + height * 0.5, 0.518 * z),
                        Vec3::new(0.135 * x, height, 0.015 * z),
                        rotation,
                        WATER_PIPE,
                        false,
                    );
                }
                if tank.networked {
                    self.water_ports(part, highlight);
                    return;
                }

                let feed_l_s = match snapshot
                    .tags
                    .get(&format!("{}.feed_l_s", part.id))
                    .map(|state| state.value)
                {
                    Some(Value::Float(rate)) => rate,
                    _ => 0.0,
                };
                let flowing = feed_l_s > 0.0;
                let supply = Color32::from_rgb(75, 144, 222);
                let drain = Color32::from_rgb(57, 174, 168);
                let water = Color32::from_rgb(114, 204, 244);
                let pipe = x.min(y).min(z) * 0.14;
                let vertical = Quat::IDENTITY;

                // Source reservoir and feed pump.
                cylinder_at(
                    self,
                    [-2.50 * x, -0.12 * y, 0.0],
                    [0.83 * x, 0.70 * y, 0.83 * z],
                    vertical,
                    supply,
                    POLYMER,
                );
                cylinder_at(
                    self,
                    [-2.50 * x, 0.24 * y, 0.0],
                    [0.87 * x, 0.04 * y, 0.87 * z],
                    vertical,
                    STEEL,
                    METAL,
                );
                cylinder_at(
                    self,
                    [-2.50 * x, 0.28 * y, 0.0],
                    [0.23 * x, 0.06 * y, 0.23 * z],
                    vertical,
                    DARK,
                    METAL,
                );
                box_at(
                    self,
                    [-2.50 * x, -0.48 * y, 0.0],
                    [0.90 * x, 0.04 * y, 0.90 * z],
                    DARK,
                );
                box_at(
                    self,
                    [-1.61 * x, -0.465 * y, 0.0],
                    [0.61 * x, 0.07 * y, 0.58 * z],
                    DARK,
                );
                cylinder_at(
                    self,
                    [-1.70 * x, -0.30 * y, 0.0],
                    [0.37 * z, 0.36 * x, 0.25 * y],
                    along_x,
                    DRIVE,
                    METAL,
                );
                cylinder_at(
                    self,
                    [-1.46 * x, -0.30 * y, 0.0],
                    [0.44 * z, 0.18 * x, 0.30 * y],
                    along_x,
                    GUARD,
                    METAL,
                );
                box_at(
                    self,
                    [-1.55 * x, -0.13 * y, 0.0],
                    [0.17 * x, 0.075 * y, 0.19 * z],
                    if flowing { ACTIVE } else { DARK },
                );

                // Pipe up to the inlet, then overflow from the opposite side of the tank.
                for (center, length, angle, color) in [
                    ([-1.99 * x, -0.30 * y, 0.0], 0.38 * x, along_x, supply),
                    ([-1.12 * x, -0.30 * y, 0.0], 0.56 * x, along_x, supply),
                    (
                        [-0.86 * x, -0.07 * y, 0.0],
                        0.46 * y + pipe,
                        vertical,
                        supply,
                    ),
                    ([-0.67 * x, 0.16 * y, 0.0], 0.38 * x + pipe, along_x, supply),
                    ([1.34 * x, 0.08 * y, 0.0], 1.70 * x + pipe, along_x, drain),
                    (
                        [2.19 * x, -0.055 * y, 0.0],
                        0.27 * y + pipe,
                        vertical,
                        drain,
                    ),
                ] {
                    cylinder_at(self, center, [pipe, length, pipe], angle, color, POLYMER);
                }
                for (offset, color) in [
                    ([-0.51 * x, 0.16 * y, 0.0], supply),
                    ([0.51 * x, 0.08 * y, 0.0], drain),
                ] {
                    cylinder_at(
                        self,
                        offset,
                        [1.55 * pipe, 0.46 * pipe, 1.55 * pipe],
                        along_x,
                        color,
                        METAL,
                    );
                }

                // Open discharge into a floor drain.
                cylinder_at(
                    self,
                    [2.19 * x, -0.455 * y, 0.0],
                    [0.87 * x, 0.09 * y, 0.87 * z],
                    vertical,
                    STEEL,
                    METAL,
                );
                cylinder_at(
                    self,
                    [2.19 * x, -0.404 * y, 0.0],
                    [0.65 * x, 0.018 * y, 0.65 * z],
                    vertical,
                    DARK,
                    POLYMER,
                );
                if flowing {
                    let volume_l = snapshot
                        .water_outlet_volume_l
                        .get(&(part.id.clone(), 0))
                        .copied()
                        .unwrap_or(0.0);
                    let thickness = 0.48 * pipe;
                    cylinder_at(
                        self,
                        [2.19 * x, -0.298 * y, 0.0],
                        [thickness, 0.20 * y, thickness],
                        vertical,
                        water,
                        POLYMER,
                    );
                    for (start, length, height) in [
                        (-1.35 * x, 0.44 * x, -0.30 * y),
                        (0.60 * x, 1.48 * x, 0.08 * y),
                    ] {
                        if let Some(mut markers) =
                            PipeMarkers::new(length, pipe, feed_l_s, volume_l)
                        {
                            while markers.next_m < length {
                                cylinder_at(
                                    self,
                                    [start + markers.next_m, height, 0.0],
                                    markers.size.to_array(),
                                    along_x,
                                    FLOW_MARKER,
                                    POLYMER,
                                );
                                markers.next_m += markers.spacing_m;
                            }
                        }
                    }
                }
            }
            PartKind::Crate { size, .. } if preview => self.parcel(
                center,
                Vec3::from(product.map_or(size, |(_, spec)| spec.size)),
                rotation,
                product,
                highlight,
            ),
            PartKind::Crate { .. } => {}
        }
        self.water_ports(part, highlight);
    }
}

struct PipeMarkers {
    next_m: f32,
    spacing_m: f32,
    size: Vec3,
}

impl PipeMarkers {
    fn new(length_m: f32, pipe_diameter_m: f32, rate_l_s: f32, volume_l: f64) -> Option<Self> {
        if length_m <= 0.0 || rate_l_s <= 0.0 {
            return None;
        }
        let spacing_m = (length_m / 12.0).max(0.32).min(length_m);
        let area_m2 = std::f64::consts::PI * (f64::from(pipe_diameter_m) / 2.0).powi(2);
        // Exaggerate motion by four for readability at the camera's default scale.
        let travel_m = 4.0 * (volume_l * 0.001) / area_m2;
        let next_m =
            (travel_m + f64::from(spacing_m) / 2.0).rem_euclid(f64::from(spacing_m)) as f32;

        let strength = (rate_l_s / (rate_l_s + 0.1)).sqrt();
        let diameter = pipe_diameter_m * (1.1 + 0.4 * strength);
        Some(Self {
            next_m,
            spacing_m,
            size: Vec3::new(diameter, 0.02 + 0.045 * strength, diameter),
        })
    }
}

fn water_flow(part: &Part, snapshot: &Snapshot) -> f32 {
    water_number(part, snapshot, "flow_l_s")
}

fn water_number(part: &Part, snapshot: &Snapshot, role: &str) -> f32 {
    match snapshot
        .tags
        .get(&format!("{}.{}", part.id, role))
        .map(|state| state.value)
    {
        Some(Value::Float(value)) => value,
        _ => 0.0,
    }
}

pub struct ObjectDrag {
    id: String,
    pointer: Pos2,
    origin: Vec3,
    yaw: f32,
    moved: bool,
    placement: Option<snap::Placement>,
}

pub struct Interaction {
    pub selection: Option<Option<String>>,
    pub movement: Option<(String, snap::Placement)>,
    pub library_drop: Option<(super::library::Entry, [f32; 2])>,
}

fn box_depth(instance: &Instance, origin: Vec3, eye: Vec3) -> Option<f32> {
    let offset = origin - Vec3::from_slice(&instance.center);
    let mut near = f32::INFINITY;
    let mut far = f32::NEG_INFINITY;
    for axis in [&instance.axis_x, &instance.axis_y, &instance.axis_z] {
        let axis = Vec3::from_slice(axis);
        let length = axis.length();
        let unit = axis.try_normalize()?;
        let projected = unit.dot(offset);
        let speed = unit.dot(eye);
        let half = length * 0.5;
        if speed.abs() < 0.00001 {
            if projected.abs() > half {
                return None;
            }
        } else {
            let a = (-half - projected) / speed;
            let b = (half - projected) / speed;
            near = near.min(a.max(b));
            far = far.max(a.min(b));
        }
    }
    (near >= far).then_some(near)
}

fn crate_at(part: &Part, snapshot: &Snapshot, preview: bool) -> Option<(Vec3, Quat)> {
    if preview {
        Some((
            Vec3::from(part.position),
            Quat::from_rotation_y(part.yaw.to_radians()),
        ))
    } else {
        snapshot
            .parcels
            .iter()
            .find(|parcel| parcel.id == part.id)
            .map(|parcel| {
                (
                    Vec3::from(parcel.position),
                    Quat::from_array(parcel.rotation),
                )
            })
    }
}

fn pick<'a>(
    project: &'a Project,
    snapshot: &Snapshot,
    preview: bool,
    view: View,
    pointer: Pos2,
) -> Option<&'a Part> {
    let delta = (pointer - view.center) / view.scale;
    let origin = view.target + view.right * delta.x - view.up * delta.y;
    let mut scratch = Scene::default();
    let mut closest = None;
    let mut anchors = None;
    for part in &project.parts {
        let mut center = Vec3::from(part.position);
        if let PartKind::Crate { size, .. } = part.kind {
            let Some((at, rotation)) = crate_at(part, snapshot, preview) else {
                continue;
            };
            center = at;
            let size = part
                .product
                .as_deref()
                .and_then(|id| project.product(id))
                .map_or(size, |product| product.size);
            scratch.block(at, Vec3::from(size), rotation, STEEL, false);
        } else {
            scratch.part(project, part, snapshot, false, preview);
        }

        let depth = scratch
            .instances
            .iter()
            .chain(&scratch.cylinders)
            .filter_map(|instance| box_depth(instance, origin, view.eye))
            .max_by(f32::total_cmp);
        if let Some(depth) = depth
            && closest.is_none_or(|(best, _)| depth > best)
        {
            closest = Some((depth, part));
        }
        let distance = view.project(center).distance(pointer);
        if distance < 15.0 && anchors.is_none_or(|(best, _)| distance < best) {
            anchors = Some((distance, part));
        }
        scratch.instances.clear();
        scratch.cylinders.clear();
        scratch.windows.clear();
        scratch.meshes.clear();
    }
    closest.or(anchors).map(|(_, part)| part)
}

pub fn draw(
    ui: &mut egui::Ui,
    language: Language,
    camera: &mut Camera,
    quality: GraphicsQuality,
    project: &Project,
    snapshot: &Snapshot,
    selection: Option<&str>,
    preview: bool,
    editable: bool,
    object_drag: &mut Option<ObjectDrag>,
) -> Interaction {
    let parts = &project.parts;
    let (rect, response) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    let mut view = View::new(rect, camera);
    let mut interaction = Interaction {
        selection: None,
        movement: None,
        library_drop: None,
    };
    let delta = ui.input(|i| i.pointer.delta());
    if object_drag.is_none() && response.dragged_by(egui::PointerButton::Secondary) {
        if ui.input(|i| i.modifiers.shift) {
            camera.target -= view.ground_delta(delta);
        } else {
            camera.yaw -= delta.x * 0.012;
            camera.pitch = (camera.pitch + delta.y * 0.009).clamp(0.21, 1.48);
        }
    }
    if object_drag.is_none() && response.hovered() {
        let zoom = ui.input(|i| i.smooth_scroll_delta.y);
        camera.zoom = (camera.zoom * (zoom * 0.0018).exp()).clamp(0.28, 5.0);
    }
    view = View::new(rect, camera);
    if editable
        && let Some(entry) = response.dnd_release_payload::<super::library::Entry>()
        && let Some(pointer) = ui.input(|i| i.pointer.latest_pos())
    {
        let plane = view.project(Vec3::Y * entry.height(project));
        let position = view.ground_delta(pointer - plane);
        interaction.library_drop = Some((
            (*entry).clone(),
            [
                position.x.clamp(-1000.0, 1000.0),
                position.z.clamp(-1000.0, 1000.0),
            ],
        ));
    }
    let primary = egui::PointerButton::Primary;
    if response.clicked_by(primary) {
        let nearest = response
            .interact_pointer_pos()
            .and_then(|pos| pick(project, snapshot, preview, view, pos));
        interaction.selection = Some(nearest.map(|part| part.id.clone()));
    }
    if !editable || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        *object_drag = None;
    } else if response.drag_started_by(primary)
        && let Some(pointer) = ui.input(|i| i.pointer.press_origin())
        && rect.contains(pointer)
        && let Some(part) = pick(project, snapshot, preview, view, pointer)
    {
        let mut origin = Vec3::from(part.position);
        if matches!(part.kind, PartKind::Crate { .. })
            && let Some((at, _)) = crate_at(part, snapshot, preview)
        {
            origin.x = at.x;
            origin.z = at.z;
        }
        *object_drag = Some(ObjectDrag {
            id: part.id.clone(),
            pointer,
            origin,
            yaw: part.yaw,
            moved: false,
            placement: None,
        });
        interaction.selection = Some(Some(part.id.clone()));
    }

    let mut marker = None;
    if let Some(drag) = object_drag.as_mut()
        && let Some(part) = parts.iter().find(|part| part.id == drag.id)
        && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
    {
        let displacement = pos - drag.pointer;
        drag.moved |= displacement.length_sq() >= 4.0;
        if drag.moved {
            let mut position = drag.origin + view.ground_delta(displacement);
            position.x = position.x.clamp(-1000.0, 1000.0);
            position.z = position.z.clamp(-1000.0, 1000.0);
            position.y = drag.origin.y;
            let raw = snap::Placement {
                position,
                yaw: drag.yaw,
            };
            drag.placement = Some(
                if !ui.input(|i| i.modifiers.ctrl)
                    && let Some(snapped) = snap::find(project, part, &raw)
                {
                    marker = Some(snapped.marker);
                    snapped.placement
                } else {
                    raw
                },
            );
        }
    }
    let released = if !ui.input(|i| i.pointer.button_down(primary)) {
        object_drag.take()
    } else {
        None
    };
    let selection = interaction
        .selection
        .as_ref()
        .map_or(selection, |id| id.as_deref());
    let moving = object_drag.as_ref().or(released.as_ref()).and_then(|drag| {
        drag.placement
            .as_ref()
            .map(|placement| (drag.id.as_str(), placement))
    });

    let moved_part = moving.and_then(|(id, at)| {
        let mut part = parts.iter().find(|part| part.id == id)?.clone();
        part.position = at.position.to_array();
        part.yaw = at.yaw;
        Some(part)
    });
    let painter = ui.painter_at(rect);
    if rect.is_positive() {
        let mut scene = Scene {
            instances: Vec::with_capacity(parts.len() * 8 + snapshot.parcels.len() * 3 + 1),
            ..Default::default()
        };
        for part in parts {
            let highlight = selection == Some(part.id.as_str());
            if let Some(moved) = moved_part.as_ref().filter(|moved| moved.id == part.id) {
                scene.part(project, moved, snapshot, highlight, true);
            } else {
                scene.part(project, part, snapshot, highlight, preview);
            }
        }
        if !project.pipes.is_empty() {
            let mut parts_by_id: BTreeMap<_, _> =
                parts.iter().map(|part| (part.id.as_str(), part)).collect();
            if let Some(part) = &moved_part {
                parts_by_id.insert(part.id.as_str(), part);
            }
            for pipe in &project.pipes {
                if let (Some(from), Some(to)) = (
                    parts_by_id.get(pipe.from.as_str()),
                    parts_by_id.get(pipe.to.as_str()),
                ) {
                    scene.water_pipe(pipe, from, to, snapshot);
                }
            }
        }
        if !preview {
            for parcel in &snapshot.parcels {
                if moving.is_some_and(|(id, _)| id == parcel.id) {
                    continue;
                }
                scene.parcel(
                    Vec3::from(parcel.position),
                    Vec3::from(parcel.size),
                    Quat::from_array(parcel.rotation),
                    parcel
                        .product
                        .as_deref()
                        .and_then(|id| project.product(id).map(|spec| (id, spec))),
                    selection == Some(parcel.id.as_str()),
                );
                if let Some(contents) = parcel.liquid
                    && contents.volume_l > 0.0
                {
                    let size = Vec3::from(parcel.size);
                    let rotation = Quat::from_array(parcel.rotation);
                    let height =
                        size.y * 0.65 * (contents.volume_l / f64::from(contents.capacity_l)) as f32;
                    scene.block(
                        Vec3::from(parcel.position)
                            + rotation
                                * Vec3::new(0.0, -0.41 * size.y + height * 0.5, 0.51 * size.z),
                        Vec3::new(0.20 * size.x, height, 0.012 * size.z),
                        rotation,
                        WATER_PIPE,
                        false,
                    );
                }
                if parcel.scrap {
                    let rotation = Quat::from_array(parcel.rotation);
                    let size = Vec3::from(parcel.size);
                    scene.block(
                        Vec3::from(parcel.position) + rotation * Vec3::Y * (size.y * 0.5 + 0.004),
                        Vec3::new(size.x * 0.28, 0.008, size.z * 0.28),
                        rotation,
                        Color32::from_rgb(233, 76, 61),
                        false,
                    );
                }
            }
        }
        scene.surface(
            Vec3::new(camera.target.x, -0.06, camera.target.z),
            Vec3::new(280.0, 0.12, 280.0),
            Quat::IDENTITY,
            Color32::from_rgb(111, 118, 122),
            CONCRETE,
            0.0,
            false,
        );
        let cube_count = scene.instances.len() as u32;
        let mut meshes = Vec::new();
        if !scene.cylinders.is_empty() {
            let start = scene.instances.len() as u32;
            scene.instances.extend(scene.cylinders);
            meshes.push(MeshBatch {
                mesh: cylinder_mesh(quality.radial_segments()),
                instances: start..scene.instances.len() as u32,
            });
        }
        for (mesh, instances) in scene.meshes.into_values() {
            let start = scene.instances.len() as u32;
            scene.instances.extend(instances);
            meshes.push(MeshBatch {
                mesh: mesh.clone(),
                instances: start..scene.instances.len() as u32,
            });
        }
        // The orthographic camera uses the same viewing direction for all panes.
        scene.windows.sort_by(|a, b| {
            let depth = |instance: &Instance| view.eye.dot(Vec3::from_slice(&instance.center));
            depth(a).total_cmp(&depth(b))
        });
        let window_start = scene.instances.len() as u32;
        scene.instances.extend(scene.windows);
        painter.add(render::paint(
            rect,
            view.matrix(),
            view.eye,
            camera.target,
            quality,
            cube_count,
            window_start,
            scene.instances,
            meshes,
        ));
    }
    if let Some(part) = parts
        .iter()
        .find(|part| selection == Some(part.id.as_str()))
    {
        let position = moving
            .filter(|(id, _)| *id == part.id)
            .map_or_else(|| Vec3::from(part.position), |(_, at)| at.position);
        let label = view.project(position + Vec3::Y * 0.5);
        painter.text(
            label,
            egui::Align2::CENTER_BOTTOM,
            &part.name,
            egui::FontId::proportional(13.0),
            SELECT,
        );
    }

    if let Some(marker) = marker {
        let center = view.project(marker);
        painter.circle_stroke(center, 8.0, egui::Stroke::new(2.0, ACTIVE));
        painter.circle_filled(center, 2.5, ACTIVE);
    }
    if let Some(drag) = released
        && let Some(placement) = drag.placement
    {
        interaction.movement = Some((drag.id, placement));
        ui.ctx().request_repaint();
    }
    painter.text(
        rect.left_bottom() + Vec2::new(18.0, -18.0),
        egui::Align2::LEFT_BOTTOM,
        language.text(if editable {
            "Left drag: move    Ctrl: bypass snap    Right drag: orbit    Shift + right drag: pan    Scroll: zoom"
        } else {
            "Right drag: orbit    Shift + right drag: pan    Scroll: zoom"
        }),
        egui::FontId::proportional(11.5),
        Color32::from_rgb(166, 181, 183),
    );
    interaction
}

pub fn float(snapshot: &Snapshot, id: &str, role: &str) -> f32 {
    match snapshot
        .tags
        .get(&format!("{id}.{role}"))
        .map(|tag| tag.value)
    {
        Some(Value::Float(value)) => value,
        _ => 0.0,
    }
}

fn flag(snapshot: &Snapshot, id: &str, role: &str) -> bool {
    snapshot
        .tags
        .get(&format!("{id}.{role}"))
        .is_some_and(|tag| tag.value == Value::Bool(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_markers_vary_size_and_travel_with_discharge() {
        assert!(PipeMarkers::new(1.0, PIPE_DIAMETER_M, 0.0, 0.0).is_none());
        let start = PipeMarkers::new(1.0, PIPE_DIAMETER_M, 0.00015, 0.0).unwrap();
        let slow = PipeMarkers::new(1.0, PIPE_DIAMETER_M, 0.00015, 0.000075).unwrap();
        let fast = PipeMarkers::new(1.0, PIPE_DIAMETER_M, 0.15, 0.075).unwrap();

        assert!(fast.size.x > slow.size.x && fast.size.y > slow.size.y);
        let slow_travel = slow.next_m - start.next_m;
        assert!(slow_travel > 0.0 && (fast.next_m - start.next_m) > 900.0 * slow_travel);
    }
}
