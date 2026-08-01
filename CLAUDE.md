# Cutaway — Codebase Guide

Cutaway is a desktop tool that visualises the architecture of software
projects: current state through lenses, deltas between versions, and
redlines for planned changes. See README.md for the product description.

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
  inspection/          Application core: builds the model from sources.
    src/ports/         Ports of the core, one file per port.
  comparison/          Domain: deltas between two architecture versions.
  redlining/           Domain: proposed changes on top of an architecture.
  adapters/
    git/               Driven adapter: git repository as a SourceTree (gix).
    treesitter/        Driven adapter: SyntaxAnalyzer per language (tree-sitter).
    gui/               Driving adapter: eframe/egui desktop shell.
  cutaway/             Composition root: wires adapters to the core, starts the GUI.
  e2e/                 Cucumber suite + the ApplicationDriver port it drives.
```

### Rules

- Dependencies point inward only: adapters depend on the core and the domain,
  never the reverse. The domain crates depend on nothing but `thiserror`.
- Every need the core has from the outside world is an explicit port: a trait
  in `src/ports/<port>.rs` together with the value and error types that cross
  the boundary. One file per port.
- Only the composition root (`crates/cutaway`) and the e2e driver know which
  concrete adapters exist.
- The GUI receives capabilities (e.g. `ProjectLoader`) from the composition
  root; it never constructs adapters itself.
- Nothing outside `crates/adapters/treesitter` knows which languages exist.
  A new language = a new analyzer in that crate + wiring in the composition
  root.

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
