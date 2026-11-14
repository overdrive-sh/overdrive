# Slice 02 — Host-pure placement in the test source

Story: US-PLACE-02. Job: J-PLAT-007. Depends on: slice 01.

## Outcome

A pure integration test is placed host-pure by a declaration in the test source, needs no table entry, and runs on Metal with no guest and no namespace, exactly as a table host-pure test does today. The declaration is on the test (`#[overdrive_test(host_pure)]`), on its module (`placement!(host_pure)`) or the binary's default (`overdrive_test_default!(host_pure)`); precedence is test, then module, then binary default (DDD-4).

## In the slice

- The three host-pure declarations (grammar in the feature delta's interface contracts) and their precedence.
- A child module does not inherit its parent's declaration.
- A binary with a default and a non-host-pure record is refused as `placement_conflict`.
- Wrapper behaviour for a host-pure test: original runner chain, no namespace, no table lookup.
- One or two representative host-pure tests, one module and one host-only binary migrated with their entries deleted.

## Not in the slice

Bulk migration (slice 06), the lint (slice 07).

## Entry point and evidence

`cargo xtask metal run -- cargo nextest run -p overdrive-store-local --features integration-tests -E 'test(malformed_intent_bytes_cause_refuse_to_start)'`. Evidence: PASS without a guest boot; the same test with its entry deleted does not fail `test_unclassified`.

## Acceptance

- [ ] A test placed host-pure by its own declaration, its module's `placement!(host_pure)` or its binary's `overdrive_test_default!(host_pure)` passes on Metal with no table entry.
- [ ] Precedence is test, then module, then binary default; a module declaration with a guest-declared test in it leaves that test in its guest.
- [ ] A child module is not placed by its parent's declaration.
- [ ] A binary with a default and a non-host-pure record is refused as `placement_conflict`.
- [ ] No guest is booted for a host-pure test.
- [ ] Runner chain and isolation equal today's host-pure behaviour.
- [ ] The existing `invalid_run_context` diagnostics still fire for a declared test.
- [ ] Every declaration is in the test source.

## Risk

A mislabelled test is not made safe by the declaration; the testing.md rule that node-global mutators are never host-pure stays in force and is a review item.

## Estimate

1 to 2 days.
