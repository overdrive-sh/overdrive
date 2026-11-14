# Slice 01 — Guest placement on the test (walking skeleton)

Story: US-PLACE-01. Job: J-PLAT-007. Status: production code, promoted (spike gate PROMOTE, user decision 2026-11-13); DESIGN approved by the user, 2026-11-14.

## Outcome

One existing `cgroup-default` integration test declares its placement with `#[overdrive_test(guest = "cgroup-default")]`, has no entry in `infra/metal/test-placement.json`, and passes on the real Metal path with its body running inside a fresh guest.

## In the slice

1. New proc-macro crate providing the attribute, guest case only.
2. A guest-kit library extracted out of `fwd_guest_suite.rs` exposing `inside_guest() -> bool` and `run_in_guest(group, exe, test_name)`. The host test fails when the in-guest run fails. The dispatcher keeps working through the extracted code.
3. One migrated test (candidate: `integration::cluster_and_node_commands::cluster_status_with_no_server_returns_transport_error`, final pick at slice start); its JSON entry deleted; `test-wrapper.sh` runs it without a table lookup.
4. Per-test guest boot time recorded (one guest per test).

## Not in the slice

`host_pure`, guest-lane fixtures, native-launcher, per-group artifact declaration, the rest of the table, deleting the dispatcher, the lint.

## Entry point and evidence

`cargo xtask metal run -- cargo nextest run -p <crate> --features integration-tests,kvm-tests -E 'test(<name>)'` on the native Metal box (Lima is no signal). Evidence: the PASS line, the malformed-run-id host FAIL, and the boot-time record. No test-only wiring may stand in for the wrapper or guest kit.

## Acceptance

- [ ] Migrated test PASSes on Metal with no table entry.
- [ ] On the shared-guest path a failing in-guest body yields a host FAIL for that test and a nonzero run status. This criterion is verified when slice 09 lands; the per-test path's failure mapping has no scenario of its own (DISTILL D-7).
- [ ] A guest that cannot start (a Metal run with a malformed run id) is a host FAIL naming the group and stage; the body never runs on the host.
- [ ] An unknown group is rejected by name.
- [ ] Outside a Metal run the test runs its body in place, as it does today; on a Metal run its body never runs on the host (user ruling, 2026-11-14).
- [ ] An unmigrated guest test still passes through the dispatcher after the extraction.
- [ ] Boot time for the migrated test is retained.
- [ ] No GitHub issue number or roadmap-step id appears in any new identifier.

## Points the spike resolved

- O1: a self-declared test is told apart from an unplaced one by its ELF-section record (DDD-2); the host-side body runs in the wrapper's local-mode vsock namespace.
- The in-guest marker is `OVERDRIVE_TEST_IN_GUEST` (DDD-10); the general shape is DESIGN's.

## Estimate and gate

1 to 3 days. Promotion gate decided (PROMOTE) by the spike in `docs/feature/metal-test-placement-attribute/spike/`; findings feed DESIGN.
