# service-vip-allocator — Feature Evolution

**Feature ID**: `service-vip-allocator`
**Branch**: `marcus-sa/vip-allocator-module`
**Duration**: 2026-05-13 (DISCUSS opened) — 2026-05-19 (DELIVER + finalize close)
**Status**: Delivered — 13/13 DELIVER steps complete, adversarial review PASS,
mutation gate kill-rate 100% on touched files (with feature-wide
gap-closing commit `916f79bb`), 1249/1250 Lima workspace tests pass
(single failure is documented pre-existing cross-worktree `target/`
contamination, passes in isolation), workspace clippy + OpenAPI gate
clean, DES integrity 13/13 steps with 5/5 TDD phases logged each.
**ADRs**: [ADR-0049](../product/architecture/adr-0049-platform-issued-service-vip-allocator.md)
(4 amendments), [ADR-0050](../product/architecture/adr-0050-intent-side-workload-aggregate.md),
[ADR-0051](../product/architecture/adr-0051-wire-side-submit-spec-input.md)

---

## What shipped

A platform-issued `ServiceVipAllocator` for Phase 1 single-node Service
workloads. Operators submitting a Service spec receive an allocator-issued
`Ipv4Addr` from a default-with-override VIP range
(`10.96.0.0/16` by default; overridable via `[networking.service_vip]` in
the control-plane TOML). VIPs are issued idempotently on submit (the same
spec resubmitted yields the same VIP), allocations survive control-plane
crash/restart via redb-persisted allocator state, and VIPs are reclaimed
on terminal-state observation through the `WorkloadLifecycle` reconciler
emitting `Action::ReleaseServiceVip`.

The feature also introduces two parallel intent-layer aggregates
(`WorkloadIntent` per ADR-0050, `SubmitSpecInput` per ADR-0051) that
separate the three workload-spec representations the platform now
handles distinctly: TOML-parsed `WorkloadSpec` (parser surface),
JSON-deserialised `SubmitSpecInput` (wire surface), and persisted
`WorkloadIntent` (intent / store surface).

### Production code

**Allocator primitives** (`crates/overdrive-dataplane/src/service_vip/`):

- `ServiceVipAllocator` — `VipRange`-scoped allocator with
  content-addressed allocation memo, monotonic free-list reuse on
  release, and exhaustion as a typed error rather than a panic.
- `BackendIdAllocator` relocated to `service_vip/` alongside the new
  allocator (consolidation; same code, new home).
- Persistence wrapper: rkyv-encoded allocator state crosses the redb
  boundary through the typed `<Allocator>::archive_for_store` /
  `<Allocator>::from_store_bytes` codec discipline (ADR-0048 pattern).
- Boot-time Earned Trust probe: control-plane refuses to start when
  the allocator state on disk disagrees with the active `VipRange`
  configuration (operator must explicitly drain or align — no silent
  recovery).
- `Action::ReleaseServiceVip { vip }` — emitted by the reconciler at
  terminal-state observation; dispatched by the action shim to the
  allocator's release path. ADR-0049 was amended 2026-05-19 to make
  reuse-on-release the canonical semantics (the original
  non-reuse-monotonic shape would have exhausted `/16` at 65K
  lifetime allocations — operational footgun).

**Intent-layer aggregates** (`crates/overdrive-core/src/aggregate/`):

- `WorkloadIntent` (ADR-0050) — intent-side aggregate; `Job` codec
  relocated to `WorkloadIntent::archive_for_store` /
  `WorkloadIntent::from_store_bytes`. The `IntentStore` trait surface
  stays bytes-passthrough by design (shared with future `RaftStore`
  Phase 2 snapshot contract per ADR-0048).
- `SubmitSpecInput` (ADR-0051) — wire-side discriminator parallel to
  `WorkloadIntent`; the CLI / control-plane handler boundary decodes
  JSON into `SubmitSpecInput`, then translates into the intent
  aggregate. The three-type-family pattern (TOML→`WorkloadSpec` |
  JSON→`SubmitSpecInput` | persisted→`WorkloadIntent`) is now the
  canonical shape for every future workload kind.

**Parser surface** (`crates/overdrive-cli` + `crates/overdrive-core`):

- `Listener.vip` field removed at the parser level (ADR-0049
  amendment 2026-05-14). The TOML parser rejects operator-supplied
  VIPs structurally — there is no field for the operator to set.
  Submit-echo returns the allocator-issued VIP for operator
  visibility (Phase 1's interim CLI surface; Phase 2 issue #182
  tracks the operator-facing CLI surface for active range
  inspection).

**Control-plane wiring**
(`crates/overdrive-control-plane/src/app_state.rs` +
`src/handlers/submit.rs` + `src/reconciler/workload_lifecycle.rs`):

- `AppState` carries the allocator with default-with-override
  `VipRange` resolution (ADR-0049 amendment 2026-05-15:
  default-with-override beats refuse-to-start for Phase 1 operability
  — grounded in the research doc).
- `submit_spec` handler issues VIPs idempotently against the
  content-addressed memo; alloc-status reports the issued VIP.
- `WorkloadLifecycle` reconciler emits `Action::ReleaseServiceVip` on
  terminal-state observation (ADR-0049 amendment 2026-05-19: VIP
  reuse semantics).
- Action shim dispatches `ReleaseServiceVip` to the allocator's
  release surface; allocator returns the VIP to the free list for
  monotonic reuse.

### Test coverage

Acceptance tests cover the full S-VIP scenario catalogue from the
DISTILL test-scenarios specification, split across three crates by
ownership:

- `crates/overdrive-core/tests/` —
  - S-VIP-P01 (newtype round-trip property)
  - S-VIP-13, S-VIP-14 (parser-level `Listener.vip` rejection)
  - S-VIP-06 (reconciler-emission layer of `Action::ReleaseServiceVip`)
- `crates/overdrive-control-plane/tests/` —
  - S-VIP-01 through S-VIP-07 (submit / re-submit / allocate / release
    end-to-end through the control-plane handler + reconciler +
    action shim path)
  - Boot-path / Earned Trust probe coverage
- `crates/overdrive-dataplane/tests/` —
  - S-VIP-05 (persistence across restart), S-VIP-10 (no partial
    state on exhaustion), S-VIP-20 (release idempotent), S-VIP-19
    (boot probe refuses inconsistent state), S-VIP-12 (constructed
    from `/24`), S-VIP-16/17/18 (`VipRange::new` rejection cases),
    S-VIP-21 (reserved-address skipping)
  - S-VIP-P02 / S-VIP-P03 / S-VIP-P04 — allocator + `VipRange`
    property tests (duplicate-token freedom, capacity invariant,
    reserved-skip)

The S-VIP-08, S-VIP-09, S-VIP-15 placeholders in the catalogue map to
internal allocator scenarios covered through the property-test
families above.

### Mutation testing

Per-step mutation runs landed at 100% kill rate on each step's
touched files. A feature-wide gap-closing commit (`916f79bb`) closed
residual mutants surfaced during the L1-L6 refactor pass on
`overdrive-dataplane` allocator paths. Mutation gate is the structural
defense — every public allocator behaviour is exercised by a test that
fails on the mutation.

### Adversarial review (Phase 4)

Reviewer (architect agent) reported **PASS** across 13 scrutiny areas:

1. Concurrency safety (allocator mutex discipline)
2. rkyv schema evolution (envelope pattern conformance per ADR-0048)
3. Three-type-family separation (no leakage of `WorkloadSpec` /
   `SubmitSpecInput` / `WorkloadIntent` across boundaries)
4. Persist-inputs-not-derived-state (allocator memo is content-
   addressed; reuse free list is observed state, not derived)
5. Earned Trust on boot (state-mismatch refuses startup; structured
   `health.startup.refused` event)
6. Single-cut greenfield migration (no deprecation / grace period /
   feature-flagged old path)
7. VIP reuse semantics (ADR-0049 amended; operational footgun
   averted)
8. Exactly-once dispatch (`Action::ReleaseServiceVip` idempotent on
   replay; double-release returns `Ok(())` rather than erroring)
9. HTTP error surface (Service-arm rejection at admission cites GH
   #183 for wire-shape widening, not #182)
10. Test altitude (per `.claude/rules/debugging.md` § 7 — tests
    assert on observable behaviour, not implementation reachability)
11. GitHub issue references (every deferral language carries a real
    issue number verified via `gh issue view`)
12. Deferrals (only two: #182 operator-facing CLI surface, #183
    wire-shape widening; both user-approved before creation per
    `feedback_no_unilateral_gh_issues.md`)
13. DST safety (allocator is sync; no async / no wall-clock /
    Sim/Host trait shape unchanged)

---

## Steps completed (13)

Commit SHAs map to each DELIVER step. Read via
`git log fdbee68d..HEAD --oneline` (commit range starts at
`8c835043 docs(discuss): service-vip-allocator wave artifacts`).

| Step  | Commit          | Title                                                                                |
|-------|-----------------|--------------------------------------------------------------------------------------|
| 01-01 | `39b1233d`      | feat(dataplane): add ServiceVipAllocator + relocate BackendIdAllocator               |
| 01-02 | `4cbeb70a`      | refactor(core,dataplane): consolidate ServiceVip newtype into id.rs                  |
| 01-03 | `4b0dcca3`      | feat(dataplane,core,store): persistence wrapper for ServiceVipAllocator              |
| 02-01 | `a9fce5c2`      | refactor(core,cli): remove Listener.vip field; parser-level rejection                |
| 02-02 | `f9ed5ffe`      | feat(control-plane): VIP allocator config parsing + boot refusal                     |
| 02-03a| `2a520742`      | refactor(core,control-plane,store,cli,sim): introduce WorkloadIntent (ADR-0050)      |
| 02-03b| `0ab75419`      | refactor(core,control-plane,cli): SubmitSpecInput wire layer + JobSpecInput cascade (ADR-0051) |
| 02-03c| `32480b48`      | feat(control-plane,dataplane): wire ServiceVipAllocator into AppState (default-with-override) |
| 02-03d| `d572ece0`      | feat(control-plane): Service-arm submit/alloc-status with VIP allocation (ADR-0049)  |
| 02-04 | `09a5502e`      | feat(dataplane,control-plane): boot-time Earned Trust probe for ServiceVipAllocator  |
| 03-01 | `7a251c3e`      | feat(core,control-plane): WorkloadLifecycle reconciler emits Action::ReleaseServiceVip on terminal observation |
| 03-02 | `83334ae8`      | feat(control-plane): action-shim dispatch arm for Action::ReleaseServiceVip          |
| 03-03 | `f791b755`      | feat(dataplane,control-plane): VIP reuse on release + S-VIP-06/07 end-to-end (ADR-0049 amended 2026-05-19) |

Post-DELIVER refactor / gap-closing commits:

- `5ae229e1` — fix(integration-gate): reconcile post-rebase with SystemGC feature (main)
- `618fc9c6` — refactor(dataplane): L1 RPP — fix docstring drift on allocator reuse policy
- `0f1b8156` — refactor(dataplane): L1 RPP — fix remaining counter-shape doc drift on persistent allocator
- `916f79bb` — test(mutants): close feature-wide mutation gaps for service-vip-allocator

---

## Key architectural decisions

### ADR-0049 — Platform-issued ServiceVipAllocator (4 amendments)

The original decision: platform issues Service VIPs from an operator-
configured `[networking.service_vip]` range, refuse-to-start when
unset, allocator never reuses (monotonic forever).

| Date         | Amendment                                                                                                  |
|--------------|------------------------------------------------------------------------------------------------------------|
| 2026-05-14   | Generic-rejection at admission: any operator-supplied `vip` field on a Service workload is rejected        |
| 2026-05-14   | Parser-level removal of `Listener.vip` — operators cannot supply a VIP because the field does not exist    |
| 2026-05-15   | Default-with-override: `10.96.0.0/16` is the default range; refuse-to-start dropped for Phase 1 operability |
| 2026-05-19   | VIP reuse on release: released VIPs return to the free list (monotonic-forever would exhaust `/16` at 65K) |

### ADR-0050 — WorkloadIntent intent-side aggregate

Codec relocated from `Job` to `WorkloadIntent`. The store boundary
stays bytes-passthrough; the typed codec lives on the intent value.
First codified instance of the post-ADR-0048 pattern applied to a new
domain aggregate (rather than retrofitted onto an existing one).

### ADR-0051 — SubmitSpecInput wire-side discriminator

Parallel to ADR-0050 but for the JSON wire surface. The control-plane
handler decodes JSON into `SubmitSpecInput`, translates to
`WorkloadIntent`, persists via the codec. Three-type-family separation
(TOML→`WorkloadSpec` | JSON→`SubmitSpecInput` | persisted→
`WorkloadIntent`) is now the canonical shape for every workload kind
that crosses the parser / wire / intent boundaries.

---

## Structural findings during DELIVER (worth recording)

Three step-splitting decisions that should inform future feature
estimates:

- **02-03 → 02-03a + 02-03b split**: the wire-shape migration's blast
  radius was larger than the DISTILL roadmap estimated. The
  `WorkloadIntent` codec relocation alone touched ~20 call sites; the
  `SubmitSpecInput` cascade compounded across CLI, control-plane, and
  sim. Splitting mid-step preserved the green-bar property without
  forcing a multi-day checkpoint resume.
- **02-03b → 02-03b + 02-03c split**: the `SubmitSpecInput` migration
  cascaded across 21 fixtures + 6 acceptance tests + the streaming
  ingest path. Splitting let 02-03b land the type-system shape and
  02-03c land the boot-path wiring as separate green-bar commits.
- **02-03c → 02-03c + 02-03d split**: 02-03c landed the infra cascade
  (allocator wired into `AppState`); 02-03d landed the Service-arm
  business-logic code (submit/alloc-status handler arm with VIP
  allocation). Cleaner GREEN scope per step than a single conflated
  commit.

One mid-step ADR amendment:

- **Step 03-03 contradicted ADR-0049 mid-step**: the original
  monotonic-no-reuse semantics meant S-VIP-07 (released VIPs reuse
  on subsequent submit) was unrepresentable. Rather than land the
  test as a `#[should_panic]` scaffold or carry an unresolved
  contradiction, the architect agent was invoked to amend ADR-0049
  (2026-05-19) and the implementation landed against the amended
  decision. Structural finding logged to user, user-approved before
  the amendment landed.

---

## Lessons learned

**Default-with-override > refuse-to-start for Phase 1.** The research
doc (`docs/research/orchestration/service-vip-range-config-patterns.md`)
grounded the 2026-05-15 amendment with prior-art evidence from
Kubernetes (`--service-cluster-ip-range` default `10.0.0.0/24`), Nomad
(no built-in VIPs; operator supplies), and Cilium / Calico (default
ranges with override). Refusing to start when an operator hasn't
explicitly configured a range optimises for the wrong failure mode:
operators learning the platform hit "boot refused" before they hit
"workload submitted." Phase 2's operator-facing CLI surface (GH #182)
will close the visibility loop without re-imposing the boot gate.

**Splitting mid-step when scope > 30 turns is cheaper than
checkpoint+resume.** Three splits (02-03 → a+b → c → d) saved
substantial token spend on context-replay. The DES log carries one
parent `02-03` entry and four child entries (`02-03a`, `02-03b`,
`02-03c`, `02-03d`); DES integrity validation treats the parent as
the expected pre-split entry.

**User-approved structural findings land as architect ADR
amendments, not crafter inline edits.** The 2026-05-19 VIP-reuse
amendment was surfaced by the crafter mid-step, user-approved, and
landed by the architect agent (not the crafter). This honors
`feedback_delegate_to_architect.md` and keeps the ADR's authority
boundary clean.

**Operational footguns are caught by populated-thought-experiments,
not by tests.** S-VIP-07's reuse semantics weren't in the original
test scenarios. The scenario was added when the crafter ran the
numbers: `/16` × 65K = exhaustion in a moderately-busy cluster's
lifetime. Tests would only have caught this at 65K allocations — by
which point recovery requires operator drain or pool resize. The
amendment landed in step 03-03 before any production exposure.

---

## Deferrals

Both deferrals are user-approved, real GitHub issues with scoped
acceptance criteria:

- **[#182](https://github.com/marcus-sa/overdrive/issues/182)** —
  Operator-facing CLI surface for active VIP allocator range
  inspection. Phase 2. Phase 1 satisfies the visibility need via
  submit-echo of the issued VIP.
- **[#183](https://github.com/marcus-sa/overdrive/issues/183)** —
  WorkloadDescription Service-arm wire-shape widening (`oneOf`
  discriminator for `describe_workload`). Parallel to the ADR-0051
  migration on the submit path. Phase 2.

No hand-wavy "future ticket" pointers; both issues are referenced by
number at every deferral citation site in code and ADRs.

---

## Permanent artifacts

| Artifact                                                                                   | Location                                                              |
|--------------------------------------------------------------------------------------------|-----------------------------------------------------------------------|
| ADR-0049 (4 amendments) — Platform-issued ServiceVipAllocator                              | `docs/product/architecture/adr-0049-platform-issued-service-vip-allocator.md` |
| ADR-0050 — Intent-side WorkloadIntent aggregate                                            | `docs/product/architecture/adr-0050-intent-side-workload-aggregate.md` |
| ADR-0051 — Wire-side SubmitSpecInput discriminator                                         | `docs/product/architecture/adr-0051-wire-side-submit-spec-input.md`   |
| Research — VIP range config patterns (Kubernetes / Nomad / Cilium / Calico prior art)      | `docs/research/orchestration/service-vip-range-config-patterns.md`    |
| Research — WorkloadSpec / Intent separation patterns                                       | `docs/research/aggregates/workload-spec-intent-separation-patterns.md` |
| Acceptance tests (S-VIP-01 through S-VIP-07)                                               | `crates/overdrive-control-plane/tests/acceptance/`                    |
| Allocator implementation                                                                   | `crates/overdrive-dataplane/src/service_vip/`                         |
| Wave artifacts (preserved; SSOT for wave-matrix status)                                    | `docs/feature/service-vip-allocator/`                                 |

---

## Quality gates

- **DES integrity**: 13/13 steps logged with 5/5 TDD phases each
  (`PREPARE | RED_ACCEPTANCE | RED_UNIT | GREEN | COMMIT`). One stray
  `02-03` parent entry is the pre-split DES record (expected; the
  four child split entries `02-03a/b/c/d` carry the actual phase
  logs).
- **Mutation gate**: per-step 100% kill rate on touched files;
  feature-wide gap-closing commit `916f79bb` closed residual gaps
  surfaced during L1-L6 refactor.
- **Adversarial review**: PASS across 13 scrutiny areas.
- **Lima workspace test suite**: 1249/1250 tests pass. The single
  failure is a documented pre-existing Lima-shared `target/`
  cross-worktree contamination — the test passes in isolation and
  is unrelated to this feature's surface.
- **Workspace clippy**: clean.
- **OpenAPI gate**: clean.
- **Cargo check workspace (Lima)**: clean.
