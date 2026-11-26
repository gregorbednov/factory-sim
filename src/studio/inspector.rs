use eframe::egui::{self, RichText, Ui};

use super::{i18n::Language, production_ui};
use crate::{
    production::Equipment,
    project::{Part, PartKind, Project},
};

pub(super) fn properties(
    ui: &mut Ui,
    language: Language,
    project: &Project,
    index: usize,
    editable: bool,
) -> Option<Part> {
    let mut part = project.parts.get(index)?.clone();
    let products = &project.products;
    let production = &project.production;
    let doors: Vec<_> = project
        .parts
        .iter()
        .filter(|part| {
            matches!(
                part.kind,
                PartKind::Equipment {
                    equipment: Equipment::SafetyDoor,
                    ..
                }
            )
        })
        .map(|part| (part.id.clone(), part.name.clone()))
        .collect();
    let robots: Vec<_> = project
        .parts
        .iter()
        .filter(|part| matches!(part.kind, PartKind::Robot { .. }))
        .map(|part| (part.id.clone(), part.name.clone()))
        .collect();
    let feed_belts: Vec<_> = project
        .parts
        .iter()
        .filter(|part| part.kind.feed_surface().is_some())
        .map(|part| (part.id.clone(), part.name.clone()))
        .collect();
    let unload_robot = project.parts.iter().any(|station| {
        matches!(&station.kind, PartKind::Machine { machine } if machine.unload_robot.as_deref() == Some(part.id.as_str()))
    });
    let mut changed = false;
    ui.label(
        RichText::new(language.text(part.kind.label()))
            .size(18.0)
            .strong(),
    );
    ui.label(
        RichText::new(&part.id)
            .monospace()
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_enabled_ui(editable, |ui| {
        changed |= ui.text_edit_singleline(&mut part.name).changed();
        ui.label(
            RichText::new(language.text("Position (m)")).color(ui.visuals().weak_text_color()),
        );
        changed |= dimensions(ui, &mut part.position, -1000.0..=1000.0);
        changed |= parameter(
            ui,
            language.text("Yaw (°)"),
            &mut part.yaw,
            -360.0..=360.0,
            1.0,
        );
        if matches!(
            part.kind,
            PartKind::Emitter { emission: None, .. } | PartKind::Crate { .. }
        ) {
            changed |= production_ui::product_picker(
                ui,
                language,
                "Product",
                &mut part.product,
                products,
                Some("Manual box"),
            );
        }
        match &mut part.kind {
            PartKind::Conveyor { size, speed } => {
                ui.label(
                    RichText::new(language.text("Length / height / width"))
                        .color(ui.visuals().weak_text_color()),
                );
                changed |= dimensions(ui, size, 0.02..=1000.0);
                changed |= parameter(
                    ui,
                    language.text("Max speed (m/s)"),
                    speed,
                    0.02..=1000.0,
                    0.05,
                );
            }
            PartKind::Pusher { stroke, speed } => {
                changed |= parameter(ui, language.text("Stroke (m)"), stroke, 0.02..=1000.0, 0.02);
                changed |= parameter(ui, language.text("Speed (m/s)"), speed, 0.02..=1000.0, 0.05);
            }
            PartKind::Sensor { range } => {
                changed |= parameter(ui, language.text("Range (m)"), range, 0.02..=1000.0, 0.02);
            }
            PartKind::Emitter {
                interval,
                size,
                mass,
                emission,
                random_spawn_belt,
            } => {
                changed |= parameter(
                    ui,
                    language.text("Interval (s)"),
                    interval,
                    0.02..=1000.0,
                    0.05,
                );
                changed |= production_ui::emission(ui, language, emission, products);
                let before = random_spawn_belt.clone();
                let belt_name = feed_belts
                    .iter()
                    .find_map(|(id, name)| (Some(id) == before.as_ref()).then_some(name.as_str()))
                    .unwrap_or(language.text("Fixed position"));
                egui::ComboBox::from_label(language.text("Random spawn on conveyor"))
                    .selected_text(belt_name)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            random_spawn_belt,
                            None,
                            language.text("Fixed position"),
                        );
                        for (id, name) in &feed_belts {
                            ui.selectable_value(random_spawn_belt, Some(id.clone()), name);
                        }
                    });
                changed |= before != *random_spawn_belt;
                if emission.is_none() && part.product.is_none() {
                    changed |= parameter(ui, language.text("Mass (kg)"), mass, 0.02..=1000.0, 0.05);
                    changed |= dimensions(ui, size, 0.02..=1000.0);
                }
            }
            PartKind::Crate { size, mass } => {
                if part.product.is_none() {
                    changed |= parameter(ui, language.text("Mass (kg)"), mass, 0.02..=1000.0, 0.05);
                    changed |= dimensions(ui, size, 0.02..=1000.0);
                }
            }
            PartKind::Equipment {
                equipment,
                size,
                scale_capacity,
            } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
                if *equipment == Equipment::Scale {
                    ui.horizontal(|ui| {
                        changed |= ui
                            .selectable_value(scale_capacity, 20, language.text("20 kg"))
                            .changed();
                        changed |= ui
                            .selectable_value(scale_capacity, 100, language.text("100 kg"))
                            .changed();
                    });
                }
            }
            PartKind::Machine { machine } => {
                changed |= production_ui::machine(ui, language, machine, products, &doors, &robots)
            }
            PartKind::Robot { robot } => {
                if unload_robot {
                    ui.label(language.text("This robot unloads machines automatically."));
                }
                changed |= production_ui::robot(ui, language, robot, unload_robot)
            }
            PartKind::Staff { staff } => {
                changed |= production_ui::staff(ui, language, staff, production)
            }
            PartKind::FlowGuide {
                length,
                height,
                inlet,
                outlet,
            } => {
                for (label, value) in [
                    ("Length (m)", length),
                    ("Height (m)", height),
                    ("Inlet width (m)", &mut *inlet),
                ] {
                    changed |= parameter(ui, language.text(label), value, 0.02..=1000.0, 0.01);
                }
                if *outlet > *inlet {
                    *outlet = *inlet;
                    changed = true;
                }
                changed |= parameter(
                    ui,
                    language.text("Outlet width (m)"),
                    outlet,
                    0.02..=*inlet,
                    0.01,
                );
            }
            PartKind::Remover { size } | PartKind::Barrier { size } => {
                changed |= dimensions(ui, size, 0.02..=1000.0)
            }
        }
    });
    (editable && changed).then_some(part)
}

pub(super) fn dimensions(
    ui: &mut Ui,
    values: &mut [f32; 3],
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.horizontal(|ui| {
        let mut changed = false;
        for (name, value) in ["X", "Y", "Z"].into_iter().zip(values) {
            changed |= ui
                .add(
                    egui::DragValue::new(value)
                        .prefix(format!("{name} "))
                        .range(range.clone())
                        .speed(0.02),
                )
                .changed();
        }
        changed
    })
    .inner
}

pub(super) fn parameter(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).range(range).speed(speed))
            .changed()
    })
    .inner
}
