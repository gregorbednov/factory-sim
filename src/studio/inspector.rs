use eframe::egui::{self, RichText, Ui};

use super::{i18n::Language, production_ui};
use crate::{
    production::Equipment,
    project::{Part, PartKind, Pipe, Project, Tag, Value},
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
        .filter(|belt| belt.feed_station(part.position).is_some())
        .map(|part| (part.id.clone(), part.name.clone()))
        .collect();
    let unload_robot = project.parts.iter().any(|station| {
        matches!(&station.kind, PartKind::Machine { machine } if machine.unload_robot.as_deref() == Some(part.id.as_str()))
    });
    let manufactured = |id: &str| {
        project
            .parts
            .iter()
            .any(|part| matches!(&part.kind, PartKind::Machine { machine } if machine.output == id))
    };
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
            let emitter = matches!(part.kind, PartKind::Emitter { .. });
            changed |= production_ui::filtered_product_picker(
                ui,
                language,
                "Product",
                &mut part.product,
                products,
                Some("Manual box"),
                |id| !emitter || !manufactured(id),
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
                changed |= production_ui::emission(ui, language, emission, products, |id| {
                    !manufactured(id)
                });
                let emits_manufactured = emission.as_ref().map_or_else(
                    || part.product.as_deref().is_some_and(manufactured),
                    |recipe| recipe.products().any(manufactured),
                );
                if emits_manufactured {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        language
                            .text("Choose a product without a production recipe for the emitter."),
                    );
                }
                let before = random_spawn_belt.clone();
                let belt_name = feed_belts
                    .iter()
                    .find_map(|(id, name)| (Some(id) == before.as_ref()).then_some(name.as_str()))
                    .unwrap_or_else(|| {
                        language.text(if before.is_some() {
                            "Conveyor outside the feed area"
                        } else {
                            "Fixed position"
                        })
                    });
                if before.is_some() && !feed_belts.iter().any(|(id, _)| Some(id) == before.as_ref())
                {
                    ui.label(
                        language.text("Place the emitter above its conveyor to resume feeding."),
                    );
                }
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
            PartKind::HeatedTank(tank) => {
                changed |= dimensions(ui, &mut tank.size, 0.02..=1000.0);
                for (label, value) in [
                    (
                        "Initial water temperature (°C)",
                        &mut tank.initial_temperature_c,
                    ),
                    ("Ambient temperature (°C)", &mut tank.ambient_temperature_c),
                ] {
                    changed |= parameter(ui, language.text(label), value, 0.0..=99.9, 0.1);
                }
                for (label, value, min, speed) in [
                    ("Working volume (L)", &mut tank.volume_l, 0.01, 0.1),
                    ("Heat loss (W/K)", &mut tank.heat_loss_w_per_k, 0.0, 0.1),
                    (
                        "Maximum heater power (W)",
                        &mut tank.heater_max_w,
                        0.01,
                        1.0,
                    ),
                ] {
                    changed |= parameter(ui, language.text(label), value, min..=f32::MAX, speed);
                }
                if tank.networked {
                    let mut initial = tank.starting_volume_l().clamp(0.0, tank.volume_l);
                    if parameter(
                        ui,
                        language.text("Initial water volume (L)"),
                        &mut initial,
                        0.0..=tank.volume_l,
                        0.1,
                    ) || tank
                        .initial_volume_l
                        .is_some_and(|volume| volume > tank.volume_l)
                    {
                        tank.initial_volume_l = Some(initial);
                        changed = true;
                    }
                } else {
                    changed |= parameter(
                        ui,
                        language.text("Feed temperature (°C)"),
                        &mut tank.feed_temperature_c,
                        0.0..=99.9,
                        0.1,
                    );
                    changed |= parameter(
                        ui,
                        language.text("Maximum feed (L/s)"),
                        &mut tank.feed_max_l_s,
                        0.0..=f32::MAX,
                        0.001,
                    );
                }
            }
            PartKind::WaterSource {
                size,
                temperature_c,
                max_flow_l_s,
                dirt_mg_l,
            } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
                changed |= parameter(
                    ui,
                    language.text("Water temperature (°C)"),
                    temperature_c,
                    0.0..=99.9,
                    0.1,
                );
                changed |= parameter(
                    ui,
                    language.text("Water dirt (mg/L)"),
                    dirt_mg_l,
                    0.0..=f32::MAX,
                    1.0,
                );
                changed |= parameter(
                    ui,
                    language.text("Maximum flow (L/s)"),
                    max_flow_l_s,
                    0.0..=f32::MAX,
                    0.001,
                );
            }
            PartKind::WaterPump {
                size,
                max_flow_l_s,
                shutoff_head_m,
                dirt_limit_mg,
            } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
                changed |= parameter(
                    ui,
                    language.text("Rated flow at zero head (L/s)"),
                    max_flow_l_s,
                    0.0..=f32::MAX,
                    0.001,
                );
                changed |= parameter(
                    ui,
                    language.text("Shutoff head (m)"),
                    shutoff_head_m,
                    f32::MIN_POSITIVE..=f32::MAX,
                    0.05,
                );
                changed |= parameter(
                    ui,
                    language.text("Pump dirt limit (mg)"),
                    dirt_limit_mg,
                    0.01..=f32::MAX,
                    1.0,
                );
            }
            PartKind::WaterValve {
                size,
                max_flow_l_s,
                open_resistance,
            } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
                changed |= parameter(
                    ui,
                    language.text("Maximum flow (L/s)"),
                    max_flow_l_s,
                    0.0..=f32::MAX,
                    0.001,
                );
                changed |= parameter(
                    ui,
                    language.text("Fully open valve resistance"),
                    open_resistance,
                    0.0..=f32::MAX,
                    0.1,
                );
            }
            PartKind::WaterDrain { size, max_flow_l_s } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
                changed |= parameter(
                    ui,
                    language.text("Maximum flow (L/s)"),
                    max_flow_l_s,
                    0.0..=f32::MAX,
                    0.001,
                );
            }
            PartKind::WaterJunction { size, .. } | PartKind::WaterInstrument { size, .. } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
            }
            PartKind::WaterFilter {
                size,
                clean_resistance,
                dirty_resistance,
                capacity_mg,
                efficiency_pct,
            } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
                changed |= parameter(
                    ui,
                    language.text("Clean filter resistance"),
                    clean_resistance,
                    0.0..=*dirty_resistance,
                    0.1,
                );
                changed |= parameter(
                    ui,
                    language.text("Loaded filter resistance"),
                    dirty_resistance,
                    *clean_resistance..=f32::MAX,
                    0.1,
                );
                changed |= parameter(
                    ui,
                    language.text("Filter capacity (mg)"),
                    capacity_mg,
                    0.01..=f32::MAX,
                    1.0,
                );
                changed |= parameter(
                    ui,
                    language.text("Filter efficiency (%)"),
                    efficiency_pct,
                    0.0..=100.0,
                    0.1,
                );
            }
            PartKind::WaterHeater { size, max_power_w } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
                changed |= parameter(
                    ui,
                    language.text("Maximum heater power (W)"),
                    max_power_w,
                    0.01..=f32::MAX,
                    1.0,
                );
            }
            PartKind::WaterHeatExchanger {
                size,
                conductance_w_k,
                resistance,
            } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
                changed |= parameter(
                    ui,
                    language.text("Thermal conductance (W/K)"),
                    conductance_w_k,
                    0.01..=f32::MAX,
                    1.0,
                );
                changed |= parameter(
                    ui,
                    language.text("Circuit resistance"),
                    resistance,
                    0.0..=f32::MAX,
                    0.1,
                );
            }
            PartKind::WaterFiller {
                size,
                max_flow_l_s,
                dose_l,
            } => {
                changed |= dimensions(ui, size, 0.02..=1000.0);
                changed |= parameter(
                    ui,
                    language.text("Maximum flow (L/s)"),
                    max_flow_l_s,
                    0.0001..=f32::MAX,
                    0.001,
                );
                changed |= parameter(
                    ui,
                    language.text("Target dose (L)"),
                    dose_l,
                    0.0001..=f32::MAX,
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

pub(super) fn pipes(
    ui: &mut Ui,
    language: Language,
    project: &Project,
    id: &str,
    editable: bool,
) -> Option<Vec<Pipe>> {
    let mut change = None;
    for (outlet, label) in [(false, "Water inlet"), (true, "Water outlet")] {
        let count = project
            .parts
            .iter()
            .find(|part| part.id == id)
            .unwrap()
            .kind
            .water_port_count(outlet);
        for port in 0..count {
            let label_text = language.text(label);
            if count > 1 {
                ui.label(format!("{} {}", label_text, port + 1));
            } else {
                ui.label(label_text);
            }
            change = pipe_port(ui, language, project, id, editable, outlet, port).or(change);
        }
    }
    change
}

fn pipe_port(
    ui: &mut Ui,
    language: Language,
    project: &Project,
    id: &str,
    editable: bool,
    outlet: bool,
    port: u8,
) -> Option<Vec<Pipe>> {
    let link = project.pipes.iter().enumerate().find(|(_, pipe)| {
        if outlet {
            pipe.from == id && pipe.from_port == port
        } else {
            pipe.to == id && pipe.to_port == port
        }
    });
    let mut connected = None;
    let mut removed = None;
    let mut updated = None;
    if let Some((index, pipe)) = link {
        let other_id = if outlet { &pipe.to } else { &pipe.from };
        let other = project
            .parts
            .iter()
            .find(|part| &part.id == other_id)
            .unwrap();
        let other_port = if outlet { pipe.to_port } else { pipe.from_port };
        ui.label(format!(
            "{} ({}) · {}",
            other.name,
            other.id,
            other_port + 1
        ));
        let mut resistance = pipe.resistance_per_m;
        ui.add_enabled_ui(editable, |ui| {
            if parameter(
                ui,
                language.text("Pipe resistance per metre"),
                &mut resistance,
                f32::MIN_POSITIVE..=f32::MAX,
                0.1,
            ) {
                updated = Some((index, resistance));
            }
        });
        if ui
            .add_enabled(
                editable,
                egui::Button::new(language.text("Disconnect pipe")),
            )
            .clicked()
        {
            removed = Some(index);
        }
    } else {
        ui.add_enabled_ui(editable, |ui| {
            egui::ComboBox::from_id_salt((id, outlet, port))
                .selected_text(language.text("Connect pipe…"))
                .show_ui(ui, |ui| {
                    let mut available = false;
                    for other in &project.parts {
                        for other_port in 0..other.kind.water_port_count(!outlet) {
                            let mut pipe = if outlet {
                                Pipe::new(id, other.id.as_str())
                            } else {
                                Pipe::new(other.id.as_str(), id)
                            };
                            (pipe.from_port, pipe.to_port) = if outlet {
                                (port, other_port)
                            } else {
                                (other_port, port)
                            };
                            if project.can_connect_pipe(&pipe) {
                                available = true;
                                if ui
                                    .selectable_label(
                                        false,
                                        format!(
                                            "{} ({}) · {}",
                                            other.name,
                                            other.id,
                                            other_port + 1
                                        ),
                                    )
                                    .clicked()
                                {
                                    connected = Some(pipe);
                                }
                            }
                        }
                    }
                    if !available {
                        ui.label(language.text("No compatible free port"));
                    }
                });
        });
    }
    if connected.is_some() || removed.is_some() || updated.is_some() {
        let mut pipes = project.pipes.clone();
        if let Some(pipe) = connected {
            pipes.push(pipe);
        }
        if let Some((index, resistance)) = updated {
            pipes[index].resistance_per_m = resistance;
        }
        if let Some(index) = removed {
            pipes.remove(index);
        }
        ui.separator();
        return Some(pipes);
    }
    ui.separator();
    None
}

pub(super) fn tag_label<'a>(language: Language, project: &'a Project, tag: &'a Tag) -> &'a str {
    project.tag_names.get(&tag.id).map_or_else(
        || {
            language.text(
                tag.name
                    .rsplit_once(" · ")
                    .map_or(&tag.name, |(_, role)| role),
            )
        },
        String::as_str,
    )
}

pub(super) fn tag_input(ui: &mut Ui, tag: &Tag, value: &mut Value) {
    match value {
        Value::Bool(value) => {
            ui.checkbox(value, "");
        }
        Value::Float(value) => {
            let mut drag = egui::DragValue::new(value).speed(0.02).max_decimals(3);
            if tag.id.ends_with("flow_l_s") {
                drag = drag.min_decimals(4).max_decimals(6);
            }
            if let Some([lo, hi]) = tag.range {
                drag = drag.range(lo..=hi);
            }
            ui.add(drag);
        }
        Value::Int(value) => {
            ui.add(egui::DragValue::new(value));
        }
    }
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
