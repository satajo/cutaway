//! The port through which scenarios drive the application.

use cutaway_analyzer_rust::RustSourceAnalyzer;
use cutaway_architecture::{ArchitectureGraph, ElementId, Relation, RelationKind};
use cutaway_inspection::inspect;
use cutaway_lenses::{BoundaryView, Detail, boundary_view};
use cutaway_planning::ports::plan_store::PlanStore;
use cutaway_planning::{Note, Plan, ProposedChange, Subject};

use crate::fakes::{InMemoryPlanStore, InMemorySourceTree};

/// What every scenario may do to the application, stated in domain terms.
/// Boundaries and connections go by their display names, exactly as a user
/// sees them. Implementations decide which surface receives the actions.
pub trait ApplicationDriver {
    fn add_source_file(&mut self, path: &str, contents: &str);
    fn inspect_project(&mut self) -> Result<(), String>;
    /// `level` is `packages`, `modules`, or `items`.
    fn view_boundaries(&mut self, level: &str) -> Result<(), String>;
    fn boundary_names(&self) -> Vec<String>;
    fn connections(&self) -> Vec<(String, String)>;
    fn sever_connection(&mut self, from: &str, to: &str) -> Result<(), String>;
    fn draw_connection(&mut self, from: &str, to: &str) -> Result<(), String>;
    fn annotate_connection(&mut self, from: &str, to: &str, note: &str) -> Result<(), String>;
    fn removal_is_planned(&self, from: &str, to: &str) -> bool;
    fn addition_is_planned(&self, from: &str, to: &str) -> bool;
    fn note_on_connection(&self, from: &str, to: &str) -> Option<String>;
    fn saved_plan(&self) -> Option<Plan>;
    fn working_plan(&self) -> Plan;
}

/// Drives the application cores in-process, with the same analyzer the
/// composition root wires into the real application.
#[derive(Debug, Default)]
pub struct InProcessDriver {
    sources: InMemorySourceTree,
    graph: Option<ArchitectureGraph>,
    view: Option<BoundaryView>,
    plan: Plan,
    store: InMemoryPlanStore,
}

impl InProcessDriver {
    fn view(&self) -> &BoundaryView {
        self.view.as_ref().expect("a boundary view is active")
    }

    fn boundary_id(&self, name: &str) -> Result<ElementId, String> {
        self.view()
            .graph
            .elements()
            .find(|element| element.name.as_str() == name)
            .map(|element| element.id.clone())
            .ok_or_else(|| format!("no boundary named {name}"))
    }

    fn connection(&self, from: &str, to: &str) -> Result<Relation, String> {
        Ok(Relation {
            from: self.boundary_id(from)?,
            to: self.boundary_id(to)?,
            kind: RelationKind::DependsOn,
        })
    }

    fn save(&mut self) -> Result<(), String> {
        self.store
            .save(&self.plan)
            .map_err(|error| error.to_string())
    }
}

impl ApplicationDriver for InProcessDriver {
    fn add_source_file(&mut self, path: &str, contents: &str) {
        let path = cutaway_inspection::ports::source_tree::SourcePath::new(path)
            .expect("scenarios use valid source paths");
        self.sources.add_file(path, contents);
    }

    fn inspect_project(&mut self) -> Result<(), String> {
        let graph =
            inspect(&self.sources, &[&RustSourceAnalyzer]).map_err(|error| error.to_string())?;
        self.graph = Some(graph);
        Ok(())
    }

    fn view_boundaries(&mut self, level: &str) -> Result<(), String> {
        let detail = match level {
            "packages" => Detail::Packages,
            "modules" => Detail::Modules,
            "items" => Detail::Items,
            other => return Err(format!("unknown detail level {other}")),
        };
        let graph = self.graph.as_ref().ok_or("no project inspected yet")?;
        self.view = Some(boundary_view(graph, detail).map_err(|error| error.to_string())?);
        Ok(())
    }

    fn boundary_names(&self) -> Vec<String> {
        self.view()
            .graph
            .elements()
            .map(|element| element.name.to_string())
            .collect()
    }

    fn connections(&self) -> Vec<(String, String)> {
        let view = self.view();
        let name = |id: &ElementId| {
            view.graph
                .element(id)
                .map_or_else(|| id.to_string(), |element| element.name.to_string())
        };
        view.provenance
            .keys()
            .map(|relation| (name(&relation.from), name(&relation.to)))
            .collect()
    }

    fn sever_connection(&mut self, from: &str, to: &str) -> Result<(), String> {
        let relation = self.connection(from, to)?;
        self.plan
            .propose(ProposedChange::RemoveRelation(relation))
            .map_err(|error| error.to_string())?;
        self.save()
    }

    fn draw_connection(&mut self, from: &str, to: &str) -> Result<(), String> {
        let relation = self.connection(from, to)?;
        self.plan
            .propose(ProposedChange::AddRelation(relation))
            .map_err(|error| error.to_string())?;
        self.save()
    }

    fn annotate_connection(&mut self, from: &str, to: &str, note: &str) -> Result<(), String> {
        let relation = self.connection(from, to)?;
        let note = Note::new(note).map_err(|error| error.to_string())?;
        self.plan.annotate(Subject::Relation(relation), note);
        self.save()
    }

    fn removal_is_planned(&self, from: &str, to: &str) -> bool {
        self.connection(from, to)
            .is_ok_and(|relation| self.plan.plans_removal_of(&relation))
    }

    fn addition_is_planned(&self, from: &str, to: &str) -> bool {
        self.connection(from, to)
            .is_ok_and(|relation| self.plan.plans_addition_of(&relation))
    }

    fn note_on_connection(&self, from: &str, to: &str) -> Option<String> {
        let relation = self.connection(from, to).ok()?;
        self.plan
            .annotation_of(&Subject::Relation(relation))
            .map(|note| note.as_str().to_owned())
    }

    fn saved_plan(&self) -> Option<Plan> {
        self.store.saved()
    }

    fn working_plan(&self) -> Plan {
        self.plan.clone()
    }
}
