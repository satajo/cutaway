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
attach to the nearest visible box, framed boxes included: an arrow that ends
at a box speaks about the box's own code or the box as a whole, and what
passes between a box and its own contents stays inside it. Existing
connections are monochrome; severing one turns it
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

## License

Copyright (C) 2026 Sami Jokela

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version.

This program is distributed in the hope that it will be useful, but WITHOUT
ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License along with
this program. If not, see <https://www.gnu.org/licenses/>.

The full text is in [LICENSE](LICENSE).
