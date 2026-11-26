mod editor_ui;
mod geometry;
mod i18n;
mod inspector;
mod library;
mod production_ui;
mod render;
mod snap;
mod viewport;

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use chrono::Datelike;
use clap::ValueEnum;
use editor_ui::{ACCENT, MUTE, TEXT, WARN, theme};
use eframe::{
    egui::{self, Color32, RichText, Ui},
    egui_wgpu::{WgpuSetup, WgpuSetupCreateNew},
};
use i18n::Language;
use inspector::parameter;
use library::placement_height;
use serde::{Deserialize, Serialize};

use crate::{
    product::{self, Product, UpAxis},
    project::{Direction, Driver, Part, PartKind, Project, Tag},
    runtime::{Action, Session},
    sim::{HZ, Quality, Snapshot},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, ValueEnum, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    #[default]
    Vulkan,
    #[cfg(not(target_os = "macos"))]
    #[value(name = "opengl")]
    OpenGl,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Self::Vulkan => "Vulkan",
            #[cfg(not(target_os = "macos"))]
            Self::OpenGl => "OpenGL",
        }
    }

    fn flags(self) -> wgpu::Backends {
        match self {
            Self::Vulkan => wgpu::Backends::VULKAN,
            #[cfg(not(target_os = "macos"))]
            Self::OpenGl => wgpu::Backends::GL,
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
struct Settings {
    backend: Backend,
    #[serde(default)]
    language: Language,
    #[serde(default)]
    graphics_quality: render::GraphicsQuality,
}

fn settings_path() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "windows") {
        env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    dir.map(|dir| dir.join("factory-sim").join("studio.json"))
}

pub fn open(
    path: Option<PathBuf>,
    backend: Option<Backend>,
    fps_limit: Option<u32>,
    run: bool,
) -> Result<()> {
    let path_for_settings = settings_path();
    let settings = path_for_settings
        .as_ref()
        .filter(|path| path.exists())
        .map(|path| -> Result<Settings> { Ok(serde_json::from_slice(&fs::read(path)?)?) })
        .transpose()?
        .unwrap_or_default();
    let backend = backend.unwrap_or(settings.backend);
    let project = if let Some(path) = &path {
        Project::load(path)?
    } else {
        Project::demo()
    };
    let session = Session::new(project.clone(), true)?;
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = backend.flags();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320.0, 820.0])
            .with_min_inner_size([880.0, 560.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: WgpuSetup::CreateNew(setup),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "Factory Sim",
        options,
        Box::new(move |cc| {
            theme(&cc.egui_ctx);
            let gpu = cc
                .wgpu_render_state
                .as_ref()
                .map(|state| {
                    render::install(state);
                    let adapter = state.adapter.get_info();
                    log::info!("Studio graphics: {:?} · {}", adapter.backend, adapter.name);
                    adapter.name
                })
                .unwrap_or_else(|| "GPU".to_owned());
            if run {
                session.runtime.handle.execute(Action::Run)?;
            }
            Ok(Box::new(Studio {
                project,
                session,
                backend,
                settings,
                path_for_settings,
                path,
                gpu,
                selection: None,
                camera: viewport::Camera::default(),
                object_drag: None,
                dirty: false,
                unsaved: false,
                notice: None,
                dialog: None,
                serial: 0,
                show_tags: false,
                shown: false,
                fps_limit,
                rates: Rates::default(),
            }))
        }),
    )
    .map_err(|e| anyhow!("starting {} editor: {e}", backend.label()))
}

enum Dialog {
    New,
    Library(library::Library),
    File {
        action: FileAction,
        path: String,
        error: Option<String>,
    },
    ConfirmOpen(PathBuf),
    Drivers {
        json: String,
        error: Option<String>,
    },
    Production {
        json: String,
        error: Option<String>,
    },
    Import(ImportProduct),
    Settings,
}

enum FileAction {
    Open,
    Save { then_open: Option<PathBuf> },
}

const PROJECT_COMMANDS: [&str; 6] = ["New", "Open", "Save", "I/O", "Production", "Settings"];

struct ImportProduct {
    path: String,
    units: Option<Units>,
    up: Option<UpAxis>,
    mass: f32,
    pending: Option<Receiver<Result<Product>>>,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Units {
    Millimeters,
    Centimeters,
    Meters,
    Inches,
}

impl Units {
    fn label(self, language: Language) -> &'static str {
        language.text(match self {
            Self::Millimeters => "Millimeters",
            Self::Centimeters => "Centimeters",
            Self::Meters => "Meters",
            Self::Inches => "Inches",
        })
    }

    fn scale(self) -> f64 {
        match self {
            Self::Millimeters => 0.001,
            Self::Centimeters => 0.01,
            Self::Meters => 1.0,
            Self::Inches => 0.0254,
        }
    }
}

impl Default for ImportProduct {
    fn default() -> Self {
        Self {
            path: String::new(),
            units: None,
            up: None,
            mass: 2.0,
            pending: None,
            error: None,
        }
    }
}

impl ImportProduct {
    fn show(&mut self, ui: &mut Ui, paused: bool, language: Language) -> Option<Product> {
        if let Some(pending) = &self.pending
            && paused
        {
            match pending.try_recv() {
                Ok(Ok(product)) => return Some(product),
                Ok(Err(error)) => {
                    self.pending = None;
                    self.error = Some(format!("{error:#}"));
                }
                Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.error = Some("The model importer stopped unexpectedly".into());
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        ui.add_enabled_ui(self.pending.is_none(), |ui| {
            if let Some(path) = ui.input(|input| {
                input
                    .raw
                    .dropped_files
                    .first()
                    .map(|file| file.path().to_path_buf())
            }) {
                self.path = path.to_string_lossy().into_owned();
            }
            ui.label(language.text("STEP (.step, .stp), OBJ or STL"));
            ui.add(
                egui::TextEdit::singleline(&mut self.path)
                    .hint_text(language.text("Model file path"))
                    .desired_width(f32::INFINITY),
            );
            let path = PathBuf::from(self.path.trim());
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let obj = extension == "obj";
            let default_units = if obj {
                Units::Meters
            } else {
                Units::Millimeters
            };
            let default_up = if obj { UpAxis::Y } else { UpAxis::Z };
            let mut units = self.units.unwrap_or(default_units);
            let mut up = self.up.unwrap_or(default_up);
            ui.horizontal(|ui| {
                ui.label(language.text("File units"));
                egui::ComboBox::from_id_salt("import-units")
                    .selected_text(units.label(language))
                    .show_ui(ui, |ui| {
                        for option in [
                            Units::Millimeters,
                            Units::Centimeters,
                            Units::Meters,
                            Units::Inches,
                        ] {
                            if ui
                                .selectable_value(&mut units, option, option.label(language))
                                .clicked()
                            {
                                self.units = Some(option);
                            }
                        }
                    });
                ui.label(language.text("Up axis"));
                if ui.selectable_value(&mut up, UpAxis::Y, "Y").clicked() {
                    self.up = Some(UpAxis::Y);
                }
                if ui.selectable_value(&mut up, UpAxis::Z, "Z").clicked() {
                    self.up = Some(UpAxis::Z);
                }
            });
            parameter(
                ui,
                language.text("Mass (kg)"),
                &mut self.mass,
                0.001..=1000.0,
                0.05,
            );
            if ui
                .add_enabled(
                    paused && !self.path.trim().is_empty(),
                    egui::Button::new(language.text("Import and place")),
                )
                .clicked()
            {
                let (send, receive) = mpsc::channel();
                let context = ui.ctx().clone();
                let mass = self.mass;
                let job = std::thread::Builder::new()
                    .name("product-import".into())
                    .spawn(move || {
                        let _ = send.send(product::import(&path, units.scale(), up, mass));
                        context.request_repaint();
                    });
                self.error = job.as_ref().err().map(ToString::to_string);
                if job.is_ok() {
                    self.pending = Some(receive);
                }
            }
        });
        if self.pending.is_some() && paused {
            ui.spinner();
        }
        if !paused {
            ui.label(language.text("Pause the simulation to add the product."));
        }
        if let Some(error) = &self.error {
            ui.colored_label(WARN, language.error(error));
        }
        None
    }
}

struct Studio {
    project: Project,
    session: Session,
    backend: Backend,
    settings: Settings,
    path_for_settings: Option<PathBuf>,
    path: Option<PathBuf>,
    gpu: String,
    selection: Option<String>,
    camera: viewport::Camera,
    object_drag: Option<viewport::ObjectDrag>,
    dirty: bool,
    unsaved: bool,
    notice: Option<(String, bool)>,
    dialog: Option<Dialog>,
    serial: u64,
    show_tags: bool,
    shown: bool,
    fps_limit: Option<u32>,
    rates: Rates,
}

#[derive(Default)]
struct Rates {
    start: Option<(Instant, u64)>,
    frames: u32,
    measured: Option<(f64, f64)>,
}

impl Rates {
    fn sample(&mut self, snapshot: &Snapshot, now: Instant) {
        if !snapshot.running {
            *self = Self::default();
            return;
        }
        let (start, tick) = *self.start.get_or_insert((now, snapshot.tick));
        if snapshot.tick < tick {
            *self = Self::default();
            return;
        }
        self.frames += 1;
        let elapsed = now.duration_since(start).as_secs_f64();
        if elapsed >= 1.0 {
            let fps = f64::from(self.frames.saturating_sub(1)) / elapsed;
            let hz = (snapshot.tick - tick) as f64 / elapsed;
            self.measured = Some((fps, hz));
            log::debug!("viewport {:.1} FPS; simulation {:.1} Hz", fps, hz);
            self.start = Some((now, snapshot.tick));
            self.frames = 1;
        }
    }
}

impl Studio {
    fn set_error(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.notice = Some((e.to_string(), true));
        }
    }

    fn edit(&mut self) {
        self.dirty = true;
        self.unsaved = true;
    }

    fn apply_edits(&mut self) -> Result<()> {
        if self.dirty {
            let session = Session::new(self.project.clone(), true)?;
            self.session = session;
            self.dirty = false;
        }
        Ok(())
    }

    fn action(&mut self, action: Action) {
        let result = self
            .apply_edits()
            .and_then(|()| self.session.runtime.handle.execute(action));
        self.set_error(result);
    }

    fn add_part(
        &mut self,
        kind: PartKind,
        product: Option<&str>,
        custom_name: Option<String>,
        at: Option<[f32; 2]>,
    ) {
        let id = loop {
            self.serial += 1;
            let id = format!("part-{}", self.serial);
            if !self.project.parts.iter().any(|part| part.id == id) {
                break id;
            }
        };
        let spec = product.and_then(|id| self.project.product(id));
        let name = custom_name.unwrap_or_else(|| {
            spec.map_or_else(
                || self.settings.language.text(kind.label()),
                |spec| {
                    if product.is_some_and(|id| self.project.products.contains_key(id)) {
                        spec.name
                    } else {
                        self.settings.language.text(spec.name)
                    }
                },
            )
            .to_owned()
        });
        let y = placement_height(&kind, spec);
        let [x, z] = at.unwrap_or([(self.serial % 5) as f32 - 2.0, -2.2]);
        self.project.parts.push(Part {
            id: id.clone(),
            name,
            position: [x, y, z],
            product: product.map(str::to_owned),
            yaw: 0.0,
            kind,
        });
        self.selection = Some(id);
        self.show_tags = false;
        self.edit();
    }

    fn add_product(&mut self, id: &str) {
        if let Some(spec) = self.project.product(id) {
            self.add_part(
                PartKind::Crate {
                    size: spec.size,
                    mass: spec.mass,
                },
                Some(id),
                None,
                None,
            );
        }
    }

    fn remove_part(&mut self, id: &str) {
        if self.session.runtime.handle.snapshot().running || !self.project.remove_part(id) {
            return;
        }
        if self.selection.as_deref() == Some(id) {
            self.selection = None;
        }
        self.edit();
    }

    fn top(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        let language = self.settings.language;
        ui.add_space(7.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Factory Sim")
                    .size(22.0)
                    .strong()
                    .color(ACCENT),
            );
            ui.separator();
            let mut title_rect = ui.available_rect_before_wrap();
            title_rect.set_width(title_rect.width().min(180.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(title_rect), |ui| {
                ui.add(egui::Label::new(RichText::new(&self.project.name).size(15.0)).truncate());
            });
            if self.unsaved {
                let (_, indicator) =
                    ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(indicator.rect.center(), 3.0, WARN);
                indicator.on_hover_text(language.text("Unsaved project"));
            }
            ui.separator();
            let (playback, command, icon) = if snapshot.running {
                ("Pause", Action::Pause, PlaybackIcon::Pause)
            } else {
                ("Run", Action::Run, PlaybackIcon::Play)
            };
            let border = ui.visuals().widgets.noninteractive.bg_stroke;
            egui::Frame::new()
                .stroke(border)
                .fill(border.color)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    ui.horizontal(|ui| {
                        for (label, icon, action, enabled) in [
                            (playback, icon, command, true),
                            (
                                "Step",
                                PlaybackIcon::Step,
                                Action::Step(1),
                                !snapshot.running,
                            ),
                            ("Reset", PlaybackIcon::Reset, Action::Reset, true),
                        ] {
                            if playback_button(ui, language.text(label), icon, enabled).clicked() {
                                self.action(action);
                            }
                        }
                    });
                });
            let spacing = ui.spacing();
            let button_font = egui::TextStyle::Button.resolve(ui.style());
            let command_width = PROJECT_COMMANDS
                .iter()
                .map(|label| {
                    let text_width = ui
                        .painter()
                        .layout_no_wrap(language.text(label).to_owned(), button_font.clone(), TEXT)
                        .size()
                        .x;
                    (text_width + 2.0 * spacing.button_padding.x)
                        .max(spacing.interact_size.x)
                        .ceil()
                        + spacing.item_spacing.x
                })
                .sum::<f32>();
            let compact = ui.available_width() < command_width;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if compact {
                    ui.menu_button(language.text("Menu"), |ui| {
                        for command in PROJECT_COMMANDS {
                            if ui.button(language.text(command)).clicked() {
                                self.project_command(command);
                                ui.close();
                            }
                        }
                    });
                } else {
                    for command in PROJECT_COMMANDS.into_iter().rev() {
                        if ui.button(language.text(command)).clicked() {
                            self.project_command(command);
                        }
                    }
                }
            });
        });
    }

    fn project_command(&mut self, command: &str) {
        self.dialog = Some(match command {
            "New" => Dialog::New,
            "Open" => Dialog::File {
                action: FileAction::Open,
                path: self.project_path_text(""),
                error: None,
            },
            "Save" => self.save_dialog(None),
            "I/O" => Dialog::Drivers {
                json: serde_json::to_string_pretty(&self.project.drivers).unwrap(),
                error: None,
            },
            "Production" => Dialog::Production {
                json: serde_json::to_string_pretty(&self.project.production).unwrap(),
                error: None,
            },
            "Settings" => Dialog::Settings,
            _ => unreachable!("unknown toolbar command"),
        });
    }

    fn project_path_text(&self, fallback: &str) -> String {
        self.path
            .as_ref()
            .map_or_else(|| fallback.to_owned(), |path| path.display().to_string())
    }

    fn save_dialog(&self, then_open: Option<PathBuf>) -> Dialog {
        Dialog::File {
            action: FileAction::Save { then_open },
            path: self.project_path_text("line.json"),
            error: None,
        }
    }

    fn save_project(&mut self, path: &Path) -> Result<()> {
        self.project.save(path)?;
        self.path = Some(path.to_owned());
        self.unsaved = false;
        Ok(())
    }

    fn open_project(&mut self, path: &Path) -> Result<()> {
        let project = Project::load(path)?;
        let session = Session::new(project.clone(), true)?;
        self.project = project;
        self.session = session;
        self.path = Some(path.to_owned());
        self.selection = None;
        self.dirty = false;
        self.unsaved = false;
        self.camera = viewport::Camera::default();
        self.object_drag = None;
        Ok(())
    }

    fn resume_open(&mut self, path: PathBuf) {
        if let Err(error) = self.open_project(&path) {
            self.dialog = Some(Dialog::File {
                action: FileAction::Open,
                path: path.display().to_string(),
                error: Some(error.to_string()),
            });
        }
    }

    fn left(&mut self, ui: &mut Ui, running: bool) {
        let language = self.settings.language;
        ui.horizontal(|ui| {
            ui.label(RichText::new(language.text("Scene")).size(19.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!running, |ui| {
                    if ui.button(language.text("+ Add")).clicked() {
                        self.dialog = Some(Dialog::Library(library::Library::new(
                            &self.project,
                            language,
                        )));
                    }
                });
            });
        });
        ui.separator();
        let mut selection = self.selection.clone();
        let remove = editor_ui::scene_list(ui, language, &self.project, &mut selection, running);
        if selection != self.selection {
            self.selection = selection;
            self.show_tags = false;
        }
        if let Some(id) = remove {
            self.remove_part(&id);
        }
    }

    fn right(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        let language = self.settings.language;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.show_tags, false, language.text("Inspector"));
            ui.selectable_value(&mut self.show_tags, true, language.text("All tags"));
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.show_tags {
                if self.dirty {
                    for tag in self.project.tags().values() {
                        self.tag_row(ui, tag, snapshot);
                    }
                } else {
                    for state in snapshot.tags.values() {
                        self.tag_row(ui, &state.tag, snapshot);
                    }
                }
                return;
            }
            let Some(id) = self.selection.clone() else {
                ui.label(
                    RichText::new(
                        language.text(
                            "Select a scene object to edit its properties and watch its tags.",
                        ),
                    )
                    .color(MUTE),
                );
                return;
            };
            let Some(index) = self.project.parts.iter().position(|part| part.id == id) else {
                return;
            };
            if let Some(part) = ui
                .push_id(&id, |ui| {
                    inspector::properties(ui, language, &self.project, index, !snapshot.running)
                })
                .inner
            {
                self.project.parts[index] = part;
                self.edit();
            }
            if ui
                .add_enabled(
                    !snapshot.running,
                    egui::Button::new(language.text("Remove object")).shortcut_text("Delete"),
                )
                .clicked()
            {
                self.remove_part(&id);
                return;
            }
            ui.separator();
            if let Some(pipes) =
                inspector::pipes(ui, language, &self.project, &id, !snapshot.running)
            {
                if self.dirty {
                    self.project.pipes = pipes;
                } else {
                    let result = self
                        .session
                        .runtime
                        .handle
                        .execute(Action::SetPipes(pipes.clone()));
                    if result.is_ok() {
                        self.project.pipes = pipes;
                        self.unsaved = true;
                    }
                    self.set_error(result);
                }
            }
            let prefix = format!("{id}.");
            for (tag_id, tag) in self.project.tags().range(prefix.clone()..) {
                if !tag_id.starts_with(&prefix) {
                    break;
                }
                self.tag_row(ui, tag, snapshot);
            }
        });
    }

    fn tag_row(&mut self, ui: &mut Ui, tag: &Tag, snapshot: &Snapshot) {
        let language = self.settings.language;
        let state = snapshot.tags.get(&tag.id);
        let live = state.map_or(tag.default, |state| state.value);
        let mut value = live;
        let mut enabled = tag.direction == Direction::Actuator;
        if self.dirty {
            enabled = false;
        }
        ui.push_id(&tag.id, |ui| {
            ui.horizontal(|ui| {
                let label = inspector::tag_label(language, &self.project, tag);
                ui.label(label).on_hover_text(&tag.id);
                if state.is_some_and(|state| state.quality == Quality::Stale) {
                    ui.label(RichText::new(language.text("Stale")).color(WARN));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(enabled, |ui| inspector::tag_input(ui, tag, &mut value));
                });
            });
            if value != live {
                let result = self
                    .session
                    .runtime
                    .handle
                    .execute(Action::Write(vec![(tag.id.clone(), value)]));
                self.set_error(result);
            }
            if !snapshot.running {
                ui.collapsing(
                    RichText::new(&tag.id).size(10.5).monospace().color(MUTE),
                    |ui| {
                        let mut name = self
                            .project
                            .tag_names
                            .get(&tag.id)
                            .cloned()
                            .unwrap_or_else(|| tag.name.clone());
                        if ui.text_edit_singleline(&mut name).changed() {
                            if name.trim().is_empty() {
                                self.project.tag_names.remove(&tag.id);
                            } else {
                                self.project.tag_names.insert(tag.id.clone(), name);
                            }
                            self.edit();
                        }
                    },
                );
            }
        });
    }

    fn bottom(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        let language = self.settings.language;
        ui.horizontal(|ui| {
            let (label, color) = if snapshot.running {
                (language.text("Running"), ACCENT)
            } else {
                (language.text("Paused"), WARN)
            };
            let (indicator, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().circle_filled(indicator.center(), 3.0, color);
            ui.label(RichText::new(label).color(color));
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "{:.2} {}     {HZ} {}     {}: {}",
                    snapshot.elapsed_seconds,
                    language.text("s"),
                    language.text("Hz"),
                    language.text("Parcels"),
                    snapshot.parcels.len()
                ))
                .color(MUTE),
            );
            if self
                .project
                .parts
                .iter()
                .any(|part| matches!(part.kind, PartKind::Staff { .. } | PartKind::Machine { .. }))
                && let Some(time) = snapshot.calendar_time
            {
                ui.label(format!(
                    "{} {}",
                    language.weekday(time.weekday()),
                    time.format("%Y-%m-%d %H:%M:%S")
                ));
            }
            if let Some((fps, hz)) = self.rates.measured {
                ui.label(
                    RichText::new(format!(
                        "{fps:.0} FPS · {hz:.0} {}",
                        language.text("sim Hz")
                    ))
                    .color(MUTE),
                );
            }
            if self.dirty && ui.button(language.text("Apply changes")).clicked() {
                let res = self.apply_edits();
                self.set_error(res);
            }
            if let Some((message, is_error)) = &self.notice {
                let text = if *is_error {
                    language.error(message)
                } else {
                    language.text(message).to_owned()
                };
                ui.label(RichText::new(text).color(if *is_error { WARN } else { ACCENT }));
                if ui.small_button("×").clicked() {
                    self.notice = None;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(self.backend.label()).color(MUTE))
                    .on_hover_text(&self.gpu);
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context, snapshot: &Snapshot) {
        let language = self.settings.language;
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut open = true;
        let mut done = false;
        match &mut dialog {
            Dialog::Library(catalog) => {
                if snapshot.running {
                    done = true;
                } else {
                    let choice = egui::Window::new(language.text("Element library"))
                        .open(&mut open)
                        .default_pos(ctx.content_rect().min + egui::vec2(28.0, 88.0))
                        .default_width(610.0)
                        .show(ctx, |ui| catalog.ui(ui, &self.project, snapshot, language))
                        .and_then(|response| response.inner.flatten());
                    match choice {
                        Some(library::Choice::Place(entry)) => {
                            self.add_part(
                                entry.kind,
                                entry.product.as_deref(),
                                Some(entry.name),
                                None,
                            );
                            done = true;
                        }
                        Some(library::Choice::Import) => {
                            self.dialog = Some(Dialog::Import(ImportProduct::default()));
                            done = true;
                        }
                        None => {}
                    }
                }
            }
            Dialog::New => {
                egui::Window::new(language.text("New project"))
                    .open(&mut open)
                    .resizable(false)
                    .show(ctx, |ui| {
                        if self.unsaved {
                            ui.colored_label(
                                WARN,
                                language.text("There are unsaved changes in the current project."),
                            );
                        }
                        if ui.button(language.text("Create empty project")).clicked() {
                            let project = Project::empty(language.text("Untitled line"));
                            match Session::new(project.clone(), true) {
                                Ok(session) => {
                                    self.project = project;
                                    self.session = session;
                                    self.path = None;
                                    self.selection = None;
                                    self.dirty = false;
                                    self.unsaved = true;
                                    self.camera = viewport::Camera::default();
                                    done = true;
                                }
                                Err(error) => self.set_error(Err(error)),
                            }
                        }
                    });
            }
            Dialog::File {
                action,
                path,
                error,
            } => {
                let (title, submit) = match action {
                    FileAction::Open => ("Open project", "Open"),
                    FileAction::Save { then_open: None } => ("Save project", "Save"),
                    FileAction::Save { .. } => ("Save current project", "Save and open"),
                };
                let response =
                    egui::Modal::new(egui::Id::unique(("project file", title))).show(ctx, |ui| {
                        ui.set_width(430.0);
                        ui.heading(language.text(title));
                        ui.label(language.text("Project file path"));
                        let edit =
                            ui.add(egui::TextEdit::singleline(path).desired_width(f32::INFINITY));
                        if let Some(error) = error {
                            ui.colored_label(WARN, language.error(error));
                        }
                        ui.horizontal(|ui| {
                            let submit = ui
                                .add_enabled(
                                    !path.trim().is_empty(),
                                    egui::Button::new(language.text(submit)),
                                )
                                .clicked();
                            let cancel = ui.button(language.text("Cancel")).clicked();
                            let enter =
                                edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            (submit || (enter && !path.trim().is_empty()), cancel)
                        })
                        .inner
                    });
                let (submitted, cancelled) = response.inner;
                if cancelled || response.should_close() {
                    done = true;
                } else if submitted {
                    let path = PathBuf::from(path.trim());
                    match action {
                        FileAction::Open if self.unsaved => {
                            self.dialog = Some(Dialog::ConfirmOpen(path));
                            done = true;
                        }
                        FileAction::Open => match self.open_project(&path) {
                            Ok(()) => done = true,
                            Err(failure) => *error = Some(failure.to_string()),
                        },
                        FileAction::Save { then_open } => match self.save_project(&path) {
                            Ok(()) => {
                                done = true;
                                if let Some(destination) = then_open.take() {
                                    self.resume_open(destination);
                                }
                            }
                            Err(failure) => *error = Some(failure.to_string()),
                        },
                    }
                }
            }
            Dialog::ConfirmOpen(path) => {
                let response =
                    egui::Modal::new(egui::Id::unique("confirm open project")).show(ctx, |ui| {
                        ui.set_width(430.0);
                        ui.heading(language.text("Unsaved project"));
                        ui.label(language.text("Save changes before opening the other project?"));
                        ui.label(RichText::new(path.display().to_string()).color(MUTE));
                        ui.horizontal(|ui| {
                            if ui.button(language.text("Save…")).clicked() {
                                self.dialog = Some(self.save_dialog(Some(path.clone())));
                                done = true;
                            }
                            if ui.button(language.text("Open without saving")).clicked() {
                                self.resume_open(path.clone());
                                done = true;
                            }
                            if ui.button(language.text("Cancel")).clicked() {
                                done = true;
                            }
                        });
                    });
                done |= response.should_close();
            }
            Dialog::Import(import) => {
                let product = egui::Window::new(language.text("Import 3D product"))
                    .open(&mut open)
                    .resizable(false)
                    .default_width(440.0)
                    .show(ctx, |ui| import.show(ui, !snapshot.running, language))
                    .and_then(|response| response.inner)
                    .flatten();
                if let Some(product) = product {
                    let id = (1..)
                        .map(|number| format!("product-{number}"))
                        .find(|id| !self.project.products.contains_key(id))
                        .unwrap();
                    self.project.products.insert(id.clone(), product);
                    self.add_product(&id);
                    done = true;
                }
            }
            Dialog::Production { json, error } => {
                egui::Window::new(language.text("Production"))
                    .open(&mut open)
                    .default_width(600.0)
                    .show(ctx, |ui| {
                        if let Some(settings) = editor_ui::production(
                            ui,
                            language,
                            &self.project,
                            json,
                            error,
                            !snapshot.running,
                        ) {
                            self.project.production = settings;
                            self.edit();
                            done = true;
                        }
                    });
            }
            Dialog::Settings => {
                egui::Window::new(language.text("Language and graphics"))
                    .open(&mut open)
                    .id(egui::Id::unique("studio-preferences"))
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(language.text("Language"));
                            ui.radio_value(
                                &mut self.settings.language,
                                Language::Russian,
                                "Русский",
                            );
                            ui.radio_value(
                                &mut self.settings.language,
                                Language::English,
                                "English",
                            );
                        });
                        ui.separator();
                        let previous_quality = self.settings.graphics_quality;
                        editor_ui::graphics_quality(
                            ui,
                            language,
                            &mut self.settings.graphics_quality,
                        );
                        ui.separator();
                        ui.label(format!(
                            "{} · {} · {}",
                            language.text("Active"),
                            self.backend.label(),
                            self.gpu
                        ));
                        let previous = self.settings.backend;
                        ui.radio_value(&mut self.settings.backend, Backend::Vulkan, "Vulkan");
                        #[cfg(not(target_os = "macos"))]
                        ui.radio_value(&mut self.settings.backend, Backend::OpenGl, "OpenGL");
                        let backend_changed = previous != self.settings.backend;
                        if backend_changed
                            || self.settings.language != language
                            || self.settings.graphics_quality != previous_quality
                        {
                            let result = self.save_settings();
                            if backend_changed && result.is_ok() {
                                self.notice = Some((
                                    "Backend saved. Restart Factory Sim to apply.".into(),
                                    false,
                                ));
                            }
                            self.set_error(result);
                            ctx.request_repaint();
                        }
                        if self.settings.backend != self.backend {
                            ui.label(language.text("Applied on next launch."));
                        }
                        if ui.button(language.text("Reset view")).clicked() {
                            self.camera = viewport::Camera::default();
                        }
                    });
            }
            Dialog::Drivers { json, error } => {
                egui::Window::new(language.text("PLC connections"))
                    .open(&mut open)
                    .default_width(630.0)
                    .show(ctx, |ui| {
                        for (id, state) in &snapshot.drivers {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(id).monospace());
                                ui.label(RichText::new(language.text(state)).color(MUTE));
                            });
                        }
                        ui.separator();
                        ui.label(language.text("Driver profiles and symbol mappings"));
                        egui::ScrollArea::vertical()
                            .max_height(395.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(json)
                                        .code_editor()
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(16),
                                );
                            });
                        if let Some(error) = error.as_ref() {
                            ui.colored_label(WARN, language.error(error));
                        }
                        if ui
                            .add_enabled(
                                !snapshot.running,
                                egui::Button::new(language.text("Apply profiles")),
                            )
                            .clicked()
                        {
                            let res = (|| {
                                let drivers: Vec<Driver> = serde_json::from_str(json)?;
                                let mut project = self.project.clone();
                                project.drivers = drivers;
                                let session = Session::new(project.clone(), true)?;
                                self.project = project;
                                self.session = session;
                                self.dirty = false;
                                self.unsaved = true;
                                Ok::<(), anyhow::Error>(())
                            })();
                            *error = res.err().map(|e| e.to_string());
                            done = error.is_none();
                        }
                    });
            }
        }
        if open && !done {
            self.dialog = Some(dialog);
        }
    }

    fn save_settings(&self) -> Result<()> {
        let path = self
            .path_for_settings
            .as_ref()
            .context("no settings directory")?;
        fs::create_dir_all(path.parent().context("invalid settings path")?)?;
        fs::write(path, serde_json::to_vec_pretty(&self.settings)?)?;
        Ok(())
    }
}

impl eframe::App for Studio {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.shown {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.request_repaint();
            self.shown = true;
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let frame_start = Instant::now();
        let snapshot = self.session.runtime.handle.snapshot();
        self.rates.sample(&snapshot, frame_start);
        egui::Panel::top("toolbar")
            .exact_size(54.0)
            .show(ui, |ui| self.top(ui, &snapshot));
        egui::Panel::bottom("status")
            .exact_size(31.0)
            .show(ui, |ui| self.bottom(ui, &snapshot));
        egui::Panel::left("scene")
            .default_size(205.0)
            .min_size(168.0)
            .show(ui, |ui| self.left(ui, snapshot.running));
        egui::Panel::right("inspector")
            .default_size(268.0)
            .min_size(224.0)
            .show(ui, |ui| self.right(ui, &snapshot));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let interaction = viewport::draw(
                    ui,
                    self.settings.language,
                    &mut self.camera,
                    self.settings.graphics_quality,
                    &self.project,
                    &snapshot,
                    self.selection.as_deref(),
                    self.dirty,
                    !snapshot.running
                        && (self.dialog.is_none()
                            || matches!(self.dialog, Some(Dialog::Library(_)))),
                    &mut self.object_drag,
                );
                if let Some((entry, at)) = interaction.library_drop {
                    self.add_part(
                        entry.kind,
                        entry.product.as_deref(),
                        Some(entry.name),
                        Some(at),
                    );
                    self.dialog = None;
                }
                if let Some(selection) = interaction.selection {
                    self.selection = selection;
                    if self.selection.is_some() {
                        self.show_tags = false;
                    }
                }
                if let Some((id, movement)) = interaction.movement
                    && let Some(part) = self.project.parts.iter_mut().find(|part| part.id == id)
                    && (part.position != movement.position.to_array() || part.yaw != movement.yaw)
                {
                    part.position = movement.position.to_array();
                    part.yaw = movement.yaw;
                    self.edit();
                }
            });
        self.dialogs(ui.ctx(), &snapshot);
        let ctx = ui.ctx();
        if !snapshot.running
            && self.dialog.is_none()
            && !ctx.text_edit_focused()
            && !egui::Popup::is_any_open(ctx)
            && let Some(id) = self.selection.clone()
            && ctx.input_mut(|input| {
                input.modifiers.is_none()
                    && input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
            })
        {
            self.remove_part(&id);
            ctx.request_repaint();
        }
        if snapshot.running {
            if let Some(fps) = self.fps_limit {
                let frame = Duration::from_secs_f64(1.0 / f64::from(fps));
                ui.ctx()
                    .request_repaint_after(frame.saturating_sub(frame_start.elapsed()));
            } else {
                ui.ctx().request_repaint();
            }
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        }
    }
}

enum PlaybackIcon {
    Play,
    Pause,
    Step,
    Reset,
}

fn playback_button(ui: &mut Ui, label: &str, icon: PlaybackIcon, enabled: bool) -> egui::Response {
    let mut button = egui::Button::new("")
        .min_size(egui::vec2(34.0, 32.0))
        .corner_radius(0)
        .stroke(egui::Stroke::NONE);
    if matches!(icon, PlaybackIcon::Play | PlaybackIcon::Pause) {
        button = button.fill(Color32::from_rgb(49, 87, 77));
    }
    let response = ui.add_enabled(enabled, button);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let color = ui.style().interact(&response).fg_stroke.color;
    let color = if enabled {
        color
    } else {
        color.gamma_multiply(0.5)
    };
    let stroke = egui::Stroke::new(1.8, color);
    let center = response.rect.center();
    let point = |x, y| center + egui::vec2(x, y);
    let painter = ui.painter();

    match icon {
        PlaybackIcon::Play | PlaybackIcon::Step => {
            let stepping = matches!(icon, PlaybackIcon::Step);
            let offset = if stepping { -2.0 } else { 0.0 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    point(-4.5 + offset, -6.0),
                    point(5.5 + offset, 0.0),
                    point(-4.5 + offset, 6.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
            if stepping {
                painter.line_segment([point(6.0, -6.0), point(6.0, 6.0)], stroke);
            }
        }
        PlaybackIcon::Pause => {
            for x in [-5.0, 1.5] {
                painter.rect_filled(
                    egui::Rect::from_min_max(point(x, -6.0), point(x + 3.5, 6.0)),
                    0,
                    color,
                );
            }
        }
        PlaybackIcon::Reset => {
            let arc = (0..=18)
                .map(|step| {
                    let angle = (-173.0_f32 + step as f32 * 15.0).to_radians();
                    point(angle.cos() * 6.4, angle.sin() * 6.4)
                })
                .collect();
            painter.add(egui::Shape::line(arc, stroke));
            painter.add(egui::Shape::line(
                vec![point(-6.4, -5.3), point(-6.4, -0.8), point(-1.9, -0.8)],
                stroke,
            ));
        }
    }
    if response.has_focus() {
        painter.rect_stroke(
            response.rect.shrink(2.0),
            1.0,
            stroke,
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(label).on_disabled_hover_text(label)
}

#[cfg(test)]
mod localization_tests {
    use super::*;

    #[test]
    fn settings_without_language_open_in_russian() {
        let settings: Settings = serde_json::from_str(r#"{"backend":"vulkan"}"#).unwrap();
        assert_eq!(settings.language, Language::Russian);
        assert_eq!(settings.backend, Backend::Vulkan);
        assert_eq!(settings.graphics_quality, render::GraphicsQuality::Balanced);
        let english = Settings {
            language: Language::English,
            ..settings
        };
        let serialized = serde_json::to_string(&english).unwrap();
        assert_eq!(
            serde_json::from_str::<Settings>(&serialized)
                .unwrap()
                .language,
            Language::English
        );
    }

    #[test]
    fn equipment_and_generated_tags_have_russian_labels() {
        let kinds = library::presets();
        let mut project = Project::demo();
        project.parts.clear();
        project.tag_names.clear();
        for (index, kind) in kinds.enumerate() {
            let label = kind.label();
            assert_ne!(Language::Russian.text(label), label);
            project.parts.push(Part {
                id: format!("equipment-{index}"),
                name: label.into(),
                position: [0.0; 3],
                yaw: 0.0,
                product: None,
                kind,
            });
        }
        for tag in project.tags().into_values() {
            let label = tag
                .name
                .rsplit_once(" · ")
                .map_or(tag.name.as_str(), |(_, role)| role);
            assert_ne!(
                Language::Russian.text(label),
                label,
                "missing translation for {}",
                tag.id
            );
        }
        for (_, spec) in product::BOXES {
            assert_ne!(Language::Russian.text(spec.name), spec.name);
        }
    }
}
