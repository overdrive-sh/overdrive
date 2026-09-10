# Independent RCA review — VM lifecycle latency, issue #283

## Metadata

| Field | Value |
| --- | --- |
| Review ID | `rca_rev_20260909_issue283` |
| Reviewer | `nw-troubleshooter-reviewer` (Codex) |
| Model instruction | User-selected GPT 5.6 Luna, maximum thinking |
| Reviewed artifact | `docs/analysis/root-cause-analysis-vm-lifecycle-latency-283.md` |
| RCA baseline | `d880ce983276e7d4107450b264f6ceab64c70eff` |
| Review scope | Causality and evidence for #283 only; no production/API/design change |
| Review date | 2026-09-09 |

## Executive verdict

The RCA's substantive diagnosis is supported: the current convergence owner
serially awaits a broker batch and each action, the VM stop path has an
unconditional two-second request window, and a live long-running guest child
prevents the sequential guest shutdown read so the VMM reaches its ten-second
grace and kill. The seeded control-plane witness, native stage partition,
finite-job control, source path, and explicit historical-witness correction
form a coherent bounded diagnosis.

Iteration 1 requested two narrowly scoped evidence-honesty revisions. Iteration
2 confirms both are closed: the saved observation patch now passes an ordinary
`git apply --check`, and the RCA confines the SHUTDOWN observation to
application-level `write_all`/`flush` returns while explicitly disclaiming wire
delivery, peer consumption, and a guest-read syscall. The final verdict is
`APPROVED`. Neither finding authorizes a production fix, new API, timeout,
concurrency mechanism, or design change.

## Materials and independent checks

I read the complete RCA and its diagnostic sources, the mandatory project
instructions, the troubleshooter-reviewer role and review criteria, and fetched
both GitHub issues with comments:

```text
gh issue view 283 --comments --json number,title,body,comments
gh issue view 260 --comments --json number,title,body,comments
```

Both returned `comments: []`. Issue #283 explicitly rejects serial VM
lifecycle execution and unexplained seconds of latency as acceptable, requires
separate lifecycle stages and a seeded production-owner witness, and forbids
timeout-only resolution. Issue #260 records the related serial dispatch and
unused `TickContext.deadline` as deferred design work. The RCA preserves that
scope and does not treat historical deferral as approval.

The following read-only checks were run during iteration 1 and are retained
as historical review evidence; the applicability result was re-run in
iteration 2 below:

| Check | Result |
| --- | --- |
| `python3 docs/analysis/summarize-vm-lifecycle-283.py` against first traced, second traced, and untraced healthy logs | Passed; independently reproduced the RCA's stage durations and line mappings |
| `git apply --check docs/analysis/native-observation-283.patch` | Failed, `corrupt patch at line 132` (the finding below) |
| `git apply --check --recount docs/analysis/native-observation-283.patch` | Passed for all four production files; confirms the issue is hunk-count metadata, not unmatched context |
| `git diff --check` | Passed |
| `bash -n` on the three diagnostic shell scripts | Passed |
| Search for `issue283` in production source | No temporary observation markers remain |
| `git diff` of control-plane/host/worker production source | Empty; only the pre-existing unrelated `AGENTS.md` change, diagnostic test/dependencies, and RCA artifacts are dirty |

No broad suite or mutation run was performed. The retained intentionally red
seeded replay is the evidence under review, not a request to make the
diagnostic green.

## Production-path and test-boundary verification

The current source supports the RCA's owner path and reachability claims:

1. `crates/overdrive-control-plane/src/lib.rs:3320-3393` drains the broker,
   then awaits each `run_convergence_tick` in `for eval in pending`; the
   cancellation select is only reached after the batch completes.
2. `crates/overdrive-control-plane/src/action_shim/mod.rs:900-950` serially
   awaits each action. Its StartAllocation arm awaits `driver.start` at
   `:1931`, and its StopAllocation arm awaits `driver.stop` at `:2825`.
3. `crates/overdrive-worker/src/vm_driver.rs:1651-1757` moves the Live claim to
   EndingInFlight, writes shutdown through the beacon, awaits the full
   `VM_SHUTDOWN_REQUEST_DEADLINE`, then awaits VMM termination and cleanup.
4. `crates/overdrive-host/src/vmm.rs:534-574` returns early only for an
   already-observed exit; otherwise it waits for the reaper or the supplied
   grace timer and then kills and reaps.
5. `crates/overdrive-init/src/main.rs:164-206` orders execute, EXIT, shutdown
   read, and poweroff; `:1029-1043` uses blocking `Command::status()`. This is
   the source-supported explanation for why a still-running service child has
   not reached the shutdown read. It is not presented as a traced guest
   syscall.

The new diagnostic test at
`crates/overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs:141-221` uses
seed `283001`, real HTTPS handlers, real intent/view stores, the production
broker and convergence task, and the real action-shim path. `DelayedDriver`
holds an existing `Driver` port operation behind an external barrier; it does
not write observation rows, choose broker evaluations, or add a production
seam. The `SimDataplane` is the existing driven-port override for this
control-plane subject, not a claim to model Cloud Hypervisor. All three live
tests have `/// CONTRACT_SHAPE: bounded-change.` declarations, and the
`reqwest`, `base64`, and `toml` dev-dependencies are existing workspace edges
needed to exercise HTTPS and read the generated trust material.

The final replay is honest about its result: healthy control passed; slow
start and slow stop intentionally failed while-held progress and both passed
the after-release recovery assertion. The two failing tests are not
reclassified as production fixes or acceptance tests, and no test binary
spawns the production Overdrive executable.

## Evidence and causal assessment

### Shared convergence owner (Branch A)

This branch is reproducibly causal at the control-plane boundary. The test
waits until the real owner enters the held Start or Stop effect, submits the
independent workload afterward, advances logical time ten times while
`GET /v1/cluster/info` remains successful, and observes no independent
`Running` row. Releasing the same effect makes the independent workload
progress; the no-delay control progresses without the hold. This establishes
the complete caller/owner path without fabricating consequence state. The RCA
also correctly distinguishes this driven-port invariant from native VMM timing
and explicitly corrects the now-passing historical seed 257210 rather than
using it as current proof.

The stop decorator calls the Sim driver's stop side effect before waiting, but
the pending future still represents the production completion boundary:
`VmDriver::stop` changes lifecycle ownership before it awaits the shutdown,
VMM, and cleanup phases. The RCA describes this distinction rather than
claiming that the Sim adapter models native teardown.

### Two-second request window (Branch B)

The three native healthy logs independently reproduce the partition. The
untraced log maps stop entry at `19:56:56.046305Z` through the retained
marker calculation to these exact durations: stop entry to writer completion
`0.119 ms`, stop entry to terminate entry `2,002.164 ms`, terminate entry to
cleanup entry `10,017.545 ms`, cleanup `0.630 ms`, total Driver stop
`12,020.339 ms`. The first and second traced controls show the same two-second
and ten-second windows. Source lines `1715-1735` explain why even a completed
writer still awaits the already-started deadline. The arithmetic is internally
consistent; the writer qualification is addressed in Finding F-02.

### Ten-second VMM grace and guest ordering (Branch C)

The first traced process file records `kill(4075088, SIGKILL) = 0` followed by
`wait4(4075088, ... SIGKILL ...)`; the corresponding log records VMM reap with
signal 9 after the grace. The repeated traced and untraced logs agree. The
finite checked-in Job controls report VMM exit code 0 and reap in
`112.179 ms`, `113.641 ms`, and `30.445 ms` after EXEC release, while the
long-lived Service has no EXIT/poweroff console evidence. Together with the
byte-identical `/init`, `/sbin/init`, and fresh current-source init hashes,
this rules out an intrinsic ten-second VMM or cleanup floor.

The RCA properly labels the guest-read conclusion as source-supported
inference. Console evidence and the sequential `overdrive-init` source support
the mechanism, but the host process trace does not observe a guest kernel
syscall; the report does not silently promote that inference to one.

### Alternatives and limits

The investigation pursues and either eliminates or bounds the relevant
alternatives: transport/writer delay versus the fixed request window; early
VMM exit versus actual SIGKILL/reap; native cleanup leak versus sub-millisecond
cleanup and final absence checks; ptrace perturbation versus untraced stop
repetition; and the old seed-257210 timeout versus its changed 120-second cap.
It also covers cancellation, retry, and server-shutdown ownership and does not
invent a crash or cancellation defect. P4 correctly retains the limitation
that process tracing perturbed start time (4.5–5.5 seconds traced versus about
1.13 seconds untraced) and that the remaining guest/vsock/host subpartition was
not measured.

Native identity and cleanup evidence is sufficient for this bounded diagnosis:
the canonical wrapper records the supplied target, exclusive lease token,
fail-closed x86_64/KVM preflight, source commit, binary hashes, kernel/rootfs
hashes, and final `exit=0`; the final `finalize` invocation performs exact
absence checks for the SVM materialization, three allocation run directories,
and cgroup scopes. The untraced and traced trial cleanup reports all zero
teardown deltas. `lease-holder.sh` removes token-owned metadata on `EXIT`, and
the successful outer wrapper closes the release stream and waits for that
holder. The RCA is appropriately candid that the first local launcher
transcript was overwritten and that an early wrong debugfs path was retained
and superseded, while preserving separate remote raw captures and statuses.

One archival limitation is non-blocking: each captured
`diagnostic-sources.tgz` contains the core runner, capture wrapper, RCA, and
Sim test, but not the observation patch, console observer, or summarizer. Those
files remain in the reviewed workspace and are named in the RCA; the patch's
reproducibility defect is nevertheless a required correction below.

## Findings and dispositions

### F-01 — HIGH — saved observation patch is not cleanly applicable

Iteration 1 evidence: `git apply --check docs/analysis/native-observation-283.patch`
returns exit 128 with `error: corrupt patch at line 132`. The final hunk is
declared as `@@ -1754,6 +1763,8 @@`, but it contains five old lines and
seven new lines. `git apply --check --recount` succeeds for
`handlers.rs`, `lib.rs`, `vmm.rs`, and `vm_driver.rs`, so this is a bounded
hunk-count defect rather than evidence that the markers could not be used.

Impact: the RCA presents this file as the saved observation-only patch that
supports the native stage measurements, but a third party cannot apply it with
the normal Git command. The raw logs, binary identity, source paths, and
untraced repetition still support the diagnosis; this finding is about the
integrity and reproducibility of the evidence package, not production
correctness.

Required scoped revision: correct the final hunk counts (the content requires
`@@ -1754,5 +1763,7 @@`), run and retain a successful ordinary `git apply --check`, and record that
verification with the associated capture or RCA. If the capture archive is
intended as the immutable evidence bundle, include the corrected patch (and
the observer/parser used to interpret it) or explicitly label the archive
partial. Do not change production behavior or add instrumentation beyond the
existing observation markers.

Iteration 1 disposition: **OPEN; blocks approval until the saved patch is
corrected and its clean applicability is evidenced.**

Iteration 2 disposition: **CLOSED.** The final hunk is now
`@@ -1754,5 +1763,7 @@`; an independent ordinary `git apply --check
docs/analysis/native-observation-283.patch` exited 0, and the retained capture
at `.context/issue283-review-patch-applicability.ax3QEu/` records the same
command and status. The patch SHA-256 is
`7a9ec310738d3df3dd6b0ff5c4d1f6028a376e4cda8090e0e158ce0665cedf97`. The
observation content was not changed, and the RCA explicitly labels the source
archives partial rather than claiming they contain the omitted observer/parser
files.

### F-02 — MEDIUM — writer instrumentation does not prove wire delivery

Iteration 1 evidence: the patch changes the two beacon branches to log the return values of
`write_half.write_all(b"SHUTDOWN\\n").await` and `write_half.flush().await`.
The retained log reports `written=Ok(()) flushed=Ok(())` in `0.119–0.432 ms`.
The native process trace is restricted to `%process`, and the guest read is
not traced. A successful Tokio writer call establishes that the host writer
did not await transport backpressure at that boundary; it does not establish
that a peer consumed the bytes or that the guest reached its read.

The RCA already makes the guest-read limitation explicit, but phrases the
result in places as “the host delivers SHUTDOWN” and “the host wrote and
flushed SHUTDOWN.” Those phrases can be read as wire-delivery claims. The
causal conclusion remains valid when stated at the observed boundary: the
writer's application-level calls returned promptly while `VmDriver::stop`
continued to await its fixed deadline.

Required scoped revision: replace the stronger phrases in the result,
prediction, and Branch B evidence with “`write_all`/`flush` returned” (or
equivalent), and state once that no peer-consumption or guest-read syscall is
claimed. Do not require a new syscall trace or production instrumentation for
this diagnosis.

Iteration 1 disposition: **OPEN; remediate as a wording/evidence-boundary
correction in the RCA.**

Iteration 2 disposition: **CLOSED.** Re-reading the corrected result, P3,
stage table, Branch B, and Branch C wording confirms that each SHUTDOWN claim
is limited to observed application-level `write_all`/`flush` returns. The RCA
now explicitly says it does not claim wire delivery, peer consumption, or a
guest-read syscall. The measured durations and source-supported guest-ordering
inference remain unchanged, so no new native replay or broader test was
necessary for this wording-only correction.

### Accepted non-findings

- The Sim test is not a full native `VmDriver`/Cloud Hypervisor simulation, but
  the RCA declares that boundary and uses the existing `Driver` driven port to
  prove the shared convergence-owner invariant required by #283. No new seam
  or fabricated observation state was found.
- The native failure-example trials exit 1 because the existing cleanup sees a
  replacement beacon bind collision. The RCA retains those trials as failed
  controls and does not use them as passing acceptance evidence; no cleanup
  assertion was loosened.
- No production observation markers remain locally, and no source/API/design
  change is requested by this review. The pre-existing `AGENTS.md` modification
  remains outside this review's ownership.

## Dimension scores

| Dimension | Score | Independent assessment |
| --- | ---: | --- |
| Causality logic | 9/10 | Complete owner paths and counterfactual release/finite-job controls connect each observed delay to a mechanism; guest-read step is explicitly qualified. |
| Evidence quality | 8/10 | Corrected patch applicability and bounded writer wording remove the two iteration-1 integrity defects; the partial source archives and untraced guest-read limitation remain honestly disclosed. |
| Alternative hypotheses | 9/10 | Transport, VMM early exit, cleanup, tracing, historical cap, cancellation/retry, and failure-path alternatives are pursued or explicitly bounded. |
| Five-Why depth | 9/10 | Branches A–C reach evidence-backed stopping points rather than inventing organizational or subsystem motives; forward and reverse checks are included. |
| Completeness and coverage | 8/10 | Covers shared-owner blocking, start/stop stages, guest ordering, controls, failure retention, cleanup, source identity, and instrumentation limits; no unsupported universal performance claim. |
| Solution traceability | 8/10 | Because this is diagnosis-only, no remedy is selected; bounded design questions map directly to A/B/C and preserve the #283/#260 boundary without orphan API recommendations. |

Overall score: **8.50/10** (51/6). This is the final score after iteration 2;
the iteration-1 score was 8.33/10 (50/6) while F-01 and F-02 were open.

## Iteration log

### Iteration 1 — 2026-09-09

- Read the complete RCA, issue #283/#260 bodies and comments, production owner
  paths, diagnostic test, scripts, observation patch, retained Sim/native
  captures, and mandatory review criteria.
- Recomputed stage durations from the retained first traced, second traced,
  and untraced logs; verified the kill/reap trace, native identities, final
  cleanup status, and intentionally red Sim results.
- Ran focused static checks listed above; discovered F-01 and F-02.
- No remediation was performed and no production or diagnostic file was
  modified by this reviewer.
- Verdict: `CHANGES_REQUESTED`.

### Iteration 2 — 2026-09-09

- Re-read only the investigator's bounded corrections for F-01 and F-02 and
  verified that the substantive native observations, stage measurements,
  intentionally red Sim result, and production source were not changed by
  those corrections.
- Independently ran the corrected ordinary command
  `git apply --check docs/analysis/native-observation-283.patch`; it exited 0.
  Independently computed the corrected patch SHA-256 as
  `7a9ec310738d3df3dd6b0ff5c4d1f6028a376e4cda8090e0e158ce0665cedf97`,
  matching the corrected hash recorded in the RCA; the retained applicability
  capture independently records the command and exit status.
- Verified the corrected RCA states application-level `write_all`/`flush`
  returns and explicitly disclaims wire delivery, peer consumption, and a
  guest-read syscall. Verified that existing source archives are labeled
  partial and are not represented as complete evidence bundles.
- No native replay, broad suite, mutation run, production instrumentation,
  test assertion, API, timeout, or architecture change was needed or
  authorized. The first review's dirty-worktree and evidence-preservation
  checks remain in force; this reviewer changed only this review artifact.
- F-01 disposition: `CLOSED`. F-02 disposition: `CLOSED`.
- Final score: **8.50/10** (51/6). Verdict: `APPROVED`.

## Verdict

**APPROVED** — F-01 and F-02 are closed by bounded, independently verified
corrections. The underlying #283 diagnosis remains bounded, evidence-backed,
and correctly separated from future DESIGN/remediation work; this approval
does not authorize production, API, test, or architecture changes.
