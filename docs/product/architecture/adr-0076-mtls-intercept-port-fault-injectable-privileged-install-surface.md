# ADR-0076 — Extract the `MtlsIntercept` port over the privileged intercept-install surface, so the fail-closed call-site ordering becomes testable

## Status

Accepted. 2026-08-01 (rev 4, same day — revised after user direction on scope and
justification).
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
this decision into the rejected "port at the action_shim boundary" shape. Three
methods put the boundary exactly at the un-ownable `libc` / `nft` surface and
keep the worker's ordering exercised.

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

**`action_shim::dispatch` / `dispatch_single` signatures are UNCHANGED** —
`mtls_worker` stays `Option<&Arc<MtlsInterceptWorker>>`. This is what keeps the
blast radius off every dispatch call site and is the property distinguishing
this decision from a port at the shim boundary.

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

## Alternatives Considered

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
(`Option<&Arc<dyn SomeInterceptPort>>`).** **Rejected**: it abstracts the wrong
thing. The un-ownable surface is `libc` / `nft`, not `MtlsInterceptWorker`,
which is code we own — mocking it means the test stops exercising the worker's
real ordering and partial-teardown logic. It also widens the blast radius to
every dispatch call site and the composition root for *less* coverage, and it is
the closest of all candidates to genuine test-induced design damage (hollowing
out a concrete collaborator purely so a test can substitute it).

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

- **NO production behaviour change.** With the boot probe struck (§ Decision 4),
  every production path behaves byte-for-byte as before: `HostMtlsIntercept`'s
  three methods are one-line delegations to the free functions `start_alloc`
  already called, and `run_server` gains no gate. Revs 1–3 carried a
  refuse-to-boot behaviour change; rev 4 does not.
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
