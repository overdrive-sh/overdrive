# Slice 06 — Host-pure placements migrated

Stories: US-PLACE-06a, 06b, 06c. Job: J-PLAT-007. Depends on: slices 02, 07.

## Outcome

All 3,855 host-pure placements live on their tests; the table holds no `host-pure` entry; no test changes kind.

## Batches (one story each, 1-3 days, scripted)

| Story | Batch |
|---|---|
| 06a | `overdrive-core*` and `overdrive-sim*` binary ids |
| 06b | `overdrive-control-plane*`, `overdrive-worker*`, `overdrive-cli*`, `overdrive-host*`, `overdrive-gateway`, `overdrive-reconcilers*`, `overdrive-store-local*` |
| 06c | all remaining binary ids (`overdrive-dataplane*`, `overdrive-fwd*`, `overdrive-init*`, `overdrive-netlink`, `overdrive-system-conformance*`, `overdrive-testing*`, `xtask*`, fixtures) |

A batch estimated over 3 days is split by binary id with the same acceptance.

## In each batch

- Declare in the test source per DDD-4 and delete the JSON entries: a host-only binary takes one `overdrive_test_default!(host_pure)`; a mixed binary takes `placement!(host_pure)` in each host-pure module (a child module declares its own; none inherits); a test may instead carry its own attribute.
- Compare per-test kind before and after by parsing the JSON (the parity check of DDD-15).
- A test found to mutate node-global kernel state (nft, routes, links, BPF, cgroups, netns, vsock) while declared host-pure is reported as a finding, not carried forward.
- Run the migrated binaries on Metal and compare pass sets.

## Not in the slice

Guest placements (slice 05); changing any test's kind.

## Entry point and evidence

`cargo xtask metal run -- cargo nextest run --features integration-tests` over the batch's binaries; rename one batch test (for example `malformed_intent_bytes_cause_refuse_to_start` for 06b) and re-run with zero JSON edits.

## Acceptance

- [ ] Batch entries removed; kind identical per test.
- [ ] Pass sets equal pre-migration.
- [ ] Mis-placements reported, not carried over.
- [ ] A binary with a default and a non-host-pure record is refused as `placement_conflict`; mixed binaries are placed per module.
- [ ] A child module declares its own `placement!(host_pure)`; it does not inherit its parent's.
- [ ] After 06a-c: `host-pure` entries are 3,855 to 0.
