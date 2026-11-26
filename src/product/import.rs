use std::{
    collections::HashMap,
    fs::File,
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};

use anyhow::{Context, Result, bail, ensure};
use truck_meshalgo::prelude::{MeshedShape, RobustMeshableShape};
use truck_stepio::r#in::{Table, ruststep};

use super::{Geometry, MAX_TRIANGLES, Mesh, Product};

const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
pub enum UpAxis {
    Y,
    Z,
}

pub fn import(path: &Path, meters_per_unit: f64, up: UpAxis, mass: f32) -> Result<Product> {
    ensure!(
        meters_per_unit.is_finite() && meters_per_unit > 0.0,
        "invalid scale"
    );
    let mut data = Vec::new();
    File::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut data)?;
    ensure!(data.len() as u64 <= MAX_BYTES, "model file exceeds 32 MiB");
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let (geometry, raw) = match extension.to_ascii_lowercase().as_str() {
        "step" | "stp" => {
            let source = String::from_utf8(data).context("STEP must be a text file")?;
            let tolerance = (0.0001 / meters_per_unit).max(0.000001);
            let mesh = std::panic::catch_unwind(|| read_step(&source, tolerance))
                .map_err(|_| anyhow::anyhow!("CAD kernel could not tessellate this STEP file"))??;
            (
                Geometry::Step {
                    source: source.into(),
                    meters_per_unit,
                    z_up: up == UpAxis::Z,
                },
                mesh,
            )
        }
        "obj" => (Geometry::Obj, read_obj(&data)?),
        "stl" => {
            let mesh = stl_io::read_stl(&mut Cursor::new(data)).context("read STL mesh")?;
            ensure!(
                mesh.faces.len() <= MAX_TRIANGLES,
                "model exceeds the triangle limit"
            );
            (
                Geometry::Stl,
                RawMesh {
                    positions: mesh
                        .vertices
                        .into_iter()
                        .map(|v| [v[0], v[1], v[2]].map(f64::from))
                        .collect(),
                    triangles: mesh
                        .faces
                        .into_iter()
                        .map(|f| f.vertices.map(|i| i as u32))
                        .collect(),
                    normals: None,
                },
            )
        }
        _ => bail!("choose a STEP, OBJ or STL model"),
    };
    let (mesh, size) = raw.normalize(meters_per_unit, up)?;
    let product = Product {
        name: path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Imported product")
            .into(),
        size,
        mass,
        geometry,
        mesh: Arc::new(mesh),
    };
    product.validate()?;
    ensure!(
        product.mesh.collision_shape().is_some(),
        "model must enclose a three-dimensional volume for physics"
    );
    Ok(product)
}

#[derive(Default)]
struct RawMesh {
    positions: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    normals: Option<Vec<[f64; 3]>>,
}

impl RawMesh {
    fn normalize(self, meters_per_unit: f64, up: UpAxis) -> Result<(Mesh, [f32; 3])> {
        ensure!(
            !self.triangles.is_empty() && self.triangles.len() <= MAX_TRIANGLES,
            "model must contain 1 to {MAX_TRIANGLES} triangles"
        );
        // Ignore isolated vertices so render bounds and collision use the same surface.
        let mut used = vec![false; self.positions.len()];
        let mut triangles = self.triangles;
        for &i in triangles.iter().flatten() {
            *used.get_mut(i as usize).context("invalid triangle index")? = true;
        }
        let mut lower = [f64::INFINITY; 3];
        let mut upper = [f64::NEG_INFINITY; 3];
        let orient = |[x, y, z]: [f64; 3]| match up {
            UpAxis::Y => [x, y, z],
            UpAxis::Z => [x, z, -y],
        };
        let mut positions = Vec::new();
        let mut normals = self.normals.as_ref().map(|_| Vec::new());
        let mut remap = vec![0; used.len()];
        for (i, point) in self.positions.into_iter().enumerate() {
            if !used[i] {
                continue;
            }
            let point = orient(point);
            if let (Some(output), Some(input)) = (&mut normals, &self.normals) {
                let normal = glam::DVec3::from(orient(input[i]));
                output.push(normal.normalize_or_zero().as_vec3().to_array());
            }
            for axis in 0..3 {
                ensure!(point[axis].is_finite(), "non-finite model coordinate");
                lower[axis] = lower[axis].min(point[axis]);
                upper[axis] = upper[axis].max(point[axis]);
            }
            remap[i] = positions.len() as u32;
            positions.push(point);
        }
        for i in triangles.iter_mut().flatten() {
            *i = remap[*i as usize];
        }
        let center: [f64; 3] = std::array::from_fn(|axis| (lower[axis] + upper[axis]) * 0.5);
        let size =
            std::array::from_fn(|axis| ((upper[axis] - lower[axis]) * meters_per_unit) as f32);
        let positions: Vec<_> = positions
            .into_iter()
            .map(|p| {
                std::array::from_fn(|axis| ((p[axis] - center[axis]) * meters_per_unit) as f32)
            })
            .collect();
        let vertex = |index: u32| glam::Vec3::from(positions[index as usize]);
        triangles.retain(|&[a, b, c]| {
            let [a, b, c] = [vertex(a), vertex(b), vertex(c)];
            (b - a).cross(c - a).length_squared() > 0.0
        });
        Ok((
            Mesh {
                positions,
                triangles,
                normals,
            },
            size,
        ))
    }
}

fn read_obj(data: &[u8]) -> Result<RawMesh> {
    let options = tobj::LoadOptions {
        triangulate: true,
        single_index: true,
        ignore_lines: true,
        ignore_points: true,
        ..Default::default()
    };
    // Imports are self-contained and never open files referenced by OBJ materials.
    let (models, _) = tobj::load_obj_buf(&mut Cursor::new(data), &options, |_| {
        Ok((Vec::new(), Default::default()))
    })
    .context("read OBJ mesh")?;
    let mut output = RawMesh::default();
    let mut normals = Vec::new();
    for model in models {
        if model.mesh.positions.len() == model.mesh.normals.len() {
            normals.extend(
                model
                    .mesh
                    .normals
                    .chunks_exact(3)
                    .map(|n| [n[0], n[1], n[2]]),
            );
        } else {
            normals.resize(normals.len() + model.mesh.positions.len() / 3, [0.0; 3]);
        }
        let offset = output.positions.len() as u32;
        output.positions.extend(
            model
                .mesh
                .positions
                .chunks_exact(3)
                .map(|p| [p[0], p[1], p[2]]),
        );
        output.triangles.extend(
            model
                .mesh
                .indices
                .chunks_exact(3)
                .map(|i| [i[0] + offset, i[1] + offset, i[2] + offset]),
        );
    }
    if normals.iter().any(|normal| *normal != [0.0; 3]) {
        output.normals = Some(normals);
    }
    Ok(output)
}

fn read_step(source: &str, tolerance: f64) -> Result<RawMesh> {
    let exchange =
        ruststep::parser::parse(source).map_err(|e| anyhow::anyhow!("invalid STEP: {e}"))?;
    ensure!(
        exchange.data.len() == 1,
        "expected a standalone STEP part with one data section"
    );
    let table = Table::from_data_section(&exchange.data[0]);
    ensure!(
        table.shell.len() == 1,
        "expected a STEP part with a single shell"
    );
    let holder = table.shell.values().next().unwrap();
    let shell = table
        .to_compressed_shell(holder)
        .map_err(|e| anyhow::anyhow!("read STEP B-Rep: {e}"))?;
    ensure!(
        shell.faces.len() == holder.cfs_faces.len(),
        "one or more STEP faces could not be converted"
    );
    let meshed = shell.robust_triangulation(tolerance);
    ensure!(
        meshed.faces.iter().all(|face| face.surface.is_some()),
        "one or more STEP faces could not be tessellated"
    );
    let polygon = meshed.to_polygon();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut triangles = Vec::new();
    let mut indices = HashMap::new();
    for face in polygon.faces().triangle_iter() {
        ensure!(
            triangles.len() < MAX_TRIANGLES,
            "model exceeds the triangle limit"
        );
        // Normals on opposite sides of a B-Rep edge can differ at the same position.
        triangles.push(face.map(|vertex| {
            *indices.entry((vertex.pos, vertex.nor)).or_insert_with(|| {
                let point = polygon.positions()[vertex.pos];
                let normal = vertex.nor.map_or([0.0; 3], |index| {
                    let normal = polygon.normals()[index];
                    [normal.x, normal.y, normal.z]
                });
                let index = positions.len() as u32;
                positions.push([point.x, point.y, point.z]);
                normals.push(normal);
                index
            })
        }));
    }
    Ok(RawMesh {
        positions,
        triangles,
        normals: Some(normals),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        project::{Project, Value},
        sim::Simulation,
    };

    const OBJ: &str =
        "v 0 0 0\nv 200 0 0\nv 0 300 0\nv 0 0 400\nf 1 3 2\nf 1 2 4\nf 1 4 3\nf 2 3 4\n";

    #[test]
    fn obj_product_survives_project_save_and_is_emitted() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let model = temp.path().join("Wedge.obj");
        std::fs::write(&model, OBJ)?;
        let product = import(&model, 0.001, UpAxis::Z, 1.7)?;
        assert_eq!(product.size, [0.2, 0.4, 0.3]);
        assert_eq!(product.mesh.triangles.len(), 4);
        let mut project = Project::demo();
        project.products.insert("wedge".into(), product);
        for part in &mut project.parts {
            if matches!(part.id.as_str(), "parcel" | "supply") {
                part.product = Some("wedge".into());
            }
        }
        let project_path = temp.path().join("project.json");
        project.save(&project_path)?;
        std::fs::remove_file(model)?;
        let project = Project::load(&project_path)?;
        let mut simulation = Simulation::new(project)?;
        let initial = &simulation.snapshot().parcels[0];
        assert_eq!(initial.product.as_deref(), Some("wedge"));
        assert_eq!(initial.size, [0.2, 0.4, 0.3]);
        simulation.command("supply.enabled", Value::Bool(true))?;
        simulation.step();
        let snapshot = simulation.snapshot();
        let emitted = snapshot
            .parcels
            .iter()
            .find(|parcel| parcel.id.starts_with("supply#"))
            .context("expected emitted product")?;
        assert_eq!(emitted.product.as_deref(), Some("wedge"));
        assert_eq!(emitted.size, initial.size);
        Ok(())
    }

    #[test]
    fn step_retains_brep_and_tessellates() -> Result<()> {
        use truck_modeling::{Point3, Vector3, builder};
        use truck_stepio::out::{CompleteStepDisplay, StepModel};

        let vertex = builder::vertex(Point3::new(0.0, 0.0, 0.0));
        let edge = builder::tsweep(&vertex, Vector3::new(200.0, 0.0, 0.0));
        let face = builder::tsweep(&edge, Vector3::new(0.0, 300.0, 0.0));
        let solid = builder::tsweep(&face, Vector3::new(0.0, 0.0, 400.0)).compress();
        let source =
            CompleteStepDisplay::new(StepModel::from(&solid), Default::default()).to_string();
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("Block.step");
        std::fs::write(&path, &source)?;
        let product = import(&path, 0.001, UpAxis::Z, 2.0)?;
        let Geometry::Step {
            source: retained,
            z_up: true,
            ..
        } = &product.geometry
        else {
            panic!("expected a native STEP source");
        };
        assert_eq!(&**retained, source);
        assert_eq!(product.size, [0.2, 0.4, 0.3]);
        assert_eq!(product.mesh.triangles.len(), 12);
        Ok(())
    }

    #[test]
    fn imports_stl_and_rejects_flat_mesh() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("Wedge.stl");
        let mesh = read_obj(OBJ.as_bytes())?;
        let triangles = mesh.triangles.iter().map(|indices| stl_io::Triangle {
            normal: stl_io::Normal::new([0.0; 3]),
            vertices: indices
                .map(|index| stl_io::Vertex::new(mesh.positions[index as usize].map(|x| x as f32))),
        });
        stl_io::write_stl(&mut File::create(&path)?, triangles)?;
        let imported = import(&path, 0.001, UpAxis::Y, 2.0)?;
        assert_eq!(imported.size, [0.2, 0.3, 0.4]);
        assert!(matches!(imported.geometry, Geometry::Stl));

        let path = dir.path().join("Flat.obj");
        std::fs::write(&path, "v 0 0 0\nv 1 0 1\nv 0 1 1\nf 1 2 3\n")?;
        assert!(import(&path, 1.0, UpAxis::Y, 1.0).is_err());
        Ok(())
    }
}
