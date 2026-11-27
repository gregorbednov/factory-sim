use std::collections::BTreeMap;

use eframe::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
    pos2, vec2,
};

use super::{editor_ui::*, i18n::Language, inspector};
use crate::{
    project::{
        Direction, Part, PartKind, Pipe, Project, Value, WaterMeasurement,
        diagram::{
            COORDINATE_LIMIT, DiagramRoute, Instrument, InstrumentFunction, PipeEnds,
            ProcessDiagram, Quantity,
        },
    },
    sim::{Quality, Snapshot},
};

const COLUMN: f32 = 225.0;
const ROW: f32 = 156.0;
const GRID: f32 = 20.0;
const NODE_SIZE: Vec2 = vec2(76.0, 54.0);
const BACKGROUND: Color32 = Color32::from_rgb(20, 26, 30);
const GRID_COLOR: Color32 = Color32::from_rgb(35, 44, 49);

#[derive(Default)]
pub(super) struct View {
    pub open: bool,
    pan: Vec2,
    scale: f32,
    fit: bool,
    selected_pipe: Option<PipeEnds>,
    instrument_tag: Option<String>,
    drag: Option<Drag>,
}

#[derive(Default)]
pub(super) struct Edit {
    pub selection: Option<Option<String>>,
    pub diagram: Option<ProcessDiagram>,
    pub pipes: Option<Vec<Pipe>>,
}

impl Edit {
    fn diagram(&mut self, project: &Project) -> &mut ProcessDiagram {
        self.diagram
            .get_or_insert_with(|| project.process_diagram.clone())
    }
}

enum Target {
    Node(String),
    Bend(PipeEnds),
    Canvas,
}

struct Drag {
    target: Target,
    start: Pos2,
    origin: Pos2,
    point: Pos2,
}

struct Route<'a> {
    pipe: &'a Pipe,
    key: PipeEnds,
    bend: Pos2,
    points: [Pos2; 7],
}

#[derive(Clone, Copy)]
struct Canvas {
    origin: Pos2,
    scale: f32,
}

impl Canvas {
    fn point(self, position: Pos2) -> Pos2 {
        self.origin + position.to_vec2() * self.scale
    }

    fn unproject(self, point: Pos2) -> Pos2 {
        ((point - self.origin) / self.scale).to_pos2()
    }
}

impl View {
    pub fn tabs(&mut self, ui: &mut Ui, language: Language) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.open, false, language.text("3D scene"));
            ui.selectable_value(&mut self.open, true, language.text("Process diagram"));
        });
        if !self.open {
            self.drag = None;
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        snapshot: Option<&Snapshot>,
        selected: Option<&str>,
        can_edit_pipes: bool,
        language: Language,
    ) -> Edit {
        let mut edit = Edit::default();
        let parts: BTreeMap<_, _> = project
            .parts
            .iter()
            .filter(|part| has_water_ports(part))
            .map(|part| (part.id.as_str(), part))
            .collect();
        if parts.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(language.text("Add water equipment to see a process diagram."));
            });
            self.scale = 0.0;
            self.selected_pipe = None;
            self.drag = None;
            return edit;
        }

        let layout = positions(project, &parts);
        let selected_part = selected.and_then(|id| parts.get(id).copied());
        if selected.is_some() {
            self.selected_pipe = None;
        } else if let Some(key) = &self.selected_pipe
            && !project
                .pipes
                .iter()
                .any(|pipe| PipeEnds::from(pipe) == *key)
        {
            self.selected_pipe = None;
        }

        ui.horizontal_wrapped(|ui| {
            if ui.button(language.text("Fit diagram")).clicked() {
                self.fit = true;
            }
            if ui.button("−").clicked() {
                self.zoom(0.8, Vec2::ZERO);
            }
            if ui.button("+").clicked() {
                self.zoom(1.25, Vec2::ZERO);
            }
            ui.small(
                language.text("Drag symbols or the background. Select a pipe to move its bend."),
            );
        });
        if let Some(part) = selected_part {
            ui.collapsing(language.text("Diagram properties"), |ui| {
                node_properties(
                    ui,
                    language,
                    project,
                    part,
                    &mut self.instrument_tag,
                    &mut edit,
                );
            });
            ui.collapsing(language.text("Water connections"), |ui| {
                if !can_edit_pipes {
                    ui.label(language.text("Pause to edit properties"));
                }
                edit.pipes = inspector::pipes(ui, language, project, &part.id, can_edit_pipes);
            });
        } else if let Some(key) = &self.selected_pipe {
            ui.horizontal_wrapped(|ui| {
                let from = parts.get(key.from.as_str());
                let to = parts.get(key.to.as_str());
                if let (Some(from), Some(to)) = (from, to) {
                    ui.label(format!(
                        "{} {} → {} {}",
                        from.name,
                        key.from_port + 1,
                        to.name,
                        key.to_port + 1
                    ));
                }
                if project
                    .process_diagram
                    .routes
                    .iter()
                    .any(|route| &route.pipe == key)
                    && ui.button(language.text("Reset pipe bend")).clicked()
                {
                    edit.diagram(project)
                        .routes
                        .retain(|route| &route.pipe != key);
                }
            });
        }

        let (response, painter) = ui.allocate_painter(
            ui.available_size().max(Vec2::splat(1.0)),
            Sense::click_and_drag(),
        );
        let rect = response.rect;
        let sheet = edit.diagram.as_ref().unwrap_or(&project.process_diagram);
        let mut routes = routes(project, sheet, &layout);
        if self.scale == 0.0 || std::mem::take(&mut self.fit) {
            let mut bounds = Rect::NOTHING;
            for &point in layout.values() {
                bounds.extend_with(point - vec2(100.0, 96.0));
                bounds.extend_with(point + vec2(100.0, 65.0));
            }
            for route in &routes {
                bounds.extend_with(route.bend);
            }
            let scale = rect.size() / (bounds.size() + Vec2::splat(32.0));
            self.scale = scale.x.min(scale.y).clamp(0.08, 1.25);
            self.pan = -bounds.center().to_vec2() * self.scale;
        }
        let primary = egui::PointerButton::Primary;
        if self.drag.is_none() && response.hovered() {
            let (scroll, zoom, pointer) = ui.input(|input| {
                (
                    input.smooth_scroll_delta.y,
                    input.zoom_delta(),
                    input.pointer.hover_pos(),
                )
            });
            if let Some(pointer) = pointer {
                self.zoom((scroll * 0.002).exp() * zoom, pointer - rect.center());
            }
        }
        if response.dragged_by(egui::PointerButton::Middle) {
            self.pan += response.drag_delta();
        }
        let canvas = Canvas {
            origin: rect.center() + self.pan,
            scale: self.scale,
        };
        if response.clicked_by(primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let point = canvas.unproject(pointer);
            match self.pick(point, &layout, &routes) {
                Target::Node(id) => {
                    self.selected_pipe = None;
                    edit.selection = Some(Some(id));
                }
                Target::Bend(pipe) => {
                    self.selected_pipe = Some(pipe);
                    edit.selection = Some(None);
                }
                Target::Canvas => {
                    self.selected_pipe = None;
                    edit.selection = Some(None);
                }
            }
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.drag = None;
        } else if response.drag_started_by(primary)
            && let Some(start) = ui.input(|input| input.pointer.press_origin())
        {
            let target = self.pick(canvas.unproject(start), &layout, &routes);
            let origin = match &target {
                Target::Node(id) => {
                    self.selected_pipe = None;
                    edit.selection = Some(Some(id.clone()));
                    layout[id.as_str()]
                }
                Target::Bend(key) => {
                    self.selected_pipe = Some(key.clone());
                    edit.selection = Some(None);
                    routes.iter().find(|route| &route.key == key).unwrap().bend
                }
                Target::Canvas => Pos2::ZERO,
            };
            self.drag = Some(Drag {
                target,
                start,
                origin,
                point: origin,
            });
        }

        if let Some(drag) = &mut self.drag {
            if matches!(drag.target, Target::Canvas) {
                self.pan += response.drag_delta();
            } else if let Some(pointer) = ui.input(|input| input.pointer.latest_pos()) {
                let point = drag.origin + (pointer - drag.start) / self.scale;
                drag.point = pos2(limit(point.x), limit(point.y));
            }
        }
        let canvas = Canvas {
            origin: rect.center() + self.pan,
            ..canvas
        };
        let mut layout = layout;
        if let Some(Drag {
            target: Target::Node(id),
            point,
            ..
        }) = &self.drag
            && let Some(position) = layout.get_mut(id.as_str())
        {
            *position = *point;
            routes = self::routes(project, sheet, &layout);
        }
        if let Some(Drag {
            target: Target::Bend(key),
            point,
            ..
        }) = &self.drag
            && let Some(route) = routes.iter_mut().find(|route| &route.key == key)
        {
            route.bend = *point;
            route.points = route_points(route.points[0], route.points[6], *point);
        }
        let selected = edit.selection.as_ref().map_or(selected, |id| id.as_deref());
        draw_grid(&painter, rect, canvas);
        for route in &routes {
            let active = self.selected_pipe.as_ref() == Some(&route.key);
            let outlet = parts[route.key.from.as_str()];
            draw_route(
                &painter,
                canvas,
                route,
                active,
                pipe_flow(snapshot, outlet, route.pipe.from_port),
            );
        }
        draw_instruments(&painter, canvas, sheet, snapshot, &layout, language);
        for (id, part) in parts {
            let label = sheet
                .nodes
                .get(id)
                .and_then(|node| node.label.as_deref())
                .unwrap_or(&part.name);
            draw_node(
                &painter,
                canvas,
                layout[id],
                part,
                label,
                selected == Some(id),
                snapshot,
                language,
            );
        }

        if !ui.input(|input| input.pointer.button_down(primary))
            && let Some(drag) = self.drag.take()
            && drag.point != drag.origin
        {
            let diagram = edit.diagram(project);
            let point = [drag.point.x, drag.point.y];
            match drag.target {
                Target::Node(id) if layout.contains_key(id.as_str()) => {
                    diagram.nodes.entry(id).or_default().position = Some(point);
                }
                Target::Bend(pipe) if routes.iter().any(|route| route.key == pipe) => {
                    diagram.routes.retain(|route| route.pipe != pipe);
                    diagram.routes.push(DiagramRoute { pipe, bend: point });
                }
                _ => {}
            }
        }
        edit
    }

    fn zoom(&mut self, factor: f32, anchor: Vec2) {
        if self.scale > 0.0 {
            let next = (self.scale * factor).clamp(0.08, 3.5);
            self.pan = anchor + (self.pan - anchor) * (next / self.scale);
            self.scale = next;
        }
    }

    fn pick(&self, point: Pos2, layout: &BTreeMap<&str, Pos2>, routes: &[Route<'_>]) -> Target {
        let tolerance = 8.0 / self.scale;
        if let Some(route) = routes.iter().find(|route| {
            self.selected_pipe.as_ref() == Some(&route.key)
                && route.bend.distance(point) <= tolerance * 1.5
        }) {
            return Target::Bend(route.key.clone());
        }
        if let Some((&id, _)) = layout.iter().rev().find(|&(_, &center)| {
            Rect::from_center_size(center, NODE_SIZE)
                .expand(4.0)
                .contains(point)
        }) {
            return Target::Node(id.to_owned());
        }
        routes
            .iter()
            .flat_map(|route| route.points.windows(2).map(move |edge| (route, edge)))
            .map(|(route, edge)| {
                let segment = edge[1] - edge[0];
                let ratio = (point - edge[0]).dot(segment) / segment.length_sq().max(0.001);
                let distance = point.distance(edge[0] + segment * ratio.clamp(0.0, 1.0));
                (route, distance)
            })
            .filter(|(_, distance)| *distance <= tolerance)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map_or(Target::Canvas, |(route, _)| Target::Bend(route.key.clone()))
    }
}

fn has_water_ports(part: &Part) -> bool {
    part.kind.has_water_inlet() || part.kind.has_water_outlet()
}

fn as_pos([x, y]: [f32; 2]) -> Pos2 {
    pos2(x, y)
}

fn limit(value: f32) -> f32 {
    value.clamp(-COORDINATE_LIMIT, COORDINATE_LIMIT)
}

fn positions<'a>(
    project: &'a Project,
    parts: &BTreeMap<&'a str, &Part>,
) -> BTreeMap<&'a str, Pos2> {
    let mut levels = BTreeMap::new();
    let mut incoming: BTreeMap<_, Vec<_>> = parts.keys().map(|&id| (id, Vec::new())).collect();
    let mut outgoing = incoming.clone();
    for pipe in &project.pipes {
        if let Some(inputs) = incoming.get_mut(pipe.to.as_str())
            && let Some(outputs) = outgoing.get_mut(pipe.from.as_str())
        {
            inputs.push(pipe.from.as_str());
            outputs.push(pipe.to.as_str());
        }
    }
    let mut remaining: BTreeMap<_, _> = incoming
        .iter()
        .map(|(&id, inputs)| (id, inputs.len()))
        .collect();
    let mut ready: Vec<_> = remaining
        .iter()
        .filter_map(|(&id, &count)| (count == 0).then_some(id))
        .collect();
    while let Some(part) = ready.pop() {
        let level = incoming[part]
            .iter()
            .map(|id| levels[id] + 1)
            .max()
            .unwrap_or(0);
        levels.insert(part, level);
        for &next in &outgoing[part] {
            let count = remaining.get_mut(next).unwrap();
            *count -= 1;
            if *count == 0 {
                ready.push(next);
            }
        }
    }
    let mut columns = BTreeMap::<_, Vec<_>>::new();
    for part in project.parts.iter().filter(|part| has_water_ports(part)) {
        let id = part.id.as_str();
        columns
            .entry(levels.get(id).copied().unwrap_or(0))
            .or_default()
            .push(id);
    }
    let mut layout = BTreeMap::new();
    for (level, column) in columns {
        let middle = (column.len() - 1) as f32 * 0.5;
        for (row, id) in column.into_iter().enumerate() {
            let position = project
                .process_diagram
                .nodes
                .get(id)
                .and_then(|node| node.position);
            layout.insert(
                id,
                position.map_or_else(
                    || pos2(level as f32 * COLUMN, (row as f32 - middle) * ROW),
                    as_pos,
                ),
            );
        }
    }
    layout
}

fn routes<'a>(
    project: &'a Project,
    sheet: &ProcessDiagram,
    layout: &BTreeMap<&str, Pos2>,
) -> Vec<Route<'a>> {
    let bends: BTreeMap<_, _> = sheet
        .routes
        .iter()
        .map(|route| (&route.pipe, route.bend))
        .collect();
    let parts: BTreeMap<_, _> = project
        .parts
        .iter()
        .map(|part| (part.id.as_str(), part))
        .collect();
    project
        .pipes
        .iter()
        .filter_map(|pipe| {
            let key = PipeEnds::from(pipe);
            let from = *layout.get(key.from.as_str())?;
            let to = *layout.get(key.to.as_str())?;
            let offset = |end: &str, outlet: bool, index: u8| {
                let count = parts[end].kind.water_port_count(outlet);
                (f32::from(index) - (f32::from(count) - 1.0) / 2.0) * 18.0
            };
            let start = from + vec2(NODE_SIZE.x / 2.0, offset(&key.from, true, key.from_port));
            let end = to + vec2(-NODE_SIZE.x / 2.0, offset(&key.to, false, key.to_port));
            let bend = bends
                .get(&key)
                .copied()
                .map_or_else(|| default_bend(start, end, layout), as_pos);
            Some(Route {
                pipe,
                key,
                bend,
                points: route_points(start, end, bend),
            })
        })
        .collect()
}

fn default_bend(start: Pos2, end: Pos2, layout: &BTreeMap<&str, Pos2>) -> Pos2 {
    if end.x < start.x + 36.0 {
        return pos2(start.x + 18.0, start.y.min(end.y) - 62.0);
    }
    let midpoint = pos2((start.x + end.x) / 2.0, end.y);
    let points = route_points(start, end, midpoint);
    let between = layout
        .values()
        .copied()
        .filter(|node| start.x < node.x && node.x < end.x);
    if !between.clone().any(|node| {
        let bounds = Rect::from_center_size(node, NODE_SIZE).expand(12.0);
        points
            .windows(2)
            .any(|edge| Rect::from_two_pos(edge[0], edge[1]).intersects(bounds))
    }) {
        return midpoint;
    }
    let lower = start.y > end.y;
    let edge = if lower { f32::max } else { f32::min };
    let y = between.map(|node| node.y).fold(edge(start.y, end.y), edge);
    pos2(start.x + 18.0, y + if lower { 78.0 } else { -78.0 })
}

fn route_points(start: Pos2, end: Pos2, bend: Pos2) -> [Pos2; 7] {
    [
        start,
        start + vec2(18.0, 0.0),
        pos2(bend.x, start.y),
        bend,
        pos2(end.x - 18.0, bend.y),
        end - vec2(18.0, 0.0),
        end,
    ]
}

fn node_properties(
    ui: &mut Ui,
    language: Language,
    project: &Project,
    part: &Part,
    instrument_tag: &mut Option<String>,
    edit: &mut Edit,
) {
    let current = project.process_diagram.nodes.get(&part.id);
    let mut label = current
        .and_then(|node| node.label.clone())
        .unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(language.text("Diagram label"));
        if ui
            .add(
                egui::TextEdit::singleline(&mut label)
                    .id_salt(&part.id)
                    .hint_text(&part.name),
            )
            .changed()
        {
            let nodes = &mut edit.diagram(project).nodes;
            let node = nodes.entry(part.id.clone()).or_default();
            node.label = (!label.trim().is_empty()).then_some(label);
            if node.position.is_none() && node.label.is_none() {
                nodes.remove(&part.id);
            }
        }
        if current.is_some_and(|node| node.position.is_some())
            && ui.button(language.text("Auto-position symbol")).clicked()
        {
            let nodes = &mut edit.diagram(project).nodes;
            let node = nodes.entry(part.id.clone()).or_default();
            node.position = None;
            if node.label.is_none() {
                nodes.remove(&part.id);
            }
        }
    });

    let tags = project.tags();
    let prefix = format!("{}.", part.id);
    let measurements: Vec<_> = tags
        .range(prefix.clone()..)
        .take_while(|(id, _)| id.starts_with(&prefix))
        .filter(|(id, tag)| tag.direction == Direction::Sensor && Quantity::for_tag(id).is_some())
        .collect();
    if measurements.is_empty() {
        return;
    }
    if instrument_tag
        .as_ref()
        .is_none_or(|selected| !measurements.iter().any(|(id, _)| *id == selected))
    {
        *instrument_tag = Some(measurements[0].0.clone());
    }
    ui.push_id(&part.id, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(language.text("Instrumentation (GOST 21.208)"));
            let selected = instrument_tag.as_ref().unwrap();
            egui::ComboBox::from_id_salt("measurement")
                .selected_text(language.text(&tags[selected].name))
                .show_ui(ui, |ui| {
                    for (id, tag) in &measurements {
                        ui.selectable_value(
                            instrument_tag,
                            Some((*id).clone()),
                            language.text(&tag.name),
                        );
                    }
                });
            let id = instrument_tag.as_ref().unwrap();
            let Some(current) = project.process_diagram.instruments.get(id) else {
                if ui.button(language.text("Add symbol")).clicked() {
                    let number = project
                        .process_diagram
                        .instruments
                        .values()
                        .map(|instrument| instrument.number)
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1);
                    edit.diagram(project).instruments.insert(
                        id.clone(),
                        Instrument {
                            number,
                            function: InstrumentFunction::Indication,
                            output_tag: None,
                        },
                    );
                }
                return;
            };
            let mut instrument = current.clone();
            egui::ComboBox::from_id_salt("function")
                .selected_text(language.text(instrument.function.label()))
                .show_ui(ui, |ui| {
                    for function in [
                        InstrumentFunction::Indication,
                        InstrumentFunction::Transmission,
                        InstrumentFunction::Control,
                        InstrumentFunction::Alarm,
                    ] {
                        ui.selectable_value(
                            &mut instrument.function,
                            function,
                            language.text(function.label()),
                        );
                    }
                });
            ui.label(language.text("Position number"));
            ui.add(egui::DragValue::new(&mut instrument.number).range(1..=u32::MAX));
            if instrument.function == InstrumentFunction::Control {
                let output = instrument.output_tag.as_ref().map_or_else(
                    || language.text("No output signal").to_owned(),
                    |id| format!("{id} · {}", language.text(&tags[id].name)),
                );
                egui::ComboBox::from_id_salt("output")
                    .selected_text(output)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut instrument.output_tag,
                            None,
                            language.text("No output signal"),
                        );
                        for (output, tag) in &tags {
                            if tag.direction == Direction::Actuator
                                && let Some((owner, _)) = output.split_once('.')
                                && project
                                    .parts
                                    .iter()
                                    .any(|part| part.id == owner && has_water_ports(part))
                            {
                                ui.selectable_value(
                                    &mut instrument.output_tag,
                                    Some(output.clone()),
                                    format!("{output} · {}", language.text(&tag.name)),
                                );
                            }
                        }
                    })
                    .response
                    .on_hover_text(language.text("The signal documents an external control link."));
            } else {
                instrument.output_tag = None;
            }
            if ui.button(language.text("Remove symbol")).clicked() {
                edit.diagram(project).instruments.remove(id);
            } else if instrument != *current {
                edit.diagram(project)
                    .instruments
                    .insert(id.clone(), instrument);
            }
        });
    });
}

impl InstrumentFunction {
    fn label(self) -> &'static str {
        match self {
            Self::Indication => "Indication",
            Self::Transmission => "Transmission",
            Self::Control => "Control",
            Self::Alarm => "Alarm",
        }
    }

    fn letter(self) -> char {
        match self {
            Self::Indication => 'I',
            Self::Transmission => 'T',
            Self::Control => 'C',
            Self::Alarm => 'A',
        }
    }
}

impl Quantity {
    fn letter(self) -> char {
        match self {
            Self::Flow => 'F',
            Self::Level => 'L',
            Self::Pressure => 'P',
            Self::Temperature => 'T',
        }
    }

    fn unit(self, language: Language) -> &'static str {
        match (self, language) {
            (Self::Flow, Language::Russian) => "л/с",
            (Self::Flow, Language::English) => "L/s",
            (Self::Level, _) => "%",
            (Self::Pressure, Language::Russian) => "кПа",
            (Self::Pressure, Language::English) => "kPa",
            (Self::Temperature, _) => "°C",
        }
    }
}

fn draw_grid(painter: &Painter, rect: Rect, canvas: Canvas) {
    painter.rect_filled(rect, 0.0, BACKGROUND);
    let step = (GRID * canvas.scale).max(12.0);
    let mut x = rect.left() + (canvas.origin.x - rect.left()).rem_euclid(step);
    while x < rect.right() {
        painter.vline(x, rect.y_range(), Stroke::new(1.0, GRID_COLOR));
        x += step;
    }
    let mut y = rect.top() + (canvas.origin.y - rect.top()).rem_euclid(step);
    while y < rect.bottom() {
        painter.hline(rect.x_range(), y, Stroke::new(1.0, GRID_COLOR));
        y += step;
    }
}

fn draw_route(painter: &Painter, canvas: Canvas, route: &Route<'_>, selected: bool, flow: bool) {
    let points = route.points.map(|point| canvas.point(point));
    let color = if flow { ACCENT } else { MUTE };
    let stroke = Stroke::new(if selected { 2.8 } else { 1.65 }, color);
    painter.add(Shape::line(points.to_vec(), stroke));
    let tip = points[6] - vec2(2.0, 0.0);
    let size = (8.0 * canvas.scale).clamp(4.0, 12.0);
    painter.add(Shape::convex_polygon(
        vec![
            tip,
            tip + vec2(-size, -size * 0.49),
            tip + vec2(-size, size * 0.49),
        ],
        color,
        Stroke::NONE,
    ));
    if selected {
        painter.circle(
            canvas.point(route.bend),
            5.0,
            PANEL,
            Stroke::new(2.0, ACCENT),
        );
    }
}

fn pipe_flow(snapshot: Option<&Snapshot>, part: &Part, port: u8) -> bool {
    let role = match (part.kind.water_port_count(true), port) {
        (2, 0) => "outlet_1_l_s",
        (2, _) => "outlet_2_l_s",
        (_, _) => "flow_l_s",
    };
    snapshot
        .and_then(|snapshot| snapshot.tags.get(&format!("{}.{role}", part.id)))
        .is_some_and(|tag| {
            tag.quality == Quality::Good
                && matches!(tag.value, Value::Float(flow) if flow > 0.00001)
        })
}

fn draw_instruments(
    painter: &Painter,
    canvas: Canvas,
    sheet: &ProcessDiagram,
    snapshot: Option<&Snapshot>,
    layout: &BTreeMap<&str, Pos2>,
    language: Language,
) {
    let mut by_part = BTreeMap::<&str, Vec<_>>::new();
    for (tag, instrument) in &sheet.instruments {
        if let Some((part, _)) = tag.split_once('.')
            && let Some(quantity) = Quantity::for_tag(tag)
            && layout.contains_key(part)
        {
            by_part
                .entry(part)
                .or_default()
                .push((tag, instrument, quantity));
        }
    }
    let mut symbols = Vec::new();
    for (id, instruments) in by_part {
        let node = layout[id];
        for (index, (tag, instrument, quantity)) in instruments.into_iter().enumerate() {
            let offset = [-29.0, 29.0, -87.0, 87.0][index % 4];
            let center = node + vec2(offset, -57.0 - (index / 4) as f32 * 54.0);
            let attach = node + vec2(offset.clamp(-32.0, 32.0), -NODE_SIZE.y / 2.0);
            let stroke = Stroke::new(1.15, WARN);
            painter.line_segment([canvas.point(center), canvas.point(attach)], stroke);
            if let Some(target) = instrument
                .output_tag
                .as_deref()
                .and_then(|id| id.split_once('.'))
                .and_then(|(id, _)| layout.get(id))
            {
                let end = *target - vec2(0.0, NODE_SIZE.y / 2.0);
                let height = end.y.min(center.y) - 28.0;
                let points = [center, pos2(center.x, height), pos2(end.x, height), end]
                    .map(|point| canvas.point(point));
                painter.add(Shape::line(points.to_vec(), stroke));
                let tip = points[3];
                let size = (7.0 * canvas.scale).clamp(3.0, 10.0);
                painter.line_segment([tip, tip + vec2(-size * 0.48, -size)], stroke);
                painter.line_segment([tip, tip + vec2(size * 0.48, -size)], stroke);
            }
            symbols.push((center, tag.as_str(), instrument, quantity));
        }
    }
    for (center, tag, instrument, quantity) in symbols {
        let center = canvas.point(center);
        let radius = 17.0 * canvas.scale;
        painter.circle(center, radius, BACKGROUND, Stroke::new(1.3, WARN));
        painter.hline(
            (center.x - radius)..=(center.x + radius),
            center.y,
            Stroke::new(1.0, WARN),
        );
        if canvas.scale < 0.27 {
            continue;
        }
        let code = format!("{}{}", quantity.letter(), instrument.function.letter());
        painter.text(
            center - vec2(0.0, radius * 0.49),
            Align2::CENTER_CENTER,
            code,
            FontId::monospace((12.0 * canvas.scale).max(6.0)),
            WARN,
        );
        let number = instrument.number.to_string();
        painter.text(
            center + vec2(0.0, radius * 0.47),
            Align2::CENTER_CENTER,
            &number,
            FontId::monospace(
                (13.0 * canvas.scale / (number.len() as f32 / 3.0).max(1.0)).max(4.0),
            ),
            TEXT,
        );
        if let Some((value, color)) = live_value(snapshot, tag, language) {
            painter.text(
                center - vec2(0.0, radius * 1.1),
                Align2::CENTER_BOTTOM,
                value,
                FontId::monospace((11.0 * canvas.scale).max(7.0)),
                color,
            );
        }
    }
}

fn live_value(
    snapshot: Option<&Snapshot>,
    id: &str,
    language: Language,
) -> Option<(String, Color32)> {
    let (precision, unit) = match Quantity::for_tag(id) {
        Some(quantity) => (
            if quantity == Quantity::Flow { 3 } else { 1 },
            quantity.unit(language),
        ),
        None if id.ends_with(".opening_pct") => (0, "%"),
        None => return None,
    };
    let tag = snapshot?.tags.get(id)?;
    let Value::Float(value) = tag.value else {
        return None;
    };
    let formatted = format!("{value:.precision$} {unit}");
    Some(if tag.quality == Quality::Good {
        (formatted, ACCENT)
    } else {
        (format!("{} · {}", formatted, language.text("Stale")), WARN)
    })
}

fn draw_node(
    painter: &Painter,
    canvas: Canvas,
    center: Pos2,
    part: &Part,
    label: &str,
    selected: bool,
    snapshot: Option<&Snapshot>,
    language: Language,
) {
    let at = |x, y| canvas.point(center + vec2(x, y));
    let rect = Rect::from_center_size(canvas.point(center), NODE_SIZE * canvas.scale);
    let color = if selected { ACCENT } else { TEXT };
    let stroke = Stroke::new(1.6, color);
    painter.rect_filled(
        rect.shrink(1.0),
        4.0,
        if selected { PANEL } else { BACKGROUND },
    );
    if selected {
        painter.rect_stroke(rect, 4.0, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
    }

    let line = |points: &[(f32, f32)]| {
        painter.add(Shape::line(
            points.iter().map(|&(x, y)| at(x, y)).collect(),
            stroke,
        ));
    };
    let box_outline = |x: f32, y: f32, radius: f32| {
        painter.rect_stroke(
            Rect::from_two_pos(at(-x, -y), at(x, y)),
            radius,
            stroke,
            StrokeKind::Middle,
        );
    };
    for outlet in [false, true] {
        let count = part.kind.water_port_count(outlet);
        for index in 0..count {
            let side = if outlet { 1.0 } else { -1.0 };
            let y = (f32::from(index) - (f32::from(count) - 1.0) / 2.0) * 18.0;
            if matches!(part.kind, PartKind::WaterJunction { .. }) {
                line(&[(NODE_SIZE.x / 2.0 * side, y), (side * 13.0, y), (0.0, 0.0)]);
            } else {
                line(&[(NODE_SIZE.x / 2.0 * side, y), (side * 18.0, y)]);
            }
        }
    }
    match part.kind {
        PartKind::WaterSource { .. } => {
            line(&[(-17.0, -10.0), (18.0, 0.0), (-17.0, 10.0), (-17.0, -10.0)])
        }
        PartKind::WaterDrain { .. } => {
            line(&[
                (-18.0, 0.0),
                (0.0, 0.0),
                (0.0, 10.0),
                (-12.0, 10.0),
                (12.0, 10.0),
            ]);
            line(&[(-8.0, 15.0), (8.0, 15.0)]);
        }
        PartKind::WaterPump { .. } => {
            painter.circle_stroke(at(0.0, 0.0), 18.0 * canvas.scale, stroke);
            line(&[(-9.0, -9.0), (12.0, 0.0), (-9.0, 9.0), (-9.0, -9.0)]);
        }
        PartKind::WaterValve { .. } => {
            line(&[
                (-18.0, -11.0),
                (18.0, 11.0),
                (18.0, -11.0),
                (-18.0, 11.0),
                (-18.0, -11.0),
            ]);
            line(&[(0.0, 0.0), (0.0, -16.0), (-8.0, -16.0), (8.0, -16.0)]);
        }
        PartKind::HeatedTank(_) => {
            box_outline(18.0, 21.0, 6.0 * canvas.scale);
            line(&[(-13.0, 4.0), (13.0, 4.0)]);
        }
        PartKind::WaterJunction { .. } => {
            painter.circle_filled(at(0.0, 0.0), (3.0 * canvas.scale).max(1.5), color);
        }
        PartKind::WaterInstrument { measurement, .. } => {
            let quantity = match measurement {
                WaterMeasurement::Flow => Quantity::Flow,
                WaterMeasurement::Pressure => Quantity::Pressure,
                WaterMeasurement::Temperature => Quantity::Temperature,
            };
            painter.circle_stroke(at(0.0, 0.0), 18.0 * canvas.scale, stroke);
            if canvas.scale >= 0.27 {
                painter.text(
                    at(0.0, 0.0),
                    Align2::CENTER_CENTER,
                    quantity.letter(),
                    FontId::monospace((16.0 * canvas.scale).max(7.0)),
                    color,
                );
            }
        }
        PartKind::WaterFilter { .. } => {
            line(&[
                (-18.0, 0.0),
                (0.0, -15.0),
                (18.0, 0.0),
                (0.0, 15.0),
                (-18.0, 0.0),
            ]);
            line(&[(0.0, -14.0), (0.0, 14.0)]);
        }
        PartKind::WaterHeater { .. } => {
            box_outline(18.0, 14.0, 0.0);
            line(&[
                (-13.0, 5.0),
                (-7.0, -5.0),
                (0.0, 5.0),
                (7.0, -5.0),
                (13.0, 5.0),
            ]);
        }
        PartKind::WaterHeatExchanger { .. } => {
            box_outline(18.0, 17.0, 4.0 * canvas.scale);
            line(&[(-18.0, -9.0), (18.0, -9.0)]);
            line(&[(-18.0, 9.0), (18.0, 9.0)]);
            line(&[(-7.0, 4.0), (-2.0, -4.0), (2.0, 4.0), (7.0, -4.0)]);
        }
        PartKind::WaterFiller { .. } => {
            line(&[
                (-18.0, 0.0),
                (0.0, 0.0),
                (0.0, 6.0),
                (-8.0, 15.0),
                (8.0, 15.0),
                (0.0, 6.0),
            ]);
        }
        _ => {}
    }
    if canvas.scale < 0.26 {
        return;
    }
    let label_size = (13.0 * canvas.scale).max(8.0);
    let pad = (COLUMN * 0.46 * canvas.scale).max(10.0);
    let label_center = at(0.0, 32.0);
    let galley = painter.layout(
        label.to_owned(),
        FontId::proportional(label_size),
        color,
        pad * 2.0,
    );
    let clip = Rect::from_min_max(
        label_center - vec2(pad, 0.0),
        label_center + vec2(pad, label_size * 2.25),
    );
    painter.with_clip_rect(clip).galley(
        label_center - vec2(galley.size().x * 0.5, 0.0),
        galley,
        color,
    );
    let role = match part.kind {
        PartKind::HeatedTank(_) => "level_pct",
        PartKind::WaterValve { .. } => "opening_pct",
        PartKind::WaterInstrument {
            measurement: WaterMeasurement::Temperature,
            ..
        } => "temperature_c",
        PartKind::WaterInstrument {
            measurement: WaterMeasurement::Pressure,
            ..
        } => "pressure_kpa",
        _ => "flow_l_s",
    };
    if let Some((value, color)) = live_value(snapshot, &format!("{}.{role}", part.id), language) {
        painter.text(
            at(0.0, 32.0) + vec2(0.0, label_size * 2.05),
            Align2::CENTER_TOP,
            value,
            FontId::monospace((11.5 * canvas.scale).max(7.0)),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_a_symbol_changes_only_the_sheet() -> anyhow::Result<()> {
        let project: Project =
            serde_json::from_str(include_str!("../../examples/heated-tank.json"))?;
        let ctx = egui::Context::default();
        let mut view = View::default();
        let frame = |view: &mut View, events| {
            let mut edit = Edit::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1300.0, 560.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    edit = view.show(ui, &project, None, None, true, Language::English);
                },
            );
            let canvas = output.shapes.iter().find_map(|item| match &item.shape {
                Shape::Rect(rect) if rect.fill == BACKGROUND && rect.rect.width() > 1000.0 => {
                    Some(rect.rect)
                }
                _ => None,
            });
            output.textures_delta.clear();
            (canvas.unwrap(), edit)
        };
        let (rect, edit) = frame(&mut view, vec![]);
        assert!(edit.diagram.is_none());
        let parts = project
            .parts
            .iter()
            .map(|part| (part.id.as_str(), part))
            .collect();
        let tank = positions(&project, &parts)["tank"];
        let canvas = Canvas {
            origin: rect.center() + view.pan,
            scale: view.scale,
        };
        let from = canvas.point(tank);
        let to = from + vec2(30.0, 25.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut view, vec![egui::Event::PointerMoved(from)]);
        frame(&mut view, vec![button(from, true)]);
        let (_, edit) = frame(&mut view, vec![egui::Event::PointerMoved(to)]);
        assert_eq!(edit.selection, Some(Some("tank".to_owned())));
        assert!(edit.diagram.is_none());
        let (_, edit) = frame(&mut view, vec![button(to, false)]);
        let diagram = edit
            .diagram
            .expect("releasing the symbol saves its position");
        let position = as_pos(diagram.nodes["tank"].position.unwrap());
        assert!(position.distance(canvas.unproject(to)) < 0.01);
        assert!(edit.pipes.is_none());
        Ok(())
    }
}
