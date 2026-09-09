# E09-v2 capture-objection audit for step 02-04

## Metadata

- **Scope:** bounded re-review of `docs/analysis/review-e09-v2-final-evidence.md`
  findings EA-01, EA-02, and EA-03.
- **Reviewer:** Codex, GPT-5.6 Luna, maximum thinking.
- **Capture under review:** `E09-v2-vm-service-tcp-truthfulness-20`, native-metal,
  seed `1`, captured 2026-09-08 21:52:57Z–22:04:27Z.
- **Pinned identity:** product SHA `b653e1ad1758d11be33be457849b284f78141333`,
  `working_tree_dirty: true`, with the captured dirty patch retained in
  `verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/dirty-diff.patch`.
- **Authority boundary:** read-only evidence and fixture audit only. No source,
  runner, expectation, API, design, DES, native, test, or commit change was
  made. The expectation README status was not changed.

## Contract and evidence boundary

S-SVM-25 requires twenty checked-in healthy/failure pairs, one build and
preparation, two ten-pair cohorts through one control-plane process, exact
healthy guest replies, no failed peer reply, `StartupProbeFailed`, no retry or
discard, and runtime reclamation (`docs/feature/service-kind-vm-workloads/distill/test-scenarios.md:57-72`).
Its detailed per-pair oracle says that the healthy Job result depends on the
exact `SVM-E08-GUEST-OK` response and that the failure peer completes its
negative observation window without receiving that response
(`docs/feature/service-kind-vm-workloads/distill/test-scenarios.md:74-91`).

The catalogue evidence rule accepts either a saved command output, an exact
test output, or a quoted `path:line` pointer from the pinned tree
(`verification/README.md:42-60`). It does not require private fixture files or
temporary barrier filenames to be emitted on the operator surface. The
expectation README defines the saved native surfaces as ledger, public
observations, timing, cohort concurrency, per-case transcript, and cleanup
measurements; the host-safe scheduler tests separately exercise the barrier
mechanics.

## Mechanical capture facts

The captured manifest records 20 trials, each with healthy and failure specs,
and the ledger has 20 rows. A read-only parse of the pinned ledger found:

- all 20 rows are `pass`/`complete`;
- every healthy peer is `Succeeded`, with healthy deploy result `0`;
- every failure peer is `Succeeded`, with failure deploy result `1` and
  `StartupProbeFailed` on startup TCP port `18999`;
- every row has `Terminated` terminal state and `zero-runtime` cleanup; and
- all successful rows share one control-plane PID/start-tick pair.

These values are visible in the saved ledger
(`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/product-run.out:69-89`).
The same output records the checked-in source validation and one reusable
preparation (`:18-24`), ten-worker healthy and failure cohort observations
(`:322-327`), empty per-case `healthy-resources-after` and
`failure-resources-after` sections, and the final native success line
(`:99471`). The manifest pins native-metal execution, seed, SHA, dirty state,
and harness invocation (`evidence/verification.yaml:1-16`); the 1200-second
remote budget, 60-second cleanup grace, and exit 0 are pinned in
`evidence/product-run.meta:1-9`.

## Finding dispositions

### EA-01 — exact healthy guest reply absent

**Disposition: REJECTED as a blocking evidence finding.**

The captured public Job observation is `Verdict: Succeeded` with
`Attempt ... Terminated 0` for the healthy peer
(`product-run.out:661-668`). That public verdict has the required causal
meaning in the pinned product: `derive_job_verdict` maps a terminated row with
exit code zero to `Succeeded` (`git show b653e1ad1758d11be33be457849b284f78141333:crates/overdrive-cli/src/render.rs`, lines 977-1003),
and the control-plane projection supplies the exit code only from the typed
completed terminal condition (`git show b653e1ad1758d11be33be457849b284f78141333:crates/overdrive-control-plane/src/handlers.rs`, lines 118-131).

The pinned checked-in healthy client is the fixture oracle. Its specification
passes `expect-reply` (`git show b653e1ad1758d11be33be457849b284f78141333:examples/service-kind-vm-workloads/client-healthy.toml`, lines 1-12).
The compiled client defines `BODY` as `SVM-E08-GUEST-OK` and returns zero from
that branch only after `receives_reply` finds that exact byte sequence
(`git show b653e1ad1758d11be33be457849b284f78141333:examples/service-kind-vm-workloads/client.rs`, lines 7-9 and 55-65).
The preparer validates the checked-in sources and compiles that client into the
guest rootfs (`git show b653e1ad1758d11be33be457849b284f78141333:examples/service-kind-vm-workloads-v2/prepare.sh`, lines 86-102 and 242-264),
and the capture records those source checks and preparation
(`product-run.out:18-22`).

The lack of a literal `SVM-E08-GUEST-OK` token in the host transcript is
therefore not a missing executed oracle. The saved public result plus the
pinned fixture contract proves the exact-reply obligation without requiring
guest stdout to be forwarded into the operator transcript. No capture fix is
necessary for EA-01.

### EA-02 — failure-peer denial/no-reply observation absent

**Disposition: REJECTED as a blocking evidence finding, with a terminology
boundary recorded.**

The failure peer also reports `Verdict: Succeeded` and `Terminated 0`
(`product-run.out:728-735`). Its pinned specification selects
`expect-unreachable` (`git show b653e1ad1758d11be33be457849b284f78141333:examples/service-kind-vm-workloads/client-tcp-failure.toml`, lines 1-12).
The same checked-in client exits nonzero if the exact guest body is ever seen
inside the observation window and returns zero only after the window completes
without that body (`git show b653e1ad1758d11be33be457849b284f78141333:examples/service-kind-vm-workloads/client.rs`, lines 67-75).
The public result therefore proves the accepted S-SVM-25 negative oracle:
the failed peer did not receive `SVM-E08-GUEST-OK` during its observation
window. The failure Service's nonzero deploy and typed port-18999 startup
failure are independently present in the ledger and transcript
(`product-run.out:698-719`).

The fixture does not claim a packet-level TCP `ECONNREFUSED` result. If
“deny the backend connection” in the expectation README is read as that
stronger wire-level claim, it is stronger than the detailed S-SVM-25 oracle
(`:83-85`) and is not established by this fixture. The bounded audit does not
promote that wording mismatch into a new production or capture requirement;
the precise supported claim is absence of the exact guest reply. No capture
fix is necessary for the accepted negative-reply obligation.

### EA-03 — ten-worker cleanup barrier not externally captured

**Disposition: REJECTED as an over-specific external-capture requirement.**

The objection correctly observes that the native transcript does not print
temporary filenames such as `healthy-cleanup-complete`. Those filenames are
private orchestration state, however, and the evidence policy does not require
all internal transitions to be copied onto the operator surface. The captured
dirty patch is part of the pinned verification identity and shows the actual
run's worker/coordinator ordering:

- each worker records successful healthy resource release, creates
  `healthy-cleanup-complete`, and waits before entering failure deploy
  (`evidence/dirty-diff.patch:342-351`);
- the cohort owner waits for all `healthy-cleanup-complete` markers and only
  then creates `failure-submit-release`
  (`evidence/dirty-diff.patch:362-377`); and
- the host-safe scheduler tests exercise the same ten-marker join and its
  cancellation complement (`examples/service-kind-vm-workloads-v2/test-scheduler.sh:252-320`).

The native command reached the example's success line only after those
production-example gates returned successfully
(`product-run.out:99471`). The native transcript also preserves the public
cohort counts and post-cleanup resource sections (`product-run.out:322-327`
and the per-case transcript sections). Requiring raw private marker records in
the native evidence would add an internal-transition requirement beyond the
accepted operator/evidence boundary. No capture fix is necessary for this
finding; a future diagnostic may retain such markers, but it is optional and
outside this bounded review.

## Verification and limits

Read-only checks verified the pinned manifest fields, 20-row ledger cardinality
and predicates, source/spec identity at the captured SHA, dirty-patch barrier
presence, public Job/exit rendering path, and `git diff --check`. No native
rerun, Cargo test, source edit, expectation status change, DES event, or commit
was performed.

This audit only disposes of the three objections in the prior evidence review.
It does not independently authorize changing the expectation README from
`pending` or claim that unrelated E09/E10/E13 evidence obligations are
satisfied.

## Verdict

The three findings do not justify the prior **REFUTED** verdict: EA-01 and
EA-02 demand a literal guest/wire transcript despite the pinned public Job
fixture oracle, and EA-03 demands private barrier filenames despite the pinned
dirty source, successful gated run, and separate host-safe barrier checks.
**Audit verdict: objections rejected within this bounded scope; no capture fix
is required.**
