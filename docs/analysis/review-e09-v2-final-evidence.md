# E09-v2 final evidence audit

## Metadata

- Expectation: `E09-v2-vm-service-tcp-truthfulness-20`
- Capture: native-metal, 2026-09-08 21:52:57Z through 22:04:27Z
- Seed: `1`
- Product SHA: `b653e1ad1758d11be33be457849b284f78141333`
- Capture status: runner exit `0`; 20 ledger rows, all `pass`/`complete`
- Auditor scope: evidence and the current expectation/anchor contract only
- Producer code, runner implementation, tests, and the native environment were not inspected or executed for this audit.

## Verdict

**REFUTED.** The capture proves substantial lifecycle and cleanup facts, but it does not contain executed evidence for the two peer truthfulness obligations, and it does not expose the required ten-worker cleanup barrier markers. The expectation README remains `pending`; no status change is authorized by this audit.

## Contract used for the audit

The current expectation requires each healthy VM peer Job to receive the exact
`SVM-E08-GUEST-OK` reply and requires each failure peer to observe a denied
backend connection. It also requires each worker to complete the healthy peer
and Service stop plus runtime cleanup before announcing
`healthy-cleanup-complete`, with failure submissions released only after all
ten workers announce that marker (`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/README.md:19-31`).

The same obligations are stated by the accepted S-SVM-25 anchor and the E09-v2
functional handoff (`docs/feature/service-kind-vm-workloads/distill/test-scenarios.md:57-72`, `docs/feature/service-kind-vm-workloads/distill/e09-v2-functional-budget-handoff.md:83-90`). The accepted current restart interpretation permits a same-allocation
`Running` recovery with a positive restart count and a prior `Failed`
observation when the startup probe failed and deploy was nonzero; an incidental
beacon bind error is not required.

## Findings

### E09-V2-EA-01 — exact healthy guest reply is absent (blocking)

The evidence does not prove that any healthy peer received the exact required
guest reply. The 20 ledger rows record only `healthy_peer=Succeeded`
(`evidence/product-run.out:69-89`). The first healthy peer transcript likewise
contains only public Job status, exit `0`, and memory; it contains no guest
stdout, response body, wire result, or exact payload
(`evidence/product-run.out:661-668`).

The captured `product-run.out`, `run.log`, and `tcp-truthfulness-20.tsv` have
zero occurrences of `SVM-E08-GUEST-OK`. A peer Job's terminal `Succeeded`
state is not evidence of the required response body, so the exact healthy
truthfulness claim cannot be inferred from the ledger or status line.

### E09-V2-EA-02 — failure peer denial/no-reply observation is absent (blocking)

The evidence does not prove that a failed Service denied the negative-control
peer's backend connection. Every failure ledger row records only
`failure_peer=Succeeded` alongside `StartupProbeFailed`
(`evidence/product-run.out:69-89`). The first failure peer transcript again
contains only `Verdict: Succeeded`, exit `0`, and memory
(`evidence/product-run.out:728-735`); it has no denial result, backend-connect
failure observation, guest output, or wire capture.

The captured files contain no `connection denied`, `backend connection`,
`negative-control`, or equivalent executed negative-observation marker. The
failure startup-probe and nonzero deploy evidence are real
(`evidence/product-run.out:698-719`), but they do not establish the separate
peer boundary condition. The peer's `Succeeded` status cannot be used to
infer that its helper observed denial.

### E09-V2-EA-03 — ten-worker cleanup barrier is not evidenced (blocking)

The expectation requires an explicit per-worker ordering marker and a cohort
owner release after all ten `healthy-cleanup-complete` announcements. The
capture has no occurrence of `healthy-cleanup-complete` (or a corresponding
failure-submission release marker). Its concurrency section reports aggregate
worker counts and host resource counts only
(`evidence/product-run.out:322-327`). Aggregate `workers=10` plus empty
per-pair `*-resources-after` sections does not prove the required ordering
between each worker's peer/Service stop, cleanup, marker, and release.

## Satisfied evidence claims and accepted non-findings

- Native execution, fail-closed preflight, canonical exclusive lease,
  configured concurrency, and one control-plane PID/start identity are
  captured in `evidence/product-run.out:1-24` and
  `evidence/verification.yaml:1-16`.
- The input manifest has exactly 20 trials with healthy/failure pairs, and the
  ledger has exactly 20 unique complete rows sharing the same control-plane
  PID/start ticks (`evidence/product-run.out:25-89`). No row is marked failed
  or not-run-cancelled in this complete run.
- Healthy Services are shown stable with TCP `18081` as the witness and public
  TCP/HTTP probes passing (`evidence/product-run.out:628-652`). Failure deploys
  are nonzero and publish `StartupProbeFailed` for TCP `18999`
  (`evidence/product-run.out:698-719`). No failure Service stable message was
  found.
- The same-allocation restart interpretation is evidenced by 20
  `allocation recovered from a terminal observation` records with
  `restart_count=1` and `prior_state=failed` (for example,
  `evidence/product-run.out:413-425` and `555-579`), with sampled public
  `Running` state for the same allocation (`evidence/product-run.out:1575-1576`).
  This is accepted by the current anchor contract.
- Per-case runtime snapshots show owned cgroup/run-dir/rootfs-clone entries in
  the active section and empty `*-resources-after` sections (for example,
  `evidence/product-run.out:766-777`); the audit's read-only count across the
  20 case sections found zero after-snapshot entries for both pair members.
  The final snapshot also reports no hypervisors and no E09-owned transient
  resources (`evidence/product-run.out:99471-99707`). Store totals remain
  unchanged for both cohorts (`evidence/product-run.out:317-320`).
- The output's incidental `bind beacon listener: Address already in use`
  text (`evidence/product-run.out:751-765`) was not treated as a requirement or
  finding, per the current contract.
- The recorded 1200-second remote budget plus 60-second cleanup grace was not
  exceeded: the capture ran from 21:52:57Z to 22:04:27Z and exited `0`
  (`evidence/product-run.meta:1-10`).

## Verification performed

This was an evidence-only audit. Read-only checks counted ledger rows, unique
trial IDs, shared PID/start ticks, peer/status fields, restart records, stable
healthy messages, exact payload/negative-observation tokens, cleanup sections,
and cohort summary rows in the captured artifacts. No native rerun, Cargo test,
source or runner inspection, production/API/design edit, issue, commit, or DES
change was performed.

## Remediation disposition

No remediation was performed. The missing peer payload/denial observations and
the missing cleanup-barrier markers are evidence gaps in this capture; this
audit does not prescribe a code or runner change. The expectation README was
left unchanged at `Status: pending`.

## Final review verdict

**REFUTED — keep E09-v2 pending.**
