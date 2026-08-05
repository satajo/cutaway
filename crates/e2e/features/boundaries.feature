Feature: The boundary lens

  The architecture appears as boundaries at an adjustable level of detail:
  packages, the modules within them, or the individual items within the
  modules. Connections attach to the nearest visible boundary, single boxes
  and boundaries with visible children alike. A connection that ends at a
  boundary's border speaks about the boundary's own code or the boundary as
  a whole; the connections into its parts end at the parts. What passes
  between a boundary and its own contents stays inside it. A package's root
  source file is the package's own code, not a boundary of its own: its
  declarations are the package's items and its child modules sit directly
  inside the package.

  Scenario: Package dependencies declared in manifests appear as connections
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    When the project is inspected
    And the boundaries are viewed at "packages" level
    Then the boundaries are "app, engine"
    And a connection goes from "app" to "engine"

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
    And the boundaries are viewed at "packages" level
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
    And the boundaries are viewed at "modules" level
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
    And the boundaries are viewed at "modules" level
    Then the boundaries do not include "crate"
    And the boundary "engine" contains "physics"
    And the boundary "engine" contains "render"

  Scenario: An integration test file keeps a box of its own beside the modules
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
    And the boundaries are viewed at "modules" level
    Then the boundaries do not include "crate"
    And the boundary "engine" contains "physics"
    And the boundary "engine" contains "tests/behaviour.rs"

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
    And the boundaries are viewed at "modules" level
    Then the boundaries include "tests"
    And no connection goes from "tests" to "engine"

  Scenario: A dependency on a whole boundary shows at every detail
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    And a source file "crates/app/src/lib.rs" containing:
      """
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      """
    When the project is inspected
    And the boundaries are viewed at "modules" level
    Then a connection goes from "app" to "engine"
    When the boundaries are viewed at "packages" level
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
    And the boundaries are viewed at "packages" level
    Then a connection goes from "app" to "engine"
    When the boundary "app" is expanded
    Then the boundaries include "wiring"
    And the boundaries do not include "physics"
    And a connection goes from "wiring" to "engine"

  Scenario: Collapsing a boundary keeps the dependency on it as a whole
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      """
    When the project is inspected
    And the boundaries are viewed at "modules" level
    Then a connection goes from "app" to "engine"
    When the boundary "engine" is collapsed
    Then a connection goes from "app" to "engine"

  Scenario: Item detail exposes individual declarations
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
    And the boundaries are viewed at "items" level
    Then the boundaries include "run"
    And a connection goes from "app" to "run"
