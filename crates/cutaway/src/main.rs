//! Composition root: the only place that knows every adapter. It wires the
//! driven adapters (git, the Rust analyzer, the JSON plan store) into the
//! application cores and starts the driving adapter (the GUI).

use std::path::Path;
use std::process::ExitCode;

use cutaway_git::GitSourceTree;
use cutaway_gui::OpenedProject;
use cutaway_inspection::inspect;
use cutaway_plan_json::JsonPlanStore;
use cutaway_redlining::ports::plan_store::PlanStore;
use cutaway_rust::RustSourceAnalyzer;

fn main() -> ExitCode {
    let opener = Box::new(|path: &Path| {
        let tree = GitSourceTree::open(path).map_err(|error| error.to_string())?;
        let graph = inspect(&tree, &[&RustSourceAnalyzer]).map_err(|error| error.to_string())?;
        let store = JsonPlanStore::for_repository(path);
        let plan = store
            .load()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        Ok(OpenedProject {
            graph,
            plan,
            store: Box::new(store),
        })
    });

    match cutaway_gui::run(opener) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
