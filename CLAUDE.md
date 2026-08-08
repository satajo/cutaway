# Cutaway — Codebase Guide

Cutaway is a desktop tool that visualises the architecture of software
projects: current state through lenses, deltas between versions, and
plans for future changes. See README.md for the product description.

## Commands

All development runs inside the nix dev shell, through Make:

```sh
nix develop            # enter the dev shell (or use direnv: `direnv allow`)
make check             # verify the entire project; must pass before commit
```

| Target           | Purpose                                          |
| ---------------- | ------------------------------------------------ |
| `make check`     | fmt-check + lint + all tests. The gate for every change. |
| `make fmt`       | Format the workspace.                            |
| `make lint`      | Clippy with warnings denied (pedantic enabled).  |
| `make test`      | All workspace tests, e2e included.               |
| `make e2e`       | Only the Cucumber e2e suite.                     |
| `make run`       | Start the application.                           |

For ad-hoc tools not in the shell: `nix shell nixpkgs#<pkg> --command <cmd>`.

## Architecture

Strict hexagonal architecture, one crate per concern, named after the domain
concept it serves (screaming architecture):

```
crates/
  architecture/        Domain: the architecture model (elements, relations, graph).
                       The source tree is the containment skeleton: directories
                       hold what lies in them, files hold the declarations
                       written in them. One node carries up to two readings of
                       the same boundary, each under its own name - what a
                       language read (project, package, module, type, function)
                       and what the tree shows (directory, file) - because the
                       module `element` and the file `element.rs` are one
                       boundary a reader addresses, not two. Every file-backed
                       node carries a fingerprint - its contents condensed to a
                       digest - so a comparison can read a content change in any
                       file at all.
  inspection/          Application core: builds the model from sources.
                       Inspection is total: every file and directory of the
                       tree stands in the graph. An analyzer never states where
                       an element sits; it states what it read and the extent it
                       read it over - a whole file, a whole directory, a file
                       together with its expansion directory, the repository
                       root, or a span inside a file. The core builds the tree
                       and fuses an element onto the piece it interprets whole
                       and alone; two interpretations of one piece are
                       contested, so neither fuses and both stand inside it. A
                       plain directory earns a boundary only where it groups two
                       things or more - a single-child chain dissolves and hands
                       its segments to the substrate name below it - while a
                       directory an extent names always survives.
    src/ports/         SourceTree, SourceAnalyzer, ProjectHistory. One file
                       per port.
  lenses/              Domain: boundary views - rollups of the graph along a
                       Cut, with provenance per rolled-up edge. A Cut holds two
                       independent decisions: the frontier (the set of open
                       boundaries; opening one reveals exactly one layer) and
                       the vocabulary (the element kinds the picture renders; a
                       node draws while the vocabulary holds any of its readings
                       and speaks as that one, and a node no rendered kind
                       reaches is transparent, so its contents hoist to the
                       nearest rendered ancestor). Edges attach to the
                       nearest rendered boundary, leaves and frames alike: an
                       edge ending at a frame speaks about the frame's own
                       code or the frame as a whole, and what passes between a
                       boundary and its own contents stays inside it.
  comparison/          Domain: two architecture versions held together - the
                       delta (what appeared, what disappeared, what changed:
                       a surviving id whose readings or fingerprint differ),
                       the union graph a comparison picture lays out, and
                       readings that attribute every change to the nearest
                       rendered boundary: what arrives reads as added, what
                       goes as removed, and a surviving boundary hiding churn
                       or a content change as modified. The union draws a
                       survivor where it arrives, because a picture nests as a
                       tree; a moved element's readings still climb from both
                       places it stood.
  planning/            Planning core: plans, change sets, notes, annotations.
    src/ports/         PlanStore.
  adapters/            Named role-first (<port concept>-<technology>), so the
                       adapters of one port sit together in any sorted listing.
    analyzer-go/       Driven adapter: Go ecosystem as a SourceAnalyzer
                       (go.mod manifests, sources via tree-sitter).
    analyzer-rust/     Driven adapter: Rust ecosystem as a SourceAnalyzer
                       (Cargo manifests via toml, sources via tree-sitter).
    analyzer-typescript/ Driven adapter: TypeScript and JavaScript ecosystem as a
                       SourceAnalyzer (package.json via serde_json, sources via
                       tree-sitter).
    plan-file/         Driven adapter: PlanStore as cutaway.json in the root of
                       the planned repository. The format is the agent contract.
    source-git/        Driven adapter: git repository as a SourceTree and as a
                       ProjectHistory (gix); a version is a commit.
    gui/               Driving adapter: eframe/egui shell with the boundary
                       canvas. Two modes on a left rail: Explore (read and
                       annotate one architecture) and Compare (two versions
                       against each other, the plan staying out).
  cutaway/             Composition root: wires adapters to the cores, starts the GUI.
  e2e/                 Cucumber suite + the ApplicationDriver port it drives.
```

### Rules

- Dependencies point inward only: adapters depend on the cores and the
  domain, never the reverse. The domain crates depend on nothing but
  `thiserror`.
- Every need a core has from the outside world is an explicit port: a trait
  in `src/ports/<port>.rs` together with the value and error types that cross
  the boundary. One file per port.
- Only the composition root (`crates/cutaway`) and the e2e driver know which
  concrete adapters exist.
- The GUI receives capabilities (`ProjectOpener`, the opened project's
  `PlanStore`) from the composition root; it never constructs adapters
  itself.
- Only the analyzer crates under `crates/adapters/` know which languages
  exist, one language each. A new language = a new analyzer crate under
  `crates/adapters/` + wiring in the composition root.
- Element ids are deterministic and derive only from source paths, kinds,
  and names (`project:<name>`, `package:<name>`, `<path>`,
  `<path>#<kind>:<name>`), so graphs of different versions align.

### Adding a port

1. Declare the trait and its boundary types in
   `crates/<core>/src/ports/<port>.rs`.
2. Implement it in an adapter crate under `crates/adapters/`.
3. Wire the adapter in `crates/cutaway/src/main.rs`.
4. Add an in-memory fake to `crates/e2e/src/fakes.rs` if scenarios need it.

## Practices

- Parse, don't validate: constructors enforce invariants (`ElementId`,
  `SourcePath`) and return typed errors. New concepts get their own types.
- Defaults are strict. Lenient behavior (skipping unparseable files, lossy
  decoding) requires an explicit, user-visible opt-in; none exists today.
- Tests are named after the behavior they pin down
  (`a_relation_requires_both_of_its_endpoints_to_exist`), not after the code
  they call.
- E2e scenarios express domain behavior and talk only to `ApplicationDriver`;
  they must survive a GUI rewrite unchanged.
- `make check` must pass before every commit.
