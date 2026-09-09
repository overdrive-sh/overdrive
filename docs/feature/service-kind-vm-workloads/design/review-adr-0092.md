# DESIGN review — ADR-0092 marked host HTTP probe connector

## Metadata

| Field | Value |
| --- | --- |
| Feature | `service-kind-vm-workloads` / GH #257 |
| Artifact under review | `docs/product/architecture/adr-0092-marked-host-http-probe-connector.md` |
| Supporting amendment artifacts | `feature-delta.md`, `design/wave-decisions.md`, `brief.md`, and `c4-diagrams.md` |
| Reviewer | Fresh independent architecture reviewer |
| Review date | 2026-09-06 |
| Iteration | 1 |
| Verdict | **APPROVED** |

## Scope and contract checked

This review is limited to the user-authorized DESIGN remediation for the
reachable VM HTTP self-interception failure. It does not accept the existing
provisional implementation as-is. In particular, its leaf-pinned
`tower-service = "0.3.3"` is not the accepted manifest shape; the approved
implementation contract is the root workspace declaration plus
`tower-service.workspace = true` in `overdrive-worker` stated by ADR-0092.

The reviewed amendment authorizes exactly one private effect below the
unchanged `HttpProber` port: a cloneable
`tower_service::Service<hyper::Uri>` connector in `HyperHttpProber` that uses
the existing `MTLS_LEG_S_DIAL_MARK` as `SO_MARK` before a non-loopback
`connect(2)`. It expressly excludes new public methods, target-origin
metadata, descriptor or observation changes, persistence, routes/rules,
protocols, CLI/API, lifecycle policy, TCP changes, and Exec changes.

## Evidence and reachability

| Claim | Evidence reviewed | Assessment |
| --- | --- | --- |
| The failing path is production-reachable | `run_server` composes `HyperHttpProber` at `crates/overdrive-control-plane/src/lib.rs:1697-1705`; `VmDriver::on_alloc_running` calls the shared runner at `crates/overdrive-worker/src/vm_driver.rs:1874-1876`; `ProbeRunner::start_alloc` projects the task-local descriptor and `probe_tick` invokes the existing `HttpProber` at `crates/overdrive-worker/src/probe_runner/mod.rs:319-350,521-615`. | Pass. This is the real `serve` → `deploy` owner path, not a test-only assembly. |
| The unmarked connection is intercepted | `install_inbound_tproxy` appends the OUTPUT divert for the guest address at `crates/overdrive-worker/src/mtls_intercept.rs:374-397`; its `meta mark != MTLS_LEG_S_DIAL_MARK` condition excludes only the existing marked trusted dial. The earlier native-metal E08 trace records TCP startup pass but readiness `http GET .../ready` failing as `client error (SendRequest)` in `verification/expectations/E08-vm-service-guest-health/evidence/run.log:41-43`. | Pass. The observed failure and the exact rule predicate align. |
| The proposed effect is necessary and sufficient at this boundary | The candidate socket is marked before `connect(2)`, the only point at which the SYN traverses OUTPUT. The later native-metal E08 product trace shows the same projected readiness probe as `last=pass` in `evidence/product-run.out:43-46`; the checked-in guest fixture's `/ready` default is 204 in `examples/service-kind-vm-workloads/guest_server.rs:21-26,109-115`. | Pass. Marking after connect, or adding VM-origin data to the port, would not address the observed owner-path interception. |
| No lifecycle gate moves | ADR-0092's ownership table retains the action-shim/driver `Running`, startup `Stable`, readiness eligibility, liveness termination, and `WorkloadLifecycle` restart ownership. The real pass trace still stops later on the independently scoped Stable issue rather than reclassifying it as an adapter success. | Pass. The socket effect yields only an existing `ProbeOutcome`; it does not delay, revoke, or reinterpret `Running`. |

The captured E08 command still exits nonzero because the later Stable condition
is not reached. That does not weaken the narrowly claimed adapter evidence:
the trace separately records the readiness result as pass. ADR-0092 correctly
keeps that later lifecycle work out of this remediation.

## Architecture and contract assessment

- The decision is proportionate to the proven fault. It reuses the worker's
  existing mark constant and pre-existing OUTPUT exemption; it neither creates
  a second dataplane mechanism nor changes the guest-address projection from
  ADR-0090.
- The adapter-wide non-loopback rule is the correct private boundary. The
  unchanged port receives only a URL and timeout, so a VM-only method or
  target-origin field would be new public surface. An explicit destination is
  retained verbatim; only the socket mark selects the existing exemption when
  the matching divert already exists.
- The exact connector contract is implementation-feasible without a new
  public seam: `tower_service::Service<hyper::Uri>` is the legacy Hyper
  client's connector boundary, and a private module test can inspect its own
  socket mark. IPv4/IPv6 matching, pre-connect ordering, loopback non-marking,
  and no retained socket state are stated rather than left to crafter choice.
- The manifest boundary is exact and conforms to the repository dependency
  rule: root `[workspace.dependencies]` owns `tower-service = "0.3.3"` and
  the worker consumes it through `.workspace = true`. `tower-service` is
  already locked transitively; no additional package, feature, or resolution
  is authorized.
- Existing status classification, redirect refusal, response-body discard,
  explicit target behavior, and Exec behavior remain owned by their current
  contracts. ADR-0092 requires their existing coverage to remain in force
  rather than recasting the transport change as a new health policy.

## Findings

No blocking, critical, high, medium, or low findings.

## Verification obligations for the resumed DELIVER step

The following are acceptance obligations from the approved design, not new
scope:

1. Move the direct dependency to the exact workspace/worker manifest shape;
   do not retain the provisional leaf pin or add any other dependency.
2. Add private-adapter evidence that a non-loopback candidate has the existing
   mark before `connect(2)` and a loopback candidate remains unmarked, without
   adding a port method or test-only production seam.
3. Preserve the current HTTP status, redirect, timeout, and body-discard
   coverage; preserve all TCP and Exec behavior.
4. Re-run E08 through the built default-feature `overdrive serve` + `overdrive
   deploy` native-metal journey. It must show the guest `/ready` 204 as the
   projected readiness result `last=pass`, with no runner-installed target,
   route, listener, or mark.

## Remediation disposition

| Item | Disposition |
| --- | --- |
| `R02-01-2`: adapter/dependency change was outside ADR-0090 and the original step boundary | **Resolved by design.** ADR-0092 now authorizes only the private marked connector and the exact workspace dependency boundary. |
| Existing provisional connector implementation | **Not approved as-is.** The original `02-01` crafter must make it conform to ADR-0092, including the manifest boundary and the stated evidence obligations, before implementation re-review. |

## Verdict

**APPROVED.** ADR-0092 is a bounded, evidence-backed amendment that closes the
proven production HTTP self-interception gap without inventing public API or
expanding storage, dataplane, lifecycle, protocol, or operator surface. The
original `02-01` crafter may resume only for this exact approved remediation,
followed by the required implementation review.
