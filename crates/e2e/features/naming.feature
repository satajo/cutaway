Feature: What a box is called

  A boundary carries up to two names - what a language calls it and what the
  tree calls it - and the vocabulary of the picture decides which of them a
  box speaks. Two boxes in one frame speaking one name name neither of them,
  so each falls back to the place it stands at instead: the place carries
  the segments of every directory that dissolved into it, and no two places
  read alike.

  Scenario: Two modules the language names alike read as the places they stand at
    Given a source file "package.json" containing:
      """
      {"name": "app"}
      """
    And a source file "lib/a/index.ts" containing:
      """
      export const first = 1;
      """
    And a source file "lib/b/index.ts" containing:
      """
      export const second = 2;
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the boundary "lib" contains "a/index.ts"
    And the boundary "lib" contains "b/index.ts"

  Scenario: A module no namesake stands beside keeps the name its language gives it
    Given a source file "package.json" containing:
      """
      {"name": "app"}
      """
    And a source file "lib/index.ts" containing:
      """
      export const only = 1;
      """
    And a source file "lib/helpers.ts" containing:
      """
      export const help = 2;
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the boundary "lib" contains "index"
    And the boundary "lib" contains "helpers"
