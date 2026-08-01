Feature: The boundary lens

  The architecture appears as boundaries - packages, and the modules within
  them - with connections for the dependencies that cross boundary lines.
  Dependencies inside one boundary stay out of sight at that level.

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

  Scenario: Module boundaries expose intra-package structure
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
    Then a connection goes from "crate" to "physics"
