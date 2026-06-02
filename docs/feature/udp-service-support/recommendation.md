# Recommendation — `update_service` proto-threading (scoped DIVERGE)

**Feature:** udp-service-support · **Decision under divergence:** how to
thread per-service L4 protocol through `Dataplane::update_service` so
production `EbpfDataplane` installs `REVERSE_NAT_MAP` entries matching the
declared proto (GH #163). · **Closes review finding H3** ("simpler
alternative unweighed") against `feature-delta.md`'s DISCUSS wave.

> **Honesty mandate satisfied.** The user holds a STANDING preference for
> the typed aggregate (option 2 / DISCUSS D1). The locked developer-tool
> taste matrix places option 2 **third (3.57)**, behind two simpler
> options that thread proto without an aggregate. This recommendation
> reports that honestly and does NOT rubber-stamp the preference. The
> dissenting case for the aggregate is documented in full below.

---

## CRITICAL baseline correction (carried into this recommendation)

The SHIPPED trait today is **option C**:

```rust
async fn update_service(&self, vip: Ipv4Addr, backends: Vec<Backend>) -> Result<(), DataplaneError>;
```

— verified at `crates/overdrive-core/src/traits/dataplane.rs:101`. No
`service_id`, no `ServiceVip` newtype. The phase-2 `architecture.md §5
Q-Sig` **locked decision A** (`update_service(service_id: ServiceId, vip:
ServiceVip, backends)`) was **NEVER landed on the trait** — it is a paper
decision. Every option below is therefore a transition **FROM shipped-C**,
not from locked-A. The `Action::DataplaneUpdateService { service_id, vip,
backends, correlation }` envelope already carries `service_id` BESIDE
vip/backends (`crates/overdrive-control-plane/src/action_shim/validate.rs:288`).
The Sim hardcode is `reverse_nat_keys_for`'s `[Proto::Tcp, Proto::Udp]` at
`crates/overdrive-sim/src/adapters/dataplane.rs:277`.

---

## Decision statement (for the DISCUSS revision)

> **Proceed with threading the protocol as a typed field of the existing
> `update_service` call shape — NOT as a whole-call aggregate.** The
> taste matrix selects **Option 6 (`ServiceFrontend` newtype on the first
> argument, `backends` separate) at 4.17**, in a statistical tie with
> **Option 1 (positional `proto` arg) at 4.13**, both decisively ahead of
> the typed-aggregate **Option 2 at 3.57**. The DISCUSS D1 lock on the
> *aggregate* is **not supported by the scored evidence** and should be
> revised to "thread proto into the existing call shape (newtype-on-arg
> preferred over positional, a secondary DESIGN choice); the whole-call
> aggregate is the documented dissent." **Assuming** the single DESIGN
> risk is acceptable: that multi-listener fan-out (US-05) is handled by
> the **hydrator emitting one call per listener** rather than the trait
> carrying `Vec<Listener>`.

The Option-6-vs-Option-1 gap (Δ 0.04) is inside scoring noise — the choice
between them is a **secondary DESIGN preference**, not a divergence-level
decision. Both are the same architectural family: *proto threaded into the
existing call, no aggregate*. DISCUSS should lock the **family**; DESIGN
picks newtype-vs-positional.

---

## Top 3 options

### Option 6 — `ServiceFrontend` newtype — Score 4.17 (RECOMMENDED)

`update_service(frontend: ServiceFrontend, backends)` where
`ServiceFrontend` wraps `(vip, port, proto)`; `backends` stays a separate
argument.

- **Why it scores well:** T2=4 and T4=4 — `ServiceFrontend` is the
  *forward-path twin* of the existing `BackendKey { ip, port, proto }`
  newtype the engineer already reads on the reverse side (prior art:
  Katran `VipKey { address, port, proto }` is byte-identical shape). The
  twin makes the lockstep set-equality trivial to express, and the
  concept is strongly anchored — near-zero new mental load.
- **Core trade-off:** one new newtype vs option 1's zero new types
  (option 1 wins T1=5 vs 6's T1=4). The newtype is the price of the
  type-system enforcement of "no service is described without its proto."
- **Key risk (must be true):** multi-listener (US-05) is handled by
  hydrator fan-out, NOT by the frontend carrying a list. If multi-listener
  must live *inside the trait surface*, option 3 or 5 is the better fit.
- **Hire criteria:** choose this when you want proto type-enforced into
  the call shape, the lockstep gate to be a one-line set-equality, and the
  smallest reviewable blast radius compatible with newtype-STRICT (C4).

### Option 1 — Positional `proto` — Score 4.13 (CO-LEADER)

`update_service(vip, proto, backends)` — one new scalar argument.

- **Why it scores well:** T1=5 (maximal subtraction — one scalar, no new
  type) and the smallest blast radius of all six options.
- **Core trade-off:** a 4th positional argument on a call that is already
  `(vip, …, backends)` slightly erodes call-site readability (the exact
  concern DISCUSS D1 raised) and does not type-enforce "proto belongs to
  the service identity."
- **Key risk:** same multi-listener assumption as option 6.
- **Hire criteria:** choose this if the team values zero-new-types over
  the `BackendKey` twin symmetry, and accepts a positional 4-tuple call.

### Option 2 — Typed aggregate — Score 3.57 (USER'S PREFERENCE; the DISSENT)

`update_service(descriptor)` carrying `(vip, port, proto, backends)` as one
value.

- **Why it scores lower:** T1=3, T2=3, T4=3 — the descriptor adds a
  wrapping aggregate concept beside the `(vip,port,proto)` shape already
  expressible, every call site constructs it (largest single-PR surface of
  the typed family), and it forces the service_id-reconciliation question
  (B2) that can false-green the C2 grep-AC if unresolved.
- **Why it might still be right (the dissent):** see § Dissenting case.
- **Hire criteria:** choose this if the multi-listener `Vec<Listener>`
  shape (US-05) is wanted *inside the descriptor from slice 01*, OR if
  re-converging toward locked-A's typed `(service_id, ServiceVip, …)`
  surface in ONE move is judged worth the larger blast radius.

---

## REQUIRED analysis per surviving option

### Blast radius (concrete call sites; single-cut per C6 — no deprecation shims)

| Option | Trait | EbpfDataplane | SimDataplane | action-shim dispatch | hydrator | lockstep invariant | New type? |
|---|---|---|---|---|---|---|---|
| **6 (rec)** | sig change (1 arg → newtype) | consume newtype | consume newtype; narrow `[Tcp,Udp]`→proto | construct `ServiceFrontend` from Action fields | unchanged (US-05 = hydrator fan-out) | build `ServiceFrontend`; assert proto twin | `ServiceFrontend` |
| **1** | sig change (+1 scalar) | read `proto` arg | read `proto`; narrow hardcode | pass `proto` from Action | unchanged | pass `proto`; assert | none |
| **2** | sig change (→ descriptor) | destructure descriptor | destructure; narrow hardcode | **construct descriptor** (every field) | unchanged or builds descriptor | build descriptor; assert | `ServiceDescriptor` |
| **3** | sig change (listener unit) | per-listener install | per-listener install | per-listener dispatch | **emission reshape (US-05 native)** | per-listener assert | listener tuple type |
| **5** | sig change (→ Vec<Listener> agg) | **internal fan-out loop** | **internal fan-out loop** | build aggregate | one-call-per-service | assert over fanned-out set | rich aggregate |
| **4** | **UNCHANGED** | Step 4b proto-recovery via `service_id` lookup | mirror recovery | unchanged sig; proto on Action | maybe pre-expand per-proto | **retarget over recovery path** | none (logic moves) |

The recommended option-6 blast radius is: **trait + both adapters +
action-shim + lockstep invariant = 5 sites**, hydrator UNCHANGED for the
US-01/US-04 milestone. This is the same site-count as option 1 and roughly
half of option 2's (which additionally pays the descriptor-construction
cost at every site and the service_id-reconciliation resolution in-PR).

### service_id / ServiceVip reconciliation verdict (review B2 hinges here)

**The recommended option (6) verdict:** `ServiceFrontend` **re-absorbs
`ServiceVip`** (it is the natural typed home for the validated VIP —
re-converging the *VIP* half of locked-A) but **leaves `service_id` on the
`Action` envelope** where it already lives (`validate.rs:288`).
Justification:

- `service_id` is an *action-routing / correlation* concern (it pairs with
  `correlation` on the Action), not a *dataplane-key* concern — the
  SERVICE_MAP outer key is `(ServiceVip, port)` and the REVERSE_NAT key is
  `BackendKey { ip, port, proto }`; neither is keyed by `service_id`
  (phase-2 architecture.md §5 Drift-3). Pulling `service_id` into the
  `update_service` surface would put a non-key field on a key-deriving
  call.
- This makes C2 ("single source of `(vip,port,proto)`") **precise and
  grep-checkable**: the `(vip,port,proto)` triple lives ONLY inside
  `ServiceFrontend`; `service_id`/`correlation` stay on the Action. The
  C2 grep-AC passes iff no call site reconstructs `(vip,port,proto)` from
  scattered args — which is structurally true once the triple is a
  newtype.

**For option 1:** `service_id`/`ServiceVip` are NOT re-absorbed — `vip`
stays raw `Ipv4Addr`, `proto` is a scalar, both stay on the Action. C2 is
satisfied by convention (no reconstruction) but NOT type-enforced.

**For option 2:** the descriptor MUST resolve this explicitly — it may
re-absorb both (toward locked-A), `ServiceVip` only, or neither. This
unresolved choice is precisely review B2's gap and a cost the aggregate
pays that options 1/6 do not.

> **DISCUSS-revision answer to B2:** the descriptor/frontend carries
> `(ServiceVip, port, Proto)`; `service_id` + `correlation` stay on the
> `Action::DataplaneUpdateService` envelope by design. C2 + the US-01
> grep-AC are restated against this defined pass condition.

### Sim≡Ebpf lockstep — how the identical `(ip,port,proto)→vip` set gets pinned against BOTH adapters

The fix (US-02) makes both adapters install the SAME key set; the gate
(US-03) must *pin* that. The recommended option makes this clean:

1. **The key type is shared.** Both adapters derive `BackendKey { ip,
   port, proto }` from the same `ServiceFrontend.(port, proto)` projection.
   The lockstep assertion is a `BTreeSet<BackendKey>` equality — option 6's
   twin shape makes the expected-set construction a one-liner (the gate
   already builds exactly this set today at
   `reverse_nat_lockstep.rs:158-165`, currently hardcoding `[Tcp,Udp]` —
   US-01 narrows that to `frontend.proto`).
2. **Tier 1 (in-process) pins SimDataplane** directly, as today
   (`evaluate_reverse_nat_lockstep`).
3. **The real `EbpfDataplane` cannot run in-process** (it needs a kernel +
   bpffs) — so a *pure-Tier-1 retarget of the SAME invariant against the
   real adapter is INFEASIBLE* (this is review **H1**, and it is real). The
   honest pinning is **two-pronged**:
   - **Tier 1** asserts the Sim adapter produces the proto-correct set
     (and that the Sim's `[Tcp,Udp]` hardcode is narrowed to
     `frontend.proto` — review H2).
   - **Tier 3 acceptance** (real veth, behind `integration-tests`, via
     `cargo xtask lima run`) drives the REAL `EbpfDataplane.update_service`
     for a UDP frontend and asserts `bpftool map dump REVERSE_NAT_MAP`
     contains the `(ip,port,udp)` entry — the production-adapter half of
     the equality. Plus a **Tier 2 `BPF_PROG_TEST_RUN` triptych** asserts
     `xdp_reverse_nat_lookup` rewrites a `proto=17` response source to the
     VIP.
   The "byte-identical set across BOTH adapters" claim is thus pinned by
   *Sim (Tier 1) ∪ Ebpf (Tier 2+3)* meeting at the shared `BackendKey`
   set — NOT by running both inside one DST process.

> **This makes H1 a DESIGN-resolved fact, not an open SPIKE inside the
> slice it gates:** the lockstep is Tier 1 (Sim) + Tier 3 acceptance
> (Ebpf), because the in-process both-adapter retarget is infeasible. The
> US-03 AC "OR Tier 3" must collapse to "Tier 1 Sim **and** Tier 3 Ebpf
> acceptance," and K2's measurement restated to match. The recommended
> option does not change this conclusion — it makes the shared-key-set
> assertion *simplest to write* (twin shape), which is why option 6 scores
> T4=4 where option 4 (recovery-by-lookup) scores T4=2: option 4's proto
> recovery diverges between Sim and Ebpf, fighting the very equality the
> gate asserts.

### Testability (Tier 1 / Tier 2 / Tier 3) per option

| Option | Tier 1 DST | Tier 2 `BPF_PROG_TEST_RUN` | Tier 3 real-veth e2e |
|---|---|---|---|
| **6 (rec)** | Sim set-equality over `BackendKey` twin — trivial | UDP triptych on `xdp_reverse_nat_lookup` (proto=17) — unchanged shape | Ebpf `update_service(frontend_udp)` + `bpftool dump` + wire capture VIP-source |
| **1** | same set-equality; proto passed scalar | same triptych | same e2e; call passes scalar proto |
| **2** | same set-equality; build descriptor | same triptych | same e2e; build descriptor |
| **3** | per-listener set-equality | same triptych | multi-listener e2e native |
| **5** | set-equality over adapter-fanned set | same triptych | one-call multi-listener e2e |
| **4** | **set-equality fights the recovery indirection** (Sim vs Ebpf recover proto differently) | same triptych | recovery path must be exercised end-to-end — most fragile |

All options support the three-tier story; option 4 is the only one whose
Tier 1 lockstep is *structurally harder* because the proto-recovery path
differs between adapters — the exact thing C5 ("production not shaped by
simulation") and the lockstep exist to prevent.

---

## Dissenting case — for Option 2 (typed aggregate; the user's preference)

**The scoring almost did not choose the leaders, and here is the honest
case the aggregate could still be right:**

1. **Multi-listener inside the trait (O5/US-05).** The matrix penalizes
   the aggregate's T3 on the assumption that multi-listener is a *hydrator*
   concern. If the team decides the **trait surface itself** should express
   a multi-listener service (because per-(VIP,port) is the SERVICE_MAP
   outer key and a `Vec<Listener>` descriptor maps to it directly), then
   option 2/5's richer shape stops being "front-loaded complexity" and
   becomes "the correct granularity," and its T3 penalty evaporates. Prior
   art supports this: Kubernetes `ServicePort` is exactly a list, and #39188
   is the failure of NOT having protocol in the per-port key.

2. **One-move re-convergence toward locked-A's typed intent.** The
   aggregate is the only option that can re-absorb BOTH `service_id` AND
   `ServiceVip` in a single migration, paying the call-site cost once and
   landing the typed surface architecture.md §5 originally wanted (locked-A)
   — extended to proto + multi-listener. Options 1/6 leave `service_id` on
   the Action permanently; if the team's long-horizon design wants
   `update_service` to be the typed service SSOT, the aggregate is the
   strategic move and the 0.56-point taste gap is the acceptable price of
   not migrating twice.

3. **Industry weight.** 3 of 4 prior-art references (Cilium `L4Addr`,
   Katran `VipKey`, k8s `ServicePort`) bind proto into a typed
   service/key structure. Option 6 captures most of this (the `VipKey`
   twin) — but if the team weights "match the dominant industry aggregate
   shape" higher than the developer-tool Subtraction/Speed weights this
   matrix locked, option 2 rises. **To make option 2 win the matrix you
   would raise T1+T4's importance DOWN and a new "industry-alignment"
   criterion UP — an explicit, documentable weight change, not a silent
   override.** The matrix as locked (developer-tool profile) does not
   support it; a different *product framing* (strategic-surface profile)
   could.

**Verdict on the dissent:** legitimate but conditional. It wins only if
(a) multi-listener is a trait-surface concern, OR (b) the team commits to
`update_service`-as-typed-SSOT as a strategic goal worth a larger blast
radius. Neither is established by the validated jobs (J-OPS-004/J-PLAT-004
are served identically by all three top options on O1–O3). Absent that
commitment, the matrix's choice (option 6/1) stands.

---

## What the DISCUSS revision (B1 / B2 / H3) should now say

- **B1 (from-state):** "The shipped trait is option **C**
  (`update_service(vip: Ipv4Addr, backends)`, `dataplane.rs:101`);
  locked-A was never landed. This feature threads proto **FROM C**."
- **B2 (service_id reconciliation):** "The frontend/descriptor carries
  `(ServiceVip, port, Proto)`; `service_id` + `correlation` stay on the
  `Action::DataplaneUpdateService` envelope by design. C2 + US-01's
  grep-AC pass condition = no call site reconstructs `(vip,port,proto)`
  outside the newtype."
- **H3 (simpler alternative now weighed):** "A scoped DIVERGE scored 6
  options on a locked developer-tool taste matrix. The simpler
  thread-proto-into-the-existing-call family (Option 6 newtype 4.17 /
  Option 1 positional 4.13) outscores the typed aggregate (Option 2,
  3.57). **D1 is revised from 'aggregate' to 'thread proto as a typed
  field of the existing call (newtype preferred); aggregate is the
  documented dissent, conditional on multi-listener-in-trait or
  typed-SSOT strategy.'** The aggregate is NOT rubber-stamped; the
  blast-radius delta is now evidenced, not assumed."

---

## Decision for DISCUSS wave

> **Proceed with Option 6 (`ServiceFrontend` newtype threading
> `(ServiceVip, port, Proto)` into the existing `update_service` call;
> `backends` separate; `service_id` stays on the Action), assuming
> multi-listener fan-out (US-05) is a hydrator concern, not a trait-surface
> one.** Option 1 (positional proto) is an acceptable DESIGN-level
> substitute (Δ 0.04). The typed aggregate (Option 2) is the documented
> dissent — adopt it ONLY if DESIGN establishes multi-listener-in-trait or
> `update_service`-as-typed-SSOT as a goal, recorded as an explicit
> weight-profile change. The lockstep is pinned as **Tier 1 (Sim
> set-equality) + Tier 3 acceptance (real Ebpf `bpftool` dump) + Tier 2
> triptych** — the in-process both-adapter retarget (H1) is infeasible and
> is hereby resolved at DIVERGE, not deferred into slice 03.

ADR amendment to phase-2 architecture.md §5 Q-Sig (C → the chosen
thread-proto family, superseding the paper locked-A) is the **architect's
job in DESIGN** — this DIVERGE forward-points only; it does NOT edit ADRs.

---

## Prism independent peer review (appended post-DIVERGE)

```yaml
# --- Prism independent peer review (appended post-DIVERGE) ---
prism_independent_review:
  reviewer: nw-diverger-reviewer (Prism)
  review_date: 2026-06-02
  review_type: independent-adversarial (NOT self-review)
  anchored_on_self_verdict: false
  verdict: APPROVED
  decision_reopened: false
  locked_option_6_stands: true

  scores_recomputed_independently: true
  top_three_arithmetic: verified-correct   # 6=4.17, 1=4.13, 2=3.57
  fix_1_3.62_to_3.57: verified-applied-and-consistent
  ranking: "6 > 1 > 2 > 3 > 5 > 4 (unaffected by erratum below)"

  code_anchors_verified_against_live_tree:
    - "dataplane.rs:101 = update_service(vip, backends) — option C from-state CONFIRMED"
    - "sim/dataplane.rs:277 = [Proto::Tcp, Proto::Udp] hardcode CONFIRMED"
    - "validate.rs:288 = Action::DataplaneUpdateService carries service_id CONFIRMED"
    - "backend_key.rs:137 = BackendKey{ip,port,proto} — Katran VipKey twin CONFIRMED"

  dimension_gates:
    jtbd_rigor: PASSED
    research_quality: PASSED
    option_diversity: PASSED
    taste_application: PASSED   # with non-decision-affecting erratum
    recommendation_coherence: PASSED

  findings:
    - id: PRISM-1
      severity: low
      type: arithmetic-erratum
      location: taste-evaluation.md:163 (Option 5 weighted total)
      detail: "Cell read 2.85; correct value is 2.75 (4.0*.25+2*.15+2*.20+2*.15+3*.25=2.75). Missed by self-review FIX-1. Rank-5 of 6; decision and ranking UNAFFECTED."
      remediation: "Corrected to 2.75 by orchestrator at review-landing time (not deferred)."
      status: RESOLVED
    - id: PRISM-2
      severity: none
      type: praise
      detail: "Weight discipline honest: dev-tool weights penalize Opt 2 exactly where mechanically justified; no industry-alignment criterion smuggled in; dissent names the explicit weight change that would flip it. No motivated reasoning toward the user's standing preference, no over-correction against it."
    - id: PRISM-3
      severity: low
      type: nitpick
      detail: "ODI O5 folds SCTP-extension + multi-listener into one outcome; #39188 is same-root/different-surface vs #163 (artifact hedges). Both acceptable for a scoped study."

  handoff: "APPROVED to proceed. Locked Option 6 (ServiceFrontend newtype family) stands. PRISM-1 cell corrected; no blocking defects."
```
