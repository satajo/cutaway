Feature: Planning on the boundary view

  Connections can be severed, new ones drawn, and any of them annotated.
  Every markup lands in the project's plan, which persists immediately: the
  plan is the work order later handed to an agent.

  Background:
    Given a package "app" at "crates/app" depending on "engine"
    And a package "engine" at "crates/engine"
    When the project is inspected
    And the boundaries are viewed at "packages" level

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
