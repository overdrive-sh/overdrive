# DESIGN Wave-Decisions — `mtls-intercept-install-fault-seam` (GH #250)

> Authored by Morgan (nw-solution-architect), 2026-08-01. PROPOSE mode.
> Scope: **all priorities** — OQ-1 … OQ-9 from
> `docs/research/testing/fault-injection-seam-fail-closed-paths-research.md`
> § "Open Questions for DESIGN" are each pinned below. Full record:
> **ADR-0076**; full design + verbatim API surface:
> `design/architecture.md`.
>
> Two DESIGN-verified findings reframe the whole decision set and are recorded
> as **DFS-0a** and **DFS-0b** before the OQ answers, because every OQ answer
> depends on them.

## Review pointer

- **Iteration 1 review completed 2026-08-01** (nw-solution-architect-reviewer,
  opus). Verdict: `rejected_pending_revisions` — **0 critical, 5 high, 5 medium,
  4 low**; the rejection was mechanical (>3 HIGH), not a judgement against the
  approach. **Both load-bearing claims DFS-0a and DFS-0b were independently
  verified against source and CONFIRMED.** All 5 HIGH and all medium/low
  findings are **resolved** in the current revision:
  - **H1** — OQ-4's boot-probe justification did not survive verification (a
    node without `CAP_NET_ADMIN` already fails at the *upstream* netns
    provision seam, so the probe buys correct *diagnosis*, not new protection).
    **Resolved**: re-framed throughout; DFS-1 now stands on T2 alone and the
    probe is explicitly strikeable.
  - **H2** — `architecture.md` § 4.7 was labelled "exhaustive" but omitted
    `tests/integration/alloc_netns_lifecycle.rs:118`. **Resolved**: row added,
    table re-derived mechanically (1 production + 9 non-production across 7
    files), and a governing rule added so no call site requires crafter
    judgement.
  - **H3** — T1's assertions omitted the forward-carried fields
    (`workload_id`/`node_id`/`kind`/`started_at`) and `TransitionSource` — a
    NAMED bug class here (#248). **Resolved**: A-8/A-9/A-10 added.
  - **H4** — the mutation contract was narrower than the gate: the
    `exclude_re` entry is a bare *function-name anchor*, so deleting it
    un-suppresses every mutant in the function while the gate scores kill-rate.
    **Resolved**: obligation restated as 100% of the function's mutants.
  - **H5** — the trait contract stated postconditions the sanctioned sim
    adapter could not honour (`IP_TRANSPARENT`, "exactly ONE nft rule", guard
    `Drop` removes a rule). **Resolved**: contract split — the trait states only
    what both adapters honour; substrate specifics moved to
    `HostMtlsIntercept`'s own rustdoc. T4's asserted set now coincides exactly
    with the trait contract.
- **Iteration 2 review completed 2026-08-01.** Verdict: **`approved`** — 0
  critical, 1 high, 4 medium, 3 low. H2/H3/H4 confirmed **RESOLVED** against
  source; H1 and H5 confirmed **PARTIALLY RESOLVED** with a named residue each.
  All iteration-2 findings are **now closed** in this revision:
  - **H1-R** — the H1 correction had reached the narrative but not the
    **verbatim** blocks a crafter lands (§ 4.4's `run_server` comment and
    § 4.5's `MtlsBootError::InterceptProbe` rustdoc still said a
    capability-less node "fails every workload at `start_alloc` time").
    **Closed**: both pinned blocks rewritten to the upstream-provision-seam
    framing.
  - **M-1** — `install_outbound` / `install_inbound` still carried an
    un-relativised "the capture is in effect" postcondition the sim cannot
    honour. **Closed**: both relativised ("against this adapter's OWN
    substrate"), and § 5.4's coincidence claim qualified to "modulo one
    deliberately-unobservable clause", with that clause listed rather than
    silently dropped.
  - **M-2** — the assertion table's third column enumerated mutants
    cargo-mutants does not generate (it neither inserts statements nor
    substitutes call arguments, and the helper has no binary operators).
    **Closed**: column renamed "Regression it defends"; a new § 6.5 requires
    the ACTUAL mutant set to be enumerated before claiming the contract is met,
    and states plainly that a 100% kill over a one-mutant set is **vacuous** —
    so A-8/A-9/A-10 rest on the #248 bug class, not on mutation coverage.
    *(Mechanism corrected at DELIVER review, 2026-08-01: § 6.5 originally
    prescribed `cargo mutants --list`, which cannot run here —
    `.claude/hooks/block-cargo-mutants.ts` denies it even behind a
    `cargo xtask lima run --` prefix, and the xtask wrapper has no `--list`
    mode. The enumeration is now read off the scoped run's own guest
    `target/xtask/mutants.out/outcomes.json`. The requirement is unchanged;
    only the command is.)*
  - **M-3** — § 5.2 told the crafter to reuse a zero-argument `build_worker()`
    *and* to arm a fault on the sim it constructs inline, which is
    unsatisfiable. **Closed**: a test-local `build_worker(intercept: Arc<dyn
    MtlsIntercept>)` is pinned verbatim, with the exact `Arc::clone`-then-cast
    arming order.
  - **M-4** — the two `AllocIntercept` guard-field docstrings
    (`mtls_intercept_worker.rs:260-263`, `:269-273`) assert nft-by-handle
    behaviour that goes false under `Box<dyn InterceptGuard>`. **Closed**: added
    to the same-step doc-fix list.
  - **L-1/L-2/L-3** — file count 8 → **7**; Earned Trust renumbered 13 → **12**
    to match the seven in-tree citations; an out-of-contract fault-pairing note
    added to `SimMtlsIntercept`'s rustdoc.
- **Rev 4 — user direction, 2026-08-01** (not a review iteration). The
  **mechanism is unchanged**: the `MtlsIntercept` port is still built, in the
  same feature/PR as the killer test, with the DELIVER ordering of DFS-6
  intact. What changed:
  - **The justification is restated honestly and narrowed to ONE leg.** DFS-1
    previously read "justified by T2 + the boot probe". It now rests on **the
    call-site-ordering testability (T2) alone**, and states plainly that
    DFS-0a and DFS-0b each refute a justification the issue and research
    assumed — the port is **not** justified by mutant-killability, and **not**
    by enabling a default-lane end-to-end test.
  - **The boot `CAP_NET_ADMIN` probe is STRUCK** (OQ-4 reconciled below):
    production behaviour change, out of #250's scope, and it buys a better
    boot-time *diagnosis*, not a new safety property.
  - **DFS-8 added** — the Lima+root mutation-gate fact, which is what makes
    T2's integration-lane placement acceptable and closes DFS-0b as a
    non-problem for coverage.
  - **The `WorkloadNetns` deferral now cites its verified home,
    [#197](https://github.com/overdrive-sh/overdrive/issues/197).** No issue
    was created.
- Review files are the reviewer's artifacts and are not edited by the
  architect.

---

## Pre-decisions — the two findings

| # | Finding | Evidence | Consequence |
|---|---|---|---|
| **DFS-0a** | **The target mutant is killable TODAY at ZERO production cost.** `fail_closed_on_mtls_install` is module-private, so an in-crate `#[cfg(test)] mod tests` in `action_shim/mod.rs` can call it directly; all eight arguments are constructible in the default lane with no I/O. The issue's stated blocker is **misdiagnosed** — `#[non_exhaustive]` on `MtlsInterceptInstallError` is **enum-level only**, with no per-variant `#[non_exhaustive]` and every variant public with public field types, so per the Rust Reference it blocks exhaustive *matching*, not *construction*. | `crates/overdrive-worker/src/mtls_intercept_worker.rs:119-184`; `action_shim/mod.rs:413`; research Finding 4.4 / Gap G4 | The "accept the gap / permanent justified exclusion" branch (the Cilium-persuaded alternative the dispatch offered) is **moot** — there is no gap to accept. The port must be justified by something OTHER than the mutant. See DFS-1. |
| **DFS-0b** | **An end-to-end `dispatch` killer test CANNOT be default-lane — port or no port.** `provision_and_inject_netns` is gated on the SAME `mtls_worker.is_some()` flag and runs UPSTREAM of the mTLS install. `mtls_worker: Some(..)` therefore forces real `ip netns`/veth shell-outs; without root the alloc is driven `Failed` by the *sibling* `fail_closed_on_netns_provision` handler and `worker.start_alloc` is never reached. | `action_shim/mod.rs:830-869`, call sites `:1169` / `:1376`; `tests/integration/alloc_netns_lifecycle.rs` is `is_root()`-gated for exactly this reason | The end-to-end test **T2** is integration-lane + Lima + root, and no port design choice changes that. *(Rev 4: this is **not** a blocker. Per **DFS-8**, Lima+root integration tests participate in the mutation gate, so T2's lane is a wall-clock cost, not a coverage cost. The `WorkloadNetns` port that would lift it is out of scope, with its verified home at [#197](https://github.com/overdrive-sh/overdrive/issues/197) — no issue was created.)* |

---

## Key Decisions

| # | Decision | Alternatives rejected, and why | Source |
|---|---|---|---|
| **DFS-1** | **Adopt Candidate A** — extract an `MtlsIntercept` port trait as a 4th mandatory `Arc<dyn …>` on `MtlsInterceptWorker::new`; `action_shim` signatures UNCHANGED. **Justified by ONE thing: T2, the call-site-ordering test.** *(Rev 4 — the justification is restated; the decision is unchanged.)* The port makes it possible **at all** to assert that on an install failure the `StartAllocation` / `RestartAllocation` arms `return` **before** `driver.release_for_exit_emission(handle)` (`mod.rs:1307` before `:1319`; `:1507` before `:1519`), so a now-`Failed` alloc never releases its exit watcher. Nothing can force `start_alloc` to fail on demand today, so that security-relevant ordering is **wholly untested**. **Explicitly NOT justified by:** *(a)* killing the mutant — DFS-0a: killable today, default-lane, at zero production cost (verified: the helper is an `async fn` with no `pub` at `action_shim/mod.rs:413`, and that file's own `#[cfg(test)] mod tests` at `:1841` already reaches parent items via `use super::{…}` at `:1869`); *(b)* enabling a default-lane end-to-end test — DFS-0b: impossible without a second port (verified: `provision_and_inject_netns` at `mod.rs:830` short-circuits **only** on `mtls_worker.is_none()` at `:838`, so arming the mTLS seam unavoidably reaches `provision_workload_netns(&plan)` at `:855` and real `ip netns` shell-outs); *(c)* the boot probe — **struck**, OQ-4. T2 is Lima+root-gated, which is acceptable per **DFS-8**. | **(a) "T1 only — kill the mutant, skip the port."** Cheapest, and genuinely tempting after DFS-0a. Rejected: T1 is a *helper-level* test. It cannot assert the **call-site** properties — that `start_alloc`'s `Err` actually reaches the helper, and that `release_for_exit_emission` is never called for a now-`Failed` alloc (the `return` at `:1307` sits before the release at `:1319`). Removing the `exclude_re` entry on the strength of a test that never exercises the production call site is a green-suite-over-a-hollow-assertion outcome. It also permanently forecloses the port, because the mutant — the only forcing function — would already be dead. **(b) Candidate B** (port at the `action_shim` boundary): abstracts the wrong thing — the un-ownable surface is `libc`/`nft`, not `MtlsInterceptWorker`, which is code we own; mocking it means the test stops exercising the worker's real ordering + partial-teardown logic. Widens the blast radius to every dispatch call site for less coverage. **(c) Candidate C** (`#[cfg(feature)]` fault field): fails on its own terms — the default lane compiles *without* `integration-tests`, so the seam is invisible to the test it exists to enable; plus it is an optional-by-construction seam (the anti-builder rule) and production shaped by simulation. **(d) Accept the gap** (Cilium): moot per DFS-0a. | research Rank 1 / Findings 1.3, 4.1a/b, 6.1; `.claude/rules/development.md` § "Port-trait dependencies", § "Production code is not shaped by simulation" |
| **DFS-2 (Cilium)** | **The Cilium counter-precedent is acknowledged and overridden on a narrow, checkable ground.** Cilium (`//go:build privileged_tests`, `sudo make tests-privileged`) deliberately does NOT abstract its kernel surface, and it is the closest peer project in the world to this codebase. It is overridden because its model exercises privileged code **for success** and offers **no mechanism to make a privileged call fail on demand** — which is the entire requirement here. Under root the real install *succeeds*; there is no Cilium-shaped way to reach the fail-closed handler at all. The two approaches are **complements, not alternatives**, and this repo already runs both (real Tier-3 suites *and* `Sim*` adapters). | Rejected: citing Cilium as licence for "Tier-3 only" — that would be citing it for a capability it does not have. Also rejected: over-claiming hexagonal-architecture authority the other way. Cockburn states port granularity is *"largely a matter of taste"* with *"no particular damage in choosing the 'wrong' number"*, and the mutation-testing literature is **silent** on changing production code for killability (research Gap G1). **Neither literature is cited in this design as authority for extracting the port.** The justification is local: this repo's own port discipline + the security asymmetry. | research Finding 6.2, Conflict 2, Finding 1.1, Gap G1 |
| **OQ-1** | **3 methods**, not 1: `bind_transparent` / `install_outbound` / `install_inbound`. | **Single `install(spec) -> Result<InstalledIntercept, MtlsInterceptInstallError>`** (research Rank 2) rejected on a decisive structural ground the research did not surface: it moves `start_alloc`'s **ordering and fail-closed partial-teardown discipline** into the host adapter, so the sim adapter *replaces* logic **we own** — collapsing Candidate A into Candidate B in miniature (the very objection that sinks B, and DHH's "mocking what you own"). 3 methods keep the boundary exactly at the un-ownable `libc`/`nft` surface and keep the worker's ordering exercised by the test. Secondary gains: per-stage fault granularity lets T1/T2 assert the `stage` string (a specification assertion), and the realism criterion distinguishes `EPERM`-on-bind from `nft`-non-zero as different real faults. | research Finding 1.1 (no external criterion — a deliberate local judgement), Finding 6.1 (domain-shaped, not mechanism-shaped), Finding 5.3 |
| **OQ-2** | **SYNC.** No `#[async_trait]`. Exact signatures pinned verbatim in `architecture.md` § 4.1 — **three methods, no `probe()`** (rev 4, OQ-4): `fn bind_transparent(&self, addr: SocketAddrV4) -> Result<std::net::TcpListener>`; `fn install_outbound(&self, host_veth: &str, agent_leg_f_port: u16) -> Result<Box<dyn InterceptGuard>>`; `fn install_inbound(&self, virt: SocketAddrV4, agent_leg_c_port: u16) -> Result<Box<dyn InterceptGuard>>`. `Result` is the existing `crate::mtls_intercept::Result<T, E = InterceptError>`. | **`#[async_trait]`** rejected: every underlying primitive is a blocking syscall (`libc::socket`/`setsockopt`) or a blocking `std::process::Command` (`run_nft`/`run_ip` are sync fns), and `start_alloc` is itself `pub fn`, not `async fn` — making the port async would force `start_alloc` async and ripple for zero awaited I/O. The repo criterion recorded on `MtlsResolve` is *"async only where the contract genuinely awaits store I/O"*; nothing here awaits. Sync is also dyn-compatible with no `Pin<Box<dyn Future>>` allocation per install. **Generic `W: MtlsIntercept`** rejected: the type parameter would propagate virally through `MtlsInterceptWorker`, `AppState`, and `Option<&Arc<MtlsInterceptWorker>>` at every dispatch call site — a far larger blast radius than one vtable dispatch on a path that already spawns `nft`. **Function-pointer / closure field** rejected: an optional-by-construction seam, i.e. the anti-builder rule in another costume. | research Findings 4.2, 4.3, Gap G5; `crates/overdrive-worker/src/mtls_intercept.rs:1228,1255` (sync `run_ip`/`run_nft`) |
| **OQ-3** | **`overdrive-worker`** owns the trait — new module `crates/overdrive-worker/src/mtls_intercept_port.rs`, `pub mod mtls_intercept_port;`. Host adapter `HostMtlsIntercept` co-located. Sim adapter `SimMtlsIntercept` in `crates/overdrive-sim/src/adapters/mtls_intercept.rs` (module decl **and** `pub use mtls_intercept::{SimInterceptFault, SimMtlsIntercept};` matching the sibling re-exports at `adapters/mod.rs:70-71`), requiring `overdrive-worker.path` in `overdrive-sim`'s `[dependencies]`. | **`overdrive-core`** (where `MtlsResolve` lives) is rejected on a **trade-off**, not an impossibility *(overclaim corrected at review iteration 1)*. The half that IS structural: relocating `TproxyInterceptGuard` into `core` would drag its `Drop` — which shells out to real `nft` — onto a `crate_class = "core"` compile path, a direct dst-lint / ADR-0003 violation. The half that is a **judgement**: `InterceptGuard` is a brand-new, method-less, I/O-free marker trait this design invents and could live in `core` at zero dst-lint cost; core placement would then require **minting a new core-side intercept-error type duplicating `InterceptError`**. Reusing the existing typed error — and keeping the port beside the four functions it wraps — is worth the non-core placement. Stated as a trade-off so the alternative is foreclosed by reasoning, not by an asserted impossibility. **Sim adapter inside `overdrive-worker`** rejected: a `Sim*` type in an `adapter-host` crate's production `src/` is production shaped by simulation, and `.claude/rules/development.md` places sim adapters in `overdrive-sim`. **Precedent for the chosen shape already exists:** `SimViewStore` in `overdrive-sim` implements `overdrive_control_plane::view_store::ViewStore` — a port trait declared in an `adapter-host` crate, with `overdrive-control-plane.path` already in `overdrive-sim`'s `[dependencies]`. **Not a new edge class:** `overdrive-sim → overdrive-control-plane → overdrive-worker` means `overdrive-worker` is *already* in `overdrive-sim`'s normal dep graph; the `overdrive-worker [dev-dep]→ overdrive-sim` cycle likewise already exists today and Cargo resolves it. | ADR-0003 crate classes; `crates/overdrive-sim/Cargo.toml:50-57`; `crates/overdrive-worker/Cargo.toml:79`; `crates/overdrive-control-plane/Cargo.toml:59` |
| **OQ-4** | **NO — the port carries NO `probe()`, and there is NO boot gate. STRUCK from scope at user direction (rev 4).** *(Reversal of the rev-1…rev-3 answer, which said YES-but-strikeable. This row supersedes it; the earlier position is preserved in the rejected column for the record.)* Removed entirely: the trait method; `HostMtlsIntercept::probe`; the `InterceptError::Probe { reason }` variant (so `crates/overdrive-worker/src/mtls_intercept.rs` is **not edited at all**); the `MtlsBootError::InterceptProbe { source }` variant (so `crates/overdrive-control-plane/src/error.rs` is **not edited at all**); the `run_server` step-(4) gate (which now only *wires* the adapter); `SimMtlsIntercept::probe_failure` + `with_probe_failure`; the T3 probe cases; the T4 probe row; and the verification-catalogue graduation. **Consequence: `run_server` gains no new failure mode and this feature makes NO production behaviour change.** The three rationale legs: **(1)** it is a **production behaviour change** — `overdrive serve` refusing to start where it previously started — and therefore out of GH #250's scope, which is a fault-injection seam for an existing fail-closed path; **(2)** it buys a **better boot-time diagnosis, not a new safety property** (review iteration 1's own H1 finding — a capability-less node already fails every deploy at the upstream netns seam, since `ip netns add` / `ip link add type veth` need the same capability); **(3)** striking it **costs the design nothing**, because DFS-1 already rests on T2 alone. Per CLAUDE.md § "Deferrals require GitHub issues" this takes the **drop the deferral language** option: recorded as out of scope, with **no forward pointer, no promised future slice, and no issue number**. | **Keeping the probe** — the rev-1…rev-3 answer — rejected on the three grounds above. Its own strongest argument was that Earned Trust makes `probe()` a first-class responsibility for every driven adapter and that both sibling mTLS ports probe-and-refuse at the same site. That argument is **answered on the substance, not waived**: the capability this port depends on is already proven per-deploy at a seam running strictly upstream of every call to it, so the probe would re-prove at boot what the deploy path proves anyway. The two sibling ports probe substrates with no such upstream proof. The departure from ADR-0071's `wire → probe → use` invariant is therefore deliberate and recorded (ADR-0076 § Decision 4), not an oversight — the trait's own rustdoc carries a `# NO probe()` block telling a future crafter not to add one back without superseding that decision. Also still rejected, for the record: **the first draft's over-claim** that the probe was "the only production value the port carries with the tests deleted" (DHH's criterion) — source verification refuted it at review iteration 1. **A warn-only, non-refusing probe** — rejected then for leaving the wrong-cause diagnosis in place, and moot now. | user direction, rev 4; review iteration 1 H1; ADR-0076 § Decision 4; `action_shim/mod.rs:830-869`, `:1169`/`:1376`; CLAUDE.md § "Deferrals require GitHub issues" |
| **OQ-5** | **NOTHING blocks construction, and NO escape hatch is added.** The issue's premise is corrected: `MtlsInterceptInstallError::LegFBind(InterceptError::TransparentListener { .. })` compiles from `overdrive-control-plane` **today** (DFS-0a). The five private `const fn` constructors on `MtlsInterceptInstallError` **stay private** — the worker remains its only constructor, which is correct. `MtlsInterceptInstallError` gains **no** variant and **no** public constructor. The sim scripts `InterceptError` (public, not `#[non_exhaustive]`, all variants with public fields) and the worker maps it through the existing private constructors unchanged. | **`pub fn install_failed(...)` on `MtlsInterceptInstallError`** rejected: unnecessary (nothing is blocked) and it would widen a security-critical error type's construction surface for no gain. **`#[doc(hidden)] pub fn __for_test(...)`** rejected outright: public API in SemVer terms while pretending not to be, and a test-shaped hole in a production type — the same objection that sinks Candidate C, in miniature. **Adding a `From` bridge** rejected: `Inbound(#[from] InterceptError)` already exists and the other four variants deliberately use `#[source]` to keep their `Display` distinct. Note the research's own caveat holds: construction was never sufficient anyway — a test can build the error but still cannot make `start_alloc` *return* it. That is what the port is for. | research Finding 4.4 / Gap G4, verified against `mtls_intercept_worker.rs:119-254` and `mtls_intercept.rs:102-152` |
| **OQ-6** | **Cover BOTH arms with two test functions; do NOT collapse the duplication.** T2 ships `Action::StartAllocation` (`mod.rs:1307`) and `Action::RestartAllocation` (`mod.rs:1507`, seeded with a prior `Running` row) as separate cases. | **Collapsing the two byte-identical blocks into a shared helper** rejected: what is duplicated is a 6-line `if let Some(worker) && let Err(cause) = … { return …; }` guard whose *body* already delegates entirely to the shared `fail_closed_on_mtls_install`. The two blocks close over different locals (`workload_id`/`node_id` vs `prior_row.*`) and sit in different lexical contexts, so extraction would need a control-flow-signal return type — indirection for negative gain, and exactly what DHH's criterion warns against. Two test cases are the cheaper and more direct defense. Note the suppressed mutant is **one** whole-body mutant on the shared helper, killable from either arm; the second arm defends against a future divergent edit to one block, which is the real risk. | `action_shim/mod.rs:1304-1318` vs `:1504-1518`; `.cargo/mutants.toml:615` (function-name anchor) |
| **OQ-7** | **Assert ALL of them, split across two tests by what each can structurally reach.** **T1 (default lane, helper-level, TEN assertions):** `Ok(())` returned; a superseding `Failed` row with a strictly greater `updated_at.counter`; `TransitionReason::MtlsInterceptInstallFailed { stage, detail }` with the exact `stage` string and `detail == cause.to_string()`; `workload_addr: None` + `terminal: None`; `driver.stop` called exactly once; **`release_for_exit_emission` NEVER called**; exactly one `LifecycleEvent` with `from == prior_state`, `to == Failed`; **(A-8)** `alloc_id`/`workload_id`/`node_id`/`kind` byte-equal to the seeded `Running` row; **(A-9)** `started_at` byte-equal and NOT `None`; **(A-10)** the event's `source == TransitionSource::Reconciler`. Parameterised over all four `stage()` strings so the `stage()` match arms are themselves a killed mutation surface. **T2 (integration lane, call-site-level, 4 assertions × 2 arms):** the `Running`-then-`Failed` supersession through the real production guard; **`release_for_exit_emission` never called for the alloc**; `on_alloc_running` never called; **(A-9')** the net slot is RETAINED (a characterisation assertion — the fail-closed path returns before `teardown_and_release_netns`, and T2 is the first test able to observe it). | **(added at review iteration 1 — H3)** A-8/A-9/A-10 close a real gap: the helper forward-carries five fields out of the `Running` row into `build_alloc_status_row` (`:440-450`) and stamps a `TransitionSource` (`:457`), and the first draft asserted none of it — so a mutant replacing `running_row.started_at` with `None` survived all seven original assertions. Forward-carry drop is a NAMED bug class in this repo (it is why `workload_addr` became a required parameter — GH #248 / dial-by-name 02-02), and these are exactly the mutants that a **function-name-anchored** suppression removal newly exposes (see OQ-9). — Rejected alternatives: **asserting only the `Failed` row**: it leaves the gate-non-release — the subtlest, most security-relevant, and most easily-omitted property — undefended, and it is a distinct mutation target. **Asserting only in T1** rejected: the gate non-release at the *call site* is a property of the `return` placement (`:1307` before `:1319`), which a helper-level test structurally cannot reach; a reordering that releases first survives T1 entirely. **Asserting on internal worker state / call counts of components we own** rejected: that is the change-detector archetype Google names as *unproductive* — every assertion above pins the **specification** ("an install failure drives the alloc `Failed`, stops the driver, and never releases the gate"), observed at a port boundary (`ObservationStore` rows, `Driver` calls, the lifecycle bus). | research Finding 3.2; `.claude/rules/testing.md`; the PORT-TO-PORT litmus in `tests/acceptance/finalize_failed_forward_carries_workload_addr.rs:30-35` |
| **OQ-8** | **Split by lane, with the fault-arm limit stated openly.** *(Rev 4: the probe cases are gone from both, per OQ-4.)* *Default lane:* T3 — sim-only contract test (each `SimInterceptFault` materialises the pinned `InterceptError` variant; a standing fault fires on two consecutive calls; `clear_faults` disarms all three). All fault arms, zero I/O. *Integration lane (Lima + root):* T4 `mtls_intercept_equivalence` — drives `HostMtlsIntercept` and `SimMtlsIntercept` through the same sequence and asserts the **`Ok`-arm** observable contract (`bind_transparent(127.0.0.1:0) → Ok` with a NON-ZERO port and two calls yielding distinct ports; installs `→ Ok(guard)`, guard drops cleanly). **The fault arms are NOT equivalence-testable** — the host adapter cannot be made to fail on demand, which is the exact reason this port exists. They are pinned by the trait's four-section rustdoc contract plus T3. | **Claiming full host↔sim equivalence** rejected as dishonest — it cannot be delivered and asserting it would be aspirational. **Skipping T4 entirely** rejected: the `Ok`-arm contract (non-zero ephemeral port, distinct ports per call, clean guard drop) is real and is what the worker's ordering depends on. The gap is smaller than it looks: each `HostMtlsIntercept` method is a one-line delegation with no logic of its own to diverge. | `.claude/rules/development.md` § "The DST equivalence test is the structural guard"; research Conflict 2 |
| **OQ-9** | **Confirmed, and strengthened twice.** The `.cargo/mutants.toml` `exclude_re` entry `"fail_closed_on_mtls_install"` (`~:592-615`, with its justification comment) **and** the source-site `// mutants: skip` block (`action_shim/mod.rs:403-412`) are BOTH deleted in the **same commit as T1**. The gate is re-run scoped (`cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests --package overdrive-control-plane --file crates/overdrive-control-plane/src/action_shim/mod.rs`); on macOS read the **guest** `mutants-summary.json`, not the stale host artifact. **T1 alone must suffice** — *and rev 4 corrects why*: because of the **DELIVER ordering** (DFS-6), T1 lands in step 1, before the port and therefore before T2 exists, so on that commit T1 is the only test defending the function. It is **NOT** because a Lima-gated test fails to count toward the gate — per **DFS-8** it does count, and the earlier rationale ("the kill must not depend on Lima, root, or the `integration-tests` feature") was wrong on that point. **The obligation is EVERY mutant in the function at 100%, not the whole-body mutant at ≥80%** *(corrected at review iteration 1 — H4)*: the entry is a bare **function-name anchor** (its own comment says "the whole helper is uncovered, not just the whole-body mutant"), so deleting it un-suppresses every mutant cargo-mutants generates inside the function, while `cargo xtask mutants` scores **kill rate across the diff window** — satisfying "the whole-body mutant is caught" can still fail the PR gate. Assertions A-8/A-9/A-10 (OQ-7) exist to make 100% achievable; without them the forward-carry mutants survive. | **Removing the entry on T2's strength** rejected: T2 is Lima+root-gated, so the gate would silently depend on an environment the default mutation run does not have. **Keeping the entry as a permanent justified exclusion** (the Cilium branch) rejected per DFS-0a — the mutant is *productive* by Google's own taxonomy (not trivially equivalent; a test for it asserts the specification, not the implementation), and it is now killable for free. **Adding a NEW `exclude_re` entry for `HostMtlsIntercept`'s delegations pre-emptively** rejected: add one only if the diff-scoped gate actually reports them missed, and then with the standard justification comment naming T4 — matching the sibling `sweep_one_chain` / `list_named_chain` entries. | research Finding 3.2; `.claude/rules/testing.md` § "Mutation testing"; memory note on the macOS guest-path trap |
| **DFS-3** | **`InterceptGuard` marker trait + `Box<dyn InterceptGuard>` in place of the concrete `TproxyInterceptGuard`** on the port's install returns and on `AllocIntercept`'s two guard fields. | Necessary because a sim adapter must not return a real `TproxyInterceptGuard` — its `Drop` shells out to `nft` and would execute a real rule deletion from a test double. **Adding an `inert()` constructor to `TproxyInterceptGuard`** rejected: production code shaped by simulation, verbatim. **Making the guard field `Option`-of-something in production** rejected for the same reason. The marker trait is an honest domain abstraction — a guard's *entire* contract is its `Drop`, so a trait with no methods is the accurate shape, and a reader with no knowledge of the sim reads "the intercept port returns RAII guards the worker holds for the alloc lifetime." | `.claude/rules/development.md` § "Production code is not shaped by simulation"; `mtls_intercept.rs:1102-1148` |
| **DFS-4** | **Sim fault lifetime is STANDING (fires on every call while armed), not consume-on-use.** | Deliberately diverges from `SimMtlsResolve::script_resolve_fault`'s `.take()` shape, because the faults differ in kind: a poisoned store handle is transient, whereas a missing `CAP_NET_ADMIN` or an absent `nft` binary fails EVERY call. Standing faults also remove call-order dependence — `start_alloc` calls `bind_transparent` twice (leg-F then leg-C), and consume-on-use would make "which leg failed" an artifact of ordering rather than a test's explicit choice. Requires the `Clone` `SimInterceptFault` descriptor (`InterceptError` is not `Clone` — it carries `std::io::Error`), which also keeps the scripted faults expressed in **real** error shapes (`errno` / failing-command `reason`) per the realism criterion. | research Finding 5.3; `crates/overdrive-sim/src/adapters/mtls_resolve.rs:152` (the consume-on-use `.take()` is in the `resolve` impl, NOT in `script_resolve_fault` at `:130` — citation corrected at review iteration 1; the decision is unaffected) |
| **DFS-5** | **Sim `Ok`-arm of `bind_transparent` binds a REAL, PLAIN (non-transparent) loopback listener — and this is documented as pushing any test that drives it into the INTEGRATION lane.** | There is no way to fabricate a `std::net::TcpListener` without a syscall, and the worker's `Ok` path consumes a live listener it accepts on. **Returning a fabricated/`Option` listener** rejected (production shaped by simulation). The consequence is scoped, not hidden: the **default-lane** killer test T1 never touches the sim at all, and T3 drives only fault arms (which short-circuit before any syscall), so the default lane stays I/O-free. Tests that need the sim's `Ok` path (T2, T4) are integration-lane anyway for independent reasons (DFS-0b). | `.claude/rules/testing.md` § "Integration vs unit gating" — "Real network — binding sockets" |
| **DFS-6** | **DELIVER ordering: T1 + the two suppression deletions land as ONE step, FIRST — before the port extraction.** | T1 is independently valuable, independently gated, needs no production change, and de-risks the mutation contract from everything that follows. Bundling it with the port would make the gate's green depend on the port's correctness. | `architecture.md` § 9 |
| **DFS-8** | **Lima+root integration tests PARTICIPATE in the mutation gate — so T2's integration-lane placement is not a coverage compromise.** *(Added at rev 4; this is the fact that makes DFS-0b tolerable and that closes the "we need a second port" pressure.)* The canonical CI invocation is `cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests`, and **`cargo xtask lima run` runs as root by default**. `.claude/rules/testing.md` makes the Lima prefix *mandatory* for any mutation run carrying `--features integration-tests` **precisely because** without it the `#[cfg(target_os = "linux")]` surface is unreachable and *"the kill-rate gate becomes meaningless."* A Lima+root T2 therefore kills call-site mutants **in the real gate**. **Default-lane placement is a WALL-CLOCK property, not a COVERAGE one.** | Two consequences follow and are carried through the design. **(1)** OQ-9's "T1 alone must suffice" is re-grounded on the DELIVER ordering (DFS-6), not on a false claim that Lima-gated tests do not count. **(2)** Building a `WorkloadNetns` port to make T2 default-lane would buy wall-clock and local ergonomics, **not** gate coverage — a second, independent reason it stays out of scope here (see "Rejected in full"). Rejected framing: *"T2 is Lima-gated, therefore the fail-closed path is effectively ungated."* False — it is gated by the canonical CI mutation invocation. | `.claude/rules/testing.md` § "Mutation testing (cargo-mutants)" → Usage, § "Running tests — Lima VM"; CI per-PR job F |
| **DFS-7** | **The trait contract states ONLY what BOTH adapters can honour; substrate specifics live on `HostMtlsIntercept`'s own rustdoc.** *(Added at review iteration 1 — H5.)* Moved OFF the trait and onto the host adapter: `IP_TRANSPARENT` + `IP_FREEBIND` on the bound socket; "exactly ONE `nft` rule appended to the shared prerouting chain"; the shared-routing-infra convergence; "guard `Drop` removes that rule by handle". What REMAINS on the trait: a bound-and-listening listener with a NON-ZERO port when `addr` carried 0, distinct listeners per call, a guard owning exactly what its call acquired whose `Drop` neither panics nor errors, and nothing-acquired-outlives-an-`Err`. | The first draft stated the substrate specifics as **trait** postconditions — making the contract **unimplementable by half its sanctioned implementors** (the sim binds a plain listener and appends no rule, both by necessity per DFS-5 / DFS-3). `.claude/rules/development.md` § "Trait definitions specify behavior, not just signature" is explicit that adapters diverging on the same call means *"the bug is in the trait contract, not in either adapter — fix the trait docstring first."* Four rustdoc sections were present, so the rule was formally satisfied and substantively violated. The divergence was on the **Ok** arms — exactly where T4 claims equivalence — which is why T4 would otherwise have had to assert a WEAKER set than the contract stated. Post-split, contract and T4 assertion set **coincide exactly**, and the honest gap shrinks to the fault arms alone (OQ-8), which is what the design always claimed. | review iteration 1 H5; `.claude/rules/development.md` § "Trait definitions specify behavior, not just signature" |


---

## DFS-9 — the one production fix (added 2026-08-01, after DELIVER step 04-01)

| # | Decision | Alternatives rejected, and why | Source |
|---|---|---|---|
| **DFS-9** | **The superseding `Failed` row derives its LWW counter from the row it supersedes — and this feature therefore DOES make a production behaviour change.** DFS-1…DFS-8 and OQ-4 all rest on "no production behaviour change"; **that claim is withdrawn, not qualified.** Step 04-01's A-1' — the first test able to observe the fail-closed path through the real `action_shim::dispatch` — found that `fail_closed_on_mtls_install` builds its `Failed` row from the SAME `tick` and SAME `node_id` as the `Running` row it must supersede, so both carry a byte-identical `LogicalTimestamp` and `dominates` returns `false`. The `Failed` row is **silently dropped by both adapters**, leaving the alloc **durably recorded `Running` with no interception installed** — the exact surface this feature exists to defend. Fix: a new module-private `superseding_timestamp(tick, superseded)` returning `max(tick+1, superseded.counter+1)`, plus `build_alloc_status_row`'s `tick` parameter becoming a **required** `updated_at: LogicalTimestamp` so every writer decides its stamp explicitly. Character-exact surface: `architecture.md` § 4.8. Blast-radius audit: ADR-0076 § 7c — **only the two mTLS fail-closed arms are affected**; `fail_closed_on_netns_provision` is verified clean (pre-`Running` seam, fresh row, nothing to supersede). Lands as DELIVER **step 04-02**, un-ignoring A-1' on both arms. | **(a) Change `LogicalTimestamp::dominates` to let an equal-`(counter, writer)` incoming row win.** Rejected — it would be a bug: equal timestamps genuinely are not newer, and the LWW idempotency case (re-delivered gossip is a no-op) depends on `false`. It would also make acceptance order-dependent across gossip replay. The comparator is the SSOT both adapters consult and it is correct; the shim's counter assignment was wrong. **(b) Reorder — install the intercept BEFORE writing the `Running` row** (Alt-J). Makes the collision structurally impossible, and is tempting on principle. Rejected as a materially larger behaviour change than the defect requires: it deletes an observable durable transition, leaves a spawned driver with NO row if the `Failed` write then fails, and changes what a concurrent reader sees on every successful start. The chosen fix **restores** the behaviour the design already claimed rather than redesigning the sequence. **(c) A per-write monotonic `AtomicU64` in the shim** (Alt-K). Rejected: threads mutable state through `dispatch` (the signature DFS-1 deliberately keeps unchanged) and is **restart-unsafe** without seeding from the store's high-water mark — which is itself an out-of-scope systemic finding. **(d) Synthesize a distinct/advanced `TickContext` for the second write** (Alt-L). Rejected: the tick is one per-evaluation snapshot whose `now`/`now_unix`/`deadline` the whole reconcile path reads for consistency; fabricating a second tick to move one counter is a lie about which tick the write belongs to. **(e) Patch `row.updated_at` after building.** Rejected: that is precisely the shape `build_alloc_status_row`'s own `workload_addr` comment rejects for the identical bug class (GH #248). | `action_shim/mod.rs:429`, `:1239`, `:1457`, `:1728`; `overdrive-core/src/traits/observation_store.rs:261`; `worker/exit_observer.rs:541-544` (the precedent); ADR-0076 rev 5 § Decision 7 |

**Three systemic LWW findings surfaced by the audit are OUT OF SCOPE and were
NOT fixed** (ADR-0076 § 7d, recorded as observed facts — **no GitHub issue was
created; agents do not open issues unilaterally**):

1. **Cross-restart counter regression.** `tick_n` resets to `0` on every boot
   (`control-plane/src/lib.rs:2434`, bumped at `:2469`, ~864k/day at the 100 ms
   cadence) and is never seeded from anything persistent, while `alloc_status`
   rows ARE durable across restart (ADR-0012 § "Restart semantics"). Post-restart
   writes for a pre-existing alloc are therefore dropped by LWW until the tick
   counter catches up. **Verified from source; NOT reproduced at runtime.**
2. **Next-tick tie residual.** A same-tick supersede consumes counter `tick+2`,
   so a write on the immediately-following tick ties. Pre-existing in identical
   shape for the exit observer, whose module doc already documents it as
   accepted. DFS-9 does not close it.
3. **Other tick-derived rows.** `ServiceBackendRow` is written by two reconcilers
   both using `tick.tick + 1` and keyed on `service_id` alone (reachability not
   audited); `NodeHealthRow` uses wall-clock seconds, so two heartbeats in one
   second collide (benign at current intervals).

Findings 1 and 2 share one remedy — monotone-against-prior at every write site —
which is a larger decision than this feature. DFS-9's required `updated_at`
parameter is the enabling precondition for it, not a down payment on it.

---

## Rejected in full (recorded so they are not re-litigated)

- **Candidate C — `#[cfg(feature = "integration-tests")]` fault field on
  `MtlsInterceptWorker`.** Three independent fatal grounds: (1) the default lane
  compiles *without* the feature, so the seam is invisible to the very test
  #250 asks for; (2) an `Option<Fault>` defaulted to `None` is an
  optional-by-construction dependency — *"'optional' means 'tests can
  forget'"*; (3) it is production code shaped by simulation, by definition. The
  existing `ControlPlaneConfig::{mtls_probe_fault, dataplane_probe_fault}` do
  **not** rescue it — those are integration-lane seams for scenarios never
  claimed to be default-lane.
- **Moving `InterceptError` / `TproxyInterceptGuard` into `overdrive-core`** to
  let the trait live beside `MtlsResolve`. Relocating `TproxyInterceptGuard`
  would put a real-`nft` `Drop` on a `crate_class = "core"` compile path — an
  ADR-0003 / dst-lint violation. Relocating `InterceptError` is *possible* but
  would mean minting a duplicate core-side intercept-error type; reusing the
  existing typed error is worth the non-core placement. (See OQ-3 — this is a
  trade-off, not an impossibility; the first draft overclaimed.)
- **A `WorkloadNetns` port** to make T2 default-lane. Correct in principle, out
  of scope for #250 — and *(rev 4)* **closed against a verified existing home:
  [#197](https://github.com/overdrive-sh/overdrive/issues/197)** (veth →
  first-class network reconciler), whose **Scope item 1** reads verbatim: *"A
  network port trait (`Link`/`Address`/`Route` ops) with a `Host` adapter
  (netlink / `rtnetlink` / `ip(8)`) and a `Sim` adapter (in-memory HashMap)."*
  That **is** the `WorkloadNetns` port. Verified with
  `gh issue view 197 --comments`; **no new issue was created.** Two independent
  reasons not to build it here: **(1)** it would land #197's port **without the
  reconciler it exists to serve** and prejudge that design — the port's shape
  should be settled by the convergence semantics it is for
  (`.claude/rules/reconcilers.md` § Bar 2), not by what makes one
  fault-injection test default-lane; **(2)** per **DFS-8** it buys **no gate
  coverage** this design lacks, only wall-clock and local ergonomics.
- **The boot `CAP_NET_ADMIN` probe** *(rev 4 — struck; see OQ-4 for the full
  record)*. Not deferred, not tracked, no issue number: a production behaviour
  change out of #250's scope that buys diagnosis rather than protection.
  Recorded here so it is not re-proposed as an obvious omission — the trait
  carries a `# NO probe()` rustdoc block naming ADR-0076 § Decision 4 as the
  decision that would have to be superseded.
- **Claiming external authority.** Cockburn gives no criterion for port
  granularity (*"largely a matter of taste"*), and the mutation-testing
  literature is silent on changing production code for killability. Neither is
  cited here as authority for or against the extraction; the mutation result is
  used only as *evidence that an unasserted specification-level behaviour
  exists*, which is what research Finding 3.1 licenses it to do.

---

## ADR

**ADR-0076** —
`docs/product/architecture/adr-0076-mtls-intercept-port-fault-injectable-privileged-install-surface.md`.
An ADR is warranted: this adds a new port trait to the platform's port surface,
places it in a non-`core` crate (a first for the mTLS port family), adds a new
boot-refusal gate to the composition root, and adds a cross-crate dependency
edge (`overdrive-sim → overdrive-worker`). Verified unclaimed — the highest
existing ADR is 0075.
