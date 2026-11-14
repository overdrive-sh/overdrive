# Slice 07 — Unplaced test fails before the Metal box

Story: US-PLACE-07. Job: J-PLAT-007. Depends on: slices 02 and 03 (the host-pure, lane and launcher forms its rule and its fixture-existence check read). It lands before the migrations, because its rule is true at every migration stage.

## Outcome

An integration test with no placement declaration fails a check that runs in a command Marcus already uses and boots no guest, so the miss never reaches the Metal box and a plain `#[test]` cannot silently run on the host.

## In the slice

- `cargo xtask placement-lint` (DESIGN DDD-3, approved): every listed test has its own record, an applicable module or binary declaration, or (until retirement) a table entry; it also reports a binary that mixes a default with a non-host-pure record and a lane fixture that is not a listed test.
- A failure naming the binary and the test.
- A negative fixture: one deliberately unplaced test proves the check fires.
- A unit-lane test is placed by a module or binary declaration and flagged otherwise (DDD-4).

## Not in the slice

Choosing which placement a test should have; adding a new CI lane or tool (proposal to the user first).

## Entry point and evidence

`cargo xtask placement-lint`, wired into CI and pre-push beside `dst-lint` and `conformance-lint`. Evidence: the negative fixture fails naming the test; a clean tree passes; no later Metal run emits `test_unclassified`.

## Acceptance

- [ ] Unplaced integration test fails before any guest boots, naming binary and test.
- [ ] Fully declared binaries pass; a declared test beside table-listed tests does not trip it.
- [ ] A unit-lane test with a module or binary declaration passes; one without is flagged.
- [ ] A binary with `overdrive_test_default!(host_pure)` and a non-host-pure record is flagged `placement_conflict`.
- [ ] `cargo xtask placement-lint` runs in CI and pre-push beside `dst-lint` and `conformance-lint`.
