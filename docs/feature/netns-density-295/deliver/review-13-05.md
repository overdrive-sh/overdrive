# DELIVER review — STEP 13-05 guest-prefix boot checks, gateway route and leg-B marking

## Review metadata

| Field | Value |
|---|---|
| Feature / step | netns-density-295 / 13-05 |
| Review ID | STEP13-05-DELIVER-POSTCOMMIT-01 |
| Review date | 2026-10-29 |
| Reviewer | Fresh isolated post-commit DELIVER implementation reviewer |
| Iteration | 1 |
| Implementation range | 71eb8a63e..62fb2d99c3795eaf0d968dedad078f038dafeff7 |
| Implementation commit | a4c07cdf047c9541299c81241fcc55f2d79859a2 |
| DES persistence commit | 62fb2d99c3795eaf0d968dedad078f038dafeff7 |
| Final verdict | NEEDS_REVISION |
| Open findings | 1 blocking typed-error contract finding |

This review applies the repository AGENTS.md, the applicable Rust, testing,
verification, development, workflow, and DELIVER rules, and the installed
software-crafter review dimensions. The repository's native Markdown review
artifact convention takes precedence over the review skill's legacy YAML
output and two-iteration cap.

Only this review artifact is written. No production source, executable test
body, DES event, roadmap, design artifact, spike source, host state, or commit
was changed by the reviewer. The pre-existing modified AGENTS.md remains
untouched.

## Authority and review boundary

The authoritative contract is STEP 13-05 in
docs/feature/netns-density-295/deliver/roadmap.json:1403-1442, the feature
delta's D8-PREFIX-CFG, D8-PREFIX-VIP, GW-LOCAL and LEGB-DIAL sections, and
ADR-0185, ADR-0190 and ADR-0172. ADR-0152 supplies the gateway-local route
ownership. The accepted native-support handoff and its independent DISTILL
review are also in scope:

- docs/feature/netns-density-295/deliver/distill-13-05-native-support.md
- docs/feature/netns-density-295/deliver/review-distill-13-05-native-support.md

The handoff's 13 approved iterations establish the bounded existing GuestKit
support, including the 98-body cgroup-default group, 46-body mtls-relay
group, complete 122-identity native exclusion ledger, private filesystem and
confinement boundary, exact route read-back, and ADR-0172 four-rule identity.
Those approvals were checked against the current source and receipts; they are
not treated as a substitute for this implementation review.

The accepted implementation boundary requires:

1. typed startup refusals for a node interface address or configured VIP range
   intersecting GUEST_NODE_PREFIX, before any boot effect, with the exact
   payloads and health.startup.refused, reason guest_network.probe;
2. one boot-only, never-removed local <gateway>/32 dev lo route in the local
   table, type Local, scope Host, protocol 247, with atomic keep/replace/add
   semantics and no route covering the rest of the prefix;
3. HostMtlsEnforcement::new(identity, limits, guest_prefix, guest_dial) with
   guest_prefix required and every caller passing it explicitly;
4. conditional leg-B options before connect() for a peer inside the prefix:
   SO_MARK = 1, IP_BIND_ADDRESS_NO_PORT, and bind(127.0.0.1:0); peers
   outside the prefix retain the existing unmarked, unbound path;
5. the exact ADR-0172 output projection: TCP reset first, plain
   ICMP-port-unreachable fallback for non-TCP, unchanged registered-destination
   accept, gateway exclusion, prerouting drop, and no new public rule/API;
6. no prefix-wide route, runtime gateway-route writer, STEP 13-06 host-internal
   deny logic, or V-LEGB-SERVE resolver/workload-address mechanism; and
7. existing scenario, lane, executor, model, and public API boundaries with
   the native and normal gates independently evidenced.

## Commit and DES integrity

Both commits retain the Marcus author and contain exactly one
Co-Authored-By: Codex <codex@openai.com> trailer plus Step-Id: 13-05.
No Claude, Anthropic, or generated-by attribution appears. The implementation
commit is the 60-file, +12097/-103 change from the approved STEP 13-04
review commit; the DES commit only persists the commit phase.

The persisted execution log is valid JSON and records the actual ordered
history:

| Phase | Result | UTC timestamp |
|---|---|---|
| RED | PASS | 2026-10-28 14:49:16 |
| GREEN | FAIL | 2026-10-28 16:06:46 |
| GREEN | FAIL | 2026-10-28 18:14:21 |
| GREEN | PASS | 2026-10-28 23:24:20 |
| COMMIT | PASS | 2026-10-28 23:25:28 |

The failed GREEN attempts remain in their original receipts and are classified
as fixture/tooling corrections by the approved DISTILL record. They were not
relabelled as passes, erased, or converted into exclusions. The current
worktree has only the user-owned AGENTS.md change outside the reviewed
commits.

## Contract and implementation assessment

### Startup overlap refusals and caller projection

probe_guest_flow_startup reads one host-facts snapshot and checks the first
operator-supplied VIP range before interface addresses at
crates/overdrive-control-plane/src/guest_network.rs:813-865. It returns
GuestPrefixOverlapsVipRange { prefix, vip_range } or
GuestPrefixOverlapsNodeAddress { prefix, interface, address } directly. The
composition-root path at lib.rs:3162-3185 and the supplied-owner path at
lib.rs:3188-3205 preserve those typed causes in
ControlPlaneError::GuestNetworkBoot and emit health.startup.refused, reason
guest_network.probe.

The activated source-local and production-composition cells assert both exact
payloads, the VIP-first precedence, and zero effects before refusal. The
current source contains no generic-string replacement and no new error
variant.

### Gateway route ownership and shape

The boot owner calls converge_gateway_route only after the forwarder and both
listeners have completed at guest_network.rs:1216-1248. Runtime component
repair branches at :1163-1209 do not call the route writer. The host seam
passes only GUEST_PREFIX_ROUTE_PROTOCOL to the fixed helper at
flow_io/host.rs:438-452. The helper constructs a /32 local route on lo,
table RT_TABLE_LOCAL, type Local, scope Host, and protocol 247 at
overdrive-netlink/src/client.rs:451-471; native read-back observes the exact
local 100.95.0.1 dev lo proto 247 scope host route and no main/table-100
route. No prefix-wide route or route deletion is present in the reviewed
production path.

The accepted route ownership support distinguishes created and borrowed route
states and removes only a fixture-owned route after exact validation. It does
not authorize the product owner to remove the boot-sticky route.

### Conditional leg-B dial and constructor shape

HostMtlsEnforcement::new has the exact four-input public shape at
crates/overdrive-dataplane/src/mtls/mod.rs:168-193, and the production factory
passes both deps.guest_prefix and deps.guest_dial at :152-165. The outbound
path carries the prefix through its private establish helper and dial_leg.

At mtls/mod.rs:724-810, a peer outside the prefix takes the unchanged
TcpStream::connect_timeout path. On Linux, a peer inside the prefix creates
one socket, sets SO_MARK to MTLS_TPROXY_MARK (0x1), sets
IP_BIND_ADDRESS_NO_PORT, binds 127.0.0.1:0, then performs the marked
connect. All socket-option errors are typed MtlsEnforcementError::Io; the
socket is owned by TcpStream before the connect helper can return. The three
existing LEGB cells observe loopback source plus mark 1 for the registered
tuple, the outside-prefix control's non-loopback/non-marked behavior, and
ECONNREFUSED with no wildcard-listener accept for an unregistered tuple.

The current production path has no resolver, workload-address manufacture,
new port, public dial method, or STEP 13-06 deny mechanism. The V-LEGB-SERVE
validation scope remains a later end-to-end item.

### ADR-0172 output rule projection

overdrive-netlink/src/nft.rs:734-794 now builds four output rules. The
registered destination rule remains before the refusal rules. The new private
TCP rule matches the guest prefix, excludes the gateway, matches L4 TCP, and
encodes nft reject with TCP reset. The following private rule is the plain
ICMP-port-unreachable reject used by non-TCP traffic. The prerouting drop,
gateway exclusion, leg-S exemption, destination set, and rule order remain
unchanged. OUTPUT_RULE_COUNT = 4 and the worker postcondition identity at
mtls_intercept_port.rs:1246-1251 agree with the encoder. Native TCP refusal,
UDP EPERM, remote drop, and registered leg-C receipts pass the exact
stakeholder-visible outcomes.

### Typed semantic mapping

| Designed failure | Current implementation / caller path | Assessment |
|---|---|---|
| VIP range intersects prefix | GuestNetworkError::GuestPrefixOverlapsVipRange { prefix, vip_range } from probe_guest_flow_startup; ControlPlaneError::GuestNetworkBoot; health.startup.refused / guest_network.probe | PASS |
| Node address intersects prefix | GuestNetworkError::GuestPrefixOverlapsNodeAddress { prefix, interface, address }; same caller projection | PASS |
| lo cannot be resolved | HostGuestFlowIo::converge_gateway_route maps NetlinkError::LinkAbsent { iface: "lo" } to GuestNetworkError::Netlink { operation: LocalRouteConverge, source } | PASS |
| Atomic gateway route request fails | The same owner path preserves GuestNetworkError::Netlink { operation: LocalRouteConverge, source }, but the low-level route payload uses the wrong operation label, documented below | BLOCKING |
| Leg-B socket setup/connect fails | Private setup maps the kernel error to approved MtlsEnforcementError::Io; no new public error taxonomy is introduced | PASS |

The overlap variants and outer GuestNetworkBoot envelope preserve semantic
payloads. The only mismatch is the approved low-level route operation label;
it is not replaced by a diagnostic string and does not justify inventing a
new error variant.

## Native and execution evidence

The current durable receipts provide the required independent boundaries:

| Evidence | Result |
|---|---|
| mtls-relay native gate | Metal run 8066a14e0a5afc0f0c4919fd, raw SHA abd817092d3554facc960601d8fcefb4a052bd49eaf03f1a77204108220bd494; exact 46 bodies, zero failed/ignored/runtime skips, exit 0 |
| cgroup-default native gate | Metal run beb529fdc4eee0728470cc16, raw SHA 9d49b618face96566cac28f4ae8f238cd7f8174bf360de32117e23dd2fdaac14; retained 55 + STEP 13-04 18 + STEP 13-05 5 plus the native control-plane suffix, exact 98-body accounting, zero failed/ignored/runtime skips, exit 0 |
| Full normal package gate | Metal run e541a56569eeecc22907cdf1, raw SHA 4799e4a679d9ab42c44953974709215d8307b4d07bef137927f383d57cab1bcb; 1,224 passed, 131 accounted skips, zero failures, exit 0 |
| Source/runner package | Metal source package run 8a6a73dc0b9bdc2ff03eea1c, raw SHA 594319921ab964e170384c145065b66edbdcb0ec9772a3ebc15ef3ae2d0d9c41; formatting and Clippy pass |
| Scoped diff check | Metal run 4598d9bc8b5f6833e8735077, exit 0 |
| JSON and local formatting checks | Roadmap/execution log parse pass; cargo fmt --all -- --check pass |

The native receipts preserve failed-attempt evidence for missing guest tools,
DNS holder/route fixtures, runtime skip detection, route rendering, and the
automatic dummy device. The approved DISTILL review documents each causal
failure and bounded correction. The known quiescence baseline seeds remain
outside this step's claim. Mutation testing was not run, consistent with the
repository rule and the step handoff.

The ordinary package receipt is an independent Rust test layer and the native
receipts are the disposable-Cloud-Hypervisor black-box boundary. No production
binary is spawned by Rust tests, and the native runner does not invoke
cargo test, nextest, or an overdrive-* test binary as its system under
test. Physical host management namespaces, routes, nftables, cgroups, and
modules were not used as the test substrate.

## Review dimensions

| Dimension | Result | Evidence |
|---|---|---|
| Exact API shape | PASS | The four-input HostMtlsEnforcement::new shape and bounded caller fallout match the accepted design; no public API was invented. |
| Typed semantic errors | BLOCKED | Gateway route failures retain the approved outer variant but expose host-local-replace where the approved representation is gateway-local-replace. |
| Startup ordering | PASS | Overlap checks happen before forwarder probe/listeners/routes and emit the exact startup reason. |
| Gateway route ownership | PASS apart from the operation label | Fixed local /32 shape, protocol 247, boot-only writer, no runtime writer or route removal, and native read-back all pass. |
| LEGB-DIAL | PASS | Conditional mark, no-port option, loopback bind, outside-prefix preservation, fail-closed unregistered tuple, and no-uplink evidence pass. |
| ADR-0172 rule semantics | PASS | Four-rule private identity and native TCP reset/UDP EPERM/remote drop/leg-C outcomes pass. |
| Test integrity | PASS | Existing bodies were activated; support corrections remain private, bounded, and independently DISTILL-reviewed. |
| Native/Tier-3 boundary | PASS | 46, 98, and normal 1,224-body receipts are independently retained with exact accounting. |
| Verification completeness | PASS | Build/check/Clippy/fmt, native gates, normal gate, JSON, and scoped diff receipts are present; mutation is correctly deferred. |
| Scope | PASS | No prefix-wide route, runtime route repair, resolver invention, STEP 13-06 deny logic, new lane, scenario, model, or public mechanism was pulled forward. |
| RPP / readability | PASS for the bounded implementation | No new L1/L2 smell affecting the accepted outcome was found. |

## Findings

### issue (blocking): The approved GW-LOCAL typed route operation label is not implemented

The accepted typed representation is explicit in
docs/feature/netns-density-295/feature-delta.md:11402-11419 and the approved
error-variants review at
docs/feature/netns-density-295/deliver/review-design-12-01-error-variants.md:784-796:

    NetlinkError::Route { op: "gateway-local-replace", source }

The current production implementation instead constructs
NetlinkError::Route { op: "host-local-replace", source } at
crates/overdrive-netlink/src/client.rs:451-471. This is an exact typed
payload mismatch, not a wording preference. op is a public payload of the
approved NetlinkError::Route representation, and the design pins the label
callers and diagnostics receive.

The mismatch is reachable through the real production owner path. Boot calls
HostSharedGuestNetworkOwner::converge_shared at
crates/overdrive-control-plane/src/guest_network.rs:1216-1248; after the
forwarder and listeners, it calls HostGuestFlowIo::converge_gateway_route at
crates/overdrive-control-plane/src/guest_network/flow_io/host.rs:438-452;
that calls Client::converge_ipv4_host_local_route, whose one RTM_NEWROUTE
request returns the mismatched Route payload on a real route NACK. The owner
preserves the outer GuestNetworkError::Netlink { operation:
LocalRouteConverge, source } and the composition root projects the refusal
through the approved guest_network.converge startup path. A missing lo
separately reaches the approved LinkAbsent branch, so this finding concerns
the route-request error branch only.

The complete native and normal suites do not close this finding: they prove
the successful route shape and caller outcomes, while no current assertion
checks the low-level op payload. The source-level reproduction is direct and
bounded: the reachable map_err expression emits the unapproved literal under
the exact route failure branch; no timing, concurrency, cancellation, or
test-only state is involved.

Required disposition: the original STEP 13-05 crafter must preserve the
existing public API, outer LocalRouteConverge mapping, route ownership, and
atomic mechanism while changing the low-level operation label to the approved
"gateway-local-replace". A bounded existing seam assertion may be added only
if needed to demonstrate the approved payload; no new public API, route
mechanism, scenario, lane, or architecture is authorized. Re-run the relevant
typed route failure evidence and return this same step to this reviewer.
No later step may start.

Disposition: OPEN — blocks approval.

### praise: The LEGB-DIAL implementation preserves both halves of the security boundary

The production socket path sets the mark, IP_BIND_ADDRESS_NO_PORT, and
loopback bind before connect only for guest-prefix peers, while the existing
non-prefix path remains unchanged. The current disposable guest receipt shows
the positive leg-B source as 127.0.0.1, mark 1, the exact guest-prefix peer,
and no uplink/default route; the outside-prefix and unregistered controls
remain active. This is the accepted ADR-0190 mechanism and does not introduce
the discarded prefix-wide route.

### praise: The native-support remediation is evidence-preserving and fail-closed

The 13 DISTILL iterations retain failed attempts, classify actual fixture
preconditions, and correct only existing GuestKit staging, route ownership
bookkeeping, parser normalization, skip detection, and automatic dummy-device
cleanup. The 46/98 native boundaries and full normal 1,224-body receipt are
separate evidence layers. The runner never turns a runtime infrastructure skip
into a pass and keeps the physical host outside the mutation boundary.

## Iteration history and verdict

### Iteration 1 — 2026-10-29

This fresh post-commit review covered the complete implementation diff, the
accepted feature delta and ADR contracts, the full production caller paths,
typed error mappings, changed tests and support, the 13-iteration DISTILL
handoff/review, DES history, commit attribution, and all retained native and
normal receipts. Startup overlap refusal, gateway route shape/ownership,
conditional leg-B marking, ADR-0172 output projection, test integrity, scope,
native confinement, and verification evidence pass. The exact low-level route
operation label is wrong on the reachable typed route-error branch, so the
typed contract gate remains open.

Verdict: NEEDS_REVISION. Remediate the one-line typed operation mismatch
through the original STEP 13-05 crafter and return this same step for
re-review. No later roadmap step may start. No production or test change was
made by this reviewer.

# NEEDS_REVISION

## Iteration 2 — typed route-operation remediation re-review

### Re-review metadata

| Field | Value |
|---|---|
| Review iteration | 2, same STEP 13-05 reviewer |
| Re-review date | 2026-10-29 |
| Remediation commit | `286600bff3dc723e10ca6efce1e2d1f5116f809d` |
| Remediation scope | One production string literal in `overdrive-netlink/src/client.rs:471` |
| Remediation receipt | Metal run `22f2e4688c2dc31194fb4658`, raw SHA `282c08620f1af871d9c4193e4c7b92030812b770d8a3924c8e07d7f726747179` |
| Final verdict | **APPROVED** |

The original crafter changed only the low-level route error operation label
from `host-local-replace` to the exact approved `gateway-local-replace`.
The commit retains the original author, exactly one Codex co-author trailer,
and `Step-Id: 13-05`. No test body, native runner, DES event, public API,
route mechanism, caller mapping, or design artifact changed.

### Disposition of the blocking finding

**F-01 — CLOSED.** `Client::converge_ipv4_host_local_route` now emits
`NetlinkError::Route { op: "gateway-local-replace", source }` at
`crates/overdrive-netlink/src/client.rs:451-471`, exactly matching the
feature delta at line 11419 and the approved error-variants review at
lines 784-796. The helper still constructs the same fixed local `/32` route
with table local, `lo`, type Local, scope Host, caller-supplied protocol, and
one atomic add/replace request.

The caller boundary is unchanged: `HostGuestFlowIo::converge_gateway_route`
continues to pass protocol 247 and map all failures to
`GuestNetworkError::Netlink { operation: LocalRouteConverge, source }` at
`flow_io/host.rs:438-452`. Boot still invokes the route writer only after the
forwarder and listeners at `guest_network.rs:1216-1248`; runtime repair still
does not write the gateway route. No other route operation label was changed:
the FIB lookup remains `fib-lookup`, and unrelated route helpers retain their
existing labels.

The remediation is therefore a necessary exact-contract correction, with no
new semantic variant, generic string carrier, public surface, architecture,
or behavior change. The previous source-level reproduction is closed by the
changed expression, and the durable Metal remediation check confirms the
affected crate and callers still compile and test cleanly.

### Remediation verification

| Check | Result | Evidence |
|---|---|---|
| Focused overdrive-netlink library tests | **PASS** | 50 tests passed, 0 skipped in Nextest run `a6b5cb6f-82c7-464b-bb16-0c8c855a17ac` |
| Package compilation and Clippy with warnings denied | **PASS** | Same durable Metal run `22f2e4688c2dc31194fb4658`, exit 0 |
| Formatting and scoped diff check | **PASS** | Same remediation receipt; no unrelated diff |
| Exact operation-label search | **PASS** | Production route writer and approved design both contain `gateway-local-replace`; no `host-local-replace` remains in the reviewed route path |
| Retained native gates | **PASS, unchanged** | Previously reviewed 46-body and 98-body receipts remain valid; remediation cannot affect their native behavior |
| Retained normal package gate | **PASS, unchanged** | Previously reviewed 1,224 passed / 131 accounted skips receipt remains valid |
| Mutation testing | Correctly deferred | No per-step mutation run, per repository rule |

The reviewer did not rerun unchanged native or normal gates. The one-line
remediation is outside their behavior and does not alter any native body,
selector, fixture, model, or accounting boundary. The user-owned dirty
`AGENTS.md` remains preserved; the review artifact is the only reviewer-owned
worktree file.

### Iteration 2 verdict

The sole blocking typed-error finding is closed. The exact approved route
operation label now flows through the existing low-level error carrier and
unchanged `LocalRouteConverge` caller mapping. No unresolved contract,
security, lifecycle, native-boundary, test-integrity, scope, or verification
finding remains.

**Verdict: APPROVED.** STEP 13-05 may advance under the strict DELIVER
sequence. No STEP 13-06 work was started by this reviewer.

# APPROVED
