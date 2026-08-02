# Reconciler Discipline

When — and whether — a piece of code should converge desired-vs-actual
state, and what the minimum bar is when it manages real resources.

This doc governs the **triage decision**: *should this be a reconciler,
and which bar must it meet?* The **implementation contract** — the
`Reconciler` trait shape (pure-sync `reconcile() → (Vec<Action>,
View)`), runtime mechanics (bulk-load + write-through), View schema
evolution, the worked retry-memory example — lives in
`.claude/rules/development.md` § "Reconciler I/O" and is the SSOT for
*how* to write one. This file is the SSOT for *when* and *whether*. The
reconciler-vs-workflow split lives in `development.md` § "Workflow
contract"; this file points at it rather than restating it.

The rule below was extracted from the veth-provisioner
adopt-without-state-verification bug (ADR-0061, amended 2026-06-03) and
the codebase reconciler audit that followed it. Both are distilled into
the precedent section at the end.

---

## The decision rule

**A reconciler candidate manages desired-vs-actual over a real resource
(kernel object, OS state, external system, durable store) where the
actual can DRIFT or be left PARTIAL (crash mid-operation), but currently
uses imperative one-shot / apply-once / adopt-and-skip logic instead of
observe → diff → converge.**

It is a candidate when ALL of these hold:

1. **There is a desired state independent of the actual.** The intent
   comes from config / intent store / a derivation — never inferred from
   "what's already there." If you cannot name the desired state without
   reading the kernel, you have no SSOT — fix that first (per
   `development.md` § "Persist inputs, not derived state").
2. **The actual state is observable.** You can read what currently
   exists (`getifaddrs`, `bpftool map dump`, `cgroup.subtree_control`,
   a `SELECT`, an external GET) and compare.
3. **The actual can diverge from desired** — drift while running, OR be
   left partial by a crash between sub-steps of a non-atomic apply.
4. **Re-running toward desired is safe** — each step is idempotent
   (add-if-missing, swallow `EEXIST`/`AlreadyExists`), so convergence
   tolerates being interrupted and re-run from the top.

---

## The two bars

"Should this be a reconciler?" is two questions, not one. Conflating
them produces both over-engineering (a full `Reconciler` impl for a
boot-time one-shot) and the bug class this rule exists to prevent
(imperative apply-once over state that drifts).

### Bar 1 — converge, don't apply-once (the floor; non-negotiable)

Any code meeting the four criteria MUST be **idempotent observe → diff →
converge**, even if it is a boot-time one-shot that never becomes a
`Reconciler` trait impl. Observe actual, compute the missing steps, add
only those, idempotently. This is the minimum bar and it is not optional
— adopt-and-skip over drift/partial-prone state is a bug (the
half-provisioned-resource class), not a style choice.

### Bar 2 — promote to the `Reconciler` trait (the destination)

Graduate to a full `Reconciler` impl on the runtime (pure-sync
`reconcile() → (Vec<Action>, View)`, per `development.md` § "Reconciler
I/O") when the state needs **continuous** convergence — drift repaired
*while the system is up*, not merely completed across restarts. That
requires the runtime machinery plus, usually, a new observe surface into
`actual`, new `Action` variants, and a host port trait.

### Converge-on-boot is the valid intermediate

A one-shot, idempotent observe → diff → converge at boot self-heals
across *reboots* — each boot re-diffs and completes whatever the last
crashed boot left partial — without a continuously-ticking reconciler.
Ship **Bar 1** when runtime drift is not yet in the threat model (e.g.
single-node, a resource not externally perturbed); defer **Bar 2**
behind a tracked issue until it is. Do NOT force a full `Reconciler`
impl when converge-on-boot suffices — but NEVER ship apply-once to dodge
writing the converge.

---

## Not a candidate

- **Pure computation.** No external/actual state — a `#[test]` or a
  proptest is the tool, not convergence.
- **Genuinely-terminal sequences (workflow-shaped).** A multi-step
  operation with a natural `Ok(result)` terminus is a *workflow*, not a
  reconciler — see `development.md` § "Workflow contract" and its
  reconciler-vs-workflow decision table. "Migrate X from A to B"
  terminates; "keep X looking like Y" converges.
- **Stateless request handling.** A handler that computes a response
  from its inputs holds no desired-vs-actual.
- **An executor already driven by a reconciler.** If a reconciler
  upstream computes the desired state and this code merely APPLIES it as
  an `Action` effect, it is an **action executor** (ADR-0023
  action-shim), not a reconciler candidate — wrapping it in its own
  observe → diff loop duplicates the reconciler that already owns the
  diff. `EbpfDataplane::update_service` is the canonical example: the
  `ServiceMapHydrator` reconciler owns desired-vs-actual; the dataplane
  is correctly its executor. Do not "make the executor a reconciler."
- **One-shot over already-idempotent primitives whose partial state
  self-heals.** Where every sub-step is a kernel/fs no-op on re-apply
  (`mkdir -p`, `subtree_control` controller re-enable) AND a
  mid-sequence crash leaves a state the next idempotent re-run
  completes, the apply-once is a *weaker* offender — Bar 1 (add an
  observe/verify pass) still improves it, but it is not the acute
  half-provisioned-resource bug. Judge by "does a crash leave an
  unrecoverable/misleading state?" (veth: yes — an adopted half-pair
  failed two layers downstream with a misleading error → promote) vs
  "does the next boot's idempotent re-write fix it?" (cgroup slices:
  mostly → lower urgency).

---

## Symptoms during review

The shapes that signal Bar 1 is being violated:

- `if <resource>.exists() { return Ok(()) }` / `.status.success() =>
  return Ok(())` — **adopt-and-skip.** Presence of one resource is taken
  as proof the whole desired state is satisfied. (The veth bug: `ip link
  show <cli>` success → adopt the pair untouched, never checking
  addresses / peer / up-state.)
- A `provision` / `setup` / `bootstrap` / `ensure` / `install` /
  `attach` fn that runs a sequence of mutating steps with NO prior
  observation of actual state — it writes desired and assumes.
- An error whose remediation names a *manual* fix ("run `ip link del …`
  and retry") on a target that is an immutable/appliance OS with no
  operator shell. There is no operator — the system must self-heal.
- A non-atomic create sequence (resource visible after step 1, more
  fallible steps after) with no path that completes a partially-created
  resource on the next run.
- **A `View` field holding a fingerprint / hash / "last written" marker
  of what the reconciler EMITTED, consulted as the diff.** The
  hash-shaped sibling of adopt-and-skip:

  ```rust
  if resource.exists() { return Ok(()) }      // the classic
  if Some(new_fp) == prev_fp { continue }     // the same bug, hashed
  ```

  Presence of a matching hash *of what I emitted* is taken as proof the
  desired state is satisfied. Two independent defects hide in it:

  1. **It is not `actual`.** A reconciler that never reads back the
     resource it manages cannot detect drift in it — only drift in its
     own intent. A populated `State.actual` field is NOT sufficient
     evidence: check that `actual` is *the resource this reconciler
     manages*, not a second input it derives desired from.
  2. **"Last written" is usually unverifiable.** The marker is stamped on
     the *emit* path — before the effect is attempted — and the write
     surface commonly returns `Ok(())` whether or not the write landed.
     The field then records "last emitted" while its docstring claims
     "last successfully written", and any failed write is permanently
     forgotten: the reconciler has dedup'd itself out of ever retrying.

  Compounding hazard: the runtime fsyncs the `View` **before**
  dispatching the actions (`development.md` § "Reconciler I/O", STEP 7 →
  STEP 8). A marker stamped on emit therefore outlives the effect it
  claims to record, across crashes and restarts.

  Watch for the marker being justified by `development.md` § "Persist
  inputs, not derived state" — a hash the reconciler *computed* is
  derived state; the input is the observed resource it declined to read.
  Citing that rule for such a field is a smell, not a defence.

---

## Codebase precedent

- **Converge-on-boot (Bar 1):** `veth_provisioner::provision`
  (`crates/overdrive-control-plane/src/veth_provisioner.rs`) — observe →
  `converge_steps` (pure) → idempotent execute; completes a
  half-provisioned pair, recreates a corrupted one, never tears down a
  usable one. ADR-0061 § 3.1 (amended 2026-06-03 "adopt untouched" →
  "idempotent converge-on-boot").
- **Full reconcilers (Bar 2):** `WorkloadLifecycle`,
  `ServiceMapHydrator`, `ServiceLifecycle`
  (`crates/overdrive-core/src/reconcilers/`,
  `crates/overdrive-core/src/service_lifecycle.rs`). **`ServiceMapHydrator`
  is the reference shape** — its `State` carries `desired` AND an `actual`
  that is the resource it manages (`ServiceHydrationStatus`), with
  `RetryMemory` sitting *beside* that real diff rather than standing in
  for it (`service_map_hydrator.rs:143-157`).
- **A `Reconciler` impl that did NOT converge — the fingerprint
  anti-pattern above, worked end to end.** `BackendDiscoveryBridge`
  (`crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs`).
  **Fixed by ADR-0079 (2026-08-02) — the defect is described below in the
  past tense and is no longer live at this site.** Kept because it remains
  the clearest in-tree account of the shape, and because the shape is not
  extinct (next bullet).

  *What it was.* A `Reconciler` by trait and apply-once by structure.
  `BackendDiscoveryBridgeState` held `desired: ServiceListenerSet` and
  `actual: RunningAllocSet` — but that `actual` was a second *input*,
  cross-producted with `desired.listeners` to compute the row to write.
  The resource it manages is the `service_backends` rows, and it never
  read them: the runtime's bridge `hydrate_actual` arm called
  `alloc_status_rows()`, and the runtime's only `service_backends_rows`
  call belonged to `ServiceMapHydrator`'s *desired* arm. So
  `view.last_written_fingerprint` WAS the diff.

  *Two defects hid in it, independent of each other.* **(1) It was not
  `actual`.** The bridge could observe drift in its own intent and
  nothing else. Its populated `State.actual` field was not evidence to
  the contrary — it was the wrong resource. **(2) "Last written" was
  unverifiable.** The marker was stamped on the *emit* path, and the
  write surface reports success either way: the LWW merge helper
  `apply_service_backends_lww`
  (`crates/overdrive-store-local/src/observation_backend.rs:1141-1172`)
  *does* compute the verdict and return it at `:1171`, and `write`
  discards it (`:500-503` — `if accepted { self.emit(row); } Ok(())`).
  The View docstring nevertheless claimed to record the row the bridge
  "successfully wrote", citing § "Persist inputs, not derived state" as
  its justification — false on both halves. And per the fsync-then-
  dispatch ordering named in the symptom bullet above, the marker
  outlived the effect it claimed to record, across crashes and restarts.

  *How it was found.* Not by reading the code — by a reproduction, and
  by a **contrast between two reconcilers under one fault**. While
  investigating a cross-restart LWW counter regression, a
  `ServiceBackendRow` write silently dropped by the observation store was
  never retried and the stale row stood indefinitely; the *same* drop
  against `WorkloadLifecycle` — which guards on the observed row, not on
  a view marker — self-healed on the next tick. That asymmetry is what
  localised the defect. Reproduced 2026-08-01; full analysis in
  `docs/analysis/root-cause-analysis-cross-restart-lww-counter-regression.md`
  § 4.2–4.3.

  *What fixed it (ADR-0079).* The state gained `service_backends:
  BTreeMap<ServiceId, ServiceBackendRow>`
  (`backend_discovery_bridge.rs:186`), hydrated by reading
  `service_backends_rows` under the *same* `ServiceId` derivation the
  desired arm uses, so the two halves cannot drift
  (`reconciler_runtime.rs:2792-2806`). The diff became structural
  equality against that observed row (`backend_discovery_bridge.rs:410-415`),
  and `last_written_fingerprint` was deleted — `BackendDiscoveryBridgeView`
  is now a field-less struct (`:256`). Retry falls out of the runtime's
  `has_work` self-re-enqueue; no View field, no backoff memo, and no
  write receipt on the store trait.

  *Two rulings worth carrying to the next instance.* **Converge only on
  what you author.** `ServiceBackendRow` has a second writer —
  `ServiceLifecycle` authors `healthy` — so whole-row convergence would
  have made the bridge the deterministic winner of every arbitration and
  erased the readiness signal. The bridge instead carries `healthy`
  through from the observed row, which also makes that field diff-inert:
  no convergence decision can be derived from a field the reconciler does
  not own. **A fingerprint is fine as a content-address and fatal as a
  diff.** `fingerprint()` survives verbatim at the correlation-key site
  (`backend_discovery_bridge.rs:419-423`); only its use as the dedup was
  removed.

- **The same shape, still live in tree:**
  `ServiceLifecycleView::last_emitted_backend_fingerprint`
  (`crates/overdrive-core/src/service_lifecycle.rs:370-387`, read at
  `:861`, stamped on emit at `:865`). Its own docstring names the bridge
  field above as the pattern it copies. It is deliberately not fixed:
  `ServiceLifecycle` authors only `healthy` on a row it *shares* with the
  bridge, so "diff desired against the stored row" is unavailable to it
  until row ownership is resolved — converging it on the whole row would
  make it fight the bridge, reintroducing on the health side exactly the
  clobbering the bridge fix removed on the membership side. ADR-0079 § D4
  records the exclusion and § D9 the ownership decision. Consequence, and
  it is real: a dropped readiness write is still permanently forgotten.
- **Executor, NOT a reconciler:** `EbpfDataplane` map writes — driven by
  `ServiceMapHydrator` via `Action::DataplaneUpdateService`. The
  dataplane is the executor; the hydrator owns the diff.
- **Deferred Bar-2 promotions (tracked):** veth → first-class network
  reconciler is [#197](https://github.com/overdrive-sh/overdrive/issues/197);
  cgroup hierarchy setup is
  [#198](https://github.com/overdrive-sh/overdrive/issues/198); XDP
  attachment lifecycle is
  [#199](https://github.com/overdrive-sh/overdrive/issues/199); the
  inbound-TPROXY shared routing infra (fwmark `ip rule` + `local` route +
  shared nft chain) is
  [#234](https://github.com/overdrive-sh/overdrive/issues/234). All
  four are Bar-1-today / Bar-2-when-drift-matters and share the same
  "host/node infrastructure reconciler" machinery; #197 is the candidate
  home for that shared model.

---

## Cross-references

- `.claude/rules/development.md` § "Reconciler I/O" — the `Reconciler`
  trait contract and runtime mechanics (Bar 2 implementation; SSOT for
  *how*).
- `.claude/rules/development.md` § "Workflow contract" —
  reconciler-vs-workflow decision table (the terminal-sequence
  disqualifier).
- `.claude/rules/development.md` § "Persist inputs, not derived state" —
  why desired must not be inferred from observed actual.
- `.claude/rules/testing.md` § "Tier 1 — Deterministic Simulation
  Testing" — convergence logic is the canonical
  `assert_eventually!(desired == actual)` target; a pure
  `converge_steps`-style diff is default-lane unit-testable.
- `.claude/rules/debugging.md` § "Leftover XDP attachments across runs"
  — the downstream hazard a converge-on-boot XDP attach (#199) closes.
- ADR-0035 / ADR-0036 (reconciler runtime), ADR-0023 (action-shim
  executor boundary), ADR-0061 (converge-on-boot precedent).
