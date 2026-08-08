Feature: The file tree beneath the picture

  The directories and files of the sources are the skeleton every picture
  hangs on, whatever language wrote them. What a language reads fuses onto
  that skeleton where the two coincide: the module written in one file is
  that file, and the package occupying one directory is that directory. One
  box, two names.

  The vocabulary decides which of the two names the box speaks. With the
  whole vocabulary the languages' reading leads. Drop the tree and only the
  languages' reading stands, exactly as it stood before the tree became the
  skeleton. Drop the languages' reading and what stands is the listing
  itself, a directory holding one thing dissolved into the name of what it
  held, and a module spanning a file and a directory standing as the one
  entry it is.

  Background:
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;

      pub struct Session;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub mod forces;

      pub fn step() {}
      """
    And a source file "crates/engine/src/physics/forces.rs" containing:
      """
      pub fn apply() {}
      """
    And a source file "README.md" containing:
      """
      About the engine.
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened

  Scenario: The whole vocabulary draws the tree with what the languages read fused onto it
    Then the boundaries are "Cargo.toml, README.md, Session, apply, engine, lib.rs, physics, physics::forces, src, step"
    And the boundary "engine" contains "Cargo.toml"
    And the boundary "engine" contains "src"
    And the boundary "src" contains "lib.rs"
    And the boundary "src" contains "physics"
    And the boundary "lib.rs" contains "Session"
    And the boundary "physics" contains "step"
    And the boundary "physics" contains "physics::forces"
    And the boundary "physics::forces" contains "apply"

  Scenario: Hiding the tree leaves what the languages read
    When the file tree is hidden
    Then the boundaries are "Session, apply, engine, physics, physics::forces, step"
    And the boundary "engine" contains "Session"
    And the boundary "engine" contains "physics"
    And the boundary "physics" contains "step"
    And the boundary "physics" contains "physics::forces"
    And the boundary "physics::forces" contains "apply"

  Scenario: Hiding what the languages read leaves the listing
    When only the file tree is shown
    Then the boundaries are "Cargo.toml, README.md, crates/engine, forces.rs, lib.rs, physics, src"
    And the boundary "crates/engine" contains "Cargo.toml"
    And the boundary "crates/engine" contains "src"
    And the boundary "src" contains "lib.rs"
    And the boundary "src" contains "physics"
    And the boundary "physics" contains "forces.rs"
