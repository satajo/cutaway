//! The port through which scenarios drive the application.

use cutaway_architecture::ArchitectureGraph;
use cutaway_inspection::inspect;
use cutaway_inspection::ports::source_tree::SourcePath;
use cutaway_treesitter::RustSyntaxAnalyzer;

use crate::fakes::InMemorySourceTree;

/// What every scenario may do to the application, stated in domain terms.
/// Implementations decide which surface actually receives the actions.
pub trait ApplicationDriver {
    fn add_source_file(&mut self, path: &str, contents: &str);
    fn inspect_project(&mut self) -> Result<(), String>;
    fn element_names(&self) -> Vec<String>;
}

/// Drives the application core in-process, with the same analyzers the
/// composition root wires into the real application.
#[derive(Debug, Default)]
pub struct InProcessDriver {
    sources: InMemorySourceTree,
    inspected: Option<ArchitectureGraph>,
}

impl ApplicationDriver for InProcessDriver {
    fn add_source_file(&mut self, path: &str, contents: &str) {
        let path = SourcePath::new(path).expect("scenarios use valid source paths");
        self.sources.add_file(path, contents);
    }

    fn inspect_project(&mut self) -> Result<(), String> {
        let graph =
            inspect(&self.sources, &[&RustSyntaxAnalyzer]).map_err(|error| error.to_string())?;
        self.inspected = Some(graph);
        Ok(())
    }

    fn element_names(&self) -> Vec<String> {
        self.inspected
            .iter()
            .flat_map(ArchitectureGraph::elements)
            .map(|element| element.name.to_string())
            .collect()
    }
}
