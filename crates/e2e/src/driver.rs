//! The port through which scenarios drive the application.

use std::collections::BTreeSet;

use cutaway_analyzer_rust::RustSourceAnalyzer;
use cutaway_architecture::{ArchitectureGraph, ElementId, Relation, RelationKind};
use cutaway_inspection::inspect;
use cutaway_lenses::{BoundaryView, Cut, Detail, boundary_view, self_leaf_frame};
use cutaway_planning::ports::plan_store::PlanStore;
use cutaway_planning::{GroupStanding, Note, Plan, ProposedChange, Subject};

use crate::fakes::{InMemoryPlanStore, InMemorySourceTree};

/// What every scenario may do to the application, stated in domain terms.
/// Boundaries and connections go by their display names, exactly as a user
/// sees them. Implementations decide which surface receives the actions.
pub trait ApplicationDriver {
    fn add_source_file(&mut self, path: &str, contents: &str);
    fn inspect_project(&mut self) -> Result<(), String>;
    /// `level` is `packages`, `modules`, or `items`.
    fn view_boundaries(&mut self, level: &str) -> Result<(), String>;
    /// Opens one boundary a step deeper than the rest of the picture.
    fn expand_boundary(&mut self, name: &str) -> Result<(), String>;
    /// Closes one boundary a step back toward a single box.
    fn collapse_boundary(&mut self, name: &str) -> Result<(), String>;
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
    cut: Option<Cut>,
    view: Option<BoundaryView>,
    plan: Plan,
    store: InMemoryPlanStore,
}

impl InProcessDriver {
    fn view(&self) -> &BoundaryView {
        self.view.as_ref().expect("a boundary view is active")
    }

    /// Paints the current cut anew, exactly as the GUI does after the reader
    /// opens or closes a boundary: the lens views the architecture with the
    /// plan's added dependencies drawn in, so a drawn connection rolls up
    /// at every cut the way the concrete ones do.
    fn rebuild_view(&mut self) -> Result<(), String> {
        let graph = self.graph.as_ref().ok_or("no project inspected yet")?;
        let cut = self.cut.as_ref().ok_or("no boundary view yet")?;
        let viewed = self.plan.with_planned_dependencies(graph);
        let view = boundary_view(&viewed, cut).map_err(|error| error.to_string())?;
        self.view = Some(view);
        Ok(())
    }

    fn step_boundary(
        &mut self,
        name: &str,
        step: fn(&mut Cut, &BoundaryView, &ElementId) -> bool,
    ) -> Result<(), String> {
        let id = self.boundary_id(name)?;
        let view = self.view.as_ref().ok_or("no boundary view yet")?;
        let cut = self.cut.as_mut().ok_or("no boundary view yet")?;
        if !step(cut, view, &id) {
            return Err(format!("the detail inside {name} cannot go any further"));
        }
        self.rebuild_view()
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

    /// Every concrete dependency behind the connection between two named
    /// boundaries. The plan anchors to these, never to the boundary pair,
    /// so a markup holds at whatever cut the dependencies reattach.
    fn concrete_behind(&self, from: &str, to: &str) -> Result<BTreeSet<Relation>, String> {
        let pair = self.connection(from, to)?;
        Ok(self.view().concrete_behind(&pair))
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
        // A new detail for the whole picture drops the boundaries opened or
        // closed under the old one: those decisions answered the old detail.
        self.cut = Some(Cut::uniform(detail));
        self.rebuild_view()
    }

    fn expand_boundary(&mut self, name: &str) -> Result<(), String> {
        self.step_boundary(name, Cut::expand)
    }

    fn collapse_boundary(&mut self, name: &str) -> Result<(), String> {
        self.step_boundary(name, Cut::collapse)
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
        let mut pairs: Vec<(String, String)> = view
            .provenance
            .keys()
            .map(|relation| (name(&relation.from), name(&relation.to)))
            .collect();
        // A drawn dependency naming an open frame as a whole still shows -
        // the plan speaks at every cut - where the architecture's own
        // whole-frame dependencies wait at a coarser detail.
        for (pair, concrete) in &view.coarse {
            if matches!(self.plan.standing_of(concrete), GroupStanding::Added) {
                pairs.push((name(&pair.from), name(&pair.to)));
            }
        }
        pairs
    }

    fn sever_connection(&mut self, from: &str, to: &str) -> Result<(), String> {
        let concrete = self.concrete_behind(from, to)?;
        if concrete.is_empty() {
            return Err(format!("no connection goes from {from} to {to}"));
        }
        for relation in concrete {
            self.plan
                .propose(ProposedChange::RemoveRelation(relation))
                .map_err(|error| error.to_string())?;
        }
        self.save()
    }

    fn draw_connection(&mut self, from: &str, to: &str) -> Result<(), String> {
        // A drawn dependency stores real element ids: a pick on a frame's
        // own-content box means the frame it belongs to.
        let pair = self.connection(from, to)?;
        let relation = Relation {
            from: self_leaf_frame(&pair.from).unwrap_or(pair.from),
            to: self_leaf_frame(&pair.to).unwrap_or(pair.to),
            kind: RelationKind::DependsOn,
        };
        self.plan
            .propose(ProposedChange::AddRelation(relation))
            .map_err(|error| error.to_string())?;
        // The addition joins the viewed graph, so the picture rolls it up.
        self.rebuild_view()?;
        self.save()
    }

    fn annotate_connection(&mut self, from: &str, to: &str, note: &str) -> Result<(), String> {
        let concrete = self.concrete_behind(from, to)?;
        if concrete.is_empty() {
            return Err(format!("no connection goes from {from} to {to}"));
        }
        let note = Note::new(note).map_err(|error| error.to_string())?;
        // The note lands on every concrete dependency behind the rolled-up
        // connection, so it is findable at any cut that shows them.
        if matches!(self.plan.standing_of(&concrete), GroupStanding::Added) {
            for relation in concrete {
                self.plan
                    .explain(&ProposedChange::AddRelation(relation), Some(note.clone()))
                    .map_err(|error| error.to_string())?;
            }
        } else {
            for relation in concrete {
                if self.plan.plans_addition_of(&relation) {
                    continue;
                }
                self.plan
                    .annotate(Subject::Relation(relation), note.clone());
            }
        }
        self.save()
    }

    fn removal_is_planned(&self, from: &str, to: &str) -> bool {
        self.concrete_behind(from, to).is_ok_and(|concrete| {
            matches!(self.plan.standing_of(&concrete), GroupStanding::Removed)
        })
    }

    fn addition_is_planned(&self, from: &str, to: &str) -> bool {
        self.concrete_behind(from, to)
            .is_ok_and(|concrete| matches!(self.plan.standing_of(&concrete), GroupStanding::Added))
    }

    fn note_on_connection(&self, from: &str, to: &str) -> Option<String> {
        let concrete = self.concrete_behind(from, to).ok()?;
        let standing = self.plan.standing_of(&concrete);
        concrete
            .iter()
            .find_map(|relation| match standing {
                GroupStanding::Added => self
                    .plan
                    .note_of(&ProposedChange::AddRelation(relation.clone())),
                GroupStanding::Removed => self
                    .plan
                    .note_of(&ProposedChange::RemoveRelation(relation.clone())),
                _ => self
                    .plan
                    .annotation_of(&Subject::Relation(relation.clone())),
            })
            .map(|note| note.as_str().to_owned())
    }

    fn saved_plan(&self) -> Option<Plan> {
        self.store.saved()
    }

    fn working_plan(&self) -> Plan {
        self.plan.clone()
    }
}
