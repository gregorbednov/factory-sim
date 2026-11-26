#![cfg(all(target_arch = "wasm32", feature = "web"))]

#[path = "studio/geometry.rs"]
mod geometry;
#[path = "studio/i18n.rs"]
mod i18n;
#[path = "studio/library.rs"]
mod library;
mod product;
mod production;
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
use project::{Part, PartKind, Project, Value};
use sim::{HZ, Simulation, Snapshot};
use viewport::{Camera, ObjectDrag};

const CELL: &str = include_str!("../examples/production-cell.json");

struct Demo {
    sim: Simulation,
    draft: Option<Project>,
    library: Option<library::Library>,
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
            library: None,
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
        self.draft.get_or_insert_with(|| self.sim.project.clone())
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
pub struct WebDemo {
    demo: Rc<RefCell<Demo>>,
    runner: eframe::WebRunner,
}

#[wasm_bindgen]
impl WebDemo {
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
            Some(library::Library::new(demo.project(), Language::Russian))
        };
        demo.context.request_repaint();
        Ok(())
    }

    pub fn reset(&self) -> Result<(), JsValue> {
        let project = self.demo.borrow().project().clone();
        self.swap_simulation(Simulation::new(project).map_err(js_error)?);
        Ok(())
    }

    pub fn replace_project(&self, json: &str) -> Result<(), JsValue> {
        let mut project: Project = serde_json::from_str(json).map_err(js_error)?;
        // Retain mappings for round-tripping JSON. No network runtime is linked.
        for driver in &mut project.drivers {
            driver.enabled = false;
        }
        self.swap_simulation(Simulation::new(project).map_err(js_error)?);
        Ok(())
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
            "name": demo.project().name, "edited": demo.draft.is_some(),
            "library_open": demo.library.is_some(),
            "good": count("good_exit.count"), "scrap": count("reject_exit.count")
        })
        .to_string()
    }

    pub fn destroy(&self) {
        self.runner.destroy();
    }
}

impl WebDemo {
    fn swap_simulation(&self, sim: Simulation) {
        let mut demo = self.demo.borrow_mut();
        demo.sim = sim;
        demo.draft = None;
        demo.library = None;
        demo.last_frame = None;
        demo.pending_ticks = 0.0;
        demo.refresh();
    }
}

struct WebStudio {
    demo: Rc<RefCell<Demo>>,
    camera: Camera,
    object_drag: Option<ObjectDrag>,
    selection: Option<String>,
    quality: render::GraphicsQuality,
}

impl eframe::App for WebStudio {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let mut demo = self.demo.borrow_mut();
        demo.advance(ui.input(|input| input.time));
        if demo.snapshot.running {
            self.object_drag = None;
        }
        let mut placed = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let interaction = viewport::draw(
                    ui,
                    Language::Russian,
                    &mut self.camera,
                    self.quality,
                    demo.project(),
                    &demo.snapshot,
                    self.selection.as_deref(),
                    demo.draft.is_some(),
                    !demo.snapshot.running,
                    &mut self.object_drag,
                );
                if let Some(selection) = interaction.selection {
                    self.selection = selection;
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
            if let Some(library::Choice::Place(entry)) = choice {
                placed = Some((entry, None));
            } else if open {
                demo.library = Some(catalog);
            }
        }
        if let Some((entry, at)) = placed {
            self.selection = Some(demo.place(entry, at));
            demo.library = None;
            ui.ctx().request_repaint();
        }
    }
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[wasm_bindgen]
pub async fn start_demo(
    canvas: web_sys::HtmlCanvasElement,
    webgl: bool,
) -> Result<WebDemo, JsValue> {
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
                cc.egui_ctx.set_visuals(egui::Visuals::dark());
                state.borrow_mut().context = cc.egui_ctx.clone();
                Ok(Box::new(WebStudio {
                    demo: state,
                    camera: Camera::default(),
                    object_drag: None,
                    selection: None,
                    quality: if webgl {
                        render::GraphicsQuality::Performance
                    } else {
                        render::GraphicsQuality::Balanced
                    },
                }))
            }),
        )
        .await?;
    Ok(WebDemo { demo, runner })
}
