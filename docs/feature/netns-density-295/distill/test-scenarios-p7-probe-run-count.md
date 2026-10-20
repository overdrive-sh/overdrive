# P7 — Probe-run counting and ProbeResultRow V2 (netns-density-295, PROBE-RUN-COUNT)

Owners: `overdrive-core` (`ProbeRunSeq`, `SameStatusRuns`,
`ProbeResultRowV2`, envelope `V2`), `LocalObservationStore` and
`SimObservationStore` (`write_probe_result` merge on `run_seq`),
`ProbeRunner` (`overdrive-worker`, VM probes through `GuestStreamDial`),
`ServiceLifecycle` (`overdrive-reconcilers`, read-time threshold with the
act-once `liveness_last_acted_run`), driven through `run_convergence_tick`
and the action shim. Marker: `pending DELIVER step 9`.

## Schema and newtypes (`overdrive-core`, default lane)

| Test | Contract | Oracle | Pending |
|---|---|---|---|
| `schema_evolution::probe_result_row::probe_result_row_v1_decodes_and_up_converts_with_first_run` | R5-PROBE-V2: explicit `Envelope::V1` fixture up-converts with run FIRST, streak ONE | `matches!(V1)`; hand-written V2 | step 9 |
| `…v2_decodes_through_current_envelope`, `fixtures_match_current_encoding`, `latest_wraps_v2`, `discriminant_offset_triangulation` (offset 72, tags 0/1), `unknown_version_probe_surfaces` (supported max 1) | V2 codec | golden bytes | **active** |
| `acceptance::probe_run_seq::*` newtype round trips, malformed / zero refusals, successor, saturation; `v1_row_up_converts_with_first_run_and_streak_one` (proptest) | newtype completeness | round trip; errors | step 9 |
| `acceptance::probe_run_seq::v2_row_round_trips_through_the_envelope_codec`, `…through_serde` | V2 codec (proptest) | round trip | **active** |
| `acceptance::probe_result_row_envelope::*` | latest (V2) round trip; latest discriminant 1; unknown tag → `UnknownVersion { supported_max: 1 }` | rkyv | **active** |

## Merge (R5-PROBE-MERGE)

| Test | Contract | Fault | Oracle | Pending |
|---|---|---|---|---|
| `overdrive-store-local` `acceptance::probe_result_merge::probe_row_merges_by_run_sequence_identically_in_both_stores` | merge on `run_seq` in both stores | greater / equal / smaller run, greater run with earlier `observed_at`, sim injected failed write | identical stored row per step; `Row` event only for accepted writes | step 9 |
| `…each_key_keeps_its_own_greatest_run_in_both_stores` | role / probe-idx key separation | — | per-key greatest | step 9 |
| `observation_backend` source-local `failed_probe_write_changes_nothing_and_the_same_run_is_accepted_after` | `Err` postcondition | `ProbeResultWriteFault::BeforeCommit` | unchanged row; no event; retry accepted | step 9 |
| `…undecodable_stored_probe_row_is_replaced_by_any_write` | ADR-0048 §3 | corrupt stored row | replaced | **active** |

## ServiceLifecycle view (`overdrive-reconcilers` acceptance)

`probe_run_count::persisted_view_with_streak_fields_decodes_without_them`
(step 9) and `liveness_last_acted_run_survives_persistence` (**active**).

## Producer and composed behaviour

| Test | Contract | Stimulus | Oracle | Pending |
|---|---|---|---|---|
| worker `probe_runner::run_numbering_tests::every_written_run_counts_once_and_missing_samples_count_nothing` | run numbering and streak (layer-2 proptest) | 1–8 runs of Pass/Fail/Capacity/Quiesced/WriteFail | stored `run_seq` = written count; `same_status_runs` trailing; no-row runs emit one `probe.not_sampled`; one `DialPurpose::Probe` dial per run | step 9 |
| worker `…tcp_vm_probe_is_healthy_iff_the_dial_succeeds` | TCP VM probe | each `GuestDialError` | `Fail` with `as_str` detail; `Ok` → `Pass` | step 9 |
| control-plane `reconciler_runtime::probe_run_count` (R5-PROBE-COUNT baseline + cases a–h, new streak, readiness, per-kind) | threshold over the producer's rows; act once per crossed run; retry stop per newer crossed row | coalesced runs; HTTP path not a request target; failed row write; Capacity / Quiesced; backward clock step; resubmit; status flip; failed stop | rows, stop calls, `liveness_last_acted_run`, terminal `Stopped { LivenessProbe }`, published backend health | step 9 |
| `quint_probe_count` driver | `probe_count.qnt` — `probe_streak`: 9 `#[quint_test]` + `#[quint_run]` | ProbeRun → `SimGuestStreamDial` + `SimClock::tick` / injected write failure; ConsumerLook → scripted `SimDriver` + `run_convergence_tick`; ClockBack → `step_wall_clock_back` | `rseq` / `rst` / `rsame` / `running` from `SimObservationStore` (marked arbiter, paired with R5-PROBE-MERGE); `A` from `loaded_service_lifecycle_views_for_test` | step 9 |

The randomized `probe_count_conformance_probe_streak` replays traces and
reaches the RED scaffolds (verified on Lima); the named runs hit UI-V-QT1.

## Migrations

Every `ProbeResultRow` literal outside P7 sets `run_seq` (following the
observation time, so the old observed-at assertions keep their meaning
under the new merge) and `same_status_runs: ONE`. `ProbeRunner::new` takes
the required `guest_dial: Arc<dyn GuestStreamDial>` last; `run_server`
passes the guest-flow owner's dial (DG-2 row 19).

## Blockers

UI-V-FIXTURE-BYTES (hand-derived V1/V2 golden bytes — the GREEN schema
tests confirm them on Lima, so this item is closed), UI-V-STREAK-FIELDS
(removal of the View streak fields deferred to step 9 with the stale
bodies), UI-V-RESUBMIT-ORDER (case f needs the VM start path of the
attachment cut), UI-V-PROBERUNNER-ARG (`guest_dial` position), UI-V-QT1.
