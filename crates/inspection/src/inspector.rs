use std::collections::BTreeSet;

use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementName, GraphError, Relation, RelationKind,
    SemanticKind,
};

use crate::ports::source_analyzer::{Interpretation, SourceAnalysisError, SourceAnalyzer};
use crate::ports::source_tree::{ProjectName, SourceTree, SourceTreeError};
use crate::substrate::{self, ExtentError};

/// Builds the architecture graph of one version of a source tree.
///
/// The inspector contributes the project root element, collects what every
/// analyzer read, and hands the whole to the assembler, which builds the file
/// tree and fuses the interpretations onto it. Relations from all analyzers
/// merge as a set: the same fact found twice is one relation.
///
/// Inspection is total: every file of the tree stands in the graph, as a node
/// of its own or fused with what a language read there, and every one of them
/// carries a fingerprint of its contents. What no language read stands as a
/// plain file or directory, grouped by the same laws the whole tree follows.
pub fn inspect(
    tree: &dyn SourceTree,
    analyzers: &[&dyn SourceAnalyzer],
) -> Result<ArchitectureGraph, InspectionError> {
    let files = tree.files()?;
    let project = project_element(&tree.name());
    let project_id = project.id.clone();

    let mut graph = ArchitectureGraph::new();
    graph.add_element(project)?;

    let mut interpretations: Vec<Interpretation> = Vec::new();
    let mut relations = BTreeSet::new();
    for analyzer in analyzers {
        let structure = analyzer.analyze(&files)?;
        interpretations.extend(structure.interpretations);
        relations.extend(structure.relations);
    }

    for placed in substrate::assemble(&files, &interpretations)? {
        let child = placed.element.id.clone();
        graph.add_element(placed.element)?;
        relations.insert(Relation {
            from: placed.parent.unwrap_or_else(|| project_id.clone()),
            to: child,
            kind: RelationKind::Contains,
        });
    }
    for relation in relations {
        graph.add_relation(relation)?;
    }
    Ok(graph)
}

fn project_element(name: &ProjectName) -> Element {
    Element::semantic(
        ElementId::new(format!("project:{name}")).expect("a project name is never empty"),
        SemanticKind::Project,
        ElementName::new(name.as_str()).expect("a project name is never empty"),
    )
}

#[derive(Debug, thiserror::Error)]
pub enum InspectionError {
    #[error(transparent)]
    Source(#[from] SourceTreeError),
    #[error("analysis of the sources failed")]
    Analysis {
        #[from]
        source: SourceAnalysisError,
    },
    #[error("an analyzer read something the sources do not hold")]
    Extent {
        #[from]
        source: ExtentError,
    },
    #[error("the inspected sources describe an inconsistent architecture")]
    Inconsistent {
        #[from]
        source: GraphError,
    },
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{ElementKind, SemanticKind};

    use super::*;
    use crate::ports::source_analyzer::{Extent, SourceStructure};
    use crate::ports::source_tree::{DirectoryPath, SourceFile, SourcePath};

    #[derive(Debug, Default)]
    struct FakeTree {
        files: Vec<SourceFile>,
    }

    impl FakeTree {
        fn holding(files: &[(&str, &str)]) -> Self {
            Self {
                files: files
                    .iter()
                    .map(|(path, contents)| SourceFile {
                        path: SourcePath::new(*path).unwrap(),
                        contents: contents.as_bytes().to_vec(),
                    })
                    .collect(),
            }
        }
    }

    impl SourceTree for FakeTree {
        fn name(&self) -> ProjectName {
            ProjectName::new("fixture").unwrap()
        }

        fn files(&self) -> Result<Vec<SourceFile>, SourceTreeError> {
            Ok(self.files.clone())
        }
    }

    struct FakeAnalyzer(SourceStructure);

    impl SourceAnalyzer for FakeAnalyzer {
        fn analyze(&self, _files: &[SourceFile]) -> Result<SourceStructure, SourceAnalysisError> {
            Ok(self.0.clone())
        }
    }

    fn package(id: &str, name: &str, extent: Extent) -> Interpretation {
        Interpretation {
            element: Element::semantic(
                ElementId::new(id).unwrap(),
                SemanticKind::Package,
                ElementName::new(name).unwrap(),
            ),
            extent,
        }
    }

    fn directory(path: &str) -> Extent {
        Extent::Directory(DirectoryPath::new(path).unwrap())
    }

    fn contains(graph: &ArchitectureGraph, from: &str, to: &str) -> bool {
        graph.relations().any(|relation| {
            relation.from == ElementId::new(from).unwrap()
                && relation.to == ElementId::new(to).unwrap()
                && relation.kind == RelationKind::Contains
        })
    }

    #[test]
    fn the_project_root_holds_what_lies_at_the_top_of_the_tree() {
        let graph = inspect(&FakeTree::holding(&[("README.md", "hi")]), &[]).unwrap();

        assert!(contains(&graph, "project:fixture", "README.md"));
    }

    #[test]
    fn the_file_tree_forms_the_containment_chain() {
        let tree = FakeTree::holding(&[("crates/a/Cargo.toml", ""), ("crates/a/src/lib.rs", "")]);
        let analyzer = FakeAnalyzer(SourceStructure {
            interpretations: vec![package("package:a", "a", directory("crates/a"))],
            relations: Vec::new(),
        });
        let graph = inspect(&tree, &[&analyzer]).unwrap();

        assert!(contains(&graph, "project:fixture", "package:a"));
        assert!(contains(&graph, "package:a", "crates/a/src/lib.rs"));
    }

    #[test]
    fn the_same_relation_found_by_two_analyzers_is_one_relation() {
        let tree = FakeTree::holding(&[("a/x", ""), ("b/x", "")]);
        let depends = Relation {
            from: ElementId::new("package:a").unwrap(),
            to: ElementId::new("package:b").unwrap(),
            kind: RelationKind::DependsOn,
        };
        let one = FakeAnalyzer(SourceStructure {
            interpretations: vec![
                package("package:a", "a", directory("a")),
                package("package:b", "b", directory("b")),
            ],
            relations: vec![depends.clone()],
        });
        let two = FakeAnalyzer(SourceStructure {
            interpretations: Vec::new(),
            relations: vec![depends.clone()],
        });
        let graph = inspect(&tree, &[&one, &two]).unwrap();

        assert_eq!(
            graph.relations().filter(|r| **r == depends).count(),
            1,
            "the duplicate must merge instead of failing"
        );
    }

    #[test]
    fn two_analyzers_declaring_the_same_element_fail_the_inspection() {
        let tree = FakeTree::holding(&[("a/x", ""), ("b/x", "")]);
        let one = FakeAnalyzer(SourceStructure {
            interpretations: vec![package("package:a", "a", directory("a"))],
            relations: Vec::new(),
        });
        let two = FakeAnalyzer(SourceStructure {
            interpretations: vec![package("package:a", "a", directory("b"))],
            relations: Vec::new(),
        });
        assert!(matches!(
            inspect(&tree, &[&one, &two]),
            Err(InspectionError::Inconsistent { .. })
        ));
    }

    #[test]
    fn a_relation_to_an_undeclared_element_fails_the_inspection() {
        let tree = FakeTree::holding(&[("a/x", "")]);
        let analyzer = FakeAnalyzer(SourceStructure {
            interpretations: vec![package("package:a", "a", directory("a"))],
            relations: vec![Relation {
                from: ElementId::new("package:a").unwrap(),
                to: ElementId::new("package:missing").unwrap(),
                kind: RelationKind::DependsOn,
            }],
        });
        assert!(matches!(
            inspect(&tree, &[&analyzer]),
            Err(InspectionError::Inconsistent { .. })
        ));
    }

    #[test]
    fn an_element_read_out_of_nothing_the_sources_hold_fails_the_inspection() {
        let analyzer = FakeAnalyzer(SourceStructure {
            interpretations: vec![package("package:a", "a", directory("crates/a"))],
            relations: Vec::new(),
        });
        assert!(matches!(
            inspect(&FakeTree::default(), &[&analyzer]),
            Err(InspectionError::Extent { .. })
        ));
    }

    #[test]
    fn an_empty_project_is_just_its_root() {
        let graph = inspect(&FakeTree::default(), &[]).unwrap();
        assert_eq!(graph.elements().count(), 1);
        assert_eq!(
            graph.elements().next().unwrap().primary_kind(),
            ElementKind::Project
        );
    }

    #[test]
    fn a_file_no_language_reads_stands_in_the_graph_as_itself() {
        let graph = inspect(&FakeTree::holding(&[("README.md", "hello")]), &[]).unwrap();

        let readme = graph
            .element(&ElementId::new("README.md").unwrap())
            .expect("the file stands in the graph");
        assert_eq!(readme.primary_kind(), ElementKind::File);
        assert!(readme.fingerprint.is_some());
    }
}
