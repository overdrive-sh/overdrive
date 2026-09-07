# Design review — ADR-0094: marked host TCP probe sockets

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Decision under review | [ADR-0094](../../../product/architecture/adr-0094-marked-host-tcp-probe-sockets.md) |
| Review iteration | 1 |
| Reviewer | Fresh independent architecture review |
| Date | 2026-09-07 |
| Verdict | **APPROVED** |

## Scope reviewed

ADR-0094 is a deliberately narrow amendment to the private production
`TokioTcpProber` adapter. For each resolved TCP candidate it requires the
existing `MTLS_LEG_S_DIAL_MARK` before `connect` when the destination is
non-loopback, and requires a zero (unmarked) socket for loopback. The existing
`TcpProber::probe(&self, host, port, timeout)` trait method and all of its
operator-facing result and timeout behaviour remain unchanged.

The review also checked the stated exclusions: no new public TCP API or target
metadata, no descriptor or persistence change, no HTTP or UDP change, and no
nftables/routing, lifecycle, CLI, protocol, component, or dependency change.
UDP remains explicitly outside this decision and needs its own reproduced
failure and reviewed design before it can be changed.

## Reachability and evidence

| Evidence | Review result |
|---|---|
| Reproduced operator failure | The native-metal built-binary capture in [`E13` product output](../../../../verification/expectations/E13-vm-service-inferred-tcp-startup/evidence/product-run.out) records `overdrive deploy` of `zero-probes-failure.toml` exiting `0` after reporting the unbound inferred TCP `0.0.0.0:18998` probe as stable (lines 22-32). The same capture shows the allocation mTLS intercept installed before the false success (lines 48-51). This is a reachable production failure, not a test-only trace. |
| Probe owner path | The production composition root constructs `Arc<TokioTcpProber>` at `crates/overdrive-control-plane/src/lib.rs:1697-1703`. `VmDriver::on_alloc_running` calls `ProbeRunner::start_alloc` at `crates/overdrive-worker/src/vm_driver.rs:1874-1876`; `start_alloc` projects the task-local VM TCP target at `probe_runner/mod.rs:449-478`; and the probe tick invokes the existing TCP port. The current adapter uses an unmarked `TcpStream::connect` at `probe_runner/tcp_prober.rs:68-84`. |
| Existing exemption and mark identity | `MTLS_LEG_S_DIAL_MARK` is the established core constant in `crates/overdrive-core/src/dataplane/mtls_mark.rs:21`. The mTLS installer adds the OUTPUT-chain mark-accept exemption before its divert rule in `crates/overdrive-worker/src/mtls_intercept.rs:674-695` and `:782-808`; the netlink expressions compare that exact mark and accept it in `crates/overdrive-netlink/src/nft.rs:613-620`. The later per-workload OUTPUT divert only catches the unmarked dial. |

The complete live owner path therefore explains the observed result: an
unmarked host-side SYN to the projected VM address can be caught by the
existing OUTPUT TPROXY path and look like a completed TCP handshake even while
the guest listener is absent. Marking the private non-loopback probe socket
uses the already-installed recursion exemption and leaves loopback's existing
local semantics intact.

## Architecture assessment

The amendment conforms to the accepted VM target model in ADR-0090 without
altering it: `project_network_probe_target` already replaces a VM wildcard TCP
target with the provisioned `workload_addr`; ADR-0094 only ensures the actual
socket reaches that target. It neither re-projects descriptors nor introduces
target-origin plumbing.

Its implementation boundary is appropriate. Candidate resolution and socket
construction must stay private to `TokioTcpProber`; adding an origin argument,
a second prober method, a public socket abstraction, or persisted descriptor
state would diverge from the decision. Applying the mark before each
non-loopback candidate's connect, while preserving candidate order, timeout,
and the existing error-to-reason mapping, is sufficient to remove the proven
interception without changing TCP probe semantics. A mark-setting failure is
correctly constrained to the current ordinary I/O failure channel.

No lifecycle gate is added or moved. The Running action-to-driver handoff,
backend health readiness, and Service/Workload lifecycle ownership named in
the ADR remain the existing owners. The change has no dependency implication:
the worker already has the platform capability used by the existing HTTP
adapter; ADR-0094 authorizes no `Cargo.toml` modification.

praise: The ADR makes the causal boundary unusually clear: it anchors the
change to a real built-product false positive, names the precise OUTPUT
exemption being reused, and explicitly prevents the otherwise tempting HTTP,
UDP, target-model, and lifecycle expansions.

## Findings

No blocking, high, or medium findings. The requested private socket-mark
behaviour is authorized with an exact enough boundary for implementation and
does not conflict with ADR-0090 through ADR-0093, the architecture brief, the
feature delta, or the E09/E13 contracts.

## Required delivery evidence

Approval of the design is not evidence that the remediation has landed. The
original step 02-03 crafter must supply all of the following before delivery
review can approve the implementation:

1. Direct adapter tests proving a non-loopback TCP socket carries exactly
   `MTLS_LEG_S_DIAL_MARK` before connection and a loopback TCP socket remains
   unmarked. The tests must not install routes, addresses, listeners, or
   nftables effects to manufacture the result.
2. Preservation of the current TCP invalid-target, DNS, refused, timeout, I/O
   reason mapping, candidate order, timeout scope, and immediate post-handshake
   stream drop. No public `TcpProber` surface or persisted state may be added.
3. Current built-binary native-metal recaptures of E09's 100 healthy/unbound
   pairs and E13's bound/unbound inferred-TCP cases. E09 must demonstrate all
   100 healthy pairs pass and every unbound case fails; E13 must demonstrate
   the inferred unbound `18998` case reaches `StartupProbeFailed` rather than
   `Stable`.
4. A final scope audit confirming that HTTP, UDP, nftables rules/routes,
   lifecycle ownership, CLI/protocol surfaces, components, and dependencies
   were untouched. UDP is not an implementation follow-up for this step.

## Dispositions

| Item | Disposition |
|---|---|
| Proven false positive for the unbound inferred TCP probe | Resolved by the authorized private non-loopback socket mark, pending implementation evidence. |
| E09 TCP truthfulness matrix | Required re-capture after implementation; its current failure evidence cannot be treated as proof of the desired contract. |
| E13 inferred TCP contract | Required re-capture after implementation; the current native-metal capture is the regression proof for this decision. |
| HTTP mark implementation | Read-only precedent only; no modification authorized. |
| UDP probing | Explicitly rejected as out of scope pending independent reproduction and design review. |
| New public API, descriptor/persistence field, route/rule, lifecycle, or dependency | Not authorized; any such change is a design divergence. |

## Verification performed

This review independently read ADR-0094 and its ADR-0090–0093 context, the
architecture brief and feature delta, the design-wave decisions, E09 and E13
contracts, the E13 native-metal failure capture, and the production TCP prober,
VM driver, probe-runner, mTLS interception, netlink, and composition-root
paths cited above. No production code, tests, execution log, or unrelated
worktree content was changed as part of this design review.

## Verdict

**APPROVED.** ADR-0094 authorizes exactly the private `TokioTcpProber`
non-loopback marking behaviour needed to bypass the existing OUTPUT TPROXY
interception. The original delivery step may resume only within the stated
boundary and with the required direct and native-metal evidence.

