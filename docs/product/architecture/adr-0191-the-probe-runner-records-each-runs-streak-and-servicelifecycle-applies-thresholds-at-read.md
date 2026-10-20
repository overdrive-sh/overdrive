# ADR-0191 — The probe runner numbers each run and records its streak; ServiceLifecycle applies probe thresholds when it reads the row

## Status

**Accepted — PROBE-RUN-COUNT approved by user 2026-10-13** (user ruling
"Probe threshold counting (round-2 H-A)"); the threshold rule — the probe
runner records the observed streak, `ServiceLifecycle` applies the threshold
when it reads the row and acts once per crossing — **approved by user
2026-10-13** (user ruling "Probe streak at source (replaces
PROBE-RUN-FLIP)"); the retry of a failed liveness stop (decision 5) — **D-1, option
(a), approved by user 2026-10-13** (user decision "D-1 liveness stop
retry"). GH #295. Recorded in the #295 feature delta,
`docs/feature/netns-density-295/feature-delta.md` § *[REF] vsock Attachment
Replacement DESIGN* (decision index), § *[REF] VM probe runner* (the
producer's numbering and streak and the consumer's threshold rule) and
§ *[REF] Observation row — ProbeResultRow V2 and the probe run sequence
(PROBE-RUN-COUNT)* (the row and the merge).

Related decisions, each its own ADR: the probe runner subsystem and the probe
row (ADR-0054), the per-role probe key and supervisor (ADR-0080), the single
restart authority that acts on a liveness terminate (ADR-0087), the
health-check spec and its thresholds (ADR-0057), the VM probe outcomes
that record no sample (ADR-0187), and the event a run that writes no row
emits (ADR-0192).

## Context

The probe runner's per-role probe task (`crates/overdrive-worker/src/probe_runner/mod.rs`,
`supervised_probe_loop` :560-597, `probe_tick` :483-541) runs each declared
probe once per period and writes one probe row per run, keyed by
allocation, role and probe index (ADR-0080). ServiceLifecycle reads only the
latest row per key and counts the thresholds the operator declared:

- the liveness failure streak advances on every reconcile pass whose latest
  row is a failure (`crates/overdrive-reconcilers/src/service_lifecycle.rs:1044-1095`);
- the readiness success streak advances on every pass whose latest row is a
  pass, and resets on a missing row (`:1198-1239`);
- startup attempts advance only when the latest failing row's observation
  time differs from the last one counted (`:1260-1286`).

Reconcile passes are not tied to the probe period. A pass that runs more
often than the probe counts one run several times; a period with no row —
a VM probe refused for node capacity or quiescence (ADR-0187), an adapter
error, a failed observation write — leaves the last row in place to be
counted again, so a single stale liveness failure can reach the threshold
and restart the workload with no new evidence.

ServiceLifecycle cannot reconstruct the streak from what it reads. The
evaluation broker coalesces submits for one reconciler and target
(`crates/overdrive-core/src/eval_broker.rs:78`, `:109-124`), and hydration
keeps only the latest row per key, so any number of runs, of any status, can
land between two looks. De-duplicating by observation time counts two runs
between looks as one, and a wall-clock step breaks the ordering. Counting
the runs since the last look by a run number still assumes every one of
them had the latest row's status: a failure, a pass and a failure between
two looks, after a counted failure, would count as three consecutive
failures (model counterexample, `specs/quint/probe-count`, hazard
`streak_bugReconstruct`).

The producer executes every run, so it is the only component that knows the
streak. The kubelet's prober keeps the same count (`resultRun`); in every
orchestrator examined except Nomad a probe streak moves once per probe
execution, and Nomad's `check_restart` counts elapsed time instead
(`docs/research/orchestration/probe-threshold-counting-kubernetes-nomad-research.md`,
findings K-1, N-2).

## Decision

**The probe runner numbers each written run and records the observed
streak in the probe row; ServiceLifecycle compares that streak with the
threshold when it reads the row, keeps no streak of its own, and acts once
per crossing.**

1. **The producer numbers runs.** Each probe task holds a run sequence per
   probe key for its life. The first row it writes carries sequence one;
   every later row carries the previous written row's sequence plus one. A
   run that writes no row — no sample (ADR-0187), an adapter error, a failed
   write — consumes no number. The sequence is monotone for the key's life:
   a probe task is never re-spawned for a live allocation, and a restart, or
   a `serve` restart, mints a new allocation. An allocation id is never
   re-minted for a key that has a stored probe row: the successor attempt is
   minted above every accepted allocation row and every reserved allocation
   key (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:1377-1387`),
   and allocation rows persist — the absent-intent GC stops them and does not
   delete them. Any change that garbage-collects allocation rows (the
   unbounded growth of view entries and terminal allocation rows is GH #320)
   deletes their probe rows in the same change.
2. **The producer records the observed streak.** Each written row also
   carries the number of consecutive written runs, ending with this one,
   that produced this row's status: one for the first written row, the
   previous written row's count plus one when the status is the same, one
   otherwise. A run that writes no row neither breaks nor extends it. The
   count is an observation, independent of any threshold. It lives with the
   run sequence in the probe task and is not persisted: a new probe task
   starts from one, and a new task exists only for a new allocation, so a
   restart can only delay an action, never cause an early one.
3. **The row merges by run sequence.** A written row replaces the stored row
   for its key only if its run sequence is greater. Its observation time no
   longer decides the merge; it remains the time shown to operators. The
   consumer's rule below depends on the stored row being the producer's
   latest: a merge by observation time could, after a backward clock step,
   keep an older row whose streak is no longer the current one.
4. **The consumer applies the threshold at read time.** A failure threshold
   is crossed when the latest row is a failure whose streak is at least the
   threshold, and a success threshold when the latest row is a pass whose
   streak is at least the threshold. ServiceLifecycle keeps no streak and
   does no arithmetic over the runs since it last looked, so coalesced or
   skipped looks cannot miscount, and a changed threshold in the spec takes
   effect at the next read.
5. **Act once per crossing; retry a failed stop on the next run (D-1).** For
   the liveness terminate, ServiceLifecycle
   keeps the run sequence at which it last acted for the probe, and acts on
   a crossed row of a running allocation only if that row is newer; it
   records the row's sequence only when it acts. A re-read of the same row
   does nothing. The stop is dispatched within the reconcile that emits it,
   so after a stop that succeeds the allocation is no longer running and
   nothing more is emitted; after a stop that fails, the next crossed row
   emits it again, so a failed stop is retried on the next probe row that is
   still past the threshold — at most once per new probe run, so the retry
   interval is bounded by the probe period — and never twice on one row. The
   startup failure is already announced once per allocation, and readiness
   is a level re-derived at every read; neither needs this memory.
6. **No new row changes nothing, for every role.** A key with no new row
   leaves every verdict where it is; readiness no longer resets on a missing
   row. Such a run is made visible by the not-sampled event (ADR-0192).
7. **All workload kinds.** The rule applies to every probed workload, not
   only VMs, and to the probes ServiceLifecycle reads today (index 0 of each
   role; the other indices are GH #319). The restart authority is
   unchanged (ADR-0087).

The row's new fields, their types, the version, the view field and the
exact threshold rule are in the feature delta.

## Alternatives considered

- **Per-pass counting (today).** One run is counted once per reconcile pass,
  so the threshold depends on how often the reconciler wakes; a stale row is
  counted across a no-sample window. Rejected.
- **De-duplicate by observation time.** Counts a distinct row once, but
  coalesced looks count two runs as one (undercount), and wall-clock steps
  break the ordering. Rejected.
- **Count the runs since the last look by run sequence alone.** Counts every
  run once, but assumes every run between two looks had the latest row's
  status, so a status that changed between looks overcounts and can
  terminate early (`streak_bugReconstruct`). Capping that count by a
  producer streak keeps two counters that must agree. Rejected.
- **Count elapsed time, as Nomad's `check_restart` does.** The threshold
  becomes a duration that assumes one run per interval; a probe that records
  nothing still advances toward a restart, which contradicts ADR-0187.
  Rejected.
- **Threshold in the producer, publish a verdict.** The probe runner would
  own the thresholds and ServiceLifecycle would read a level, as the
  kubelet's consumers do. The thresholds are policy of the reconcilers that
  act on them — the startup gate and the liveness terminate in
  ServiceLifecycle, which feeds the single restart authority (ADR-0087), and
  readiness → backend eligibility in whichever reconciler owns backend
  membership (ServiceLifecycle today; ADR-0080 § D5 moves it to the backend
  discovery bridge). Moving them into the worker splits that policy between
  the worker and the reconcilers. The producer records a fact, the streak,
  and each consumer applies its own threshold to it. Rejected.
- **Act once per failure streak.** Recording the streak's start and never
  acting again within the streak also prevents a repeat, but a stop that
  fails is then never retried while the failures continue
  (`streak_bugOncePerStreak`). Rejected.
- **Act at most once per probe key.** Never retries a failed stop at all
  (`streak_bugOnceEver`). Rejected.
- **Persist the producer's streak.** Would let a new task continue a streak,
  but a new task belongs to a new allocation, whose streak starts afresh
  anyway. Rejected.

## Consequences

- A threshold is reached after that many consecutive written probe runs of
  the same status, which is what the operator's threshold states, however
  often ServiceLifecycle reconciles and however many runs fall between two
  looks.
- A period with no sample advances and resets nothing; the not-sampled
  event (ADR-0192) makes it visible.
- A threshold change applies at the next read, with no reconciler state to
  migrate.
- A liveness stop is emitted once per crossing and repeated only when the
  previous one failed, at most once per probe run; today's code resets its
  counter on the emit and repeats the stop after a further threshold of
  failing reconcile passes.
- The probe row merges by run sequence, not observation time, in both the
  host and the simulated observation store.
- Amendment notes are carried by the decisions this one changes: ADR-0054
  § 5 (the merge), ADR-0055 (the View's probe counters), ADR-0080 § D5 and
  § D8 (no counter relocates to the backend discovery bridge) and ADR-0097
  D2 (startup-attempt de-duplication).
- The probe row gains a version: the new layout is appended to the row's
  envelope under the version-bump procedure of `.claude/rules/development.md`
  § "rkyv schema evolution". A row of the earlier version can only belong to
  an allocation of an earlier `serve` process, which boot reclamation
  reclaims before any probe runs.
- ServiceLifecycle's view loses the liveness and readiness streaks, the
  startup attempts and the startup attempts' last-seen observation time, and
  gains the run sequence at which it last terminated each liveness probe.
- The durable row now carries the streak an operator view could show; the
  API and CLI output are not changed by this decision.
- A crash restart leaves its predecessor's probe tasks running (GH #318);
  they write under the predecessor's allocation id, a different key, so they
  do not affect the successor's rows or verdicts.
