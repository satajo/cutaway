//! Composition root: the only place that knows every adapter. It wires the
//! driven adapters (git, tree-sitter) into the application core and starts
//! the driving adapter (the GUI).

use std::path::Path;
use std::process::ExitCode;

use cutaway_git::GitSourceTree;
use cutaway_inspection::inspect;
use cutaway_treesitter::RustSyntaxAnalyzer;

fn main() -> ExitCode {
    let loader = Box::new(|path: &Path| {
        let tree = GitSourceTree::open(path).map_err(|error| error.to_string())?;
        inspect(&tree, &[&RustSyntaxAnalyzer]).map_err(|error| error.to_string())
    });

    match cutaway_gui::run(loader) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
