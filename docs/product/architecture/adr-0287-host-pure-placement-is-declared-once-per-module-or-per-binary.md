# ADR-0287 — Host-pure placement is declared once per module or per binary

## Status

**Approved by the user, 2026-11-14 (U-4); independent DESIGN review pending.**
DDD-4 in the feature delta.

## Context

Once the placement table is gone (ADR-0282), the 3,855 host-pure and unit-lane
tests in the workspace need a placement. 66 host-only binaries hold 2,222 of
them; 21 mixed binaries hold 1,635 host tests beside guest-placed ones.

## Decision

Two declarations exist, both accepting only `host_pure`. A per-binary default
(`overdrive_test_default!`) applies to every test of a binary that has neither
its own attribute nor a module declaration, and is allowed only in a binary
with no guest-placed, lane-placed or native-launcher test (a binary that has the
default and any such record is refused as `placement_conflict`). A per-module
declaration (`placement!`) applies to the tests of exactly that module; a child
module does not inherit it. Precedence: the test's own attribute, then its
module's declaration, then the binary default. Each declaration is a named
item, so a duplicate does not compile.

## Alternatives

- **Every test declares** (about 4,640 sites; every future unit test needs the
  attribute): the most churn for no placement gain; rejected.
- **A per-binary default only**: cannot serve the 21 mixed binaries without a
  per-test attribute on all 1,635 host tests; rejected as the only form.
- **A per-module declaration only**: needs one in every module of the 66
  host-only binaries where one line per binary suffices; rejected as the only
  form.
- **A binary default that allows mixing**: a forgotten declaration in a mixed
  binary silently runs on the host, the failure the guard exists to prevent;
  rejected.
- **A read-only module declaration** (tests still carry a bare attribute): the
  every-test-declares option with a shorter spelling; rejected.

## Consequences

One line places a host-only binary; a mixed binary needs a declaration per
host-pure module. The declaration count per module is measured by the
migration script.

## Links

- [Decision and grammar](../../feature/metal-test-placement-attribute/feature-delta.md#ddd-4--host-pure-and-unit-lane-scale-f-3)
- [ADR-0282](adr-0282-a-metal-test-placement-is-declared-on-the-test-and-recorded-in-its-binary.md)
