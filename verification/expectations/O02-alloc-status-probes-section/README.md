# O02 — `overdrive workload describe` renders a Probes section for a Service

**Surface:** O (operator CLI) · **KPI:** K4 · **Status:** `pending`

## Expectation

For a **Service** alloc with health-check probes, `overdrive workload describe
<job>` renders a **Probes** section: one row per probe, each showing role
(startup / readiness / liveness), probe index, a mechanic summary (e.g.
`tcp 0.0.0.0:8080`), last status, and last-observed timestamp. A probe with
no result yet renders `pending` (not blank); an inferred default probe
renders an `(inferred)` suffix. A **Job-** or **Schedule-**kind alloc renders
**no** Probes section at all.

- Anchor: S-SHCP-CLI-01..06 (Probes-section render contracts)
- Anchor: docs/feature/service-health-check-probes/discuss/outcome-kpis.md — K4

## Precondition change (2026-08-02) — the blocking gap is closed; a NEW one gates `Pass`

O02 was `pending` for one specific reason: **no probes surface existed on the
live path.** `render::probes_section` and `format_workload_describe_json` were
pure functions with acceptance tests and zero production callers, and
`AllocStatusResponse` carried no probe rows at all. That reason is gone.

**What now exists** (uncommitted at time of writing — `git status` shows
`api.rs`, `handlers.rs`, `render.rs` modified):

- `AllocStatusResponse` carries `probes: Vec<ProbeDescriptor>` (the
  declaration side, including the ADR-0058-synthesised default) and
  `probe_results: Vec<ProbeResultRowJson>` (the observation side) —
  `crates/overdrive-control-plane/src/api.rs`.
- `handlers::alloc_status` populates both, joining
  `ObservationStore::list_probe_results_for_alloc` per allocation row —
  `crates/overdrive-control-plane/src/handlers.rs:1195-1238`.
- `render::probes_section` has a **real production call site**:
  `render::workload_describe` calls it at `crates/overdrive-cli/src/render.rs:275`,
  and `main.rs:173` calls `render::workload_describe` on the
  `overdrive workload describe` path.

Still true and unchanged: **`format_workload_describe_json` still has no
production call site** (only `tests/acceptance/probes_section_render.rs`), so
the `--json` half of this surface remains dead.

**Why this still cannot reach `Pass` with the current fixture.** The runner
deploys `examples/quick-bind-service.toml`, whose only probe is a
**TCP-mechanic** startup probe. Under production `serve` + `deploy` a TCP probe
cannot reach the workload at all: the workload binds inside its own
`ovd-ns-NNNN` (the mTLS netns gate), while the probe runner connects from the
control plane's namespace and performs no `setns`
(`crates/overdrive-worker/src/probe_runner/mod.rs` — `http_probe_host` at :451
folds the bind-side wildcard `0.0.0.0` to `127.0.0.1`; the TCP mechanic goes
through it at :511). Measured black-box in
`../O07-liveness-probe-drives-restart/evidence/namespace_reachability.txt`.
So sub-claim 2 ("a probe row shows a role + mechanic summary") would render,
but every row's status would be a namespace artefact rather than an honest
health signal.

**An `exec`-mechanic probe is the only mechanic that can currently express
workload health on the production path** — an exec probe joins only the
workload *cgroup*, never a network or mount namespace, and is verified scored
correctly end-to-end (`exit 0` → 0 restarts, `exit 1` → 106 restarts from an
identical 19 executions) in
`docs/analysis/root-cause-analysis-probe-runner-exec-inert-and-ungated-restart-loop.md`
§ 2.2. A future capture wanting a truthful `Pass` should use an exec-probe
fixture, or wait for the netns gap to close.

**Status is unchanged and deliberately so.** Setting it requires a fresh
capture plus a different-fox adversarial audit per `.claude/rules/verification.md`;
this note records only that the *original* blocker is gone.

## Verification

Precondition: a control plane is reachable and a Service has been deployed
(the runner uses `examples/quick-bind-service.toml` — repo-root `examples/`;
`crates/overdrive-cli/examples/` does not exist). Note the fixture **does not
reach `Stable` under production `serve` + `deploy`** — it is driven to
`Failed { StartupProbeFailed }` at the 60s startup deadline for the
namespace reason above; it reaches `Stable` only in the in-process integration
tests, which pass `SimDataplane` as `dataplane_override` and therefore compose
no netns. See that fixture's own header for the full account. If the control
plane is unreachable the runner prints the `overdrive serve` +
`overdrive deploy` commands and exits `pending`.

The runner deploys the quick-bind Service, then runs
`overdrive workload describe <job>` through Lima and captures the render verbatim.
Sub-claims:

1. The render contains a `Probes` heading / section.
2. At least one probe row shows a role + mechanic summary (e.g. `tcp `).
3. A probe with no result renders `pending`, not a blank cell.
4. (Negative) Deploying `examples/coinflip.toml` (Job kind) and rendering its
   status shows **no** Probes section.

`satisfied` requires sub-claims 1–4 on a Lima run, reviewed adversarially for
"is the row actually legible to an operator?" (Step 4 — don't outsource taste).

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh O02`. Not yet run.
