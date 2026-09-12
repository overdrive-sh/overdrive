# E10 — HTTP startup status classes agree for Exec and VM Services

Status: `pending` (the retained corrected raw capture requires independent
evidence re-review)
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: no; bounded cross-driver mechanic matrix
Contract shape: `bounded-change` (current allocation state plus the retained
depth-one failure snapshot, restart count, probe result, and owned-resource complement)

## Expectation

Drive the same checked-in HTTP server through eight specs: Exec and VM crossed
with 204, 302, 404, and 503. Both drivers must make 204 Stable and must reject
302/404/503 as startup failures carrying the numeric status. Redirects are not
followed. Both 503 cells return the nonempty bounded response-body sentinel
`SVM-E10-FAILURE-BODY-MUST-NOT-LEAK`; that deletion-sensitive fixture must
appear zero times in operator-visible output.

The cleanup trajectory is incarnation-aware. A 204 Service is `Running` when
the example issues its operator stop and must finish `Terminated`. For
302/404/503, the control-plane NDJSON lane has already emitted its existing
`Accepted` wire event, but the accepted CLI presentation boundary renders that
acknowledgement only for the successful `Accepted` -> `Stable` summary. A
`Failed` terminal instead renders the existing terminal-only typed `Error:`
block naming the exact HTTP status (and `redirect not followed` for 302), then
exits exactly 1; its public PTY summary does not contain a separate `Accepted.`
block. Acceptance of the submission does not turn the terminal startup failure
into exit 0: ADR-0032 section 9 and ADR-0059 keep every Service `Failed`
terminal nonzero, while ADR-0093 and its approved DESIGN review preserve the
failure rendering boundary. The public describe independently retains the
failed probe's identity, configuration, status, and reason. Run
intent is still present, so the existing WorkloadLifecycle policy authorizes a
same-allocation recovery. The example polls through the old incarnation's
transient terminal row until public describe shows that same allocation
`Running` with a positive restart count, a depth-one prior `Failed` snapshot,
and the failed startup probe. A transient `Failed` or `Terminated` row is not a
settled no-recovery result and does not authorize a blanket terminal union.

Only after that recovered `Running` incarnation is observable does the example
record operator stop intent. Its current state must become `Terminated` with a
stopped reason, while the allocation ID, restart count, prior `Failed` snapshot,
and failed probe evidence remain preserved in a second observation after every
named allocation resource is absent. For that probe, preservation means the same
Startup/index-0 identity, HTTP method and target, failed status, and exact
failure reason; its `last_observed_at` may stay equal or advance when the same
failure is observed again, but must not regress. The accepted current session
may advance the stopped observation during teardown; a stale earlier session
must not replace the settled result with a crash-shaped observation.

Cleanup is an absolute complement, not a required positive delta. The post-stop
and final snapshots must contain no allocation cgroup, VM run directory,
allocation-owned hypervisor, or network name observed for the current session;
the final snapshot must also show the fixed materialization absent. When the
current-session snapshot observed a named resource, its disappearance is kept
as additional evidence. When cleanup won the observation race and that snapshot
was already clean, the same absolute absence remains sufficient and no synthetic
before-to-after delta is required. Whole-host deltas remain diagnostic only so
unrelated concurrent host activity cannot decide this allocation's result.

- Anchor: S-SVM-26 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-2 and K2 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: ADR-0090 explicit/default HTTP target semantics.
- Anchor: ADR-0093 and its approved DESIGN review, which make the CLI
  `Accepted` acknowledgement prefix success-only and preserve `Failed`
  terminal rendering.
- Anchor: ADR-0078 depth-one `last_terminated` plus monotone `restart_count` occurrence semantics.
- Anchor: ADR-0099 accepted Running publication before restart release.
- Anchor: ADR-0100 accepted-session ownership and stale-watcher refusal.

## Verification

The activated `http-status-cross-driver` mode runs all eight checked-in specs
against isolated built-product instances and retains
`raw-cells/e10-<driver>-<status>/` for every cell before deriving the eight-cell
ledger. Each raw directory keeps:

- the exact deploy command, PTY-visible stdout, wrapper stderr, PTY transcript,
  and process exit (0 for 204; 1 for 302/404/503); every failure transcript must
  carry its terminal-only typed `Error:` block and exact numeric HTTP cause and
  must not claim the success-only CLI `Accepted.` prefix;
- the before-stop public describe row, current externally observable row
  identity (`allocation_id`, current row stamp, restart count), failed-probe
  line, recovery observation trail, active kernel/resource snapshot, and any
  allocation-owned resource names actually observed before stop;
- the exact stop command, stdout, stderr, and exit, plus after-stop and
  post-runtime-cleanup public describe rows; and
- before/post-stop/final raw resource snapshots, the absolute named-resource
  complement, the whole-host diagnostic delta, the stop observation trail, and
  a comparison receipt derived from the retained public rows.

The accepted session's private `Arc<BeaconWriter>` identity is deliberately not
a public field (ADR-0100 D1/D2), so E10 does not invent one. Its black-box
session witness is the current allocation row plus its active owned resources.
Stale-session refusal is observed externally after those resources disappear:
the second public describe must still carry the same allocation ID, restart
count, depth-one prior `Failed`, the same failed probe semantics with a
non-regressive observation timestamp, and operator-stopped state. The seeded
and source-local ADR-0100 tests remain the independent proof of the private
pointer-identity comparison itself.

The derived ledger records driver, status, allocation ID, state and restart
count before and after stop, the failure surface, a post-cleanup settled state,
the exact trajectory, rendered probe result, raw stdout/stderr/describe byte
counts, sentinel occurrence counts for each operator surface, and named-resource
cleanup complement. Passing
requires the recovered-current/prior-failure pairing above in every failing
cell and the healthy first-incarnation pairing in both 204 cells, the same HTTP
status policy for both drivers, and zero
occurrences of `SVM-E10-FAILURE-BODY-MUST-NOT-LEAK` in deploy stdout, deploy
stderr, workload describe output, and the rendered probe-result field. The
ledger itself records only those zero counts, never the response body.
