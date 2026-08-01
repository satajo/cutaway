//! GUI adapter: the eframe/egui desktop shell of Cutaway.
//!
//! The GUI drives the application core and knows nothing about where
//! architectures come from: the composition root hands it a
//! [`ProjectLoader`] and the GUI only renders what comes back. This keeps
//! every view testable against the core alone and lets the e2e suite drive
//! the same core without a window.

use std::path::Path;

use cutaway_architecture::{ArchitectureGraph, ElementKind};
use eframe::egui;

/// Loads the architecture of the project at a path.
///
/// The composition root wires this to the real adapters. Failures arrive as
/// human-readable text: the GUI only displays them, it never reacts to
/// individual failure causes.
pub type ProjectLoader = Box<dyn Fn(&Path) -> Result<ArchitectureGraph, String>>;

pub fn run(loader: ProjectLoader) -> Result<(), StartupError> {
    eframe::run_native(
        "Cutaway",
        eframe::NativeOptions::default(),
        Box::new(move |_context| Ok(Box::new(CutawayApp::new(loader)))),
    )
    .map_err(|source| StartupError::Gui {
        reason: source.to_string(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("cannot start the GUI: {reason}")]
    Gui { reason: String },
}

struct CutawayApp {
    loader: ProjectLoader,
    repository_path: String,
    project: Option<Result<ArchitectureGraph, String>>,
}

impl CutawayApp {
    fn new(loader: ProjectLoader) -> Self {
        Self {
            loader,
            repository_path: String::new(),
            project: None,
        }
    }
}

impl eframe::App for CutawayApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("repository").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Repository:");
                ui.text_edit_singleline(&mut self.repository_path);
                if ui.button("Open").clicked() {
                    self.project = Some((self.loader)(Path::new(&self.repository_path)));
                }
            });
        });

        egui::Panel::left("elements").show(ui, |ui| {
            ui.heading("Elements");
            match &self.project {
                None => {
                    ui.label("Open a repository to inspect its architecture.");
                }
                Some(Err(reason)) => {
                    ui.colored_label(ui.visuals().error_fg_color, reason);
                }
                Some(Ok(graph)) => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for element in graph.elements() {
                            ui.label(format!("{} {}", kind_symbol(element.kind), element.name));
                        }
                    });
                }
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Cutaway");
            ui.label(
                "The cutaway canvas arrives here: lenses over the architecture, \
                 version deltas, and redlines.",
            );
        });
    }
}

fn kind_symbol(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Module => "▤",
        ElementKind::Function => "ƒ",
        ElementKind::Type => "T",
    }
}
