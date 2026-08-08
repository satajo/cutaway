Feature: Comparing two versions

  The comparison shows what changes between two versions of the project. The
  picture lays out everything either version holds and reads it through the
  ordinary boundary lens: what arrives reads as added, what goes reads as
  removed, and a boundary that stands in both versions while something
  changes out of sight inside it reads as modified. A boundary the change
  never touches reads as nothing at all.

  Every file speaks through its contents, so an edit that leaves every
  declaration standing still reads as modified at the boundary that file
  became - a module, a manifest, or a file no language reads.

  Every reading sits at the nearest boundary the picture draws, so opening a
  modified boundary moves the reading onto the change itself. A connection
  reads from the dependencies behind it: it arrives when they all arrive, it
  goes when they all go, and it changes while it stands and the mix behind it
  shifts.

  The plan plays no part. A comparison reads what the project did; the plan
  speaks about what the project is to do, and neither belongs in the other's
  picture.

  Scenario: Two identical versions read as no change at all
    Given in version "before" a package "app" at "crates/app" depending on "engine"
    And in version "before" a package "engine" at "crates/engine"
    And in version "after" a package "app" at "crates/app" depending on "engine"
    And in version "after" a package "engine" at "crates/engine"
    When the change from version "before" to version "after" is viewed
    And the file tree is hidden
    Then the boundary "app" reads as unchanged
    And the boundary "engine" reads as unchanged
    And the connection from "app" to "engine" reads as "unchanged"

  Scenario: A package only the newer version holds reads as added
    Given in version "before" a package "app" at "crates/app"
    And in version "before" a package "store" at "crates/store"
    And in version "after" a package "app" at "crates/app"
    And in version "after" a package "store" at "crates/store"
    And in version "after" a package "engine" at "crates/engine"
    When the change from version "before" to version "after" is viewed
    And the file tree is hidden
    Then the boundary "engine" reads as "added"
    And the boundary "app" reads as unchanged

  Scenario: A package only the older version holds stands in the picture and reads as removed
    Given in version "before" a package "app" at "crates/app"
    And in version "before" a package "store" at "crates/store"
    And in version "before" a package "engine" at "crates/engine"
    And in version "after" a package "app" at "crates/app"
    And in version "after" a package "store" at "crates/store"
    When the change from version "before" to version "after" is viewed
    And the file tree is hidden
    Then the boundaries include "engine"
    And the boundary "engine" reads as "removed"

  Scenario: A closed package hiding a new module inside it reads as modified
    Given in version "before" a package "engine" at "crates/engine"
    And in version "before" a source file "crates/engine/src/lib.rs" containing:
      """
      """
    And in version "after" a package "engine" at "crates/engine"
    And in version "after" a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And in version "after" a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    When the change from version "before" to version "after" is viewed
    And the file tree is hidden
    Then the boundaries do not include "physics"
    And the boundary "engine" reads as "modified"

  Scenario: Opening the package moves the reading onto the module itself
    Given in version "before" a package "engine" at "crates/engine"
    And in version "before" a source file "crates/engine/src/lib.rs" containing:
      """
      """
    And in version "after" a package "engine" at "crates/engine"
    And in version "after" a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And in version "after" a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    When the change from version "before" to version "after" is viewed
    And the file tree is hidden
    And the boundary "engine" is expanded
    Then the boundaries include "physics"
    And the boundary "physics" reads as "added"

  Scenario: A boundary whose place in the tree changed still draws, where it is arriving
    Given in version "before" a source file "crates/engine/Cargo.toml" containing:
      """
      [package]
      name = "engine"
      """
    And in version "before" a source file "crates/engine/src/lib.rs" containing:
      """
      pub fn run() {}
      """
    And in version "after" a source file "crates/engine/Cargo.toml" containing:
      """
      [package]
      name = "engine"
      """
    And in version "after" a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      pub fn run() {}
      """
    And in version "after" a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {}
      """
    When the change from version "before" to version "after" is viewed
    And every boundary is opened
    Then the boundaries include "src"
    And the boundary "src" contains "lib.rs"
    And the boundary "src" contains "physics"
    And the boundary "physics" reads as "added"

  Scenario: A module whose code changes while its declarations stand reads as modified
    Given in version "before" a package "engine" at "crates/engine"
    And in version "before" a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And in version "before" a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {
          apply(1);
      }
      """
    And in version "after" a package "engine" at "crates/engine"
    And in version "after" a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And in version "after" a source file "crates/engine/src/physics.rs" containing:
      """
      pub fn step() {
          apply(2);
      }
      """
    When the change from version "before" to version "after" is viewed
    And every boundary is opened
    Then the boundary "physics" reads as "modified"
    And the boundary "step" reads as unchanged
    And the boundary "lib.rs" reads as unchanged

  Scenario: A manifest whose contents change reads as modified
    Given in version "before" a source file "crates/engine/Cargo.toml" containing:
      """
      [package]
      name = "engine"
      version = "0.1.0"
      """
    And in version "before" a source file "crates/engine/src/lib.rs" containing:
      """
      pub fn run() {}
      """
    And in version "after" a source file "crates/engine/Cargo.toml" containing:
      """
      [package]
      name = "engine"
      version = "0.2.0"
      """
    And in version "after" a source file "crates/engine/src/lib.rs" containing:
      """
      pub fn run() {}
      """
    When the change from version "before" to version "after" is viewed
    And every boundary is opened
    Then the boundary "Cargo.toml" reads as "modified"
    And the boundary "engine" reads as unchanged

  Scenario: A dependency the newer version adds reads as an added connection
    Given in version "before" a package "app" at "crates/app"
    And in version "before" a package "engine" at "crates/engine"
    And in version "after" a package "app" at "crates/app" depending on "engine"
    And in version "after" a package "engine" at "crates/engine"
    When the change from version "before" to version "after" is viewed
    And the file tree is hidden
    Then the connection from "app" to "engine" reads as "added"

  Scenario: The plan plays no part in the comparison
    Given a package "engine" at "crates/engine"
    And in version "before" a package "engine" at "crates/engine"
    And in version "after" a package "engine" at "crates/engine"
    When the project is inspected
    And the boundaries are viewed
    And the file tree is hidden
    And the removal of "engine" is planned
    And a package named "transport" is planned
    And the change from version "before" to version "after" is viewed
    And the file tree is hidden
    Then the boundaries include "engine"
    And the boundary "engine" reads as unchanged
    And the boundaries do not include "transport"
