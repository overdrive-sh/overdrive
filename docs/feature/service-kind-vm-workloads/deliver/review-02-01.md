# DELIVER review — step 02-01: HTTP guest targeting

## Metadata

| Field | Value |
| --- | --- |
| Feature | `service-kind-vm-workloads` |
| Step | `02-01` |
| Commits reviewed | `8e85c0866fd1556d85e3edcbc0ee2e9b4bdc3ebd`; `96358b895888f45365fd8b54c9b879a0b20253b6`; `418642e140145dbbf9d23e589ae0c27679c9c3ce`; `894c13f79ddc6782e97fcd488c1da83969953bff` |
| Parents | `4b50eac1`; `10ae94f62cc159738c65fc1cd545ccfa13d69bca`; `418642e140145dbbf9d23e589ae0c27679c9c3ce` |
| Reviewer | Fresh isolated implementation reviewer |
| Iterations | 1–4 |
| Final verdict | **APPROVED** |

## Accepted contract

ADR-0090 and DESIGN ACD-1 require `ProbeRunner::start_alloc(&AllocationSpec)` to project an immutable task-local network target at registration: VM omitted HTTP hosts and `0.0.0.0` HTTP/TCP wildcards use that allocation's provisioned `workload_addr`; all explicit hosts and Exec defaults retain their existing semantics. The descriptor remains declared intent. This projection does not own `Running` or add VM health policy.

Roadmap step 02-01 additionally requires the delivered HTTP policy to remain intact: 2xx passes; 3xx, 4xx, and 5xx fail numerically; redirects are not followed; response bodies are bounded and hidden. Required scenarios are S-SVM-10, S-SVM-11, and S-SVM-14.

## Iteration 1 review

### Findings

#### R02-01-1 — issue (blocking): transitioned acceptance tests lack the required outcome anchor

The three tests transitioned from their RED scaffolds in this commit have a `CONTRACT_SHAPE` declaration but no required rustdoc line `Outcome anchor: DISCUSS Elevator Pitch`:

| Scenario | Location |
| --- | --- |
| S-SVM-10 | `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:254-259` |
| S-SVM-11 | `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:304-308` |
| S-SVM-14 | `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:419-424` |

Mechanical evidence: `rg -n 'Outcome anchor|CONTRACT_SHAPE' crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs` reports all eight `CONTRACT_SHAPE` declarations and zero outcome anchors. The reviewer role's Contract Shape Compliance gate requires both declarations for every transitioned acceptance test. This is a test-contract artifact defect, not a production behavior change.

**Remediation:** add the exact outcome-anchor rustdoc line to each of the three transitioned tests, without changing the accepted behavior, test assertions, or public API.

### Contract and production-path evidence

- `project_network_probe_target` is called only while `start_alloc` prepares its task-local descriptor clone (`crates/overdrive-worker/src/probe_runner/mod.rs:319-320`); the persisted `AllocationSpec.probe_descriptors` is not mutated.
- Its VM-only matrix is exact: TCP wildcard, HTTP omission, and HTTP wildcard are selected at lines 453-460; the guest address replaces only that clone at lines 470-477. Explicit hosts and Exec descriptors return untouched.
- `probe_tick` receives the projected clone, invokes the existing HTTP adapter, and writes only a `ProbeResultRow` (`crates/overdrive-worker/src/probe_runner/mod.rs:521-615`). It does not write an allocation status, so this step cannot own `Running`.
- The delivered production adapter still makes one GET, classifies the returned status directly, follows no redirect, and never consumes a response body (`crates/overdrive-worker/src/probe_runner/http_prober.rs:107-131`). Existing real-server integration coverage exercises 200, 503, and 302 behavior (`crates/overdrive-worker/tests/integration/probe_runner/real_http_probe.rs:63-129`).

No unsupported `Vm + None` behavior, new public API, lifecycle policy, persistent state, or test-only production wiring was introduced by this commit.

### Test integrity and scope

- The diff is limited to the expected production module, its acceptance test, and the DES log. The production change is necessary for the new VM HTTP projection; it is not fixture-only.
- S-SVM-10, S-SVM-11, and S-SVM-14 replace only their own RED-scaffold bodies with observable task/adapter/store assertions. No existing passing assertion was weakened or removed.
- The step has three named behavior groups and three transitioned acceptance tests, within the reviewer test budget of six.
- The `CONTRACT_SHAPE` values themselves are present and match the roadmap: bounded-change (S-SVM-10, S-SVM-14) and unbounded-preservation (S-SVM-11).

## Verification

| Command | Result |
| --- | --- |
| `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(vm_default_and_wildcard_network_probe_targets_resolve_to_workload_addr_once) or test(explicit_network_probe_hosts_are_preserved_for_both_drivers) or test(vm_http_probe_preserves_status_policy_and_bounded_body_handling)'` | PASS — 3/3 |
| `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance` | PASS — 96/96 |
| `git diff --check 8e85c086^ 8e85c086` | PASS |

## Iteration 1 remediation disposition

| Finding | Disposition |
| --- | --- |
| R02-01-1 | Open — return to the original step 02-01 crafter for the three rustdoc-only additions, then re-review. |

## Iteration 1 verdict

**REJECTED.** The production projection and behavioral assertions conform to the accepted ADR and roadmap, and the reviewed test runs are green. The required Contract Shape outcome anchors are absent on all three tests transitioned in this step; the reviewer cannot approve until that mechanical acceptance-test contract is restored.

## Iteration 2 review

### Remediation verification

Reviewed remediation commit `96358b895888f45365fd8b54c9b879a0b20253b6`
(`test(worker): anchor VM HTTP targeting outcomes`) against the iteration-one
finding. Its diff is limited to the three required rustdoc declarations:

- S-SVM-10 now has `/// Outcome anchor: DISCUSS Elevator Pitch` immediately
  after its `CONTRACT_SHAPE` declaration at
  `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:258`.
- S-SVM-11 now has the same declaration at line 308.
- S-SVM-14 now has the same declaration at line 425.

The remediation adds no production behavior, public API, test assertion, or
fixture changes. The iteration-one production/design, scope, and test-integrity
assessment therefore remains valid. The exact required declaration closes the
only mechanical Contract Shape Compliance defect.

### Finding disposition

| ID | Iteration-one disposition | Iteration-two result |
|---|---|---|
| R02-01-1 | Open — outcome-anchor declaration missing on S-SVM-10, S-SVM-11, and S-SVM-14 | **Resolved** — all three exact declarations are present in the remediation-only diff. |

### Iteration 2 verification

| Command | Result | Evidence |
|---|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(vm_default_and_wildcard_network_probe_targets_resolve_to_workload_addr_once) or test(explicit_network_probe_hosts_are_preserved_for_both_drivers) or test(vm_http_probe_preserves_status_policy_and_bounded_body_handling)'` | PASS | 3 tests run; 3 passed. |
| `cargo fmt --check` | PASS | Completed with exit status 0. |

## Iteration 2 final verdict

**APPROVED.** Iteration-two remediation is narrowly scoped, closes R02-01-1,
and the required focused acceptance verification remains green. No unresolved
findings remain for roadmap step 02-01.

## Iteration 3 review — real-product regression remediation

### Reachability and necessity

The later native-metal E08 execution proves the original failure is reachable
on the real owner path, not a simulated or test-only state. `run_server`
constructs the production `HyperHttpProber` at
`crates/overdrive-control-plane/src/lib.rs:1697-1701`; a VM driver receives
the existing shared runner and calls `ProbeRunner::start_alloc` at
`crates/overdrive-worker/src/vm_driver.rs:1874-1876`. The task-local VM HTTP
target is then driven through `probe_tick` to that adapter. For the same
guest address and declared listener port, the real mTLS worker installs the
OUTPUT divert with the existing `MTLS_LEG_S_DIAL_MARK` exemption at
`crates/overdrive-worker/src/mtls_intercept.rs:374-392`.

Before this remediation the unmarked Hyper connection therefore matched the
OUTPUT divert and reached the listener that expects the mTLS leg; the captured
E08 output records the resulting `client error (SendRequest)`. The later
captured product execution records the same built-product Service as
`Running`, with the TCP startup probe and HTTP readiness probe both
`last=pass`. It subsequently fails only while waiting for the later
Stable-lifecycle work; that state is outside step 02-01 and is not treated as
an HTTP remediation failure here.

Commit `418642e1` changes the real connection path: its private connector
creates a `TcpSocket`, applies the pre-existing `MTLS_LEG_S_DIAL_MARK` before
`connect(2)` for non-loopback addresses
(`crates/overdrive-worker/src/probe_runner/http_prober.rs:48-131`), and
`HyperHttpProber::probe` supplies that connector to the existing client at
lines 201-206. This is technically necessary for the proven OUTPUT-rule
interaction, preserves the existing status classification and response-body
behavior, and introduces no public method, type, protocol, persistence, or
lifecycle policy.

### Finding

#### R02-01-2 — issue (blocking DESIGN gap): the proven fix changes an adapter the accepted step explicitly keeps unchanged

The remediation is not admissible under the accepted step-02-01 contract. The
roadmap's `implementation_notes` state that the “HTTP adapter stays
single-example and unchanged”; its production scope names only
`probe_runner/mod.rs`. ADR-0090 also promises “no new storage, adapter,
protocol, lifecycle state, or dependency”
(`docs/product/architecture/adr-0090-vm-service-network-probe-target-projection.md:120-129`),
and the DESIGN technology stack likewise says “no new dependency, daemon,
transport, or protocol”
(`docs/feature/service-kind-vm-workloads/design/wave-decisions.md:306-311`).

`418642e1` instead adds a direct `tower-service = "0.3.3"` dependency at
`crates/overdrive-worker/Cargo.toml:84-87` and replaces the established
Hyper client connector in `http_prober.rs`. The added package was already
present transitively in `Cargo.lock`, but a new direct manifest dependency is
still a dependency change; moreover its hard-coded leaf version violates the
repository's workspace-dependency rule. This is neither compiler-required
fallout nor a test-only correction.

The E08 reproduction establishes that the pre-remediation behavior is a real
production defect. It does not authorize a DELIVER reviewer to amend the
accepted “adapter unchanged / no dependency” boundary. The current design has
an unblocked mechanism gap: it needs an explicit DESIGN decision on whether
and how this existing mark exemption may be consumed by HTTP probes, including
the permitted dependency/manifest boundary. Do not ask the crafter to iterate
on the connector inside this DELIVER step until that design decision is
accepted and independently reviewed.

### Test integrity and verification

- No existing test assertion was weakened or removed by `418642e1`; its
  committed diff changes only the HTTP adapter and manifest/lockfile.
- The three step-owned acceptance tests remain valid for S-SVM-10, S-SVM-11,
  and S-SVM-14, but they use the simulated adapter and cannot prove the
  real-kernel `SO_MARK` / OUTPUT-TPROXY interaction. The E08 product trace is
  the relevant real-product evidence for the newly discovered condition.
- The remediation adds no test-only production wiring and does not alter
  `Running`, `Stable`, readiness, liveness, or restart ownership.

| Command or evidence | Result |
|---|---|
| Focused step-02-01 acceptance command | PASS — 3/3 |
| `cargo xtask lima run -- cargo clippy -p overdrive-worker --all-targets -- -D warnings` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check 10ae94f6 418642e1` | PASS |
| Native-metal E08 captured product run | HTTP readiness probe is `last=pass`; later Stable wait remains nonzero and belongs to step 02-02, not this finding. |

### Iteration 3 remediation disposition

| Finding | Disposition |
|---|---|
| R02-01-1 | Resolved in iteration 2. |
| R02-01-2 | **Open — return to DESIGN.** A design amendment and independent review must authorize the HTTP-adapter/connector and dependency boundary before a DELIVER implementation can be accepted. |

## Iteration 3 final verdict

**REJECTED — DESIGN gap.** The real-product failure and the marked-socket
mechanism are convincingly reachable, narrowly behavioral, and do not create
public API or lifecycle-policy divergence. They nevertheless contradict the
accepted step's explicit unchanged-adapter/no-dependency boundary. A green
focused suite and the partial E08 success cannot approve that unaccepted
architecture change.

## Iteration 4 review — ADR-0092 implementation follow-up

### Design-amendment disposition and contract conformance

Independent DESIGN review `review-adr-0092.md` approves the user-authorized
ADR-0092 amendment and explicitly resolves R02-01-2 as a design matter. The
amendment authorizes exactly the private, cloneable
`tower_service::Service<hyper::Uri>` connector below the unchanged
`HttpProber` port, with the existing `MTLS_LEG_S_DIAL_MARK` applied before
each non-loopback `connect(2)` and no mark for loopback. It also pins the sole
manifest change: root `tower-service = "0.3.3"` plus
`overdrive-worker`'s `tower-service.workspace = true`.

Commit `894c13f79ddc6782e97fcd488c1da83969953bff` conforms to that exact
shape. `MarkedHttpConnector` remains private and `HyperHttpProber::new` and
the `HttpProber::probe(url, timeout)` port are unchanged. Its connector keeps
the URI host and port (defaulting only an omitted port to 80), resolves using
`tokio::net::lookup_host`, creates the matching IPv4/IPv6 `TcpSocket`, and
returns the connected stream to the existing Hyper client. It introduces no
target-origin metadata, port method, descriptor mutation, route, rule,
protocol, storage, lifecycle state, or TCP/Exec behavior.

The follow-up moves the direct connector declaration from the worker leaf to
the root workspace and changes the worker to workspace consumption. The
`Cargo.lock` delta from `10ae94f6` to `894c13f7` is only the required
`overdrive-worker` package dependency edge to the already-resolved
`tower-service 0.3.3`; it adds no package, feature, version, or lockfile
resolution. This is the exact manifest boundary ADR-0092 permits.

### Production reachability and effect ordering

The real owner path remains concrete and unchanged: `run_server` constructs
the production `HyperHttpProber`
(`crates/overdrive-control-plane/src/lib.rs:1697-1705`),
`VmDriver::on_alloc_running` starts the shared runner
(`crates/overdrive-worker/src/vm_driver.rs:1874-1876`), and
`ProbeRunner::start_alloc` projects the task-local descriptor before spawning
the loop (`crates/overdrive-worker/src/probe_runner/mod.rs:298-350`).
`probe_tick` sends its HTTP URL through the existing `HttpProber` port and
writes the existing result row (`mod.rs:521-615`).

For the guest address on this path, the worker's installed OUTPUT divert
matches only an unmarked connection; the established
`MTLS_LEG_S_DIAL_MARK` exemption is the first branch that avoids it
(`crates/overdrive-worker/src/mtls_intercept.rs:374-392`). The private helper
creates and marks the socket at `http_prober.rs:101-137`, while
`connect_marked` invokes `connect` only after that helper returns
(`http_prober.rs:94-99`). The SYN therefore observes the existing exemption,
not the TPROXY divert. Socket creation, mark installation, resolution, and
connect failures still return through Hyper's existing transport-error path;
the runner retains its established cancellation, retry, and lifecycle owners.

The native-metal built-product capture directly corroborates the result:
`verification/expectations/E08-vm-service-guest-health/evidence/product-run.out:44-45`
records both startup and projected HTTP readiness as `last=pass`. Its exit is
nonzero solely because the checked-in example then waits for `Stable`
(`product-run.out:46`). This review treats neither that later lifecycle state
nor its uncommitted step-02-02 work as evidence against the completed adapter
effect.

### Test integrity and verification

The two new module tests call the private production socket constructor before
any connection and read the live fd's `SO_MARK`: the non-loopback candidate
equals the existing mark and the loopback candidate remains zero. They do not
add a public or test-only production seam. The existing three step-owned
acceptance tests retain their exact Contract Shape and Outcome-anchor
declarations; no assertion was weakened or removed. The existing real HTTP
integration tests continue to cover the unchanged status, redirect, and
transport contracts through a loopback server.

| Command or evidence | Result |
| --- | --- |
| `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(non_loopback_probe_socket_has_the_agent_mark_before_connect) or test(loopback_probe_socket_remains_unmarked_before_connect)'` | PASS — 2/2. |
| Focused step-02-01 acceptance command | PASS — 3/3. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-worker --features integration-tests -E 'test(given_real_http_server_200_when_hyper_http_prober_probes_then_returns_pass) or test(given_real_http_server_503_when_hyper_http_prober_probes_then_returns_fail_named) or test(given_real_http_server_302_when_hyper_http_prober_probes_then_returns_fail_no_follow) or test(given_unbound_port_when_hyper_http_prober_probes_then_returns_fail_connection_refused)'` | PASS — 4/4. |
| `cargo xtask lima run -- cargo clippy -p overdrive-worker --all-targets -- -D warnings` | PASS. |
| `cargo fmt --check` and `git diff --check 418642e1 894c13f7` | PASS. |
| Native-metal E08 built-product capture | Projected readiness `http GET .../ready` renders `last=pass`; later Stable wait remains separately scoped. |

No mutation testing was run; it is a final-wave gate, not a per-step gate.

### Iteration 4 remediation disposition

| Finding | Disposition |
| --- | --- |
| R02-01-1 | Resolved in iteration 2. |
| R02-01-2 | **Resolved by approved ADR-0092 and this conforming implementation.** The former leaf pin is removed; the private connector, manifest boundary, and required evidence match the independently reviewed amendment. |

## Final verdict

**APPROVED.** The follow-up implements the approved, bounded ADR-0092
amendment exactly: a private pre-connect mark on non-loopback HTTP probe
sockets, an unmarked loopback path, and the specified workspace dependency
boundary. It preserves the public port, target projection, HTTP policy,
dataplane ownership, and lifecycle ownership. The later `Stable` failure is
not a step-02-01 defect and is not carried into this verdict.
