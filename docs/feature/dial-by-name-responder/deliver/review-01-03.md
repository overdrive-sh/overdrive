# Adversarial Review — `dial-by-name-responder` step 01-03

**Reviewer:** `nw-software-crafter-reviewer` (Opus, `reviewer_model: inherit`)
**Commit:** `53236c6a`
**Verdict:** NEEDS_REVISION (blocking) — the reviewer said REJECTED; substance is identical: one true blocker that cascades, plus a fail-closed divergence, plus a root-cause roadmap gap.
**Orchestrator status:** all three blocking claims independently verified against source (line refs below). Root cause confirmed to be a roadmap gap, not only an implementation defect. No fix applied; the allocation-ownership decision was surfaced to the user and pinned via the architect (see § Resolution).

**Scope reviewed:** `answer.rs`, `name_index.rs` (+ `frontend_addr_allocator.rs` from 01-04 as the dependency), `dns_answer_for.rs`, `dns_name_index.rs`.

---

## praise: what's genuinely well done

- **`FrontendAddrAllocator` (01-04) is clean** — smallest-free scan separated from the atomic held-map wrapper; `assign` is a single locked check-and-act with no TOCTOU (`frontend_addr_allocator.rs:208-221`); reserved-endpoint discipline pinned by pure tests; exhaustion refuses rather than reuses.
- **The List-then-Watch skeleton faithfully mirrors `ServiceBackendsResolve`** — single-owner drain, idempotent re-probe under the lock (`name_index.rs:341-357`), `Drop` aborts the task, no lock held across `.await`.
- **OQ-1 was handled correctly** — the `job_of` local parse helper (`name_index.rs:94-110`) with `id.rs` untouched is one of the two sanctioned shapes; the decision + rationale is recorded in the module rustdoc and commit body. The *good* counterexample to the blocker below.

---

## issue (blocking): `answer_for` is not pure — it mutates the single-source-of-truth allocator on the read path

**Mechanism (verified against source):**
- `answer.rs:14-20,35-43` claim `answer_for` "performs NO I/O … trivially deterministic and DST-replayable" and reads the index "ONLY through its public `frontend_for` query."
- `answer_for` → `frontend_for` (`name_index.rs:262`) → `self.allocator.assign(name).ok()` → `assign` **inserts** `<job> → F` into the shared `Arc<Mutex<BTreeMap>>` (`frontend_addr_allocator.rs:219`) whenever the `<job>` is not already held.

**Consequences:**
1. The purity claim is false — a first resolvable query for a not-yet-bound `<job>` *allocates* an address as a side effect.
2. Not a function of `(name, qtype, index)` alone — `smallest_free_addr` depends on what is already held, so the `F` a fresh `<job>` receives depends on the *order DNS queries arrive in*, contradicting "deterministic / DST-replayable."
3. The index has become a **second writer** to the allocator, coupling assignment to health/resolvability — at odds with the allocator's own contract (`frontend_addr_allocator.rs:19-34`: keyed by logical `<job>`, assigned on declaration, released only on logical deletion, no health state). S-DBN-IDX-04 and the pinned-surface clause say the index **consumes** the binding and "the binding is the allocator's, **not the index's**" — read semantics.
4. Downstream: a declared-but-never-dialed healthy `<job>` gets no `F`, so 02-00's `by_frontend` re-key (reads the *same* allocator) cannot translate it. Allocation lifecycle becomes driven by DNS traffic.

**Why this is the CLAUDE.md precedent, not a nitpick:** the allocator exposed only `assign` (mutate), `release` (mutate), `snapshot` (read-all) — there was **no read-only "give me F for `<job>` if already bound" accessor**. The crafter hit that gap and reached for `assign` because it is the nearest thing that compiles, instead of surfacing it as a decision/BLOCKER. Exactly the "implement to the design — never invent API surface" failure the repo rule exists to prevent — and it slipped through because the tests below cannot see it.

**Corrective:** make `frontend_for` a true read — `self.allocator.snapshot().get(name).copied()` — and escalate the allocation-ownership gap (below) rather than papering it with assign-on-read.

## issue (blocking): the acceptance tests cannot distinguish read-from vs assign-into — S-DBN-IDX-04 is unverified

- `records_of` (`dns_name_index.rs:101-104`) and every pre-assert (`dns_name_index.rs:102,137,169,217,407,440`) themselves call `allocator.assign(&name)`. Because `assign` is idempotent, `answer_for(...) == records_of(...)` passes whether `answer_for` *reads* the binding the test created or *creates* its own.
- **No test asserts the allocator's `snapshot()` is unchanged across a resolvable query** for a `<job>` the test did not pre-assign. A mutant deleting the `assign` call inside `frontend_for` survives the whole suite — so the **100% mutation kill-rate is false confidence here**: mutants flip operators/returns, never the read-vs-write semantic. The "single source / no second source" property the step exists to guarantee is the one property the suite does not pin.
- **Corrective:** add a pre/post-`snapshot()` equality assertion around a resolvable query whose `<job>` the test did *not* pre-assign. That test fails today — which is the point.

## issue (blocking): fail-stale instead of fail-closed — divergence from the precedent it claims to mirror

- The sibling `ServiceBackendsResolve::spawn_drain` sets a `watch_healthy = false` faulted flag on relist failure / watch death so `resolve` returns `StoreUnreadable` (`mtls_resolve_adapter.rs:~407-448`). **Fail-closed.**
- `NameIndex::spawn_drain` on relist failure just `return`s with **no faulted state** (`name_index.rs:312-314`), and the drain also exits silently on stream end. `frontend_for` then keeps answering `Records(vec![F])` from the last-known resolvability set indefinitely.
- For a DNS responder this is the wrong posture — it serves liveness answers from a signal it has stopped updating, the precise stale-answer hazard the stable-`F` design fights, and it contradicts the pinned-surface clause's "mirroring `mtls_resolve_adapter.rs:307-471`."
- **Corrective:** mirror the faulted flag; `frontend_for` returns `None` (withhold) when faulted.

---

## Root cause (orchestrator finding): the roadmap names every reader of `FrontendAddrAllocator` but no production writer

REV-2 wired all readers — 01-03 (index), 02-00 (`by_frontend`), 02-01 (construct + inject) — but **never named the actor that calls `assign(<job>)` on declaration / `release(<job>)` on logical deletion**. Grep of `roadmap.json`: `assign`/`release` appear only in 01-04's own Tier-1 tests and in passive-voice property descriptions (S-DBN-WS-STABLE, S-DBN-NXDOMAIN-02 — "the allocator's idempotent assign('server')…"); 02-01's composition criterion is "**construct** … and **inject**", never "call `assign` on deploy"; the FRONTEND-01..04 → 01-04 mapping shows no other step owns the lifecycle. The crafter's assign-on-read is the *symptom*; the missing writer step is the *disease*.

---

## Non-blocking

- **suggestion:** `apply_row` shared-addr eviction (`name_index.rs:149-166`) — if two distinct `service_id`s contribute the same `SocketAddr` to the same `<job>`, evicting one service's stale set (`addrs.remove(addr)`) drops the shared addr and can wrongly empty `by_name[job]` while the other service is still healthy. Low probability (one-service-per-job is the design posture) but unenforced; add a comment or a proptest asserting the one-service-per-job invariant.
- **nitpick:** `relist_into`/`probe` stringify store errors via `.map_err(|e| e.to_string())` (`name_index.rs:282,345`). At a startup-refusing probe boundary it is tolerable, but it is the "flatten typed error to String" shape § "Errors" warns against — a typed `NameIndexError` would let `run_server` branch on cause when 02-01 wires it.
- **question:** RED scaffolds are not visible in the committed test files (only GREEN assertions). RED→GREEN landed in one commit, acceptable for proptest acceptance — but the execution log shows RED_ACCEPTANCE then GREEN both PASS; confirm the RED phase genuinely failed for the right reason before GREEN.
- **praise:** vertical-slice honesty is clean — `mod.rs:27-31` is explicit that `responder.rs` (the socket loop) and run_server wiring are later slices; the step does not overclaim production-drive.

---

## Resolution (decided 2026-06-25)

Ownership path **(a) — a deploy-time lifecycle assigner** (assign on `<job>` declaration, release on logical deletion), validated and pinned by `@nw-solution-architect`, landing as a named roadmap step before 02-02 (the walking skeleton requires `F` bound on deploy). 01-03 re-scoped to a **pure read seam**. The three code correctives above are then applied as a 01-03 follow-up (read-only `frontend_for`, the `snapshot()`-unchanged test, the fail-closed faulted flag) once ownership is pinned. Order is load-bearing: ownership first, then code — do not "fix it green" by keeping assign-on-read.
