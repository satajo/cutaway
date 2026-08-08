Feature: Planning whole elements

  A boundary can be marked for removal together with everything inside it,
  and a new boundary can be planned before any source declares it. Both land
  in the project's plan, which persists immediately: the plan is the work
  order later handed to an agent.

  Background:
    Given a package "viewer" at "crates/viewer"
    And a package "model" at "crates/model"
    And a source file "crates/viewer/src/lib.rs" containing:
      """
      mod wiring;
      use model::run;
      """
    And a source file "crates/viewer/src/wiring.rs" containing:
      """
      """
    And a source file "crates/model/src/lib.rs" containing:
      """
      pub fn run() {}
      """
    When the project is inspected
    And the boundaries are viewed
    And the file tree is hidden

  Scenario: Planning the removal of a package severs what crosses its border
    When the removal of "model" is planned
    Then the plan marks "model" for removal
    And the plan marks the connection from "viewer" to "model" for removal
    And the saved plan equals the working plan

  Scenario: A planned removal reaches everything inside the boundary
    When the removal of "model" is planned
    And the boundary "model" is expanded
    Then the plan marks "run" for removal

  Scenario: Restoring a planned removal clears the marks
    When the removal of "model" is planned
    And the removal of "model" is restored
    Then the plan does not mark "model" for removal
    And the plan leaves the connection from "viewer" to "model" alone
    And the saved plan equals the working plan

  Scenario: A planned package stands in the picture before any source declares it
    When a package named "engine" is planned
    Then the boundaries include "engine"
    And the plan proposes an element "engine"
    And the saved plan equals the working plan

  Scenario: A planned module appears inside the boundary that holds it
    When the boundaries are viewed
    And the file tree is hidden
    And every boundary is opened
    And only the structure is shown
    And a "module" named "physics" is planned inside "model"
    Then the boundaries include "physics"
    And the boundary "model" contains "physics"
    And the plan proposes an element "physics"
    And the saved plan equals the working plan

  Scenario: A connection drawn to a planned element survives the cut changing
    When the boundaries are viewed
    And the file tree is hidden
    And every boundary is opened
    And only the structure is shown
    And a "module" named "physics" is planned inside "model"
    And a connection is drawn from "wiring" to "physics"
    Then the plan proposes a connection from "wiring" to "physics"
    When the boundaries are viewed
    And the file tree is hidden
    And every boundary is opened
    And only the structure is shown
    Then the boundaries include "physics"
    And a connection goes from "wiring" to "physics"
    And the plan proposes a connection from "wiring" to "physics"
    And the saved plan equals the working plan
