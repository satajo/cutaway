Feature: The boundary lens

  The architecture appears as boundaries at an adjustable level of detail:
  packages, the modules within them, or the individual items within the
  modules. Connections attach only to boundaries without visible children.
  A boundary that contains other boundaries shows its own content as a
  "self" boundary, and a dependency that names such a boundary as a whole
  waits at a coarser detail. Dependencies inside one boundary stay out of
  sight at that level.

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

  Scenario: Module detail shows a boundary's own code as its self boundary
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
    When the project is inspected
    And the boundaries are viewed at "modules" level
    Then a connection goes from "self" to "physics"

  Scenario: A dependency on a whole boundary waits at a coarser detail
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
    Then no connection goes from "app" to "engine"
    When the boundaries are viewed at "packages" level
    Then a connection goes from "app" to "engine"

  Scenario: Item detail exposes individual declarations
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
    When the project is inspected
    And the boundaries are viewed at "items" level
    Then a connection goes from "self" to "step"
