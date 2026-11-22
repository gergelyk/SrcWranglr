//! The file explorer on the left: the project's source files as a folder
//! tree. Clicking a file shows it in the source pane.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::pane_bar::{self, Edge};
use crate::model::CodeGraph;

/// What the user asked for in the explorer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAction {
    /// Show this file.
    Open(String),
    Hide,
}

/// A folder: its subfolders and the files in it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    /// `(name, path)` of each file.
    files: Vec<(String, String)>,
}

impl Dir {
    fn of(paths: impl IntoIterator<Item = String>) -> Self {
        let mut root = Dir::default();
        for path in paths {
            let mut dir = &mut root;
            let mut parts = path.split('/').filter(|p| !p.is_empty()).peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_some() {
                    dir = dir.dirs.entry(part.to_string()).or_default();
                } else {
                    dir.files.push((part.to_string(), path.clone()));
                }
            }
        }
        root.sort();
        root
    }

    fn sort(&mut self) {
        self.files.sort();
        self.files.dedup();
        self.dirs.values_mut().for_each(Dir::sort);
    }

    fn count(&self) -> usize {
        self.files.len() + self.dirs.values().map(Dir::count).sum::<usize>()
    }
}

/// The files the index knows of: those with symbols or places in them.
fn files_of(graph: &CodeGraph) -> Vec<String> {
    let mut files: Vec<String> = graph
        .symbols
        .values()
        .map(|s| s.file.clone())
        .chain(graph.occurrences.keys().cloned())
        .filter(|f| !f.is_empty())
        .collect();
    files.sort();
    files.dedup();
    files
}

#[derive(Default)]
pub struct FileExplorer {
    /// The tree of the graph it was built from.
    tree: Option<(Arc<CodeGraph>, Dir)>,
    /// The file the folders were last opened up to.
    revealed: Option<String>,
}

impl FileExplorer {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        graph: &Arc<CodeGraph>,
        current: Option<&str>,
    ) -> Option<FileAction> {
        if self
            .tree
            .as_ref()
            .is_none_or(|(g, _)| !Arc::ptr_eq(g, graph))
        {
            self.tree = Some((graph.clone(), Dir::of(files_of(graph))));
            self.revealed = None;
        }
        let Some((_, tree)) = self.tree.as_ref() else {
            return None;
        };
        let should_hide = pane_bar::heading(ui, format!("Files ({})", tree.count()), Edge::Left);
        ui.separator();
        // Open the folders above the current file once it changes.
        let reveal = current.filter(|c| self.revealed.as_deref() != Some(*c));
        let mut clicked = None;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                show_dir(ui, tree, current, reveal, 0, &mut clicked);
            });
        if let Some(file) = reveal {
            self.revealed = Some(file.to_string());
        }
        match (should_hide, clicked) {
            (true, _) => Some(FileAction::Hide),
            (false, Some(file)) => Some(FileAction::Open(file)),
            (false, None) => None,
        }
    }
}

fn show_dir(
    ui: &mut egui::Ui,
    dir: &Dir,
    current: Option<&str>,
    reveal: Option<&str>,
    depth: usize,
    clicked: &mut Option<String>,
) {
    for (name, sub) in &dir.dirs {
        let mut header = egui::CollapsingHeader::new(name.as_str())
            .id_salt((depth, name, sub.files.first()))
            .icon(|ui, openness, response| {
                let rect = icon_rect(ui, response.rect);
                paint_folder(ui.painter(), rect, openness > 0.5);
            })
            .default_open(false);
        if reveal.is_some_and(|file| holds(sub, file)) {
            header = header.open(Some(true));
        }
        header.show(ui, |ui| {
            show_dir(ui, sub, current, reveal, depth + 1, clicked);
        });
    }
    for (name, path) in &dir.files {
        let is_current = current == Some(path.as_str());
        let row = ui
            .horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(ICON_SIZE, egui::Sense::hover());
                paint_file(ui.painter(), rect, name);
                ui.selectable_label(is_current, name.as_str())
            })
            .inner;
        if row.clicked() {
            *clicked = Some(path.clone());
        }
        if is_current && reveal == current {
            row.scroll_to_me(None);
        }
    }
}

const ICON_SIZE: egui::Vec2 = egui::vec2(14.0, 14.0);

/// The icon's square, centered in the arrow's place.
fn icon_rect(ui: &egui::Ui, slot: egui::Rect) -> egui::Rect {
    let _ = ui;
    egui::Rect::from_center_size(slot.center(), ICON_SIZE)
}

fn paint_folder(painter: &egui::Painter, rect: egui::Rect, is_open: bool) {
    let color = egui::Color32::from_rgb(0xE0, 0xB0, 0x4A);
    let tab = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(0.0, 1.0),
        egui::vec2(rect.width() * 0.45, 3.0),
    );
    painter.rect_filled(tab, 1.0, color.gamma_multiply(0.8));
    let body =
        egui::Rect::from_min_max(rect.left_top() + egui::vec2(0.0, 3.0), rect.right_bottom());
    painter.rect_filled(body, 2.0, color);
    if is_open {
        let flap =
            egui::Rect::from_min_max(body.left_top() + egui::vec2(2.0, 3.0), body.right_bottom());
        painter.rect_filled(flap, 2.0, color.gamma_multiply(1.25));
    }
}

/// A page with a folded corner, colored by what the file is.
fn paint_file(painter: &egui::Painter, rect: egui::Rect, name: &str) {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    let color = match ext {
        "h" | "hh" | "hpp" | "hxx" => egui::Color32::from_rgb(0x8A, 0x7B, 0xD8),
        "c" => egui::Color32::from_rgb(0x5F, 0xA8, 0xDE),
        "cc" | "cpp" | "cxx" => egui::Color32::from_rgb(0x4C, 0xB8, 0x8E),
        _ => egui::Color32::from_gray(0x9A),
    };
    let page = rect.shrink2(egui::vec2(2.0, 0.0));
    let fold = 4.0;
    let points = vec![
        page.left_top(),
        page.right_top() + egui::vec2(-fold, 0.0),
        page.right_top() + egui::vec2(0.0, fold),
        page.right_bottom(),
        page.left_bottom(),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
    let corner = vec![
        page.right_top() + egui::vec2(-fold, 0.0),
        page.right_top() + egui::vec2(0.0, fold),
        page.right_top() + egui::vec2(-fold, fold),
    ];
    painter.add(egui::Shape::convex_polygon(
        corner,
        color.gamma_multiply(0.55),
        egui::Stroke::NONE,
    ));
    let line_color = egui::Color32::from_white_alpha(170);
    for dy in [7.0, 10.0] {
        let y = page.top() + dy;
        painter.line_segment(
            [
                egui::pos2(page.left() + 2.0, y),
                egui::pos2(page.right() - 2.0, y),
            ],
            egui::Stroke::new(1.0, line_color),
        );
    }
}

/// Whether `file` is somewhere in `dir`.
fn holds(dir: &Dir, file: &str) -> bool {
    dir.files.iter().any(|(_, path)| path == file) || dir.dirs.values().any(|d| holds(d, file))
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn files_are_grouped_by_folder_in_name_order() {
        let tree = Dir::of(paths(&[
            "src/b.cc",
            "src/a.cc",
            "main.cc",
            "src/io/r.h",
            "src/a.cc",
        ]));
        assert_eq!(tree.files, [("main.cc".into(), "main.cc".into())]);
        assert_eq!(tree.count(), 4);
        let src = &tree.dirs["src"];
        let names: Vec<_> = src.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["a.cc", "b.cc"]);
        assert!(holds(src, "src/io/r.h"));
        assert!(!holds(src, "main.cc"));
    }

    #[test]
    fn clicking_a_file_in_the_explorer_opens_it() {
        struct State {
            explorer: FileExplorer,
            graph: Arc<CodeGraph>,
            current: Option<String>,
            action: Option<FileAction>,
        }
        let graph = Arc::new(crate::demo::demo_codebase().expect("demo"));
        let file = files_of(&graph).remove(0);
        let state = State {
            explorer: FileExplorer::default(),
            graph,
            current: Some(file.clone()),
            action: None,
        };
        let mut harness = crate::app::fonts::test_harness().build_ui_state(
            |ui, state: &mut State| {
                let current = state.current.as_deref();
                if let Some(action) = state.explorer.show(ui, &state.graph, current) {
                    state.action = Some(action);
                }
            },
            state,
        );
        harness.run();
        let name = file.rsplit('/').next().unwrap().to_string();
        harness.get_by_label(&name).click();
        harness.run();
        assert_eq!(harness.state().action, Some(FileAction::Open(file)));
    }
}
