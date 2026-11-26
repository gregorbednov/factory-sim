use std::sync::Arc;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[cfg(feature = "studio")]
mod import;
#[cfg(feature = "studio")]
pub use import::{UpAxis, import};

pub const MAX_TRIANGLES: usize = 250_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normals: Option<Vec<[f32; 3]>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Step {
        source: Arc<str>,
        meters_per_unit: f64,
        z_up: bool,
    },
    Obj,
    Stl,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Product {
    pub name: String,
    pub size: [f32; 3],
    pub mass: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquid_capacity_l: Option<f32>,
    pub geometry: Geometry,
    pub mesh: Arc<Mesh>,
}

#[derive(Clone, Copy)]
pub struct ProductSpec<'a> {
    #[cfg(any(feature = "studio", feature = "web"))]
    pub name: &'a str,
    pub size: [f32; 3],
    pub mass: f32,
    pub liquid_capacity_l: Option<f32>,
    #[cfg(any(feature = "studio", feature = "web"))]
    pub color: [u8; 3],
    #[cfg(any(feature = "studio", feature = "web"))]
    pub cardboard: bool,
    pub mesh: Option<&'a Arc<Mesh>>,
}

const fn box_spec(
    _name: &'static str,
    size: [f32; 3],
    mass: f32,
    _color: [u8; 3],
    _cardboard: bool,
) -> ProductSpec<'static> {
    ProductSpec {
        #[cfg(any(feature = "studio", feature = "web"))]
        name: _name,
        size,
        mass,
        liquid_capacity_l: None,
        #[cfg(any(feature = "studio", feature = "web"))]
        color: _color,
        #[cfg(any(feature = "studio", feature = "web"))]
        cardboard: _cardboard,
        mesh: None,
    }
}

pub const BOXES: [(&str, ProductSpec<'static>); 9] = [
    (
        "box-carton",
        box_spec(
            "Standard carton",
            [0.42, 0.38, 0.46],
            2.0,
            [190, 143, 93],
            true,
        ),
    ),
    (
        "box-tall",
        box_spec(
            "Tall carton",
            [0.32, 0.60, 0.33],
            3.0,
            [211, 179, 129],
            true,
        ),
    ),
    (
        "box-plastic",
        box_spec(
            "Low plastic box",
            [0.60, 0.20, 0.40],
            1.4,
            [64, 132, 184],
            false,
        ),
    ),
    (
        "raw-blank",
        box_spec(
            "Raw material",
            [0.25, 0.12, 0.34],
            1.25,
            [117, 148, 158],
            false,
        ),
    ),
    (
        "machined-base",
        box_spec(
            "Product base",
            [0.24, 0.08, 0.33],
            0.83,
            [60, 145, 91],
            false,
        ),
    ),
    (
        "machined-lid",
        box_spec(
            "Product lid",
            [0.24, 0.04, 0.33],
            0.40,
            [66, 110, 180],
            false,
        ),
    ),
    (
        "assembled-product",
        box_spec(
            "Assembled product",
            [0.24, 0.12, 0.33],
            1.23,
            [58, 129, 143],
            false,
        ),
    ),
    (
        "bottle-500ml",
        ProductSpec {
            liquid_capacity_l: Some(0.5),
            ..box_spec(
                "Bottle 0.5 L",
                [0.08, 0.22, 0.08],
                0.045,
                [100, 178, 184],
                false,
            )
        },
    ),
    (
        "canister-2l",
        ProductSpec {
            liquid_capacity_l: Some(2.0),
            ..box_spec(
                "Canister 2 L",
                [0.15, 0.23, 0.12],
                0.12,
                [92, 140, 177],
                false,
            )
        },
    ),
];

impl Product {
    pub fn spec(&self) -> ProductSpec<'_> {
        ProductSpec {
            #[cfg(any(feature = "studio", feature = "web"))]
            name: &self.name,
            size: self.size,
            mass: self.mass,
            liquid_capacity_l: self.liquid_capacity_l,
            #[cfg(any(feature = "studio", feature = "web"))]
            color: [133, 162, 177],
            #[cfg(any(feature = "studio", feature = "web"))]
            cardboard: false,
            mesh: Some(&self.mesh),
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.trim().is_empty(), "empty product name");
        ensure!(
            self.liquid_capacity_l
                .is_none_or(|v| v.is_finite() && v > 0.0),
            "invalid liquid capacity"
        );
        ensure!(
            self.mass.is_finite() && (0.001..=1000.0).contains(&self.mass),
            "invalid mass"
        );
        ensure!(
            self.size
                .iter()
                .all(|x| x.is_finite() && (0.0001..=1000.0).contains(x)),
            "model dimensions must be between 0.1 mm and 1 km on all axes"
        );
        let Mesh {
            positions,
            triangles,
            normals,
        } = &*self.mesh;
        ensure!(
            !triangles.is_empty()
                && triangles.len() <= MAX_TRIANGLES
                && positions.len() <= MAX_TRIANGLES * 3,
            "invalid mesh size"
        );
        if let Some(normals) = normals {
            ensure!(
                normals.len() == positions.len() && normals.iter().flatten().all(|n| n.is_finite()),
                "invalid vertex normals"
            );
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for p in positions {
            for axis in 0..3 {
                ensure!(p[axis].is_finite(), "invalid vertex");
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
        for axis in 0..3 {
            let half = self.size[axis] * 0.5;
            let tolerance = half * 0.001;
            ensure!(
                (min[axis] + half).abs() <= tolerance && (max[axis] - half).abs() <= tolerance,
                "mesh bounds do not match the product size"
            );
        }
        ensure!(
            triangles
                .iter()
                .flatten()
                .all(|&i| (i as usize) < positions.len()),
            "invalid triangle index"
        );
        if let Geometry::Step {
            source,
            meters_per_unit,
            ..
        } = &self.geometry
        {
            ensure!(
                !source.is_empty() && meters_per_unit.is_finite() && *meters_per_unit > 0.0,
                "invalid STEP source or scale"
            );
        }
        Ok(())
    }
}

impl Mesh {
    pub fn collision_shape(&self) -> Option<rapier3d::prelude::SharedShape> {
        use rapier3d::prelude::{Point, SharedShape};
        let points: Vec<_> = self.positions.iter().copied().map(Point::from).collect();
        SharedShape::convex_hull(&points).filter(|shape| {
            let volume = shape.mass_properties(1.0).mass();
            volume.is_finite() && volume > 0.0
        })
    }
}
