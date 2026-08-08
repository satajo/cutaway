// Cucumber's step macros pass captured arguments by value; the signatures
// are the macro contract, not a style choice.
#![allow(clippy::needless_pass_by_value)]

use cucumber::{World, gherkin::Step, given, then, when};
use cutaway_e2e::driver::{ApplicationDriver, InProcessDriver};

#[derive(Debug, Default, World)]
struct CutawayWorld {
    driver: InProcessDriver,
}

/// The manifest that declares one package.
fn manifest(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\n")
}

/// The manifest of a package that declares a dependency.
fn manifest_depending_on(name: &str, dependency: &str) -> String {
    format!(
        "{}\n[dependencies]\n{dependency} = {{ path = \"x\" }}\n",
        manifest(name)
    )
}

/// The root source file that exercises a declared dependency. Only a
/// dependency the code exercises appears in the picture.
fn root_using(dependency: &str) -> String {
    format!("use {};\n", dependency.replace('-', "_"))
}

/// The body a docstring step carries.
fn docstring(step: &Step) -> &str {
    step.docstring
        .as_ref()
        .expect("the step carries a docstring")
        .trim_start_matches('\n')
}

#[given(expr = "a package {string} at {string}")]
fn a_package(world: &mut CutawayWorld, name: String, dir: String) {
    world
        .driver
        .add_source_file(&format!("{dir}/Cargo.toml"), &manifest(&name));
}

// The step writes both the manifest declaration and a root source file that
// uses the dependency. A scenario that states its own root file replaces the
// latter.
#[given(expr = "a package {string} at {string} depending on {string}")]
fn a_package_depending_on(world: &mut CutawayWorld, name: String, dir: String, dependency: String) {
    world.driver.add_source_file(
        &format!("{dir}/Cargo.toml"),
        &manifest_depending_on(&name, &dependency),
    );
    world
        .driver
        .add_source_file(&format!("{dir}/src/lib.rs"), &root_using(&dependency));
}

#[given(expr = "a source file {string} containing:")]
fn a_source_file_containing(world: &mut CutawayWorld, path: String, step: &Step) {
    world.driver.add_source_file(&path, docstring(step));
}

#[given(expr = "in version {string} a package {string} at {string}")]
fn a_package_in_version(world: &mut CutawayWorld, version: String, name: String, dir: String) {
    world.driver.add_source_file_in_version(
        &version,
        &format!("{dir}/Cargo.toml"),
        &manifest(&name),
    );
}

#[given(expr = "in version {string} a package {string} at {string} depending on {string}")]
fn a_package_depending_on_in_version(
    world: &mut CutawayWorld,
    version: String,
    name: String,
    dir: String,
    dependency: String,
) {
    world.driver.add_source_file_in_version(
        &version,
        &format!("{dir}/Cargo.toml"),
        &manifest_depending_on(&name, &dependency),
    );
    world.driver.add_source_file_in_version(
        &version,
        &format!("{dir}/src/lib.rs"),
        &root_using(&dependency),
    );
}

#[given(expr = "in version {string} a source file {string} containing:")]
fn a_source_file_in_version_containing(
    world: &mut CutawayWorld,
    version: String,
    path: String,
    step: &Step,
) {
    world
        .driver
        .add_source_file_in_version(&version, &path, docstring(step));
}

#[when("the project is inspected")]
fn the_project_is_inspected(world: &mut CutawayWorld) {
    world.driver.inspect_project().expect("inspection succeeds");
}

#[when("the boundaries are viewed")]
fn the_boundaries_are_viewed(world: &mut CutawayWorld) {
    world
        .driver
        .view_boundaries()
        .expect("the boundary view builds");
}

#[when(expr = "the change from version {string} to version {string} is viewed")]
fn the_change_between_versions_is_viewed(world: &mut CutawayWorld, before: String, after: String) {
    world
        .driver
        .compare_versions(&before, &after)
        .expect("the comparison builds");
}

#[then(expr = "the boundary {string} reads as {string}")]
fn the_boundary_reads_as(world: &mut CutawayWorld, name: String, reading: String) {
    assert_eq!(world.driver.change_reading_of(&name), Some(reading));
}

#[then(expr = "the boundary {string} reads as unchanged")]
fn the_boundary_reads_as_unchanged(world: &mut CutawayWorld, name: String) {
    assert_eq!(world.driver.change_reading_of(&name), None);
}

#[then(expr = "the connection from {string} to {string} reads as {string}")]
fn the_connection_reads_as(world: &mut CutawayWorld, from: String, to: String, reading: String) {
    assert_eq!(
        world.driver.connection_reading_of(&from, &to),
        Some(reading)
    );
}

#[when("every boundary is opened")]
fn every_boundary_is_opened(world: &mut CutawayWorld) {
    world
        .driver
        .open_all_boundaries()
        .expect("the boundaries open");
}

/// The structure is the boxes the sources are organised in; the declarations
/// inside them are the detail a reader drops to see it.
#[when("only the structure is shown")]
fn only_the_structure_is_shown(world: &mut CutawayWorld) {
    for kind in ["types", "functions"] {
        world.driver.hide_kind(kind).expect("the kind hides");
    }
}

/// The directories and files a listing shows stand in every picture by
/// default. Dropping both leaves what the languages read: packages, modules,
/// and the declarations in them.
#[when("the file tree is hidden")]
fn the_file_tree_is_hidden(world: &mut CutawayWorld) {
    for kind in ["directories", "files"] {
        world.driver.hide_kind(kind).expect("the kind hides");
    }
}

/// What the languages read is packages, modules, and the declarations in
/// them. Dropping all of it leaves the directories and files a listing shows.
#[when("only the file tree is shown")]
fn only_the_file_tree_is_shown(world: &mut CutawayWorld) {
    for kind in ["packages", "modules", "types", "functions"] {
        world.driver.hide_kind(kind).expect("the kind hides");
    }
}

#[when(expr = "{string} are hidden from the picture")]
fn a_kind_is_hidden(world: &mut CutawayWorld, kind: String) {
    world.driver.hide_kind(&kind).expect("the kind hides");
}

#[when(expr = "{string} are shown in the picture")]
fn a_kind_is_shown(world: &mut CutawayWorld, kind: String) {
    world.driver.show_kind(&kind).expect("the kind shows");
}

#[when(expr = "the boundary {string} is expanded")]
fn the_boundary_is_expanded(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .expand_boundary(&name)
        .expect("the boundary expands");
}

#[when(expr = "the boundary {string} is opened fully")]
fn the_boundary_is_opened_fully(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .expand_boundary_fully(&name)
        .expect("the boundary opens");
}

#[when(expr = "the boundary {string} is collapsed")]
fn the_boundary_is_collapsed(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .collapse_boundary(&name)
        .expect("the boundary collapses");
}

#[when(expr = "the picture is focused on {string}")]
fn the_picture_is_focused_on(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .focus_boundary(&name)
        .expect("the picture focuses");
}

#[when("the whole picture is shown again")]
fn the_whole_picture_is_shown_again(world: &mut CutawayWorld) {
    world.driver.unfocus();
}

#[then(expr = "the boundary {string} cannot be expanded")]
fn the_boundary_cannot_be_expanded(world: &mut CutawayWorld, name: String) {
    assert!(
        world.driver.expand_boundary(&name).is_err(),
        "expected {name} to stay closed"
    );
}

#[when(expr = "the connection from {string} to {string} is severed")]
fn the_connection_is_severed(world: &mut CutawayWorld, from: String, to: String) {
    world
        .driver
        .sever_connection(&from, &to)
        .expect("severing succeeds");
}

#[when(expr = "a connection is drawn from {string} to {string}")]
fn a_connection_is_drawn(world: &mut CutawayWorld, from: String, to: String) {
    world
        .driver
        .draw_connection(&from, &to)
        .expect("drawing succeeds");
}

#[when(expr = "the connection from {string} to {string} is annotated with {string}")]
fn the_connection_is_annotated(world: &mut CutawayWorld, from: String, to: String, note: String) {
    world
        .driver
        .annotate_connection(&from, &to, &note)
        .expect("annotating succeeds");
}

#[when(expr = "the removal of {string} is planned")]
fn the_removal_of_an_element_is_planned(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .plan_element_removal(&name)
        .expect("planning the removal succeeds");
}

#[when(expr = "the removal of {string} is restored")]
fn the_removal_of_an_element_is_restored(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .restore_element(&name)
        .expect("restoring succeeds");
}

#[when(expr = "a {string} named {string} is planned inside {string}")]
fn an_element_is_planned_inside(
    world: &mut CutawayWorld,
    kind: String,
    name: String,
    parent: String,
) {
    world
        .driver
        .add_element_inside(&parent, &kind, &name)
        .expect("planning the element succeeds");
}

#[when(expr = "a package named {string} is planned")]
fn a_package_is_planned(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .add_package(&name)
        .expect("planning the package succeeds");
}

#[when(expr = "the rename of {string} to {string} is planned")]
fn the_rename_of_an_element_is_planned(world: &mut CutawayWorld, name: String, to: String) {
    world
        .driver
        .plan_rename(&name, &to)
        .expect("planning the rename succeeds");
}

/// An act the application may refuse. The scenario states what the plan
/// holds afterwards, which is the behavior either way.
#[when(expr = "the rename of {string} to {string} is attempted")]
fn the_rename_of_an_element_is_attempted(world: &mut CutawayWorld, name: String, to: String) {
    let _ = world.driver.plan_rename(&name, &to);
}

#[when(expr = "the split of {string} into {string} is planned")]
fn the_split_of_an_element_is_planned(world: &mut CutawayWorld, name: String, parts: String) {
    let parts: Vec<&str> = parts.split(',').map(str::trim).collect();
    world
        .driver
        .plan_split(&name, &parts)
        .expect("planning the split succeeds");
}

#[when(expr = "the merge of {string} into {string} is planned")]
fn the_merge_of_an_element_is_planned(world: &mut CutawayWorld, name: String, into: String) {
    world
        .driver
        .plan_merge(&name, &into)
        .expect("planning the merge succeeds");
}

#[when(expr = "the rework of {string} is planned")]
fn the_rework_of_an_element_is_planned(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .plan_rework(&name)
        .expect("planning the rework succeeds");
}

#[when(expr = "the modification of {string} is discarded")]
fn the_modification_of_an_element_is_discarded(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .discard_modification(&name)
        .expect("discarding succeeds");
}

#[when(expr = "the element {string} is annotated with {string}")]
fn the_element_is_annotated(world: &mut CutawayWorld, name: String, note: String) {
    world
        .driver
        .annotate_element(&name, &note)
        .expect("annotating succeeds");
}

#[then(expr = "the plan modifies {string} by {string}")]
fn the_plan_modifies_an_element(world: &mut CutawayWorld, name: String, stated: String) {
    assert_eq!(world.driver.modification_of(&name), Some(stated));
}

#[then(expr = "the plan modifies {string} in no way")]
fn the_plan_modifies_an_element_in_no_way(world: &mut CutawayWorld, name: String) {
    assert_eq!(world.driver.modification_of(&name), None);
}

#[then(expr = "the element {string} carries the note {string}")]
fn the_element_carries_the_note(world: &mut CutawayWorld, name: String, note: String) {
    assert_eq!(world.driver.note_on_element(&name), Some(note));
}

#[then(expr = "the plan marks {string} for removal")]
fn the_plan_marks_an_element_for_removal(world: &mut CutawayWorld, name: String) {
    assert!(world.driver.element_removal_is_planned(&name));
}

#[then(expr = "the plan does not mark {string} for removal")]
fn the_plan_does_not_mark_an_element_for_removal(world: &mut CutawayWorld, name: String) {
    assert!(!world.driver.element_removal_is_planned(&name));
}

#[then(expr = "the plan proposes an element {string}")]
fn the_plan_proposes_an_element(world: &mut CutawayWorld, name: String) {
    assert!(world.driver.element_addition_is_planned(&name));
}

#[then(expr = "the boundary {string} contains {string}")]
fn the_boundary_contains(world: &mut CutawayWorld, frame: String, inside: String) {
    let contents = world.driver.contents_of(&frame);
    assert!(
        contents.contains(&inside),
        "expected {frame} to contain {inside}, it holds {contents:?}"
    );
}

#[then(expr = "the boundary {string} holds nothing")]
fn the_boundary_holds_nothing(world: &mut CutawayWorld, frame: String) {
    let contents = world.driver.contents_of(&frame);
    assert!(
        contents.is_empty(),
        "expected {frame} to hold nothing, it holds {contents:?}"
    );
}

#[then(expr = "the boundaries are {string}")]
fn the_boundaries_are(world: &mut CutawayWorld, expected: String) {
    let mut names = world.driver.boundary_names();
    names.sort();
    let expected: Vec<String> = expected.split(", ").map(str::to_owned).collect();
    assert_eq!(names, expected);
}

#[then(expr = "the boundaries include {string}")]
fn the_boundaries_include(world: &mut CutawayWorld, expected: String) {
    let names = world.driver.boundary_names();
    assert!(
        names.contains(&expected),
        "expected a boundary {expected}, the view has {names:?}"
    );
}

#[then(expr = "the boundaries do not include {string}")]
fn the_boundaries_do_not_include(world: &mut CutawayWorld, expected: String) {
    let names = world.driver.boundary_names();
    assert!(
        !names.contains(&expected),
        "expected no boundary {expected}, the view has {names:?}"
    );
}

#[then(expr = "a connection goes from {string} to {string}")]
fn a_connection_goes(world: &mut CutawayWorld, from: String, to: String) {
    let connections = world.driver.connections();
    assert!(
        connections.contains(&(from.clone(), to.clone())),
        "expected a connection {from} -> {to}, the view has {connections:?}"
    );
}

#[then(expr = "no connection goes from {string} to {string}")]
fn no_connection_goes(world: &mut CutawayWorld, from: String, to: String) {
    let connections = world.driver.connections();
    assert!(
        !connections.contains(&(from.clone(), to.clone())),
        "expected no connection {from} -> {to}, the view has {connections:?}"
    );
}

#[then(expr = "the plan marks the connection from {string} to {string} for removal")]
fn the_plan_marks_for_removal(world: &mut CutawayWorld, from: String, to: String) {
    assert!(world.driver.removal_is_planned(&from, &to));
}

#[then(expr = "the plan leaves the connection from {string} to {string} alone")]
fn the_plan_leaves_a_connection_alone(world: &mut CutawayWorld, from: String, to: String) {
    assert!(!world.driver.removal_is_planned(&from, &to));
}

#[then(expr = "the plan proposes a connection from {string} to {string}")]
fn the_plan_proposes(world: &mut CutawayWorld, from: String, to: String) {
    assert!(world.driver.addition_is_planned(&from, &to));
}

#[then(expr = "the connection from {string} to {string} carries the note {string}")]
fn the_connection_carries_the_note(
    world: &mut CutawayWorld,
    from: String,
    to: String,
    note: String,
) {
    assert_eq!(world.driver.note_on_connection(&from, &to), Some(note));
}

#[then("the saved plan equals the working plan")]
fn the_saved_plan_equals_the_working_plan(world: &mut CutawayWorld) {
    assert_eq!(
        world.driver.saved_plan().as_ref(),
        Some(&world.driver.working_plan())
    );
}

#[tokio::main]
async fn main() {
    CutawayWorld::run("features").await;
}
