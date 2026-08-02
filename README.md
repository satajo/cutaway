# Cutaway

Cutaway draws "cutaway drawings" of software projects: interactive views of a
project's architecture through different lenses. Point it at a git repository
and it builds an architecture model of the committed sources, which you can
then examine from three angles:

- **Lenses** — views of the current architecture at a chosen abstraction level.
- **Deltas** — what changed in the architecture between two versions.
- **Plans** — proposed changes drawn on top of the current architecture, to
  plan work before any code moves.

AI agents produce large codebases faster than people can read them. Cutaway
raises the abstraction level at which you work with such a codebase: instead
of reading files, you inspect, compare, and plan its architecture.

Cutaway is fully local and self-contained. It reads the repository on your
disk and talks to nothing else.

## Status

Early but usable. The application opens a git repository, inspects the Rust
sources of its `HEAD` commit (Cargo manifests, module structure, imports),
and draws the boundary lens at an adjustable level of detail: packages, the
modules within them, or the individual items within the modules, as nested
boxes with the dependencies that cross boundary lines as arrows. Arrows
attach only to boxes without visible children; a box that contains others
shows its own code as a `self` box, and a dependency on a whole box waits at
a coarser detail. Existing connections are monochrome; severing one turns it
red, drawing a new one turns it green, and any connection or boundary can
carry a note. All markup saves immediately to
`cutaway.json` in the root of the inspected repository — a versioned JSON
work order ready to hand to an AI agent. The delta view exists as a domain
model only.

## Installation

With Nix (flakes enabled):

```sh
nix profile install github:satajo/cutaway
```

Or run without installing:

```sh
nix run github:satajo/cutaway
```

## Development

The flake provides the complete toolchain:

```sh
nix develop
make check
```

`make check` verifies the whole project: formatting, lints, unit tests, and
the Cucumber e2e suite. See the [Makefile](Makefile) for the individual
targets and [CLAUDE.md](CLAUDE.md) for the codebase structure and practices.
