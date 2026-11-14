# ADR-0282 — A Metal test's placement is declared on the test and recorded in its binary

## Status

**Proposed — awaiting user approval (DDD-1, DDD-2 in the feature delta).**
The eager listing check (`cargo xtask placement-lint`, DDD-3) and the host-pure
declarations (per module and per binary, DDD-4) are approved by the user,
2026-11-14 (U-1, U-4). If accepted, it supersedes in part ADR-0280: the placement *source* (a
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

A test declares its placement on itself, or, for the tests of a module that
share a guest, through one declaration in that module which the tests read by
name (user-approved shape, 2026-11-13; see ADR-0283). The placement is written into the
test binary as a versioned record in the `overdrive_test_placement` ELF
section, and the Metal wrapper reads that record to place the test. A host-pure
placement may also be declared once for a module (`placement!(host_pure)`,
applying to the tests of exactly that module) or once for a binary
(`overdrive_test_default!(host_pure)`, allowed only in a binary with no
guest-placed, lane-placed or native-launcher test); precedence is the test's own
attribute, then its module's declaration, then the binary default. A test
with no record and no applicable declaration is refused before its body runs.
An eager listing check (`cargo xtask placement-lint`) applies the same rule, and
checks that every lane fixture exists, across every listed test before any
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
- **Every test declares, with no module or binary declaration** (about 4,640
  sites): the most churn for no placement gain; rejected for host-pure tests.
- **A per-binary default only**: cannot serve the 21 binaries that mix host-pure
  and guest-placed tests, so those would need a per-test attribute on all 1,635
  host tests; rejected as the only form.
- **A per-module declaration only**: needs a declaration in every module of 66
  host-only binaries (2,222 tests) where one line per binary suffices; rejected
  as the only form.

## Consequences

One edit site per placement. The record format and the wrapper's reader are a
cross-process contract, pinned in the feature delta. A linker that drops the
record leaves the test unplaced, which is refused, never run on the host. The
unplaced-test guard moves from the table to the record.

## Links

- [Interface contract and decisions](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-interface-contracts)
- [ADR-0280](adr-0280-metal-nextest-execution-uses-explicit-test-placement.md)
- [Walking skeleton findings](../../feature/metal-test-placement-attribute/spike/findings.md)
