use std::collections::BTreeSet;

use eframe::egui::{ComboBox, ScrollArea, TextEdit, Ui};

use super::Edit;
use super::Language;
use crate::project::{
    Project,
    diagram::{DiagramKind, DiagramVariant, InstrumentNotation, LinkChange, ProcessDiagram},
};

#[derive(Default)]
pub(super) struct Controls {
    pub active: Option<String>,
    new_name: String,
    new_kind: DiagramKind,
    new_notation: InstrumentNotation,
    rename: String,
    source: Option<String>,
    selected_links: BTreeSet<LinkChange>,
}

impl Controls {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        language: Language,
        edit: &mut Edit,
    ) -> bool {
        if self
            .active
            .as_ref()
            .is_some_and(|id| !project.diagram_variants.contains_key(id))
        {
            self.active = None;
        }
        let previous = self.active.clone();
        ui.horizontal_wrapped(|ui| {
            ComboBox::from_id_salt("diagram_variant")
                .selected_text(name(
                    project,
                    self.active.as_deref().unwrap_or_default(),
                    language,
                ))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.active, None, language.text("Main variant"));
                    for (id, variant) in &project.diagram_variants {
                        ui.selectable_value(&mut self.active, Some(id.clone()), &variant.name);
                    }
                });
            ui.menu_button(language.text("New variant…"), |ui| {
                ui.add(
                    TextEdit::singleline(&mut self.new_name)
                        .hint_text(language.text("Variant name")),
                );
                kind_picker(ui, language, &mut self.new_kind);
                notation_picker(ui, language, &mut self.new_notation);
                if ui.button(language.text("Create variant")).clicked() {
                    let serial = (1..)
                        .find(|number| {
                            !project
                                .diagram_variants
                                .contains_key(&format!("diagram-{number}"))
                        })
                        .unwrap();
                    let id = format!("diagram-{serial}");
                    let name = if self.new_name.trim().is_empty() {
                        format!("{} {serial}", language.text("Variant"))
                    } else {
                        self.new_name.trim().to_owned()
                    };
                    let mut variants = project.diagram_variants.clone();
                    variants.insert(
                        id.clone(),
                        DiagramVariant {
                            name: name.clone(),
                            sheet: ProcessDiagram {
                                kind: self.new_kind,
                                notation: self.new_notation,
                                ..Default::default()
                            },
                        },
                    );
                    edit.variants = Some(variants);
                    self.new_name.clear();
                    self.rename = name;
                    self.active = Some(id);
                    ui.close();
                }
            });
            if let Some(id) = self.active.clone()
                && let Some(variant) = project.diagram_variants.get(&id)
            {
                ui.menu_button(language.text("Manage variant…"), |ui| {
                    ui.add(
                        TextEdit::singleline(&mut self.rename)
                            .hint_text(language.text("Variant name")),
                    );
                    let rename = self.rename.trim();
                    if !rename.is_empty()
                        && rename != variant.name
                        && ui.button(language.text("Rename variant")).clicked()
                    {
                        let mut variants = project.diagram_variants.clone();
                        variants.get_mut(&id).unwrap().name = rename.to_owned();
                        edit.variants = Some(variants);
                        ui.close();
                    }
                    ui.menu_button(language.text("Delete variant…"), |ui| {
                        if ui
                            .button(language.text("Confirm variant deletion"))
                            .clicked()
                        {
                            let mut variants = project.diagram_variants.clone();
                            variants.remove(&id);
                            edit.variants = Some(variants);
                            self.active = None;
                            ui.close();
                        }
                    });
                });
            }
        });
        if self.active == previous {
            return false;
        }
        if let Some(id) = self.active.as_deref()
            && let Some(existing) = project.diagram_variants.get(id)
        {
            self.rename.clone_from(&existing.name);
        }
        self.source = None;
        self.selected_links.clear();
        true
    }

    pub fn settings(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        sheet: &ProcessDiagram,
        language: Language,
        edit: &mut Edit,
    ) {
        ui.horizontal_wrapped(|ui| {
            let mut kind = sheet.kind;
            let mut notation = sheet.notation;
            kind_picker(ui, language, &mut kind);
            notation_picker(ui, language, &mut notation);
            if kind != sheet.kind || notation != sheet.notation {
                let changed = edit.diagram(sheet);
                changed.kind = kind;
                changed.notation = notation;
            }
        });
        if project.diagram_variants.is_empty() {
            return;
        }
        ui.collapsing(language.text("Synchronize control links"), |ui| {
            let target = self.active.as_deref().unwrap_or_default();
            let choices =
                std::iter::once("").chain(project.diagram_variants.keys().map(String::as_str));
            let old_source = self.source.clone();
            ComboBox::from_id_salt("control_link_source")
                .selected_text(self.source.as_ref().map_or_else(
                    || language.text("Choose source variant"),
                    |id| name(project, id, language),
                ))
                .show_ui(ui, |ui| {
                    for id in choices.filter(|&id| id != target) {
                        ui.selectable_value(
                            &mut self.source,
                            Some(id.to_owned()),
                            name(project, id, language),
                        );
                    }
                });
            if old_source != self.source {
                self.selected_links.clear();
            }
            let Some(id) = self.source.as_deref().filter(|&id| id != target) else {
                return;
            };
            let Some(source) = project.diagram((!id.is_empty()).then_some(id)) else {
                self.source = None;
                self.selected_links.clear();
                return;
            };
            let changes = sheet.changes_from(source);
            self.selected_links
                .retain(|change| changes.contains(change));
            if changes.is_empty() {
                ui.label(language.text("Control links already match."));
                return;
            }
            ScrollArea::vertical().max_height(145.0).show(ui, |ui| {
                for change in changes {
                    let action = match (&change.before, &change.after) {
                        (None, _) => "Add link",
                        (_, None) => "Remove link",
                        _ => "Replace link",
                    };
                    let before = change.before.as_deref().unwrap_or("—");
                    let after = change.after.as_deref().unwrap_or("—");
                    let label = format!(
                        "{} · {} · {} → {}",
                        language.text(action),
                        change.input,
                        before,
                        after
                    );
                    let mut checked = self.selected_links.contains(&change);
                    if ui.checkbox(&mut checked, label).changed() {
                        if checked {
                            self.selected_links.insert(change);
                        } else {
                            self.selected_links.remove(&change);
                        }
                    }
                }
            });
            if ui
                .add_enabled(
                    !self.selected_links.is_empty(),
                    eframe::egui::Button::new(language.text("Apply selected links")),
                )
                .clicked()
            {
                edit.diagram(sheet)
                    .apply_links(std::mem::take(&mut self.selected_links));
            }
        });
    }
}

fn name<'a>(project: &'a Project, id: &str, language: Language) -> &'a str {
    project
        .diagram_variants
        .get(id)
        .map_or(language.text("Main variant"), |variant| &variant.name)
}

fn kind_picker(ui: &mut Ui, language: Language, kind: &mut DiagramKind) {
    ComboBox::from_id_salt("diagram_kind")
        .selected_text(language.text(kind.label()))
        .show_ui(ui, |ui| {
            for choice in [
                DiagramKind::Functional,
                DiagramKind::IsoBlock,
                DiagramKind::IsoPfd,
                DiagramKind::IsoPid,
            ] {
                ui.selectable_value(kind, choice, language.text(choice.label()));
            }
        });
}

fn notation_picker(ui: &mut Ui, language: Language, notation: &mut InstrumentNotation) {
    ComboBox::from_id_salt("instrument_notation")
        .selected_text(language.text(notation.label()))
        .show_ui(ui, |ui| {
            for choice in [InstrumentNotation::Gost, InstrumentNotation::Isa] {
                ui.selectable_value(notation, choice, language.text(choice.label()));
            }
        });
}

impl DiagramKind {
    fn label(self) -> &'static str {
        match self {
            Self::Functional => "Automation scheme",
            Self::IsoBlock => "ISO 10628 · Block diagram",
            Self::IsoPfd => "ISO 10628 · Process flow (PFD)",
            Self::IsoPid => "ISO 10628 · Piping and instrumentation (P&ID)",
        }
    }
}

impl InstrumentNotation {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Gost => "Instrumentation (GOST 21.208)",
            Self::Isa => "Instrumentation (ISA-5.1)",
        }
    }
}
