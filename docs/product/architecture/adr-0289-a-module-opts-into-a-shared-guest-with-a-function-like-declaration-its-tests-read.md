# ADR-0289 — A module opts into a shared guest with a function-like declaration its tests read

## Status

**Shape approved by the user, 2026-11-13; independent DESIGN review pending.**
DDD-1 and DDD-7 in the feature delta. The hidden item's name and the conflict
rule are proposed.

## Context

Sharing a guest is a property of a module, not of one test. A placement
attribute on the module would state it once, but on stable Rust a proc-macro
attribute on `mod x;`, or on an inline `mod` that contains one, fails with
E0658 (`proc_macro_hygiene`), and an inner custom attribute needs
`custom_inner_attributes`. The repository's integration entrypoints use exactly
`mod integration { mod a; mod b; }`.

## Decision

A module declares sharing once with a plain function-like macro,
`shared_guest!(guest = "<group>"[, profile = "<profile>"])`, which expands to a
module-private item recording the group, the profile and the declaring module's
path. A test with `#[overdrive_test]` and no arguments names that item by plain
path, so ordinary name resolution binds the declaration of the module the test
sits in, and a test with no declaration in scope does not compile. The sharing
scope is the declaration (the module recorded in it), so a child module that
imports the declaration deliberately shares its guest. A test with an explicit
placement of its own overrides the declaration for that test.

## Alternatives

- **A per-test `shared` flag**: a flag repeated on every test lets one test
  omit it or name another profile, which splits the module across guests;
  rejected by the user, 2026-11-13.
- **An attribute on the module**: refused by stable Rust (above); rejected.
- **A separate attribute per placement kind**: five macros and five error
  surfaces for one concept; rejected.
- **A compile error when an explicit placement meets a declaration in the same
  module**: not buildable by name resolution (the explicit form cannot observe a
  sibling declaration); rejected.

## Consequences

A missing declaration surfaces as rustc's E0425 naming the hidden item, whose
name is the instruction. The sharing scope and the owner it selects are
ADR-0283.

## Links

- [Decision and grammar](../../feature/metal-test-placement-attribute/feature-delta.md#ddd-1--attribute-surface-question-1)
- [Probe findings](../../feature/metal-test-placement-attribute/spike/findings-module-declaration.md)
- [ADR-0283](adr-0283-the-tests-of-a-module-share-one-guest-through-a-per-module-owner-process.md)
