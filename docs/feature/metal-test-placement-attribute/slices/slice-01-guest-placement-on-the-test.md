# Slice 01 — Guest placement on the test (walking skeleton)

Story: US-PLACE-01. Job: J-PLAT-007. Status: production code, promotion pending the spike's gate decision (user-approved skeleton contract, 2026-11-13).

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

`cargo xtask metal run -- cargo nextest run -p <crate> --features integration-tests,kvm-tests -E 'test(<name>)'` on the native Metal box (Lima is no signal). Evidence: the PASS line, a deliberate in-guest failure producing a host FAIL, and the boot-time record. No test-only wiring may stand in for the wrapper or guest kit.

## Acceptance

- [ ] Migrated test PASSes on Metal with no table entry.
- [ ] Making the in-guest body fail yields a host FAIL with the guest output visible and a nonzero run status.
- [ ] A guest that cannot boot or exceeds its bound is a host FAIL naming the group and stage.
- [ ] An unknown group is rejected by name.
- [ ] Outside a Metal run the test runs its body in place, as it does today; on a Metal run its body never runs on the host (user ruling, 2026-11-14).
- [ ] An unmigrated guest test still passes through the dispatcher after the extraction.
- [ ] Boot time for the migrated test is retained.
- [ ] No GitHub issue number or roadmap-step id appears in any new identifier.

## Risks for the spike to resolve and record

- O1: how the wrapper treats a self-declared test versus an unplaced one while the table coexists, and who provides the local-mode vsock namespace to the host-side body.
- The in-guest marker mechanism and how the host test binary reaches the guest are DESIGN's to generalise; the skeleton may choose the thinnest working form.

## Estimate and gate

1 to 3 days. Promotion gate decided by the spike in `docs/feature/metal-test-placement-attribute/spike/`; findings feed DESIGN.
