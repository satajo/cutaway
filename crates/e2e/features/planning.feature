Feature: Planning on the boundary view

  Connections can be severed, new ones drawn, and any of them annotated.
  Every markup lands in the project's plan, which persists immediately: the
  plan is the work order later handed to an agent.

  Background:
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    When the project is inspected
    And the boundaries are viewed

  Scenario: Severing a connection marks it for removal
    When the connection from "app" to "engine" is severed
    Then the plan marks the connection from "app" to "engine" for removal
    And the saved plan equals the working plan

  Scenario: Drawing a connection proposes a new dependency
    When a connection is drawn from "engine" to "app"
    Then the plan proposes a connection from "engine" to "app"
    And the saved plan equals the working plan

  Scenario: Annotating a connection records the rationale
    When the connection from "app" to "engine" is annotated with "app must stop reaching into engine"
    Then the connection from "app" to "engine" carries the note "app must stop reaching into engine"
    And the saved plan equals the working plan

  Scenario: A planned removal stays planned when the boundary it enters opens
    Given a package "viewer" at "crates/viewer"
    And a package "model" at "crates/model"
    And a source file "crates/viewer/src/lib.rs" containing:
      """
      use model::run;
      """
    And a source file "crates/model/src/lib.rs" containing:
      """
      mod runner;
      pub use runner::run;
      """
    And a source file "crates/model/src/runner.rs" containing:
      """
      pub fn run() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And the connection from "viewer" to "model" is severed
    And the boundary "model" is expanded
    Then a connection goes from "viewer" to "runner"
    And the plan marks the connection from "viewer" to "runner" for removal
    And the saved plan equals the working plan

  Scenario: A drawn dependency stays visible when the picture closes around it
    Given a package "viewer" at "crates/viewer"
    And a package "model" at "crates/model"
    And a source file "crates/viewer/src/lib.rs" containing:
      """
      mod wiring;
      """
    And a source file "crates/viewer/src/wiring.rs" containing:
      """
      """
    And a source file "crates/model/src/lib.rs" containing:
      """
      mod physics;
      """
    And a source file "crates/model/src/physics.rs" containing:
      """
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    And only the structure is shown
    And a connection is drawn from "physics" to "wiring"
    And the boundaries are viewed
    Then a connection goes from "model" to "viewer"
    And the plan proposes a connection from "model" to "viewer"

  Scenario: A drawn dependency stays visible when the boundary it names opens
    Given a package "viewer" at "crates/viewer"
    And a package "model" at "crates/model"
    And a source file "crates/viewer/src/lib.rs" containing:
      """
      mod wiring;
      """
    And a source file "crates/viewer/src/wiring.rs" containing:
      """
      """
    And a source file "crates/model/src/lib.rs" containing:
      """
      """
    When the project is inspected
    And the boundaries are viewed
    And a connection is drawn from "model" to "viewer"
    And the boundary "viewer" is expanded
    Then a connection goes from "model" to "viewer"
    And the plan proposes a connection from "model" to "viewer"
