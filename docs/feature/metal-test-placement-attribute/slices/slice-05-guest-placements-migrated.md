# Slice 05 — Guest placements migrated

Stories: US-PLACE-05a, 05b, 05c. Job: J-PLAT-007. Depends on: slices 01, 04 (and 03 where a batch holds lane or native-launcher entries).

## Outcome

All 771 guest-group placements live on their tests; the table holds no `guest-group` entry; no test changes kind or group.

## Batches (one story each, 1-3 days, scripted)

| Story | Batch |
|---|---|
| 05a | groups `core`, `probe`, `unload`, `ulp`, `stress`, `nested-boundary` |
| 05b | group `mtls-relay`, including `overdrive-bpf::integration` |
| 05c | group `cgroup-default` |

A batch estimated over 3 days is split by binary id with the same acceptance.

## In each batch

- Move each test's declaration onto the test and delete its JSON entry.
- Compare, per test, kind and group before and after; the comparison parses the JSON (no `grep -c`).
- Run the migrated binaries on Metal and compare pass sets.

## Not in the slice

Host-pure entries (slice 06); re-placing any test (a changed placement is reported as a finding).

## Entry point and evidence

`cargo xtask metal run -- cargo nextest run --features integration-tests,kvm-tests` over the batch's binaries; rename one batch test (for example `tcp_syn_plus_rst_flags_drops_with_malformed_header_counter` for 05b) and re-run with zero JSON edits.

## Acceptance

- [ ] Batch entries removed; per-test kind and group identical before and after.
- [ ] Pass set per migrated binary equals the pre-migration pass set.
- [ ] A migration that would change a group is rejected and reported.
- [ ] Unmigrated batches still run as before.
- [ ] After 05a-c: `guest-group` entries are 771 to 0.
