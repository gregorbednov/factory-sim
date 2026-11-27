#![cfg(all(target_arch = "wasm32", feature = "web"))]

#[path = "studio/editor_ui.rs"]
mod editor_ui;
#[path = "studio/geometry.rs"]
mod geometry;
#[path = "studio/i18n.rs"]
mod i18n;
#[path = "studio/inspector.rs"]
mod inspector;
#[path = "studio/library.rs"]
mod library;
#[path = "studio/process_ui.rs"]
mod process_ui;
mod product;
mod production;
#[path = "studio/production_ui.rs"]
mod production_ui;
mod project;
#[path = "studio/render.rs"]
mod render;
mod sim;
#[path = "studio/snap.rs"]
mod snap;
#[path = "studio/viewport.rs"]
mod viewport;

use std::{cell::RefCell, rc::Rc, sync::Arc};

use eframe::egui;
use egui::Context;
use wasm_bindgen::prelude::*;

use i18n::Language;
use production::Equipment;
use project::{Direction, Part, PartKind, Pipe, Project, Value, diagram::ProcessDiagram};
use sim::{HZ, Simulation, Snapshot};
use viewport::{Camera, ObjectDrag};

const CELL: &str = include_str!("../examples/production-cell.json");

enum Dialog {
    Production { json: String, error: Option<String> },
    Settings,
}

struct Demo {
    sim: Simulation,
    draft: Option<Project>,
    unsaved: bool,
    dialog: Option<Dialog>,
    scene_open: bool,
    import_requested: bool,
    reset_view: bool,
    library: Option<library::Library>,
    selection: Option<String>,
    inspector_open: bool,
    show_tags: bool,
    tag_error: Option<String>,
    snapshot: Snapshot,
    context: Context,
    last_frame: Option<f64>,
    pending_ticks: f64,
}

impl Demo {
    fn new(project: Project) -> anyhow::Result<Self> {
        let sim = Simulation::new(project)?;
        Ok(Self {
            snapshot: sim.snapshot(),
            sim,
            draft: None,
            unsaved: false,
            dialog: None,
            scene_open: false,
            import_requested: false,
            reset_view: false,
            library: None,
            selection: None,
            inspector_open: true,
            show_tags: false,
            tag_error: None,
            context: Context::default(),
            last_frame: None,
            pending_ticks: 0.0,
        })
    }

    fn project(&self) -> &Project {
        self.draft.as_ref().unwrap_or(&self.sim.project)
    }

    fn apply_draft(&mut self) -> anyhow::Result<()> {
        if let Some(project) = &self.draft {
            self.sim = Simulation::new(project.clone())?;
            self.draft = None;
            self.pending_ticks = 0.0;
        }
        Ok(())
    }

    fn edit(&mut self) -> &mut Project {
        self.unsaved = true;
        self.draft.get_or_insert_with(|| self.sim.project.clone())
    }

    fn update_diagram(&mut self, diagram: ProcessDiagram) {
        let project = self.draft.as_mut().unwrap_or(&mut self.sim.project);
        project.process_diagram = diagram;
        self.unsaved = true;
    }

    fn update_pipes(&mut self, pipes: Vec<Pipe>) {
        if let Some(project) = &mut self.draft {
            project.replace_pipes(pipes);
        } else {
            match self.sim.set_pipes(pipes) {
                Ok(()) => {
                    self.unsaved = true;
                    self.tag_error = None;
                    self.refresh();
                }
                Err(error) => self.tag_error = Some(error.to_string()),
            }
        }
        self.context.request_repaint();
    }

    fn place(&mut self, entry: library::Entry, at: Option<[f32; 2]>) -> String {
        let project = self.edit();
        let (serial, id) = (1..)
            .map(|serial| (serial, format!("part-{serial}")))
            .find(|(_, id)| project.parts.iter().all(|part| part.id != *id))
            .unwrap();
        let y = entry.height(project);
        let [x, z] = at.unwrap_or([
            (serial % 5) as f32 * 2.0 - 4.0,
            -3.0 - (serial / 5) as f32 * 2.0,
        ]);
        // The demo has no live controller, so newly placed powered equipment gets
        // a local start command. An exported project retains the same commands.
        let command = match entry.kind {
            PartKind::Emitter { .. } => Some("enabled"),
            PartKind::Machine { .. } | PartKind::Robot { .. } => Some("start"),
            PartKind::Equipment {
                equipment:
                    Equipment::Roller
                    | Equipment::Scale
                    | Equipment::Turntable
                    | Equipment::ChainTransfer,
                ..
            } => Some("forward"),
            _ => None,
        };
        if let Some(command) = command {
            project
                .initial_commands
                .insert(format!("{id}.{command}"), Value::Bool(true));
        }
        project.parts.push(Part {
            id: id.clone(),
            name: entry.name,
            position: [x, y, z],
            yaw: 0.0,
            product: entry.product,
            kind: entry.kind,
        });
        id
    }

    fn can_remove_selection(&self) -> bool {
        !self.sim.running && self.library.is_none() && self.selection.is_some()
    }

    fn remove_selection(&mut self) {
        if self.can_remove_selection()
            && let Some(id) = self.selection.take()
        {
            self.edit().remove_part(&id);
            self.context.request_repaint();
        }
    }

    fn show_tags(
        &self,
        ui: &mut egui::Ui,
        language: Language,
        id: &str,
        writes: &mut Vec<(String, Value)>,
    ) {
        if self.draft.is_some() {
            ui.label(language.text("Run or step to apply scene changes and read live tags."));
            return;
        }

        let prefix = format!("{id}.");
        let states = || {
            self.snapshot
                .tags
                .range(prefix.clone()..)
                .take_while(|(tag_id, _)| tag_id.starts_with(&prefix))
        };
        if states().next().is_none() {
            ui.label(language.text("No tags for this object"));
            return;
        }
        if let Some(error) = &self.tag_error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("{} · {error}", language.text("Tag command failed")),
            );
        }
        for (direction, title) in [
            (Direction::Actuator, "Commands"),
            (Direction::Sensor, "Sensors"),
        ] {
            let tags = states()
                .map(|(_, state)| state)
                .filter(|state| state.tag.direction == direction);
            let mut tags = tags.peekable();
            if tags.peek().is_none() {
                continue;
            }
            ui.strong(language.text(title));
            for state in tags {
                let tag = &state.tag;
                ui.push_id(&tag.id, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(inspector::tag_label(language, self.project(), tag))
                            .on_hover_text(&tag.id);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if direction == Direction::Actuator {
                                let mut value = state.value;
                                inspector::tag_input(ui, tag, &mut value);
                                if value != state.value {
                                    writes.push((tag.id.clone(), value));
                                }
                            } else {
                                let text = match state.value {
                                    Value::Bool(on) => {
                                        language.text(if on { "On" } else { "Off" }).to_owned()
                                    }
                                    Value::Int(value) => value.to_string(),
                                    Value::Float(value) if tag.id.ends_with("flow_l_s") => {
                                        format!("{value:.6}")
                                    }
                                    Value::Float(value) => format!("{value:.3}"),
                                };
                                ui.monospace(text).on_hover_text(&tag.id);
                            }
                        });
                    });
                });
            }
            ui.add_space(6.0);
        }
    }

    fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        let language = Language::Russian;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.show_tags, false, language.text("Inspector"));
            ui.selectable_value(&mut self.show_tags, true, language.text("Tags"));
        });
        ui.separator();

        let Some(id) = self.selection.as_ref() else {
            ui.label(
                language.text("Select a scene object to edit its properties and watch its tags."),
            );
            return;
        };
        let Some(index) = self.project().parts.iter().position(|part| part.id == *id) else {
            return;
        };
        let mut writes = Vec::new();
        let mut pipes = None;
        let changed = egui::ScrollArea::vertical()
            .id_salt(self.show_tags)
            .max_height(ui.available_height())
            .show(ui, |ui| {
                ui.push_id(id, |ui| {
                    if self.show_tags {
                        ui.label(&self.project().parts[index].name);
                        self.show_tags(ui, language, id, &mut writes);
                        None
                    } else {
                        if self.snapshot.running {
                            ui.label(language.text("Pause to edit properties"));
                        }
                        let part = inspector::properties(
                            ui,
                            language,
                            self.project(),
                            index,
                            !self.snapshot.running,
                        );
                        pipes = inspector::pipes(
                            ui,
                            language,
                            self.project(),
                            id,
                            !self.snapshot.running,
                        );
                        if let Some(error) = &self.tag_error {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                        part
                    }
                })
                .inner
            })
            .inner;
        if !writes.is_empty() {
            self.tag_error = self
                .sim
                .command_batch(writes)
                .err()
                .map(|error| error.to_string());
            self.refresh();
        }
        if let Some(part) = changed {
            self.edit().parts[index] = part;
            ui.ctx().request_repaint();
        }
        if let Some(pipes) = pipes {
            self.update_pipes(pipes);
        }
    }

    fn scene_ui(&mut self, ui: &mut egui::Ui) {
        let language = Language::Russian;
        let running = self.snapshot.running;
        ui.horizontal(|ui| {
            ui.heading(language.text("Scene"));
            if ui
                .add_enabled(!running, egui::Button::new(language.text("+ Add")))
                .clicked()
            {
                self.library = Some(library::Library::new(self.project(), language));
                self.scene_open = false;
            }
        });
        ui.separator();
        let mut selection = self.selection.clone();
        let remove = editor_ui::scene_list(ui, language, self.project(), &mut selection, running);
        if self.scene_open && selection != self.selection {
            self.scene_open = false;
            self.inspector_open = true;
        }
        self.selection = selection;
        if let Some(id) = remove {
            self.edit().remove_part(&id);
            if self.selection.as_deref() == Some(&id) {
                self.selection = None;
            }
        }
    }

    fn show_dialog(
        &mut self,
        ctx: &Context,
        quality: &mut render::GraphicsQuality,
        camera: &mut Camera,
    ) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut open = true;
        let language = Language::Russian;
        match &mut dialog {
            Dialog::Production { json, error } => {
                let changed = egui::Window::new(language.text("Production"))
                    .open(&mut open)
                    .default_width((ctx.content_rect().width() - 30.0).clamp(230.0, 580.0))
                    .show(ctx, |ui| {
                        editor_ui::production(
                            ui,
                            language,
                            self.project(),
                            json,
                            error,
                            !self.snapshot.running,
                        )
                    })
                    .and_then(|window| window.inner.flatten());
                if let Some(settings) = changed {
                    self.edit().production = settings;
                    open = false;
                }
            }
            Dialog::Settings => {
                egui::Window::new(language.text("Settings"))
                    .open(&mut open)
                    .resizable(false)
                    .show(ctx, |ui| {
                        editor_ui::graphics_quality(ui, language, quality);
                        ui.separator();
                        if ui.button(language.text("Reset view")).clicked() {
                            *camera = Camera::default();
                        }
                    });
            }
        }
        if open {
            self.dialog = Some(dialog);
        }
    }

    fn refresh(&mut self) {
        self.snapshot = self.sim.snapshot();
        self.context.request_repaint();
    }

    fn advance(&mut self, now: f64) {
        if !self.sim.running {
            return;
        }
        let previous = self.last_frame.replace(now).unwrap_or(now);
        // Drop time spent in a suspended tab while keeping a fixed physics step.
        self.pending_ticks += (now - previous).clamp(0.0, 0.10) * f64::from(HZ);
        let due = self.pending_ticks.floor() as u32;
        self.pending_ticks -= f64::from(due);
        for _ in 0..due {
            self.sim.step();
        }
        if due > 0 {
            self.snapshot = self.sim.snapshot();
        }
        self.context.request_repaint();
    }
}

#[wasm_bindgen]
pub struct WebApp {
    demo: Rc<RefCell<Demo>>,
    runner: eframe::WebRunner,
}

#[wasm_bindgen]
impl WebApp {
    pub fn set_running(&self, running: bool) -> Result<(), JsValue> {
        let mut demo = self.demo.borrow_mut();
        if running {
            demo.apply_draft().map_err(js_error)?;
            demo.library = None;
        }
        demo.sim.running = running;
        demo.last_frame = None;
        demo.refresh();
        Ok(())
    }

    pub fn step(&self) -> Result<(), JsValue> {
        let mut demo = self.demo.borrow_mut();
        if demo.sim.running {
            return Err(js_error("Поставьте симуляцию на паузу перед шагом"));
        }
        demo.apply_draft().map_err(js_error)?;
        demo.last_frame = None;
        demo.sim.step();
        demo.refresh();
        Ok(())
    }

    pub fn toggle_library(&self) -> Result<(), JsValue> {
        let mut demo = self.demo.borrow_mut();
        if demo.sim.running {
            return Err(js_error(
                "Поставьте симуляцию на паузу перед редактированием сцены",
            ));
        }
        demo.library = if demo.library.is_some() {
            None
        } else {
            demo.dialog = None;
            Some(library::Library::new(demo.project(), Language::Russian))
        };
        demo.context.request_repaint();
        Ok(())
    }

    pub fn toggle_inspector(&self) {
        let mut demo = self.demo.borrow_mut();
        demo.inspector_open = demo.scene_open || !demo.inspector_open;
        demo.scene_open = false;
        demo.context.request_repaint();
    }

    pub fn toggle_scene(&self) {
        let mut demo = self.demo.borrow_mut();
        demo.scene_open = !demo.scene_open;
        demo.context.request_repaint();
    }

    pub fn show_settings(&self) {
        let mut demo = self.demo.borrow_mut();
        demo.dialog = Some(Dialog::Settings);
        demo.library = None;
        demo.context.request_repaint();
    }

    pub fn show_production(&self) -> Result<(), JsValue> {
        let mut demo = self.demo.borrow_mut();
        let json = serde_json::to_string_pretty(&demo.project().production).map_err(js_error)?;
        demo.dialog = Some(Dialog::Production { json, error: None });
        demo.library = None;
        demo.context.request_repaint();
        Ok(())
    }

    pub fn take_import_request(&self) -> bool {
        std::mem::take(&mut self.demo.borrow_mut().import_requested)
    }

    pub fn import_model(
        &self,
        file_name: &str,
        bytes: &[u8],
        scale: f64,
        z_up: bool,
        mass: f32,
    ) -> Result<(), JsValue> {
        if self.demo.borrow().sim.running {
            return Err(js_error(
                "Поставьте симуляцию на паузу перед импортом модели",
            ));
        }
        let up = if z_up {
            product::UpAxis::Z
        } else {
            product::UpAxis::Y
        };
        let model = product::import_bytes(file_name, bytes, scale, up, mass).map_err(js_error)?;
        let mut demo = self.demo.borrow_mut();
        let entry = library::Entry::product(
            (1..)
                .map(|n| format!("product-{n}"))
                .find(|id| !demo.project().products.contains_key(id))
                .unwrap(),
            model.name.clone(),
            model.size,
            model.mass,
        );
        demo.edit()
            .products
            .insert(entry.product.clone().unwrap(), model);
        demo.selection = Some(demo.place(entry, None));
        demo.library = None;
        demo.context.request_repaint();
        Ok(())
    }

    pub fn new_project(&self) -> Result<(), JsValue> {
        let project = Project::empty(Language::Russian.text("Untitled line"));
        self.swap_project(project, true)
    }

    pub fn open_example(&self) -> Result<(), JsValue> {
        self.swap_project(serde_json::from_str(CELL).map_err(js_error)?, false)
    }

    pub fn mark_saved(&self) {
        self.demo.borrow_mut().unsaved = false;
    }

    pub fn remove_selected(&self) {
        self.demo.borrow_mut().remove_selection();
    }

    pub fn reset(&self) -> Result<(), JsValue> {
        let project = self.demo.borrow().project().clone();
        self.swap_simulation(Simulation::new(project).map_err(js_error)?);
        Ok(())
    }

    pub fn replace_project(&self, json: &str) -> Result<(), JsValue> {
        self.swap_project(serde_json::from_str(json).map_err(js_error)?, false)
    }

    pub fn project_json(&self) -> Result<String, JsValue> {
        serde_json::to_string_pretty(self.demo.borrow().project()).map_err(js_error)
    }

    pub fn status(&self) -> String {
        let demo = self.demo.borrow();
        let snap = &demo.snapshot;
        let count = |key: &str| match snap.tags.get(key).map(|tag| tag.value) {
            Some(Value::Int(value)) => Some(value),
            _ => None,
        };
        serde_json::json!({
            "running": snap.running, "tick": snap.tick, "seconds": snap.elapsed_seconds,
            "name": demo.project().name, "edited": demo.draft.is_some(), "unsaved": demo.unsaved,
            "scene_open": demo.scene_open,
            "library_open": demo.library.is_some(), "can_remove": demo.can_remove_selection(),
            "has_selection": demo.selection.is_some(), "inspector_open": demo.inspector_open,
            "good": count("good_exit.count"), "scrap": count("reject_exit.count")
        })
        .to_string()
    }

    pub fn destroy(&self) {
        self.runner.destroy();
    }
}

impl WebApp {
    fn swap_project(&self, project: Project, unsaved: bool) -> Result<(), JsValue> {
        if self.demo.borrow().sim.running {
            return Err(js_error(
                "Поставьте симуляцию на паузу перед сменой проекта",
            ));
        }
        self.swap_simulation(Simulation::new(project).map_err(js_error)?);
        let mut demo = self.demo.borrow_mut();
        demo.reset_view = true;
        demo.unsaved = unsaved;
        Ok(())
    }

    fn swap_simulation(&self, sim: Simulation) {
        let mut demo = self.demo.borrow_mut();
        demo.sim = sim;
        demo.draft = None;
        demo.library = None;
        demo.selection = None;
        demo.dialog = None;
        demo.import_requested = false;
        demo.show_tags = false;
        demo.tag_error = None;
        demo.last_frame = None;
        demo.pending_ticks = 0.0;
        demo.refresh();
    }
}

struct WebStudio {
    demo: Rc<RefCell<Demo>>,
    camera: Camera,
    process: process_ui::View,
    object_drag: Option<ObjectDrag>,
    quality: render::GraphicsQuality,
}

impl eframe::App for WebStudio {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let mut demo = self.demo.borrow_mut();
        demo.advance(ui.input(|input| input.time));
        if std::mem::take(&mut demo.reset_view) {
            self.camera = Camera::default();
            self.process = process_ui::View::default();
            self.object_drag = None;
        }
        if demo.snapshot.running || demo.selection.is_none() {
            self.object_drag = None;
        }
        let mut placed = None;
        let wide = ui.available_width() >= 820.0;
        if wide {
            egui::Panel::left("scene")
                .default_size(205.0)
                .min_size(168.0)
                .show(ui, |ui| demo.scene_ui(ui));
            if demo.inspector_open {
                egui::Panel::right("inspector")
                    .default_size(285.0)
                    .min_size(224.0)
                    .show(ui, |ui| demo.inspector_ui(ui));
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                self.process.tabs(ui, Language::Russian);
                if self.process.open {
                    self.object_drag = None;
                    if let Some(error) = &demo.tag_error {
                        ui.colored_label(editor_ui::WARN, Language::Russian.error(error));
                    }
                    let edit = self.process.show(
                        ui,
                        demo.project(),
                        demo.draft.is_none().then_some(&demo.snapshot),
                        demo.selection.as_deref(),
                        !demo.snapshot.running && demo.dialog.is_none(),
                        Language::Russian,
                    );
                    if let Some(selection) = edit.selection {
                        demo.selection = selection;
                    }
                    if let Some(diagram) = edit.diagram {
                        demo.update_diagram(diagram);
                    }
                    if let Some(pipes) = edit.pipes {
                        demo.update_pipes(pipes);
                    }
                    return;
                }
                let interaction = viewport::draw(
                    ui,
                    Language::Russian,
                    &mut self.camera,
                    self.quality,
                    demo.project(),
                    &demo.snapshot,
                    demo.selection.as_deref(),
                    demo.draft.is_some(),
                    !demo.snapshot.running && demo.dialog.is_none(),
                    &mut self.object_drag,
                );
                if let Some(selection) = interaction.selection {
                    demo.selection = selection;
                }
                if let Some((entry, at)) = interaction.library_drop {
                    placed = Some((entry, Some(at)));
                }
                if let Some((id, movement)) = interaction.movement {
                    let index = demo.project().parts.iter().position(|part| {
                        part.id == id
                            && (part.position != movement.position.to_array()
                                || part.yaw != movement.yaw)
                    });
                    if let Some(index) = index {
                        let part = &mut demo.edit().parts[index];
                        part.position = movement.position.to_array();
                        part.yaw = movement.yaw;
                    }
                }
            });
        if placed.is_none()
            && let Some(mut catalog) = demo.library.take()
        {
            let mut open = true;
            let ctx = ui.ctx();
            let width = (ctx.content_rect().width() - 24.0).clamp(150.0, 610.0);
            let choice = egui::Window::new(Language::Russian.text("Element library"))
                .open(&mut open)
                .default_pos(ctx.content_rect().min + egui::vec2(12.0, 12.0))
                .default_width(width)
                .show(ctx, |ui| {
                    catalog.ui(ui, demo.project(), &demo.snapshot, Language::Russian)
                })
                .and_then(|response| response.inner.flatten());
            match choice {
                Some(library::Choice::Place(entry)) => placed = Some((entry, None)),
                Some(library::Choice::Import) => demo.import_requested = true,
                None if open => demo.library = Some(catalog),
                None => {}
            }
        }
        if let Some((entry, at)) = placed {
            demo.selection = Some(demo.place(entry, at));
            demo.library = None;
            ui.ctx().request_repaint();
        }

        let ctx = ui.ctx();
        if !wide && demo.library.is_none() && demo.dialog.is_none() {
            if demo.scene_open {
                let mut open = true;
                egui::Window::new(Language::Russian.text("Scene"))
                    .open(&mut open)
                    .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 12.0))
                    .max_height((ctx.content_rect().height() - 48.0).max(100.0))
                    .default_width(230.0)
                    .show(ctx, |ui| demo.scene_ui(ui));
                demo.scene_open &= open;
            } else if demo.inspector_open && demo.selection.is_some() {
                let mut open = true;
                let width = (ctx.content_rect().width() - 28.0).clamp(150.0, 338.0);
                egui::Window::new(Language::Russian.text("Inspector"))
                    .open(&mut open)
                    .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
                    .default_width(width)
                    .max_width(width)
                    .max_height((ctx.content_rect().height() - 48.0).max(100.0))
                    .show(ctx, |ui| demo.inspector_ui(ui));
                demo.inspector_open = open;
            }
        }
        demo.show_dialog(ctx, &mut self.quality, &mut self.camera);
        if demo.can_remove_selection()
            && demo.dialog.is_none()
            && !ctx.text_edit_focused()
            && !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|input| {
                input.modifiers.is_none()
                    && input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
            })
        {
            self.object_drag = None;
            demo.remove_selection();
        }
    }
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[wasm_bindgen]
pub async fn start_app(canvas: web_sys::HtmlCanvasElement, webgl: bool) -> Result<WebApp, JsValue> {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};

    let _ = eframe::WebLogger::init(log::LevelFilter::Warn);
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = if webgl {
        wgpu::Backends::GL
    } else {
        wgpu::Backends::BROWSER_WEBGPU
    };
    if webgl {
        setup.device_descriptor = Arc::new(|adapter| wgpu::DeviceDescriptor {
            label: Some("Factory Sim WebGL2 device"),
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d.min(8192),
                ..wgpu::Limits::downlevel_webgl2_defaults()
            },
            ..Default::default()
        });
    }
    let project = serde_json::from_str(CELL).map_err(js_error)?;
    let demo = Rc::new(RefCell::new(Demo::new(project).map_err(js_error)?));
    let state = demo.clone();
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions {
                renderer: eframe::Renderer::Wgpu,
                wgpu_options: WgpuConfiguration {
                    wgpu_setup: WgpuSetup::CreateNew(setup),
                    ..Default::default()
                },
                ..Default::default()
            },
            Box::new(move |cc| {
                let graphics = cc
                    .wgpu_render_state
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Browser graphics unavailable"))?;
                render::install(graphics);
                editor_ui::theme(&cc.egui_ctx);
                state.borrow_mut().context = cc.egui_ctx.clone();
                Ok(Box::new(WebStudio {
                    demo: state,
                    camera: Camera::default(),
                    process: process_ui::View::default(),
                    object_drag: None,
                    quality: if webgl {
                        render::GraphicsQuality::Performance
                    } else {
                        render::GraphicsQuality::Balanced
                    },
                }))
            }),
        )
        .await?;
    Ok(WebApp { demo, runner })
}
