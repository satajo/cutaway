Feature: Sources a language cannot read whole

  A language reads what it can and says where it could not. One construct it
  cannot make sense of thins the reading of that file; it never costs the
  file its place, the declarations standing around the break, or the rest of
  the project its picture.

  Every such place is declared: the file, and what stood in the way. The
  declaration is what keeps the picture honest, so a reader is never shown a
  partial reading as a whole one - and in a comparison it says which side was
  read thin, because declarations missing from one version otherwise read as
  a change the project never made.

  Scenario: A source file the language can make nothing of still stands in the picture
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      ][ not a language at all %%
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the boundaries include "physics"
    And the boundary "src" contains "physics"
    And the reading declares a gap in "crates/engine/src/physics.rs"

  Scenario: The declarations that parsed stand in the picture
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub struct Force;

      pub fn apply( {
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the boundaries include "Force"

  Scenario: The part that could not be read is declared as a gap
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And a source file "crates/engine/src/physics.rs" containing:
      """
      pub struct Force;

      pub fn apply( {
      """
    When the project is inspected
    Then the reading declares a gap in "crates/engine/src/physics.rs"

  Scenario: A package whose manifest cannot be read declares its gap and leaves the other packages standing
    Given a package "engine" at "crates/engine"
    And a source file "crates/store/Cargo.toml" containing:
      """
      [package
      name = "store"
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the reading declares a gap in "crates/store/Cargo.toml"
    And the boundaries include "engine"
    And the boundaries do not include "store"
    And the boundary "crates" contains "store/Cargo.toml"

  Scenario: A project the languages read whole declares no gap
    Given a package "engine" at "crates/engine"
    And a source file "crates/engine/src/lib.rs" containing:
      """
      pub struct Force;
      """
    When the project is inspected
    Then the reading declares no gaps

  Scenario: A version read thin declares its gap on that side of the comparison
    Given in version "before" a package "engine" at "crates/engine"
    And in version "before" a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And in version "before" a source file "crates/engine/src/physics.rs" containing:
      """
      pub struct Force;

      pub fn apply( {
      """
    And in version "after" a package "engine" at "crates/engine"
    And in version "after" a source file "crates/engine/src/lib.rs" containing:
      """
      mod physics;
      """
    And in version "after" a source file "crates/engine/src/physics.rs" containing:
      """
      pub struct Force;

      pub fn apply() {}
      """
    When the change from version "before" to version "after" is viewed
    Then the reading of version "before" declares a gap in "crates/engine/src/physics.rs"
    And the reading of version "after" declares no gaps
