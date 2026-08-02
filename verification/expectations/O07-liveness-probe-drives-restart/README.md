# O07 — a declared liveness probe reaches the reconciler's restart decision

**Surface:** O (operator CLI) · **KPI:** K1 · **Status:** `pending`

<!-- Status rationale: evidence IS captured and executed in Lima (see
evidence/verification.yaml — executed_in_lima: true). The status is left
`pending` DELIBERATELY, not for want of a capture:

1. Per `.claude/rules/verification.md`, the agent that wrote the implementation
   companion, the fixtures and this runner MUST NOT also declare the expectation
   satisfied. The verdict belongs to a different-fox adversarial audit reading
   only `evidence/`.
2. The captured evidence is SPLIT — sub-claims 1-3 pass, sub-claim 4 is
   REFUTED (see § Evidence). The legend's `partial` and `broken` both require a
   linked issue, and creating a GitHub issue requires explicit user approval per
   CLAUDE.md § "Deferrals require GitHub issues". So the honest move is to
   record the split verbatim here and leave the verdict to the audit + the
   operator, rather than self-stamping any status.

Do NOT read `pending` as "not yet run". It was run; the results are below. -->

## Expectation

A Service that declares a `[[health_check.liveness]]` probe is **supervised for
liveness on the production path**, and the liveness observation **reaches the
reconciler's restart decision** — so the platform restarts an allocation whose
liveness probe is failing, and leaves alone one whose liveness probe is
passing.

Concretely, across three Services deployed to one `overdrive serve` and each
differing from the next by exactly one input:

| Fixture | Liveness probe | Expected `overdrive workload describe` |
|---|---|---|
| `examples/liveness-absent-service.toml` | none declared | `Restarts 0` |
| `examples/liveness-fails-service.toml` | targets a port nothing binds | `Restarts > 0` |
| `examples/liveness-holds-service.toml` | targets its OWN bound listener | `Restarts 0` |

The baseline attributes any churn to the liveness **declaration**; the control
attributes it to the liveness **outcome** (`.claude/rules/debugging.md` § 5 —
compare populations, not isolated failures).

### Why this is the ADR-0080 Stage 1 proof, and why the startup probe is load-bearing

All three fixtures declare **one startup probe**, and that is not decoration.
`project_probe_descriptors` concatenates the role vectors as
`startup ++ readiness ++ liveness`, and `ProbeRunner::start_alloc` used to
assign each descriptor's `probe_idx` from the **flat enumerate index** over that
concatenation — while every consumer filters per-role
(`role == Liveness && probe_idx == 0`). With a non-empty `startup_probes`
vector, the liveness probe therefore landed at flat index 1 and **no restart
decision ever saw it** (ADR-0080 § "A third instance, same class"). With
`startup_probes` empty it would have landed at flat 0 and matched by accident,
and these fixtures would prove nothing.

ADR-0080 § D1 restores the parser-assigned per-role `ProbeDescriptor.idx`;
§ D2 puts `role` into the durable observation key so the three roles' index-0
rows stop colliding under LWW. Together they make `probe_idx == 0` mean what
every consumer already assumed.

This is the **operator-surface** companion to that fix. The in-process tiers
could not have caught the defect: the readiness/liveness acceptance suites build
`ServiceAllocFact` by hand with `latest_readiness_probe: Some(...)` — a state
the production hydrate cannot produce (ADR-0080 § "Why nothing caught it"). Only
driving the built binary exercises the real hydrate boundary, which is precisely
the gap this catalogue exists to close.

- Anchor: ADR-0080 § D1 (`docs/product/architecture/adr-0080-probe-idx-is-per-role-and-stable-does-not-stop-the-supervisor.md` — restore `ProbeDescriptor.idx`, parser-assigned per role; `ProbeRunner::start_alloc` MUST consume `descriptor.idx` and MUST NOT derive one from `enumerate()`)
- Anchor: ADR-0080 § D2 (`role` joins the durable composite key `(alloc_id, role, probe_idx)`; D1-without-D2 is forbidden)
- Anchor: ADR-0080 § "A third instance, same class" (the liveness filter at `reconciler_runtime.rs:3046-3053` is broken identically and more severely — liveness-driven restart is dead on the production path for any Service with a startup probe)
- Anchor: ADR-0080 § "Why nothing caught it — the key cannot represent the contract" (no test exercises the hydrate filter; `service_lifecycle_readiness.rs` hand-builds a state production cannot produce)
- Anchor: ADR-0055 (`TerminalCondition::Stable` is NON-terminal; readiness and liveness are continuous post-Stable) — the contract D4 restores
- Anchor: ADR-0057 § 132-134 (`probe_idx` is the 0-indexed position within the per-role array) and `crates/overdrive-core/src/observation/probe_result_row.rs:159` (same contract, restated on the durable row)

## Verification

The runner brings up an **ephemeral control plane itself**, inside a single
root-context Lima invocation, with an EXIT-trap teardown sweep on every exit
path (kill serve, cgroup mass-kill + rmdir, XDP detach) and before/after no-leak
probes written into `evidence/`. `liveness-fails` restarts continuously by
design, so its cgroup scope churns throughout — the sweep is not optional.

`overdrive serve` boots the persistent workload-identity CA, so it needs a
key-encryption key. The runner supplies it through the **production**
systemd-creds delivery path (`$CREDENTIALS_DIRECTORY/overdrive-ca-root`), not a
test seam: no `SimKek`, no `with_credentials_dir`, no `OVERDRIVE_CA_KEK` dev
fallback.

Black-box only: the surface is the built `overdrive` binary and what the kernel
exposes (`ip`, `ss`, `bpftool`, cgroupfs). No `overdrive-*` crate is linked.

Sub-claims:

1. All three `overdrive deploy <spec> --detach` commands exit `0` and print
   `Accepted.`
2. **(Baseline)** `liveness-absent` — no liveness probe declared — shows
   `Restarts 0`. Without this, nothing below is attributable to liveness.
3. **(Positive)** `liveness-fails` — a declared liveness probe targeting an
   unbound port — shows `Restarts > 0`. The liveness observation reaches the
   restart decision at per-role index 0.
4. **(Control)** `liveness-holds` — a declared liveness probe targeting the port
   its own workload binds — shows `Restarts 0`. This is the sub-claim that
   separates *"liveness is consulted"* from *"liveness is consulted **and
   correct**"*.

`satisfied` requires all four on a Lima run, reviewed by a different-fox
adversarial auditor reading only `evidence/`.

## What the evidence does not prove

Recorded up front so no reader over-reads the capture:

- **Not ADR-0080 § D4** (`Stable` stops only the startup role). Crossing the
  `Stable` boundary requires the startup probe to PASS, and on this composition
  a TCP probe cannot reach the workload at all (see § Evidence). No allocation
  in this capture reaches `Stable`, so D4's per-role teardown is untouched by
  this evidence. The observation window is deliberately shorter than the 60s
  startup deadline so the baseline survives it; run longer and all three
  allocations are driven to `Failed` on startup instead.
- **Not the readiness headline.** ADR-0080's operator-visible headline is that a
  Service declaring `[[health_check.readiness]]` is permanently `MeshUnreachable`
  and NXDOMAIN. That is consumed at `mtls_resolve_adapter.rs` and
  `dns_responder/name_index.rs` — neither is reachable from the operator CLI.
  Proving it black-box needs a converged two-workload mesh deploy with the
  production workload-identity CA — the same **#227** (disposable full-system
  Lima VM EDD harness) on **#75** (Image Factory OS image) precondition that
  holds E04 and E05 at `pending`. O07 is deliberately scoped to what the built
  binary genuinely shows today rather than inflated to that claim.
- **Not the Probes render.**

  *At capture time (SHA `86d6331b`, 2026-08-02T09:45:01Z) — and this is what
  the pinned evidence was gathered against:* `overdrive workload describe` had
  no `Probes` section on the live path. `render::probes_section` and
  `format_workload_describe_json` were pure functions with acceptance tests and
  **no production call site**, and `AllocStatusResponse` carried no probe rows.
  So probe role / index / status were not directly observable, and O07 infers
  the liveness path from restart behaviour rather than reading it off a probe
  row. **That inference is what the evidence below rests on, and it is
  unaffected by anything that landed afterwards.**

  *Refreshed against the current tree (2026-08-02, uncommitted — per
  `.claude/rules/debugging.md` § "Refresh measurements when source changes"):*
  the surface now exists. `AllocStatusResponse` carries `probes` +
  `probe_results` (`crates/overdrive-control-plane/src/api.rs`), populated by
  `handlers::alloc_status` (`handlers.rs:1195-1238`), and
  `render::probes_section` has a real production call site via
  `render::workload_describe` (`crates/overdrive-cli/src/render.rs:275`, reached
  from `main.rs:173`). `format_workload_describe_json` is still dead (tests
  only). This closes the blocking precondition the sibling **O02** was `pending`
  on — see `../O02-alloc-status-probes-section/README.md` — though a TCP-mechanic
  fixture still cannot produce an honest `Pass` there for the same namespace
  reason documented below. **Re-capture O07 at current HEAD before citing its
  numbers as live**; this expectation is a snapshot pinned to `86d6331b`.
- **Not the liveness verdict's correctness** — see sub-claim 4 below, which is
  refuted.

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh O07` — SHA
`86d6331b`, `SEED=1`, `executed_in_lima: true`, `runner_exit_code: 1`
(see `evidence/verification.yaml`). The working tree was dirty (ADR-0080
Stage 1 is uncommitted); the diff is preserved verbatim at
`evidence/dirty-diff.patch` and the file list at `evidence/dirty-status.txt`.

The non-zero runner exit is **sub-claim 4 refuting**, which is data, not a
harness error.

### Observed — the whole population, both snapshots

| Fixture | Liveness probe | T+20s (gated) | T+40s (context) |
|---|---|---|---|
| `liveness-absent` | none | `Running` · **Restarts 0** | `Failed` · Restarts 0 |
| `liveness-fails` | → 9192, unbound | `Running` · **Restarts 38** | `Running` · Restarts 82 |
| `liveness-holds` | → 8093, its own listener | `Running` · **Restarts 39** | `Running` · Restarts 82 |

All three are `Running` at the gated snapshot, so the baseline's `Restarts 0` is
attributable to the absent liveness probe and not to terminality — the runner
asserts that confound guard explicitly.

### Per-sub-claim verdict

| # | Sub-claim | Verdict | Reason |
|---|---|---|---|
| 1 | all three deploys exit 0 + print `Accepted.` | **pass** | `deploy_{absent,fails,holds}.meta` each `# exit: 0`; each `.out` opens with `Accepted.` |
| 2 | baseline (no liveness probe) not restarted | **pass** | `describe_absent.out` — `alloc-liveness-absent-0  Running  0` |
| 3 | declared + failing liveness probe → restarted | **pass** | `describe_fails.out` — `Running  38`, with a `last terminated:` block per restart |
| 4 | declared liveness probe on its OWN bound listener → NOT restarted | **REFUTED** | `describe_holds.out` — `Running  39`. A workload whose listener is demonstrably serving is restarted just as hard as one whose liveness target does not exist. |

Verbatim, `evidence/describe_absent.out` vs `evidence/describe_fails.out` at the
gated snapshot — the two rows that carry sub-claims 2 and 3:

```
Alloc                    State        Restarts   Since
alloc-liveness-absent-0  Running      0          (c=4,w=local)
    reason: driver started
```

```
Alloc                    State        Restarts   Since
alloc-liveness-fails-0   Running      38         (c=99,w=local)
    reason: driver started
    last terminated: Terminated at (c=98,w=local) — stopped (by operator)
```

### What sub-claims 2 + 3 establish

A Service that declares a liveness probe is restarted; an otherwise-identical
Service that does not is untouched, in the same control plane, over the same
window. The **only** differing input is the `[[health_check.liveness]]` block.
So the liveness observation now reaches the reconciler's restart decision on the
real `overdrive serve` + `overdrive deploy` path.

That is the ADR-0080 § D1 fix, observed black-box. Both fixtures declare a
startup probe, so `startup_probes` is non-empty and — under the pre-Stage-1 flat
concatenated index — the liveness descriptor landed at `probe_idx == 1` while the
consumer filtered for `probe_idx == 0`. The restart in sub-claim 3 is not
reachable under that arrangement.

### Why sub-claim 4 is refuted — a namespace boundary, NOT an ADR-0080 regression

`evidence/namespace_reachability.txt` is the diagnosis, measured black-box on the
kernel surface in the same run:

```
=== ip netns list ===
ovd-ns-0002 (id: 74)
ovd-ns-0001 (id: 1)
ovd-ns-0000 (id: 0)

=== host-namespace TCP listeners on the fixture ports ===
(none of 8091/8092/8093/9192 listening in the host namespace)

=== netns ovd-ns-0002 TCP listeners ===
LISTEN 0      5            0.0.0.0:8093      0.0.0.0:*    users:(("socat",pid=2912655,fd=5))
=== netns ovd-ns-0001 TCP listeners ===
LISTEN 0      5            0.0.0.0:8092      0.0.0.0:*    users:(("socat",pid=2912614,fd=5))
=== netns ovd-ns-0000 TCP listeners ===
LISTEN 0      5            0.0.0.0:8091      0.0.0.0:*    users:(("socat",pid=2911538,fd=5))

=== host-namespace connect attempts (what a TCP probe would do) ===
  0.0.0.0:8091 REFUSED-OR-UNREACHABLE
  0.0.0.0:8092 REFUSED-OR-UNREACHABLE
  0.0.0.0:8093 REFUSED-OR-UNREACHABLE
  0.0.0.0:9192 REFUSED-OR-UNREACHABLE
```

Every workload IS serving — `ss` shows each `socat` LISTENing on its port — but
each socket lives inside the allocation's own network namespace, created by the
mTLS netns gate on the production path. A connect from the control plane's
namespace is refused for all three. So on this composition a TCP health probe
**cannot express "my workload is healthy"**: it fails identically for a serving
workload and for a port that does not exist.

Two consequences visible in this same capture:

- **Liveness.** `liveness-holds` restarts indistinguishably from
  `liveness-fails` — sub-claim 4.
- **Startup.** The baseline's own TCP startup probe is unreachable too, so its
  budget exhausts and it is driven to `Failed` between the two snapshots
  (`describe_absent_t40s.out`). The two liveness Services never reach `Failed`
  only because each restart resets their startup window.

**This is not caused by ADR-0080, and Stage 1 did not regress it.** Before Stage
1 the liveness observation was discarded before any decision consumed it, so an
unreachable liveness probe was inert. Stage 1 correctly connects the observation
to the restart decision — which is exactly why a pre-existing probe-reachability
gap becomes operator-visible for the first time. The refutation is evidence
*about the probe mechanic*, and it is reported, not repaired, here.

### Housekeeping

The run left the shared VM clean: `probe_before_loopback.txt` and
`probe_post_teardown_loopback.txt` both `HEALTHY`, and
`probe_post_teardown_cgroups.txt` shows `(no alloc-*.scope)` — non-trivial here,
since `liveness-fails` and `liveness-holds` churn a cgroup scope roughly twice a
second for the whole window.

## Open notes from adversarial audit (2026-08-02) — unresolved, recorded not repaired

Two findings against this expectation's own rigor. Both are recorded verbatim
rather than papered over; neither is fixed here, because fixing either requires
a fresh capture plus a different-fox audit (`.claude/rules/verification.md`).

**(a) Only ONE run is pinned — any "reproduced across consecutive captures"
framing is unsupported.** `evidence/` holds a single `verification.yaml`, a
single `run.log`, and a single set of `describe_*` outputs. The two timestamps
in the table above (T+20s / T+40s) are two *snapshots within one run*, not two
independent captures. Repetition across runs is therefore **not** established,
and no claim in this expectation may lean on it. (Verified against the
directory listing; no such claim currently appears in this README's text — the
note stands as a scope guard so one is not introduced by a future edit.)

**(b) Anchor independence is weaker than the `anchor_status: present` stamp
suggests.** Governing rule 3 requires an external contract that **predates**
verification. Of the six `- Anchor:` lines above, **four point at ADR-0080** —
and ADR-0080 was **untracked** at the pinned SHA `86d6331b`
(`git ls-tree 86d6331b docs/product/architecture/` lists no `adr-0080-*` file;
it was part of the same uncommitted Stage 1 working tree this run captured).
An anchor that ships in the same dirty tree as the thing it anchors does not
predate verification, so those four do not discharge rule 3.

The two anchors that **do** hold at `86d6331b` — both tracked, both
independently verified:

- ADR-0055 (`adr-0055-service-lifecycle-reconciler.md` — `Stable` as a
  non-terminal condition)
- ADR-0057 (`adr-0057-health-check-toml-spec.md` — `probe_idx` is the
  0-indexed position within the per-role array)

The anchors are **deliberately not deleted**: they are the correct pointers to
the contract under test, and removing them would hide the dependency rather
than disclose it. The honest reading is that this expectation rests on two
independent anchors plus four same-tree ones — which is a real weakening of
rule 3, and a reason a re-capture after ADR-0080 lands would be strictly
stronger evidence than this one.

### The `what, forever` witnesses

Per the Stabilize doctrine this expectation is the design-time `why`. The
regression alarms for the same contract are the Stage-1 in-process guards
ADR-0080 § D7 specifies — notably the hydrate-boundary integration test (D7.1),
the durable-key separation test (D7.2), the adapter-equivalence test (D7.3), and
`crates/overdrive-control-plane/tests/acceptance/stable_does_not_stop_probe_supervision.rs`
(D7.4). O07 is the black-box operator-surface complement those tiers under-serve
— and the reason it exists is that the tier which *should* have caught this
defect hand-built a `ServiceAllocFact` production cannot produce.
