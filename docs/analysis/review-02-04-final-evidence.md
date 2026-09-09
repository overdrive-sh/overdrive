# Step 02-04 final evidence audit

## Metadata

- **Scope:** independent final different-fox audit of the current E09-v2,
  E10, and E13 captures.
- **Auditor:** Codex, GPT-5.6 Luna, maximum thinking.
- **Pinned product SHA:** `9f6702e431c1a68a3374445368ea8fc525f23bc4`.
- **Closure provenance:** evidence/docs commit `8f5d0deef82f29264a5af0a076a9c99fee746f6b`; DES commit-phase record `f5c9a7830a46eaed165a8f89b0f07bc21b3fd847`.
- **Pinned seed:** `1` for all three expectation manifests.
- **Substrate:** `native-metal` for all three captures; each manifest records
  `executed_in_lima: false` because these are native-metal runs.
- **Scope boundary:** accepted anchors, checked-in fixture contracts and their
  pinned source quotes, expectation READMEs, manifests, metadata, ledgers, and
  captured product output. The implementation and runner that produced the
  evidence were not inspected, executed, or changed.

## Audit authority and disposition of prior objections

The operational evidence policy requires a pinned SHA, dirty state, seed, date,
harness invocation, declared substrate, and executed output before an
expectation can be `satisfied` (`verification/README.md:42-70`). The accepted
anchors are S-SVM-25, S-SVM-26, and S-SVM-29 in
`docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.

The bounded disposition in
`docs/analysis/review-02-04-capture-findings.md` is part of the current audit
record. It rejects the former E09 EA-01 and EA-02 objections because the
checked-in client fixture is the independent exact-reply/negative-reply oracle,
and rejects EA-03 because private cleanup marker filenames are not an
operator-surface evidence requirement. The accepted negative oracle means that
the failed peer completes its observation window without receiving the exact
guest reply; it does not claim a packet-level `ECONNREFUSED` result. This audit
does not reinstate those rejected requirements.

## Current capture manifests

| Expectation | Capture date and duration | Harness/product identity | Exit and result |
|---|---|---|---|
| E09-v2 | `2026-09-08T23:34:23Z`–`23:46:05Z` | product SHA `9f6702e`; harness SHA `f8dad8b`; seed `1` | exit `0`, `succeeded` |
| E10 | `2026-09-08T23:53:37Z`–`23:55:50Z` | product SHA `9f6702e`; harness SHA `f8dad8b`; seed `1` | exit `0`, `succeeded` |
| E13 | `2026-09-08T23:55:54Z`–`23:58:15Z` | product SHA `9f6702e`; harness SHA `f8dad8b`; seed `1` | exit `0`, `succeeded` |

The complete manifest fields are retained in the three `evidence/verification.yaml`
files. The exact commands, native-metal substrate, timeout metadata, and exit
codes are retained in each `evidence/product-run.meta`:

- E09-v2: `verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/verification.yaml:1-16` and `product-run.meta:1-9`.
- E10: `verification/expectations/E10-vm-service-http-cross-driver-status/evidence/verification.yaml:1-16` and `product-run.meta:1-5`.
- E13: `verification/expectations/E13-vm-service-inferred-tcp-startup/evidence/verification.yaml:1-16` and `product-run.meta:1-5`.

All three product transcripts show the canonical metal lease acquisition and
fail-closed x86_64/KVM preflight before the ledger. E09-v2 records those
surfaces at `evidence/product-run.out:1-29`; E10 records them at
`evidence/product-run.out:1-20`; E13 records them at
`evidence/product-run.out:1-20`.

## E09-v2 audit — 20 paired TCP journeys

### Contract and cardinality

The manifest has exactly 20 trial inputs, each with one healthy and one failure
spec (`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/product-run.out:30-72`). The ledger has exactly one
`pass`/`complete` row for every trial 1 through 20, with no duplicate trial,
shared control-plane PID `3737452`, and shared start ticks `219628685`
(`product-run.out:73-95`). A read-only ledger parse confirmed 20 rows, trials
1–20 once, and one control-plane identity.

The native transcript records the checked-in source validation, one product
build, one reusable artifact preparation, one control-plane metadata record, and
configured concurrency 10 (`product-run.out:18-29`). The two cohort summaries
report ten workers for each healthy and failure phase
(`product-run.out:327-332`). The final product oracle reports 20/20 pairs and
explicitly records no retries, replacements, or discarded pairs
(`product-run.out:101163-101165`). This satisfies the current bounded contract;
the earlier E10 setup refusal is unrelated to this E09-v2 run.

### Healthy and failure truthfulness

Across the ledger, every healthy row has deploy result `0`, a passing startup
TCP target on port `18081`, peer `Succeeded`, and `zero-runtime` cleanup; every
failure row has deploy result `1`, typed `StartupProbeFailed` on TCP port
`18999`, peer `Succeeded`, and `zero-runtime` cleanup
(`product-run.out:74-94`). The first complete pair's public transcript shows
the healthy Service Stable with the real TCP `18081` witness, passing TCP and
HTTP observations, and a terminated successful peer Job
(`product-run.out:630-670`). The corresponding failure transcript shows a
nonzero deploy, a failed `18999` startup probe, no Stable result, and a
successful negative peer Job (`product-run.out:700-737`). The same predicates
are present in all 20 ledger rows; a read-only count found 20 healthy stable
witnesses, 20 restart-recovery records, and no failure stable witness.

The exact-reply and negative-reply meanings are pinned by the checked-in
fixture at the capture SHA. `examples/service-kind-vm-workloads/client.rs:7`
defines `SVM-E08-GUEST-OK`; its `expect-reply` branch returns zero only after
`receives_reply` finds that exact body (`:55-64`), while its
`expect-unreachable` branch exits nonzero if that body is ever received and
returns zero only after the observation window completes without it (`:67-75`).
The healthy and failure client specs select those two modes respectively
(`examples/service-kind-vm-workloads/client-healthy.toml:4-6`,
`client-tcp-failure.toml:4-6`). The public `Succeeded` results therefore have
the accepted fixture-oracle meaning. A host transcript need not contain the
guest body's literal bytes, and this audit does not require a stronger wire
error such as `ECONNREFUSED`.

### Restart interpretation and cleanup

The accepted current interpretation permits the same allocation to recover to
`Running` with a positive restart count and a prior `Failed` observation when
the typed startup failure and nonzero deploy result are present. The transcript
records 20 `allocation recovered from a terminal observation` events with
`restart_count=1` and `prior_state=failed` (for example,
`product-run.out:413-425` and `555-602`), and the public observations show the
same allocation running with restart count 1 (for example,
`product-run.out:1455-1457`). The incidental beacon bind error in the failed
trajectory (`product-run.out:752-767`) is not used as a required oracle.

All 20 healthy and 20 failure case sections contain active owned runtime
entries followed by empty `healthy-resources-after` and
`failure-resources-after` sections; a read-only count found 20 of each section
and zero nonempty after-snapshot entries. The first case is representative at
`product-run.out:768-779`. Cohort store totals remain unchanged
(`product-run.out:322-326`). The final snapshot reports no hypervisors, no
E09-owned run directories, empty BPF/nft transient sections, empty loops,
mounts, and clones, and the owned materialization is removed
(`product-run.out:101165-101401`). Existing unrelated cgroup scopes and shared
network rules remain visible as retained infrastructure, as expected.

One non-fatal `/proc/<pid>/stat` sampling race is preserved at
`product-run.out:27`. It does not alter the complete ledger identity, native
preflight, final PASS, or exit-zero evidence, so it is recorded as a diagnostic
note rather than a contract failure.

**E09-v2 verdict: SATISFIED.** The exact 20-pair/two-cohort/one-control-plane
contract, typed startup outcomes and fixture peer oracles, accepted restart
trajectory, scoped cleanup, native substrate, and pinned execution evidence are
complete.

## E10 audit — cross-driver HTTP status matrix

The accepted S-SVM-26 contract requires the same checked-in application across
Exec and VM forms, 204 success, rejection of 302/404/503 without following a
redirect, equal status policy, bounded 503 body handling, and state-specific
cleanup (`docs/feature/service-kind-vm-workloads/distill/test-scenarios.md:109-122`; expectation README:10-29).

The successful current ledger has exactly the eight required cells: Exec/VM ×
204/302/404/503. Both 204 rows are `Terminated` through
`operator-stop-running`; Exec failure rows use the accepted
`replacement-operator-stop` trajectory; VM failure rows use the accepted
`replacement-start-rejected` trajectory and remain `Failed`. Every row has the
expected numeric `status-204`, `status-302`, `status-404`, or `status-503`
probe result and zero-delta cleanup
(`verification/expectations/E10-vm-service-http-cross-driver-status/evidence/http-status-cross-driver.tsv:1-9`). A read-only comparison found
byte-equal status policy for each Exec/VM status pair.

The pinned checked-in guest fixture defines the 503-only nonempty
`SVM-E10-FAILURE-BODY-MUST-NOT-LEAK` body and emits the selected HTTP status
without a redirect target (`examples/service-kind-vm-workloads/guest_server.rs:8-10`,
`:83-103`). The eight ledger rows record zero sentinel occurrences in deploy
stdout, deploy stderr, describe output, and rendered probe output; the same
zero counts are present in the captured product output
(`evidence/product-run.out:20-31`). This satisfies the deletion-sensitive
operator-surface policy without requiring the body to appear in the transcript.

The first E10 attempt is retained separately at
`evidence/first-attempt-2026-09-08T234614Z/` with exit 1 and
`execution_status: failed` because setup refused to overwrite an existing
materialization (`first-attempt-.../product-run.meta:1-5`,
`first-attempt-.../verification.yaml:1-16`). It stopped before the matrix and
is not counted as a favorable trial or a discarded matrix cell. The successful
root evidence is a distinct rerun at `23:53:38Z`–`23:55:50Z`, exit 0
(`evidence/product-run.meta:1-5`).

**E10 verdict: SATISFIED.** All eight matrix cells, status policy, zero
sentinel exposure, accepted state-specific trajectories, cleanup, native
preflight/lease, and pinned successful rerun evidence are complete.

## E13 audit — inferred guest-targeted TCP startup

The accepted S-SVM-29 anchor requires two no-health-check VM Services, one
bound and one unbound listener, inferred guest-targeted TCP startup outcomes,
complementary VM peer Jobs, and exact-reply/negative-reply behavior
(`docs/feature/service-kind-vm-workloads/distill/test-scenarios.md:175-190`; expectation README:10-15,23-32).

The checked-in fixture specs contain listeners on `18081` and `18998` with no
health-check table (`examples/service-kind-vm-workloads/zero-probes.toml:1-21`,
`zero-probes-failure.toml:1-20`). Their VM clients select `expect-reply` and
`expect-unreachable` respectively (`client-zero-probes.toml:1-8`,
`client-zero-probes-failure.toml:1-8`). The same pinned client contract defines
the exact guest body and the negative observation window described above.

The current two-row ledger records healthy deploy exit 0, `Stable`, inferred
`true`, target `guest`, peer `exact-reply`, healthy eligibility, and
`zero-delta` cleanup; the failure row records nonzero deploy,
`StartupProbeFailed`, inferred `true`, target `guest`, peer
`unreachable-verified`, unhealthy eligibility, and `zero-delta` cleanup
(`verification/expectations/E13-vm-service-inferred-tcp-startup/evidence/zero-probes.tsv:1-3`). The native transcript records canonical lease, preflight, the
two-row ledger, and the successful complementary-peer PASS
(`evidence/product-run.out:1-25`).

**E13 verdict: SATISFIED.** The inferred guest-target contract, complementary
peer results, typed failure, eligibility, cleanup, native substrate, and pinned
execution evidence are complete.

## Verification performed

Read-only checks parsed the current E09-v2, E10, and E13 ledgers; verified
cardinality, unique trial/cell identities, shared E09 control-plane identity,
status predicates, peer oracle fields, restart records, resource-after section
counts, sentinel counts, cleanup deltas, manifest SHA/seed/substrate fields,
native lease/preflight output, and exit codes. The checked-in fixture source
quotes were read at the manifest SHA. No native rerun, Cargo test, heavy test,
runner or producer implementation inspection, source edit, API/design edit,
DES event, issue, or commit was performed.

## Remediation disposition

No remediation was necessary. The prior E09 EA-01/EA-02/EA-03 objections are
superseded by the accepted fixture-oracle and evidence-boundary disposition in
`docs/analysis/review-02-04-capture-findings.md`. The three current expectation
READMEs and their INDEX rows are updated to `satisfied` with this audit linked;
the failed E10 setup attempt remains preserved as historical evidence.

## Final verdict

**SATISFIED — E09-v2, E10, and E13 current native captures pass this
independent final evidence audit.**
