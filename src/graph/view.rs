//! The interactive graph widget. In *trail* mode (Sourcetrail style) the
//! canvas shows the selected symbol's trail (who uses it on the left, what
//! it uses on the right), or a per-file overview when nothing is selected.
//! In *call hierarchy* mode it shows who calls whom, top-down from the
//! selected function, or from the program's entry points.
//!
//! Controls:
//! - click any symbol (pill or class title) to make it the active symbol;
//!   double-click it to make it the root of the view, framed afresh
//! - click an entry of the legend to hide (or show again) that kind of
//!   symbol or relation
//! - right-click a symbol for what can be done with it
//! - "Back" returns to the previous symbol; "Overview" clears the selection
//! - scroll or pinch: zoom around the pointer; drag: pan
//! - double-click the background, or "Fit": frame everything

use std::collections::HashSet;

use egui::{Pos2, Response, Sense, Vec2};

use super::camera::Camera;
use super::hierarchy::{all_entry_points, build_hierarchy, CallIndex};
use super::overview::build_overview;
use super::render::{self, font_for, Badge, Frame, Legend, Palette, LEGEND_STRIP_HEIGHT};
use super::scene::{LegendItem, Scene, TextRole};
use super::trail::build_trail;
use crate::app::fonts::control;
use crate::model::{CodeGraph, SymbolId};

/// Extra world-space margin around the scene when fitting it to the view.
const FIT_PADDING: f32 = 40.0;
/// Zoom change per scrolled point: `factor = exp(scroll * ZOOM_PER_SCROLL_POINT)`.
const ZOOM_PER_SCROLL_POINT: f32 = 0.0015;
const BUTTON_ZOOM_STEP: f32 = 1.25;
/// Framing a new scene never zooms out further than this; bigger scenes are
/// shown around the selection instead, to pan from. "Fit" frames everything.
const READABLE_ZOOM: f32 = 0.5;
/// Back-navigation depth kept.
const MAX_HISTORY: usize = 100;

/// What the right-click menu of a symbol asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeAction {
    /// Make it the root of the view.
    Explore(SymbolId),
    ShowDocs(SymbolId),
    FindLike(SymbolId),
    CrossReferences(SymbolId),
    ToggleBookmark(SymbolId),
    EditNote(SymbolId),
    OpenInEditor(SymbolId),
}

enum ToolbarAction {
    Back,
    Overview,
    ZoomIn,
    ZoomOut,
    Fit,
    Mode(ViewMode),
}

/// What the canvas lays out for the selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Trail,
    CallHierarchy,
}

impl ViewMode {
    fn label(self) -> &'static str {
        match self {
            ViewMode::Trail => "Trail",
            ViewMode::CallHierarchy => "Call hierarchy",
        }
    }
}

/// The selection and mode a scene was laid out for.
type SceneKey = (Option<SymbolId>, ViewMode);

/// Where "Go to Entry Point" landed: entry point `position` of `count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryStep {
    pub id: SymbolId,
    pub position: usize,
    pub count: usize,
}

pub struct GraphView {
    /// Laid-out scene and what it was built for.
    scene: Option<(SceneKey, Scene)>,
    mode: ViewMode,
    camera: Camera,
    /// Keep the scene framed (e.g. across window resizes) until the user
    /// pans or zooms manually.
    is_auto_fit: bool,
    /// Frame the whole scene even if that makes it hard to read ("Fit").
    is_fit_all: bool,
    /// Call graph of the project, built on first use of the call hierarchy.
    /// A new project gets a new `GraphView`, so this never goes stale.
    call_index: Option<CallIndex>,
    /// Every entry point, best first, built on first use (same lifetime
    /// as `call_index`).
    entry_points: Option<Vec<SymbolId>>,
    selected: Option<SymbolId>,
    history: Vec<SymbolId>,
    hovered: Option<SymbolId>,
    /// Legend entries switched off.
    hidden: HashSet<LegendItem>,
    /// Where the legend entries were last drawn, in screen coordinates.
    legend: Legend,
    /// The symbol whose right-click menu is open.
    menu_target: Option<SymbolId>,
    /// What the menu asked for, until the app takes it.
    pending: Option<NodeAction>,
    /// Connection count for the hovered symbol, so hovering doesn't scan
    /// every edge on every frame.
    hover_connections: Option<(SymbolId, usize)>,
    /// Explanation shown in the toolbar (computed with the last scene).
    notice: Option<String>,
}

impl Default for GraphView {
    fn default() -> Self {
        Self {
            scene: None,
            mode: ViewMode::default(),
            camera: Camera::default(),
            is_auto_fit: true,
            is_fit_all: false,
            call_index: None,
            entry_points: None,
            selected: None,
            history: Vec::new(),
            hovered: None,
            hidden: HashSet::new(),
            legend: Legend {
                items: Vec::new(),
                height: LEGEND_STRIP_HEIGHT,
            },
            menu_target: None,
            pending: None,
            hover_connections: None,
            notice: None,
        }
    }
}

impl GraphView {
    pub fn selected(&self) -> Option<SymbolId> {
        self.selected
    }

    pub fn with_mode(mode: ViewMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    /// Lays the scene out and frames it afresh, as for a new selection.
    pub fn relayout(&mut self) {
        self.scene = None;
        self.is_auto_fit = true;
        self.is_fit_all = false;
    }

    /// What the right-click menu asked for since last taken.
    pub fn take_action(&mut self) -> Option<NodeAction> {
        self.pending.take()
    }

    pub fn mode(&self) -> ViewMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ViewMode) {
        self.mode = mode;
    }

    /// Returns to the previous symbol, if any.
    pub fn back(&mut self) {
        if !self.history.is_empty() {
            self.selected = self.history.pop();
        }
    }

    /// Clears the selection (overview, or entry points in call hierarchy).
    pub fn show_overview(&mut self) {
        if let Some(previous) = self.selected.take() {
            self.history.push(previous);
        }
    }

    /// Frames the whole scene again, however small that makes it.
    pub fn fit(&mut self) {
        self.is_auto_fit = true;
        self.is_fit_all = true;
    }

    /// Every entry point of the project, best first: `main` and other
    /// program entry functions, then functions nobody calls.
    pub fn entry_points(&mut self, graph: &CodeGraph) -> &[SymbolId] {
        if self.entry_points.is_none() {
            let calls = self.call_index.get_or_insert_with(|| CallIndex::new(graph));
            self.entry_points = Some(all_entry_points(graph, calls));
        }
        self.entry_points.as_deref().unwrap_or_default()
    }

    /// Selects the entry point after the selected one (the first, if the
    /// selection isn't an entry point), so repeating it steps through all.
    pub fn go_to_entry_point(&mut self, graph: &CodeGraph) -> Option<EntryStep> {
        let current = self.selected;
        let list = self.entry_points(graph);
        let position = next_in_cycle(list, current)?;
        let step = EntryStep {
            id: list[position],
            position,
            count: list.len(),
        };
        self.select(step.id);
        Some(step)
    }

    /// Make `id` the active symbol, remembering the previous one for "Back".
    pub fn select(&mut self, id: SymbolId) {
        if self.selected == Some(id) {
            return;
        }
        if let Some(previous) = self.selected {
            self.history.push(previous);
            if self.history.len() > MAX_HISTORY {
                self.history.remove(0);
            }
        }
        self.selected = Some(id);
    }

    /// Draws the graph; `badge_of` tells which symbols have notes.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        graph: &CodeGraph,
        badge_of: &dyn Fn(SymbolId) -> Option<Badge>,
    ) -> Response {
        let action = self.show_toolbar(ui);
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let over_legend = response.hover_pos().and_then(|p| self.legend_item_at(p));
        if let Some(item) = over_legend.filter(|_| response.clicked()) {
            self.toggle_legend(item);
            ui.ctx().request_repaint();
        }
        let pointer_local = response
            .hover_pos()
            .filter(|_| over_legend.is_none())
            .map(|p| (p - rect.min).to_pos2());

        let scene = self.take_scene(ui, graph);
        let before = self.selected;
        if over_legend.is_some() {
            self.hovered = None;
        } else {
            self.handle_pointer(&response, &scene, pointer_local);
            self.show_node_menu(&response);
        }
        if self.selected != before {
            // Nothing else changes on screen for a click, so no frame
            // would follow to lay out and show the new selection.
            ui.ctx().request_repaint();
        }
        self.apply_toolbar_action(action, rect.size());
        let fit_area = rect.size() - Vec2::new(0.0, self.legend.height);
        self.update_camera(&scene, fit_area);
        self.handle_scroll_zoom(ui, &response, pointer_local);

        let painter = ui.painter_at(rect);
        let frame = Frame {
            scene: &scene,
            camera: self.camera,
            origin: rect.min,
            hovered: self.hovered,
            palette: Palette::for_visuals(ui.visuals()),
            badge_of,
            hidden: &self.hidden,
        };
        let legend = render::paint(&painter, &frame);
        if legend.height != self.legend.height {
            ui.ctx().request_repaint();
        }
        self.legend = legend;
        self.notice = notice(&scene, graph, self.mode);
        self.scene = Some(((self.selected, self.mode), scene));

        self.with_hover_tooltip(response, graph)
    }

    /// The scene for the current selection, rebuilt (and re-fitted) when the
    /// selection changed since it was laid out.
    fn take_scene(&mut self, ui: &egui::Ui, graph: &CodeGraph) -> Scene {
        if let Some((built_for, scene)) = self.scene.take() {
            if built_for == (self.selected, self.mode) {
                return scene;
            }
        }
        let painter = ui.painter();
        let measure = |text: &str, role: TextRole| {
            let galley =
                painter.layout_no_wrap(text.to_owned(), font_for(role), egui::Color32::WHITE);
            galley.size().x
        };
        self.is_auto_fit = true;
        self.is_fit_all = false;
        self.hover_connections = None;
        let is_trail = self.mode == ViewMode::Trail && self.selected.is_some();
        let mut scene = match (self.mode, self.selected) {
            (ViewMode::CallHierarchy, focus) => {
                let calls = self.call_index.get_or_insert_with(|| CallIndex::new(graph));
                build_hierarchy(graph, calls, focus, &measure)
            }
            (ViewMode::Trail, Some(id)) => build_trail(graph, id, &self.hidden, &measure),
            (ViewMode::Trail, None) => build_overview(graph, &measure),
        };
        // The trail already leaves out what is switched off.
        if !is_trail {
            leave_out(&mut scene, &self.hidden);
        }
        scene
    }

    fn legend_item_at(&self, pos: Pos2) -> Option<LegendItem> {
        self.legend
            .items
            .iter()
            .find(|(_, rect)| rect.contains(pos))
            .map(|(item, _)| *item)
    }

    /// Hides `item`'s symbols or relations, or shows them again.
    fn toggle_legend(&mut self, item: LegendItem) {
        if !self.hidden.remove(&item) {
            self.hidden.insert(item);
        }
        self.scene = None;
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui) -> Option<ToolbarAction> {
        ui.horizontal(|ui| {
            let mut action = None;
            for mode in [ViewMode::Trail, ViewMode::CallHierarchy] {
                let label = control(mode.label());
                if ui.selectable_label(self.mode == mode, label).clicked() {
                    action = Some(ToolbarAction::Mode(mode));
                }
            }
            ui.separator();
            let back = egui::Button::new(control("◀ Back"));
            if ui.add_enabled(!self.history.is_empty(), back).clicked() {
                action = Some(ToolbarAction::Back);
            }
            let overview_label = match self.mode {
                ViewMode::Trail => "Overview",
                ViewMode::CallHierarchy => "Entry points",
            };
            let overview = egui::Button::new(control(overview_label));
            if ui.add_enabled(self.selected.is_some(), overview).clicked() {
                action = Some(ToolbarAction::Overview);
            }
            ui.separator();
            if ui.button(control("−")).on_hover_text("Zoom out").clicked() {
                action = Some(ToolbarAction::ZoomOut);
            }
            ui.label(format!("{:>3.0}%", self.camera.zoom * 100.0));
            if ui.button(control("+")).on_hover_text("Zoom in").clicked() {
                action = Some(ToolbarAction::ZoomIn);
            }
            let fit = ui.button(control("Fit")).on_hover_text("Frame everything");
            if fit.clicked() {
                action = Some(ToolbarAction::Fit);
            }
            ui.separator();
            ui.weak("click: explore · scroll: zoom · drag: pan · double-click: fit");
            if let Some(notice) = &self.notice {
                ui.separator();
                ui.label(egui::RichText::new(notice).italics());
            }
            action
        })
        .inner
    }

    fn apply_toolbar_action(&mut self, action: Option<ToolbarAction>, canvas_size: Vec2) {
        let center = (canvas_size * 0.5).to_pos2();
        let zoom_factor = match action {
            Some(ToolbarAction::Back) => return self.back(),
            Some(ToolbarAction::Overview) => return self.show_overview(),
            Some(ToolbarAction::Fit) => return self.fit(),
            Some(ToolbarAction::Mode(mode)) => return self.set_mode(mode),
            Some(ToolbarAction::ZoomIn) => BUTTON_ZOOM_STEP,
            Some(ToolbarAction::ZoomOut) => 1.0 / BUTTON_ZOOM_STEP,
            None => return,
        };
        self.camera = self.camera.zoomed_at(zoom_factor, center);
        self.is_auto_fit = false;
    }

    /// Hover, background panning, and click-to-explore.
    fn handle_pointer(&mut self, response: &Response, scene: &Scene, pointer_local: Option<Pos2>) {
        self.hovered = pointer_local
            .map(|p| self.camera.local_to_world(p))
            .and_then(|world| scene.hit(world));

        if response.dragged() {
            self.camera = self.camera.panned(response.drag_delta());
            self.is_auto_fit = false;
        }
        if response.double_clicked() {
            match self.hovered {
                // The first click selected it; the app navigates to it.
                Some(id) => self.pending = Some(NodeAction::Explore(id)),
                None => self.fit(),
            }
        } else if response.clicked() {
            if let Some(id) = self.hovered {
                self.select(id);
            }
        }
    }

    fn show_node_menu(&mut self, response: &Response) {
        if response.secondary_clicked() {
            self.menu_target = self.hovered;
            if let Some(id) = self.menu_target {
                self.select(id);
            }
        }
        let Some(id) = self.menu_target else {
            return;
        };
        let mut chosen = None;
        let items: [(&str, fn(SymbolId) -> NodeAction); 7] = [
            ("Explore from Here", NodeAction::Explore),
            ("Show Docs", NodeAction::ShowDocs),
            ("Find Code Like This", NodeAction::FindLike),
            ("Cross References", NodeAction::CrossReferences),
            ("Toggle Bookmark", NodeAction::ToggleBookmark),
            ("Note…", NodeAction::EditNote),
            ("Open in Editor", NodeAction::OpenInEditor),
        ];
        response.clone().context_menu(|ui| {
            for (label, action) in items {
                if ui.button(label).clicked() {
                    chosen = Some(action(id));
                    ui.close();
                }
            }
        });
        if chosen.is_some() {
            self.pending = chosen;
        }
    }

    fn update_camera(&mut self, scene: &Scene, canvas_size: Vec2) {
        if canvas_size.x <= 0.0 || canvas_size.y <= 0.0 || !self.is_auto_fit {
            return;
        }
        if let Some(bounds) = scene.bounds() {
            let focus = scene
                .active
                .and_then(|id| scene.anchor(id))
                .map(|r| r.center());
            let padded = bounds.expand(FIT_PADDING);
            self.camera = if self.is_fit_all {
                Camera::fit(padded, canvas_size)
            } else {
                Camera::fit_readable(padded, canvas_size, focus, READABLE_ZOOM)
            };
        }
    }

    fn handle_scroll_zoom(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        pointer_local: Option<Pos2>,
    ) {
        let Some(anchor) = pointer_local.filter(|_| response.hovered()) else {
            return;
        };
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = pinch * (scroll * ZOOM_PER_SCROLL_POINT).exp();
        if (factor - 1.0).abs() > f32::EPSILON {
            self.camera = self.camera.zoomed_at(factor, anchor);
            self.is_auto_fit = false;
        }
    }

    fn with_hover_tooltip(&mut self, response: Response, graph: &CodeGraph) -> Response {
        let Some(symbol) = self.hovered.and_then(|id| graph.symbols.get(&id)) else {
            return response;
        };
        let connections = match self.hover_connections {
            Some((id, count)) if id == symbol.id => count,
            _ => {
                let count = graph.edges_for(symbol.id).count();
                self.hover_connections = Some((symbol.id, count));
                count
            }
        };
        response.on_hover_ui_at_pointer(|ui| {
            ui.strong(&symbol.qualified_name);
            ui.label(format!(
                "{} · {}:{}",
                symbol.kind.label(),
                symbol.file,
                symbol.line
            ));
            ui.weak(format!("{connections} connection(s) · click to explore"));
        })
    }
}

/// Text overlaid on the canvas when the view needs explaining.
fn notice(scene: &Scene, graph: &CodeGraph, mode: ViewMode) -> Option<String> {
    if graph.symbols.is_empty() {
        return Some("No symbols — open a project with File > Open Project…".into());
    }
    if mode == ViewMode::CallHierarchy {
        return hierarchy_notice(scene);
    }
    match (scene.active, scene.hidden_neighbors) {
        (Some(_), 0) => None,
        (Some(_), hidden) => Some(format!("{hidden} more related symbols not shown")),
        (None, 0) => Some("Click a symbol to explore its relationships".into()),
        (None, hidden) => Some(format!(
            "Click a symbol to explore · {hidden} symbols in more files not shown"
        )),
    }
}

fn hierarchy_notice(scene: &Scene) -> Option<String> {
    let hidden = match scene.hidden_neighbors {
        0 => String::new(),
        n => format!(" · {n} more not shown"),
    };
    match (scene.active, scene.pills.is_empty()) {
        (Some(_), true) => Some("No calls from here — select a function or class".into()),
        (None, true) => Some("No calls found in this project".into()),
        (Some(_), false) => (!hidden.is_empty()).then(|| format!("Calls made from here{hidden}")),
        (None, false) => Some(format!("Calls from the program's entry points{hidden}")),
    }
}

/// Takes what the legend has switched off out of `scene`, and what is then
/// no longer connected to the active symbol. The active symbol stays,
/// whatever its kind.
fn leave_out(scene: &mut Scene, hidden: &HashSet<LegendItem>) {
    if hidden.is_empty() {
        return;
    }
    let active = scene.active;
    let is_off = |kind| hidden.contains(&LegendItem::of_symbol(kind));
    scene
        .pills
        .retain(|p| Some(p.id) == active || !is_off(p.kind));
    scene
        .groups
        .retain(|g| (g.id.is_some() && g.id == active) || !g.kind.is_some_and(is_off));
    for group in &mut scene.groups {
        group
            .pills
            .retain(|p| Some(p.id) == active || !is_off(p.kind));
    }
    let drawable = |scene: &Scene, w: &super::scene::Wire| {
        !hidden.contains(&LegendItem::Relation(w.kind))
            && scene.anchor(w.from).is_some()
            && scene.anchor(w.to).is_some()
    };
    let wires = std::mem::take(&mut scene.wires);
    scene.wires = wires.into_iter().filter(|w| drawable(scene, w)).collect();
    let secondary = std::mem::take(&mut scene.secondary_wires);
    scene.secondary_wires = secondary
        .into_iter()
        .filter(|w| drawable(scene, w))
        .collect();
    if let Some(active) = active {
        drop_unconnected(scene, active);
    }
}

/// Drops what has no chain of wires to `active`. A group's pills count as
/// the group.
fn drop_unconnected(scene: &mut Scene, active: SymbolId) {
    let unit = |scene: &Scene, id: SymbolId| -> Vec<SymbolId> {
        let group = scene
            .groups
            .iter()
            .find(|g| g.id == Some(id) || g.pills.iter().any(|p| p.id == id));
        match group {
            Some(g) => {
                g.id.into_iter()
                    .chain(g.pills.iter().map(|p| p.id))
                    .collect()
            }
            None => vec![id],
        }
    };
    let mut reached: HashSet<SymbolId> = unit(scene, active).into_iter().collect();
    loop {
        let before = reached.len();
        for wire in &scene.wires {
            for (near, far) in [(wire.from, wire.to), (wire.to, wire.from)] {
                if reached.contains(&near) {
                    reached.extend(unit(scene, far));
                }
            }
        }
        if reached.len() == before {
            break;
        }
    }
    scene.pills.retain(|p| reached.contains(&p.id));
    scene.groups.retain(|g| {
        g.id.is_some_and(|id| reached.contains(&id))
            || g.pills.iter().any(|p| reached.contains(&p.id))
    });
    scene
        .wires
        .retain(|w| reached.contains(&w.from) && reached.contains(&w.to));
    scene
        .secondary_wires
        .retain(|w| reached.contains(&w.from) && reached.contains(&w.to));
}

/// Index of the item after `current` in `list`, wrapping; the first item
/// if `current` isn't in it; `None` for an empty list.
fn next_in_cycle(list: &[SymbolId], current: Option<SymbolId>) -> Option<usize> {
    if list.is_empty() {
        return None;
    }
    let after_current = current
        .and_then(|c| list.iter().position(|id| *id == c))
        .map_or(0, |i| (i + 1) % list.len());
    Some(after_current)
}

#[cfg(test)]
mod tests {
    use egui::{Rect, Vec2};

    use super::*;
    use crate::graph::scene::Pill;
    use crate::model::SymbolKind;

    /// Two pills 4000 × 3000 apart: far too big to read when fitted.
    fn big_scene() -> Scene {
        let pill = |id, x, y| Pill {
            id,
            label: format!("f{id}"),
            kind: SymbolKind::Function,
            rect: Rect::from_min_size(Pos2::new(x, y), Vec2::new(80.0, 22.0)),
        };
        Scene {
            pills: vec![pill(1, 0.0, 0.0), pill(2, 4000.0, 3000.0)],
            ..Scene::default()
        }
    }

    const CANVAS: Vec2 = Vec2::new(400.0, 300.0);

    #[test]
    fn new_scenes_open_at_a_readable_zoom_but_fit_frames_everything() {
        let mut view = GraphView::default();
        view.update_camera(&big_scene(), CANVAS);
        assert!((view.camera.zoom - READABLE_ZOOM).abs() < 1e-3);

        view.fit();
        view.update_camera(&big_scene(), CANVAS);
        assert!(view.camera.zoom < READABLE_ZOOM);
    }

    #[test]
    fn manual_zoom_stops_auto_fit() {
        let mut view = GraphView::default();
        view.apply_toolbar_action(Some(ToolbarAction::ZoomIn), CANVAS);
        let zoomed = view.camera;
        view.update_camera(&big_scene(), CANVAS);
        assert_eq!(view.camera, zoomed);
    }

    #[test]
    fn back_and_overview_walk_the_history() {
        let mut view = GraphView::default();
        view.select(1);
        view.select(2);
        view.back();
        assert_eq!(view.selected(), Some(1));
        view.show_overview();
        assert_eq!(view.selected(), None);
        view.back();
        assert_eq!(view.selected(), Some(1));
        view.back(); // nothing left: stays put
        assert_eq!(view.selected(), Some(1));
    }

    #[test]
    fn entry_point_cycle_starts_at_the_first_and_wraps() {
        let list = [7, 8, 9];
        assert_eq!(next_in_cycle(&list, None), Some(0));
        assert_eq!(next_in_cycle(&list, Some(42)), Some(0));
        assert_eq!(next_in_cycle(&list, Some(7)), Some(1));
        assert_eq!(next_in_cycle(&list, Some(9)), Some(0));
        assert_eq!(next_in_cycle(&[], Some(7)), None);
    }

    #[test]
    fn go_to_entry_point_selects_main_and_reports_position() {
        let graph = crate::demo::demo_codebase().expect("demo");
        let mut view = GraphView::default();
        let step = view.go_to_entry_point(&graph).expect("demo has main");
        assert_eq!(graph.symbols[&step.id].qualified_name, "main");
        assert_eq!(view.selected(), Some(step.id));
        assert_eq!(step.position, 0);
        assert_eq!(step.count, view.entry_points(&graph).len());
    }

    #[test]
    fn go_to_entry_point_on_an_empty_graph_selects_nothing() {
        let mut view = GraphView::default();
        assert_eq!(view.go_to_entry_point(&CodeGraph::default()), None);
        assert_eq!(view.selected(), None);
    }

    #[test]
    fn mode_is_kept_by_with_mode_and_switchable() {
        let mut view = GraphView::with_mode(ViewMode::CallHierarchy);
        assert_eq!(view.mode(), ViewMode::CallHierarchy);
        view.apply_toolbar_action(Some(ToolbarAction::Mode(ViewMode::Trail)), CANVAS);
        assert_eq!(view.mode(), ViewMode::Trail);
    }

    #[test]
    fn hierarchy_notice_explains_empty_and_truncated_views() {
        let empty = Scene::default();
        assert_eq!(
            hierarchy_notice(&Scene {
                active: Some(1),
                ..empty.clone()
            })
            .as_deref(),
            Some("No calls from here — select a function or class")
        );
        let truncated = Scene {
            hidden_neighbors: 4,
            ..big_scene()
        };
        let notice = hierarchy_notice(&truncated).expect("notice");
        assert!(notice.contains("entry points") && notice.contains("4 more"));
    }
}
