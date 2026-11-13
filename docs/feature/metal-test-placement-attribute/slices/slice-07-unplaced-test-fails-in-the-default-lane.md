# Slice 07 — Unplaced test fails before the Metal box

Story: US-PLACE-07. Job: J-PLAT-007. Depends on: slices 05, 06 (or a DESIGN-defined per-binary scope, open point O3).

## Outcome

An integration test with no placement declaration fails a check that runs in a command Marcus already uses and boots no guest, so the miss never reaches the Metal box and a plain `#[test]` cannot silently run on the host.

## In the slice

- A check that every test in an integration binary declares a placement (mechanism and spelling are DESIGN's; the issue says only "a lint can enforce it").
- A failure naming the binary and the test.
- A negative fixture: one deliberately unplaced test proves the check fires.
- Tests outside integration binaries are not flagged.

## Not in the slice

Choosing which placement a test should have; adding a new CI lane or tool (proposal to the user first).

## Entry point and evidence

Candidate: `cargo xtask metal run -- cargo check --all-targets --features integration-tests` (the compile-check the rules prescribe for integration binaries). If DESIGN picks another command, that command is named here at DESIGN. Evidence: the negative fixture fails naming the test; a clean tree passes; no later Metal run emits `test_unclassified`.

## Acceptance

- [ ] Unplaced integration test fails before any guest boots, naming binary and test.
- [ ] Fully declared binaries pass; a declared test beside table-listed tests does not trip it.
- [ ] Unit-lane tests are ignored.
- [ ] The check runs in a step the repository already runs for that surface.
