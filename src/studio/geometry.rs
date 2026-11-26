use std::{
    f32::consts::TAU,
    sync::{Arc, OnceLock},
};

use glam::Vec3;

use crate::product::Mesh;

pub fn cylinder_mesh(segments: usize) -> Arc<Mesh> {
    static CACHE: OnceLock<[Arc<Mesh>; 3]> = OnceLock::new();
    let meshes = CACHE.get_or_init(|| [8, 12, 20].map(|segments| Arc::new(cylinder(segments))));
    let detail = match segments {
        8 => 0,
        12 => 1,
        _ => 2,
    };
    meshes[detail].clone()
}

fn cylinder(segments: usize) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut triangles = Vec::new();
    let mut triangle = |points: [(Vec3, Vec3); 3]| {
        let start = positions.len() as u32;
        for (point, normal) in points {
            positions.push(point.to_array());
            normals.push(normal.to_array());
        }
        triangles.push([start, start + 1, start + 2]);
    };
    let radial = |index| {
        let (z, x) = (TAU * index as f32 / segments as f32).sin_cos();
        Vec3::new(x, 0.0, z)
    };
    for index in 0..segments {
        let a = radial(index);
        let b = radial(index + 1);
        let lo = |r: Vec3| (r - Vec3::Y) * 0.5;
        let hi = |r: Vec3| (r + Vec3::Y) * 0.5;
        triangle([(lo(a), a), (hi(a), a), (hi(b), b)]);
        triangle([(lo(a), a), (hi(b), b), (lo(b), b)]);
        triangle([(Vec3::Y * 0.5, Vec3::Y), (hi(b), Vec3::Y), (hi(a), Vec3::Y)]);
        triangle([
            (Vec3::NEG_Y * 0.5, Vec3::NEG_Y),
            (lo(a), Vec3::NEG_Y),
            (lo(b), Vec3::NEG_Y),
        ]);
    }
    Mesh {
        positions,
        normals: Some(normals),
        triangles,
    }
}
