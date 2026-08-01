// Cucumber's step macros pass captured arguments by value; the signatures
// are the macro contract, not a style choice.
#![allow(clippy::needless_pass_by_value)]

use cucumber::{World, given, then, when};
use cutaway_e2e::driver::{ApplicationDriver, InProcessDriver};

#[derive(Debug, Default, World)]
struct CutawayWorld {
    driver: InProcessDriver,
}

#[given(expr = "a project with a Rust file {string} containing a function {string}")]
fn a_rust_file_with_a_function(world: &mut CutawayWorld, path: String, function: String) {
    world
        .driver
        .add_source_file(&path, &format!("pub fn {function}() {{}}\n"));
}

#[when("the project is inspected")]
fn the_project_is_inspected(world: &mut CutawayWorld) {
    world.driver.inspect_project().expect("inspection succeeds");
}

#[then(expr = "the architecture contains an element named {string}")]
fn the_architecture_contains_an_element_named(world: &mut CutawayWorld, name: String) {
    let names = world.driver.element_names();
    assert!(
        names.contains(&name),
        "expected an element named {name:?}, the architecture contains {names:?}"
    );
}

#[tokio::main]
async fn main() {
    CutawayWorld::run("features").await;
}
