# Evolution — remove legacy Exec workload driver (GH #293)

**Finalized:** 2026-09-15. **Feature slug:**
`remove-legacy-exec-workload-driver`. **Waves:** DESIGN → DISTILL → DELIVER →
FINALIZE; DISCUSS, SPIKE, and DEVOPS were intentionally absent for this
bounded greenfield deletion. **Completion record:**
[roadmap](../feature/remove-legacy-exec-workload-driver/deliver/roadmap.json),
[execution log](../feature/remove-legacy-exec-workload-driver/deliver/execution-log.json),
[approved DESIGN review](../feature/remove-legacy-exec-workload-driver/design/review-design.md),
[approved roadmap review](../feature/remove-legacy-exec-workload-driver/deliver/review-roadmap.md),
[step 02-02 review](../feature/remove-legacy-exec-workload-driver/deliver/review-02-02.md),
and [E14 evidence review](../feature/remove-legacy-exec-workload-driver/deliver/review-03-01.md).

## Business and problem context

The Exec workload driver was a temporary host-process compatibility adapter.
Keeping it after microVM execution became the supported appliance boundary
would have left an inaccurate product promise and a wide compatibility surface
across operator admission, HTTP types, archived intent, runtime dispatch,
lifecycle evidence, health probes, examples, and documentation. It also kept
host-process execution adjacent to security and isolation claims that are now
grounded in the VM guest boundary.

There were no production users or persisted-data compatibility obligations, so
the user approved a forward-only greenfield cut rather than legacy readers,
disabled variants, migration bridges, or retired-driver diagnostics. The work
removed the concrete host-process capability while preserving the existing
driver-neutral allocation lifecycle and the registry/index/observer routing
boundary needed by the supported microVM family.

The exact accepted contract remains in the
[feature delta](../feature/remove-legacy-exec-workload-driver/feature-delta.md).

## What shipped

Overdrive now admits only VM/microVM workload driver input. The TOML and HTTP
driver unions, durable workload intent, runtime payload, generated OpenAPI,
production composition, action dispatch, current examples, and active product
guidance no longer expose a live Exec workload-driver path. The public
`WorkloadSpecInput::exec_command` accessor was deleted without a replacement.

The concrete `ExecDriver`, its production composition helper, dedicated worker
tests, and its driver-specific netns error were removed. Production still
performs the existing probe-runner Earned-Trust gate, then conditionally
composes the VM capability: ordinary VMM absence may leave an empty
`DriverRegistry`, while a present but failing VMM probe still refuses startup.
`DriverRegistry`, `AllocDriverIndex`, the `Driver` port, and one exit observer
per composed driver remain the routing and lifecycle-ownership boundaries.

The host Exec health-probe mechanic was also removed. `ProbeRunner` retains its
HTTP and TCP mechanics, target projection, roles, thresholds, result rows,
supervision, cancellation, and re-registration behavior. Any future in-guest
command probe remains separately owned by GH #280 and inherits no host-cgroup
compatibility seam from this feature.

Runtime dispatch now follows the existing VM network path for every admitted
allocation. The current netns/veth/TAP/address/MAC/gateway/prefix/DNS inputs
remain because the shipped VM path uses them; this feature did not select or
implement the shared-switch, per-tap interception, shared-bridge DNS, or
transparent-mTLS replacement tracked by GH #295. The accepted order also
remains unchanged: current VM network provisioning and guest-ready driver start
precede the accepted `Running` row; transparent-mTLS installation follows that
row; installation failure writes the existing dominating `Failed` result and
withholds guest-command release.

Current operator examples and guidance were migrated to VM/microVM execution.
Host-process-only outcomes were retired, while the guest-stack E07 journey and
general UDP, dial-by-name, liveness, and probe fixtures were migrated to VM
artifacts. Retired E10 evidence remains historical and its executable entry
point fails closed. The current E12
oracle now requires the accepted predecessor identity at the terminal
observation and a distinct fresh successor after restart.

## Permanent decisions and product artifacts

The lasting decisions were created directly in the permanent architecture
namespace and were not copied during finalization:

- [ADR-0110](../product/architecture/adr-0110-microvm-only-live-driver-contract.md)
  makes live workload execution microVM-only while retaining tagged unions,
  registry/index routing, per-composed-driver exit observers, and the prior VMM
  absence/refusal split.
- [ADR-0111](../product/architecture/adr-0111-forward-only-exec-affected-spec-intent-envelopes.md)
  resets only `ServiceSpecEnvelope` and `WorkloadIntentEnvelope` to new,
  incompatible VM-only V1 families, with no historical reader or migration.
- [ADR-0112](../product/architecture/adr-0112-forward-only-exec-affected-lifecycle-envelopes.md)
  deletes the Exec-only lifecycle vocabulary and resets only
  `AllocStatusRowEnvelope` and `AllocLifecycleOccurrenceRowEnvelope` to their
  new current V1 baselines.
- [ADR-0113](../product/architecture/adr-0113-remove-host-exec-health-probe-surface.md)
  removes the host Exec probe surface, narrowly superseding the Exec clauses
  of ADR-0054 and the host-probe decision in ADR-0059 while preserving HTTP/TCP
  probing.

Two additional user-approved constraints were implemented without new ADRs:

- P-293-5 preserves the accepted P-105 contract exactly:
  `RestartAllocation.alloc_id` names the accepted numeric-current predecessor,
  `RestartAllocation.spec.alloc` names a distinct durably reserved fresh
  successor, successor outcome precedes the one exact-old cleanup attempt, and
  drivers do not choose identity.
- P-293-6 treats every GH #295 handoff item as a constraint on the state left
  by this feature, not as authority to implement the replacement dataplane.

The current architecture summary, C4 view, and outcome records already live at
their permanent homes:

- [architecture brief](../product/architecture/brief.md#microvm-only-workload-execution-after-exec-removal-gh-293-adr-0110011101120113)
- [C4 view](../product/architecture/c4-diagrams.md#microvm-only-workload-execution-after-exec-removal-gh-293)
- [outcome registry](../product/outcomes/registry.yaml), including
  `OUT-EXEC-REMOVAL-ADMISSION`, `OUT-EXEC-REMOVAL-FORWARD-SCHEMA`,
  `OUT-EXEC-REMOVAL-PROBES`, and
  `OUT-EXEC-REMOVAL-LIFECYCLE-PRESERVATION`
- [checked-in VM Service example](../../examples/service-kind-vm-workloads/)
- [canonical E14 expectation](../../verification/expectations/E14-vm-service-post-greenfield-cut/)

## Delivery record and commit lineage

All six roadmap steps completed their final COMMIT phase. The DES integrity
check passed. Explicitly skipped RED phases remain recorded rather than being
silently rewritten: step 01-02 records the missing native-metal kernel input,
and the initial documentation-only 02-02 cycle records RED as not applicable.
Subsequent 02-02 remediation cycles each record the phases they actually ran.

| Step | Delivered outcome | Implementation and closure lineage |
|---|---|---|
| 01-01 | VM-only admission and HTTP types; host Exec probe removal; new Service-spec and workload-intent V1 baselines | `4a0c94ae` |
| 01-02 | Concrete `ExecDriver`, export, composition helper, and dedicated tests removed while VM capability composition and registry ownership remain | `62c72748` |
| 01-03 | Runtime payload/network/interception dispatch collapsed onto the existing VM path without changing P-105 or GH #295 scope | `38627cf3` |
| 02-01 | Exec-only lifecycle source/reason vocabulary removed; allocation-status and occurrence envelopes reset to current V1 | `940f729b`; expectation repair `d2d6d854` |
| 02-02 | Current examples, verification entry points, README, whitepaper, jobs, journeys, and personas migrated or retired at the VM/microVM boundary, including the VM-only guest-stack E07 journey | `e36b8eb7`; remediations `2d330816`, `183cfd40`, `f1de51be`, `02803355`; corrective VM migration follows |
| 03-01 | Post-cut E14 native-metal evidence captured and independently approved | capture `9de728e5`; DES record `b5c3f3e8`; evidence approval `fb3eaae6` |

The preceding permanent-wave artifacts landed as DESIGN `db1e5e10`, DISTILL
`2944643f`, and the approved roadmap `ea3a908c`. Step 02-02's intermediate DES
and review-history commits are `910512ae`, `57e0c7c9`, `8c1b3339`, and
`71c23f52`; their complete dispositions remain in the native Markdown review
artifact rather than being flattened into a pass-only summary here.

## E14 post-cut evidence

[E14](../../verification/expectations/E14-vm-service-post-greenfield-cut/)
is the canonical SHA-pinned post-cut receipt. A different-fox reviewer approved
it as `satisfied` against source SHA
`3f628ed08865aa0cdb3fbacd1f2cf989707d4a7a` on qualified native x86_64 metal.
The capture drove only the checked-in healthy VM Service journey through the
built default-feature product boundary.

The successful second attempt records:

- Service deployment accepted and then `Stable`;
- passing guest-targeted TCP startup and HTTP readiness observations;
- a VM client Job reaching `Succeeded` after receiving the byte-exact
  `SVM-E08-GUEST-OK` reply through the Service frontend; and
- zero owned teardown deltas for VMMs, probe tasks, network state, cgroups, run
  directories, mounts, loop devices, and preparation state.

The first bounded attempt is retained append-only with exit 1 because teardown
also observed a signal-9 failed allocation. It is not presented as the
qualifying receipt. The second attempt exited 0 and is retained in both
`product-run.out` and `attempt-2-product-run.out`. Actual command, output,
substrate, source SHA, dirty state, guest artifacts, timestamps, and both
attempt dispositions remain with the expectation. The historical
[E06](../../verification/expectations/E06-vm-job-deploy-reaches-running/) and
[E08](../../verification/expectations/E08-vm-service-guest-health/) receipts
were not modified or used as proof of the post-cut implementation.

## Issues and lessons retained

- A removal must delete compatibility vocabulary without creating a new
  retired-driver contract. The corrected DESIGN removed a proposed dedicated
  parser error/message and prohibited tests whose sole subject was a deleted
  name, syntax, helper, variant, accessor, or historical payload. Surviving VM
  behavior and generic parser failures are the executable evidence.
- Schema reset scope must follow actual embedded vocabulary. Only four envelope
  owners were coupled to Exec, so resetting unrelated observation, CA,
  workflow, probe, or service-backend envelopes would have been unjustified
  churn.
- Lifecycle gate ownership must remain precise. `Running` does not promise
  that transparent-mTLS interception is already live; the latter gates the VM
  guest-command release and may supersede `Running` with the existing
  dominating `Failed` result.
- Deleting a host-process fixture can break a driver-neutral operator journey.
  Review found general UDP, dial-by-name, liveness, probe, and expectation
  consumers still pointing at removed files. Migrating those fixtures to VM
  preserved their actual product contracts.
- A retired runner that prints a message and exits 0 is testing theater. E10's
  retired runner and cleanup oracle fail closed, while E07 was restored as a
  VM-to-VM guest-stack journey with a real product runner.
- Current oracles must follow the accepted identity model even when historical
  evidence records older behavior. Review caught E12 requiring same-ID revival;
  its current contract now preserves the predecessor row and requires a fresh
  successor without rewriting the historical receipt.
- Active documentation and historical records have different migration rules.
  The README, whitepaper, journeys, personas, examples, and current expectation
  entry points were corrected; accepted ADR/evolution history and E06/E08 were
  deliberately preserved.
- The pre-existing schedule-racy P-105 fixture remained outside this feature.
  Its 39-pass/11-fail review sample was not reproducibly controlled by its
  printed seed, so it was neither changed nor used as #293 completion evidence.
- Black-box receipts and in-process tests remain independent evidence layers.
  E14 drove the built product and retained real output; Rust tests continue to
  own parser, codec, lifecycle, ordering, and internal composition guarantees.

## Mutation-gate disposition

The roadmap's final DELIVER gate was
`cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests`
with an 80% kill-rate requirement. The user explicitly directed that this
final mutation gate be skipped. No mutation command ran, no
`target/xtask/mutants-summary.json` was produced or retained for this feature,
and this evolution record claims no kill rate or passing mutation result. The
skip does not alter the completed step reviews or the independently approved
E14 evidence.

## Finalization migration and workspace disposition

The finalization destination map was evaluated. No temporary
`design/architecture-design.md`, `design/component-boundaries.md`,
`design/technology-stack.md`, `design/data-models.md`, feature-local
`design/adrs/ADR-*.md`, `distill/walking-skeleton.md`, or DISCUSS journey
artifact exists. Therefore no file required copying into `docs/architecture/`,
`docs/adrs/`, `docs/scenarios/`, or `docs/ux/`, and no duplicate permanent
artifact was created.

The canonical expectation and its evidence remain under
`verification/expectations/E14-vm-service-post-greenfield-cut/` as required by
the EDD retention rule. The feature workspace remains in place as historical
wave evidence. No workspace file, session marker, DES log, or temporary file
was deleted during this finalization pass.
