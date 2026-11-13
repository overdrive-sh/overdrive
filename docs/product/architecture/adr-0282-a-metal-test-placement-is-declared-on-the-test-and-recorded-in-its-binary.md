# ADR-0282 — A Metal test's placement is declared on the test and recorded in its binary

## Status

**Proposed — awaiting user approval (DDD-1, DDD-2, DDD-3 in the feature delta).**
If accepted, it supersedes in part ADR-0280: the placement *source* (a
hand-kept table keyed by binary id and test name) is replaced; ADR-0280's
decision that nextest stays the selection, retry, report and result owner and
places each exact body through its per-test wrapper is kept.

## Context

ADR-0280's placement table is implemented and in use (4,642 entries). It
names the isolation each test needs in a file away from the test, so a new or
renamed test is unplaced until someone edits the file, and the only gate that
catches a miss is the wrapper's per-test refusal on the Metal box. The guest
groups are also listed in the wrapper and in the dispatcher. The walking
skeleton showed a test can carry its own placement as a record in its own
binary and be placed by the wrapper without a table lookup.

## Decision

A test declares its placement on itself. The declaration is written into the
test binary as a versioned record in the `overdrive_test_placement` ELF
section, and the Metal wrapper reads that record to place the test. A test
with no record, and no accepted default, is refused before its body runs. An
eager listing check applies the same rule across every listed test before any
guest boots. The table and its lookup are retired once every test is declared.

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

## Consequences

One edit site per placement. The record format and the wrapper's reader are a
cross-process contract, pinned in the feature delta. A linker that drops the
record leaves the test unplaced, which is refused, never run on the host. The
unplaced-test guard moves from the table to the record.

## Links

- [Interface contract and decisions](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-interface-contracts)
- [ADR-0280](adr-0280-metal-nextest-execution-uses-explicit-test-placement.md)
- [Walking skeleton findings](../../feature/metal-test-placement-attribute/spike/findings.md)
