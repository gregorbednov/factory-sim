#![cfg(all(target_arch = "wasm32", feature = "web"))]

#[path = "studio/geometry.rs"]
mod geometry;
#[path = "studio/i18n.rs"]
mod i18n;
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

use project::{Project, Value};
use sim::{HZ, Simulation, Snapshot};
use viewport::{Camera, ObjectDrag};

const CELL: &str = include_str!("../examples/production-cell.json");

struct Demo {
    sim: Simulation,
    draft: Option<Project>,
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
            context: Context::default(),
            last_frame: None,
            pending_ticks: 0.0,
        })
    }

    fn project(&self) -> &Project {
        self.draft.as_ref().unwrap_or(&self.sim.project)
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
        if running && let Some(project) = demo.draft.as_ref() {
            let sim = Simulation::new(project.clone()).map_err(js_error)?;
            demo.sim = sim;
            demo.draft = None;
            demo.pending_ticks = 0.0;
        }
        demo.sim.running = running;
        demo.last_frame = None;
        demo.refresh();
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
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let interaction = viewport::draw(
                    ui,
                    i18n::Language::Russian,
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
                if let Some((id, movement)) = interaction.movement {
                    let draft = demo.project();
                    if draft.parts.iter().any(|part| {
                        part.id == id
                            && (part.position != movement.position.to_array()
                                || part.yaw != movement.yaw)
                    }) {
                        if demo.draft.is_none() {
                            demo.draft = Some(demo.sim.project.clone());
                        }
                        if let Some(part) = demo
                            .draft
                            .as_mut()
                            .unwrap()
                            .parts
                            .iter_mut()
                            .find(|part| part.id == id)
                        {
                            part.position = movement.position.to_array();
                            part.yaw = movement.yaw;
                        }
                    }
                }
            });
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
