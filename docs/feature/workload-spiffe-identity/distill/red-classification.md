# RED classification — workload-spiffe-identity (DISTILL)

Run on the Metal box (`cargo xtask metal run --`), kernel 7.0.0-38, source = the working tree at hand-off. Classes: `MISSING_FUNCTIONALITY` (correct RED), `GUARD` (green today by design), `HELD` (cannot compile until DELIVER's API change).

## Regression anchor

`tests/conformance/tests/public_ingress_gateway_end_to_end.rs`
`native_metal_kvm_public_ingress_gateway_answers_503_until_the_hydrator_reprograms_after_a_serve_restart` (S-PIG-71), unchanged. A guest-placed conformance test: its RED is proven by running it, not by `should_panic`. Why it is not `should_panic`: it is a guest-placed conformance test whose RED is the run itself; the Metal lane stays red until DELIVER lands the fix. Its tracing events are recorded in-process and not printed, so the run output carries no `BackendIdentityConflict` line; the refusal text is attached under the S-WSI-09 row below. Run `248f49fc9cb9dafe6a23136f` (`cargo xtask metal run -- cargo nextest run -p overdrive-system-conformance --features integration-tests -E 'test(answers_503_until_the_hydrator_reprograms_after_a_serve_restart)'`): FAIL in 78 s, `tests/conformance/tests/public_ingress_gateway_end_to_end.rs:665`: "the backend answers within the 60 s recovery window", observed `[(false, 0, false), (true, 502, true), (true, 502, true), ...]` for the whole window (a persistent 502). Class: MISSING_FUNCTIONALITY.

## Pending tests (each passes today through a matched `#[should_panic]`)

| Test | Class | Matched failure |
|---|---|---|
| `workload_spiffe_identity::a_workload_identity_round_trips_to_its_workload` | MISSING_FUNCTIONALITY | `SpiffeId::for_workload is not delivered yet` (scaffold) |
| `workload_spiffe_identity::the_payments_workload_has_the_documented_identity` | MISSING_FUNCTIONALITY | same |
| `workload_spiffe_identity::the_superseded_allocation_shaped_identity_names_no_workload` | MISSING_FUNCTIONALITY | `SpiffeId::workload_id is not delivered yet` |
| `workload_spiffe_identity::identities_that_are_not_exactly_a_workload_path_name_no_workload` | MISSING_FUNCTIONALITY | same |
| `issued_certificate_row_v1_rows_are_refused_and_never_translated` | MISSING_FUNCTIONALITY | "a V1 issued-certificate row must be refused, it decoded" (the pre-V2 literal; the V2-rooted literal is then asserted to be an `Envelope` error) |
| `issued_certificate_row_v2_golden_bytes_roundtrip_preserves_issued_for` | MISSING_FUNCTIONALITY | "the V2 issued-certificate row must decode" (unknown version today) |
| `issued_certificate_row_envelope_declares_v1_and_v2_with_the_repinned_offset` | MISSING_FUNCTIONALITY | `[0]`/`104` vs `[0, 1]`/`120` |
| `vm_recreation_allocation_identity::a_replacement_allocation_presents_the_identity_of_its_workload` | MISSING_FUNCTIONALITY | identity is `.../workload/<w>/alloc/<a>` |
| `vm_recreation_allocation_identity::an_initial_allocation_presents_the_identity_of_its_workload` | MISSING_FUNCTIONALITY | same |
| `workload_svid_identity_shared_by_allocations::two_allocations_of_one_workload_each_hold_their_own_svid_naming_the_workload` | MISSING_FUNCTIONALITY | "an allocation's SVID names its workload" (own-serial assertion passes first) |
| `workload_svid_identity_shared_by_allocations::after_a_serve_restart_both_allocations_recover_their_own_svid_for_the_workload` | MISSING_FUNCTIONALITY | "after a restart an allocation's SVID names its workload" |
| `workload_identity_replacement::one_call_naming_an_endpoint_twice_is_refused_and_changes_nothing` | MISSING_FUNCTIONALITY | "the same workload twice at one endpoint must be LoadFailed" |
| `workload_identity_replacement::a_replacement_converges_without_refusal_across_restarts_and_schedules` | MISSING_FUNCTIONALITY | "the dataplane refused the update" (the refusal is `backend id 1 is bound to spiffe://overdrive.local/workload/payments/alloc/alloc-payments-0, refusing spiffe://overdrive.local/workload/inventory/alloc/alloc-inventory-0`, shrunk seed 0, observed with `--no-capture`) |
| `workload_spiffe_identity::every_allocation_of_a_workload_is_issued_the_workload_identity` | MISSING_FUNCTIONALITY | "every allocation is issued its workload's identity" (`IssueSvid` carries the per-allocation identity) |
| `service_backend_names_its_workload::a_running_allocation_is_published_behind_its_workloads_identity` | MISSING_FUNCTIONALITY | "a Running allocation is published behind its workload's identity" |
| `host_and_sim_agree_that_a_different_workload_at_a_memoised_endpoint_keeps_the_id` | MISSING_FUNCTIONALITY | "a different workload at a memoised endpoint must be accepted" |
| `host_and_sim_refuse_one_call_that_names_an_endpoint_twice` | MISSING_FUNCTIONALITY | "the same workload twice at one endpoint must be LoadFailed" |
| `selection_receipt_different_workload_takeover` | MISSING_FUNCTIONALITY | "update_service must be accepted: backend id ... is bound to" |
| `selection_receipt_receipt_conf_randomized_traces` | MISSING_FUNCTIONALITY | "is bound to spiffe://overdrive.local/workload/conf-" (pinned `seed = "1"`, `max_samples = 100`; the matched RED reproduced in the gate run) |

## Guards (green today, pin the post-feature contract)

`host_and_sim_agree_that_a_replacement_allocation_of_the_same_workload_changes_nothing` (identical `Backend` today; the per-allocation case is RED in S-WSI-08/09), `selection_receipt_replacement_reuses_backend_id`, `selection_receipt_serve_restart_replacement`, `a_peer_presenting_another_identity_than_the_selected_workloads_is_rejected_exactly`. Plus the re-shaped existing tests (S-PIG-67, S-PIG-131, the gateway integration doubles, the `vm_recreation` suite).

## Held (cannot compile until DELIVER)

`svid_lifecycle_allocation_keyed_recovery.rs` (2 tests), `alloc_status_per_allocation_certificates.rs` (1 test): gated `#![cfg(any())]` with the `distill-pending-owner-api` marker; see the feature delta, "Held tests". Compile proof (throwaway worktree, never committed, local patch re-keying `ever_issued` and wiring the V2 row): both files compile with every target of `overdrive-core`, `overdrive-reconcilers` and `overdrive-control-plane` under `integration-tests`; the recovery test passes there and `held_attribution` fails only on the unimplemented `for_workload` scaffold. On the same patch the V2 schema-evolution tests (golden bytes, offset 120 with `[0, 1]`, V1-under-V2-root refusal) pass, so `FIXTURE_V2` and `FIXTURE_V1_UNDER_V2_ROOT` are measured, not hand-assumed.

## Gate results (Metal)

- `cargo fmt --all -- --check`: every new and touched file is clean; the only diffs are pre-existing and untouched here (`overdrive-bench-families/src/families/cgroup_connect_hook.rs`, `overdrive-testing/bin/guest_peer.rs`, `xtask/src/conformance_lint.rs`).
- `cargo check --workspace --all-targets`, with and without `--features integration-tests`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`, with and without `--features integration-tests`: clean.
- `cargo xtask placement-lint`, `cargo xtask conformance-lint`: exit 0.
- Scoped run of every new and changed test plus their siblings (`overdrive-core`, `-reconcilers`, `-control-plane`, `-sim`, `-dataplane`, `-gateway`): green, each pending test through its matched `#[should_panic]`.
- S-PIG-71 re-run: FAIL as classified above (persistent 502 for the 60 s window).
