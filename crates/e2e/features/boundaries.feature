Feature: The boundary lens

  The architecture appears as boundaries, and the reader shapes the picture
  in two independent ways. Every boundary starts as a closed box, and
  opening one reveals exactly one layer: its contents arrive closed, and
  opening each of them is a step of its own. Beside that stands the
  vocabulary of kinds the picture speaks - packages, directories, files,
  modules, types, functions. A hidden kind draws nothing and hands its
  contents to the box above it, so hiding modules pools their declarations in
  the package itself.

  The directories and files a listing shows are the skeleton of every
  project, so they stand in the picture by default. Hiding both leaves what
  the languages read, and that reading is what most of these scenarios speak
  about.

  Connections attach to the nearest visible boundary, single boxes
  and boundaries with visible children alike. A connection that ends at a
  boundary's border speaks about the boundary's own code or the boundary as
  a whole; the connections into its parts end at the parts. What passes
  between a boundary and its own contents stays inside it. A package's root
  source file is the package's own code, not a boundary of its own: its
  declarations are the package's items and its child modules sit directly
  inside the package.

  Scenario: A dependency between packages appears as a connection
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    When the project is inspected
    And the boundaries are viewed
    And the file tree is hidden
    Then the boundaries are "app, engine"
    And a connection goes from "app" to "engine"

  Scenario: A manifest dependency no code exercises draws no connection
    Given a source file "crates/app/Cargo.toml" containing:
      """
      [package]
      name = "app"

      [dependencies]
      engine = { path = "../engine" }
      """
    And a package "engine" at "crates/engine"
    When the project is inspected
    And the boundaries are viewed
    And the file tree is hidden
    Then no connection goes from "app" to "engine"

  Scenario: Imports in code roll up to package connections
    Given a package "app" at "crates/app"
    And a package "engine" at "crates/engine"
    And a source file "crates/app/src/lib.rs" containing:
      """
      use engine::run;
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      pub fn run() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And the file tree is hidden
    Then a connection goes from "app" to "engine"

  Scenario: A boundary's own code connects from the boundary itself
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    And a source file "crates/app/src/lib.rs" containing:
      """
      mod wiring;
      use engine::run;
      """
    And a source file "crates/app/src/wiring.rs" containing:
      """
      """
    And a source file "crates/app/tests/behaviour.rs" containing:
      """
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      pub fn run() {}
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And only the structure is shown
    And the file tree is hidden
    Then a connection goes from "app" to "engine"
    And no connection goes from "wiring" to "engine"

  Scenario: The root source file's modules sit directly inside the package
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      mod render;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    And a source file "crates/engine/src/render.rs" containing:
      """
      pub fn draw() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And only the structure is shown
    And the file tree is hidden
    Then the boundaries do not include "crate"
    And the boundary "engine" contains "physics"
    And the boundary "engine" contains "render"

  Scenario: An integration test file stands as a file of the package
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      use crate::physics::step;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    And a source file "crates/engine/tests/behaviour.rs" containing:
      """
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And only the structure is shown
    Then the boundaries do not include "crate"
    And the boundary "engine" contains "tests/behaviour.rs"
    When the file tree is hidden
    Then the boundary "engine" contains "physics"

  Scenario: What passes between a boundary and its own contents stays inside it
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      mod tests;
      pub fn run() {}
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    And a source file "crates/engine/src/tests.rs" containing:
      """
      use crate::run;
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And only the structure is shown
    And the file tree is hidden
    Then the boundaries include "tests"
    And no connection goes from "tests" to "engine"

  Scenario: A dependency on a whole boundary shows at every detail
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And only the structure is shown
    And the file tree is hidden
    Then a connection goes from "app" to "engine"
    When the boundaries are viewed
    And the file tree is hidden
    Then a connection goes from "app" to "engine"

  Scenario: One package opens deeper while the rest of the picture stays whole
    Given a package "app" at "crates/app"
    And a package "engine" at "crates/engine"
    And a source file "crates/app/src/lib.rs" containing:
      """
      mod wiring;
      """
    And a source file "crates/app/src/wiring.rs" containing:
      """
      use engine::run;
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      pub fn run() {}
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And the file tree is hidden
    Then a connection goes from "app" to "engine"
    When the boundary "app" is expanded
    Then the boundaries include "wiring"
    And the boundaries do not include "physics"
    And a connection goes from "wiring" to "engine"

  Scenario: Expanding a boundary opens one layer at a time
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And the file tree is hidden
    And the boundary "engine" is expanded
    Then the boundaries include "physics"
    And the boundaries do not include "step"
    When the boundary "physics" is expanded
    Then the boundaries include "step"

  Scenario: Opening a boundary fully reveals everything beneath it
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And the file tree is hidden
    And the boundary "engine" is opened fully
    Then the boundaries include "physics"
    And the boundaries include "step"

  Scenario: A reopened boundary remembers the openings made inside it
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And the file tree is hidden
    And the boundary "engine" is expanded
    And the boundary "physics" is expanded
    Then the boundaries include "step"
    When the boundary "engine" is collapsed
    Then the boundaries do not include "physics"
    When the boundary "engine" is expanded
    Then the boundaries include "step"

  Scenario: Collapsing a boundary keeps the dependency on it as a whole
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And only the structure is shown
    And the file tree is hidden
    Then a connection goes from "app" to "engine"
    When the boundary "engine" is collapsed
    Then a connection goes from "app" to "engine"

  Scenario: An open boundary exposes its individual declarations
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    And a source file "crates/app/src/lib.rs" containing:
      """
      use engine::run;
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      pub fn run() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And the file tree is hidden
    Then the boundaries include "run"
    And a connection goes from "app" to "run"

  Scenario: A declaration that reaches no further than its module stays inside it
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      pub fn run() {}
      fn helper() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And the file tree is hidden
    Then the boundaries include "run"
    And the boundaries do not include "helper"

  Scenario: Hidden functions hand their connections to the boundary that declares them
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    And a source file "crates/app/src/lib.rs" containing:
      """
      use engine::run;
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      pub fn run() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And the file tree is hidden
    Then a connection goes from "app" to "run"
    When "functions" are hidden from the picture
    Then the boundaries do not include "run"
    And a connection goes from "app" to "engine"
    When "functions" are shown in the picture
    Then a connection goes from "app" to "run"

  Scenario: Hidden modules pool their declarations in the package
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub struct Body {}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And the file tree is hidden
    Then the boundary "physics" contains "Body"
    When "modules" are hidden from the picture
    Then the boundaries do not include "physics"
    And the boundary "engine" contains "Body"

  Scenario: Hidden modules leave the functions their connections
    Given a package "app" at "crates/app"
    And a source file "crates/app/src/lib.rs" containing:
      """
      pub mod caller;
      pub mod callee;
      """
    And a source file "crates/app/src/caller.rs" containing:
      """
      use crate::callee::serve;

      pub fn drive() {
          serve();
      }
      """
    And a source file "crates/app/src/callee.rs" containing:
      """
      pub fn serve() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And the file tree is hidden
    And "modules" are hidden from the picture
    Then the boundary "app" contains "drive"
    And a connection goes from "drive" to "serve"

  Scenario: Opening a type reveals its methods
    Given a package "app" at "crates/app"
    And a source file "crates/app/src/lib.rs" containing:
      """
      pub mod config;
      pub mod store;
      """
    And a source file "crates/app/src/config.rs" containing:
      """
      pub struct Config;

      impl Config {
          pub fn load() {
              crate::store::read();
          }
      }
      """
    And a source file "crates/app/src/store.rs" containing:
      """
      pub fn read() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And the file tree is hidden
    Then the boundary "Config" contains "load"
    And a connection goes from "load" to "read"
    When "functions" are hidden from the picture
    Then the boundaries do not include "load"
    And a connection goes from "Config" to "store"

  Scenario: Hidden directories pool their modules in the package
    Given a source file "app/package.json" containing:
      """
      {"name":"app"}
      """
    And a source file "app/src/widgets/panel.ts" containing:
      """
      export class Panel {}
      """
    And a source file "app/src/widgets/button.ts" containing:
      """
      export class Button {}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the boundary "src/widgets" contains "panel"
    And the boundary "src/widgets" contains "button"
    When "directories" are hidden from the picture
    Then the boundaries do not include "src/widgets"
    And the boundary "app" contains "panel"
    And the boundary "app" contains "button"
