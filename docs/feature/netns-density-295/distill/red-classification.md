# Pre-DELIVER RED classification — `netns-density-295`

All commands below ran in an aarch64 Linux Docker container with the workspace
mounted read-only and a zero-byte build-only `OVERDRIVE_BPF_OBJECT` override.
That environment compiles Rust/aya/netlink surfaces but is not kernel evidence.
`cargo xtask lima run -- true` failed because the existing `overdrive` Lima VM
refused SSH before and after restart. A host-native Darwin retry stopped in the
Linux-only `linux-keyutils` dependency before repository code compiled, so it
is not RED evidence. `OVERDRIVE_METAL_TARGET` is unset.

## Phase-02 non-waived remediation bodies

| Scenario / body | Explicit command | Observed failure | Classification |
|---|---|---|---|
| S-ND295-00 `every_locked_aya_map_kind_projects_to_exact_or_opaque_semantics` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::every_locked_aya_map_kind_projects_to_exact_or_opaque_semantics -- --ignored --exact` | `D12 map-kind projection` RED panic | **RED — MISSING_FUNCTIONALITY**; body reached the exact private production projection scaffold. |
| S-ND295-00 `wrong_valid_map_properties_remain_opaque_and_schema_mismatch_is_source_less` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::wrong_valid_map_properties_remain_opaque_and_schema_mismatch_is_source_less -- --ignored --exact` | `D12 map-schema projection` RED panic | **RED — MISSING_FUNCTIONALITY**; imports, opaque semantic types, and error oracle compile. |
| S-ND295-00 `capture_failure_keeps_an_observation_identity_and_only_the_first_genuine_source` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::capture_failure_keeps_an_observation_identity_and_only_the_first_genuine_source -- --ignored --exact` | `D12 inventory capture` RED panic | **RED — MISSING_FUNCTIONALITY**; program and link enumeration both retain outer `Program` while distinguishable nested `IOError(EIO)`/`IOError(ENOENT)` values prove the earlier program source wins; link observation remains exact source-less unavailability. |
| S-ND295-00 `a_unique_unreceipted_candidate_is_ambiguous_and_never_an_owned_count` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::a_unique_unreceipted_candidate_is_ambiguous_and_never_an_owned_count -- --ignored --exact` | `D12 inventory capture` RED panic | **RED — MISSING_FUNCTIONALITY**; no-receipt ambiguity contract compiles. |
| S-ND295-00 `every_receipted_family_returns_one_and_clean_families_return_exact_zero` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::every_receipted_family_returns_one_and_clean_families_return_exact_zero -- --ignored --exact` | `D12 inventory capture` RED panic | **RED — MISSING_FUNCTIONALITY**; all eight exact zero/positive receipt assertions compile behind the missing capture implementation. |
| S-ND295-00 `startup_tcp_probe_projects_semantics_and_all_eight_counter_pairs_without_raw_abi` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::startup_tcp_probe_projects_semantics_and_all_eight_counter_pairs_without_raw_abi -- --ignored --exact` | `D14 TCP probe projection` RED panic | **RED — MISSING_FUNCTIONALITY**; the private table now includes complete-length EtherType/version/IHL/protocol/destination-port failures plus short output, both inputs, exact counters, and decrease/wrap without raw ABI leakage. |
| S-ND295-00 `every_d14_semantic_mismatch_and_lower_source_reaches_the_production_validator` | `cargo test -p overdrive-control-plane --lib guest_network::scratch_probe_packet_acceptance::every_d14_semantic_mismatch_and_lower_source_reaches_the_production_validator -- --ignored --exact` | `D14A semantic TCP validator` RED panic | **RED — MISSING_FUNCTIONALITY**; the table covers both invalid verdicts, all three invalid marks, isolated destination IP/port errors, Intercept zero/oversized deltas, an identity permutation, every single mismatch/lower source, and every adjacent first-mismatch precedence through ordered counter identity → decrease → delta. |
| S-ND295-00 `classifier_runs_precede_close_and_each_stage_is_fresh` | `cargo test -p overdrive-control-plane --lib guest_network::scratch_probe_packet_acceptance::classifier_runs_precede_close_and_each_stage_is_fresh -- --ignored --exact` | current D5 closes/adopts/queries before classifier exercise | **RED — MISSING_FUNCTIONALITY**; the new and transitioned D5 tables require the sole exercise-before-close order, typed/semantic refusal, lazy adoption, and complete cleanup/inventory. |
| S-ND295-00 `production_startup_exercises_classifier_and_detached_guard_before_admission` | `cargo xtask lima run -- cargo test -p overdrive-control-plane --test integration --features integration-tests integration::shared_guest_network_startup::production_startup_exercises_classifier_and_detached_guard_before_admission -- --ignored --exact` | executable body compiled/listed, but the existing Lima VM still refused SSH after a forced restart | **PENDING_ENVIRONMENT**; ordinary `run_server` is wrapped by the existing tracing subscriber and asserts the exact five ordered production events, continuous counters, one program id, D9 deltas, zero delivery, and fifteen `Observed(0)` fields; no monitor, poll, sleep, hook, or test panic exists. |
| S-ND295-11 `persistent_tap_and_bridge_projection_preserves_every_observable_identity_field` | `cargo test -p overdrive-netlink --lib client::tests::persistent_tap_and_bridge_projection_preserves_every_observable_identity_field -- --ignored --exact` | `D12A TAP identity projection` RED panic | **RED — MISSING_FUNCTIONALITY**; raw absent/TAP-with-exact-or-missing-owner/TUN/dummy/veth/other and correct/wrong-kind bridge rows reach the retained private projection boundary. |
| S-ND295-11 `provision_reads_every_attachment_fact_before_reporting_success` | `cargo test -p overdrive-control-plane --lib guest_network::allocation_owner_acceptance::provision_reads_every_attachment_fact_before_reporting_success -- --ignored --exact` | production owner returned with call trace `[]`, not the exact 16-call D12A sequence | **RED — MISSING_FUNCTIONALITY**; fails on current no-op provision, not the scripted leaf. |
| S-ND295-11 `every_incompatible_tap_or_bridge_identity_refuses_owner_publication` | `cargo test -p overdrive-control-plane --lib guest_network::allocation_owner_acceptance::every_incompatible_tap_or_bridge_identity_refuses_owner_publication -- --ignored --exact` | first finite-table case expected an owner-authored `TapObserve` mismatch; current provision returned `Ok(())` | **RED — MISSING_FUNCTIONALITY**; both checkpoints' exact Tap/BridgeLinkIdentity/LinkMaster facts and publication refusal assertions compile. |
| S-ND295-12 `every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement` | `cargo test -p overdrive-control-plane --lib guest_network::allocation_owner_acceptance::every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement -- --ignored --exact` | expected first `EndpointDelete` source while a later TAP-delete failure is retained for continuation; current teardown returned `Ok(())` | **RED — MISSING_FUNCTIONALITY**; the separate eleven-row table covers every cleanup mutation/read and both TAP-observation occurrences with exact operation/source, complete continuation, and retained state; same-owner retry, held lease, complement, and unrelated facts also compile. |
| S-ND295-13 `reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm` | `PROPTEST_CASES=1024 cargo test -p overdrive-sim --lib invariants::netns_density_boot_order::tests::reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm -- --ignored --exact --nocapture` | actual sweep-call snapshot retained seeded run directory/scope; proptest shrank to `seed = 0` | **RED — MISSING_FUNCTIONALITY / reproduced ordering defect**; production helper, same Sim host, real sweep port call, and seed printing all executed. |

The prior bounded gate scored ten non-waived bodies. D14A transitions S00 to
the four exact bodies above: all three source-local filters are invoked
independently and fail on missing production behavior; the ordinary-boot body is compile/list verified and
remains `PENDING_ENVIRONMENT` until Lima root is available. No D14 body fails
in import, collection, or fixture construction, and no waived placeholder is
counted.

## User-waived real-I/O placeholders — not rerun and not scored

The user explicitly waived these eight unconditional real-I/O panic
placeholders from this remediation gate. They remain reasoned-pending and are
neither RED classifications nor approval conditions here:

- `clean_and_receipted_inventory_observes_all_eight_exact_families`
- `retained_unpinned_maps_programs_and_links_survive_handle_release_and_remain_observable`
- `wrong_exact_path_owner_or_valid_map_schema_is_typed_and_never_fabricates_zero`
- `deliberate_link_loss_reaches_default_drop_and_the_exact_production_audit_cause`
- `ordinary_provision_reads_back_the_complete_attachment_before_injected_vmm_start`
- `two_attachment_teardown_releases_last_and_preserves_the_unrelated_attachment_byte_equal`
- `production_boot_trace_completes_vm_reclamation_before_stale_sweep_starts`
- `native_prior_vmm_reclamation_precedes_full_attachment_sweep_and_first_lease_acceptance`

## Compiler and unchanged-suite evidence

D14A verification update:

- Linux compile passed for `overdrive-dataplane --lib --tests` and
  `overdrive-control-plane --lib --tests --features integration-tests`.
- D14A dataplane clippy passed with `-D warnings`; focused control-plane source
  and integration targets pass after allowing only the unrelated production
  worktree's already-reported lint categories.
- The control-plane integration target passes clippy after allowing only the
  independently pre-existing production-file lint categories in the active
  D5/D9 worktree; full-package `-D warnings` remains blocked by those unrelated
  production edits, not by the three D14 bodies.
- The Lima body passes `cargo test --no-run` and exact-name listing. Lima SSH
  remains unavailable, so no real-kernel execution is claimed.
- `.config/nextest.toml` retains the explicit whole-binary
  `package(overdrive-control-plane) & binary(integration)` →
  `host-kernel-shared` override, and the roadmap retains the exact
  `nextest show-config test-groups` command. Host execution of that command was
  disk-limited; no weaker source-level serialization claim replaces it.

- Linux compile passed: `cargo check -p overdrive-dataplane --lib --tests`.
- Linux compile passed: `cargo check -p overdrive-netlink --lib --tests`.
- Linux compile passed: `cargo check -p overdrive-control-plane --lib --tests --features integration-tests`.
- Linux compile passed: `cargo check -p overdrive-sim --all-targets`.
- Linux lint passed: `cargo clippy -p overdrive-dataplane --lib --tests -- -D warnings`.
- Linux lint passed: `cargo clippy -p overdrive-netlink --lib --tests -- -D warnings`.
- Linux lint passed: `cargo clippy -p overdrive-control-plane --lib --tests --features integration-tests -- -D warnings`.
- Linux lint passed: `cargo clippy -p overdrive-sim --lib -- -D warnings`; the broader pre-existing `--all-targets` scope remains blocked by unrelated doc-markdown findings in three spike test files.
- Dataplane guest-TCX unit scope: **4 non-ignored passed, 0 failed**; five
  remediation bodies and one pre-existing body remained ignored.
- Netlink client unit scope: **7 non-ignored passed, 0 failed**; the one
  remediation body remained ignored.
- Control-plane guest-network unit scope: **9 non-ignored passed, 0 failed**;
  the three remediation bodies remained ignored.
- `cargo fmt --all -- --check` and `git diff --check` pass.

No scored failure is classified as BROKEN: collection, imports, exact
D12/D12A/D13 signatures, helper caller fallout, and fixtures compile. The
waived real-kernel and native-metal placeholders are not claimed as substrate
executions.
