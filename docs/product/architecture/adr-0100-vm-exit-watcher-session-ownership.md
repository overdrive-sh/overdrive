# ADR-0100: VM exit watchers may claim only their own accepted session

## Status

**Accepted**, 2026-09-07, following independent DESIGN review:
[iteration 1 — APPROVED](../../feature/service-kind-vm-workloads/design/review-adr-0100.md).
Approval is not an implementation or E10 completion claim. Bounded correction
for `service-kind-vm-workloads` step `02-03`. The separate E10
expectation-contract disposition was subsequently approved by the user on
2026-09-07; see the ruling. Implementation review remains pending.

The ADR directory was audited through ADR-0099 before creating this file.
An earlier unpublished ADR-0100 stop-before-release draft was withdrawn and
removed; this is a different decision and does not restore that mechanism.
ADRs 0098 and 0099 remain unchanged.

## Context and changed assumption

Brief §105a.3 requires that an instance whose ending was authored cannot author
another ending. Its watcher transition currently checks only **“Held means
Starting | Live”** at the allocation key. Seed 257205 proves that a same-ID
replacement's Starting entry satisfies that check for the **old** watcher,
after ServiceLifecycle has authored the old VM's startup failure. The failed
replacement's cgroup cleanup kills the old VMM; that old watcher authors another
natural ending despite no replacement VMM being created.

The [ruling](../../feature/service-kind-vm-workloads/design/vm-restart-ending-authorship-ruling.md)
records complete production reachability, repeatable failing evidence, the
passing reclamation control, same-ID start-failure limits, and the separately
user-approved E10 stop-oracle disposition. This ADR narrows **watcher** transitions 3 and 4
to the originating accepted session's Live entry. It does not move the
authorship release boundary or reinterpret Starting as unsupervised.

## Decision

### D1 — Public API and existing ownership stay unchanged

No public method, trait, type, enum variant, field, parameter, action, event,
configuration, dependency, or persisted shape is added or changed. Retain
`Driver::{start,stop,status,live_allocations,try_begin_reclamation,release_supervision}`,
`AllocationHandle`, `ExitEvent`, `VmSupervision`, `LiveVm`, and `LiveMap` shapes.
No attempt/generation field crosses any boundary.

Starting and Live remain Held for supervision/reclamation. EndingInFlight
remains supervised and maps to status NotFound. The original release points,
observer retry/release policy, stop transition 3b, boot cleanup, and reclamation
lease remain unchanged. Do not make release async: it acquires no new async
effect under this decision.

### D2 — Reuse the existing accepted session's identity

The READY-winning arm creates exactly one existing `Arc<BeaconWriter>` for its
accepted session. Capture `Arc::downgrade` from that **same value** at creation,
before any possibility of looking up a later allocation entry; install the
strong reference in LiveVm exactly as today and pass only the Weak reference
to that session's watcher. Do not add a second identity allocation, counter,
map, pointer-to-integer conversion, or PID/path equality test. Never create an
empty Weak as an ownership fallback, and never rediscover the witness by
allocation lookup when the watcher finally wakes.

This relies on current production's one accepted session and one BeaconWriter
per spawned watcher (`vm_driver.rs:1523–1545`), not a new reconnect assumption.
A watcher holding only Weak must not keep the writer/task/socket value alive
after LiveVm release. Rust's [Weak lifetime and pointer equality contract](https://doc.rust-lang.org/std/sync/struct.Weak.html#method.ptr_eq)
provides identity without prolonging that value's lifetime. Retained Weak
backing storage cannot be reused for a replacement writer while that witness
exists; value equality and raw process IDs are not used.

The existing `ClaimGuard::try_begin_ending` tests and mutates under one existing
LiveMap lock. Its success condition is exactly: the allocation entry is
`Live(live_vm)`, `live_vm.beacon` is Some, and its downgraded pointer is
`Weak::ptr_eq` to the watcher's captured witness. On success, retain today's
Live → EndingInFlight transition, emitted flag, and one ExitEvent send. On any
other entry/state/identity, return false and emit nothing.

The same predicate governs `ClaimGuard::drop` when `emitted` is false: remove
only the originating session's still-Live entry. Starting, pre-beacon Live,
another session's Live, EndingInFlight, and absence are exact no-ops. An emitted
guard still performs no removal. No new event/log/error variant records refusal.

Keep the Running-confirmed gate await before the claim transition. Keep guest
report draining, exit classification, cgroup-accounting reads, and channel-send
behavior unchanged. This does not fence events already emitted before a later
terminal write; no separate such failing production schedule is established
by this evidence, and no global event/publisher redesign is authorized.

### D3 — Exact private signature delta

All listed items stay private to `vm_driver.rs`. Existing parameter order is
retained; append `beacon: Weak<BeaconWriter>` at these boundaries:

| Existing item | Exact additional contract |
| --- | --- |
| `ClaimGuard` | Add field `beacon: Weak<BeaconWriter>`; retain `alloc`, `live`, `emitted`. |
| `ClaimGuard::new` | `const fn new(alloc: AllocationId, live: Arc<LiveMap>, beacon: Weak<BeaconWriter>) -> Self` |
| `VmDriver::spawn_exit_watcher_task` | Final parameter, after `gate_receiver`, is `beacon: Weak<BeaconWriter>`; return remains `()`. |
| `run_exit_watcher` | Final parameter, after `gate_receiver`, is `beacon: Weak<BeaconWriter>`; remains async returning `()`. |

`ClaimGuard::try_begin_ending(&mut self) -> bool` and its Drop signature remain
unchanged. The witness is created in the existing READY arm and carried through
these boundaries to ClaimGuard. No change to `ProvisionedVmm`, the pre-READY
boot race, or any public signature is required. Implementation details beyond
these interface/ownership contracts remain the crafter's responsibility.

## Lifecycle Gate Ownership

| Existing state/result | Owner | Promise and permitted gate | Explicitly not owned |
| --- | --- | --- | --- |
| VM natural ExitEvent emission | VmDriver's session exit watcher | Its own accepted session still owns a Live ending claim, after the existing Running-confirmed gate | Service startup verdict, another attempt's ending, restart policy |
| Supervised allocation set | VmDriver | Every Starting, Live and EndingInFlight entry is claimed | Process liveness; eligibility for a second watcher to claim it |
| Startup failure / Stable | ServiceLifecycle | Existing startup observations/deadline or opt-out decide the Service result | Driver start success or artifact reclamation timing |
| Same-ID restart | WorkloadLifecycle | Existing row, run/stop intent and budget decide another attempt | Guaranteed start success or retroactive operator stop |
| Terminal artifact disposal | VmReclamation and its executor | Terminal/unclaimed artifacts are killed/discarded without another ending | Rewriting Failed to Terminated |

### Gate G-100 — originating session still owns the live claim

- **Evidence:** the ruling's owner trace and seed-257205 failure; concrete
  production path is registered ServiceLifecycle finalization → shim release
  → WorkloadLifecycle same-ID restart → failed beacon bind/cgroup cleanup →
  original watcher → `ClaimGuard::try_begin_ending` (`vm_driver.rs:935,2128`).
- **Owner and affected result:** VmDriver's original watcher alone decides
  whether it may emit its natural ExitEvent or abandon its own claim. No
  allocation or Service state is newly gated; the identity check restores the
  already-accepted per-instance authorship promise.
- **Failure projection:** absence, Starting, pre-beacon Live, different session,
  or EndingInFlight means false/no emission/no removal. These are not driver
  start failures and introduce no typed error. Existing malformed/EOF/timeout
  classification before this gate remains unchanged for a legitimate owner.
- **Ordering/budget:** capture at the original READY arm; pass through the
  watcher; await the existing Running-confirmed gate; then compare-and-mutate
  under one lock without await. Drop applies the same predicate. No new I/O,
  timer, deadline, retry, lock, or owner cancellation is introduced.
- **Unaffected:** all entry variants still block reclamation while claimed;
  ServiceLifecycle may release after terminal authorship before VMM death;
  ordinary reclamation remains valid; WorkloadLifecycle remains the sole
  restart owner; ADR-0099 remains the successful Running acceptance gate.
- **Counterexample:** rejecting every watcher merely because it sees Live
  would suppress a current guest's real natural exit. Accepting every Live
  would let an old watcher consume a replacement session's claim. Exact
  originating-session identity distinguishes both without a process-ID fence.
- **Cancellation/reconnect:** the existing convergence loop drains dispatch
  before shutdown checks; the observer drains each consumed event's retries.
  No new shutdown path is added. The guard's existing destruction is changed
  only to restrict which entry it may remove. There is no accepted-session
  reconnect/replacement mechanism; a new driver start creates a different
  writer identity. Closed transport does not itself grant ending authorship.
- **Evidence lane:** seeded owner-path safety regression and reclamation
  control; in-process gate/ownership complements; native E10 for actual kernel
  cleanup and operator projection, never for private claim assertions.

## Verification obligations

1. Keep seed 257205's invariant and triggering schedule unchanged: one VMM,
   existing bind rejection and failed-start cgroup kill, **no new natural-crash
   occurrence from the original instance**. A replacement start-rejection
   occurrence is allowed. The no-restart control preserves the entire original
   row/history and removes artifacts. Both must pass after implementation.
2. Exercise current-session success and unrelated-session refusal, including
   the failed guard's Drop. Complement the seeded production witness with
   source-local checks over Starting, pre-beacon Live, original-session Live,
   replacement-session Live, EndingInFlight, and absence. Assert the complete
   relevant map delta, not merely a false return. These complement the proven
   production invariant; synthetic states are not additional defect findings.
3. Preserve existing natural guest exit, stop-wins, Running-confirmed release,
   start-rejection, and terminal-unclaimed reclamation suites. A late original
   watcher cannot transition/remove the replacement session's entry. Feature
   disabled/not applicable: no new switch; Exec and VM pre-READY behavior stay
   unchanged. Each live/changed test carries its Contract Shape declaration;
   source-local pure properties use `/// CONTRACT_SHAPE: pure-function.`.
4. Re-run native built-default-feature E10 after the independently approved
   implementation using the existing lease/harness. Preserve 8/8 HTTP cells,
   status truthfulness, no redirects, no sentinel exposure and zero owned
   resource deltas. A green authorship test is not a passing E10. Under the
   ruling's subsequent user-approved disposition, distinguish stopping a Running
   allocation (Terminated) from disposing an already startup-failed allocation
   (may remain Failed with its failure preserved). This is not blanket acceptance
   of arbitrary crashes or Terminated-or-Failed. The narrow example/expectation
   correction is separately user-authorized, not covered by the earlier DESIGN
   review; implementation review and full verification remain required.

## Reuse, alternatives, and consequences

| Component | Change / Contract Shape | Universe and evidence |
| --- | --- | --- |
| BeaconWriter / LiveVm | REUSE existing session identity; no shape change | Existing writer lifetime; weak capture must not retain its value |
| VM watcher / ClaimGuard | EXTEND; bounded-change | One originating session, its allocation slot, competing same-ID slot value, and emitted events; seed 257205 plus exact map/event delta assertions |
| Reclamation, observer, shim and reconcilers | REUSE unchanged | No new writes, cleanup calls, policy or persistence; existing control/regression lanes |

Alternatives rejected:

- **Check Live rather than Starting-or-Live only:** fixes the immediate seeded
  phase but does not express whose Live entry is being consumed. The existing
  session identity makes the same ownership correction complete without a
  wider architecture.
- **Retain a strong BeaconWriter or join the watcher before release:** changes
  resource lifetime/termination sequencing. Weak identity achieves refusal
  without keeping sockets alive or waiting for guest death.
- **Add per-attempt tokens, generation fields, or serialized publishers:**
  redundant with the existing one-to-one accepted-session identity for this
  gate; public/persistent/global fencing exceeds the reproduced defect.
- **Stop-before-release or pre-start reclamation:** the former was withdrawn;
  the latter changes a different startup/cleanup boundary and is not necessary
  to reject the stale watcher. Neither establishes universal Terminated-after-
  stop semantics for a previously Failed allocation.

Reliability improves by rejecting false natural-ending authorship. The normal
path adds only weak-reference handling and identity comparison under its
existing lock; no additional I/O or elapsed-time bound. The Weak backing
allocation remains until its watcher completes, but the writer value and
resources may drop normally. No new external integration or technology is
selected; existing Rust/Tokio and Earned-Trust adapter probes remain unchanged.
Existing Service-health C4 L1/L2 diagrams are retained unchanged in
[`c4-diagrams.md`](c4-diagrams.md).
Rust's private types and existing crate boundaries enforce containment; the
seeded invariant supplies behavioral enforcement. Independent review must
verify necessity and API conformity, not just green tests.
