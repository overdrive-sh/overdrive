# ADR-0081 — Endings classify into three classes, not two: Platform Reclamation, Artifact Disposal as its non-ending sibling, and the supervision handle as a claim on authoring an ending

## Status

Accepted. 2026-08-11.
Decision-makers: Hera (nw-ddd-architect, DESIGN wave).
Mode: propose.
Tags: phase-2, vm-driver, domain-model, ending-taxonomy, ubiquitous-language,
reconciler, application-arch, GH-42.

**Provenance.** This ADR is extracted from `docs/product/architecture/brief.md`
§ *Domain Model* → *VM workloads — the ending taxonomy* → **DD-1**, **DD-1(b)**
and **DD-1(b.i)** (Hera, 2026-08-11). It was reserved as **ADR-0081** by
deferral **H-1** — *"DD-1 is ADR-worthy and platform-wide, not VM-specific"* —
surfaced during the same DESIGN wave and approved by the user. **Companion
ADRs [ADR-0082](adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md)
§ D8 and [ADR-0083](adr-0083-driver-registry-and-per-driver-allocation-payload.md)
§§ D5–D7 already cite DD-1 / DD-1(b) / DD-1(b.i) by name and consume this
decision** — both were drafted before this ADR had a number, against the
domain model directly. This ADR is the record those citations now resolve to;
see § *Already consumed downstream* below. `brief.md` § *Domain Model* remains
the full rationale, evidence base and trap-by-trap argument; this ADR is the
compact, durable decision record extracted from it.

**Records a platform-wide domain rule. Does not decide any VM-specific
application of it** — the exact `Action` variants, executor signatures,
`Driver` port methods and the `VmReclamation` reconciler are
application-architecture, already pinned in ADR-0083 §§ D5–D7.

**Narrows [ADR-0078](adr-0078-crash-and-recover-is-durably-observable-last-terminated-plus-restart-count.md)
§ D1** (a reachability claim in `CrashFacts::advance`'s docstring — see
§ *Narrows ADR-0078 § D1* below) — does not supersede it, and does not touch
its mechanism. **Binds every present and future site that emits a
[ADR-0037](adr-0037-reconciler-emits-typed-terminal-condition.md)
`TerminalCondition`** — does not supersede it, and does not change its type or
publication boundary. Neither ADR is edited by this one; both edits this ADR
obliges are DELIVER-wave obligations, stated explicitly below rather than
performed here.

Depends on `.claude/rules/development.md` § "A convergent record cannot answer
'did it happen'", § "Type-driven design", § "Persist inputs, not derived
state"; `.claude/rules/reconcilers.md` § "The decision rule"; CLAUDE.md
§ "Deferrals require GitHub issues — AND user approval BEFORE creation".

**No GitHub issue is created and none is invented.** This ADR is itself the
discharge of deferral H-1 (the "should this be an ADR" question). Where
deferred, unrelated work is named below for completeness, it is cited by its
existing issue number only — #260 (bound the serial dispatch path), #261
(cross-workload capacity accounting / `TransitionReason::NoCapacity`'s
construction site), #262 (`JournalStore::probe()` never called in
production), #263 (`dst-lint`'s `BANNED_APIS` gap for `Command` /
`UnixListener`) — none of which this ADR re-litigates or expands.

---

## Context

`brief.md` § *System Architecture* → **SD-1** (Cloud Hypervisor VM driver,
2026-08-10) introduces the platform's first *routine, non-exceptional*
destruction of a healthy running workload that the platform is then obliged to
recreate: a VM host process reaped by a boot-epoch or steady-state convergence
pass, because the vsock beacon channel that would otherwise report its ending
honestly (`spike/findings.md` P2 — one guest-initiated connection carrying
`READY …` then `EXIT n` as two distinct reads, then EOF) has no watcher left
to receive it.

The platform's existing ending vocabulary has exactly one word for "the
platform ended it" — Intentional Stop (`StoppedBy::{Operator, SystemGc}`) —
and one path for "it ended badly" — Workload Failure. Neither describes this
new case, and applying either naively is wrong in a different way each time:

1. **Classify the reap as an Intentional Stop.** `is_intentionally_stopped` /
   `is_restartable`'s asymmetric filter then excludes the row from the restart
   path permanently: every VM stays dead after an `overdrive serve` restart —
   the exact inverse of SD-1's stated intent ("lets the existing
   restart/backoff reconciler re-drive them").
2. **Classify the reap as a Workload Failure (do nothing).** The reconciler's
   restart/backoff budget is charged for a destruction the workload never
   caused. Six `serve` restarts against `RESTART_BACKOFF_CEILING = 5` drive
   **every** VM workload on the node to `RestartBudgetExhausted` — a
   node-wide terminal cascade caused by routine upgrades.
3. **A third failure neither default names, and it bites first.** For a
   Job-kind workload the finalise branch is evaluated before the restart
   branch, gated on a predicate that only tests "terminal and not an
   intentional stop." A reap row satisfies that predicate and is finalised
   `TerminalCondition::Failed { exit_code: Some(0) }` — a fabricated exit code
   on a workload that never exited, and the workload is never restarted at
   all. Fixing (2) without fixing this converts a budget cascade into a
   silent lie, which is strictly worse.

**Not VM-specific, and deliberately so.** Node drain, live migration, eviction
under pressure and rolling node upgrades are all instances of the same
platform action: a live, supervised instance destroyed while the workload's
intent still stands. A word minted for the VM case would guarantee
re-deriving this rule from scratch — possibly incompatibly — at the first of
those.

**Evidence base.** `brief.md` § *System Architecture* SD-1 … SD-5;
`spike/findings.md` P2; [ADR-0078](adr-0078-crash-and-recover-is-durably-observable-last-terminated-plus-restart-count.md)
(durable crash-observability surface);
[ADR-0077](adr-0077-lww-counter-derives-from-the-prior-row-not-the-tick.md)
(LWW derivation); [ADR-0037](adr-0037-reconciler-emits-typed-terminal-condition.md)
(the typed terminal-claim surface); ADR-0047 (`WorkloadKind`); ADR-0030
(`AllocationSpec`); ADR-0023 (action-shim publication boundary).

---

## Decision

### D1 — An ending classifies into three classes, not two. Restart eligibility, restart-budget consumption and job finalisation are all functions of the class

> **Every terminal `AllocStatusRow` belongs to exactly one Ending Class.
> Restart eligibility, restart-budget consumption, and job finalisation are
> functions of that class — never of the driver, never of the terminal state
> alone, and never of a substring of a reason's text.**

| Ending Class | Meaning | Re-drive the workload? | Consumes restart **budget**? | Finalises a Job-kind workload? | Increments observable restart **count**? |
|---|---|---|---|---|---|
| **Intentional Stop** | An authority withdrew the workload or its intent | No | n/a | n/a (no successor) | n/a |
| **Workload Failure** | The workload itself ended badly | Yes | Yes | Yes — the run is over | Yes |
| **Platform Reclamation** | The platform destroyed *one runtime instance* while the workload's intent still stands | Yes | **No** | **No** — the run is not over | Yes |

**Platform Reclamation is not an ending of the workload. It is an
interruption of one allocation attempt.** That single sentence generates all
four columns.

**The general, binding form of the rule:**

> **No reconciler may author a terminal claim on a Platform-Reclamation row.**
> A terminal claim asserts *how the workload's run ended*; a reclaimed run has
> not ended. Every branch — in every reconciler, present or future — that
> would emit a terminal claim against an allocation is a binding site for this
> rule.

**The reclaimed row's `AllocState` is `Terminated`, never `Failed`.** `Failed`
asserts the workload's run ended badly; a reclaimed run did not end at all —
that is the whole content of this decision — so `Failed` would be the
misclassification this rule exists to refuse, written into the reclamation
itself. The choice is load-bearing beyond naming: at least one reconciler
branch in this codebase keys a failure-fabrication path directly off
`state == Failed`, so writing `Failed` here would make that branch reachable
by construction, for an ending that is not a failure.

**The class must be derivable from the terminal row alone.** A class derived
from the driver (e.g. "VMs are always Platform Reclamation") is a class only
VMs can be in — and node drain, eviction under pressure and live migration are
all Platform Reclamation on workloads that are not VMs. Keying the
classification on the row, rather than on which driver or which pass produced
it, keeps one rule for one concept across every workload class that will ever
need it.

### D2 — Platform Reclamation and Artifact Disposal are not two Ending Classes: one class with a precondition, plus one non-ending concept

A reclamation-shaped reconciler may run in more than one regime (e.g. a
boot-epoch pass and a steady-state tick). Those regimes are **not** two
Ending Classes. What differs between them is **whether an ending is authored
at all**:

| What is destroyed | Domain concept |
|---|---|
| A runtime instance of a **non-terminal** allocation | **Platform Reclamation** — this ADR's third Ending Class |
| Host state backing **no live instance of a non-terminal allocation** | **Artifact Disposal** — not an ending; authors no terminal row, writes no row at all |

**The precondition, stated so the safety property falls out of it rather than
sitting beside it:**

> **A reconciler may author a Platform Reclamation for an allocation exactly
> when the platform can no longer honestly classify that instance's
> ending** — that is, when it holds **no live supervision handle** for it.
> Where the handle exists, the ending is still classifiable, so reclamation is
> **never** authorised and a supervised, non-terminal instance survives every
> tick.

A boot-time convergence pass is the **degenerate case** of this same rule, not
a second rule: at boot, whatever tracks live supervision is reconstructed
empty, so the precondition is true for every instance by construction — one
predicate evaluated against an input that happens to be uniformly empty at
boot, not a special-cased boot behaviour.

**Why not promote the two regimes to two classes — three refusals:**

1. **Not derivable from the terminal row alone.** A regime-derived class
   would require the row to say which pass wrote it, which D1 forbids.
2. **Fails D1's own generality requirement.** Node drain, eviction under
   pressure and live migration each destroy a live, supervised instance at
   what would be called "steady state" under a regime-keyed vocabulary — under
   that vocabulary each is either unnameable or needs a fourth word; under the
   precondition above they are simply the class that already exists.
3. **Conflates a reconciler's own conservatism with the taxonomy.** "Never
   reclaim a supervised, non-terminal instance" is a correct, load-bearing
   safety property of a *particular* reclaiming reconciler's authorisation —
   not a property of the Ending Class itself. Freezing it into the platform's
   vocabulary would make the safety rule and the taxonomy inseparable, so the
   first feature that legitimately reclaims a live instance under a different
   authorisation would have to break the taxonomy to do it.

### D3 — The supervision handle is a claim on authoring an ending, not a grip on a running process

The precondition in D2 asks *"can the platform still honestly classify this
instance's ending?"* and answers it by asking whether a supervision handle is
held. That substitution is sound **only if the handle is held for exactly as
long as the answer is yes**:

> **The supervision handle is the platform's claim to author ONE instance's
> ending. It is held from the moment that instance starts until the moment
> that ending has been AUTHORED — the terminal row is written — or until
> authorship has been ABANDONED as impossible. It is not released at process
> death, at an exit watcher's return, or at any point at which an exit report
> is still in flight.**

If the handle were instead released at process death, the platform would
transiently hold "no live handle" for an instance whose ending it is actively
in the process of writing honestly — a window on **every ordinary exit** —
and a reclamation sweep landing inside that window would author a Platform
Reclamation over an ending the platform was mid-way through classifying
correctly. This reproduces D1's traps 2 and 3, misclassified in the opposite
direction: a crash relabelled reclamation escapes the restart budget (a
crash-looping workload restarts budget-free and the ceiling never fills); a
completed run relabelled reclamation is not finalised but re-driven — a
duplicate execution of a side-effecting run.

Three readings follow from the corrected definition:

1. **Ordinary exit.** The handle is held across the exit report, so the
   window never opens.
2. **A stop whose kill failed (an orphan).** The ending is authored **on the
   stop path** — the row is terminal while the process survives, which is
   exactly what makes it an orphan — so the handle is released **there**,
   notwithstanding the live process. What remains is an orphan process, not a
   supervised instance: the platform's claim to author that ending is already
   discharged, which is what makes the orphan reachable by Artifact Disposal
   at all.
3. **Abandonment.** Where authorship cannot complete — the write fails
   terminally, the authoring task dies with the process — the handle is
   released and the allocation becomes reclaimable. This is not a loophole:
   at that point the platform genuinely cannot classify the ending, which is
   precisely Platform Reclamation's precondition. The abandonment boundary
   must be pinned mechanically (what concludes an authorship attempt), because
   a handle that is never released is a permanently unreclaimable orphan.

**The corollary, binding the ending-authoring paths rather than the
reclamation path:** *once an instance's ending is authored, no further ending
may be authored for that instance.* Retiring the handle at authorship is the
same rule as D2's refusal to let Artifact Disposal overwrite an authored
ending, applied to the exit path instead of the sweep path. A terminal-row
instance that is *still supervised* is thereby made **unrepresentable**, not
merely disallowed — the byte-unchanged assertion on the disposal path holds
structurally rather than by the luck of no watcher being alive.

**Three consequences of the precondition, each testable and each
kill-authorising:**

1. **The blank cell — an allocation this platform is not supervising, at
   steady state, but has a non-terminal row — must be read as SETTLED, never
   as MOMENTARILY ABSENT.** Authorisation to reclaim rests on the platform's
   *actual, settled* inability to classify the ending — not on an
   allocation's absence from a set that may simply be stale between two
   independently-read observations.
2. **Absence of evidence is not evidence of absence; the predicate fails
   safe.** A supervision reading that is *unavailable* (not yet hydrated,
   hydration errored, the surface has not been populated) must read as **not
   authorised**, never as "unsupervised" — because "no live handle"
   authorises a kill. Because the supervision reading and the host
   observation are taken at two different instants, any skew between them
   must resolve toward **held**: doing nothing on a stale *held* reading costs
   one sweep interval; acting on a stale *unsupervised* reading kills a live
   instance.
3. **Authorisation is a precondition of the write, not merely of the
   emission.** A tick decides at time *t* and its executor writes at
   *t + ε*; an ending authored inside that gap is an ending, and the
   refusal to overwrite an authored ending binds the reclamation write exactly
   as it binds the disposal write. An allocation re-observed terminal at
   execute time authorises nothing, and the declared delta of the write
   collapses to empty.

### D4 — Artifact Disposal needs its own word, because reusing "Platform Reclamation" is a lie

Disposing of a *terminal* allocation's leftover host state must **not**
write a Platform-Reclamation row. That allocation's ending is already
authored — possibly as an Intentional Stop, via an operator stop whose kill
failed (D3 reading 2) — and overwriting it would re-classify an honest ending
as a platform one, increment the observable restart count for a restart that
never happens, and clobber the durable crash-observability snapshot
(ADR-0078's `LastTerminated`). The two are therefore separate concepts with
separate contracts:

| Term | Pinned meaning | What it is NOT |
|---|---|---|
| **Platform Reclamation** | The platform destroyed one runtime instance while the workload's intent still stands, and owes a replacement. | Not a stop, not a crash, not garbage collection of an absent intent. |
| **Artifact Disposal** | Destroying per-allocation host state that backs **no live runtime instance of a non-terminal allocation**. Authors no ending, writes no row, moves neither restart counter. | **Not** Platform Reclamation — that ends a live instance; this one has no live instance to end. |

### D5 — Naming

The recommended disposition is `StoppedBy::PlatformReclaimed`, appended to the
platform's existing "who ended this" vocabulary alongside `Operator`,
`Reconciler`, `Process` and `SystemGc` — the domain question is "who ended
it," the answer is "the platform," and `SystemGc` sits one variant away as the
contrast case: `SystemGc` means the intent is *gone*; `PlatformReclaimed`
means the intent stands and the platform owes a replacement. **No
VM-specific vocabulary anywhere in the row, the class, the payload, or any
predicate surface** — no `boot_epoch` / `steady_state` / `is_boot` field, no
`Vm`-prefixed disposition. Exact enum placement, discriminant index and
field types are application-architecture (ADR-0083 § D6 pins these).

### D6 — Where the discriminating fact must live

The supervision discriminator D2/D3 gate on must be an **observed input**
hydrated fresh each evaluation, never a marker a reconciler stamps on its own
prior output. `.claude/rules/reconcilers.md`'s fingerprint-as-diff
anti-pattern already forbids this structurally; the domain adds an
independent reason that holds regardless of that anti-pattern: the
precondition asks *"can the platform still classify this ending?"* — a fact
**about the world** (does a supervision handle exist for this instance?), not
a fact about what a reconciler last emitted. A stamped marker would answer a
different question, and here that substitution gates whether a live instance
is killed.

---

## Narrows ADR-0078 § D1

`CrashFacts::advance`'s edge-case bullet (quoted in ADR-0078 § D1, backed by
the code docstring at `observation_store.rs:1122-1132`) reads:

> *"An operator-stopped `Terminated` prior counts like any other terminal.
> `advance` deliberately does NOT consult an intentional-stop discriminator.
> **This is unreachable in Phase 1**: `is_restartable` excludes
> intentionally-stopped rows … so no `RestartAllocation` is emitted, and a
> resubmit mints a FRESH `alloc_id`… If a future path makes it reachable,
> excluding operator stops from the count is a decision to take THEN … do not
> improvise it now."*

**Platform Reclamation makes `Terminated → Running` on the same LWW key
reachable — for the first time, and reachable correctly.** The class this ADR
defines is restart-eligible (D1) and is written as `Terminated` (D1's
boundary note), and its restart re-drives through the existing restart
mechanism at the same allocation key. `CrashFacts::advance` needs **no code
change** — it already produces the right answer: the reclamation writes
`Terminated`, the restart writes `Running` superseding it at the same key,
and `advance` snapshots the terminal into `LastTerminated` and increments
`restart_count` exactly as it would for any other recovery. **Do not "fix"
`advance` to exempt reclamation** — doing so would erase the occurrence,
which is precisely the defect ADR-0078 exists to prevent, reproduced by the
feature that cites it.

What *does* need correction is the docstring's **claim of unreachability**,
which becomes false the moment this decision's disposition exists in code.
The docstring's advice for *operator* stops is unamended and stays correct —
`is_restartable` continues to exclude them, so `Terminated → Running` on an
operator-stopped row remains genuinely unreachable, and ADR-0078's own
instruction ("excluding operator stops from the count is a decision to take
THEN") stands unchanged for that case.

**Binding on the DELIVER-wave commit that appends `StoppedBy::PlatformReclaimed`:**
that commit must also carry a minimal, dated amendment note on ADR-0078 § D1
— in ADR-0078's own established in-place-amendment style (no supersession;
precedent: ADR-0078 § Blockers item 2's amendment note on ADR-0077) —
pointing here, and must correct the now-false "unreachable in Phase 1" clause
in both the ADR text and the `observation_store.rs` docstring in the same
commit. Editing an accepted ADR is a user decision (ADR-0078's own stated
precedent); this ADR states the obligation and its exact location, not the
edit itself.

The companion quantity split — that the **budget**
(`WorkloadLifecycleView.restart_counts`) is exempt while the durable,
operator-visible **count** (`AllocStatusRow.restart_count`) must increment —
is recorded in `brief.md` § *Domain Model* → DD-2 and is not restated here;
this ADR narrows ADR-0078's reachability claim, DD-2 rules on ADR-0078's
mechanism, and the two are consistent by construction (both rest on the same
"do not fix `advance`" instruction above).

## Binds ADR-0037

ADR-0037 defines `TerminalCondition` as a reconciler's *stable, interpretive*
claim — emitted "when its `reconcile` body concludes that no further
convergence work will be attempted for this allocation." D1's general rule
("no reconciler may author a terminal claim on a Platform-Reclamation row")
is the direct corollary of that definition applied to this class: a reclaimed
run has **not** concluded — D1's opening sentence is exactly this — so no
reconciler, present (`WorkloadLifecycle`, `ServiceLifecycle`) or future (any
reconciler ADR-0037 § 5's additive-minor SemVer convention lets add new
`TerminalCondition` variants to), may attach one to a Platform-Reclamation
row, and no terminal-shaped `Action` may carry `terminal: Some(_)` against
one either.

This does **not** narrow ADR-0037's own decision — the `TerminalCondition`
type, its Action-boundary publication mechanism, and its SemVer convention
are all unchanged. It adds one binding precondition to every site that type
has ever had, and every site it will have. ADR-0083 § D6 already discharges
this as a testable property ("no reconciler may emit a terminal claim for an
allocation carrying this disposition"), stated to hold "against reconcilers
that do not exist yet" — which is this ADR's general rule, not a
VM-specific instance of it.

`ADR-0077` (LWW derivation) is unaffected: the reclamation and its restart
both ride the existing whole-row LWW write with no new merge argument,
per `brief.md` § *Domain Model* → DD-2, not restated here.

---

## Already consumed downstream

Both companion ADRs were accepted the same day as this decision, against the
domain model directly, before this ADR existed as a number. Neither is
amended by this ADR; both are the correct application of it and are recorded
here so the cross-reference resolves in both directions.

- **ADR-0082 § D8** — the `CgroupAccounting` port and
  `TransitionReason::VmOutOfMemory`. Explicitly checked against this ADR's
  boundary and correctly excluded from it: a cgroup OOM kill is a
  **Workload Failure** (`StoppedBy::Process`, an ordinary crash, restart
  budget consumed exactly as any other crash) — **never** Platform
  Reclamation, because "DD-1's third ending class is about the platform
  losing supervision, never about *why* a supervised VM died." The
  supervision handle is never released across an OOM kill (the exit watcher
  observes the death directly and authors the ending itself), so D2/D3's
  precondition is never satisfied and Platform Reclamation is never an
  available classification for this ending. This is D1's disjointness
  property holding on the one case most likely to be confused with it.
- **ADR-0083 § D5** — Cause-variant row 13 (`VmOutOfMemory`), consistent with
  the above.
- **ADR-0083 § D6** — the binding sites: a third conjunct on the predicate
  that gates a Job-kind finalise branch; the reclaimed row's identity
  (`state: Terminated`, `reason: Some(Stopped { by: PlatformReclaimed })`,
  `terminal: None`); the totality/disjointness property and the
  emission-level property that structurally enforce D1's general rule; and
  the "must NOT fix `CrashFacts::advance`" / "must NOT zero
  `AllocStatusRow.restart_count`" corollaries that follow directly from
  § *Narrows ADR-0078 § D1* above.
- **ADR-0083 § D7** — the `VmReclamation` reconciler
  (`.claude/rules/reconcilers.md` Bar 2), the two `Action` variants
  implementing D2's Platform-Reclamation / Artifact-Disposal split with their
  payload prohibitions (no disposition parameter, no regime field — both
  refused directly by D1's "derivable from the row alone" and D5's "no
  VM-specific vocabulary"), and item 2a implementing D3's supervision-handle
  lifecycle in code (the held-until-authored-or-abandoned phases, the
  `Driver::release_supervision` method).

This ADR does not re-decide any of the above; it is the record they already
cite by name.

---

## Alternatives considered

**Reuse `StoppedBy::SystemGc` for the reclamation disposition.** Rejected —
`SystemGc` means the intent is gone; reusing it here means every VM stays
dead after an `overdrive serve` restart, the exact inverse of the intended
behaviour, reached by taking the nearest existing word rather than asking
what it means.

**Treat the reclamation as an ordinary Workload Failure (do nothing).**
Rejected — a node-wide `RestartBudgetExhausted` cascade after six routine
restarts, and (compounding it) a fabricated `Failed { exit_code: Some(0) }`
finalisation of a Job-kind workload that never exited and is never
restarted.

**A regime-derived class** (e.g. `BootEpochReclamation` /
`SteadyStateReclamation` as two distinct Ending Classes). Rejected on the
three grounds in D2: not derivable from the row alone; fails D1's generality
requirement (drain / eviction / migration all reclaim a live, supervised
instance at what such a vocabulary would call "steady state"); conflates one
reconciler's own safety conservatism with the taxonomy itself.

**A process-death reading of the supervision handle** (released the moment
the OS process exits, rather than at ending-authorship). Rejected — D3:
opens a real, non-theoretical window on every ordinary exit in which a sweep
could author a Platform Reclamation over an ending the platform was
mid-way through classifying honestly, reproducing D1's traps 2 and 3 in the
opposite direction.

**A `VmInstance` aggregate, or any VM-shaped ending vocabulary.** Rejected —
fails Vernon's small-aggregates rule (the candidate has exactly the
allocation's lifetime and no independent identity), and — the binding
reason — guarantees re-deriving this exact rule, possibly incompatibly, the
first time node drain, eviction under pressure or live migration ships.

**A single `Action`/flag carrying a boolean discriminator** (e.g.
`authors_ending: bool`) instead of one Ending Class with a precondition plus
one non-ending concept. Rejected — a boolean cannot carry a third class; a
caller-declared flag would put the safety-critical distinction on
self-declared data rather than on the observed precondition D2/D3 define.
This is application-architecture territory (ADR-0083 § D7 pins the two
`Action` variants); recorded here only because the domain reason for
refusing the flag shape is the same reason D1 refuses reading the class off
anything but the row.

---

## Consequences

### Positive

- **One rule, general over every reconciler, present and future.** Restart
  eligibility, budget consumption and job finalisation stop being reasoned
  about per driver or per reconciler; they are functions of one
  classification, testable as an emission-level property rather than a
  per-site checklist.
- **ADR-0078's occurrence-preservation guarantee is untouched.** This ADR
  narrows a *reachability claim* in a docstring; it does not touch
  `CrashFacts::advance`'s mechanism, which already produces the correct
  answer.
- **Node drain, eviction under pressure, live migration and rolling node
  upgrades inherit a ready-made Ending Class** the day any of them ships,
  with no new vocabulary to invent and no risk of a second, incompatible
  taxonomy.
- **The supervision-handle race (D3) is closed at its source.** A
  terminal-row instance that is still supervised becomes unrepresentable
  rather than merely disallowed, which is testable as a structural property
  instead of an absence of overlap the test suite has to get lucky to
  exercise.

### Negative, stated

- **`AllocStatusRow.restart_count` now increments for a reason (recovery
  from reclamation) that is not a workload failure.** An operator reading
  "Restarts: 3" cannot tell, from that number alone, how many were crashes
  versus reclamations — distinguishing them requires reading
  `last_terminated.reason`. Accepted: this is what makes the count honest per
  ADR-0078; splitting it into two counters is a Phase-2+ candidate this ADR
  does not open.
- **This ADR obliges a dated amendment note on ADR-0078 § D1 the moment the
  reclamation class lands in code** (§ *Narrows ADR-0078 § D1* above). Until
  then, ADR-0078's docstring is not yet contradicted (the class does not yet
  exist in code) but the obligation is pinned now so it is not missed at
  DELIVER.
- **A workload reclaimed after a genuine prior failure carries a stale
  restart-budget timestamp** (`WorkloadLifecycleView.last_failure_seen_at`)
  into its next restart evaluation. Per ADR-0083 § D6 this costs at most one
  already-elapsed backoff window; recorded rather than fixed, by the ADR that
  discharges this one.

---

## Out of scope

Recorded facts, no forward pointer implied and no issue invented for any of
them:

- **The exact `Action` enum shape, executor signatures, `Driver` port
  methods and the `VmReclamation` reconciler's `State`/`View` types.**
  Application-architecture; ADR-0083 §§ D5–D7.
- **The Cause axis of `TransitionReason`** (`VmKernelNotFound`,
  `VmOutOfMemory`, and `NoCapacity`'s false-emitted-`yes` documentation
  claim). A companion domain decision (`brief.md` § *Domain Model* → DD-3),
  not covered here — DD-1/this ADR governs the **Disposition** axis only, and
  a reclamation disposition must never be counted toward the Cause axis's
  distinctness requirement.
- **`TransitionReason::OutOfMemory`'s residual live-`memory.events`-subscription
  mechanism.** ADR-0082 § D8 closes the *reduced* form (a post-mortem read,
  diagnosed correctly as Workload Failure per § *Already consumed
  downstream* above); the live-subscription mechanism itself stays deferred,
  unnumbered as of this ADR.
- **Bounding the serial dispatch path** (#260), **cross-workload capacity
  accounting / `NoCapacity`'s construction site** (#261), **`JournalStore::probe()`
  never called in production** (#262), and **`dst-lint`'s `BANNED_APIS` gap
  for `Command` / `UnixListener`** (#263). Each is real, already filed, and
  orthogonal to the Ending Class taxonomy — none is Platform-Reclamation- or
  Artifact-Disposal-shaped, so none is re-litigated here. Cited only so this
  ADR does not read as unaware of them.

---

## Changelog

- 2026-08-11 — Initial accepted version. Extracted from `brief.md`
  § *Domain Model* → *VM workloads — the ending taxonomy* → DD-1 / DD-1(b) /
  DD-1(b.i) (Hera, DESIGN wave for GH #42), reserved as ADR-0081 per deferral
  H-1, user-approved. Companion ADR-0082 § D8 and ADR-0083 §§ D5–D7 already
  consumed this decision by name before it had a number; cross-referenced in
  both directions in this version.
