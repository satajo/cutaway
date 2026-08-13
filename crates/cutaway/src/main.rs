//! Composition root: the only place that knows every adapter. It wires the
//! driven adapters (git, the language analyzers, the JSON plan store) into
//! the application cores and starts the driving adapter (the GUI).

use std::error::Error;
use std::num::NonZeroUsize;
use std::path::Path;
use std::process::ExitCode;

use cutaway_analyzer_go::GoSourceAnalyzer;
use cutaway_analyzer_rust::RustSourceAnalyzer;
use cutaway_analyzer_typescript::TypeScriptSourceAnalyzer;
use cutaway_gui::{
    InspectVersionError, OpenProjectError, OpenedProject, ProjectOpener, VersionInspector,
};
use cutaway_inspection::ports::project_history::{ProjectHistory, VersionId};
use cutaway_inspection::{Inspection, inspect};
use cutaway_plan_file::JsonPlanStore;
use cutaway_planning::ports::plan_store::PlanStore;
use cutaway_source_git::GitSourceTree;

/// How far back the comparison's version pickers reach: more history than a
/// reader scrolls through, and one walk of the first-parent chain to read.
const RECENT_VERSIONS: usize = 200;

fn main() -> ExitCode {
    // Every failure keeps the failure that caused it: the GUI names the step
    // that stopped and reads the whole chain out to the reader, so a broken
    // file names itself there rather than being flattened into a headline.
    let opener: ProjectOpener = Box::new(|path: &Path| {
        let tree = GitSourceTree::open(path).map_err(reading_the_repository)?;
        let Inspection { graph, gaps } = inspect(&tree, &analyzers())?;
        let versions = tree
            .recent(NonZeroUsize::new(RECENT_VERSIONS).expect("the version limit is not zero"))
            .map_err(reading_the_repository)?;
        let store = JsonPlanStore::for_repository(path);
        let plan = store
            .load()
            .map_err(|error| OpenProjectError::Plan {
                source: Box::new(error),
            })?
            .unwrap_or_default();
        // The comparison reads other versions of this very repository, so
        // the tree that opened it is the one that answers for them.
        let inspect_version: VersionInspector = Box::new(move |id: &VersionId| {
            let pinned = tree
                .tree_at(id)
                .map_err(|error| InspectVersionError::Repository {
                    source: Box::new(error),
                })?;
            inspect(pinned.as_ref(), &analyzers()).map_err(InspectVersionError::from)
        });
        Ok(OpenedProject {
            graph,
            gaps,
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

/// Whatever the git adapter refuses while a project is opening, said as the
/// step it stopped: the GUI is told which step failed and carries the refusal
/// itself along, never learning which technology raised it.
fn reading_the_repository<E: Error + Send + Sync + 'static>(error: E) -> OpenProjectError {
    OpenProjectError::Repository {
        source: Box::new(error),
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
