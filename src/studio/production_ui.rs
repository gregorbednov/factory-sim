use super::i18n::Language;
use eframe::egui::{self, Ui};
use std::collections::BTreeMap;

use crate::{
    product::{BOXES, Product},
    production::{
        AssemblyMode, BomLine, CountRow, Emission, MAX_ASSEMBLY_COMPONENTS, Machine, Probability,
        ProbabilityRow, Production, Robot, RobotStep, Staff,
    },
};

pub fn product_picker(
    ui: &mut Ui,
    language: Language,
    caption: &str,
    id: &mut Option<String>,
    products: &BTreeMap<String, Product>,
    none: Option<&str>,
) -> bool {
    filtered_product_picker(ui, language, caption, id, products, none, |_| true)
}

fn filtered_product_picker(
    ui: &mut Ui,
    language: Language,
    caption: &str,
    id: &mut Option<String>,
    products: &BTreeMap<String, Product>,
    none: Option<&str>,
    include: impl Fn(&str) -> bool,
) -> bool {
    let name = id
        .as_ref()
        .and_then(|id| {
            products.get(id).map(|p| p.name.as_str()).or_else(|| {
                BOXES
                    .iter()
                    .find_map(|(key, spec)| (*key == id).then(|| language.text(spec.name)))
            })
        })
        .unwrap_or_else(|| language.text(none.unwrap_or("Select product")));
    let previous = id.clone();
    egui::ComboBox::from_label(language.text(caption))
        .selected_text(name)
        .show_ui(ui, |ui| {
            if let Some(label) = none {
                ui.selectable_value(id, None, language.text(label));
            }
            for (key, spec) in BOXES.iter().filter(|(key, _)| include(key)) {
                ui.selectable_value(id, Some((*key).into()), language.text(spec.name));
            }
            for (key, product) in products.iter().filter(|(key, _)| include(key)) {
                ui.selectable_value(id, Some(key.clone()), &product.name);
            }
        });
    previous != *id
}

pub fn emission(
    ui: &mut Ui,
    language: Language,
    emission: &mut Option<Emission>,
    products: &BTreeMap<String, Product>,
) -> bool {
    let mut mode = match emission {
        None => 0,
        Some(Emission::Counts { .. }) => 1,
        Some(Emission::Probability { .. }) => 2,
    };
    let previous = mode;
    egui::ComboBox::from_label(language.text("Emission"))
        .selected_text(language.text(["Single product", "Exact series", "Probabilities"][mode]))
        .show_ui(ui, |ui| {
            for (index, label) in ["Single product", "Exact series", "Probabilities"]
                .into_iter()
                .enumerate()
            {
                ui.selectable_value(&mut mode, index, language.text(label));
            }
        });
    if mode != previous {
        *emission = match mode {
            1 => Some(Emission::Counts {
                rows: vec![CountRow {
                    product: Some("box-carton".into()),
                    count: 1,
                }],
                skip: None,
            }),
            2 => Some(Emission::Probability {
                rows: vec![ProbabilityRow {
                    product: Some("box-carton".into()),
                    probability: Probability::Fixed(1.0),
                }],
            }),
            _ => None,
        };
        return true;
    }
    let mut changed = false;
    let mut remove = None;
    match emission {
        Some(Emission::Counts { rows, skip }) => {
            let mut enabled = skip.is_some();
            if ui
                .checkbox(&mut enabled, language.text("Skip before drawing"))
                .changed()
            {
                *skip = enabled.then_some(Probability::Fixed(0.0));
                changed = true;
            }
            if let Some(skip) = skip {
                changed |= probability(ui, language, skip);
            }
            for (index, row) in rows.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    changed |= product_picker(
                        ui,
                        language,
                        "Outcome",
                        &mut row.product,
                        products,
                        Some("Skip"),
                    );
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut row.count)
                                    .prefix(language.text("Count ")),
                            )
                            .changed();
                        if ui.small_button(language.text("Remove")).clicked() {
                            remove = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = remove {
                rows.remove(index);
                changed = true;
            }
            if rows.len() < 128 && ui.button(language.text("Add outcome")).clicked() {
                rows.push(CountRow {
                    product: None,
                    count: 1,
                });
                changed = true;
            }
        }
        Some(Emission::Probability { rows }) => {
            ui.label(language.text("Unassigned probability skips an emission."));
            for (index, row) in rows.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    changed |= product_picker(
                        ui,
                        language,
                        "Outcome",
                        &mut row.product,
                        products,
                        Some("Skip"),
                    );
                    changed |= probability(ui, language, &mut row.probability);
                    if ui.small_button(language.text("Remove outcome")).clicked() {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                rows.remove(index);
                changed = true;
            }
            if rows.len() < 128 && ui.button(language.text("Add outcome")).clicked() {
                rows.push(ProbabilityRow {
                    product: None,
                    probability: Probability::Fixed(0.0),
                });
                changed = true;
            }
        }
        None => {}
    }
    changed
}

fn probability(ui: &mut Ui, language: Language, value: &mut Probability) -> bool {
    let mut changed = false;
    let mut formula = matches!(value, Probability::Formula(_));
    if ui
        .checkbox(&mut formula, language.text("Formula"))
        .changed()
    {
        *value = if formula {
            Probability::Formula("0".into())
        } else {
            Probability::Fixed(0.0)
        };
        changed = true;
    }
    match value {
        Probability::Fixed(value) => {
            let mut percent = *value * 100.0;
            if ui
                .add(
                    egui::DragValue::new(&mut percent)
                        .range(0.0..=100.0)
                        .suffix(" %"),
                )
                .changed()
            {
                *value = percent / 100.0;
                changed = true;
            }
        }
        Probability::Formula(text) => {
            changed |= ui
                .add(egui::TextEdit::singleline(text).hint_text("0.5 * (t < 120)"))
                .on_hover_text(language.text("t: seconds, hour: 0–24, weekday: 1–7. Result: 0–1."))
                .changed();
        }
    }
    changed
}

fn duration(ui: &mut Ui, language: Language, name: &str, seconds: &mut f64) -> bool {
    ui.push_id(name, |ui| {
        ui.horizontal(|ui| {
            ui.label(language.text(name));
            let id = ui.scope_id().with("unit");
            let mut unit: usize = ui.data(|data| data.get_temp(id).unwrap_or(0));
            let labels = ["s", "min", "h"].map(|label| language.text(label));
            egui::ComboBox::from_id_salt("unit")
                .selected_text(labels[unit])
                .width(48.0)
                .show_ui(ui, |ui| {
                    for (index, label) in labels.into_iter().enumerate() {
                        ui.selectable_value(&mut unit, index, label);
                    }
                });
            ui.data_mut(|data| data.insert_temp(id, unit));
            let factor = [1.0, 60.0, 3600.0][unit];
            let mut value = *seconds / factor;
            if ui
                .add(
                    egui::DragValue::new(&mut value)
                        .range(1.0 / 120.0 / factor..=1e9)
                        .speed(0.1)
                        .max_decimals(6),
                )
                .changed()
            {
                *seconds = value * factor;
                true
            } else {
                false
            }
        })
        .inner
    })
    .inner
}

pub fn machine(
    ui: &mut Ui,
    language: Language,
    machine: &mut Machine,
    products: &BTreeMap<String, Product>,
    doors: &[(String, String)],
    robots: &[(String, String)],
) -> bool {
    ui.label(language.text("Area"));
    let mut changed = ui.text_edit_singleline(&mut machine.area).changed();
    if let Some(assembly) = &mut machine.assembly {
        ui.label(language.text(match assembly.mode {
            AssemblyMode::Manual => "Requires an on-duty operator in this area",
            AssemblyMode::Automatic => "Runs without an operator",
        }));
        ui.separator();
        ui.label(language.text("Bill of materials"));
        let mut remove = None;
        let mut total: u32 = assembly.bom.iter().map(|row| row.count).sum();
        for index in 0..assembly.bom.len() {
            let row = &assembly.bom[index];
            let mut product = Some(row.product.clone());
            let mut count = row.count;
            let maximum = MAX_ASSEMBLY_COMPONENTS - (total - count);
            ui.push_id(("bom-row", index), |ui| {
                changed |= filtered_product_picker(
                    ui,
                    language,
                    "Component",
                    &mut product,
                    products,
                    None,
                    |id| {
                        assembly
                            .bom
                            .iter()
                            .enumerate()
                            .all(|(i, row)| i == index || row.product != id)
                    },
                );
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut count)
                                .range(1..=maximum)
                                .prefix(format!("{} ", language.text("Quantity"))),
                        )
                        .changed();
                    if ui
                        .add_enabled(
                            assembly.bom.len() > 1,
                            egui::Button::new(language.text("Remove")),
                        )
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
            });
            total = total - row.count + count;
            assembly.bom[index] = BomLine {
                product: product.unwrap(),
                count,
            };
        }
        if let Some(index) = remove {
            total -= assembly.bom.remove(index).count;
            changed = true;
        }
        let available = BOXES
            .iter()
            .map(|(id, _)| *id)
            .chain(products.keys().map(String::as_str))
            .find(|id| assembly.bom.iter().all(|row| row.product != *id));
        if total < MAX_ASSEMBLY_COMPONENTS
            && let Some(product) = available
            && ui.button(language.text("Add component")).clicked()
        {
            assembly.bom.push(BomLine {
                product: product.into(),
                count: 1,
            });
            changed = true;
        }
        ui.separator();
    } else {
        let mut input = Some(machine.input.clone());
        if product_picker(ui, language, "Input", &mut input, products, None) {
            machine.input = input.unwrap();
            changed = true;
        }
    }
    let mut output = Some(machine.output.clone());
    if product_picker(ui, language, "Good output", &mut output, products, None) {
        machine.output = output.unwrap();
        changed = true;
    }
    changed |= product_picker(
        ui,
        language,
        "Scrap model",
        &mut machine.scrap_product,
        products,
        Some("Same as good output"),
    );
    changed |= duration(ui, language, "Cycle", &mut machine.seconds);
    changed |= duration(ui, language, "Repair", &mut machine.repair_seconds);
    for (label, value) in [
        ("Scrap", &mut machine.scrap_percent),
        ("Failure per cycle", &mut machine.failure_percent),
    ] {
        changed |= ui
            .add(
                egui::DragValue::new(value)
                    .range(0.0..=100.0)
                    .prefix(format!("{} ", language.text(label)))
                    .suffix(" %"),
            )
            .changed();
    }
    let previous = machine.door.clone();
    let name = doors
        .iter()
        .find_map(|(id, name)| (Some(id) == machine.door.as_ref()).then_some(name.as_str()))
        .unwrap_or(language.text("None"));
    egui::ComboBox::from_label(language.text("Safety door"))
        .selected_text(name)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut machine.door, None, language.text("None"));
            for (id, name) in doors {
                ui.selectable_value(&mut machine.door, Some(id.clone()), name);
            }
        });
    changed |= previous != machine.door;
    let previous = machine.unload_robot.clone();
    let name = robots
        .iter()
        .find_map(|(id, name)| (Some(id) == machine.unload_robot.as_ref()).then_some(name.as_str()))
        .unwrap_or(language.text(
            if machine.assembly.is_some() && machine.requires_operator() {
                "Operator"
            } else {
                "Built-in gripper"
            },
        ));
    let local_unload = if machine.assembly.is_some() && machine.requires_operator() {
        "Operator"
    } else {
        "Built-in gripper"
    };
    egui::ComboBox::from_label(language.text("Unload with"))
        .selected_text(name)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut machine.unload_robot, None, language.text(local_unload));
            for (id, name) in robots {
                ui.selectable_value(&mut machine.unload_robot, Some(id.clone()), name);
            }
        });
    changed |= previous != machine.unload_robot;
    if machine.unload_robot.is_some() {
        ui.label(
            language
                .text("Set both output points on the conveyor surfaces within the robot's reach."),
        );
    }
    ui.collapsing(language.text("Ports in local coordinates (m)"), |ui| {
        for (label, at) in [
            ("Intake", &mut machine.intake),
            ("Good outlet", &mut machine.outlet),
            ("Reject output", &mut machine.reject),
        ] {
            ui.label(language.text(label));
            changed |= super::dimensions(ui, at, -1000.0..=1000.0);
        }
    });
    changed
}

pub fn staff(ui: &mut Ui, language: Language, staff: &mut Staff, production: &Production) -> bool {
    ui.label(language.text("Area"));
    let mut changed = ui.text_edit_singleline(&mut staff.area).changed();
    let before = staff.calendar.clone();
    egui::ComboBox::from_label(language.text("Calendar"))
        .selected_text(
            staff
                .calendar
                .as_deref()
                .unwrap_or(language.text("Off duty")),
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut staff.calendar, None, language.text("Off duty"));
            for name in production.calendars.keys() {
                ui.selectable_value(&mut staff.calendar, Some(name.clone()), name);
            }
        });
    if production.calendars.is_empty() {
        ui.label(language.text("Create a calendar in Production."));
    }
    changed |= before != staff.calendar;
    changed
}

pub fn robot(ui: &mut Ui, language: Language, robot: &mut Robot, unload: bool) -> bool {
    let mut changed = super::parameter(
        ui,
        language.text("Speed (m/s)"),
        &mut robot.speed,
        0.01..=10.0,
        0.05,
    );
    if unload {
        return changed;
    }
    changed |= ui
        .checkbox(&mut robot.repeat, language.text("Repeat program"))
        .changed();
    ui.collapsing(language.text("Program"), |ui| {
        let mut action = None;
        let len = robot.program.len();
        let reach = robot.reach();
        for (index, step) in robot.program.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.separator();
                let labels = ["Move", "Rotate", "Wait", "Wait signal", "Grab", "Release"]
                    .map(|label| language.text(label));
                let mut op = match step {
                    RobotStep::Move { .. } => 0,
                    RobotStep::Rotate { .. } => 1,
                    RobotStep::Wait { .. } => 2,
                    RobotStep::WaitSignal => 3,
                    RobotStep::Grab => 4,
                    RobotStep::Release => 5,
                };
                let before = op;
                egui::ComboBox::from_id_salt("op")
                    .selected_text(format!("{}  {}", index + 1, labels[op]))
                    .show_ui(ui, |ui| {
                        for (index, label) in labels.into_iter().enumerate() {
                            ui.selectable_value(&mut op, index, label);
                        }
                    });
                if before != op {
                    *step = match op {
                        0 => RobotStep::Move { at: [0.0; 3] },
                        1 => RobotStep::Rotate { degrees: 0.0 },
                        2 => RobotStep::Wait { seconds: 1.0 },
                        3 => RobotStep::WaitSignal,
                        4 => RobotStep::Grab,
                        _ => RobotStep::Release,
                    };
                    changed = true;
                }
                match step {
                    RobotStep::Move { at } => {
                        ui.horizontal(|ui| {
                            for axis in 0..3 {
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut at[axis])
                                            .range(0.0..=reach[axis])
                                            .speed(0.02)
                                            .prefix(["X ", "Y ", "Z "][axis]),
                                    )
                                    .changed();
                            }
                        });
                    }
                    RobotStep::Rotate { degrees } if robot.two_axis => {
                        let mut quadrants = (*degrees / 90.0) as i32;
                        if ui
                            .add(
                                egui::DragValue::new(&mut quadrants)
                                    .range(-4..=4)
                                    .suffix(" × 90°"),
                            )
                            .changed()
                        {
                            *degrees = quadrants as f32 * 90.0;
                            changed = true;
                        }
                    }
                    RobotStep::Rotate { degrees } => {
                        changed |= super::parameter(
                            ui,
                            language.text("Angle (°)"),
                            degrees,
                            -360.0..=360.0,
                            1.0,
                        )
                    }
                    RobotStep::Wait { seconds } => {
                        changed |= duration(ui, language, "Wait", seconds)
                    }
                    _ => {}
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(index > 0, egui::Button::new("↑")).clicked() {
                        action = Some((index, Some(index - 1)));
                    }
                    if ui
                        .add_enabled(index + 1 < len, egui::Button::new("↓"))
                        .clicked()
                    {
                        action = Some((index, Some(index + 1)));
                    }
                    if ui
                        .add_enabled(len > 1, egui::Button::new(language.text("Remove")))
                        .clicked()
                    {
                        action = Some((index, None));
                    }
                });
            });
        }
        if let Some((index, target)) = action {
            if let Some(target) = target {
                robot.program.swap(index, target);
            } else {
                robot.program.remove(index);
            }
            changed = true;
        }
        if len < 256 && ui.button(language.text("Add step")).clicked() {
            robot.program.push(RobotStep::Wait { seconds: 1.0 });
            changed = true;
        }
    });
    changed
}
