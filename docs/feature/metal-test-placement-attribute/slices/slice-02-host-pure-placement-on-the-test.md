# Slice 02 — Host-pure placement on the test

Story: US-PLACE-02. Job: J-PLAT-007. Depends on: slice 01.

## Outcome

A pure integration test declares host-pure placement on itself, needs no table entry, and runs on Metal with no guest and no namespace, exactly as a table host-pure test does today.

## In the slice

- A host-pure declaration on the test (spelling pinned by DESIGN; the issue names `host_pure`).
- Wrapper behaviour for a declared host-pure test: original runner chain, no namespace, no lookup.
- One or two representative host-pure tests migrated with their entries deleted.

## Not in the slice

Bulk migration (slice 06), the lint (slice 07).

## Entry point and evidence

`cargo xtask metal run -- cargo nextest run -p overdrive-store-local --features integration-tests -E 'test(malformed_intent_bytes_cause_refuse_to_start)'`. Evidence: PASS without a guest boot; the same test with its entry deleted does not fail `test_unclassified`.

## Acceptance

- [ ] Declared host-pure test passes on Metal with no table entry.
- [ ] No guest is booted for it.
- [ ] Runner chain and isolation equal today's host-pure behaviour.
- [ ] The existing `invalid_run_context` diagnostics still fire for a declared test.
- [ ] The declaration is visible on the test.

## Risk

A mislabelled test is not made safe by the declaration; the testing.md rule that node-global mutators are never host-pure stays in force and is a review item.

## Estimate

1 day.
