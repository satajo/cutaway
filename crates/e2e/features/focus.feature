Feature: Focusing the picture on one boundary

  Every detail below packages puts the whole project's insides in front of
  the reader at once. Focusing scopes the picture to one boundary: it is the
  whole picture, the boundaries it depends on and those that depend on it
  stand at the border as single closed boxes, and everything else leaves.
  What passes between two of those partners is about neither of them and
  leaves with the rest. Looking inside a partner means focusing on it.

  Scenario: A focused boundary stands with its dependency partners alone
    Given a package "app" at "crates/app"
    And a package "engine" at "crates/engine"
    And a package "store" at "crates/store"
    And a package "other" at "crates/other"
    And a source file "crates/app/src/lib.rs" containing:
      """
      use engine::run;
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      use store::put;
      pub fn run() {}
      """
    And a source file "crates/other/src/lib.rs" containing:
      """
      use store::put;
      """
    And a source file "crates/store/src/lib.rs" containing:
      """
      pub fn put() {}
      """
    When the project is inspected
    And the boundaries are viewed at "packages" level
    Then the boundaries are "app, engine, other, store"
    When the picture is focused on "engine"
    Then the boundaries are "app, engine, store"
    And a connection goes from "app" to "engine"
    And a connection goes from "engine" to "store"

  Scenario: Dependencies between two partners leave a focused picture
    Given a package "app" at "crates/app"
    And a package "engine" at "crates/engine"
    And a package "store" at "crates/store"
    And a source file "crates/app/src/lib.rs" containing:
      """
      use engine::run;
      use store::put;
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      use store::put;
      pub fn run() {}
      """
    And a source file "crates/store/src/lib.rs" containing:
      """
      pub fn put() {}
      """
    When the project is inspected
    And the boundaries are viewed at "packages" level
    Then a connection goes from "engine" to "store"
    When the picture is focused on "app"
    Then the boundaries are "app, engine, store"
    And no connection goes from "engine" to "store"

  Scenario: A partner stands whole however it was opened
    Given a package "app" at "crates/app"
    And a package "engine" at "crates/engine"
    And a source file "crates/app/src/lib.rs" containing:
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
    And the boundary "engine" is expanded
    Then the boundaries include "physics"
    When the picture is focused on "app"
    Then the boundaries include "engine"
    And the boundaries do not include "physics"
    And the boundary "engine" cannot be expanded

  Scenario: Focusing on a partner swaps the picture around it
    Given a package "app" at "crates/app"
    And a package "engine" at "crates/engine"
    And a package "store" at "crates/store"
    And a source file "crates/app/src/lib.rs" containing:
      """
      use engine::run;
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      use store::put;
      pub fn run() {}
      """
    And a source file "crates/store/src/lib.rs" containing:
      """
      pub fn put() {}
      """
    When the project is inspected
    And the boundaries are viewed at "packages" level
    And the picture is focused on "app"
    Then the boundaries are "app, engine"
    When the picture is focused on "engine"
    Then the boundaries are "app, engine, store"

  Scenario: Showing everything again restores the whole picture
    Given a package "app" at "crates/app"
    And a package "engine" at "crates/engine"
    And a package "other" at "crates/other"
    And a source file "crates/app/src/lib.rs" containing:
      """
      use engine::run;
      """
    And a source file "crates/engine/src/lib.rs" containing:
      """
      pub fn run() {}
      """
    And a source file "crates/other/src/lib.rs" containing:
      """
      """
    When the project is inspected
    And the boundaries are viewed at "packages" level
    And the picture is focused on "app"
    Then the boundaries do not include "other"
    When the whole picture is shown again
    Then the boundaries are "app, engine, other"

  Scenario: A focused boundary opens while the picture stays about it
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
    And the picture is focused on "app"
    Then the boundaries are "app, engine"
    When the boundaries are viewed at "modules" level
    Then the boundaries include "wiring"
    And the boundaries do not include "physics"
    And a connection goes from "wiring" to "engine"
