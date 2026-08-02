//! The search palette: a name typed, and the picture opens where it lives.
//!
//! The palette searches the whole architecture, never the picture. A reader
//! who cuts the project at packages still asks for one type by name, and the
//! answer must be that type rather than a shrug. Accepting a result
//! therefore opens every boundary between the picture and the element - one
//! override per step of the containment chain - and only then makes the
//! element the selection the camera moves to.
//!
//! A query matches a name whose letters it holds in order, gaps allowed, in
//! either case. The ranking follows how directly the name answers: the name
//! typed in full, then a name beginning with the query, then a name whose
//! words begin with its letters, then a name merely holding them somewhere.
//! Ties go to the shorter name, which is the narrower answer, and then to
//! the id, so the same query always answers the same way.
//!
//! Matching, ranking, the container path of a result and the override
//! planning are pure functions of the graph and the query. The painter at
//! the end of this file only shows what they answer.

use std::collections::BTreeSet;

use cutaway_architecture::{ArchitectureGraph, ElementId, ElementKind};
use cutaway_lenses::Detail;
use eframe::egui;
use eframe::egui::text::{CCursor, CCursorRange};

use crate::focus;
use crate::label::kind_symbol;

/// How many results one glance reads. Past this a reader types another
/// letter rather than looks further down.
const RESULT_LIMIT: usize = 12;

/// What stands between the containers above a result.
const CONTAINER_STEP: &str = " › ";

/// How wide the palette floats, in points.
const WIDTH: f32 = 460.0;

/// How far below the top of the canvas the palette floats.
const TOP_MARGIN: f32 = 24.0;

/// The palette between frames.
///
/// The query outlives a closing: a reader who searched for one boundary
/// searches near it again, so re-opening offers the last query with the
/// whole of it selected, and the first keystroke replaces it.
#[derive(Default)]
pub(crate) struct Palette {
    open: bool,
    query: String,
    /// Which result the keyboard points at, counted from the top.
    highlighted: usize,
    /// True for the single frame that opens the palette, which is when the
    /// field takes the keyboard and the old query becomes its selection.
    opening: bool,
}

impl Palette {
    /// Whether the palette holds the keyboard. What the shell does with a
    /// bare key depends on it: an open palette answers every one of them.
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn open(&mut self) {
        self.open = true;
        self.opening = true;
        self.highlighted = 0;
    }

    fn close(&mut self) {
        self.open = false;
        self.opening = false;
    }
}

/// How directly a name answers a query. The order is the ranking: the
/// greater quality answers more directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Quality {
    /// The name holds the letters of the query in order, anything between
    /// them.
    Scattered,
    /// Every letter of the query starts a word of the name.
    WordBoundary,
    /// The name begins with the query.
    Prefix,
    /// The name is the query.
    Exact,
}

/// How well one name answers a query, and None when it does not answer at
/// all. Case never matters. An empty query asks nothing, and so names
/// nothing.
pub(crate) fn quality(name: &str, query: &str) -> Option<Quality> {
    let query: Vec<char> = query.chars().map(folded).collect();
    if query.is_empty() {
        return None;
    }
    let name = Name::of(name);
    if name.letters == query {
        return Some(Quality::Exact);
    }
    if name.letters.starts_with(&query) {
        return Some(Quality::Prefix);
    }
    if !name.holds(&query, Landing::Anywhere) {
        return None;
    }
    if name.holds(&query, Landing::AtWordStarts) {
        Some(Quality::WordBoundary)
    } else {
        Some(Quality::Scattered)
    }
}

/// Where the letters of a query may land in a name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Landing {
    Anywhere,
    /// Only where a word of the name begins.
    AtWordStarts,
}

/// One name, ready to match: every letter folded to lower case, and where
/// the words of the name begin.
struct Name {
    letters: Vec<char>,
    word_start: Vec<bool>,
}

impl Name {
    fn of(name: &str) -> Self {
        Self {
            letters: name.chars().map(folded).collect(),
            word_start: word_starts(name),
        }
    }

    /// Whether the letters of the query appear in order among the positions
    /// the walk admits. Taking the earliest admissible position for every
    /// letter never loses a later one, so a single greedy pass decides it.
    fn holds(&self, query: &[char], landing: Landing) -> bool {
        let mut wanted = query.iter().peekable();
        for (position, letter) in self.letters.iter().enumerate() {
            let Some(want) = wanted.peek() else {
                return true;
            };
            if landing == Landing::AtWordStarts && !self.word_start[position] {
                continue;
            }
            if letter == *want {
                wanted.next();
            }
        }
        wanted.peek().is_none()
    }
}

/// Where the words of a name begin: its first letter, every letter behind a
/// separator, and every capital that follows a lower-case letter. Sources
/// write names both ways - `source_analyzer` and `SourceAnalyzer` - and a
/// reader types the initials of either.
fn word_starts(name: &str) -> Vec<bool> {
    let mut starts = Vec::new();
    let mut previous: Option<char> = None;
    for character in name.chars() {
        let start = character.is_alphanumeric()
            && match previous {
                None => true,
                Some(before) => {
                    !before.is_alphanumeric() || (character.is_uppercase() && before.is_lowercase())
                }
            };
        starts.push(start);
        previous = Some(character);
    }
    starts
}

/// One letter as matching reads it. A letter whose lower case spells more
/// than one letter keeps its first: the palette compares names, not text.
fn folded(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}

/// One element the palette offers.
pub(crate) struct Hit {
    pub(crate) id: ElementId,
    pub(crate) name: String,
    pub(crate) kind: ElementKind,
    /// The boundaries above it, outermost first, on one line. Empty when
    /// nothing above it has a name worth showing.
    pub(crate) container: String,
}

/// The elements answering a query, the most direct answer first.
///
/// The search reads the whole graph however the picture is cut: an element
/// hidden inside a closed package is exactly what a reader searches for. The
/// project root stands for the whole picture and answers nothing, so it
/// alone stays out.
pub(crate) fn hits(graph: &ArchitectureGraph, query: &str) -> Vec<Hit> {
    let mut ranked: Vec<_> = graph
        .elements()
        .filter(|element| element.kind != ElementKind::Project)
        .filter_map(|element| Some((quality(element.name.as_str(), query)?, element)))
        .collect();
    ranked.sort_by(|(left_quality, left), (right_quality, right)| {
        right_quality
            .cmp(left_quality)
            .then_with(|| left.name.as_str().len().cmp(&right.name.as_str().len()))
            .then_with(|| left.id.cmp(&right.id))
    });
    let frames = focus::frames_of(graph);
    ranked
        .into_iter()
        .take(RESULT_LIMIT)
        .map(|(_, element)| Hit {
            id: element.id.clone(),
            name: element.name.to_string(),
            kind: element.kind,
            container: container_of(graph, &frames, &element.id),
        })
        .collect()
}

/// The names of the boundaries above an element, outermost first. The
/// project root names the whole picture, and a line that says it of every
/// result says nothing, so it stays out.
fn container_of(graph: &ArchitectureGraph, frames: &focus::Frames<'_>, id: &ElementId) -> String {
    let mut names = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = frames.get(id).copied();
    while let Some(frame) = current {
        if !seen.insert(frame) {
            break;
        }
        if let Some(element) = graph.element(frame)
            && element.kind != ElementKind::Project
        {
            names.push(element.name.to_string());
        }
        current = frames.get(frame).copied();
    }
    names.reverse();
    names.join(CONTAINER_STEP)
}

/// The overrides that open a picture down to one element: every boundary
/// above it opens far enough to show the next step toward it.
///
/// The plan reads outermost first, as opening one box after another does. A
/// boundary no detail ever shows - the project root - carries no override,
/// because an override governs the inside of a boundary the reader can see,
/// and that one never appears.
pub(crate) fn overrides_revealing(
    graph: &ArchitectureGraph,
    target: &ElementId,
) -> Vec<(ElementId, Detail)> {
    let frames = focus::frames_of(graph);
    let mut planned = Vec::new();
    // Containment is a tree, but a walk that trusts that and meets a cycle
    // never ends; the seen set bounds it.
    let mut seen = BTreeSet::new();
    let mut current = target;
    while let Some(frame) = frames.get(current).copied() {
        if !seen.insert(frame) {
            break;
        }
        let step = graph
            .element(current)
            .and_then(|element| Detail::showing(element.kind));
        let frame_shows = graph
            .element(frame)
            .is_some_and(|element| Detail::showing(element.kind).is_some());
        if let Some(detail) = step
            && frame_shows
        {
            planned.push((frame.clone(), detail));
        }
        current = frame;
    }
    planned.reverse();
    planned
}

/// What a key asks of an open palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Close,
    Previous,
    Next,
    Accept,
}

/// The keys an open palette owns, and what each of them asks.
const COMMANDS: [(egui::Key, Command); 4] = [
    (egui::Key::Escape, Command::Close),
    (egui::Key::ArrowUp, Command::Previous),
    (egui::Key::ArrowDown, Command::Next),
    (egui::Key::Enter, Command::Accept),
];

/// What this frame's keys ask of the palette. Taking them here, before any
/// widget of the frame paints, is what keeps them the palette's own: an
/// arrow moves the highlighted row rather than the text cursor, and Escape
/// closes the palette without reaching anything behind it.
fn commands(ctx: &egui::Context) -> Vec<Command> {
    ctx.input_mut(|input| {
        COMMANDS
            .into_iter()
            .filter(|(key, _)| input.consume_key(egui::Modifiers::NONE, *key))
            .map(|(_, command)| command)
            .collect()
    })
}

/// Runs the palette for one frame: the keys that open, move and close it,
/// and the panel it floats over the canvas while open. Answers with the
/// element the reader accepted, and with None every other frame.
pub(crate) fn show(
    ctx: &egui::Context,
    palette: &mut Palette,
    graph: &ArchitectureGraph,
    canvas: egui::Rect,
) -> Option<ElementId> {
    // Ctrl+F opens the palette wherever the keyboard is, a half-written note
    // included: the combination edits no text in egui, and searching is the
    // way out of a picture too large to read by eye. A bare slash is an
    // ordinary character, so it opens the palette only while no text field
    // would otherwise swallow it.
    let opens = ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        || (!ctx.text_edit_focused()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Slash)));
    if opens {
        palette.open();
    }
    if !palette.open {
        return None;
    }
    let asked = commands(ctx);
    if asked.contains(&Command::Close) {
        palette.close();
        return None;
    }

    // The palette belongs over the picture, not over the panels beside it.
    let over = if canvas.is_positive() {
        canvas
    } else {
        ctx.content_rect()
    };
    let opening = palette.opening;
    let painted = egui::Area::new(egui::Id::new("search palette"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(
            over.center().x - WIDTH / 2.0,
            over.top() + TOP_MARGIN,
        ))
        .constrain_to(ctx.content_rect())
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    field(ui, palette);
                    let hits = hits(graph, &palette.query);
                    move_highlight(palette, &asked, hits.len());
                    rows(ui, palette, &hits, asked.contains(&Command::Accept))
                })
                .inner
        });

    // A click beside the palette is a click on something else, and that
    // answer replaces the question the palette asked. The frame that opens
    // it is exempt: the toolbar button is itself such a click.
    let clicked_beside = !opening
        && ctx.input(|input| {
            input.pointer.any_click()
                && input
                    .pointer
                    .interact_pos()
                    .is_none_or(|position| !painted.response.rect.contains(position))
        });
    let accepted = painted.inner;
    if accepted.is_some() || clicked_beside {
        palette.close();
    }
    accepted
}

/// The query field, which takes the keyboard the moment the palette opens.
fn field(ui: &mut egui::Ui, palette: &mut Palette) {
    let output = egui::TextEdit::singleline(&mut palette.query)
        .hint_text("Find a boundary by name")
        .desired_width(f32::INFINITY)
        .show(ui);
    if output.response.changed() {
        // A new query is a new question, so the answer to it starts at the
        // top.
        palette.highlighted = 0;
    }
    if !palette.opening {
        return;
    }
    palette.opening = false;
    // Focus lands on the next frame, so ask for that frame: the reader
    // pressed a key and must see a field ready for the next one.
    output.response.request_focus();
    ui.ctx().request_repaint();
    // The last query stands selected, so the first keystroke replaces it and
    // a reader who wanted it kept simply presses an arrow first.
    let whole = CCursorRange::two(CCursor::new(0), CCursor::new(palette.query.chars().count()));
    let mut state = output.state;
    state.cursor.set_char_range(Some(whole));
    state.store(ui.ctx(), output.response.id);
}

/// Moves the highlighted row, and keeps it on a row that exists: a shorter
/// answer to a longer query must never leave the highlight past its end.
fn move_highlight(palette: &mut Palette, asked: &[Command], results: usize) {
    palette.highlighted = palette.highlighted.min(results.saturating_sub(1));
    if asked.contains(&Command::Previous) {
        palette.highlighted = palette.highlighted.saturating_sub(1);
    }
    if asked.contains(&Command::Next) && palette.highlighted + 1 < results {
        palette.highlighted += 1;
    }
}

/// The results, and the one the reader accepted by key or by click.
fn rows(ui: &mut egui::Ui, palette: &Palette, hits: &[Hit], accept: bool) -> Option<ElementId> {
    if hits.is_empty() {
        ui.small(if palette.query.is_empty() {
            "Type a name."
        } else {
            "No matches."
        });
        return None;
    }
    let mut accepted = accept
        .then(|| hits.get(palette.highlighted))
        .flatten()
        .map(|hit| hit.id.clone());
    for (position, hit) in hits.iter().enumerate() {
        if row(ui, hit, position == palette.highlighted).clicked() {
            accepted = Some(hit.id.clone());
        }
    }
    ui.small("↑↓ choose · Enter opens · Esc closes");
    accepted
}

/// One result: what it is, what it is called, and where it lives. The
/// container reads dimmer than the name, because the name is the answer and
/// the container only tells two alike names apart.
fn row(ui: &mut egui::Ui, hit: &Hit, highlighted: bool) -> egui::Response {
    let width = ui.available_width();
    let name = format!("{} {}", kind_symbol(hit.kind), hit.name);
    let container = egui::RichText::new(hit.container.as_str()).weak().small();
    ui.add(
        egui::Button::selectable(highlighted, (name, container, egui::Atom::grow()))
            .truncate()
            .min_size(egui::vec2(width, 0.0)),
    )
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::{Element, ElementName, Relation, RelationKind};

    use super::*;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn add(graph: &mut ArchitectureGraph, id_text: &str, name: &str, kind: ElementKind) {
        graph
            .add_element(Element {
                id: id(id_text),
                name: ElementName::new(name).unwrap(),
                kind,
            })
            .unwrap();
    }

    fn contain(graph: &mut ArchitectureGraph, frame: &str, inner: &str) {
        graph
            .add_relation(Relation {
                from: id(frame),
                to: id(inner),
                kind: RelationKind::Contains,
            })
            .unwrap();
    }

    /// project ⊃ `package:inspection` ⊃ `inspection/ports.rs` ⊃
    /// `inspection/ports/source_tree.rs` ⊃ the type `SourceTree`.
    fn graph() -> ArchitectureGraph {
        let mut graph = ArchitectureGraph::new();
        add(
            &mut graph,
            "project:cutaway",
            "cutaway",
            ElementKind::Project,
        );
        add(
            &mut graph,
            "package:inspection",
            "inspection",
            ElementKind::Package,
        );
        add(
            &mut graph,
            "inspection/ports.rs",
            "ports",
            ElementKind::Module,
        );
        add(
            &mut graph,
            "inspection/ports/source_tree.rs",
            "ports::source_tree",
            ElementKind::Module,
        );
        add(
            &mut graph,
            "inspection/ports/source_tree.rs#type:SourceTree",
            "SourceTree",
            ElementKind::Type,
        );
        contain(&mut graph, "project:cutaway", "package:inspection");
        contain(&mut graph, "package:inspection", "inspection/ports.rs");
        contain(
            &mut graph,
            "inspection/ports.rs",
            "inspection/ports/source_tree.rs",
        );
        contain(
            &mut graph,
            "inspection/ports/source_tree.rs",
            "inspection/ports/source_tree.rs#type:SourceTree",
        );
        graph
    }

    fn names(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.name.as_str()).collect()
    }

    #[test]
    fn a_query_matches_a_name_holding_its_letters_in_order() {
        assert_eq!(quality("SourceTree", "srtr"), Some(Quality::Scattered));
    }

    #[test]
    fn a_query_whose_letters_are_out_of_order_matches_nothing() {
        assert_eq!(quality("SourceTree", "treesource"), None);
    }

    #[test]
    fn matching_ignores_the_case_of_the_name_and_of_the_query() {
        assert_eq!(quality("SourceTree", "SOURCETREE"), Some(Quality::Exact));
        assert_eq!(quality("sourcetree", "SourceTree"), Some(Quality::Exact));
    }

    #[test]
    fn an_empty_query_names_nothing() {
        assert_eq!(quality("SourceTree", ""), None);
    }

    #[test]
    fn an_exact_name_outranks_a_prefix() {
        assert!(quality("plan", "plan") > quality("planning", "plan"));
    }

    #[test]
    fn a_prefix_outranks_a_word_boundary_match() {
        assert!(quality("planning", "plan") > quality("plan_and_note", "pan"));
    }

    #[test]
    fn a_word_boundary_match_outranks_a_scattered_one() {
        assert!(quality("source_tree", "st") > quality("constant", "st"));
    }

    #[test]
    fn the_initials_of_a_camel_case_name_are_word_boundaries_too() {
        assert_eq!(quality("SourceTree", "st"), Some(Quality::WordBoundary));
    }

    #[test]
    fn a_path_separator_starts_a_word() {
        assert_eq!(
            quality("ports::source_tree", "pst"),
            Some(Quality::WordBoundary)
        );
    }

    #[test]
    fn the_most_direct_answer_comes_first() {
        assert_eq!(
            names(&hits(&graph(), "sourcetree")),
            ["SourceTree", "ports::source_tree"]
        );
    }

    #[test]
    fn equally_direct_answers_rank_the_shorter_name_first() {
        let mut graph = ArchitectureGraph::new();
        add(&mut graph, "long", "planning_of_notes", ElementKind::Module);
        add(&mut graph, "short", "planning", ElementKind::Module);
        assert_eq!(
            names(&hits(&graph, "plan")),
            ["planning", "planning_of_notes"]
        );
    }

    #[test]
    fn a_result_names_the_boundaries_above_it() {
        let found = hits(&graph(), "SourceTree");
        assert_eq!(
            found[0].container, "inspection › ports › ports::source_tree",
            "the project root names the whole picture and so stays out"
        );
    }

    #[test]
    fn the_project_root_never_answers_a_query() {
        assert!(hits(&graph(), "cutaway").is_empty());
    }

    #[test]
    fn the_palette_offers_no_more_results_than_one_glance_reads() {
        let mut graph = ArchitectureGraph::new();
        for number in 0..RESULT_LIMIT + 5 {
            add(
                &mut graph,
                &format!("module:{number}"),
                &format!("planner{number}"),
                ElementKind::Module,
            );
        }
        assert_eq!(hits(&graph, "plan").len(), RESULT_LIMIT);
    }

    #[test]
    fn revealing_a_hidden_item_plans_overrides_down_its_ancestry() {
        assert_eq!(
            overrides_revealing(
                &graph(),
                &id("inspection/ports/source_tree.rs#type:SourceTree")
            ),
            vec![
                (id("package:inspection"), Detail::Modules),
                (id("inspection/ports.rs"), Detail::Modules),
                (id("inspection/ports/source_tree.rs"), Detail::Items),
            ],
            "each boundary opens far enough to show the next step toward the item"
        );
    }

    #[test]
    fn revealing_a_package_plans_no_overrides_at_all() {
        assert_eq!(
            overrides_revealing(&graph(), &id("package:inspection")),
            vec![],
            "every picture shows packages, and the root governs nothing"
        );
    }
}
