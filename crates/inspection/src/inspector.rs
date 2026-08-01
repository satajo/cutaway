use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementKind, ElementName, GraphError, Relation,
    RelationKind,
};

use crate::ports::source_tree::{SourcePath, SourceTree, SourceTreeError};
use crate::ports::syntax_analyzer::{Declaration, SyntaxAnalysisError, SyntaxAnalyzer};

/// Builds the architecture graph of one version of a source tree.
///
/// Every file becomes a `Module` element, whether or not any analyzer
/// understands it: the file layout is architecture too. Every declaration a
/// supporting analyzer finds becomes an element contained by its file's
/// element. At most one analyzer may support a given path; two analyzers
/// claiming the same file would produce colliding elements and fail.
pub fn inspect(
    tree: &dyn SourceTree,
    analyzers: &[&dyn SyntaxAnalyzer],
) -> Result<ArchitectureGraph, InspectionError> {
    let mut graph = ArchitectureGraph::new();
    for file in tree.files()? {
        let file_element = file_element(&file.path);
        let file_id = file_element.id.clone();
        graph.add_element(file_element)?;

        for analyzer in analyzers {
            if !analyzer.supports(&file.path) {
                continue;
            }
            for declaration in analyzer.analyze(&file)? {
                let element = declaration_element(&file.path, &declaration);
                let element_id = element.id.clone();
                graph.add_element(element)?;
                graph.add_relation(Relation {
                    from: file_id.clone(),
                    to: element_id,
                    kind: RelationKind::Contains,
                })?;
            }
        }
    }
    Ok(graph)
}

fn file_element(path: &SourcePath) -> Element {
    Element {
        id: ElementId::new(path.as_str()).expect("a source path is never empty"),
        name: ElementName::new(path.as_str()).expect("a source path is never empty"),
        kind: ElementKind::Module,
    }
}

/// Declaration ids embed the path and the kind so that same-named
/// declarations in different files, or in different namespaces of the same
/// file (a struct and a function may share a name in Rust), stay distinct.
fn declaration_element(path: &SourcePath, declaration: &Declaration) -> Element {
    let kind_tag = match declaration.kind {
        ElementKind::Module => "module",
        ElementKind::Function => "function",
        ElementKind::Type => "type",
    };
    Element {
        id: ElementId::new(format!("{path}#{kind_tag}:{}", declaration.name))
            .expect("the id embeds a non-empty path"),
        name: declaration.name.clone(),
        kind: declaration.kind,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InspectionError {
    #[error(transparent)]
    Source(#[from] SourceTreeError),
    #[error("analysis of a source file failed")]
    Analysis {
        #[from]
        source: SyntaxAnalysisError,
    },
    #[error("the inspected sources describe an inconsistent architecture")]
    Inconsistent {
        #[from]
        source: GraphError,
    },
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::ElementName;

    use super::*;
    use crate::ports::source_tree::SourceFile;

    #[derive(Debug)]
    struct FakeTree(Vec<SourceFile>);

    impl SourceTree for FakeTree {
        fn files(&self) -> Result<Vec<SourceFile>, SourceTreeError> {
            Ok(self.0.clone())
        }
    }

    /// Claims `.fake` files and declares one function named after the file's
    /// entire contents.
    #[derive(Debug)]
    struct FakeAnalyzer;

    impl SyntaxAnalyzer for FakeAnalyzer {
        fn supports(&self, path: &SourcePath) -> bool {
            path.extension() == Some("fake")
        }

        fn analyze(&self, file: &SourceFile) -> Result<Vec<Declaration>, SyntaxAnalysisError> {
            let name = String::from_utf8(file.contents.clone()).unwrap();
            Ok(vec![Declaration {
                name: ElementName::new(name).unwrap(),
                kind: ElementKind::Function,
            }])
        }
    }

    fn file(path: &str, contents: &str) -> SourceFile {
        SourceFile {
            path: SourcePath::new(path).unwrap(),
            contents: contents.into(),
        }
    }

    #[test]
    fn every_source_file_appears_as_a_module_element() {
        let tree = FakeTree(vec![file("README.md", "hello"), file("main.fake", "run")]);
        let graph = inspect(&tree, &[]).unwrap();
        let names: Vec<_> = graph.elements().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["README.md", "main.fake"]);
    }

    #[test]
    fn declarations_are_contained_by_their_file() {
        let tree = FakeTree(vec![file("main.fake", "run")]);
        let graph = inspect(&tree, &[&FakeAnalyzer]).unwrap();

        let file_id = ElementId::new("main.fake").unwrap();
        let declaration_id = ElementId::new("main.fake#function:run").unwrap();
        assert_eq!(
            graph.element(&declaration_id).unwrap().kind,
            ElementKind::Function
        );
        assert!(graph.relations().any(|r| {
            r.from == file_id && r.to == declaration_id && r.kind == RelationKind::Contains
        }));
    }

    #[test]
    fn files_without_a_supporting_analyzer_contribute_only_their_module_element() {
        let tree = FakeTree(vec![file("README.md", "hello")]);
        let graph = inspect(&tree, &[&FakeAnalyzer]).unwrap();
        assert_eq!(graph.elements().count(), 1);
        assert_eq!(graph.relations().count(), 0);
    }
}
