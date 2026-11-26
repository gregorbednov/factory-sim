use eframe::egui::{Color32, ColorImage};
use glam::{Vec2, Vec3};

use super::super::render::{Instance, cube_vertices};
use super::{Camera, Scene, View, cylinder_mesh};

const WIDTH: usize = 176;
const HEIGHT: usize = 104;

pub(super) fn render(scene: &Scene<'_>) -> ColorImage {
    let bounds = eframe::egui::Rect::from_min_size(
        eframe::egui::Pos2::ZERO,
        eframe::egui::vec2(WIDTH as f32, HEIGHT as f32),
    );
    let view = View::new(bounds, &Camera::default());
    let mut triangles = Vec::new();
    let mut add = |instance: &Instance, vertices: [Vec3; 3]| {
        let center = Vec3::from_slice(&instance.center);
        let x = Vec3::from_slice(&instance.axis_x);
        let y = Vec3::from_slice(&instance.axis_y);
        let z = Vec3::from_slice(&instance.axis_z);
        let points = vertices.map(|p| center + x * p.x + y * p.y + z * p.z);
        let normal = (points[1] - points[0])
            .cross(points[2] - points[0])
            .normalize_or_zero();
        let sun = Vec3::new(-0.45, 0.85, 0.35).normalize();
        let normal = if normal.dot(view.eye) < 0.0 {
            -normal
        } else {
            normal
        };
        let light = 0.36 + 0.64 * normal.dot(sun).max(0.0);
        let color = instance
            .color
            .map(|v| ((v * light).max(0.0).powf(1.0 / 2.2) * 255.0) as u8);
        triangles.push((
            points.map(|p| Vec3::new(view.right.dot(p), -view.up.dot(p), view.eye.dot(p))),
            Color32::from_rgb(color[0], color[1], color[2]),
        ));
    };
    let cube = cube_vertices();
    for instance in &scene.instances {
        for t in cube.chunks_exact(3) {
            add(instance, [0, 1, 2].map(|i| Vec3::from(t[i].position)));
        }
    }
    for instance in &scene.windows {
        for t in cube[24..30].chunks_exact(3) {
            add(instance, [0, 1, 2].map(|i| Vec3::from(t[i].position)));
        }
    }
    let cylinders = cylinder_mesh(8);
    for (mesh, instances) in std::iter::once((&cylinders, &scene.cylinders)).chain(
        scene
            .meshes
            .values()
            .map(|(mesh, instances)| (*mesh, instances)),
    ) {
        for instance in instances {
            for t in &mesh.triangles {
                add(instance, t.map(|i| Vec3::from(mesh.positions[i as usize])));
            }
        }
    }

    let mut image = ColorImage::filled([WIDTH, HEIGHT], Color32::from_rgb(23, 32, 37));
    if triangles.is_empty() {
        return image;
    }
    let (min, max) = triangles.iter().flat_map(|(points, _)| points).fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(min, max), point| (min.min(point.truncate()), max.max(point.truncate())),
    );
    let screen = Vec2::new(WIDTH as f32, HEIGHT as f32);
    let extent = (max - min).max(Vec2::splat(0.001));
    let fit = ((screen - Vec2::splat(18.0)) / extent).min_element();
    let offset = (screen - (max + min) * fit) * 0.5;
    let mut depth = vec![f32::NEG_INFINITY; WIDTH * HEIGHT];
    for (points, color) in triangles {
        let [a, b, c] = points.map(|p| (p.truncate() * fit + offset).extend(p.z));
        let edge = |u: Vec3, v: Vec3, p: Vec2| (v - u).truncate().perp_dot(p - u.truncate());
        let area = edge(a, b, c.truncate());
        if area.abs() < 0.001 {
            continue;
        }
        let lo = a.min(b).min(c).max(Vec3::ZERO);
        let hi = a.max(b).max(c).ceil();
        for y in lo.y as usize..(hi.y as usize).min(HEIGHT) {
            for x in lo.x as usize..(hi.x as usize).min(WIDTH) {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let u = edge(b, c, p) / area;
                let v = edge(c, a, p) / area;
                let w = 1.0 - u - v;
                if u >= 0.0 && v >= 0.0 && w >= 0.0 {
                    let z = u * a.z + v * b.z + w * c.z;
                    let index = y * WIDTH + x;
                    if z > depth[index] {
                        depth[index] = z;
                        image.pixels[index] = color;
                    }
                }
            }
        }
    }
    image
}
