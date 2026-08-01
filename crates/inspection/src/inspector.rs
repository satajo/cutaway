use std::collections::BTreeSet;

use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementKind, ElementName, GraphError, Relation,
    RelationKind,
};

use crate::ports::source_analyzer::{AnalyzedElement, SourceAnalysisError, SourceAnalyzer};
use crate::ports::source_tree::{ProjectName, SourceTree, SourceTreeError};

/// Builds the architecture graph of one version of a source tree.
///
/// The inspector contributes the project root element; everything else comes
/// from the analyzers. Analyzer elements nest under their declared parent, or
/// under the project root when they declare none. Relations from all
/// analyzers merge as a set: the same fact found twice is one relation.
pub fn inspect(
    tree: &dyn SourceTree,
    analyzers: &[&dyn SourceAnalyzer],
) -> Result<ArchitectureGraph, InspectionError> {
    let files = tree.files()?;
    let project = project_element(&tree.name());
    let project_id = project.id.clone();

    let mut graph = ArchitectureGraph::new();
    graph.add_element(project)?;

    let mut relations = BTreeSet::new();
    for analyzer in analyzers {
        let structure = analyzer.analyze(&files)?;
        for AnalyzedElement { element, parent } in structure.elements {
            let child_id = element.id.clone();
            graph.add_element(element)?;
            relations.insert(Relation {
                from: parent.unwrap_or_else(|| project_id.clone()),
                to: child_id,
                kind: RelationKind::Contains,
            });
        }
        relations.extend(structure.relations);
    }
    for relation in relations {
        graph.add_relation(relation)?;
    }
    Ok(graph)
}

fn project_element(name: &ProjectName) -> Element {
    Element {
        id: ElementId::new(format!("project:{name}")).expect("a project name is never empty"),
        name: ElementName::new(name.as_str()).expect("a project name is never empty"),
        kind: ElementKind::Project,
    }
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
    #[error("the inspected sources describe an inconsistent architecture")]
    Inconsistent {
        #[from]
        source: GraphError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::source_analyzer::SourceStructure;
    use crate::ports::source_tree::SourceFile;

    #[derive(Debug)]
    struct FakeTree;

    impl SourceTree for FakeTree {
        fn name(&self) -> ProjectName {
            ProjectName::new("fixture").unwrap()
        }

        fn files(&self) -> Result<Vec<SourceFile>, SourceTreeError> {
            Ok(Vec::new())
        }
    }

    struct FakeAnalyzer(SourceStructure);

    impl SourceAnalyzer for FakeAnalyzer {
        fn analyze(&self, _files: &[SourceFile]) -> Result<SourceStructure, SourceAnalysisError> {
            Ok(self.0.clone())
        }
    }

    fn element(id: &str, kind: ElementKind) -> Element {
        Element {
            id: ElementId::new(id).unwrap(),
            name: ElementName::new(id).unwrap(),
            kind,
        }
    }

    fn analyzed(id: &str, kind: ElementKind, parent: Option<&str>) -> AnalyzedElement {
        AnalyzedElement {
            element: element(id, kind),
            parent: parent.map(|p| ElementId::new(p).unwrap()),
        }
    }

    #[test]
    fn the_project_root_contains_every_parentless_element() {
        let analyzer = FakeAnalyzer(SourceStructure {
            elements: vec![analyzed("package:a", ElementKind::Package, None)],
            relations: Vec::new(),
        });
        let graph = inspect(&FakeTree, &[&analyzer]).unwrap();

        assert!(graph.relations().any(|r| {
            r.from == ElementId::new("project:fixture").unwrap()
                && r.to == ElementId::new("package:a").unwrap()
                && r.kind == RelationKind::Contains
        }));
    }

    #[test]
    fn declared_parents_form_the_containment_chain() {
        let analyzer = FakeAnalyzer(SourceStructure {
            elements: vec![
                analyzed("package:a", ElementKind::Package, None),
                analyzed("a/src/lib.rs", ElementKind::Module, Some("package:a")),
            ],
            relations: Vec::new(),
        });
        let graph = inspect(&FakeTree, &[&analyzer]).unwrap();

        assert!(graph.relations().any(|r| {
            r.from == ElementId::new("package:a").unwrap()
                && r.to == ElementId::new("a/src/lib.rs").unwrap()
                && r.kind == RelationKind::Contains
        }));
    }

    #[test]
    fn the_same_relation_found_by_two_analyzers_is_one_relation() {
        let depends = Relation {
            from: ElementId::new("package:a").unwrap(),
            to: ElementId::new("package:b").unwrap(),
            kind: RelationKind::DependsOn,
        };
        let one = FakeAnalyzer(SourceStructure {
            elements: vec![
                analyzed("package:a", ElementKind::Package, None),
                analyzed("package:b", ElementKind::Package, None),
            ],
            relations: vec![depends.clone()],
        });
        let two = FakeAnalyzer(SourceStructure {
            elements: Vec::new(),
            relations: vec![depends.clone()],
        });
        let graph = inspect(&FakeTree, &[&one, &two]).unwrap();

        assert_eq!(
            graph.relations().filter(|r| **r == depends).count(),
            1,
            "the duplicate must merge instead of failing"
        );
    }

    #[test]
    fn two_analyzers_declaring_the_same_element_fail_the_inspection() {
        let one = FakeAnalyzer(SourceStructure {
            elements: vec![analyzed("package:a", ElementKind::Package, None)],
            relations: Vec::new(),
        });
        let two = FakeAnalyzer(SourceStructure {
            elements: vec![analyzed("package:a", ElementKind::Package, None)],
            relations: Vec::new(),
        });
        assert!(matches!(
            inspect(&FakeTree, &[&one, &two]),
            Err(InspectionError::Inconsistent { .. })
        ));
    }

    #[test]
    fn a_relation_to_an_undeclared_element_fails_the_inspection() {
        let analyzer = FakeAnalyzer(SourceStructure {
            elements: vec![analyzed("package:a", ElementKind::Package, None)],
            relations: vec![Relation {
                from: ElementId::new("package:a").unwrap(),
                to: ElementId::new("package:missing").unwrap(),
                kind: RelationKind::DependsOn,
            }],
        });
        assert!(matches!(
            inspect(&FakeTree, &[&analyzer]),
            Err(InspectionError::Inconsistent { .. })
        ));
    }

    #[test]
    fn an_empty_project_is_just_its_root() {
        let graph = inspect(&FakeTree, &[]).unwrap();
        assert_eq!(graph.elements().count(), 1);
        assert_eq!(graph.elements().next().unwrap().kind, ElementKind::Project);
    }
}
