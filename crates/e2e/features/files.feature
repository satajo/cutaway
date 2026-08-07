Feature: Files no language reads

  Inspection is total: every file of the sources stands in the picture. A
  file no language reads meaning from appears as itself, a plain file, where
  it lies in the directory tree: grouped with its neighbours into the
  directories that organize them, and inside the package whose directory
  holds it. Between two versions such a file speaks through its contents:
  changed contents read as modified, and untouched contents read as nothing
  at all.

  Scenario: A file no language reads still stands in the picture
    Given a source file "README.md" containing:
      """
      All about the project.
      """
    When the project is inspected
    And the boundaries are viewed
    Then the boundaries include "README.md"

  Scenario: Files with nothing else beside them group into their directory
    Given a source file "docs/guide.md" containing:
      """
      How to start.
      """
    And a source file "docs/setup.md" containing:
      """
      How to install.
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the boundaries include "docs"
    And the boundary "docs" contains "guide.md"
    And the boundary "docs" contains "setup.md"

  Scenario: A file lying inside a package's directory stands inside the package
    Given a package "app" at "crates/app"
    And a source file "crates/app/notes.txt" containing:
      """
      Remember the demo.
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the boundary "app" contains "notes.txt"

  Scenario: A file appearing between two versions reads as added
    Given in version "before" a source file "notes.txt" containing:
      """
      keep
      """
    And in version "after" a source file "notes.txt" containing:
      """
      keep
      """
    And in version "after" a source file "extra.txt" containing:
      """
      new
      """
    When the change from version "before" to version "after" is viewed
    Then the boundary "extra.txt" reads as "added"
    And the boundary "notes.txt" reads as unchanged

  Scenario: A file disappearing between two versions stands in the picture and reads as removed
    Given in version "before" a source file "notes.txt" containing:
      """
      keep
      """
    And in version "before" a source file "extra.txt" containing:
      """
      old
      """
    And in version "after" a source file "notes.txt" containing:
      """
      keep
      """
    When the change from version "before" to version "after" is viewed
    Then the boundaries include "extra.txt"
    And the boundary "extra.txt" reads as "removed"
    And the boundary "notes.txt" reads as unchanged

  Scenario: A changed file reads as modified between two versions
    Given in version "before" a source file "config.yaml" containing:
      """
      mode: light
      """
    And in version "after" a source file "config.yaml" containing:
      """
      mode: dark
      """
    When the change from version "before" to version "after" is viewed
    Then the boundary "config.yaml" reads as "modified"

  Scenario: An untouched file reads as unchanged between two versions
    Given in version "before" a source file "config.yaml" containing:
      """
      mode: light
      """
    And in version "after" a source file "config.yaml" containing:
      """
      mode: light
      """
    When the change from version "before" to version "after" is viewed
    Then the boundary "config.yaml" reads as unchanged
