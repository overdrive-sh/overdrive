# ADR-0076 — Extract the `MtlsIntercept` port over the privileged intercept-install surface, so the fail-closed call-site ordering becomes testable

## Status

Accepted. 2026-08-01 (rev 6, same day — three factual corrections to § 7c / § 7d
applied in place, mandated by ADR-0077 § D6); amended 2026-09-01 by
ADR-0089 §7 for the action-shim allocation-lifecycle boundary only.
Decision-makers: Morgan (nw-solution-architect, DESIGN wave for GH #250). Mode:
propose. Tags: phase-1, transparent-mtls, application-arch, port-extraction,
testability, fail-closed.

**Rev 2 changes** (review iteration 1 confirmed both load-bearing findings and
raised 5 HIGH issues, all resolved): the boot probe's justification is
**corrected and weakened** — it buys correct diagnosis, not new protection, and
this decision now rests on the call-site-ordering testability alone (§ Decision
4, § Consequences); the trait contract is **split** so it states only what every
sanctioned adapter can honour (§ Decision 1a); the mutation obligation is
restated as **100% of the function's mutants** (§ Decision 6); the call-site
count is corrected from "seven" to **1 production + 9 non-production across 8
files**; Alt-G is restated as a trade-off rather than an impossibility.

**Rev 3 changes** (review iteration 2 — verdict `approved`, 0 critical / 1 high,
all findings closed): the rev-2 probe correction is propagated into the
*verbatim* `run_server` comment and `MtlsBootError::InterceptProbe` rustdoc,
which still carried the refuted claim; the two `install_*` postconditions are
relativised to "this adapter's OWN substrate" (the last Ok-arm clause the sim
could not honour); the mutation obligation gains a mandatory
`cargo mutants --list` enumeration step with an explicit vacuous-pass caveat;
the call-site file count is corrected 8 → 7.

**Rev 4 changes** (user direction, same day — the *mechanism* is unchanged, the
*justification* and two scope items are not):

1. **The justification is restated honestly and narrowed to one leg.** Revs 1–3
   leaned partly on "the port kills the missed mutant" and on "the port enables
   an end-to-end test" without saying plainly that the ADR's own Findings A and
   B refute both readings. This ADR now rests on **the call-site-ordering
   testability alone** (§ Decision 6, T2) and says so once, without hedging.
2. **The `CAP_NET_ADMIN` boot `probe()` is STRUCK from scope entirely** — the
   trait method, the `HostMtlsIntercept::probe` impl, the `InterceptError::Probe`
   variant, the `MtlsBootError::InterceptProbe` variant, the `run_server` gate,
   and the sim's probe scripting all go (§ Decision 4). Rev 3 already recorded
   it as buying diagnosis rather than protection and as strikeable; the user
   struck it.
3. **The Lima+root mutation-gate fact is recorded** (§ Decision 6) — it is why a
   Lima-gated T2 is not a coverage compromise.
4. **The `WorkloadNetns` deferral now cites its verified home,
   [#197](https://github.com/overdrive-sh/overdrive/issues/197)** (Alt-F). No
   new issue was created.

**Rev 5 changes** (2026-08-01, same day — a REAL PRODUCTION DEFECT surfaced
during DELIVER step 04-01 and is fixed in-scope at user direction):

1. **This ADR no longer claims the feature makes no production behaviour
   change. That claim is now FALSE, and revs 1–4 asserted it.** Step 04-01's
   end-to-end test — the first test in the codebase able to observe the
   fail-closed path through the real `action_shim::dispatch` — found that the
   superseding `Failed` row **loses the LWW merge and is silently dropped**, so
   an mTLS install failure leaves the allocation durably recorded `Running`
   with no interception installed. The feature's own headline claim was unmet
   in production. § Decision 7 records the defect and the fix; § Consequences
   is corrected.
2. **A blast-radius audit of every same-tick supersede site is recorded**
   (§ Decision 7c), including three systemic LWW findings that are OUT of this
   feature's scope and were NOT fixed.
3. **DELIVER gains step 04-02** carrying the production fix and un-ignoring the
   step-04-01 assertion A-1' that is currently `#[ignore]`d against this defect.

**Rev 6 changes** (2026-08-01, same day — three statements in rev 5's
out-of-scope findings are factually wrong and are corrected in place, as mandated
by ADR-0077 § D6. The **decision is unchanged**; ADR-0077 amends this ADR and
does **not** supersede it):

1. **§ 7c's `fail_closed_on_netns_provision` row is corrected from "NO" to
   AFFECTED.** The clause rev 5 used to clear it — *"Any prior row is from an
   EARLIER tick, hence a strictly smaller counter"* — is precisely the premise
   the cross-restart counter regression falsifies. It is site 1 of ADR-0077 § D2.
2. **§ 7d finding 1 is upgraded from "NOT reproduced at runtime" to REPRODUCED
   end-to-end**, through real `overdrive serve` + `overdrive deploy` + restart,
   with four independent recovery-window measurement points
   (`docs/analysis/root-cause-analysis-cross-restart-lww-counter-regression.md`
   § 2, § 3 — the "RCA" below).
3. **§ 7d finding 3 is corrected on both counts.** The tick-derived set is **ten
   production sites across four row types**, not the single row type rev 5 named
   with reachability unaudited; and `NodeHealthRow` is **wall-clock-derived and
   immune**, not a member of that set.

Findings 1 and 3 now carry a forward pointer to **ADR-0077**, which owns the
remedy — closing the "no forward pointer" gap § 7d originally recorded as
deliberate. The pointer is an accepted in-repo ADR, not a promised slice, so
CLAUDE.md § "Deferrals require GitHub issues" is satisfied without an issue.

**Rev 7 changes** (2026-09-01, bounded amendment by ADR-0089 §7): the
lower-level three-method `MtlsIntercept` port and every worker-internal
ordering decision in this ADR remain unchanged. The former statement that the
action-shim signatures remain concrete, and Alt-B's blanket rejection of any
shim-level port, are superseded for one later responsibility: the action
shim's complete allocation `start_alloc`/`stop_alloc` orchestration now uses
the two-method `MtlsInterceptLifecycle` port pinned by ADR-0089 §7. This does
not permit an install-order test to replace the worker; it gives the distinct
same-ID cross-component ordering invariant a pure lifecycle boundary.

Feature record: `docs/feature/mtls-intercept-install-fault-seam/design/`
(`architecture.md` — verbatim API surface; `wave-decisions.md` — OQ-1…OQ-9).
Research: `docs/research/testing/fault-injection-seam-fail-closed-paths-research.md`
(29 sources).

Extends ADR-0071 (transparent-mTLS enrollment, Path A) — the `MtlsIntercept`
port wraps the install primitives ADR-0071 introduced. It does **not** extend
ADR-0071's `wire → probe → use` composition-root invariant: this port carries no
`probe()` (§ Decision 4). Supersedes nothing.

## Context

`MtlsInterceptWorker::start_alloc`
(`crates/overdrive-worker/src/mtls_intercept_worker.rs:495`) installs the
per-allocation transparent-mTLS intercept by calling three **concrete free
functions** in `crate::mtls_intercept`:

- `make_transparent_listener` — raw `libc::socket` + `setsockopt(IP_TRANSPARENT)`
  + `setsockopt(IP_FREEBIND)` + bind + listen. Requires `CAP_NET_ADMIN`.
- `install_outbound_tproxy` / `install_inbound_tproxy` — shell out to `nft` and
  `ip`.

On failure it returns the typed `MtlsInterceptInstallError`, and the action shim
drives the allocation to terminal `Failed` through
`fail_closed_on_mtls_install`
(`crates/overdrive-control-plane/src/action_shim/mod.rs:413`): it stops the
just-spawned driver, writes a superseding `Failed` row carrying
`TransitionReason::MtlsInterceptInstallFailed`, emits the lifecycle event, and
deliberately does **not** release the exit-emission gate.

Four facts about the live tree motivate this decision. Each was verified during
DESIGN.

1. **The error is production-reachable on the unconditional path.**
   `run_server` sets `compose_mtls = config.dataplane_override.is_none()`
   (`crates/overdrive-control-plane/src/lib.rs:1921`), which is unconditionally
   true in production, so `mtls_worker = Some(..)` (`:2043`) and
   `worker.start_alloc(&spec)` fires for every exec allocation
   (`action_shim/mod.rs:1305`, `:1505`). Its **first, ungated** step is the
   privileged transparent bind. Real faults that produce the error: a process
   without `CAP_NET_ADMIN` (`EPERM`), a kernel lacking `IP_TRANSPARENT` /
   `IP_FREEBIND`, `EADDRINUSE`, fd exhaustion, an `nft` binary absent from the
   appliance image, `nft` non-zero exit, ruleset lock contention, and
   `ip rule` / `ip route` shared-infra failures. This is **not** a #248-shaped
   test-only state (CLAUDE.md § "Ground the premise"): the path always runs in
   production and never runs off the mTLS gate.

2. **The fail-closed handler is a fail-open-when-broken security control.** If
   it no-ops, the allocation stays `Running` with **no mTLS interception
   installed** — a workload admitted to the mesh with no identity enforcement —
   and nothing alarms. Saltzer & Schroeder's fail-safe-defaults rationale names
   exactly this: a mistake in a mechanism that *excludes* access "tends to fail
   by allowing access, a failure which may go unnoticed in normal use." Yuan et
   al. (OSDI '14) found the majority of catastrophic distributed-systems
   failures traced to error-handling code — "the last line of defense."

3. **`cargo-mutants` proved the handler is unasserted.** The whole-body
   `replace fail_closed_on_mtls_install -> Result<(), ShimError> with Ok(())`
   mutant is MISSED, and is suppressed by an `exclude_re` entry in
   `.cargo/mutants.toml` (`~:592-615`) labelled "REMOVE when #250 lands." By
   Google's own productive/unproductive taxonomy the mutant is **productive** —
   it is not trivially equivalent (it changes observable state), and a test for
   it asserts the *specification*, not the implementation — so the suppression
   is not one the literature sanctions except as a temporary marker.

4. **There is no way to make the install fail on demand.** `mtls_worker` is
   threaded as `Option<&Arc<MtlsInterceptWorker>>` — a concrete type. Under
   root the real install *succeeds*. The worker already holds three injected
   ports (`MtlsEnforcement`, `MtlsResolve`, `Clock`) and **none of them is on
   the `start_alloc` install path**.

Two further DESIGN findings bound the decision and must be recorded, because
they change what this ADR is buying.

**Finding A — the mutant is killable today at zero production cost.** The
issue's stated blocker (`#[non_exhaustive]` + private constructors on
`MtlsInterceptInstallError`) is misdiagnosed. `#[non_exhaustive]` sits at the
**enum** level only (`mtls_intercept_worker.rs:120`); there is no per-variant
`#[non_exhaustive]`, and every variant is public with public field types. Per
the Rust Reference, enum-level `#[non_exhaustive]` blocks exhaustive *matching*,
not *construction* — so
`MtlsInterceptInstallError::LegFBind(InterceptError::TransparentListener { .. })`
compiles from `overdrive-control-plane` today. And
`fail_closed_on_mtls_install` is module-private, so an in-crate
`#[cfg(test)] mod tests` can call it directly with eight arguments that are all
constructible in the default lane with no I/O. A ~60-line default-lane unit test
kills the mutant with **no production change whatsoever**.

**Finding B — an end-to-end `dispatch` test cannot be default-lane, with or
without this port.** `provision_and_inject_netns` (`action_shim/mod.rs:830`) is
gated on the **same** `mtls_worker.is_some()` flag and runs **upstream** of the
mTLS install. Any test passing `mtls_worker: Some(..)` therefore triggers real
`ip netns` / veth shell-outs; without root the allocation is driven `Failed` by
the *sibling* `fail_closed_on_netns_provision` handler and `start_alloc` is
never reached. The end-to-end test is Lima + root-gated for reasons that have
nothing to do with this port.

### What this ADR is therefore justified by — one leg, stated plainly

Findings A and B each remove a justification the issue and the research assumed.
This ADR is **not** justified by either of them, and no part of the decision
below may be read as resting on them:

- **NOT "the port is needed to kill the `fail_closed_on_mtls_install` mutant."**
  Per Finding A the mutant is killable **today**, in the default lane, by an
  in-crate `#[cfg(test)]` test calling the module-private helper directly, at
  **zero production cost**. Verified: the helper is an `async fn` with no `pub`
  at `action_shim/mod.rs:413`, and that file's own `#[cfg(test)] mod tests` at
  `:1841` already reaches parent items via `use super::{…}` at `:1869`.
- **NOT "the port enables a default-lane end-to-end test."** Per Finding B that
  is impossible without a *second* port. Verified: `provision_and_inject_netns`
  (`mod.rs:830`) short-circuits **only** on `mtls_worker.is_none()` (`:838`), so
  arming the mTLS seam unavoidably reaches `provision_workload_netns(&plan)`
  (`:855`) and real `ip netns` shell-outs.

**The surviving justification, which carries this ADR alone:**

> The port makes the **call-site ordering property testable at all**. On an
> install failure the `StartAllocation` / `RestartAllocation` arms must `return`
> **before** `driver.release_for_exit_emission(handle)`, so a now-`Failed`
> allocation never releases its exit watcher. Nothing in the tree can force
> `start_alloc` to fail on demand today — `mtls_worker` is a concrete
> `Option<&Arc<MtlsInterceptWorker>>`, and under root the real install
> *succeeds* — so this security-relevant ordering is **wholly untested**. The
> port is the only mechanism that makes it exercisable. The test is Lima+root-
> gated (Finding B), which is acceptable for the reason recorded in
> § Decision 6.

**Two evidence caveats, recorded so no reader mistakes this for an appeal to
authority.** Neither is cited anywhere in this ADR as justification:

- **Hexagonal architecture does not mandate this boundary.** Cockburn states
  that "what exactly a port is and isn't is largely a matter of taste" and that
  "it doesn't appear that there is any particular damage in choosing the 'wrong'
  number of ports." There is no external criterion for port granularity; § Decision 1
  is a **local** judgement, argued locally.
- **The mutation-testing literature is silent on production seams added for
  killability** (research Gap G1). The field holds the program fixed and varies
  the test suite, so it neither sanctions nor forbids this. The mutation result
  (Context fact 3) is used **only** as evidence that an unasserted
  specification-level behaviour exists — never as sanction for the extraction.

## Decision

### 1. Extract a 3-method `MtlsIntercept` driven port, sync, in `overdrive-worker`

A new module `crates/overdrive-worker/src/mtls_intercept_port.rs` declares:

```rust
pub trait InterceptGuard: Send + Sync {}

pub trait MtlsIntercept: Send + Sync + 'static {
    fn bind_transparent(&self, addr: SocketAddrV4) -> Result<std::net::TcpListener>;
    fn install_outbound(&self, host_veth: &str, agent_leg_f_port: u16)
        -> Result<Box<dyn InterceptGuard>>;
    fn install_inbound(&self, virt: SocketAddrV4, agent_leg_c_port: u16)
        -> Result<Box<dyn InterceptGuard>>;
}
```

**Three methods, no `probe()`** — the boot probe is struck from scope
(§ Decision 4). "3-method" is now literal, where revs 1–3 said "3 methods + a
probe".

`Result` is the existing `crate::mtls_intercept::Result<T, E = InterceptError>`.
Every method carries the four-section rustdoc contract
(preconditions / postconditions / edge cases / observable invariants) mandated
by `.claude/rules/development.md` § "Trait definitions specify behavior, not
just signature"; the contract text is pinned verbatim in the feature's
`design/architecture.md` § 4.1.

**Three methods, not one.** A single `install(spec) -> Result<InstalledIntercept, _>`
would move `start_alloc`'s ordering and fail-closed partial-teardown discipline
into the adapter, so a test double would *replace* logic we own — collapsing
this decision into the shim-level substitution rejected for GH #250's
install-ordering evidence. Three methods put this ADR's boundary exactly at
the un-ownable `libc` / `nft` surface and keep the worker's ordering exercised.
ADR-0089 §7's later, separate complete-lifecycle port does not change this
worker-level contract.

**Sync, not `#[async_trait]`.** Every underlying primitive is a blocking syscall
or a blocking `std::process::Command`, `start_alloc` is itself `pub fn`, and the
contract awaits no store I/O — the criterion already recorded on `MtlsResolve`.
Sync keeps the trait dyn-compatible with no `Pin<Box<dyn Future>>` allocation
per install.

**`overdrive-worker`, not `overdrive-core`.** The trait names `InterceptError`
and `TproxyInterceptGuard`, both defined in `overdrive-worker`, and
`overdrive-core` sits below it in the dependency graph. Relocating
`TproxyInterceptGuard` into `core` would put its `Drop` — which shells out to
real `nft` — on a `crate_class = "core"` compile path, an ADR-0003 / dst-lint
violation. Relocating `InterceptError` is *possible*, so the choice is a
trade-off rather than an impossibility: core placement would require minting a
duplicate core-side intercept-error type, and reusing the existing typed error
beside the four functions the port wraps is worth the non-core placement. The
precedent for a port trait outside `core` already exists: `SimViewStore` in
`overdrive-sim` implements `overdrive_control_plane::view_store::ViewStore`.

### 1a. The trait contract states only what EVERY sanctioned adapter can honour

Substrate specifics are documented on `HostMtlsIntercept`, not on the trait:
`IP_TRANSPARENT` + `IP_FREEBIND` on the bound socket; "exactly ONE `nft` rule
appended to the shared prerouting chain"; the shared-routing-infra convergence;
"the guard's `Drop` removes that rule by handle". The **trait** states only:
a bound-and-listening listener whose `local_addr()` port is non-zero when `addr`
carried 0; distinct listeners per call; a guard owning exactly what its call
acquired, whose `Drop` neither panics nor errors; and nothing acquired by a
failing call outliving it.

This matters because the simulation adapter binds a plain listener and appends
no rule, **both by necessity** (§ Decision 5, § Decision 2). Stating the
substrate specifics as *trait* postconditions would make the contract
unimplementable by half its sanctioned implementors —
`.claude/rules/development.md` § "Trait definitions specify behavior, not just
signature" is explicit that adapters diverging on the same call means the bug is
in the trait contract, not in either adapter. Post-split, the equivalence test's
asserted set and the trait contract **coincide exactly**, and the honest
untestable residue is the fault arms alone.

### 2. `InterceptGuard` marker trait; the port returns `Box<dyn InterceptGuard>`

`impl InterceptGuard for TproxyInterceptGuard {}`. A guard's entire contract is
its `Drop`, so a trait with no methods is the accurate shape. This exists
because a simulation adapter must not return a real `TproxyInterceptGuard` —
its `Drop` would execute a real `nft` rule deletion. Giving
`TproxyInterceptGuard` an `inert()` constructor, or making the worker's guard
fields optional, would be production code shaped by simulation.

`AllocIntercept`'s two guard fields widen accordingly
(`Option<Box<dyn InterceptGuard>>`, `Vec<Box<dyn InterceptGuard>>`).

### 3. Fourth mandatory constructor parameter on `MtlsInterceptWorker::new`

```rust
pub fn new(
    enforcement: Arc<dyn MtlsEnforcement>,
    resolve: Arc<dyn MtlsResolve>,
    clock: Arc<dyn Clock>,
    intercept: Arc<dyn MtlsIntercept>,
) -> Self
```

Mandatory, appended last, no builder — `.claude/rules/development.md`
§ "Port-trait dependencies": *"a builder makes the dependency optional, and
'optional' means 'tests can forget'."* The compiler enforces every call site is
explicit.

`start_alloc` changes at exactly four lines (the three free-function calls
become `self.intercept.*`); its signature, error mapping, ordering, and
fail-closed partial-teardown discipline are unchanged.

**Historical GH #250 boundary, superseded only as stated here.** Revs 1–6 kept
`action_shim::dispatch` / `dispatch_single` unchanged, with `mtls_worker` as
`Option<&Arc<MtlsInterceptWorker>>`, so the install-failure test exercised the
worker's real ordering. ADR-0089 §7 now replaces that dispatcher parameter
with `Option<&dyn MtlsInterceptLifecycle>` for the distinct, complete
allocation-lifecycle responsibility. Production implements the new trait for
the same `Arc<MtlsInterceptWorker>` and delegates to the same inherent
methods; the worker constructor, this ADR's `MtlsIntercept` field, and all
worker-internal install/partial-teardown ordering remain unchanged. Tests of
the behavior owned by this ADR must still exercise the real worker rather than
substitute the lifecycle port.

### 4. NO boot `probe()` — the `CAP_NET_ADMIN` gate is struck from this decision's scope

`MtlsIntercept` carries **no** `probe()` method. There is no boot gate, no
`run_server` step, no `InterceptError::Probe` variant, no
`MtlsBootError::InterceptProbe` variant, and no probe scripting on the
simulation adapter. Revs 1–3 designed all of these; rev 4 removes them.

The rationale, recorded once so it is not re-litigated:

- **It is a production behaviour change, not a test seam.** `overdrive serve`
  would refuse to start where it previously started. That is a change to the
  binary's failure surface and it is **outside GH #250's scope**, which is a
  fault-injection seam for an existing fail-closed path.
- **It buys a better boot-time diagnosis, not a new safety property** — the
  correction rev 2 already made and rev 3 propagated. A capability-less node
  already fails **every** deploy at the upstream netns-provision seam
  (`action_shim/mod.rs:830`, gated on the same `mtls_worker.is_some()` flag and
  running at `:1169` / `:1376`, strictly before `:1305` / `:1505`), because
  `ip netns add` / `ip link add type veth` need the same capability. The probe
  would replace unbounded wrong-cause per-deploy refusals with one
  correctly-caused boot refusal. Real, but it is diagnosability, and it is not
  what #250 asked for.
- **Striking it costs the decision nothing**, because this ADR already rests on
  the call-site-ordering testability alone (§ Context, § Decision 6). Rev 3
  recorded the probe as "separable and strikeable"; this is that strike.

Per CLAUDE.md § "Deferrals require GitHub issues", this is recorded as **out of
scope**, not as a deferral: there is no forward pointer, no promise of a future
slice, and no issue number. Unstated knobs are out of scope by default.

This is a deliberate, recorded departure from the `wire → probe → use`
composition-root invariant that ADR-0071 established for `MtlsResolve` and
`MtlsEnforcement`. The departure is defensible on the substance rather than on
convenience: the capability this port depends on is already proven per-deploy
at a seam that runs strictly upstream of it, so the probe would be re-proving
at boot what the deploy path proves anyway. The two sibling mTLS ports probe
substrates with no such upstream proof.

### 5. `SimMtlsIntercept` in `overdrive-sim`, with standing per-method faults

`crates/overdrive-sim/src/adapters/mtls_intercept.rs`, adding
`overdrive-worker.path` to that crate's `[dependencies]` (not a new edge class —
`overdrive-sim` already reaches `overdrive-worker` transitively via
`overdrive-control-plane`).

Faults are scripted through a small `Clone` descriptor,
`SimInterceptFault::{TransparentListener { errno }, TproxyInstall { reason }}`,
expressed in the **real** error shapes the substrate produces rather than a
generic boolean flag. Faults are **standing** (fire on every call while armed),
not consume-on-use: a missing `CAP_NET_ADMIN` or an absent `nft` binary fails
every call, and standing faults remove call-order dependence (`start_alloc`
calls `bind_transparent` twice).

Fault arms short-circuit before any syscall and are therefore **pure**
(default-lane-safe). The `Ok` arm of `bind_transparent` binds a real, **plain**
(non-transparent) loopback listener — there is no way to fabricate a
`std::net::TcpListener` — so any test driving that arm is integration-lane.

### 6. Test and mutation-gate contract

- **T1 (default lane, no production change):** an in-crate `#[cfg(test)]` test
  module in `action_shim/mod.rs` calls `fail_closed_on_mtls_install` directly
  and asserts **ten** port-boundary observables, parameterised over all four
  `stage()` strings. Three of the ten (rev 2) pin the fields the helper
  *forward-carries* out of the `Running` row — `workload_id` / `node_id` /
  `kind` / `started_at` — and the stamped `TransitionSource`. Forward-carry drop
  is a named bug class here (it is why `workload_addr` became a required
  parameter, GH #248), and those mutants are exactly what removing a
  function-name-anchored suppression newly exposes. This is what kills the
  mutant.
- **T2 (integration lane, Lima + root):** end-to-end through
  `action_shim::dispatch` with `SimMtlsIntercept` fault-armed, for **both** the
  `StartAllocation` and `RestartAllocation` arms. Asserts the call-site
  properties T1 structurally cannot reach — chiefly that
  `release_for_exit_emission` is **never called** for a now-`Failed` allocation.
  **This is the test the port exists for** (§ Context).

  **Lima-gating T2 is not a coverage compromise — it participates in the
  mutation gate.** The canonical CI invocation is
  `cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests`,
  and `cargo xtask lima run` **runs as root by default**.
  `.claude/rules/testing.md` makes the Lima prefix *mandatory* for any mutation
  run carrying `--features integration-tests` precisely because without it the
  `#[cfg(target_os = "linux")]` surface is unreachable and "the kill-rate gate
  becomes meaningless." A Lima+root T2 therefore kills call-site mutants **in
  the real gate**. Default-lane placement is a wall-clock property, not a
  coverage one — which is also why closing Finding B with a second port
  (Alt-F) buys no gate coverage this decision lacks.
- **T3 (default lane):** sim contract test, fault arms only.
- **T4 (integration lane):** `mtls_intercept_equivalence` — host↔sim `Ok`-arm
  observable equivalence. Fault-arm equivalence is **not assertable** (the host
  adapter cannot be made to fail on demand — that inability is why this port
  exists) and is not claimed.
- The `.cargo/mutants.toml` `exclude_re` entry **and** the source-site
  `// mutants: skip` block are deleted in the **same commit as T1**, and the
  scoped gate is re-run. **T1 alone must suffice.** The reason is the DELIVER
  ordering, not a claim that Lima-gated tests do not count: T1 and both
  suppression deletions land as step 1, *before* the port exists and therefore
  before T2 exists, so on that commit T1 is the only test defending the
  function. (Rev 4 corrects the rev-1 rationale "the kill must not depend on
  Lima, root, or the `integration-tests` feature" — per the mutation-gate fact
  above, a Lima+root test does count in the gate.) **The obligation is every
  mutant inside `fail_closed_on_mtls_install` at 100%, not the whole-body mutant
  alone** (rev 2): the suppression is a bare *function-name anchor*, so deleting
  it un-suppresses every mutant in the function, while `cargo xtask mutants`
  scores kill rate across the diff window — catching only the whole-body mutant
  can still fail the PR gate.

### 7. The superseding `Failed` row must derive its LWW counter from the row it supersedes (rev 5)

**This is a production behaviour change, and it is the correction of a real
defect this feature's own test found.** It is recorded here rather than in a
separate ADR because the defect falsifies *this* ADR's headline claim: the
fail-closed control it exists to make testable did not, in production, produce
the durable record it promised.

#### 7a. The defect

`fail_closed_on_mtls_install` (`action_shim/mod.rs:403`) writes a superseding
`Failed` row that **does not dominate** the `Running` row it must replace, so
both `ObservationStore` adapters silently discard it. Three legs, each verified
in source:

1. `timestamp_for(tick, writer)` (`:1728`) returns
   `LogicalTimestamp { counter: tick.tick.saturating_add(1), writer }` — the
   counter derives **only from the tick**, with no per-write sequence.
2. The `Running` row (built at `:1239` / `:1457`) and the superseding `Failed`
   row (built inside the helper at `:429`) are constructed from the **same
   `tick`**, and the `Failed` row copies `running_row.node_id` as its writer.
   Both rows therefore carry a byte-identical `(counter, writer)`.
3. `LogicalTimestamp::dominates`
   (`overdrive-core/src/traits/observation_store.rs:261`) returns `true` on
   `Greater`, `false` on `Less`, and on `Equal` tiebreaks
   `self.writer.to_string() > other.writer.to_string()` — **same writer ⇒
   `false`**.

So `failed_row.dominates(running_row) == false`. This fires in
`SimObservationStore::apply_alloc_status` **and** in the production
`overdrive-store-local::apply_alloc_status_lww` that `run_server` wires via
`wire_single_node_observation`. Both return `Ok(())` to the caller, so the lost
write is indistinguishable from success at the call site.

**Operator-visible consequence:** an mTLS install failure leaves the allocation
**durably recorded `Running` with no interception installed**. The driver *is*
stopped and the `LifecycleEvent` *is* emitted, so no workload keeps running
uninstrumented — but the durable record lies, and that record is the exact
surface this feature exists to defend.

**Why it survived until DELIVER 04-01.** Step 01-01's helper-level test seeds
the `Running` row with `SEEDED_RUNNING_COUNTER = 0` and invokes the helper with
`TICK = 7`, so its assertion A-2 ("strictly greater counter") holds in the
fixture and never in production. The artificial different-counter shape masked
the defect. Only the real `dispatch` — step 04-01's A-1' — can observe it. That
is the port's value arriving from a direction this ADR did not anticipate, and
it is worth recording as such: the justification in § Context was
call-site-ordering testability, and what the call-site test actually caught
first was a durable-state defect.

#### 7b. The fix

A same-tick supersede derives its counter from **the row it supersedes**, never
from the tick. A new module-private helper beside `timestamp_for`:

```rust
fn superseding_timestamp(tick: &TickContext, superseded: &AllocStatusRow) -> LogicalTimestamp {
    let base = timestamp_for(tick, superseded.node_id.clone());
    LogicalTimestamp {
        counter: base.counter.max(superseded.updated_at.counter.saturating_add(1)),
        writer: base.writer,
    }
}
```

and `build_alloc_status_row`'s `tick: &TickContext` parameter becomes
`updated_at: LogicalTimestamp` — a **required** parameter, so every writer must
decide its stamp explicitly. Five call sites pass
`timestamp_for(tick, <node_id>.clone())` (byte-identical values to today); the
one supersede site passes `superseding_timestamp(tick, running_row)`.

Three properties of this shape, each load-bearing:

- **`max`, not a bare `prior + 1`.** It keeps `tick` live (so
  `fail_closed_on_mtls_install`'s arity is unchanged at 8 and no unused-parameter
  lint fires), it single-sources the tick base from `timestamp_for` so the two
  cannot drift, and it is correct even if a future supersede site's prior row
  is *behind* the current tick.
- **A required parameter, not a post-build patch.** This is the discipline
  `build_alloc_status_row` already applies to `workload_addr` for the identical
  bug class (GH #248: *"Making it a parameter rather than a defaulted `None`
  field kills the forgot-to-forward-carry bug class"*). Mutating `row.updated_at`
  after building is the shape that comment rejects.
- **It generalises rather than patching one site.** After the swap, every
  writer's LWW stamp is an explicit, reviewable decision at the call site — which
  is also the precondition for the systemic work in § 7c, should it be taken up.

Honest limit: this makes the bug class **visible and deliberate**, not
*impossible*. A future writer at a supersede site can still pass
`timestamp_for(...)`. The structural defense is the required parameter plus the
rustdoc on both helpers; there is no type that forbids the wrong choice.

The existing in-repo precedent is the exit observer, which has always derived
its successor row's counter from the prior row for exactly this reason
(`worker/exit_observer.rs:541-544`). The fix brings the shim into line with it.

`LogicalTimestamp::dominates` is **NOT** changed. The comparator is correct: a
row with an equal `(counter, writer)` genuinely is not newer, and its LWW
idempotency case (re-delivered gossip is a no-op) depends on that. The bug is in
the counter the shim assigns, not in the comparison. The comparator was promoted
out of the sim crate precisely so one definition governs both adapters
(`docs/feature/fix-observation-lww-merge/deliver/rca.md`).

#### 7c. Blast-radius audit — every same-tick supersede site

A second `AllocStatusRow` write for the same alloc, by the same node, within one
tick is the necessary condition. Every candidate was checked:

| Site | Affected? | Reason |
|---|---|---|
| `StartAllocation` `Running` (`:1269`) → `fail_closed_on_mtls_install` (`:446`) | **YES — the defect** | Both rows from the same `tick`; `Failed` copies `running_row.node_id`. |
| `RestartAllocation` `Running` (`:1482`) → `fail_closed_on_mtls_install` (`:446`) | **YES — the defect, second arm** | Identical mechanism through the same shared helper. |
| `fail_closed_on_netns_provision` (`:532`) | **YES — rev 6 correction; this cell read "NO"** | It has no *same-tick* supersede: it fires at the PRE-`Running` provision seam, strictly before `driver.start` and before any row write in that dispatch, and it builds a FRESH row from `alloc_id`/`workload_id`/`node_id`. But the clause that cleared it — *"Any prior row is from an EARLIER tick, hence a strictly smaller counter"* — is **false**, and is exactly the premise the cross-restart counter regression falsifies (finding 1 below, as corrected): after a restart the tick counter restarts at 0 while the surviving row keeps its pre-restart high-water mark, so an earlier tick does **not** imply a smaller counter. It is **site 1 of ADR-0077 § D2**, which owns the remedy. |
| `StartRejected` → `Failed` (`:1239`, `state == Failed`) | **NO** | It is the SAME single write — `state` is `Running`-or-`Failed` on one `build_alloc_status_row` call. No supersede. |
| `RestartAllocation` stop-half (`:1342`) | **NO** | Calls `driver.stop` only; writes no `Terminated` row. |
| `FinalizeFailed` (`:1091`) | **NO** | One write per `dispatch_single`; `prior_row` is from an earlier tick. |
| `StopAllocation` → `Terminated` (`:1585`) | **NO** | One write per `dispatch_single`. |
| Exit observer (`exit_observer.rs:602`) | **NO — and is the precedent** | Already derives `prior.updated_at.counter + 1` (`:541-544`). Immune by construction. |

**Only the two mTLS fail-closed arms are affected *as same-tick supersedes*, and
both are fixed by the single shared helper.** The netns-provision sibling does
write a fresh pre-`Running` row with nothing to supersede — but **rev 6
withdraws the conclusion that this makes it clean**. Its counter is still
tick-derived, so it is defective under the cross-restart mechanism of finding 1,
and ADR-0077 § D2 carries it as site 1. The audit above was scoped to the
same-tick collision; it was not, and did not claim to be, an audit of the
tick-derived rule itself.

#### 7d. Three systemic LWW findings — OUT OF SCOPE, not fixed, no issue created

The audit surfaced three further exposures in the same mechanism. None is caused
by this feature, none is fixed by it, and none is a deferral with a promised
slice. They are recorded as observed facts. **No GitHub issue was created —
agents do not open issues unilaterally.**

**Rev 6 — findings 1 and 3 as rev 5 wrote them are factually wrong and are
corrected below; finding 2 stands as written.** Both corrections now point
forward to **ADR-0077**, which owns the remedy. That does not retroactively make
them deferrals of *this* feature: the decision was taken in its own ADR, exactly
as the closing paragraph of this section anticipated, and the pointer resolves to
an accepted artifact rather than a promise.

1. **Cross-restart counter regression (the largest).** `tick_n` is
   `let mut tick_n: u64 = 0;` inside `spawn_convergence_loop`
   (`control-plane/src/lib.rs:2434`), incremented once per loop iteration
   (`:2469`) at the 100 ms `DEFAULT_TICK_CADENCE` — roughly 864,000/day — and is
   **never seeded from anything persistent**. `alloc_status` rows **are** durable
   (`observation_wiring.rs:39-42` opens `observation.redb` under the data dir;
   ADR-0012 § "Restart semantics" guarantees a row written before restart is
   returned after it), and nothing truncates the table at boot. So after a
   restart the writer's counter starts at 1 while surviving rows carry the
   pre-restart high-water mark, and every action-shim write for a pre-existing
   alloc is dropped by LWW until the tick counter catches up. Same-writer means
   the tiebreak cannot rescue it.

   **Rev 6 — REPRODUCED end-to-end; the runtime caveat is withdrawn.** Rev 5
   closed this finding with *"No test, comment, ADR or RCA in the repo
   acknowledges this. Verified from source; NOT reproduced at runtime — no
   existing restart test writes a post-restart row and asserts it wins."* All of
   that is now false. The defect was reproduced through the real binary —
   `overdrive serve` + `overdrive deploy`, kill, restart on one fixed `data_dir`
   — where the post-restart write was silently dropped and `overdrive job stop`
   returned **exit 0** against a store that still read `Running`, with **no**
   warning, error, or LWW-reject line anywhere in the boot log (RCA § 2). The
   recovery window is `≈ prior_counter` ticks — *a surviving allocation is
   unwritable for at least as long as the previous control-plane process was up*
   — confirmed at **four independent measurement points**: prior counters 4, 269,
   522, and a synthetic 6000 whose boundary was exact (tick 5999 loses, 6000
   wins) (RCA § 3). **ADR-0077** now owns the remedy; this is one of the ten
   sites in its § D2.
2. **Next-tick tie residual.** A same-tick supersede consumes counter `tick+2`,
   so an ordinary write on the *immediately following* tick (`tick+1` → counter
   `tick+2`) ties and is dropped. Narrow (it needs a write on the very next
   tick), and pre-existing in identical shape for the exit observer — whose
   module doc already documents it as accepted (*"the action shim's next tick may
   dominate it, but the broadcasted `LifecycleEvent` is the permanent record"*).
   The fix in § 7b does not close it.
3. **Other tick-derived rows.** `ServiceBackendRow` is written by two different
   reconcilers (`service_lifecycle.rs:860`, `backend_discovery_bridge.rs:392`),
   both using `tick.tick + 1`, keyed on `service_id` alone — structurally exposed
   to the same collision.

   **Rev 6 — the set is larger than this, and `NodeHealthRow` is not in it.** Rev
   5 left `ServiceBackendRow`'s reachability unaudited, named no other exposed row
   type, and grouped `NodeHealthRow` with the tick-derived rows. The audited set
   is **ten production sites across four row types**: `AllocStatusRow` (five sites
   — `action_shim/mod.rs:526`, `:1076`, `:1251`, `:1470`, `:1580`),
   `ServiceBackendRow` (the two named above), `ServiceHydrationResultRow`
   (`action_shim/dataplane_update_service.rs:126`, `:155`), and
   `ReconcileConflictRow` (`reconciler_runtime.rs:1444`). All are redb-persisted
   and LWW-guarded, so all inherit the regression in finding 1 (RCA § 4.1;
   enumerated with per-site remedies and prior-row availability in **ADR-0077**
   § D2). `NodeHealthRow` is **NOT** among them: its counter is
   **wall-clock-derived** (`clock.unix_now().as_secs()`,
   `overdrive-worker/src/node_health.rs:55`), so it does not reset when the
   process does and is **immune** to the cross-restart regression. Its
   two-heartbeats-in-one-wall-clock-second collision is a separate issue — still
   benign at current heartbeat intervals — and rev 5 was wrong to file it under
   the same mechanism.

Findings 1 and 2 share one remedy — making the counter monotone against the
prior row at **every** write site (`max(tick+1, prior+1)`), which also repairs
the restart regression because a prior-derived counter cannot regress. That is a
larger decision than this feature and belongs to its own ADR; § 7b's required
`updated_at` parameter is the enabling precondition for it, not a down payment
on it.

**Rev 6:** that ADR now exists and is accepted — **ADR-0077**, *"Every durable
observation write derives its LWW counter from the row it replaces, never from
the tick"*. It generalises § 7b to all ten sites (its § D2) and takes the shape
predicted here, with the tick demoted to a **floor** rather than the source. It
**amends** this ADR — the three corrections recorded under "Rev 6 changes" — and
does **not** supersede it: the decision above stands, and § 7b's
`superseding_timestamp` is the shape ADR-0077 generalises.

## Alternatives Considered

**Alt-J — reorder: install the intercept BEFORE writing the `Running` row.**
Would make the collision structurally impossible — with no second write there is
nothing to supersede — and is genuinely attractive on principle. **Rejected**:
it is a materially larger production behaviour change than the defect requires.
It deletes an observable durable state transition, leaves a spawned driver
process with *no* row at all if the `Failed` write then fails (today the
`Running` row is at least durable), and changes what a concurrent reader sees on
every successful start, not just the failing one. The fix in § 7b restores the
behaviour this ADR already claimed (*"LWW resolves the brief observed-`Running`-
then-`Failed` window to the latest write"*) rather than redesigning the sequence.
Recorded so the option is foreclosed by reasoning, not overlooked.

**Alt-K — a per-write monotonic sequence in the shim.** An `AtomicU64` bumped
per write. **Rejected**: it requires threading mutable state through `dispatch`
(a wide blast radius on a signature this ADR deliberately keeps unchanged,
§ Decision 3), and it is **restart-unsafe** without seeding from the store's
high-water mark — which is finding 1 in § 7d, i.e. the larger decision this fix
declines to prejudge.

**Alt-L — synthesize a distinct/advanced `TickContext` for the second write.**
**Rejected**: the tick is a single per-evaluation snapshot whose `now` /
`now_unix` / `deadline` the whole reconcile path reads for consistency
(`reconciler_runtime.rs:1318`). Fabricating a second tick to move one counter is
a lie about which tick the write belongs to, and it corrupts every other field
that rides on the same snapshot.

**Alt-M — change `LogicalTimestamp::dominates` to break the equal-`(counter,
writer)` tie in favour of the incoming row.** **Rejected**, and it would be a
bug: equal timestamps genuinely are not newer, and the LWW idempotency case
(re-delivered gossip must be a no-op) depends on `false`. It would also make
row acceptance order-dependent across gossip replay. The comparator is the SSOT
both adapters consult and it is correct; the shim's counter assignment is what
was wrong.

**Alt-A — T1 only: kill the mutant, skip the port.** After Finding A this is
the cheapest option and is genuinely tempting. **Rejected** because T1 is a
*helper-level* test: it cannot assert that `start_alloc`'s `Err` actually
reaches the helper, nor that the exit-emission gate is not released — the
latter being a property of the call site's `return` placement
(`mod.rs:1307` before `:1319`) that a reordering would break with T1 still
green. Removing a suppression on the strength of a test that never exercises
the production call site is a green-suite-over-a-hollow-assertion outcome. It
also permanently forecloses the port, because the mutant — the only forcing
function — would already be dead, leaving the ordering property untestable
indefinitely.

Note this ADR does **not** reject Alt-A by claiming it fails to kill the mutant.
It kills it (Finding A). It is rejected because killing the mutant was never the
point (§ Context).

**Alt-B — port at the `action_shim` boundary
(`Option<&Arc<dyn SomeInterceptPort>>`).** **Rejected for this ADR's install-
ordering test; superseded for ADR-0089 §7's later allocation-lifecycle
responsibility.** The un-ownable install surface remains `libc` / `nft`, not
`MtlsInterceptWorker`, so GH #250 evidence must not substitute the worker and
thereby skip its real install ordering and partial-teardown logic. That
reasoning does not reject every application port forever. The later BTR-3
contract asks the action shim to order a complete, already-owned allocation
lifecycle against driver and structural-network lifecycles. For that distinct
responsibility ADR-0089 §7 accepts the narrower
`Option<&dyn MtlsInterceptLifecycle>` parameter, implements it for the same
`Arc<MtlsInterceptWorker>`, and retains this lower-level port for worker tests.
The two seams therefore preserve, rather than trade away, their respective
production logic.

**Alt-C — `#[cfg(feature = "integration-tests")]` fault field on the concrete
worker.** **Rejected** on three independent grounds, the first fatal on its own:
(1) the default lane compiles *without* `integration-tests`, so the seam is
invisible to the very test #250 asks for; (2) an `Option<Fault>` defaulted to
`None` is an optional-by-construction dependency — the anti-builder rule in
another costume; (3) it is production code shaped by simulation, by definition,
and Cargo's own additivity guidance ("enabling a feature should not disable
functionality") flags a behaviour-changing fault knob. The existing
`ControlPlaneConfig::{mtls_probe_fault, dataplane_probe_fault}` do not rescue it
— those are integration-lane seams for scenarios never claimed to be
default-lane.

**Alt-D — accept the gap (the Cilium model).** Cilium, the closest peer project
to this codebase, deliberately does **not** abstract its kernel/netlink surface;
it gates privileged tests behind `//go:build privileged_tests` and runs them
under `sudo`. Read strictly that is a serious industry vote for "Tier-3 only,"
and this ADR overrides it. **The override is narrow and checkable:** Cilium's
model can *exercise* privileged code for **success**; it provides **no**
mechanism to make a privileged call **fail on demand**, which is the entire
requirement here. Under root the real install succeeds and the fail-closed
handler is unreachable. The two approaches are complements, not alternatives —
this repo already runs both (real Tier-3 suites *and* `Sim*` adapters at port
boundaries). Additionally, after Finding A the "accept the gap" framing is moot:
there is no gap to accept, because the mutant is killable for free.

**Alt-E — single-method `install(spec) -> Result<InstalledIntercept, _>`.**
**Rejected**: see Decision §1 — it relocates logic we own into the adapter, so
the sim replaces it.

**Alt-F — a `WorkloadNetns` port so the end-to-end test can be default-lane.**
Correct in principle (it is the only thing that would lift Finding B), **out of
scope** for this decision, and now **closed against a verified existing home:
[#197](https://github.com/overdrive-sh/overdrive/issues/197)** (veth →
first-class network reconciler). #197's **Scope item 1** is verbatim: *"A network
port trait (`Link`/`Address`/`Route` ops) with a `Host` adapter (netlink /
`rtnetlink` / `ip(8)`) and a `Sim` adapter (in-memory HashMap)."* That **is** the
`WorkloadNetns` port. Building it here would land #197's port **without the
reconciler it exists to serve** and prejudge that design from inside an
unrelated feature — the port's shape should be settled by the convergence
semantics it is for (`.claude/rules/reconcilers.md` § Bar 2), not by what makes
one fault-injection test default-lane. **No new issue was created**; #197 was
verified with `gh issue view 197 --comments`.

Note also that after the mutation-gate fact in § Decision 6, Alt-F buys **no
gate coverage** this decision lacks — a Lima+root T2 already kills call-site
mutants in the real gate. What Alt-F would buy is wall-clock and local
ergonomics.

**Alt-G — moving `InterceptError` / `TproxyInterceptGuard` into
`overdrive-core`** so the trait sits beside `MtlsResolve`. **Rejected on a
trade-off, not an impossibility** (rev 2 — the first revision overclaimed).
Relocating `TproxyInterceptGuard` genuinely is blocked: it puts a real-`nft`
`Drop` on a `core`-class compile path (ADR-0003 / dst-lint). Relocating
`InterceptError` is possible, and the new `InterceptGuard` marker trait is
method-less and I/O-free so it could live in `core` at zero dst-lint cost — so
the real cost of core placement is minting a duplicate core-side intercept-error
type. Reusing the existing typed error, beside the four functions the port
wraps, is judged worth the non-core placement.

**Alt-H — generic `W: MtlsIntercept` instead of `Arc<dyn …>`.** **Rejected**:
zero dispatch cost, but the type parameter propagates virally through
`MtlsInterceptWorker`, `AppState`, and `Option<&Arc<MtlsInterceptWorker>>` at
every dispatch call site — a far larger blast radius than one vtable dispatch on
a path that already spawns `nft` subprocesses.

**Alt-I — a public or `#[doc(hidden)]` constructor on
`MtlsInterceptInstallError`.** **Rejected**: unnecessary (Finding A shows
nothing blocks construction), and `#[doc(hidden)] pub fn __for_test` is public
API in SemVer terms while pretending not to be — a test-shaped hole in a
production type, the same objection that sinks Alt-C in miniature.

## Consequences

### Positive

- **The fail-closed security control's call-site ordering becomes testable at
  all.** This is the whole of what the decision buys. The worker's install can
  be made to fail on demand for the first time, which is the only way to
  exercise the ordering that keeps the exit-emission gate un-released for a
  now-`Failed` allocation.
- **The un-ownable `libc` / `nft` surface is named once, with a contract.** Three
  one-line delegations replace three scattered free-function call sites; the
  trait's rustdoc is the SSOT both adapters implement against.
- **The mutation suppression is removed and the mutant is caught in the default
  lane by T1** — which, per Finding A, needed no production change and would
  have been true without this ADR. Recorded as a consequence, not as a
  justification.
- **Consistent with the established shape.** `MtlsIntercept` is the fourth port
  on `MtlsInterceptWorker`. It is not an exception to the codebase's topology —
  except that it deliberately carries no `probe()`, which § Decision 4 records
  and justifies.

### Negative

- **ONE production behaviour change, and it is a defect repair (rev 5).** Revs
  1–4 claimed *"NO production behaviour change"*; that claim is **withdrawn**,
  not qualified. Per § Decision 7 the superseding `Failed` row now carries a
  counter derived from the row it supersedes, so it wins the LWW merge — where
  before it was silently dropped and the allocation stayed durably recorded
  `Running` with no interception installed. `build_alloc_status_row`'s `tick`
  parameter becomes a required `updated_at: LogicalTimestamp`; the five
  non-supersede call sites pass byte-identical values, so their behaviour is
  unchanged.

  Everything else in this ADR remains behaviour-preserving: with the boot probe
  struck (§ Decision 4), `HostMtlsIntercept`'s three methods are one-line
  delegations to the free functions `start_alloc` already called, and
  `run_server` gains no gate. Revs 1–3 carried a refuse-to-boot behaviour
  change; rev 4 removed it; rev 5 adds only the defect repair.

- **The `no production behaviour change` framing shaped downstream artifacts
  that must now be read with rev 5 in hand.** The DELIVER roadmap's `notes`
  justify the absence of a `walking_skeleton_gate` partly on that claim, and
  step 04-01's test module `#[ignore]`s assertion A-1' against this very defect
  on the grounds that fixing it was out of scope. Step 04-02 lands the fix and
  un-ignores A-1'; the roadmap note is left standing because its *conclusion*
  (no scenario satisfies the walking-skeleton litmus) does not depend on the
  withdrawn premise — the fix is a durable-record repair, not a new
  operator-facing capability.
- **The `CAP_NET_ADMIN` misdiagnosis remains.** A capability-less node still
  refuses every deploy under `WorkloadNetnsProvisionFailed` rather than a cause
  naming the missing capability. This decision knowingly leaves that in place as
  out of scope (§ Decision 4); it is not tracked as a deferral.
- **One `Arc<dyn>` indirection and one mandatory constructor parameter**, with
  **1 production + 9 non-production** `MtlsInterceptWorker::new` call sites
  across 7 files to update (rev 2 — the first revision under-counted at
  "seven" and omitted
  `crates/overdrive-control-plane/tests/integration/alloc_netns_lifecycle.rs:118`).
  Every pre-existing site takes `HostMtlsIntercept`, which preserves today's
  behaviour byte-for-byte; only new tests wire the simulation adapter.
- **`Box<dyn InterceptGuard>` replaces the concrete `TproxyInterceptGuard`** on
  the worker's guard fields — one boxing per installed rule, on a path that
  already spawns `nft`.
- **A new cross-crate dependency edge** `overdrive-sim → overdrive-worker`
  (`[dependencies]`). Not a new edge *class* — `overdrive-sim` already reaches
  `overdrive-worker` transitively through `overdrive-control-plane` — but the
  direct edge is new and makes `overdrive-sim` name an `adapter-host` crate.
- **Fault-arm host↔sim equivalence is not assertable**, and this is a genuine
  gap. It is mitigated by the fact that each `HostMtlsIntercept` method is a
  one-line delegation with no logic of its own to diverge, and by the trait's
  rustdoc contract plus the sim contract test.
- **The end-to-end test remains Lima + root-gated** because of the upstream
  netns seam (Finding B). This decision does not fix that, and does not pretend
  to. It is a wall-clock and local-ergonomics cost, not a gate-coverage cost
  (§ Decision 6). Closing it belongs to
  [#197](https://github.com/overdrive-sh/overdrive/issues/197) (Alt-F).

### Neutral / non-consequences

- **`overdrive-core` is untouched.** No new trait, no new type, no new
  dependency — dst-lint scope is unchanged.
- **No new external integration**, so no consumer-driven contract tests are
  warranted. The only external dependency is the Linux kernel / `nft` surface,
  whose in-repo analogue of a contract test is the existing Tier-3 suite
  (`start_alloc_installs_both_tproxy.rs`, `bidirectional_walking_skeleton.rs`),
  which observes real `nft` state and real intercepted traffic.
- **No claim of external authority.** Cockburn gives no criterion for port
  granularity ("largely a matter of taste"), and the mutation-testing literature
  is silent on changing production code for killability (research Gap G1).
  Neither is cited here as justification; the mutation result is used only as
  evidence that an unasserted specification-level behaviour exists. This is
  restated in § Context so the caveat sits beside the justification it bounds.
