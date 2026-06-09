# RED Classification — built-in-ca-operator-composition

**Wave**: DISTILL · **Status**: scaffold pending

The Rust scaffolds use the workspace convention (`.claude/rules/testing.md` §
"RED scaffolds and intentionally-failing commits"):

- **Test-side, default lane (Slice ① pure reconciler)**:
  `#[should_panic(expected = "RED scaffold")]` with a `panic!("Not yet
  implemented -- RED scaffold (S-OC-NN / …)")` body. The files compile and are
  wired into Cargo; `cargo nextest run` reports them as PASS (the panic matches
  the expected substring) — RED, not BROKEN. DELIVER replaces the body with real
  assertions slice by slice.
- **Test-side, Lima-gated (Slice ②/③/① integration)**: `#[ignore = "blocked on
  Slice N — …"]` with the same `panic!` body. Per `.claude/rules/testing.md` §
  "What about `#[ignore]`?": `#[ignore]` is correct when the test cannot run on
  the current target — here the runtime surface is `#[cfg(feature =
  "integration-tests")]` and only reachable under Lima (real `overdrive serve`
  subprocess / real keyring / real CA), AND the production wiring does not exist
  yet. DELIVER removes `#[ignore]` per slice.

## Fail-for-right-reason gate (per scenario)

The gate for DELIVER: replace ONE scaffold at a time with a real assertion (or
remove `#[ignore]`), run that scenario, and confirm the failure is **missing
functionality** (`MISSING_FUNCTIONALITY` — the production code does not exist
yet), NOT a setup/import/wiring error (`BROKEN`).

| Scenario | Scaffold shape | DELIVER RED expectation | Confirmable in default lane? |
|---|---|---|---|
| S-OC-01 | `#[should_panic]` | After unskipping, fails because the near-expiry branch still emits `StartWorkflow` (gated), not a `"rotate-svid"` `IssueSvid` — until Slice ① flips it. | YES (Tier-1 DST, default lane) |
| S-OC-02 | `#[should_panic]` | After unskipping, asserts no `IssueSvid` for a far-future held alloc; GREEN once the rotate branch is conditional on the window. | YES |
| S-OC-03 | `#[should_panic]` | After unskipping, the `<=` boundary fixtures fail until the `near_expiry` helper is un-skipped and the threshold is 1800s. This is the LIVE mutation kill-test (`<=`→`<` / `<=`→`==`). | YES |
| S-OC-04 | `#[should_panic]` (fn `rotation_threshold_tracks_half_of_workload_svid_ttl_via_emitted_action`) | After unskipping, drives `reconcile` at `now + WORKLOAD_SVID_TTL/2` (rotates) and `now + WORKLOAD_SVID_TTL/2 + 1s` (no rotate); fails until the threshold is `WORKLOAD_SVID_TTL / 2` (1800s), not the placeholder `28_800`. Asserts ONLY the emitted action list (no private-threshold inspection). | YES |
| S-OC-05 | `#[should_panic]` | After unskipping, fails until the rotate branch (held near-expiry, `"rotate-svid"`) and the restart-recovery branch (unheld ever_issued, `"issue-svid"`) are both live and distinct. | YES |
| S-OC-10 | `#[ignore]` | After removing `#[ignore]`, fails because no `"rotate-svid"`-correlation `IssueSvid` is emitted/dispatched yet (Slice ① not flipped). | NO — **Lima only** (real CA + ObservationStore) |
| S-OC-06 | `#[ignore]` | After removing `#[ignore]`, fails because `run_server` still wires the ephemeral `RcgenCa` (no persistent boot, no on-disk sealed envelope). | NO — **Lima only** (`overdrive serve` subprocess + real keyring) |
| S-OC-07 | `#[ignore]` | After removing `#[ignore]`, fails because the ephemeral path mints a fresh root every boot (no adopt-on-restart). | NO — **Lima only** |
| S-OC-08a | `#[ignore]` (fn `serve_refuses_on_wrong_kek`) | After removing `#[ignore]`, fails because the ephemeral path never reads/decrypts a persisted envelope, so there is no WrongKek refuse-to-start. | NO — **Lima only** |
| S-OC-08b | `#[ignore]` (fn `serve_refuses_on_tampered_envelope`) | After removing `#[ignore]`, fails because the ephemeral path never reads/decrypts a persisted envelope, so there is no TamperedEnvelope refuse-to-start. | NO — **Lima only** |
| S-OC-08c | `#[ignore]` (fn `serve_refuses_on_absent_kek`) | After removing `#[ignore]`, fails because the ephemeral path generates its own throwaway material instead of refusing on an absent KEK. | NO — **Lima only** |
| S-OC-08d | `#[ignore]` (fn `serve_refusal_causes_are_pairwise_distinct`) | After removing `#[ignore]`, fails because the cause-distinct refuse-to-start stderr does not exist yet (ephemeral path never refuses), so the three messages cannot be compared. | NO — **Lima only** |
| S-OC-09 | `#[ignore]` | After removing `#[ignore]`, fails because the ephemeral path has no persisted root to leave unchanged. | NO — **Lima only** |
| S-OC-11 | `#[ignore]` | After removing `#[ignore]`, fails because `AllocStatusResponse.issued_certificates` does not exist and `alloc status` renders no issued-cert section. | NO — **Lima only** (`overdrive alloc status` subprocess) |
| S-OC-12 | `#[ignore]` | After removing `#[ignore]`, fails because there is no latest-by-`issued_at` projection / no-cert-bytes render yet. | NO — **Lima only** |
| S-OC-13 | EXISTING GREEN + DELIVER export hook | NOT a new RED scaffold — `rcgen_full_svid_chain_verifies_root_intermediate_svid` is already GREEN. Slice ③ adds the `OD_E03_CA_DIR` env-gated PEM export (test fixture change; the test stays GREEN). RED-ness here is the E03 runner being `pending` until it captures the exported PEMs. | NO — **Lima only** (`openssl verify`) |
| S-OC-14 | EXISTING GREEN + runner re-check | NOT a new RED scaffold — `rcgen_svid_leaf_carries_exactly_one_uri_san_and_leaf_profile` is already GREEN; the E03 runner re-checks the profile over the exported `svid.pem`. | NO — **Lima only** |
| S-OC-15 | EXISTING GREEN + DELIVER export hook | NOT a new RED scaffold — `rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced` is already GREEN. Slice ③ adds the env-gated further-CA export so the E03 runner can assert `openssl verify` FAILS (sub-claim 3). | NO — **Lima only** |

## This-run verification (compile + RED confirmation)

- `cargo xtask bpf-build` — ran (BPF object prereq for control-plane/core
  nextest-affected per the workspace convention).
- `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance
  --no-run` — GREEN (the new `svid_lifecycle_rotation.rs` compiles + wires).
- `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test
  integration --features integration-tests --no-run` — GREEN (the new
  `built_in_ca_operator_composition/*` integration scaffolds compile + wire).
- `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance
  -E 'test(svid_lifecycle_rotation)'` — **5 passed** (the 5 `#[should_panic]`
  scaffolds report RED via the matched panic — RED, not BROKEN).

**Re-verified after review remediation (2026-06-09)** — HIGH 1 (S-OC-04 reframed
observable + scaffold renamed `rotation_threshold_tracks_half_of_workload_svid_ttl_via_emitted_action`)
and HIGH 2 (S-OC-08 split into `serve_refuses_on_wrong_kek` /
`serve_refuses_on_tampered_envelope` / `serve_refuses_on_absent_kek` /
`serve_refusal_causes_are_pairwise_distinct`):

- `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance
  --no-run` — GREEN (renamed S-OC-04 scaffold compiles + wires).
- `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance
  -E 'test(svid_lifecycle_rotation)'` — **5 passed** (RED preserved; renamed
  S-OC-04 fn still reports RED via the matched panic).
- `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test
  integration --features integration-tests --no-run` — GREEN (the 4 split
  S-OC-08 `#[ignore]` scaffolds compile + wire).

## Lima-only confirmation note

S-OC-06..12 (Slice ②/③ operator-surface) and S-OC-10/13/14/15 can only be
confirmed `MISSING_FUNCTIONALITY` (not `BROKEN`) under Lima — their runtime
surface (`overdrive serve` / `overdrive alloc status` subprocess, real keyring,
real CA, `openssl verify`) is `#[cfg(feature = "integration-tests")]` and
unreachable on the macOS host. The `--no-run` Lima compile-check above proves the
scaffolds compile + wire; the runtime RED confirmation is a DELIVER-phase Lima
run as each `#[ignore]` is removed.

## DELIVER notes

- Replace `#[should_panic]` bodies / remove `#[ignore]` ONE scaffold at a time;
  confirm fail-for-right-reason before writing production code.
- **Slice ① single-cut** (CLAUDE.md "Delete unused code AND its tests"): in the
  same commit, delete `ROTATION_ENABLED`, `CERT_ROTATION_WORKFLOW`, the
  `StartWorkflow`/`WorkflowName` imports, the `#[mutants::skip]` on `near_expiry`,
  the `.cargo/mutants.toml` `"near_expiry"` `exclude_re` entry, AND the existing
  GREEN gated-seam test
  `near_expiry_rotation_seam_is_emit_gated_until_cert_rotation_registered` (it
  asserts the now-deleted gated-NO-emit behaviour).
- **CLAUDE.md "never invent API surface"**: the rotate path reuses
  `Action::IssueSvid` UNCHANGED. If a gap surfaces, STOP and surface it.
