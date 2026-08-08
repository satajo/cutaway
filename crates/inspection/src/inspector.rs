use std::collections::{BTreeMap, BTreeSet};

use cutaway_architecture::{
    ArchitectureGraph, Element, ElementId, ElementName, GraphError, Relation, RelationKind,
    SemanticKind,
};

use crate::ports::source_analyzer::{AnalyzedElement, SourceAnalysisError, SourceAnalyzer};
use crate::ports::source_tree::{DirectoryPath, ProjectName, SourceTree, SourceTreeError};
use crate::unclaimed;

/// Builds the architecture graph of one version of a source tree.
///
/// The inspector contributes the project root element; everything else comes
/// from the analyzers. Analyzer elements nest under their declared parent, or
/// under the project root when they declare none. Relations from all
/// analyzers merge as a set: the same fact found twice is one relation.
///
/// Inspection is total: every file of the tree stands in the graph, as
/// whatever a language analyzer made of it or - for a file no analyzer
/// claimed - as a plain file element carrying a fingerprint of its contents,
/// grouped into the directories that organize it and anchored inside the
/// package whose territory holds it.
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
    let mut claimed = BTreeSet::new();
    let mut territories: BTreeMap<DirectoryPath, ElementId> = BTreeMap::new();
    let mut contested = BTreeSet::new();
    let contain = |graph: &mut ArchitectureGraph,
                   relations: &mut BTreeSet<Relation>,
                   analyzed: AnalyzedElement|
     -> Result<(), GraphError> {
        let AnalyzedElement { element, parent } = analyzed;
        let child_id = element.id.clone();
        graph.add_element(element)?;
        relations.insert(Relation {
            from: parent.unwrap_or_else(|| project_id.clone()),
            to: child_id,
            kind: RelationKind::Contains,
        });
        Ok(())
    };
    for analyzer in analyzers {
        let structure = analyzer.analyze(&files)?;
        for analyzed in structure.elements {
            contain(&mut graph, &mut relations, analyzed)?;
        }
        relations.extend(structure.relations);
        claimed.extend(structure.claimed);
        for (dir, package) in structure.territories {
            match territories.get(&dir) {
                Some(existing) if *existing != package => {
                    contested.insert(dir);
                }
                _ => {
                    territories.insert(dir, package);
                }
            }
        }
    }
    // Two languages claiming one directory would make the anchoring
    // ambiguous, so a contested directory is no territory: its unclaimed
    // contents stand at the nearest enclosing anchor instead of inside an
    // arbitrary one of the claimants.
    for dir in contested {
        territories.remove(&dir);
    }
    for analyzed in unclaimed::unclaimed_files(&files, &claimed, &territories, &graph) {
        contain(&mut graph, &mut relations, analyzed)?;
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
    #[error("the inspected sources describe an inconsistent architecture")]
    Inconsistent {
        #[from]
        source: GraphError,
    },
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::ElementKind;

    use super::*;
    use crate::ports::source_analyzer::SourceStructure;
    use crate::ports::source_tree::{SourceFile, SourcePath};

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

    fn element(id: &str, kind: ElementKind) -> Element {
        Element::of_kind(
            ElementId::new(id).unwrap(),
            kind,
            ElementName::new(id).unwrap(),
        )
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
            ..SourceStructure::default()
        });
        let graph = inspect(&FakeTree::default(), &[&analyzer]).unwrap();

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
            ..SourceStructure::default()
        });
        let graph = inspect(&FakeTree::default(), &[&analyzer]).unwrap();

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
            ..SourceStructure::default()
        });
        let two = FakeAnalyzer(SourceStructure {
            elements: Vec::new(),
            relations: vec![depends.clone()],
            ..SourceStructure::default()
        });
        let graph = inspect(&FakeTree::default(), &[&one, &two]).unwrap();

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
            ..SourceStructure::default()
        });
        let two = FakeAnalyzer(SourceStructure {
            elements: vec![analyzed("package:a", ElementKind::Package, None)],
            relations: Vec::new(),
            ..SourceStructure::default()
        });
        assert!(matches!(
            inspect(&FakeTree::default(), &[&one, &two]),
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
            ..SourceStructure::default()
        });
        assert!(matches!(
            inspect(&FakeTree::default(), &[&analyzer]),
            Err(InspectionError::Inconsistent { .. })
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
    fn a_file_no_analyzer_claims_stands_in_the_graph() {
        let tree = FakeTree::holding(&[("README.md", "hello")]);
        let graph = inspect(&tree, &[]).unwrap();

        let readme = graph
            .element(&ElementId::new("README.md").unwrap())
            .expect("the unclaimed file stands in the graph");
        assert_eq!(readme.primary_kind(), ElementKind::File);
        assert!(graph.relations().any(|r| {
            r.from == ElementId::new("project:fixture").unwrap()
                && r.to == ElementId::new("README.md").unwrap()
                && r.kind == RelationKind::Contains
        }));
    }

    #[test]
    fn a_claimed_file_leaves_no_file_element_behind() {
        let tree = FakeTree::holding(&[("src/lib.rs", "pub fn go() {}")]);
        let analyzer = FakeAnalyzer(SourceStructure {
            elements: vec![analyzed("src/lib.rs", ElementKind::Module, None)],
            claimed: BTreeSet::from([SourcePath::new("src/lib.rs").unwrap()]),
            ..SourceStructure::default()
        });
        let graph = inspect(&tree, &[&analyzer]).unwrap();

        let claimed = graph
            .element(&ElementId::new("src/lib.rs").unwrap())
            .expect("the claimed file is the analyzer's element");
        assert_eq!(claimed.primary_kind(), ElementKind::Module);
    }

    #[test]
    fn an_unclaimed_file_inside_a_territory_stands_inside_the_package() {
        let tree = FakeTree::holding(&[("crates/a/notes.txt", "remember")]);
        let analyzer = FakeAnalyzer(SourceStructure {
            elements: vec![analyzed("package:a", ElementKind::Package, None)],
            territories: std::collections::BTreeMap::from([(
                DirectoryPath::new("crates/a").unwrap(),
                ElementId::new("package:a").unwrap(),
            )]),
            ..SourceStructure::default()
        });
        let graph = inspect(&tree, &[&analyzer]).unwrap();

        assert!(graph.relations().any(|r| {
            r.from == ElementId::new("package:a").unwrap()
                && r.to == ElementId::new("crates/a/notes.txt").unwrap()
                && r.kind == RelationKind::Contains
        }));
    }

    #[test]
    fn a_directory_two_analyzers_claim_as_territory_anchors_nothing() {
        let tree = FakeTree::holding(&[("notes.txt", "loose")]);
        let one = FakeAnalyzer(SourceStructure {
            elements: vec![analyzed("package:a", ElementKind::Package, None)],
            territories: std::collections::BTreeMap::from([(
                DirectoryPath::root(),
                ElementId::new("package:a").unwrap(),
            )]),
            ..SourceStructure::default()
        });
        let two = FakeAnalyzer(SourceStructure {
            elements: vec![analyzed("package:b", ElementKind::Package, None)],
            territories: std::collections::BTreeMap::from([(
                DirectoryPath::root(),
                ElementId::new("package:b").unwrap(),
            )]),
            ..SourceStructure::default()
        });
        let graph = inspect(&tree, &[&one, &two]).unwrap();

        assert!(
            graph.relations().any(|r| {
                r.from == ElementId::new("project:fixture").unwrap()
                    && r.to == ElementId::new("notes.txt").unwrap()
                    && r.kind == RelationKind::Contains
            }),
            "the contested territory dissolves, so the file stands at the project root"
        );
    }
}
