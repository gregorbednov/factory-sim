use std::collections::HashMap;

use eframe::egui::{self, Color32, Pos2, Rect, StrokeKind, TextureHandle, Ui, Vec2};

use super::{i18n::Language, viewport};
use crate::{
    product::{BOXES, ProductSpec},
    production::{AssemblyMode, Equipment, Machine, Robot, RobotStep, Staff, StaffRole},
    project::{
        HeatedTank, Part, PartKind, Project, default_drain_flow, default_open_valve_resistance,
        default_pump_head,
    },
    sim::Snapshot,
};

pub(super) fn presets() -> impl Iterator<Item = PartKind> {
    palette()
        .into_iter()
        .chain(equipment_palette())
        .chain(station_palette())
        .chain([
            PartKind::WaterSource {
                size: [0.5, 0.9, 0.5],
                temperature_c: 15.0,
                max_flow_l_s: 0.03,
                dirt_mg_l: 0.0,
            },
            PartKind::WaterPump {
                size: [0.42, 0.32, 0.32],
                max_flow_l_s: 0.03,
                shutoff_head_m: default_pump_head(),
                dirt_limit_mg: crate::project::default_pump_dirt_limit(),
            },
            PartKind::HeatedTank(HeatedTank::default()),
            PartKind::WaterValve {
                size: [0.3, 0.25, 0.23],
                max_flow_l_s: 0.25,
                open_resistance: default_open_valve_resistance(),
            },
            PartKind::WaterDrain {
                size: [0.42, 0.13, 0.42],
                max_flow_l_s: default_drain_flow(),
            },
            PartKind::WaterJunction {
                size: [0.3, 0.22, 0.33],
                mixing: false,
            },
            PartKind::WaterJunction {
                size: [0.3, 0.22, 0.33],
                mixing: true,
            },
            PartKind::WaterInstrument {
                size: [0.25, 0.25, 0.18],
                measurement: crate::project::WaterMeasurement::Flow,
            },
            PartKind::WaterInstrument {
                size: [0.25, 0.25, 0.18],
                measurement: crate::project::WaterMeasurement::Temperature,
            },
            PartKind::WaterInstrument {
                size: [0.25, 0.25, 0.18],
                measurement: crate::project::WaterMeasurement::Pressure,
            },
            PartKind::WaterFilter {
                size: [0.35, 0.40, 0.28],
                clean_resistance: 3.0,
                dirty_resistance: 32.0,
                capacity_mg: 4000.0,
                efficiency_pct: 98.0,
            },
            PartKind::WaterHeater {
                size: [0.42, 0.30, 0.34],
                max_power_w: 3000.0,
            },
            PartKind::WaterHeatExchanger {
                size: [0.50, 0.42, 0.48],
                conductance_w_k: 1000.0,
                resistance: 8.0,
            },
            PartKind::WaterFiller {
                size: [0.35, 0.40, 0.40],
                max_flow_l_s: 0.15,
                dose_l: 0.45,
            },
        ])
}

pub(super) fn placement_height(kind: &PartKind, spec: Option<ProductSpec<'_>>) -> f32 {
    spec.map_or_else(
        || match *kind {
            PartKind::Equipment {
                equipment: Equipment::Fence | Equipment::SafetyDoor | Equipment::StackLight,
                size,
                ..
            } => size[1] * 0.5,
            PartKind::Staff { .. } | PartKind::Robot { .. } => 0.0,
            PartKind::WaterFiller { size, .. } => size[1] + 0.25,
            _ if kind.water_size().is_some() => kind.water_size().unwrap()[1] * 0.5,
            PartKind::FlowGuide { height, .. } => 0.71 + height * 0.5,
            _ => 0.6,
        },
        |spec| 0.02 + spec.size[1] * 0.5,
    )
}

fn palette() -> [PartKind; 7] {
    [
        PartKind::Conveyor {
            size: [2.0, 0.22, 1.0],
            speed: 1.0,
        },
        PartKind::Pusher {
            stroke: 0.9,
            speed: 1.0,
        },
        PartKind::Sensor { range: 1.2 },
        PartKind::Emitter {
            interval: 2.0,
            emission: None,
            random_spawn_belt: None,
            size: [0.42, 0.38, 0.46],
            mass: 2.0,
        },
        PartKind::Remover {
            size: [0.3, 1.0, 1.2],
        },
        PartKind::Barrier {
            size: [2.0, 0.25, 0.07],
        },
        PartKind::Crate {
            size: [0.42, 0.38, 0.46],
            mass: 2.0,
        },
    ]
}

fn equipment_palette() -> impl Iterator<Item = PartKind> {
    Equipment::ALL
        .into_iter()
        .map(|equipment| PartKind::Equipment {
            size: equipment.size(),
            equipment,
            scale_capacity: 20,
        })
        .chain(std::iter::once(PartKind::FlowGuide {
            length: 2.0,
            height: 0.18,
            inlet: 0.95,
            outlet: 0.64,
        }))
}

fn station_palette() -> [PartKind; 7] {
    let mut two_axis = Robot {
        two_axis: true,
        speed: 2.0,
        ..Robot::default()
    };
    for step in &mut two_axis.program {
        if let RobotStep::Move { at } = step {
            at[0] *= 0.5;
            at[1] *= 0.65;
            at[2] = 0.0;
        }
    }
    [
        PartKind::Machine {
            machine: Machine::default(),
        },
        PartKind::Machine {
            machine: Machine::assembly_table(AssemblyMode::Manual),
        },
        PartKind::Machine {
            machine: Machine::assembly_table(AssemblyMode::Automatic),
        },
        PartKind::Robot {
            robot: Robot::default(),
        },
        PartKind::Robot { robot: two_axis },
        PartKind::Staff {
            staff: Staff {
                role: StaffRole::Operator,
                area: "shop".into(),
                calendar: None,
            },
        },
        PartKind::Staff {
            staff: Staff {
                role: StaffRole::Technician,
                area: "shop".into(),
                calendar: None,
            },
        },
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Transport,
    Sensors,
    Flow,
    Guards,
    Stations,
    Water,
    Products,
}

impl Category {
    const ALL: [Self; 7] = [
        Self::Transport,
        Self::Sensors,
        Self::Flow,
        Self::Guards,
        Self::Stations,
        Self::Water,
        Self::Products,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Transport => "Transport",
            Self::Sensors => "Sensors",
            Self::Flow => "Infeed and outfeed",
            Self::Guards => "Guards",
            Self::Stations => "Stations and staff",
            Self::Water => "Water circuit",
            Self::Products => "Products",
        }
    }

    fn for_part(kind: &PartKind) -> Self {
        match kind {
            PartKind::Conveyor { .. } => Self::Transport,
            PartKind::Equipment { equipment, .. } => match equipment {
                Equipment::Roller
                | Equipment::FreeRoller
                | Equipment::Chute
                | Equipment::Scale
                | Equipment::Turntable
                | Equipment::ChainTransfer => Self::Transport,
                Equipment::Capacitive | Equipment::LightArray => Self::Sensors,
                Equipment::StopBlade => Self::Flow,
                Equipment::Fence | Equipment::SafetyDoor => Self::Guards,
                Equipment::StackLight => Self::Stations,
            },
            PartKind::Pusher { .. }
            | PartKind::Emitter { .. }
            | PartKind::Remover { .. }
            | PartKind::FlowGuide { .. } => Self::Flow,
            PartKind::Sensor { .. } => Self::Sensors,
            PartKind::Barrier { .. } => Self::Guards,
            PartKind::Machine { .. } | PartKind::Robot { .. } | PartKind::Staff { .. } => {
                Self::Stations
            }
            PartKind::HeatedTank(_)
            | PartKind::WaterSource { .. }
            | PartKind::WaterPump { .. }
            | PartKind::WaterValve { .. }
            | PartKind::WaterDrain { .. }
            | PartKind::WaterJunction { .. }
            | PartKind::WaterInstrument { .. }
            | PartKind::WaterFilter { .. }
            | PartKind::WaterHeater { .. }
            | PartKind::WaterHeatExchanger { .. }
            | PartKind::WaterFiller { .. } => Self::Water,
            PartKind::Crate { .. } => Self::Products,
        }
    }
}

#[derive(Clone)]
pub(super) struct Entry {
    pub name: String,
    pub kind: PartKind,
    pub product: Option<String>,
    category: Category,
}

impl Entry {
    fn equipment(language: Language, kind: PartKind, label: &'static str) -> Self {
        let category = Category::for_part(&kind);
        Self {
            name: language.text(label).into(),
            kind,
            product: None,
            category,
        }
    }

    pub fn product(id: String, name: String, size: [f32; 3], mass: f32) -> Self {
        Self {
            name,
            kind: PartKind::Crate { size, mass },
            product: Some(id),
            category: Category::Products,
        }
    }

    pub fn height(&self, project: &Project) -> f32 {
        placement_height(
            &self.kind,
            self.product.as_deref().and_then(|id| project.product(id)),
        )
    }

    fn preview(&self, project: &Project, snapshot: &Snapshot) -> egui::ColorImage {
        let part = Part {
            id: String::new(),
            name: self.name.clone(),
            position: [0.0, self.height(project), 0.0],
            yaw: 0.0,
            product: self.product.clone(),
            kind: self.kind.clone(),
        };
        viewport::part_thumbnail(project, &part, snapshot)
    }
}

pub(super) enum Choice {
    Place(Entry),
    Import,
}

pub(super) struct Library {
    entries: Vec<Entry>,
    textures: HashMap<usize, TextureHandle>,
    filter: Option<Category>,
    query: String,
}

impl Library {
    pub fn new(project: &Project, language: Language) -> Self {
        let mut entries: Vec<_> = presets()
            .map(|kind| {
                let label = kind.label();
                Entry::equipment(language, kind, label)
            })
            .collect();
        let mut add = |name, kind| entries.push(Entry::equipment(language, kind, name));

        for (name, size) in [
            ("Belt conveyor 1 m", [1.0, 0.22, 1.0]),
            ("Belt conveyor 4 m", [4.0, 0.22, 1.0]),
            ("Belt conveyor 6 m", [6.0, 0.22, 1.0]),
            ("Narrow belt conveyor", [2.0, 0.22, 0.45]),
        ] {
            add(name, PartKind::Conveyor { size, speed: 1.0 });
        }
        for (name, equipment, size, scale_capacity) in [
            (
                "Roller conveyor 4 m",
                Equipment::Roller,
                [4.0, 0.22, 1.1],
                20,
            ),
            (
                "Roller conveyor 6 m",
                Equipment::Roller,
                [6.0, 0.22, 1.1],
                20,
            ),
            (
                "Conveyor scale 100 kg",
                Equipment::Scale,
                [1.3, 0.22, 1.25],
                100,
            ),
            ("Long fence panel", Equipment::Fence, [2.4, 1.8, 0.08], 20),
            (
                "Wide stop blade",
                Equipment::StopBlade,
                [0.08, 0.12, 1.1],
                20,
            ),
        ] {
            add(
                name,
                PartKind::Equipment {
                    equipment,
                    size,
                    scale_capacity,
                },
            );
        }
        add(
            "Straight guide",
            PartKind::FlowGuide {
                length: 2.0,
                height: 0.18,
                inlet: 0.66,
                outlet: 0.66,
            },
        );

        entries.extend(
            BOXES
                .iter()
                .map(|(id, spec)| {
                    (
                        (*id).to_owned(),
                        language.text(spec.name).to_owned(),
                        spec.size,
                        spec.mass,
                    )
                })
                .chain(
                    project
                        .products
                        .iter()
                        .map(|(id, spec)| (id.clone(), spec.name.clone(), spec.size, spec.mass)),
                )
                .map(|(id, name, size, mass)| Entry::product(id, name, size, mass)),
        );
        Self {
            entries,
            textures: HashMap::new(),
            filter: None,
            query: String::new(),
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        snapshot: &Snapshot,
        language: Language,
    ) -> Option<Choice> {
        let choice = ui
            .horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text(language.text("Search elements")),
                );
                if ui.button(language.text("Import 3D model…")).clicked() {
                    return Some(Choice::Import);
                }
                None
            })
            .inner;
        if choice.is_some() {
            return choice;
        }
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.filter, None, language.text("All elements"));
            for category in Category::ALL {
                ui.selectable_value(
                    &mut self.filter,
                    Some(category),
                    language.text(category.label()),
                );
            }
        });
        ui.label(
            egui::RichText::new(language.text("Click to add or drag to the scene"))
                .color(ui.visuals().weak_text_color()),
        );
        let query = self.query.trim().to_lowercase();
        let visible: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                self.filter
                    .is_none_or(|category| entry.category == category)
                    && (entry.name.to_lowercase().contains(&query)
                        || entry
                            .product
                            .as_deref()
                            .is_some_and(|id| id.contains(&query))
                        || entry.kind.label().to_lowercase().contains(&query))
            })
            .map(|(index, _)| index)
            .collect();
        if visible.is_empty() {
            ui.label(language.text("No matching elements"));
            return None;
        }

        egui::ScrollArea::vertical()
            .id_salt(self.filter.map(|c| c as u8))
            .max_height((ui.ctx().content_rect().height() * 0.60).max(170.0))
            .show(ui, |ui| {
                let columns = ((ui.available_width() + 8.0) / 150.0).floor().max(1.0) as usize;
                let mut chosen = None;
                for row in visible.chunks(columns) {
                    ui.horizontal(|ui| {
                        for &index in row {
                            let entry = &self.entries[index];
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(142.0, 116.0),
                                egui::Sense::click_and_drag(),
                            );
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            let painter = ui.painter_at(rect);
                            let style = ui.style().interact(&response);
                            painter.rect(
                                rect,
                                3.0,
                                style.weak_bg_fill,
                                style.bg_stroke,
                                StrokeKind::Inside,
                            );
                            let texture = self.textures.entry(index).or_insert_with(|| {
                                ui.ctx().load_texture(
                                    format!("library-preview-{index}"),
                                    entry.preview(project, snapshot),
                                    egui::TextureOptions::LINEAR,
                                )
                            });
                            let image = Rect::from_min_size(
                                rect.min + Vec2::splat(5.0),
                                Vec2::new(132.0, 78.0),
                            );
                            painter.image(
                                texture.id(),
                                image,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                            let galley = painter.layout(
                                entry.name.clone(),
                                egui::TextStyle::Body.resolve(ui.style()),
                                ui.visuals().text_color(),
                                132.0,
                            );
                            painter.galley(
                                Pos2::new(
                                    rect.center().x - galley.size().x * 0.5,
                                    image.bottom() + 3.0,
                                ),
                                galley,
                                ui.visuals().text_color(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::Role::Button, true, &entry.name)
                            });
                            if response.drag_started() {
                                response.dnd_set_drag_payload(entry.clone());
                            }
                            if response.clicked() {
                                chosen = Some(Choice::Place(entry.clone()));
                            }
                            response.on_hover_text(&entry.name);
                        }
                    });
                }
                chosen
            })
            .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Simulation;

    #[test]
    fn catalog_presets_have_renderable_models_and_valid_projects() -> anyhow::Result<()> {
        let mut project = Project::demo();
        let snapshot = Simulation::new(project.clone())?.snapshot();
        let catalog = Library::new(&project, Language::Russian);
        project.parts.clear();
        project.drivers.clear();
        project.initial_commands.clear();
        project.tag_names.clear();
        for (index, entry) in catalog.entries.iter().enumerate() {
            let image = entry.preview(&project, &snapshot);
            let background = image.pixels[0];
            assert!(
                image
                    .pixels
                    .iter()
                    .filter(|&&pixel| pixel != background)
                    .count()
                    > 30,
                "Empty preview for {}",
                entry.name
            );
            project.parts.push(Part {
                id: format!("library-{index}"),
                name: entry.name.clone(),
                position: [index as f32 * 8.0, entry.height(&project), 0.0],
                yaw: 0.0,
                product: entry.product.clone(),
                kind: entry.kind.clone(),
            });
        }
        project.validate()?;
        Ok(())
    }
}
