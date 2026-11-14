# ADR-0282 — A Metal test's placement is declared on the test and recorded in its binary

## Status

**Accepted — approved by the user, 2026-11-14 (DDD-1, DDD-2 in the feature delta).**
It supersedes in part ADR-0280: the placement *source* (a
hand-kept table keyed by binary id and test name) is replaced; ADR-0280's
decision that nextest stays the selection, retry, report and result owner and
places each exact body through its per-test wrapper is kept.

## Context

ADR-0280's placement table is implemented and in use (4,642 entries). It
names the isolation each test needs in a file away from the test, so a new or
renamed test is unplaced until someone edits the file, and the only gate that
catches a miss is the wrapper's per-test refusal on the Metal box. The walking
skeleton showed a test can carry its own placement as a record in its own
binary and be placed by the wrapper without a table lookup.

## Decision

A test declares its placement on itself. The placement is written into the test
binary as a record in the `overdrive_test_placement` ELF section, and the Metal
wrapper reads that record to place the test. A test with no record and no
applicable declaration (ADR-0287) is refused before its body runs. While the
table exists a record wins over a table entry; the table and its lookup are
retired once every test is declared.

## Alternatives

- **A generated table** (a build step derives the table from the tests):
  keeps a second artifact that can go stale and needs a build hook per crate;
  rejected.
- **A lint-only fix over the existing table** (check the table is complete):
  keeps three places that define placement; rejected.
- **A runtime registry the binary prints on request** (`inventory`-style):
  libtest's `main` cannot answer a placement query, so a new harness per binary
  would be needed; rejected.
- **Encoding placement in the test name**: changes every name, breaks `-E`
  selection and history; rejected.

The record and the declaration on the test are one decision: a declaration the
wrapper cannot read at run time places nothing, and the only transport the
alternatives above leave is the binary's own section.

## Consequences

One edit site per placement. The record format and the wrapper's reader are a
cross-process contract, pinned in the feature delta (three readers: the macro
writes, the wrapper and `placement-lint` read). A linker that drops the record
leaves the test unplaced, which is refused, never run on the host.

## Links

- [Interface contract and decisions](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-interface-contracts)
- [ADR-0280](adr-0280-metal-nextest-execution-uses-explicit-test-placement.md)
- [ADR-0287](adr-0287-host-pure-placement-is-declared-once-per-module-or-per-binary.md)
- [ADR-0288](adr-0288-an-eager-placement-lint-checks-every-listed-test-before-any-guest-boots.md)
- [Walking skeleton findings](../../feature/metal-test-placement-attribute/spike/findings.md)
