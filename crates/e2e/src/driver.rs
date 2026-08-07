//! The port through which scenarios drive the application.

use std::collections::{BTreeMap, BTreeSet};

use cutaway_analyzer_rust::RustSourceAnalyzer;
use cutaway_analyzer_typescript::TypeScriptSourceAnalyzer;
use cutaway_architecture::{
    ArchitectureGraph, ElementId, ElementKind, ElementName, Relation, RelationKind,
};
use cutaway_comparison::{Comparison, ElementChange, Presence};
use cutaway_inspection::inspect;
use cutaway_lenses::{BoundaryView, Cut, boundary_view};
use cutaway_planning::ports::plan_store::PlanStore;
use cutaway_planning::{
    GroupStanding, Modification, ModificationKind, Note, Plan, ProposedChange, SplitParts, Subject,
    addition_of_element,
};

use crate::fakes::{InMemoryPlanStore, InMemorySourceTree};

/// What every scenario may do to the application, stated in domain terms.
/// Boundaries and connections go by their display names, exactly as a user
/// sees them. Implementations decide which surface receives the actions.
pub trait ApplicationDriver {
    fn add_source_file(&mut self, path: &str, contents: &str);
    /// Lays a file down in one named version of the project's sources. The
    /// versions of a scenario are independent source trees; a version never
    /// named holds nothing.
    fn add_source_file_in_version(&mut self, version: &str, path: &str, contents: &str);
    fn inspect_project(&mut self) -> Result<(), String>;
    /// Reads the architecture anew: every boundary closed, every kind
    /// spoken.
    fn view_boundaries(&mut self) -> Result<(), String>;
    /// Inspects both named versions and views their comparison: the union
    /// architecture through a fresh cut, every boundary closed, every kind
    /// spoken.
    fn compare_versions(&mut self, before: &str, after: &str) -> Result<(), String>;
    /// How one rendered boundary reads in the comparison picture: "added",
    /// "removed", or "modified". None while the comparison leaves it alone.
    fn change_reading_of(&self, name: &str) -> Option<String>;
    /// How the rendered connection between two boundaries reads: "added" when
    /// every dependency behind it arrives, "removed" when every one goes,
    /// "changed" when the mix shifts while the connection stays, and
    /// "unchanged" while the comparison leaves it alone. None while the
    /// picture draws no such connection.
    fn connection_reading_of(&self, from: &str, to: &str) -> Option<String>;
    /// Opens every boundary at once, so the whole tree stands in the
    /// picture.
    fn open_all_boundaries(&mut self) -> Result<(), String>;
    /// Drops one kind from the picture's vocabulary. `kind` is the plural
    /// word a picture speaks: `packages`, `directories`, `modules`, `files`,
    /// `types`, or `functions`.
    fn hide_kind(&mut self, kind: &str) -> Result<(), String>;
    /// Puts one kind back in the picture's vocabulary, named as
    /// [`ApplicationDriver::hide_kind`] names it.
    fn show_kind(&mut self, kind: &str) -> Result<(), String>;
    /// Opens one boundary a step deeper than the rest of the picture.
    fn expand_boundary(&mut self, name: &str) -> Result<(), String>;
    /// Opens one boundary and everything beneath it in a single act.
    fn expand_boundary_fully(&mut self, name: &str) -> Result<(), String>;
    /// Closes one boundary a step back toward a single box.
    fn collapse_boundary(&mut self, name: &str) -> Result<(), String>;
    /// Scopes the picture to one boundary: it and the partners at its
    /// border, and nothing else.
    fn focus_boundary(&mut self, name: &str) -> Result<(), String>;
    /// Puts the whole project back in the picture.
    fn unfocus(&mut self);
    fn boundary_names(&self) -> Vec<String>;
    fn connections(&self) -> Vec<(String, String)>;
    fn sever_connection(&mut self, from: &str, to: &str) -> Result<(), String>;
    fn draw_connection(&mut self, from: &str, to: &str) -> Result<(), String>;
    fn annotate_connection(&mut self, from: &str, to: &str, note: &str) -> Result<(), String>;
    /// Marks one boundary, everything inside it, and the couplings crossing
    /// its border for removal.
    fn plan_element_removal(&mut self, name: &str) -> Result<(), String>;
    /// Withdraws a planned removal, the severings planned with it included.
    fn restore_element(&mut self, name: &str) -> Result<(), String>;
    /// Plans a new boundary inside another. `kind` is `module`, `type`, or
    /// `function`.
    fn add_element_inside(&mut self, parent: &str, kind: &str, name: &str) -> Result<(), String>;
    /// Plans a new package at the root of the project.
    fn add_package(&mut self, name: &str) -> Result<(), String>;
    /// States that one element keeps everything but its name.
    fn plan_rename(&mut self, name: &str, to: &str) -> Result<(), String>;
    /// States that one element becomes the several named here.
    fn plan_split(&mut self, name: &str, parts: &[&str]) -> Result<(), String>;
    /// States that one element folds into another that already exists.
    fn plan_merge(&mut self, name: &str, into: &str) -> Result<(), String>;
    /// States that the insides of one element change while its place does
    /// not. The note beside it carries the description.
    fn plan_rework(&mut self, name: &str) -> Result<(), String>;
    /// Leaves the element exactly as the sources have it.
    fn discard_modification(&mut self, name: &str) -> Result<(), String>;
    /// What the plan states about one element that stays, in words: "rename
    /// to engine", "split into engine, transport", "merge into engine", or
    /// "rework". None while the plan states nothing about it.
    fn modification_of(&self, name: &str) -> Option<String>;
    /// Writes a remark about one element: the rationale of the modification
    /// it carries, or - with none - a note on the element as it stands.
    fn annotate_element(&mut self, name: &str, note: &str) -> Result<(), String>;
    fn note_on_element(&self, name: &str) -> Option<String>;
    /// What one boundary directly holds, by display name.
    fn contents_of(&self, name: &str) -> Vec<String>;
    fn element_removal_is_planned(&self, name: &str) -> bool;
    fn element_addition_is_planned(&self, name: &str) -> bool;
    fn removal_is_planned(&self, from: &str, to: &str) -> bool;
    fn addition_is_planned(&self, from: &str, to: &str) -> bool;
    fn note_on_connection(&self, from: &str, to: &str) -> Option<String>;
    fn saved_plan(&self) -> Option<Plan>;
    fn working_plan(&self) -> Plan;
}

/// The kind a scenario names in words.
fn element_kind(kind: &str) -> Result<ElementKind, String> {
    match kind {
        "package" => Ok(ElementKind::Package),
        "directory" => Ok(ElementKind::Directory),
        "module" => Ok(ElementKind::Module),
        "file" => Ok(ElementKind::File),
        "type" => Ok(ElementKind::Type),
        "function" => Ok(ElementKind::Function),
        other => Err(format!("unknown element kind {other}")),
    }
}

/// The kind a scenario names by the plural word the picture speaks.
fn rendered_kind(kind: &str) -> Result<ElementKind, String> {
    match kind {
        "packages" => Ok(ElementKind::Package),
        "directories" => Ok(ElementKind::Directory),
        "modules" => Ok(ElementKind::Module),
        "files" => Ok(ElementKind::File),
        "types" => Ok(ElementKind::Type),
        "functions" => Ok(ElementKind::Function),
        other => Err(format!("unknown kind {other}")),
    }
}

/// Drives the application cores in-process, with the same analyzers the
/// composition root wires into the real application. Scenarios reach for
/// the ecosystem whose structure they speak about: Cargo manifests where a
/// package is the point, npm packages where a source directory is.
#[derive(Debug, Default)]
pub struct InProcessDriver {
    sources: InMemorySourceTree,
    /// The sources of each named version, one independent tree per version.
    /// A comparison inspects two of them; the unversioned `sources` stay out
    /// of it, so a scenario may explore one project and compare another.
    versioned_sources: BTreeMap<String, InMemorySourceTree>,
    /// The comparison the picture reads, while the picture reads one. It
    /// holds the union architecture the boundaries are cut from, and it
    /// answers how every boundary and every connection changed.
    comparing: Option<Comparison>,
    graph: Option<ArchitectureGraph>,
    /// The architecture the picture shows: the inspected graph with the
    /// plan's own additions drawn in. The lens reads this, and so does every
    /// question about where a planned element sits.
    viewed: Option<ArchitectureGraph>,
    cut: Option<Cut>,
    view: Option<BoundaryView>,
    plan: Plan,
    store: InMemoryPlanStore,
}

impl InProcessDriver {
    fn view(&self) -> &BoundaryView {
        self.view.as_ref().expect("a boundary view is active")
    }

    fn viewed(&self) -> &ArchitectureGraph {
        self.viewed.as_ref().expect("a project is inspected")
    }

    /// The architecture one named version holds, read with the analyzers the
    /// application wires in. A version no step ever named holds no files, and
    /// an empty tree inspects to an empty project rather than to a failure.
    fn architecture_of_version(&self, version: &str) -> Result<ArchitectureGraph, String> {
        let nothing = InMemorySourceTree::default();
        let sources = self.versioned_sources.get(version).unwrap_or(&nothing);
        inspect(sources, &[&RustSourceAnalyzer, &TypeScriptSourceAnalyzer])
            .map_err(|error| error.to_string())
    }

    /// The boundaries the picture draws, by id. A comparison reads a change
    /// at these and nowhere else: what changed out of sight speaks from the
    /// nearest one of them.
    fn rendered(&self) -> BTreeSet<ElementId> {
        self.view()
            .graph
            .elements()
            .map(|element| element.id.clone())
            .collect()
    }

    /// Paints the current cut anew, exactly as the GUI does after the reader
    /// opens or closes a boundary: the lens views the architecture with the
    /// plan's own additions drawn in, so a planned element and a drawn
    /// connection roll up at every cut the way the concrete ones do.
    ///
    /// A comparison is the exception: it reads what history did, while the
    /// plan speaks about what is to come, so the plan stays out of the
    /// comparison picture entirely.
    fn rebuild_view(&mut self) -> Result<(), String> {
        let cut = self.cut.as_ref().ok_or("no boundary view yet")?;
        // The comparison carries its own architecture - the union of the two
        // versions - so the inspected project stands untouched behind it, and
        // viewing the boundaries again returns to it unchanged.
        let viewed = if let Some(comparison) = &self.comparing {
            comparison.union().clone()
        } else {
            let graph = self.graph.as_ref().ok_or("no project inspected yet")?;
            self.plan.viewed_architecture(graph)
        };
        let view = boundary_view(&viewed, cut).map_err(|error| error.to_string())?;
        self.viewed = Some(viewed);
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
            return Err(format!("the boundary {name} opens or closes no further"));
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

    /// Puts one planned element in the plan and in the picture, exactly as
    /// the shell does: the element and its containment, then the view anew,
    /// then the plan on disk.
    fn plan_addition(
        &mut self,
        parent: Option<&ElementId>,
        kind: ElementKind,
        name: &str,
    ) -> Result<(), String> {
        let name = ElementName::new(name).map_err(|error| error.to_string())?;
        let changes =
            addition_of_element(parent, kind, &name).map_err(|error| error.to_string())?;
        for change in changes {
            self.plan
                .propose(change)
                .map_err(|error| error.to_string())?;
        }
        self.rebuild_view()?;
        self.save()
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

    /// States one modification and stores the plan. Only an element the
    /// sources declare may be modified: an element that lives only in the
    /// plan carries whatever the planner gave it, so the addition itself is
    /// what changes.
    fn plan_modification(&mut self, name: &str, kind: ModificationKind) -> Result<(), String> {
        let subject = self.boundary_id(name)?;
        let graph = self.graph.as_ref().ok_or("no project inspected yet")?;
        if graph.element(&subject).is_none() {
            return Err(format!(
                "{name} exists only in the plan; change the planned element itself"
            ));
        }
        self.plan.plan_modification(Modification {
            subject,
            kind,
            note: None,
        });
        self.save()
    }

    /// The name a scenario knows one element by.
    fn display_name(&self, id: &ElementId) -> String {
        self.viewed()
            .element(id)
            .map_or_else(|| id.to_string(), |element| element.name.to_string())
    }
}

impl ApplicationDriver for InProcessDriver {
    fn add_source_file(&mut self, path: &str, contents: &str) {
        let path = cutaway_inspection::ports::source_tree::SourcePath::new(path)
            .expect("scenarios use valid source paths");
        self.sources.add_file(path, contents);
    }

    fn add_source_file_in_version(&mut self, version: &str, path: &str, contents: &str) {
        let path = cutaway_inspection::ports::source_tree::SourcePath::new(path)
            .expect("scenarios use valid source paths");
        self.versioned_sources
            .entry(version.to_owned())
            .or_default()
            .add_file(path, contents);
    }

    fn inspect_project(&mut self) -> Result<(), String> {
        let graph = inspect(
            &self.sources,
            &[&RustSourceAnalyzer, &TypeScriptSourceAnalyzer],
        )
        .map_err(|error| error.to_string())?;
        // Inspecting a project is reading its present, so whatever comparison
        // stood before it ends here.
        self.comparing = None;
        self.viewed = Some(self.plan.viewed_architecture(&graph));
        self.graph = Some(graph);
        Ok(())
    }

    fn compare_versions(&mut self, before: &str, after: &str) -> Result<(), String> {
        let comparison = Comparison::between(
            &self.architecture_of_version(before)?,
            &self.architecture_of_version(after)?,
        );
        self.comparing = Some(comparison);
        // A comparison starts from a whole picture of its own: the scope of
        // whatever stood before it named a boundary of another architecture.
        self.cut = Some(Cut::whole());
        self.rebuild_view()
    }

    fn change_reading_of(&self, name: &str) -> Option<String> {
        let comparison = self.comparing.as_ref()?;
        let id = self.boundary_id(name).ok()?;
        let reading = *comparison.readings_at(&self.rendered()).get(&id)?;
        Some(
            match reading {
                ElementChange::Added => "added",
                ElementChange::Removed => "removed",
                ElementChange::Modified => "modified",
            }
            .to_owned(),
        )
    }

    fn connection_reading_of(&self, from: &str, to: &str) -> Option<String> {
        let comparison = self.comparing.as_ref()?;
        // The rendered connection stands for every dependency behind it, so
        // it reads as one only while all of them read alike.
        let behind = self.concrete_behind(from, to).ok()?;
        let presences: Vec<Presence> = behind
            .iter()
            .map(|relation| comparison.presence_of_relation(relation))
            .collect::<Option<Vec<Presence>>>()?;
        if presences.is_empty() {
            return None;
        }
        let all = |presence: Presence| presences.iter().all(|held| *held == presence);
        Some(
            if all(Presence::OnlyAfter) {
                "added"
            } else if all(Presence::OnlyBefore) {
                "removed"
            } else if all(Presence::InBoth) {
                "unchanged"
            } else {
                "changed"
            }
            .to_owned(),
        )
    }

    fn view_boundaries(&mut self) -> Result<(), String> {
        // Viewing the boundaries is reading one architecture, so the picture
        // leaves any comparison behind and shows the inspected project again.
        self.comparing = None;
        let mut cut = Cut::whole();
        // A fresh viewing drops the boundaries opened or closed before it:
        // those decisions answered the picture they were made in. The scope
        // stands through it: a viewing says how boundaries are read, never
        // which boundary is read.
        let scope = self.cut.as_ref().and_then(|cut| cut.scope.clone());
        cut.focus(scope);
        self.cut = Some(cut);
        self.rebuild_view()
    }

    fn open_all_boundaries(&mut self) -> Result<(), String> {
        // The frontier reaches every element of the architecture at once. A
        // flag on a leaf opens nothing and a flag on a stub is ignored, so
        // the whole tree is the shortest way to say "hold nothing back".
        let viewed = self.viewed.as_ref().ok_or("no project inspected yet")?;
        let open = viewed
            .elements()
            .map(|element| element.id.clone())
            .collect();
        let cut = self.cut.as_mut().ok_or("no boundary view yet")?;
        cut.open = open;
        self.rebuild_view()
    }

    fn hide_kind(&mut self, kind: &str) -> Result<(), String> {
        let kind = rendered_kind(kind)?;
        let cut = self.cut.as_mut().ok_or("no boundary view yet")?;
        cut.kinds.remove(&kind);
        self.rebuild_view()
    }

    fn show_kind(&mut self, kind: &str) -> Result<(), String> {
        let kind = rendered_kind(kind)?;
        let cut = self.cut.as_mut().ok_or("no boundary view yet")?;
        cut.kinds.insert(kind);
        self.rebuild_view()
    }

    fn expand_boundary(&mut self, name: &str) -> Result<(), String> {
        self.step_boundary(name, Cut::expand)
    }

    fn expand_boundary_fully(&mut self, name: &str) -> Result<(), String> {
        let id = self.boundary_id(name)?;
        let view = self.view.as_ref().ok_or("no boundary view yet")?;
        // The walk reaches beneath the picture, so it reads the architecture
        // the view was cut from rather than the view itself.
        let viewed = self.viewed.as_ref().ok_or("no project inspected yet")?;
        let cut = self.cut.as_mut().ok_or("no boundary view yet")?;
        if !cut.expand_fully(view, viewed, &id) {
            return Err(format!("the boundary {name} opens no further"));
        }
        self.rebuild_view()
    }

    fn collapse_boundary(&mut self, name: &str) -> Result<(), String> {
        self.step_boundary(name, Cut::collapse)
    }

    fn focus_boundary(&mut self, name: &str) -> Result<(), String> {
        let id = self.boundary_id(name)?;
        let cut = self.cut.as_mut().ok_or("no boundary view yet")?;
        cut.focus(Some(id));
        self.rebuild_view()
    }

    fn unfocus(&mut self) {
        let Some(cut) = self.cut.as_mut() else {
            return;
        };
        cut.focus(None);
        // The picture built while scoped, so it builds unscoped.
        self.rebuild_view().expect("the whole picture builds");
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
        let relation = self.connection(from, to)?;
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

    fn plan_element_removal(&mut self, name: &str) -> Result<(), String> {
        let id = self.boundary_id(name)?;
        let graph = self.graph.as_ref().ok_or("no project inspected yet")?;
        for change in self.plan.removal_of_element(&id, graph) {
            self.plan
                .propose(change)
                .map_err(|error| error.to_string())?;
        }
        self.save()
    }

    fn restore_element(&mut self, name: &str) -> Result<(), String> {
        let id = self.boundary_id(name)?;
        let graph = self.graph.as_ref().ok_or("no project inspected yet")?;
        let planned = self.plan.planned_removal_of_element(&id, graph);
        if planned.is_empty() {
            return Err(format!("no removal is planned for {name}"));
        }
        for change in planned {
            self.plan
                .retract(&change)
                .map_err(|error| error.to_string())?;
        }
        self.save()
    }

    fn add_element_inside(&mut self, parent: &str, kind: &str, name: &str) -> Result<(), String> {
        let parent = self.boundary_id(parent)?;
        self.plan_addition(Some(&parent), element_kind(kind)?, name)
    }

    fn add_package(&mut self, name: &str) -> Result<(), String> {
        // A package hangs under the project root, exactly as an inspected
        // one does.
        let root = self
            .graph
            .as_ref()
            .ok_or("no project inspected yet")?
            .elements()
            .find(|element| element.kind == ElementKind::Project)
            .map(|element| element.id.clone());
        self.plan_addition(root.as_ref(), ElementKind::Package, name)
    }

    fn plan_rename(&mut self, name: &str, to: &str) -> Result<(), String> {
        let to = ElementName::new(to).map_err(|error| error.to_string())?;
        self.plan_modification(name, ModificationKind::Rename { to })
    }

    fn plan_split(&mut self, name: &str, parts: &[&str]) -> Result<(), String> {
        let named: Result<Vec<ElementName>, String> = parts
            .iter()
            .map(|part| ElementName::new(*part).map_err(|error| error.to_string()))
            .collect();
        let into = SplitParts::new(named?).map_err(|error| error.to_string())?;
        self.plan_modification(name, ModificationKind::Split { into })
    }

    fn plan_merge(&mut self, name: &str, into: &str) -> Result<(), String> {
        let with = self.boundary_id(into)?;
        self.plan_modification(name, ModificationKind::Merge { with })
    }

    fn plan_rework(&mut self, name: &str) -> Result<(), String> {
        self.plan_modification(name, ModificationKind::Rework)
    }

    fn discard_modification(&mut self, name: &str) -> Result<(), String> {
        let subject = self.boundary_id(name)?;
        if self.plan.modification_of(&subject).is_none() {
            return Err(format!("no modification is planned for {name}"));
        }
        self.plan.discard_modification(&subject);
        self.save()
    }

    fn modification_of(&self, name: &str) -> Option<String> {
        let subject = self.boundary_id(name).ok()?;
        let modification = self.plan.modification_of(&subject)?;
        Some(match &modification.kind {
            ModificationKind::Rename { to } => format!("rename to {to}"),
            ModificationKind::Split { into } => format!(
                "split into {}",
                into.names()
                    .iter()
                    .map(ElementName::as_str)
                    .collect::<Vec<&str>>()
                    .join(", ")
            ),
            ModificationKind::Merge { with } => format!("merge into {}", self.display_name(with)),
            ModificationKind::Rework => "rework".to_owned(),
        })
    }

    fn annotate_element(&mut self, name: &str, note: &str) -> Result<(), String> {
        let subject = self.boundary_id(name)?;
        let note = Note::new(note).map_err(|error| error.to_string())?;
        // A modified element is described by the note on its modification:
        // one remark about one element, wherever the plan carries it.
        if let Some(modification) = self.plan.modification_of(&subject) {
            let described = Modification {
                note: Some(note),
                ..modification.clone()
            };
            self.plan.plan_modification(described);
        } else {
            self.plan.annotate(Subject::Element(subject), note);
        }
        self.save()
    }

    fn note_on_element(&self, name: &str) -> Option<String> {
        let subject = self.boundary_id(name).ok()?;
        match self.plan.modification_of(&subject) {
            Some(modification) => modification.note.as_ref(),
            None => self.plan.annotation_of(&Subject::Element(subject.clone())),
        }
        .map(|note| note.as_str().to_owned())
    }

    fn contents_of(&self, name: &str) -> Vec<String> {
        let Ok(id) = self.boundary_id(name) else {
            return Vec::new();
        };
        let view = self.view();
        view.graph
            .relations()
            .filter(|relation| relation.kind == RelationKind::Contains && relation.from == id)
            .filter_map(|relation| view.graph.element(&relation.to))
            .map(|element| element.name.to_string())
            .collect()
    }

    fn element_removal_is_planned(&self, name: &str) -> bool {
        self.boundary_id(name)
            .is_ok_and(|id| self.plan.removal_root_of(&id, self.viewed()).is_some())
    }

    fn element_addition_is_planned(&self, name: &str) -> bool {
        self.boundary_id(name)
            .is_ok_and(|id| self.plan.plans_addition_of_element(&id))
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
