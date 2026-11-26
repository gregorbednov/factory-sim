use eframe::egui::{self, Color32, RichText, Ui};

use super::{i18n::Language, render::GraphicsQuality};
use crate::{
    production::{Calendar, Production, Shift},
    project::{PartKind, Project},
};

pub(super) const PANEL: Color32 = Color32::from_rgb(28, 34, 38);
pub(super) const TEXT: Color32 = Color32::from_rgb(226, 234, 233);
pub(super) const MUTE: Color32 = Color32::from_rgb(156, 173, 175);
pub(super) const ACCENT: Color32 = Color32::from_rgb(119, 227, 193);
pub(super) const WARN: Color32 = Color32::from_rgb(249, 192, 112);

pub(super) fn theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(37, 45, 50);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(42, 53, 58);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(54, 73, 77);
    visuals.widgets.active.bg_fill = Color32::from_rgb(53, 90, 81);
    visuals.selection.bg_fill = Color32::from_rgb(52, 93, 81);
    visuals.selection.stroke.color = ACCENT;
    ctx.set_visuals(visuals);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
    });
}

pub(super) fn scene_list(
    ui: &mut Ui,
    language: Language,
    project: &Project,
    selection: &mut Option<String>,
    running: bool,
) -> Option<String> {
    let mut remove = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for part in &project.parts {
            ui.push_id(&part.id, |ui| {
                let selected = selection.as_deref() == Some(&part.id);
                let color = match part.kind {
                    PartKind::Sensor { .. } | PartKind::Emitter { .. } => ACCENT,
                    PartKind::Pusher { .. } | PartKind::Crate { .. } => WARN,
                    _ => MUTE,
                };
                let row = ui.selectable_label(selected, RichText::new(&part.name).color(color));
                if row.clicked() {
                    *selection = Some(part.id.clone());
                }
                row.context_menu(|ui| {
                    if ui
                        .add_enabled(
                            !running,
                            egui::Button::new(language.text("Remove object"))
                                .shortcut_text("Delete"),
                        )
                        .clicked()
                    {
                        remove = Some(part.id.clone());
                        ui.close();
                    }
                });
            });
        }
    });
    remove
}

pub(super) fn production(
    ui: &mut Ui,
    language: Language,
    project: &Project,
    json: &mut String,
    error: &mut Option<String>,
    editable: bool,
) -> Option<Production> {
    ui.label(language.text("Initial simulated date, seed and work calendars"));
    if ui
        .add_enabled(
            editable,
            egui::Button::new(language.text("Add weekday calendar")),
        )
        .clicked()
    {
        match serde_json::from_str::<Production>(json) {
            Ok(mut settings) => {
                let index = (1..)
                    .find(|i| !settings.calendars.contains_key(&format!("shift-{i}")))
                    .unwrap();
                settings.calendars.insert(
                    format!("shift-{index}"),
                    Calendar {
                        weekly: vec![Shift {
                            days: vec![1, 2, 3, 4, 5],
                            start_minute: 480,
                            end_minute: 960,
                        }],
                        overrides: Default::default(),
                    },
                );
                *json = serde_json::to_string_pretty(&settings).unwrap();
                *error = None;
            }
            Err(err) => *error = Some(err.to_string()),
        }
    }
    egui::ScrollArea::vertical()
        .max_height(380.0)
        .show(ui, |ui| {
            ui.add_enabled(
                editable,
                egui::TextEdit::multiline(json)
                    .code_editor()
                    .desired_rows(14)
                    .desired_width(f32::INFINITY),
            );
        });
    if let Some(error) = error.as_ref() {
        ui.colored_label(WARN, language.error(error));
    }
    if ui
        .add_enabled(
            editable,
            egui::Button::new(language.text("Apply production settings")),
        )
        .clicked()
    {
        let result = (|| {
            let settings: Production = serde_json::from_str(json)?;
            let mut candidate = project.clone();
            candidate.production = settings;
            candidate.validate()?;
            Ok::<_, anyhow::Error>(candidate.production)
        })();
        match result {
            Ok(settings) => return Some(settings),
            Err(err) => *error = Some(format!("{err:#}")),
        }
    }
    None
}

pub(super) fn graphics_quality(ui: &mut Ui, language: Language, quality: &mut GraphicsQuality) {
    ui.label(language.text("Graphics quality"));
    ui.horizontal_wrapped(|ui| {
        for (value, label) in [
            (GraphicsQuality::Performance, "Performance"),
            (GraphicsQuality::Balanced, "Balanced"),
            (GraphicsQuality::High, "High quality"),
        ] {
            ui.radio_value(quality, value, language.text(label));
        }
    });
}
