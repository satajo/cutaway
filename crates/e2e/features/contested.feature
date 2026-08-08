Feature: Two readings of one place

  A directory or a file fuses with what a language reads out of it only while
  that reading is the sole one. Two languages reading the same place agree
  about neither name, so the place keeps its own and both readings stand
  inside it. Nothing disappears either way: every manifest, every file, and
  both packages stay in the picture.

  Scenario: Two packages naming one directory leave the directory standing with both inside it
    Given a source file "crates/app/Cargo.toml" containing:
      """
      [package]
      name = "app-rs"
      """
    And a source file "crates/app/package.json" containing:
      """
      {"name": "app-ts"}
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the boundaries are "Cargo.toml, app-rs, app-ts, crates/app, package.json"
    And the boundary "crates/app" contains "app-rs"
    And the boundary "crates/app" contains "app-ts"
    And the boundary "crates/app" contains "Cargo.toml"
    And the boundary "crates/app" contains "package.json"

  Scenario: Two packages naming the repository root stand beside each other
    Given a source file "Cargo.toml" containing:
      """
      [package]
      name = "app-rs"
      """
    And a source file "package.json" containing:
      """
      {"name": "app-ts"}
      """
    And a source file "README.md" containing:
      """
      About the app.
      """
    When the project is inspected
    And the boundaries are viewed
    And every boundary is opened
    Then the boundaries are "Cargo.toml, README.md, app-rs, app-ts, package.json"
    And the boundary "app-rs" holds nothing
    And the boundary "app-ts" holds nothing
