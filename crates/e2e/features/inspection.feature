Feature: Inspecting a project's architecture

  The architecture of a project is presented as a graph of elements.
  Source files appear as modules; declarations found inside them appear
  as elements of their own.

  Scenario: Declared functions appear in the architecture
    Given a project with a Rust file "src/lib.rs" containing a function "connect"
    When the project is inspected
    Then the architecture contains an element named "src/lib.rs"
    And the architecture contains an element named "connect"
