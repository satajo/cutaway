//! Composition root: the only place that knows every adapter. It wires the
//! driven adapters (git, the language analyzers, the JSON plan store) into
//! the application cores and starts the driving adapter (the GUI).

use std::num::NonZeroUsize;
use std::path::Path;
use std::process::ExitCode;

use cutaway_analyzer_go::GoSourceAnalyzer;
use cutaway_analyzer_rust::RustSourceAnalyzer;
use cutaway_analyzer_typescript::TypeScriptSourceAnalyzer;
use cutaway_gui::{OpenedProject, VersionInspector};
use cutaway_inspection::inspect;
use cutaway_inspection::ports::project_history::{ProjectHistory, VersionId};
use cutaway_plan_file::JsonPlanStore;
use cutaway_planning::ports::plan_store::PlanStore;
use cutaway_source_git::GitSourceTree;

/// How far back the comparison's version pickers reach: more history than a
/// reader scrolls through, and one walk of the first-parent chain to read.
const RECENT_VERSIONS: usize = 200;

fn main() -> ExitCode {
    let opener = Box::new(|path: &Path| {
        let tree = GitSourceTree::open(path).map_err(|error| error.to_string())?;
        let graph = inspect(&tree, &analyzers()).map_err(|error| error.to_string())?;
        let versions = tree
            .recent(NonZeroUsize::new(RECENT_VERSIONS).expect("the version limit is not zero"))
            .map_err(|error| error.to_string())?;
        let store = JsonPlanStore::for_repository(path);
        let plan = store
            .load()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        // The comparison reads other versions of this very repository, so
        // the tree that opened it is the one that answers for them.
        let inspect_version: VersionInspector = Box::new(move |id: &VersionId| {
            let pinned = tree.tree_at(id).map_err(|error| error.to_string())?;
            inspect(pinned.as_ref(), &analyzers()).map_err(|error| error.to_string())
        });
        Ok(OpenedProject {
            graph,
            plan,
            store: Box::new(store),
            versions,
            inspect_version,
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

/// Every language Cutaway reads. One inspection of one version asks all of
/// them, so every version of a project is read through the same set.
fn analyzers() -> [&'static dyn cutaway_inspection::ports::source_analyzer::SourceAnalyzer; 3] {
    [
        &RustSourceAnalyzer,
        &GoSourceAnalyzer,
        &TypeScriptSourceAnalyzer,
    ]
}
