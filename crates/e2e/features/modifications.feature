Feature: Planning modifications of what stays

  An element that neither goes nor arrives can still change: renamed, split
  into several, merged into another, or reworked where it stands. A
  modification is an intent for whoever implements the plan, not a change to
  the architecture, so it redraws nothing and marks the element instead.
  Every modification lands in the project's plan, which persists immediately.

  Background:
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    When the project is inspected
    And the boundaries are viewed at "packages" level

  Scenario: A rename records the name the element takes
    When the rename of "engine" to "motor" is planned
    Then the plan modifies "engine" by "rename to motor"
    And the saved plan equals the working plan

  Scenario: A rework is described by its note
    When the rework of "engine" is planned
    And the element "engine" is annotated with "the transport belongs elsewhere"
    Then the plan modifies "engine" by "rework"
    And the element "engine" carries the note "the transport belongs elsewhere"
    And the saved plan equals the working plan

  Scenario: A split names the elements the boundary becomes
    When the split of "engine" into "engine, transport" is planned
    Then the plan modifies "engine" by "split into engine, transport"
    And the saved plan equals the working plan

  Scenario: A merge names the element it folds into
    When the merge of "app" into "engine" is planned
    Then the plan modifies "app" by "merge into engine"
    And the plan leaves the connection from "app" to "engine" alone
    And the saved plan equals the working plan

  Scenario: One element states one future
    When the rename of "engine" to "motor" is planned
    And the rework of "engine" is planned
    Then the plan modifies "engine" by "rework"

  Scenario: Discarding a modification clears the mark
    When the rename of "engine" to "motor" is planned
    And the modification of "engine" is discarded
    Then the plan modifies "engine" in no way
    And the saved plan equals the working plan

  Scenario: An element that exists only in the plan is changed, not modified
    When a package named "gearbox" is planned
    And the rename of "gearbox" to "cogs" is attempted
    Then the plan modifies "gearbox" in no way
