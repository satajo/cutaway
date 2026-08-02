// Cucumber's step macros pass captured arguments by value; the signatures
// are the macro contract, not a style choice.
#![allow(clippy::needless_pass_by_value)]

use cucumber::{World, gherkin::Step, given, then, when};
use cutaway_e2e::driver::{ApplicationDriver, InProcessDriver};

#[derive(Debug, Default, World)]
struct CutawayWorld {
    driver: InProcessDriver,
}

#[given(expr = "a package {string} at {string}")]
fn a_package(world: &mut CutawayWorld, name: String, dir: String) {
    world.driver.add_source_file(
        &format!("{dir}/Cargo.toml"),
        &format!("[package]\nname = \"{name}\"\n"),
    );
}

#[given(expr = "a package {string} at {string} depending on {string}")]
fn a_package_depending_on(world: &mut CutawayWorld, name: String, dir: String, dependency: String) {
    world.driver.add_source_file(
        &format!("{dir}/Cargo.toml"),
        &format!(
            "[package]\nname = \"{name}\"\n\n[dependencies]\n{dependency} = {{ path = \"x\" }}\n"
        ),
    );
}

#[given(expr = "a source file {string} containing:")]
fn a_source_file_containing(world: &mut CutawayWorld, path: String, step: &Step) {
    let contents = step
        .docstring
        .as_ref()
        .expect("the step carries a docstring");
    world
        .driver
        .add_source_file(&path, contents.trim_start_matches('\n'));
}

#[when("the project is inspected")]
fn the_project_is_inspected(world: &mut CutawayWorld) {
    world.driver.inspect_project().expect("inspection succeeds");
}

#[when(expr = "the boundaries are viewed at {string} level")]
fn the_boundaries_are_viewed(world: &mut CutawayWorld, level: String) {
    world
        .driver
        .view_boundaries(&level)
        .expect("the boundary view builds");
}

#[when(expr = "the boundary {string} is expanded")]
fn the_boundary_is_expanded(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .expand_boundary(&name)
        .expect("the boundary expands");
}

#[when(expr = "the boundary {string} is collapsed")]
fn the_boundary_is_collapsed(world: &mut CutawayWorld, name: String) {
    world
        .driver
        .collapse_boundary(&name)
        .expect("the boundary collapses");
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
