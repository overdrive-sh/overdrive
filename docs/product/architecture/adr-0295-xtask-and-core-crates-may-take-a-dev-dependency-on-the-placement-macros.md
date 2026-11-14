# ADR-0295 — `xtask` and `core` crates may take a dev-dependency on the placement macros

## Status

**Accepted — approved by the user, 2026-11-14 (U-3).**
DDD-12 in the feature delta.

## Context

Declaring a placement on a test needs the macro crate in the test's crate,
including `core`-class crates and `xtask`. `.claude/rules/development.md`
§ "xtask is build / test / dev orchestration" forbids an `overdrive-*` crate in
xtask's `[dependencies]` because the build-time compile chain would drag a
runtime crate's `build.rs` in front of the subcommand that builds its artifact.
Probe V-9 found that no automated gate reads dev-dependency edges and that the
macro crate has no `build.rs` and creates no cycle.

## Decision

Placement declarations may take a dev-dependency on `overdrive-testing-macros`
from `xtask` and from `core`-class crates. The rule's enforcement command becomes
`cargo tree -p xtask -e normal,build | rg overdrive-` (must return empty): only
normal and build edges enter the compile chain that must succeed before an xtask
subcommand runs. `xtask` also takes `overdrive-testing` as a dev-dependency for
its one guest-placed test (it pulls `overdrive-core`, `overdrive-fwd` and
`overdrive-host`, none of which has a `build.rs`). The rule's sentence that
`[dependencies]` must not contain any `overdrive-*` crate is unchanged.

## Alternatives

- **Leave xtask's one guest-placed test on the placement table**: delays the
  table's retirement; rejected.
- **Keep the literal `cargo tree -p xtask` command**: it counts dev edges and
  would fail on an edge that cannot recreate the bootstrap deadlock; rejected.

## Consequences

The widened dev graph is the accepted cost. The orchestrator edits the rule text;
the architect does not.

## Links

- [Decision DDD-12](../../feature/metal-test-placement-attribute/feature-delta.md#ddd-12--dev-dependencies-and-the-xtask-rule-f-7)
