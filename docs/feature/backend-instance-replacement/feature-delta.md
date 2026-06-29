# Feature-Delta — backend-instance-replacement (DISCUSS · DRAFT)

> **Status: DISCUSS authored (2026-06-29).** The single authoritative DISCUSS
> narrative for this feature — the compact `feature-delta.md` form mandated by the
> `nw-discuss` Outputs contract + `validate_feature_layout.py`; the legacy split
> `discuss/*.md` files (user-stories, story-map, dor-validation, outcome-kpis,
> wave-decisions) are intentionally **not** produced, their content lives here.
> Lean density, Tier-1 `[REF]` sections. Produced by Luna (nw-product-owner) on
> 2026-06-29 for `backend-instance-replacement` (GH #249). Slice briefs under
> `slices/`. **The mechanism is DECIDED in DISCUSS (operator-ratified 2026-06-29): an
> explicit lifecycle verb; `overdrive deploy` stays pure-declare. The verb's exact
> name + semantics is the OPEN part — a hard DESIGN gate. See `[D1]`.**

## Reading checklist

- ✓ `docs/product/jobs.yaml` — J-OPS-003 (the closest validated lifecycle job: schedule, supervise, converge to declared replica count, restart-on-crash, stop-cleanly); J-OPS-002 / J-OPS-004 (sibling control-surface jobs); J-MESH-001 (the dial-by-name arc whose three deferred ATs — 02-02 ×2 + 03-01 ×1 — this feature unblocks); header precedent ("JTBD distilled from whitepaper/issue, not interviews")
- ✓ `docs/product/personas/ana-platform-engineer.yaml` — Ana Moreno, the lifecycle/ops operator who reasons in intent-vs-actual state and treats `alloc status: Running` as a promise
- ✓ `docs/product/journeys/dial-a-mesh-peer-by-name.yaml` — the parent reachability journey; its STABLE/CHURN cycle step is what this feature makes operable (extension home decided below)
- ✓ `docs/product/journeys/submit-a-job.yaml` — the lifecycle-verb vocabulary precedent (`overdrive deploy`, `overdrive job stop`, `overdrive alloc status`); the `alloc status` honesty contract
- ✓ `docs/product/vision.md` — design principle 6 ("the platform learns / self-healing"); §18 reconciler-converges-to-declared-intent framing
- ✓ `crates/overdrive-control-plane/src/handlers.rs::stop_workload` (~795) — `IntentKey::for_workload_stop` written via `put_if_absent`; key existence IS the stop signal; KEEPS the `jobs/<id>` intent
- ✓ `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs` (~460-522) — the load-bearing `is_operator_stopped` short-circuit (OVERRIDING) vs the `is_intentionally_stopped`/SystemGc filter (OVERRIDABLE) asymmetry
- ✓ `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs` (~1685, ~1855) — two of the three `#[ignore = "...#249..."]` oracle ATs (S-DBN-WS-STABLE, S-DBN-CHURN)
- ✓ `crates/overdrive-control-plane/tests/integration/dns_responder_nxdomain.rs` (~1068) — the THIRD #249-blocked AT: the S-DBN-NXDOMAIN-02 RECOVERY leg (`recovered_job_after_stop_resolves_to_the_same_stable_frontend`), the withhold-not-release Tier-3 `getent` recovery observable. All three are the Tier-3 oracle this feature lands against

---

## `[REF]` Persona

**Ana Moreno — Overdrive platform engineer** (`docs/product/personas/ana-platform-engineer.yaml`).
Years of DevOps/SRE/platform-engineering background; she reasons in **intent vs
actual state** and treats `overdrive alloc status: Running` as a *promise* the
workload is serving, not just that a process started. For this feature her lens is
**lifecycle/ops** (NOT the security lens the dial-by-name journey uses for Sam): she
declares a workload, and when she needs a *fresh instance* of it — a new process,
new identity, new `workload_addr` — while keeping the workload *declared*, she
expects a single, honest, observable operator action that does exactly that.

Her frustration to avoid (directly from her persona record, generalised off UDP):
*a service reported `Running` she cannot trust, and a diagnosis that requires reading
source instead of `overdrive alloc status`*. Here the analogue is: **she stops a
workload to bring up a fresh instance, re-runs the same deploy, and nothing comes
back up — with no error that tells her why or what to run instead.** That silent
dead-end is the pain this feature removes.

> This feature is authored through Ana's lens per the codebase's
> single-persona-per-feature precedent. The dial-by-name journey (whose three deferred
> #249 ATs this unblocks) is authored through Sam's *security* lens; the act of *cycling an
> instance* is an *operator/lifecycle* act, so Ana is the correct primary persona
> here (D3-lightweight: happy path + the two key error paths, no rich emotional arc).

---

## `[REF]` JTBD — one-liner + job-tracing decision

**One-liner (the job this feature serves):** *When a declared workload's current
instance must be replaced — I need a fresh process, a fresh allocation identity, and
a fresh `workload_addr`, but the workload itself must stay declared and reachable — I
want one honest operator action that ends the current instance and brings up a new
one, so I can roll an instance without withdrawing the workload's intent and without
the platform silently refusing to bring anything back up.*

### Job-tracing decision: EXTEND `J-OPS-003` (do NOT mint a new sibling job)

**Decision: EXTEND J-OPS-003** ("Run my actual workload on the walking-skeleton
control plane and trust the platform to converge to the declared replica count …
including restarting on crash and stopping cleanly on `overdrive job stop`").

**Rationale (one line):** replacing an instance is the **same convergence job at a
finer granularity** — the *same progress* ("the platform converges declared intent
into running workloads"), the *same actor-circumstance* (Ana running her actual
workload on the single-node control plane), and the *same failure mode class*
(declared intent NOT converged into a running instance). It is the **third lifecycle
transition** of the same job — alongside the restart-on-crash and stop-cleanly
transitions J-OPS-003 already names — not a new job.

This follows the **udp-sendmsg4 elevation precedent** (jobs.yaml changelog
2026-06-05: "elevate under an existing job rather than mint a new one … the SAME
reachability job at finer granularity — same progress, same failure mode"), NOT the
J-SEC-002 mint precedent (which was justified by a *genuinely distinct* progress +
failure mode). The four-forces analysis below confirms the fit: the push/pull/anxiety
all live inside J-OPS-003's "converge to declared intent, trust what the CLI tells
me" frame.

> **Why not mint a new sibling job (e.g. J-OPS-005 "replace an instance")?**
> Considered and rejected. A "replace-after-stop" job would have the *same* outcome
> statement as J-OPS-003 ("the §18 reconciler moves declared intent into running
> workloads … `alloc status` honestly reflects state"). Minting it would fragment
> J-OPS-003 the way the udp-sendmsg4 analysis warned a per-idiom job would fragment
> J-OPS-004. The honest home is an **extension of J-OPS-003**, recorded as a
> changelog amendment + an enriched outcome clause naming the third (replace)
> transition — NOT a new top-level entry.

### Four forces (confirming the J-OPS-003 fit)

- **Push (frustration driving Ana here):** She stops a workload to roll it (deploy a
  fresh build, recover from a wedged instance, force a new `workload_addr`), re-runs
  the same `overdrive deploy`, and **nothing comes back up** — the operator-stop
  sentinel is sticky and overriding, the same-spec resubmit takes the
  `put_if_absent → KeyExists → Unchanged` path, and the `WorkloadLifecycle`
  reconciler deliberately refuses to schedule a replacement. There is *no production
  verb* that clears the sentinel. She is stuck, with no error explaining why.
- **Pull (the outcome she's reaching for):** One honest action that ends the current
  instance (`A1`, old `workload_addr`) and brings up a **new** instance (`A2`, new
  `workload_addr`) while `jobs/<id>` stays declared — observable as a **new
  AllocationId** in `overdrive alloc status` and a workload that's reachable again.
- **Anxiety (what could go wrong with the new action):** "Will replacing wipe the
  workload's *declaration* (like deletion #211 does)?" — No: intent stays declared,
  by contract. "Will it disturb the workload's *stable address binding* the mesh
  resolves by name?" — No: the dial-by-name `F`-binding stays byte-stable across the
  cycle (the `FrontendAddrAllocator`'s idempotent `assign` proves this at Tier-1;
  this feature must not regress it). "Could I accidentally replace the *wrong*
  instance / a workload that was already gone?" — the action must be honest about
  no-such-workload (404-shape), not a silent no-op.
- **Habit (the workflow Ana is transitioning from):** Today, on Kubernetes/Nomad she
  reaches for `kubectl rollout restart` / `nomad job restart`, or `kubectl delete pod`
  to force a fresh instance under a still-declared Deployment. Overdrive has the
  declared-intent model (`jobs/<id>` is the Deployment analogue) but **no verb that
  rolls one instance** — the closest, `overdrive job stop` + re-deploy, dead-ends.
  The new action must feel like the rollout-restart she already knows: the workload
  stays declared, a fresh instance comes up.

---

## `[REF]` The gap (the thing this feature closes — grounded, do NOT re-derive)

Overdrive has **no operator/production path to replace a declared workload's backend
instance** — to end the current instance (`A1`, `workload_addr`) and bring up a new
instance (`A2`, new `workload_addr`) while the workload's `jobs/<id>` intent stays
declared. The three existing lifecycle verbs each do something *else*:

| Existing verb / path | What it does | Why it is NOT instance replacement |
|---|---|---|
| `overdrive job stop` (`handlers.rs::stop_workload`) | Writes `IntentKey::for_workload_stop(<id>)` via `put_if_absent` (the key's **existence** IS the stop signal). Drives allocs → Terminated. **KEEPS** the `jobs/<id>` intent. | The stop sentinel is **sticky + OVERRIDING** (`workload_lifecycle.rs:520`, `is_operator_stopped` short-circuit). A same-spec re-deploy takes `put_if_absent → KeyExists → Unchanged` and does **NOT** clear the sentinel → the reconciler refuses to schedule a fresh instance. No verb clears it. |
| Crash-restart (`RestartAllocation`, backoff branch) | Restarts a *failed* alloc. | **Reuses the alloc_id/slot** (same `workload_addr`) — not a *new* instance identity. |
| Deletion (#211) | Withdraws the `jobs/<id>` intent entirely. | The **opposite** of this feature — here the workload must STAY declared. **Distinct from #211.** |

**The load-bearing asymmetry** (`workload_lifecycle.rs:507-519`): a **SystemGc**-stopped
row is OVERRIDABLE — it's filtered out of `active_allocs_vec`, so a fresh submit lands
a fresh placement (architecture.md §5). An **Operator**-stopped row is OVERRIDING — it
short-circuits the Run branch (`is_operator_stopped` → `return (Vec::new(), …)`), so
"a fresh schedule would undo the operator's stop." The reconciler comment is explicit:
the alloc "remains in obs as the terminal state **until the operator explicitly
re-submits the job intent**" — but **no production verb performs that explicit
re-submit-that-clears-the-stop.** That missing verb is exactly what this feature
specifies (and `[D1]` records HOW to deliver it as the open DESIGN decision).

---

## `[REF]` THE DESIGN DECISION (partly DECIDED in DISCUSS; verb-semantics OPEN — **hard gate for DESIGN**)

### `[D1]` An explicit lifecycle verb (`deploy` stays pure-declare); exact verb name + semantics is the OPEN part

The user scoped DISCUSS to "*formalize requirements + the new-verb vs
sentinel-clearing decision, then hand to DESIGN*." DISCUSS makes the *new-verb vs
sentinel-clearing* call and hands the *verb's name + semantics* to DESIGN.

**DECIDED in DISCUSS (operator-ratified 2026-06-29): an explicit lifecycle verb;
`overdrive deploy` stays pure-declare.** The two were never two implementations of one
operation — they are two *different operations*, the same split Kubernetes draws:

| Kubernetes | Overdrive | semantics |
|---|---|---|
| `kubectl apply` | `overdrive deploy <spec>` | **declare desired intent** — idempotent; same spec is a no-op (`put_if_absent → KeyExists → Unchanged`) and does NOT touch the stop sentinel. |
| `kubectl rollout restart` | the new lifecycle verb (`[D1]`) | **force the instance to be replaced by a fresh one**, intent unchanged. A distinct, named action. |

The rejected candidate — *overload a same-spec `overdrive deploy` to clear the
`for_workload_stop` sentinel and re-provision* — conflates *declare-intent* with a
*lifecycle side-effect*, and is ruled out for two concrete reasons:
- **It breaks `deploy`'s honest same-spec = no-op contract.** A routine re-deploy would
  silently gain the power to un-stop a deliberately-stopped workload — a same-spec
  `deploy` that is sometimes a no-op and sometimes a side-effecting restart depending on
  hidden stop-sentinel state.
- **It undermines the sticky-stop design.** `job stop` is sticky/overriding *by design*
  (ADR-0037 Amdt / §Bug 3); a re-apply must not quietly override an operator's stop.

**OPEN for DESIGN (the verb's name + semantics) — this is the hard gate:** the gap is
that `job stop` is effectively a *suspend* (intent retained + a sticky sentinel) with no
`start`/`resume`/`restart` counterpart, and all three oracle ATs `stop_and_converge`
*first* and then need a fresh instance — i.e. the operation is **restart-after-stop /
resume**, not a `rollout restart` of a *running* set. DESIGN decides:
- Is it `overdrive job restart <id>` with `kubectl rollout restart` breadth (works whether
  the workload is stopped *or* running — if running, stop-then-start; if stopped, just
  start)? **OR** a `start`/`resume` (un-suspend a stopped workload), possibly paired with a
  separate `restart` for cycling a *running* instance?
- The exact CLI verb shape, the HTTP path/handler, and the **response/output shape** the
  operator observes.
- The sentinel-clearing mechanics — TOCTOU-safe per `development.md` § "Check-and-act must
  be atomic" (clearing a sentinel + re-provisioning must not race a converging stop).
- DESIGN writes the ADR. DISCUSS does NOT (architect-agent territory).

**HARD GATE (reviewer Finding 3, 2026-06-29):** the concrete verb name + HTTP surface +
response shape MUST be chosen in DESIGN **before** DISTILL/DELIVER writes executable
acceptance against it. US-BIR-1/2 ACs are framed against the observable *outcome* (new
AllocationId, intent retained, `F` byte-stable, honest 404); they cannot be made
executable until `[D1]`'s verb surface is pinned. DESIGN must close `[D1]` first.

**The invariant the verb MUST satisfy (the requirement, fixed regardless of name/shape):**
1. The cycle/recovery yields a **NEW AllocationId** (`A1 ≠ A2`) and a **NEW `workload_addr`**.
2. The `jobs/<id>` **intent stays declared** throughout (NOT withdrawn — distinct from #211).
3. The operator-stop sentinel (`IntentKey::for_workload_stop`) ends up **cleared**, so `WorkloadLifecycle` provisions a fresh instance (it stops short-circuiting on `is_operator_stopped`).
4. The dial-by-name **`F`-binding stays byte-stable** across the cycle (the `FrontendAddrAllocator`'s idempotent `assign("<job>")` — withhold-not-release; `F` is per-logical-workload). Already proven at Tier-1; the feature MUST NOT regress it.
5. The action is **honest about a non-existent workload** (no `jobs/<id>` row → a 404-shape error, the same posture `stop_workload` takes), not a silent no-op.
6. `overdrive deploy` **remains pure-declare** — the new verb does NOT live on `deploy`.

---

## `[REF]` User stories

> All stories trace to **J-OPS-003** (extended). Each non-`@infrastructure` story
> carries an Elevator Pitch whose "After" is a real operator entry point. **The
> mechanism — an explicit lifecycle verb, NOT overloaded `deploy` — is decided; the
> exact verb name + semantics is design-open (`[D1]`)** — every Elevator Pitch frames the
> operator-observable OUTCOME (new AllocationId, `F` stable, intent retained) and
> annotates the verb name as provisional. ACs are embedded + testable.

### US-BIR-1 — Replace a declared workload's instance: new instance, intent retained

**Job:** `J-OPS-003`

**Elevator Pitch:**
- **Before:** Ana stops a declared workload to roll it (new build / wedged instance / force a fresh `workload_addr`), re-runs `overdrive deploy <spec>`, and **nothing comes back up** — the operator-stop sentinel is sticky and overriding, the same-spec resubmit is `Unchanged`, and `WorkloadLifecycle` refuses to schedule a replacement. She is stuck with no path forward and no error explaining why.
- **After:** Ana runs the replace action *(an explicit lifecycle verb — name design-open `[D1]`, e.g. `overdrive job restart <id>`; NOT `deploy`)* → the current instance ends and a **new instance comes up**, observable as a **NEW AllocationId** in `overdrive alloc status --job <id>` (`A1 ≠ A2`, new `workload_addr`) while the `jobs/<id>` intent stays declared.
- **Decision enabled:** Ana can roll one instance of a still-declared workload with a single honest action — and *confirm* it worked by reading the new AllocationId, the same way she trusts `alloc status` for every other lifecycle state.

**Problem:** Ana needs a fresh instance of a workload that must stay declared. The
platform has the declared-intent model but no verb that rolls one instance — the
closest path (`stop` + re-deploy) dead-ends on the sticky operator-stop sentinel.

**Solution (behavior; DESIGN owns the API + mechanism `[D1]`):** An operator action
that, for a declared `jobs/<id>` whose current instance is operator-stopped (or
running), ends the current instance and clears the operator-stop sentinel so
`WorkloadLifecycle` provisions a **fresh** instance — NEW AllocationId, NEW
`workload_addr` — while `jobs/<id>` stays declared. Single-node, Phase 2.

**Domain Examples:**
1. **Happy path (stopped → replaced):** Ana deploys `payments` (replicas=1), which reaches Running as alloc `payments-0` with `workload_addr 10.99.0.2`. She runs `overdrive job stop payments` (converges to Terminated). She runs the replace action on `payments` → a NEW alloc `payments-1` reaches Running with `workload_addr 10.99.0.6`; `overdrive alloc status --job payments` shows the new AllocationId; `jobs/payments` is still declared.
2. **Happy path (running → replaced):** Ana deploys `coinflip` (Running, alloc `coinflip-0`). Without stopping first, she runs the replace action → the current instance is ended and a NEW alloc `coinflip-1` reaches Running with a fresh `workload_addr`; intent retained.
3. **Error (no such workload):** Ana runs the replace action on `nonexistent` (never deployed; no `jobs/nonexistent` row) → an honest not-found error (404-shape, same posture as `overdrive job stop` on an unknown id), NOT a silent no-op and NOT a spurious fresh placement.

**UAT Scenarios (BDD):**

```gherkin
Scenario: Replacing a stopped workload brings up a new instance with intent retained
  Given Ana has deployed "payments" and it reached Running as allocation "payments-0"
  And Ana has run "overdrive job stop payments" and it converged to Terminated
  When Ana runs the replace action on "payments"
  Then a new allocation with a different AllocationId reaches Running
  And "overdrive alloc status --job payments" shows the new AllocationId
  And the jobs/payments intent is still declared

Scenario: Replacing a running workload cycles to a fresh instance
  Given Ana has deployed "coinflip" and it reached Running as allocation "coinflip-0"
  When Ana runs the replace action on "coinflip"
  Then the current instance ends and a new instance reaches Running with a different AllocationId and a different workload_addr
  And the jobs/coinflip intent is still declared

Scenario: Replacing a non-existent workload is rejected, not silently ignored
  Given there is no declared workload named "nonexistent"
  When Ana runs the replace action on "nonexistent"
  Then the action is rejected with a not-found error
  And no allocation is created
```

**Acceptance Criteria:**
- [ ] Driven through production entry points (`overdrive serve` + the operator action, `[D1]`) — NOT a hand-rolled harness. No test installs/clears an intent key, binds a socket, or supplies an address that production does not itself install/clear/bind/supply (CLAUDE.md vertical-slice rule).
- [ ] After the replace action on a stopped-or-running declared workload, a NEW allocation reaches Running with an AllocationId distinct from the prior one (`A1 ≠ A2`) and a distinct `workload_addr`.
- [ ] The `jobs/<id>` intent row remains present (declared) before AND after the action (distinct from #211 deletion).
- [ ] The operator-stop sentinel (`IntentKey::for_workload_stop(<id>)`) is cleared by the action, so `WorkloadLifecycle` stops short-circuiting on `is_operator_stopped` and provisions a fresh instance.
- [ ] A replace action against a workload with no `jobs/<id>` row is rejected with a not-found (404-shape) error; no allocation is created.

### US-BIR-2 — The stable address binding survives the cycle (no stale address)

**Job:** `J-OPS-003`

**Elevator Pitch:**
- **Before:** When Ana cycles an instance, there is a risk the workload's stable address binding (the `F` the mesh resolves `<job>.svc.overdrive.local` to) churns — so a peer that resolved `F` before the cycle is left pointing at a dead/stale address, or the new instance comes up behind a *different* `F`.
- **After:** Ana runs the replace action *(verb design-open `[D1]`)* → `<job>.svc.overdrive.local` re-resolves to the **same byte-identical `F`** across the cycle, and the next connection to that `F` lands the **NEW** backend instance — observable as a successful byte-exact round-trip to the fresh instance through the same `F`.
- **Decision enabled:** Ana can roll an instance without breaking the mesh's name-based reachability — peers keep dialing the same name/address and transparently land the fresh instance (no stale cached address, the SQ1-elimination guarantee).

**Problem:** A cycle that churns `F` (or strands peers on a stale address) breaks the
dial-by-name contract. The `FrontendAddrAllocator`'s idempotent `assign` already
guarantees `F`-stability at Tier-1; the replace action must not regress it, and the
re-keyed `MtlsResolve` must re-resolve the live backend per-connect.

**Solution (behavior; DESIGN owns the API):** The replace action retains the
per-logical-workload `F`-binding across the cycle (the allocator's idempotent
`assign("<job>")` — withhold-not-release), and the re-keyed `MtlsResolve` translates
`F` → the NEW live backend per-connect, so the next connect lands `A2`.

**Domain Examples:**
1. **Stable `F`, new backend:** `server` is Running behind stable frontend `F1 ∈ 10.98.0.0/16`, backend `B1 ∈ 10.99.0.0/16`. Ana cycles `server` (replace action). `getaddrinfo("server.svc.overdrive.local")` re-resolves to the **same** `F1` byte-for-byte; the next connect to `F1` lands the NEW backend `B2 ≠ B1` (byte-exact mTLS round-trip to the fresh instance).
2. **`F` never a backend addr:** across the whole cycle the resolved value is always `F1 ∈ 10.98.0.0/16`, never a per-instance backend addr `∈ 10.99.0.0/16` (neither `B1` nor `B2`).
3. **In-flight fail-fast (churn boundary):** a client holds an open in-flight connection to `B1`; Ana cycles `server` mid-connection → the in-flight connection gets a PROMPT reset/error bounded by `TCP_USER_TIMEOUT` (never an indefinite hang; NO `sock_destroy` — that's #61 scope), and a subsequent fresh connect to `F1` lands the new live backend `B2`.

**UAT Scenarios (BDD):**

```gherkin
Scenario: The name re-resolves to the same stable address across the cycle
  Given "server" is Running behind stable frontend F1 with backend instance B1
  And a connect to F1 lands B1 with a byte-exact round-trip
  When Ana runs the replace action on "server" and a new instance B2 reaches Running
  Then getaddrinfo("server.svc.overdrive.local") re-resolves to the same F1 byte-for-byte
  And the next connect to F1 lands the new backend B2 with a byte-exact round-trip
  And F1 was always a stable frontend, never a per-instance backend address

Scenario: An in-flight connection fails fast on backend churn, then the next dial is live
  Given a client holds an open in-flight connection through F1 to backend B1
  When Ana cycles "server" mid-connection
  Then the in-flight connection gets a prompt reset or error bounded by TCP_USER_TIMEOUT, never an indefinite hang
  And a subsequent fresh connect to F1 lands the new live backend B2
```

**Acceptance Criteria:**
- [ ] After the replace action, `getaddrinfo("<job>.svc.overdrive.local")` re-resolves to the **same `F` byte-for-byte** as before the cycle (the allocator's idempotent `assign` retained `F`).
- [ ] The next connect to that `F` lands the NEW backend instance `B2` (byte-exact round-trip to the fresh instance; the re-keyed `MtlsResolve` re-resolved the live backend per-connect).
- [ ] The resolved value is always a stable frontend `∈ 10.98.0.0/16`, never a per-instance backend addr `∈ 10.99.0.0/16` (neither `B1` nor `B2`).
- [ ] An in-flight connection to the old instance, when its backend is cycled mid-connection, fails fast (reset/error/EOF) bounded by `TCP_USER_TIMEOUT` — never an indefinite hang; NO `sock_destroy` (#61 scope).
- [ ] Proven through `overdrive serve` + `overdrive deploy` + the replace action (Tier-3), consistent with the dial-by-name intercept path — no second source of backend truth.

## `[REF]` Verification plan / terminal quality gate — the Tier-3 oracle (proves US-BIR-1 + US-BIR-2)

> **NOT a user story.** (Removed as US-BIR-3 per review 2026-06-29 — "land three tests
> green" is a CI-runner outcome, not a user-invocable operator action, so it fails the
> Elevator-Pitch operator-outcome bar.) The three #249-blocked acceptance tests are the
> **mandatory oracle evidence** that proves US-BIR-1 (new instance, intent retained) and
> US-BIR-2 (stable `F` across the cycle, in-flight churn fail-fast, recovery) on the
> **production path**. They are the feature's **terminal quality gate / Definition of
> Done**, folded into US-BIR-1 + US-BIR-2 — not a standalone story.

**The oracle — three `#[ignore = "…#249…"]` ATs across two files, un-ignored only once the
production replace action exists:**

| AT | File | Proves (for) |
|---|---|---|
| `answered_frontend_is_byte_stable_across_alloc_cycle_next_connect_lands_new_backend` (**S-DBN-WS-STABLE**) | `dns_responder_walking_skeleton.rs` | new AllocationId + `F` byte-stable across the cycle + next connect lands `B2` (US-BIR-1 + US-BIR-2) |
| `in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend` (**S-DBN-CHURN**) | `dns_responder_walking_skeleton.rs` | in-flight churn fails fast (`TCP_USER_TIMEOUT`-bounded), next dial lands `B2` (US-BIR-2) |
| `recovered_job_after_stop_resolves_to_the_same_stable_frontend` (**S-DBN-NXDOMAIN-02 RECOVERY**) | `dns_responder_nxdomain.rs` | a stopped `<job>` re-resolves the SAME `F` once recovered — withhold-not-release Tier-3 `getent` observable (US-BIR-1 + US-BIR-2) |

**Gate criteria (terminal DoD for US-BIR-1 + US-BIR-2):**
- All **three** `#[ignore = "…#249…"]` attributes removed (strings *removed*, not rewritten — no stale forward-pointer).
- All three cycle/recover via the **production replace action** (`[D1]`), NOT a test-only intent-key clear or a `stop_and_converge`-then-same-spec-redeploy that would dead-end.
- All three GREEN on the pinned-6.18 appliance-kernel Tier-3 matrix (the merge gate; dev-Lima is necessary-but-not-sufficient).
- No AT installs/clears a rule/key, binds a socket, or supplies an address production does not itself install/clear/bind/supply (CLAUDE.md vertical-slice rule).

**Gate scenarios (the oracle, not user-story UAT):**

```gherkin
Scenario: byte-stable-across-cycle oracle passes un-ignored
  Given the replace action is landed in production
  When the S-DBN-WS-STABLE acceptance test runs un-ignored on the pinned-6.18 Tier-3 matrix
  Then the cycle produces a new AllocationId, F re-resolves byte-identical, and the post-cycle dial lands the new live backend with a byte-exact round-trip
  And the inter-agent leg-B↔leg-C hop carries TLS 1.3 application_data records

Scenario: backend-churn fail-fast oracle passes un-ignored
  Given the replace action is landed in production
  When the S-DBN-CHURN acceptance test runs un-ignored on the pinned-6.18 Tier-3 matrix
  Then the in-flight connection fails fast bounded by TCP_USER_TIMEOUT
  And a subsequent fresh connect to F lands the new live backend B2

Scenario: withhold-not-release recovery oracle passes un-ignored
  Given the replace action is landed in production
  And the server <job> was stopped (its name resolves NXDOMAIN while stopped)
  When the S-DBN-NXDOMAIN-02 RECOVERY acceptance test recovers the SAME <job> to Running-AND-HEALTHY via the replace action on the pinned-6.18 Tier-3 matrix
  Then getent re-resolves the SAME stable F byte-for-byte (the allocator withheld, did not release, F across the stop)
```

---

## `[REF]` System Constraints (cross-cutting)

- **Single-node, Phase 2.** No multi-node, no cross-node concerns. One node's workloads (the dial-by-name arc is single-node).
- **Intent stays declared.** The replace action MUST NOT withdraw the `jobs/<id>` intent. **Distinct from deletion (#211)**, which is the opposite operation. This feature MUST NOT duplicate or depend on #211.
- **New instance identity.** The cycle yields a NEW AllocationId and NEW `workload_addr` — NOT a slot/alloc_id reuse (distinct from crash-restart `RestartAllocation`).
- **`F`-binding byte-stability is a guardrail, not a goal.** The `FrontendAddrAllocator`'s idempotent `assign` already proves it at Tier-1; the feature MUST NOT regress it. The re-keyed `MtlsResolve` re-resolves the live backend per-connect.
- **TOCTOU-safe sentinel mechanics.** Clearing `for_workload_stop` + re-provisioning must be atomic against a converging stop (`development.md` § "Check-and-act must be atomic"). DESIGN owns the exact mechanism.
- **Implement-to-design.** Behavior + the gap + the pinned invariant only. The replace verb's API surface, the CLI verb shape, the intent-key clearing mechanics, and the reconciler edit are DESIGN-wave decisions. Surface gaps as blockers; never improvise API (CLAUDE.md).
- **Vertical slices through production entry points.** Every slice closes a real loop through `overdrive serve` + the replace action (+ `overdrive deploy`). No slice ships if it only composes in a `#[test]` (CLAUDE.md).
- **NO `sock_destroy`.** In-flight connection teardown on churn is via the terminating-proxy posture (`TCP_USER_TIMEOUT`/keepalive on the worker proxy legs); `sock_destroy` is #61 scope.

---

## `[REF]` Definition of Done

- US-BIR-1: the replace action exists (mechanism per `[D1]`), ends the current instance, clears the operator-stop sentinel, and brings up a NEW instance with `jobs/<id>` retained — proven through `overdrive serve` + the action.
- US-BIR-2: `F` re-resolves byte-identical across the cycle; the next connect lands the new live backend; in-flight churn fails fast — proven Tier-3.
- **Terminal quality gate (proves US-BIR-1 + US-BIR-2):** all three `#[ignore = "…#249…"]` ATs (S-DBN-WS-STABLE, S-DBN-CHURN, S-DBN-NXDOMAIN-02 RECOVERY) un-ignored and GREEN on the pinned-6.18 Tier-3 matrix, driving the production replace action with no test-only wiring. See § "Verification plan / terminal quality gate".
- `[D1]`'s mechanism is decided (explicit lifecycle verb; `deploy` stays pure-declare); the verb name + semantics + the invariant are handed to DESIGN as a hard gate; DESIGN writes the ADR.
- Both stories (US-BIR-1, US-BIR-2) DoR-passing and tracing to J-OPS-003 (extended).

---

## `[REF]` Out of scope (cite existing issues only)

- **Workload deletion / intent withdrawal** — **#211**. The opposite operation; intent stays declared here. This feature MUST NOT duplicate or depend on it.
- **Crash-restart / `RestartAllocation`** — already shipped; reuses the alloc_id/slot, NOT a new instance identity. This feature is distinct.
- **`sock_destroy` / forcibly tearing down in-flight kernel sockets on churn** — **#61** (VIP path). In-flight churn here is the terminating-proxy fail-fast posture only.
- **The dial-by-name responder / `MtlsResolve` / intercept substrate itself** — shipped by the dial-by-name arc (#243) + transparent-mtls arc (#26/#236). This feature is a *consumer* that cycles an instance; it MUST NOT duplicate the resolve index or the intercept path.
- **Multi-node / cross-node instance replacement** — OUT of Phase-2 single-node scope.
- **Rolling/zero-downtime replacement (drain-old-then-cut-over, no live-instance gap)** — **#253**. Not in v1 scope; v1 is end-then-bring-up (single replica), which matches the oracle ATs.
- **Multi-replica (`replicas > 1`) replacement semantics (replace-all vs replace-one)** — **#254**. v1 covers `replicas = 1` (the oracle ATs).

---

## `[REF]` Walking-skeleton strategy

**No new skeleton** (D2 — brownfield). This feature unblocks the EXISTING dial-by-name
arc's three deferred ATs (02-02 walking skeleton: S-DBN-WS-STABLE + S-DBN-CHURN; 03-01
NXDOMAIN: the S-DBN-NXDOMAIN-02 RECOVERY leg). The
thinnest production loop the feature's own work closes: `overdrive serve` (one node) +
`overdrive deploy <spec>` (instance `A1` reaches Running) + the replace action `[D1]`
→ `A1` ends, the operator-stop sentinel is cleared, `WorkloadLifecycle` provisions
`A2` (NEW AllocationId, NEW `workload_addr`), `jobs/<id>` stays declared. Carpaccio
slicing applies to the feature's own work (see slice briefs); the terminal slice
un-ignores all three ATs (the Tier-3 oracle, across both `dns_responder_walking_skeleton.rs` and `dns_responder_nxdomain.rs`).

---

## `[REF]` Driving ports (for DESIGN — named, not designed)

- The **replace operator action** — the new production entry point (verb shape + HTTP path + handler are DESIGN decisions; `[D1]` is the candidate set).
- The **`IntentKey::for_workload_stop` clearing mechanism** — a production path that clears the sentinel atomically against a converging stop (DESIGN owns the mechanics; today only `put_if_absent` writes it, nothing clears it).
- The **`WorkloadLifecycle` reconciler** — `is_operator_stopped` short-circuit (`workload_lifecycle.rs:520`) must stop firing once the sentinel is cleared so a fresh placement lands (DESIGN owns the reconciler edit, if any).
- **`overdrive serve`** (composition root / `run_server`) and **`overdrive deploy <SPEC>`** — the production entry points every slice drives through.

---

## `[REF]` Pre-requisites

- **SHIPPED:** `overdrive job stop` + the `for_workload_stop` sentinel (`handlers.rs::stop_workload`); the `WorkloadLifecycle` operator-stop/SystemGc asymmetry (`workload_lifecycle.rs`); the dial-by-name responder + `FrontendAddrAllocator` idempotent `assign` + re-keyed `MtlsResolve` + intercept path (#243 / #26 / #236); the three `#[ignore]`'d ATs (the oracle — 02-02 ×2 + 03-01 ×1).
- **DESIGN-gated:** `[D1]` verb name + semantics (mechanism already decided: explicit verb, `deploy` pure-declare) + ADR; the TOCTOU-safe sentinel-clearing mechanics.
- **No DIVERGE artifacts** for this feature (`docs/feature/backend-instance-replacement/diverge/` absent) — consistent with the jobs.yaml header precedent (JTBD distilled from issue + grounded code, not interviews). Noted as a non-blocking risk; the parent arcs' DISCUSS/DESIGN history + the grounded reconciler/handler code ground the contracts.

---

## `[REF]` Shared-artifact registry

Registry-grade tracking of every value that must be single-source and consistent
across the replace path, the stop path, and the dial-by-name layer.

| Artifact | Source of truth | Consumers | Owner | Integration risk | Validation |
|---|---|---|---|---|---|
| `operator_stop_sentinel` | `IntentKey::for_workload_stop(<id>)` in the `IntentStore` (existence = stopped; `handlers.rs::stop_workload` writes via `put_if_absent`) | `WorkloadLifecycle.reconcile` (`is_operator_stopped` short-circuit); the replace action (clears it) | the stop path owns the write; **this feature adds the clearing path** | a non-atomic clear racing a converging stop (TOCTOU); a clear that leaves the row half-written → `WorkloadLifecycle` flip-flops | US-BIR-1 AC: after the action the sentinel is cleared and a fresh instance reaches Running; `development.md` atomic check-and-act discipline (DESIGN) |
| `jobs_intent_row` | `IntentKey::for_workload(<id>)` in the `IntentStore` | `stop_workload`'s 404 check; the replace action (must leave it present); `WorkloadLifecycle` | the platform's declared-intent model | the replace action accidentally withdrawing intent (→ becomes #211 deletion) | US-BIR-1 AC: `jobs/<id>` present before AND after the action |
| `allocation_id` | the `WorkloadLifecycle` placement (`A1`, `A2`); observed via `ObservationStore` alloc_status rows | `overdrive alloc status`; the ATs (`workload_running_alloc_id`) | the reconciler / scheduler | a cycle that reuses the alloc_id/slot (→ becomes crash-restart, not replacement) | US-BIR-1 AC + terminal oracle: `A1 ≠ A2` observed in alloc_status |
| `workload_addr` | the per-instance `/30` placement (`10.99.0.0/16`) | the netns derivation; the backend the intercept resolves to | the reconciler / veth provisioner | a cycle that reuses `workload_addr` (no fresh instance identity) | US-BIR-1/US-BIR-2 AC: new `workload_addr` after the cycle |
| `stable_frontend_F` | the `FrontendAddrAllocator`'s idempotent `assign("<job>")` (`10.98.0.0/16`, withhold-not-release; per-logical-workload) | the dial-by-name responder's `name_index`; the re-keyed `MtlsResolve.by_frontend`; `getaddrinfo` | the dial-by-name / transparent-mtls arc (shipped) | a replace action that churns `F` or releases it → peers stranded on a stale address (SQ1 regression) | US-BIR-2 AC + terminal oracle: `getaddrinfo` re-resolves the SAME `F` byte-for-byte; `F` never a backend addr |
| `tier3_oracle` | the three `#[ignore]`'d ATs (×2 in `dns_responder_walking_skeleton.rs`: S-DBN-WS-STABLE, S-DBN-CHURN; ×1 in `dns_responder_nxdomain.rs`: S-DBN-NXDOMAIN-02 RECOVERY) | the merge gate (pinned-6.18 Tier-3 matrix) | this feature un-ignores them | the ATs passing via test-only wiring that stands in for the missing verb (CLAUDE.md violation) | Terminal quality gate (US-BIR-1/2): un-ignored + GREEN, driving the production replace action only |

---

## `[REF]` Outcome KPIs (numeric targets + measurement method)

### Objective
*An operator can replace a declared workload's instance with one honest action — a new
instance comes up, the workload stays declared, and the mesh keeps resolving it by
name — with the guarantee mechanically checked on the CI critical path.*

| KPI | Who | Does what (behavior change) | By how much (target) | Measured by | Baseline |
|---|---|---|---|---|---|
| **K-BIR-1 — replace yields a fresh instance, intent retained** | Ana (operator) | Runs the replace action on a declared (stopped-or-running) workload and gets a NEW instance with intent retained | **100%** of replace actions on a declared workload produce a NEW AllocationId (`A1 ≠ A2`) + NEW `workload_addr` reaching Running within the Tier-3 convergence budget (20s), with `jobs/<id>` still present; **0** silent dead-ends | Tier-3: deploy → (stop) → replace; assert new AllocationId in alloc_status, new `workload_addr`, `jobs/<id>` row present | Today: **0%** — no production verb exists; `stop` + re-deploy dead-ends on the sticky sentinel |
| **K-BIR-2 — stable address survives the cycle (no stale address)** | A deployed mesh peer | Re-resolves `<job>.svc.overdrive.local` to the SAME `F` across the cycle and lands the NEW backend | **100%** of post-cycle resolutions return the byte-identical `F`; **100%** of post-cycle connects land the new live backend (`B2`); **0** stale/old-backend addresses returned or landed | Tier-3 (S-DBN-WS-STABLE): assert `f1_again == f1`, post-cycle dial lands `B2` byte-exact, `F ∈ 10.98.0.0/16` never a backend addr | Today: untestable — the AT is `#[ignore]`'d on the missing verb |
| **K-BIR-3 — in-flight churn fails fast, next dial live** | A client with an open in-flight connection | Gets a prompt error on backend churn, then a fresh connect lands the new instance | In-flight failure within **`CHURN_BOUND`** (`TCP_USER_TIMEOUT`-bounded, never an indefinite hang); subsequent fresh connect lands `B2` **100%** of the time | Tier-3 (S-DBN-CHURN): measure elapsed-to-error ≤ `CHURN_BOUND`; assert subsequent fresh connect byte-exact to `B2` | Today: untestable — the AT is `#[ignore]`'d on the missing verb |
| **K-BIR-4 — the oracle is standing, not one-time** | The CI critical path | Catches a replace/stable-address/recovery regression on every PR | All **three** deferred ATs **un-ignored and GREEN** on the pinned-6.18 Tier-3 matrix; **0** test-only wiring standing in for the production verb | Tier-3 merge gate run; review confirms no hand-installed replacement | Today: **0** — all three ATs `#[ignore]`'d, no standing guarantee |

**Metric hierarchy** — North Star: **K-BIR-1** (a fresh instance with intent retained
is the whole job). Leading: K-BIR-2/K-BIR-3 (the cycle is mesh-safe). Guardrail:
**K-BIR-2's `F`-byte-stability** must NOT degrade (the SQ1-elimination guarantee the
allocator already provides). **EDD:** no new EDD expectation proposed in DISCUSS — the
three un-ignored ATs are the standing oracle; DESIGN/DELIVER may graduate an
`O`/`E`-surface expectation if the replace action gains operator-observable CLI output
worth a black-box capture.

---

## `[REF]` DoR validation (9-item hard gate)

| # | Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | ✅ | Each story opens from Ana's pain (`stop` + re-deploy dead-ends; needs a fresh instance with intent retained) in lifecycle/intent-vs-actual vocabulary |
| 2 | User/persona with specific characteristics | ✅ | Ana Moreno (`ana-platform-engineer.yaml`), lifecycle/ops lens, reasons in intent-vs-actual, trusts `alloc status` |
| 3 | 3+ domain examples with real data | ✅ | Each story carries 3 examples with concrete names (`payments`, `coinflip`, `server`/`client`), allocs (`payments-0`/`payments-1`), addrs (`10.99.0.2`/`10.99.0.6`, `F1 ∈ 10.98.0.0/16`, `B1`/`B2 ∈ 10.99.0.0/16`) |
| 4 | UAT in Given/When/Then (3–7 scenarios) | ✅ | 4 operator-outcome scenarios across US-BIR-1/2 (+ 3 oracle gate scenarios in the verification plan, which are evidence, not user-story UAT) |
| 5 | AC derived from UAT | ✅ | Each story's AC list maps to its scenarios |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | ✅ | 2 stories (US-BIR-1, US-BIR-2), each a single behavior, each ≤3 scenarios. Per-slice DELIVER estimates (each ≤1 day): slice-01 ≈1d, slice-02 ≈0.5d, slice-03 ≈0.5d, slice-04 (terminal gate) ≈0.5d → ≈2.5d DELIVER. The ~3–5d feature figure adds the DESIGN-wave `[D1]` resolution (verb + ADR). |
| 7 | Technical notes: constraints/dependencies | ✅ | System Constraints + Pre-requisites + Driving Ports; the grounded gap table; `[D1]` open decision |
| 8 | Dependencies resolved or tracked | ✓ | Resolved/shipped: stop sentinel, reconciler asymmetry, dial-by-name responder + allocator + `MtlsResolve`, the three oracle ATs (02-02 ×2 + 03-01 ×1). **DESIGN-gated:** `[D1]` mechanism + TOCTOU-safe clearing. **Deferrals tracked** (operator-approved 2026-06-29): rolling/zero-downtime → **#253**, multi-replica replace → **#254** |
| 9 | Outcome KPIs with measurable targets | ✅ | K-BIR-1..4 with numeric targets + measurement method + baseline |

**Gate verdict:** **DISCUSS approved → hand to DESIGN.** DoR is met for both stories
(US-BIR-1, US-BIR-2); the terminal verification gate (the Tier-3 oracle) is tracked
separately as the feature's DoD, not as a story. `[D1]` (the mechanism decision) is the explicit OPEN decision DESIGN
resolves — DISCUSS is *complete* with it open by design (that is the scoped
deliverable). Item 8's deferrals were operator-approved (2026-06-29) and tracked as
**#253** (rolling/zero-downtime) and **#254** (multi-replica replace) — they do not
block the DISCUSS hand-off; they scope what v1 of the replace action covers.

---

## `[REF]` Wave-Decisions (DISCUSS)

### Decisions taken
1. **Feature type:** Backend (D1 from orchestrator) — control-plane lifecycle verb + reconciler/intent path.
2. **Walking skeleton:** No new skeleton (D2) — brownfield; unblocks the dial-by-name arc's three deferred ATs (02-02 walking skeleton ×2 + 03-01 NXDOMAIN recovery ×1). Carpaccio slicing applies to the feature's own work.
3. **UX research depth:** Lightweight (D3) — single operator persona Ana, happy path + the two key error paths (no-such-workload; no-stale-address/in-flight churn); no rich emotional arc.
4. **JTBD:** Yes (D4) — operator-facing CLI/deploy surface, NOT infrastructure-only. **EXTEND J-OPS-003** (the same convergence job at finer granularity — the third lifecycle transition), NOT a new sibling job (udp-sendmsg4 elevation precedent, NOT the J-SEC-002 mint precedent). Persona = Ana.
5. **Mechanism `[D1]`:** DECIDED in DISCUSS (operator-ratified 2026-06-29) — an explicit lifecycle verb; `overdrive deploy` stays pure-declare (overloading deploy ruled out; k8s `apply` vs `rollout restart` rationale). OPEN for DESIGN (hard gate): the verb's name + semantics (restart-with-rollout-restart-breadth vs resume/start ± a separate restart), HTTP/response shape, sentinel-clearing mechanics, + ADR. Invariant pinned.
6. **Journey home:** EXTEND `dial-a-mesh-peer-by-name.yaml` with a replace/cycle step (see Applied SSOT diffs) — do NOT create a standalone journey. The cycle is the operable form of that journey's STABLE/CHURN behavior; the journey already names the cycle as the SQ1-elimination step. (Justification: a new journey would duplicate the dial-by-name reachability arc; the replace action is the *operator verb* that makes that journey's cycle step real.)

### Scope Assessment: PASS — 2 stories + terminal verification gate, 1–2 modules, ≈2.5d DELIVER (~3–5d incl. DESIGN `[D1]`)
Stories: 2 (US-BIR-1, US-BIR-2; ≤10 ✅) + a terminal verification gate (the Tier-3 oracle — evidence, not a story). Bounded contexts/modules: the replace action (handler + intent-key clearing) + the `WorkloadLifecycle` reconciler edit (≤3 ✅). Walking-skeleton integration points: serve + deploy + the replace action + the intent store + the reconciler (≤5 ✅). **Per-slice DELIVER estimates (each ≤1 day):** slice-01 ≈1d, slice-02 ≈0.5d, slice-03 ≈0.5d, slice-04 (terminal gate) ≈0.5d → ≈2.5d DELIVER; the ~3–5d feature figure adds the DESIGN-wave `[D1]` resolution (verb + ADR). Multiple independent outcomes that could ship separately? No — all serve the single instance-replacement outcome. **Right-sized; no split needed.** (story-map does not exist as a separate file — this is the lean compact form.)

### Carpaccio slices (briefs under `slices/`)
| Slice | Goal (one line) | Est. | Terminal? |
|---|---|---|---|
| **slice-01** | Replace action ends the current instance, clears the sentinel, brings up a NEW instance with `jobs/<id>` retained — proven through serve + the action (US-BIR-1). The walking skeleton of the feature's own work. | ≈1d | |
| **slice-02** | The stable `F`-binding survives the cycle: `getaddrinfo` re-resolves the same `F`, the next connect lands the new backend, no stale address (US-BIR-2). | ≈0.5d | |
| **slice-03** | In-flight churn fails fast bounded by `TCP_USER_TIMEOUT`, subsequent fresh connect lands the new instance (US-BIR-2 churn boundary). | ≈0.5d | |
| **slice-04** | Un-`#[ignore]` all three #249 ATs (S-DBN-WS-STABLE + S-DBN-CHURN + S-DBN-NXDOMAIN-02 RECOVERY), GREEN on the pinned-6.18 Tier-3 matrix, driving the production replace action — the **terminal verification gate for US-BIR-1 + US-BIR-2** (not a user story). | ≈0.5d | ✅ terminal |

### Applied SSOT diffs (DISCUSS back-propagation — operator-ratified 2026-06-29)
- **`docs/product/jobs.yaml`:** EXTEND J-OPS-003 — enrich the `motivation`/`outcome` to name the *third* lifecycle transition (replace an instance / fresh allocation while intent stays declared), + a changelog entry recording the extend-vs-mint justification (udp-sendmsg4 elevation precedent). *Applied; operator-ratified 2026-06-29.*
- **`docs/product/journeys/dial-a-mesh-peer-by-name.yaml`:** EXTEND with a replace/cycle step + an error path (no-such-workload), recording that the operator verb's name/semantics is design-open (`[D1]`; an explicit lifecycle verb, NOT `deploy`). *Applied; operator-ratified 2026-06-29.*
- **`docs/product/personas/ana-platform-engineer.yaml`:** EXTEND the persona header note to record that Ana now also anchors the lifecycle-replacement journey (she currently anchors the two UDP journeys). *Applied; operator-ratified 2026-06-29.*

### Anti-pattern scan (clean)
- No "Implement X" stories (all open from Ana's pain). ✓
- No generic data (real names: Ana, `payments`/`coinflip`/`server`/`client`, `payments-0`/`payments-1`, `10.99.0.2`, `F1`/`B1`/`B2`). ✓
- No technical AC / technical scenario titles (outcomes: "brings up a new instance," "re-resolves the same address," "fails fast … then the next dial is live"). ✓
- No oversized story (each ≤7 scenarios, single behavior). ✓
- No abstract requirements without examples (3+ per story). ✓
- No solution-prescription in requirements — the mechanism is explicitly OPEN (`[D1]`); ACs frame observable outcomes (new AllocationId, `F` stable, intent retained), not the verb shape. ✓

### Deferrals (operator-approved 2026-06-29; tracked as GitHub issues)
1. **Rolling / zero-downtime replacement (drain-old-then-cut-over) is OUT of v1 scope — tracked as #253.** The three oracle ATs cycle/recover a single-replica instance (`replicas=1`) with an end-then-bring-up shape (`stop_and_converge` then a new instance); there is a brief window with no live instance. A zero-downtime replacement (bring `A2` up *before* ending `A1`) is a *distinct, larger* concern. v1 ships as end-then-bring-up (single replica); the no-gap variant is **#253**.
2. **`replicas > 1` replacement semantics undefined — tracked as #254.** The ATs assume `replicas=1`. For `replicas>1`, "replace the instance" could mean replace-all vs replace-one — undefined. v1 covers `replicas=1` (matching the oracle ATs); multi-replica replace semantics are **#254**.
3. **All three #249-blocked ATs' `#[ignore]` strings reference #249 directly** (`"…overdrive-sh/overdrive#249…"`, across `dns_responder_walking_skeleton.rs` ×2 and `dns_responder_nxdomain.rs` ×1). When un-ignored (slice-04), the strings are *removed*, not rewritten — no stale forward-pointer remains. Noted so DELIVER does not leave a dangling reference. (Not a blocker; a hygiene note.)

### Risks / notes
- **No DIVERGE artifacts** — consistent with the jobs.yaml header precedent (JTBD distilled from issue + grounded code, not interviews). Non-blocking; the grounded reconciler/handler/AT code + the parent arcs' history ground the contracts.
- **`[D1]`'s mechanism is decided** (explicit lifecycle verb; `deploy` stays pure-declare — operator-ratified 2026-06-29). Its verb **name + semantics** remains the scoped DISCUSS→DESIGN hand-off (a hard DESIGN gate), not an incomplete DoR. DESIGN owns the verb shape + the ADR.

### Journey extension source (written to `dial-a-mesh-peer-by-name.yaml`; draft below)

```yaml
  - id: 4
    name: "An operator replaces a declared workload's instance — same name, new instance, intent retained"
    command: "(operator replace action — an explicit lifecycle verb, name DESIGN-open per backend-instance-replacement [D1]: e.g. overdrive job restart <id>; NOT overdrive deploy, which stays pure-declare)"
    summary: >
      Ana cycles a declared mesh workload's instance: the current instance (A1,
      old workload_addr) ends and a NEW instance (A2, new workload_addr) comes up
      while jobs/<id> stays declared. The per-logical-workload F-binding is byte-stable
      across the cycle (the FrontendAddrAllocator's idempotent assign — withhold-not-release),
      so a peer re-resolving <job>.svc.overdrive.local gets the SAME F and the next
      connect lands the NEW backend (no stale address — the SQ1-elimination guarantee).
      Operator-observable via a NEW AllocationId in overdrive alloc status. This makes
      the STABLE/CHURN cycle behavior AND the withhold-not-release recovery observable
      (the three #249-deferred ATs: 02-02 S-DBN-WS-STABLE + S-DBN-CHURN, and the 03-01
      S-DBN-NXDOMAIN-02 RECOVERY leg) operable through a production verb. Feature: backend-instance-replacement (GH #249).
```

---

## Changelog

| Date | Change |
|---|---|
| 2026-06-29 | Initial DISCUSS feature-delta for backend-instance-replacement (GH #249), authored by Luna. EXTEND J-OPS-003 (not a new job). 3 stories (US-BIR-1/2/3 — US-BIR-3 later refolded into the verification gate, rev2), 4 carpaccio slices (terminal slice un-ignores all three #249 ATs — 02-02 S-DBN-WS-STABLE/CHURN + 03-01 S-DBN-NXDOMAIN-02 RECOVERY). KPIs K-BIR-1..4. DoR PASS. |
| 2026-06-29 (rev, post-review) | Reviewer + operator pass: (1) oracle corrected from two ATs to **three** across two files — added the 03-01 S-DBN-NXDOMAIN-02 RECOVERY leg (`recovered_job_after_stop_resolves_to_the_same_stable_frontend`) to US-BIR-3 / slice-04 / K-BIR-4 / the registry. (2) `[D1]` mechanism DECIDED (operator-ratified): an **explicit lifecycle verb**; `overdrive deploy` stays **pure-declare** — overloading deploy ruled out (k8s `apply` vs `rollout restart`); verb name + semantics remains the OPEN, **hard DESIGN gate**. (3) Deferrals operator-approved + tracked as **#253** (rolling/zero-downtime) and **#254** (multi-replica replace). (4) SSOT edits (J-OPS-003, journey, Ana persona) operator-ratified. |
| 2026-06-29 (rev2, post-review) | Second review pass: (1) **US-BIR-3 removed as a user story** — a "land three tests green" outcome is a CI-runner result, not a user-invocable operator action; refolded into the **terminal verification plan / quality gate** for US-BIR-1 + US-BIR-2 (the three ATs stay as mandatory oracle evidence). Now **2 stories** + 1 terminal verification gate. (2) Journey actor/arc discontinuity fixed — `dial-a-mesh-peer-by-name.yaml` step 4 carries an explicit Ana actor-handoff + lifecycle micro-arc (Sam owns steps 1-3, Ana owns step 4). (3) **Per-slice effort estimates** added (slice-01 ≈1d, 02/03/04 ≈0.5d each → ≈2.5d DELIVER; ~3–5d incl. DESIGN `[D1]`). |
