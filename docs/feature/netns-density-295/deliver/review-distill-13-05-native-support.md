# DISTILL review — STEP 13-05 existing GuestKit native support

## Review metadata

| Field | Value |
| --- | --- |
| Review ID | `STEP13-05-DISTILL-NATIVE-SUPPORT-REVIEW-01` |
| Feature / roadmap step | `netns-density-295` / `13-05` |
| Review date | 2026-10-28 |
| Reviewer | Fresh independent DISTILL source reviewer |
| Iteration | 1 |
| Scope | Existing `cgroup-default` support for the five already-authored STEP 13-05 native cells, retained `mtls-relay` coverage, native-boundary accounting, and the source handoff's required verification plan |
| Design authority | Feature-delta D8-PREFIX-CFG, D8-PREFIX-VIP, GW-LOCAL and LEGB-DIAL; ADR-0152, ADR-0172, ADR-0185 and ADR-0190; roadmap STEP 13-05 |
| Verdict | **CHANGES_REQUIRED** |
| Open findings | 1 blocker |

This review applied the repository's current `AGENTS.md`, development,
testing, verification and workflow rules, plus the nWave DISTILL,
acceptance-review, completeness, test-design and BDD criteria. The
repository's direct Rust scenario and native-Markdown conventions take
precedence over the installed review definition's generic `.feature` and YAML
examples.

## Review boundary

The reviewed DISTILL-owned source delta is limited to:

- extending the existing private `cgroup-default` manifests with the two
  already-authored control-plane startup cells and three already-authored
  worker leg-B cells;
- sequencing the three leg-B cells after STEP 13-04's temporary dummy/default
  route has been removed;
- establishing and reading back the accepted qualified-node fixture state in
  the disposable worker guest;
- adding only `sysctl` to that existing worker guest's tool payload; and
- updating the existing staging declaration's inventory comment.

The original STEP 13-05 crafter's production changes, marker removals, DES
events and execution log are concurrent work and remain separately
attributed. This review inspects those changed production and test files only
where needed to prove that the native support runs the accepted existing
bodies honestly. It does not claim or modify them.

Only this review artifact was added. No production source, test source,
runner source, staging source, acceptance body, assertion, Contract Shape
declaration, pending marker, roadmap, DES event, execution log, model, lane or
commit was changed by the reviewer.

## Reviewed artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| `deliver/distill-13-05-native-support.md` | `550da090d9d18330c0f9b0abda03418874c6890eed7cb4490278402b29480a11` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `290d25e1734f890190f9be592f3c48cf88cf865adccd89501f2b6cd7b1d42d05` |
| `xtask/src/guest_kit_prepare.rs` | `9ff73d8fb6c506995e7e4936b503ba9807ddd8fca428484d37988dc7545d8eb7` |
| `shared_guest_network_startup.rs` | `ed35d0358110dfa990cfdd295dd58e73faa6e1966a04df7615a0036bf9a5fd91` |
| `vsock_legb_dial.rs` | `b5294cd324f3658d88a46641ebc0c352159d0433ab202a30233d7d5ebaded1c5` |
| `overdrive-netlink/src/nft.rs` | `01f4b9b38faa59594c537953d9126e54a7c7adaa366c5ec09eaf28cf12deb4b6` |
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| `deliver/roadmap.json` | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| ADR-0152 | `9fc448953f40950888c59e36f0a49aa53d15ab138caea96737ea7de0b6a40c90` |
| ADR-0172 | `bee320c2650ae975442ec3d763dad68e391bc91bf4dadda61c0227e0cc60f48f` |
| ADR-0185 | `633e8f678897cd357f529a0f56e2d240afbe5ae0881e3b57487963668ad6486b` |
| ADR-0190 | `bc5c323f028c39380fa7419e54aca3908877554359edd74e7959b593020f618a` |
| V-LEGB-MARK findings | `692829d374d0c37d962b98e7e85f7f22bb04b664af141541226231379a0986d4` |

## Strengths

- The source reuses the existing `cgroup-default` selectable group, its three
  existing disposable Cloud Hypervisor guests, the same CID slots and the
  same staged integration binaries. It adds no group, lane, guest executor,
  body, scenario, public API or model.
- Every STEP 13-05 row uses one exact libtest identity with expected count one.
  `GuestRun::exec_cgroup` requires exit zero and the exact
  `1 passed; 0 failed` summary, so a missing, ignored, renamed, broadened or
  zero-selected body cannot count as a pass.
- Runtime accounting fails closed at four independently meaningful values:
  the 55-body baseline, the 18-body STEP 13-04 extension, the five-body STEP
  13-05 extension and the 78-body combined total.
- The leg-B fixture pins and reads back `route_localnet=0`, installs only the
  accepted gateway-local `/32`, proves the main table is empty, proves a
  non-gateway guest-prefix address is unroutable, proves the root namespace
  has only loopback, and proves mark-1/table-100 production state is absent
  before and after every selected leg-B body.
- STEP 13-04's disposable dummy/default route remains scoped to its two
  host-output cells. Its explicit cleanup and unroutable read-back complete
  before any STEP 13-05 leg-B body starts.
- Only `sysctl` is added to the worker guest. The selected paths already have
  `ip`, `nft`, `ss`, `uname` and the `/var/run -> /run` compatibility alias;
  source inspection finds no selected Python invocation.
- The production-output fixture correction follows ADR-0172 exactly: the
  registered-destination rule still precedes rejection, TCP guest-prefix
  fallback uses reset, the following plain reject remains the non-TCP
  ICMP-port-unreachable fallback, and the gateway exclusion and prerouting
  drop remain unchanged.
- The source handoff correctly refuses to claim the unrun native results. The
  78-body `cgroup-default` and seven-body `mtls-relay` replays remain future
  native evidence after source review.

## Exact existing scenario selection

The five runner entries resolve to the five existing active bodies exactly
once:

| Existing staged binary | Exact body | Contract Shape | Role |
| --- | --- | --- | --- |
| `overdrive-control-plane--integration` | `shared_guest_network_startup::a_vip_range_overlapping_the_guest_prefix_refuses_boot_before_any_effect` | `unbounded-preservation` | D8-PREFIX-VIP typed refusal before effect |
| `overdrive-control-plane--integration` | `shared_guest_network_startup::a_node_address_inside_the_guest_prefix_refuses_boot_naming_the_interface` | `unbounded-preservation` | D8-PREFIX-CFG typed refusal before effect |
| `overdrive-worker--integration` | `vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c` | `bounded-change` | exact marked and loopback-bound positive path to registered leg C |
| `overdrive-worker--integration` | `vsock_legb_dial::a_dial_to_a_peer_outside_the_guest_prefix_is_neither_marked_nor_bound` | `bounded-change` | conditionality control for an outside-prefix peer |
| `overdrive-worker--integration` | `vsock_legb_dial::a_marked_dial_to_an_unregistered_tuple_is_refused_and_reaches_no_host_socket` | `bounded-change` | typed TCP refusal and silent host decoy |

The test-source diff is exactly five marker deletions: two in
`shared_guest_network_startup.rs` and three in `vsock_legb_dial.rs`. There is
no authored-body or assertion change. Each selected body retains its existing
Contract Shape declaration, and source search finds no remaining STEP 13-05
pending marker in either selected file.

The two startup cells still enter the production `run_server` composition.
They assert the exact approved typed payloads, the
`guest_network.probe` refusal phase and the absence of later effects. Moving
them into the cgroup-qualified disposable control-plane guest supplies the
real startup precondition that their ordinary Metal process lacks; it does not
replace their production driving boundary.

The three worker cells still construct the real `MtlsInterceptWorker` over
the real `HostMtlsIntercept` and `HostMtlsEnforcement`, publish the real shared
owner, register the declared VM tuple through production `start_alloc`, and
drive leg B through the `MtlsEnforcement::enforce` entry point. The
`SimGuestStreamDial` remains the accepted test-side driven port for leg S and
is scripted only at the exact registered tuple. The real nft program, policy
rule, table-100 route, transparent leg-C socket, mutual-TLS handshake and
kernel socket table remain the observed path.

## Qualified worker-guest state

`STEP13_05_LEGB_SETUP` does exactly two things before the three worker cells:

1. it writes and reads back
   `net.ipv4.conf.{all,default,lo}.route_localnet=0`; and
2. it installs `local 100.95.0.1/32 dev lo proto 247`.

It does not add a main-table default, a guest-prefix `/16`, an uplink, a host
address inside the guest prefix, a mark rule or a table-100 route. This is the
accepted GW-LOCAL fixture precondition, not a substitute for the production
LEGB-DIAL mechanism.

Before and after each selected leg-B body, `STEP13_05_LEGB_AUDIT` requires:

- exact zero read-back for all three `route_localnet` keys;
- the exact tagged gateway route in the local table;
- an empty main table;
- failed lookup of `100.95.255.200`;
- exactly one root-namespace device, `lo`;
- no `fwmark 0x1 lookup 100` rule; and
- an empty table 100.

The real `HostMtlsIntercept::start_shared_owner` must therefore install the
production mark rule and table-100 route within each body, and
`Node::finish` plus existing cleanup must remove them before the post-body
audit. The setup cannot make the positive path pass by a prefix-wide route,
default route or uplink. A missing mark leaves the loopback-bound dial without
the table-100 lookup; a missing loopback bind leaves the marked local lookup
with a non-local source. The positive body's required `fwmark:0x1`, leg-B
loopback source, leg-C loopback peer and exact registered local tuple keep both
halves observable. The outside-prefix veth control remains route-disjoint and
is destroyed before its post-body audit. The unregistered-tuple control still
requires typed `Io(ConnectionRefused)` within the bound and no wildcard-host
accept.

## STEP 13-04 ordering and isolation

The execution order remains:

1. baseline manifests;
2. route-independent STEP 13-04 manifests;
3. STEP 13-04's two host-output cells under the temporary dummy/default route;
4. explicit deletion of that route and dummy link plus unroutable read-back;
5. STEP 13-04's remote-drop and registered-leg-C cells;
6. STEP 13-05 qualified-node setup; and
7. the three STEP 13-05 leg-B cells, each surrounded by the new audit.

This order prevents STEP 13-04's temporary default route from satisfying a
STEP 13-05 positive or changing the expected mark-only/bind-only causal
boundary. The worker guest begins STEP 13-05 with no main route and no
non-loopback interface. The gateway-local `/32` persists only for the
remainder of the disposable guest; no production owner removes it, and guest
destruction is its fixture cleanup.

## Exact manifest and native accounting

Independent source extraction gives:

| Manifest | Harness invocations | Expected bodies |
| --- | ---: | ---: |
| Existing control-plane acceptance | 3 | 3 |
| Existing control-plane integration | 35 | 45 |
| Existing worker integration | 7 | 7 |
| **Existing baseline total** | **45** | **55** |
| STEP 13-04 control-plane integration | 6 | 6 |
| STEP 13-04 worker route-independent | 8 | 8 |
| STEP 13-04 worker temporary-route cells | 2 | 2 |
| STEP 13-04 worker post-route cells | 2 | 2 |
| **STEP 13-04 total** | **18** | **18** |
| STEP 13-05 control-plane integration | 2 | 2 |
| STEP 13-05 worker integration | 3 | 3 |
| **STEP 13-05 total** | **5** | **5** |
| **Combined `cgroup-default` total** | **68** | **78** |

The existing disposable guest identities remain:

| Guest | CID slot | Staged binary | Baseline | STEP 13-04 | STEP 13-05 | Resulting bodies |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| `cgroup-control-plane-acceptance` | 14 | `overdrive-control-plane--acceptance` | 3 | 0 | 0 | 3 |
| `cgroup-control-plane-integration` | 15 | `overdrive-control-plane--integration` | 45 | 6 | 2 | 53 |
| `cgroup-worker-integration` | 6 | `overdrive-worker--integration` | 7 | 12 | 3 | 22 |
| **Total** | — | — | **55** | **18** | **5** | **78** |

The aggregate counters advance only after each row's exit and summary oracle
passes. They then independently require `55`, `18`, `5` and `78`. The existing
`mtls-relay` group remains separate: it stages the existing
`overdrive-dataplane--integration` binary in its own disposable guest and
requires exactly seven `mtls_inbound_guest_relay` passes. Those seven are not
double-counted in the 78-body `cgroup-default` total.

## Tools, modules, runner surface and confinement

The staging declaration keeps the same six entries. Its executable change is
only an inventory comment; no target, package, feature set or lane is added.
The selectable runner registry remains the existing eight names. The private
`exec_step13_05_legb_cells` helper sequences setup/audits around the existing
`GuestRun::exec_cgroup`; it does not introduce another guest protocol or
executor.

The worker tool delta is one `tool("sysctl")` entry. `tool()` copies the
resolved executable to `/usr/bin` in the guest and retains the existing ELF
interpreter/shared-library closure behavior. The selected startup cells use
the existing `ip` staging; the leg-B cells and their shared fixture use the
existing `ip`, `nft`, `ss` and `uname` staging plus the new `sysctl`. No
selected source calls `python3`, so omitting a Python runtime preserves the
minimal payload without changing an oracle.

No module list or load order changes. The existing nested KVM/vhost closure,
`nfnetlink`, `nf_tables`, `veth`, `tls`, STEP 13-04 tproxy/reject closure and
dummy module remain staged on the physical host and loaded only by the peer
inside the disposable guest.

The runner delta does not touch `GuestKit` confinement. Cloud Hypervisor
retains uid/gid 4200, the actual `/dev/kvm` and `/dev/vhost-vsock` owner groups
as supplementary groups, root-owned fixture-parent traversal through gid 4200
mode `0710`, per-allocation ownership, the existing Landlock grant, launch
seccomp filter, `no_new_privs`, device modes, VMM lifetime and RAII cleanup.
The runner issues no physical-host `sysctl`, nft, route, rule, cgroup-mount or
netns command; these operations occur only through the scripted peer inside
the disposable guest.

## ADR-0172 private fixture representation

The current output program has four ordered rules:

1. leg-S mark exemption;
2. exact registered-destination lookup followed by mark/accept;
3. guest-prefix, non-gateway TCP match followed by
   `NFT_REJECT_TCP_RST`; and
4. guest-prefix, non-gateway plain IPv4 reject with ICMP port-unreachable.

The first matching TCP reject terminates evaluation, making the final plain
rule the non-TCP fallback. Both rules reuse the existing constant prefix and
gateway inequality. The prerouting program remains three rules with the same
final drop. `OUTPUT_RULE_COUNT` is four, so expected identity,
normalization, observation and rollback validation all use the same exact
shape. No public type, method, error or configurable rule surface is added.

The existing constant-rule native cells retain TCP `ECONNREFUSED`, UDP
`EPERM`, remote drop and registered leg-C positive outcomes. The new
unregistered-tuple leg-B cell retains TCP `ECONNREFUSED` and the silent decoy
oracle. The correction implements the already-accepted ADR-0172 semantic set;
it does not authorize another rule or a weaker fallback.

## Acceptance-review dimensions and mandates

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The registered same-node positive is paired with outside-prefix conditionality, unregistered-tuple refusal and two typed startup refusals. |
| GWT / one behavior | 9/10 | Each Rust cell retains one concrete precondition, one production trigger and observable outcomes; no body is combined or rewritten by this support delta. |
| Domain language | 10/10 | Exact owner, route, peer, prefix, tuple, mark and refusal vocabulary is preserved throughout the handoff and manifests. |
| Coverage completeness | 9/10 | The five existing native STEP 13-05 bodies are selected exactly once; source-local and ordinary evidence remains in its existing lane. |
| User/operator value | 9/10 | Startup refusal and same-node mesh behavior remain observable through production composition and enforcement boundaries. |
| Priority validation | 10/10 | The support addresses reproduced cgroup-bootstrap ENOENT and the real-kernel leg-B environment, without expanding the scenario set. |
| Observable behavior | 10/10 | Typed refusals, refusal phase, no later effects, exact socket tuples, mark, source, timeout and no-host-accept remain the assertions. |
| Traceability | 10/10 | Exact body names map to D8-PREFIX-CFG, D8-PREFIX-VIP, LEGB-DIAL, V-LEGB-MARK and roadmap STEP 13-05. |
| Walking-skeleton boundary | 10/10 | N/A as a new walking skeleton: the delta selects existing Tier-3 focused cells and preserves their real production adapter boundary. |

| Mandate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | Startup enters production `run_server`; leg B enters `MtlsEnforcement::enforce` over real host adapters and owner composition. |
| CM-B — domain abstraction | PASS | No acceptance body or step vocabulary changes; technical fixture commands stay in the private runner. |
| CM-C — complete focused journey | PASS | Every selected cell retains stimulus through typed or kernel-visible outcome and cleanup. |
| CM-D — fixture isolation | PASS | Support configures only the disposable guest and delegates behavior to production owners. |
| CM-E — universe-bound assertion | PASS / layer N/A | These are layer-4+ integration cells, where traditional observable assertions are permitted. |
| CM-F — layer-dependent PBT | PASS | Real-I/O integration cells remain explicit single examples. |
| CM-G — two-tier rich journey | N/A | No new rich chained journey or Tier-B surface is introduced. |
| CM-H — integration sad paths | PASS | Each real-I/O failure mode remains an explicitly named example with its exact typed or errno outcome. |

The C1-C7 completeness taxonomy is unchanged because this source package adds
no scenario, state, operation, input class, mode flag, error or concurrency
claim. Its delta-completeness question is execution selection: all five
existing STEP 13-05 native cells are present exactly once, while ordinary,
source-local and retained `mtls-relay` evidence remain in their proper
boundaries. Delta completeness remains 15/15 under the checklist's N/A rule.

## Blocking finding

### F-01 — The handoff replaces the required normal package test gate with compilation and native subsets

- **Severity:** Blocker.
- **Kind:** `AT_GAP_IN_DELIVERY_SCOPE` — the roadmap package set and the
  workspace's physical-host safety boundary are already specified; no DESIGN
  choice is missing.
- **Claim:** The source handoff correctly names the required ordinary package
  set, but then declares compilation plus two native subsets to be the safe
  package gate and omits an actual normal package test run from its required
  next evidence. That weakens the accepted verification contract.

#### Exact contradiction

The roadmap requires an actual `cargo nextest run` over exactly
`overdrive-control-plane`, `overdrive-dataplane` and `overdrive-netlink`
(`deliver/roadmap.json:1436-1440`). The binding STEP 13-05 handoff additionally
requires the full package-union gate to genuinely run clean with independent
Cloud-Hypervisor-routed body accounting; focused tests are not a substitute
(`.context/netns-density-295/dispatch-13-05.md:177-188`).

The DISTILL handoff acknowledges the exact package set and says dataplane
cannot be omitted (`distill-13-05-native-support.md:115-120`). It also
correctly records that the retained `fa224a04f75a914094235928` run omitted
dataplane and therefore is not final package evidence (`:248-250`). However,
it then states:

> The safe package gate is compilation/source-local evidence on Metal plus the
> existing `mtls-relay` GuestKit group and the three new worker cells inside
> `cgroup-default`.

That statement is at `:138-140`. The `Required next evidence` block at
`:265-285` lists staging, seven-body `mtls-relay` and 78-body
`cgroup-default`, but no normal package `nextest` run. The compile receipt at
`:249` proves that all three packages build; it does not execute their
ordinary tests. The two native groups prove selected real-kernel cells; they
do not replace ordinary control-plane, dataplane and netlink package tests.

The gap is concrete without hypothesizing a production defect: after following
the handoff exactly, STEP 13-05 could be declared complete with zero actual
ordinary test execution for the dataplane package even though that crate owns
`HostMtlsEnforcement` construction and the conditional leg-B dial. This is the
same omitted-package class the handoff itself identifies in the old
four-package run.

#### Required bounded remediation

Update only `deliver/distill-13-05-native-support.md` so that it:

1. removes the statement that compilation plus native subsets is the package
   gate;
2. requires a retained, actual normal `cargo nextest run` whose package set is
   exactly `overdrive-control-plane`, `overdrive-dataplane` and
   `overdrive-netlink`;
3. preserves the physical-host rule by using fail-closed selection/accounting
   that excludes only bodies independently routed to disposable Cloud
   Hypervisor guests, including the two cgroup-qualified startup bodies and
   all retained dataplane real-kernel families;
4. records the ordinary selected/pass/skip totals and exact exit zero, so
   package omission or an unaccounted exclusion fails closed;
5. keeps the separate current native gates: exactly seven passes in
   `mtls-relay`, and exactly 55 baseline + 18 STEP 13-04 + 5 STEP 13-05 = 78
   passes in `cgroup-default`; and
6. keeps `fa224a04f75a914094235928` only as causal classification evidence for
   the two cgroup-bootstrap ENOENT failures.

No production, runner, staging, test body, assertion, scenario, lane, group,
API, model, DES or commit change is requested by this finding. Native runs
remain correctly deferred until the handoff's source plan is approved.

## Mechanical verification

| Check | Result |
| --- | --- |
| Supplied artifact identities | PASS — handoff, runner and staging hashes match the review request exactly. |
| Exact marker delta | PASS — five deletions only; no selected body or assertion rewrite. |
| Exact body resolution | PASS — five runner identities, five live source functions, zero remaining STEP 13-05 ignore markers. |
| Contract Shape | PASS — two `unbounded-preservation` and three `bounded-change` declarations retained. |
| Existing baseline accounting | PASS — 45 invocations / 55 bodies. |
| STEP 13-04 accounting | PASS — 18 invocations / 18 bodies. |
| STEP 13-05 accounting | PASS — 5 invocations / 5 bodies. |
| Combined `cgroup-default` accounting | PASS — 68 invocations / 78 bodies, separately gated as 55 + 18 + 5. |
| Existing guest boots / CID slots | PASS — unchanged three guests, slots 14/15/6. |
| Selectable groups / staging lanes | PASS — unchanged eight groups and six lane declarations. |
| `mtls-relay` retention | PASS in source — unchanged exact seven-body gate in its separate disposable guest. |
| STEP 13-04 temporary route ordering | PASS — setup/cleanup remains around only two host-output cells; STEP 13-05 starts afterward. |
| STEP 13-05 setup | PASS — only route-localnet zero and gateway-local `/32`; no prefix route, default, uplink, mark rule or table-100 route. |
| Per-body audit | PASS — exact gateway, empty main table, failed non-gateway lookup, loopback-only interfaces and clean mark/table-100 state before/after each leg-B body. |
| Positive/negative honesty | PASS — production mark+bind/socket path and exact refusal/conditionality oracles remain required. |
| Tool delta | PASS — only `sysctl`; no selected Python call. |
| Kernel modules | PASS — no list or load-order change. |
| `/var/run` alias / cleanup | PASS — existing STEP 13-04 support retained. |
| UID/GID, device groups, Landlock, seccomp, `no_new_privs` | PASS — runner/staging delta does not change GuestKit confinement. |
| ADR-0172 representation | PASS — exact TCP reset then plain non-TCP fallback; gateway exclusion and prerouting drop unchanged. |
| Runner compile / Clippy | PASS as reported by durable Metal run `3c39180b1eda95aa2d1c7d3e`; not rerun by this reviewer. |
| Exact three-package compile | PASS as reported by durable Metal run `64bdd16ffeb8608976471ce3`; not an actual package test gate. |
| Exact active-name resolution | PASS as reported by durable Metal run `bd6cbed044ce86181d5ab970`. |
| Tool audit | PASS as reported by durable Metal run `c36446a6827b604452504e8a`. |
| Ordinary fixture-causal run | PASS for classification only — `fa224a04f75a914094235928`, 1,728/1,730, exactly two cgroup-bootstrap ENOENT failures; dataplane omitted. |
| Actual normal three-package test gate | **MISSING from the required next-evidence plan — blocker F-01.** |
| Native seven-body `mtls-relay` replay | NOT RUN — correctly deferred until source-plan approval. |
| Native 78-body `cgroup-default` replay | NOT RUN — correctly deferred until source-plan approval. |
| `cargo fmt --check` | PASS — independently rerun by this reviewer. |
| `git diff --check` | PASS — independently rerun by this reviewer. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun by this reviewer. |
| Mutation testing | NOT RUN — outside this per-step source review. |
| Production / DES / commit changes by reviewer | NONE. |

## Iteration 1 verdict

**CHANGES_REQUIRED.** The executable GuestKit support is correctly bounded.
It selects the five existing STEP 13-05 cells exactly once, preserves every
authored body and Contract Shape declaration, keeps the prior 55+18 manifest
identity, adds the five-body total as a separate fail-closed gate, and runs the
three leg-B cells only after STEP 13-04's temporary route is gone. The
qualified-node setup and before/after audit preserve the accepted GW-LOCAL and
LEGB-DIAL environment without a prefix-wide workaround, default route or
uplink. Tool, module, group, lane, public API, model and confinement boundaries
remain intact. The ADR-0172 four-rule output identity matches its exact
accepted TCP/non-TCP semantics.

One blocker remains in the handoff's verification contract. It substitutes
three-package compilation plus focused native subsets for the required actual
normal test run of `overdrive-control-plane`, `overdrive-dataplane` and
`overdrive-netlink`, and its required-next-evidence list omits that normal
gate. Correct the handoff plan only, retaining safe independent disposable-
guest accounting and both native group gates, then return this source package
for re-review. Do not start the native seven-body or 78-body replays until the
source plan is approved.

# CHANGES_REQUIRED — ITERATION 1

---

# Review iteration 2 — normal three-package gate remediation

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 2 |
| Review date | 2026-10-28 |
| Scope | Text-only remediation of F-01 in the source handoff's required verification plan |
| Prior disposition | Executable GuestKit support, five-cell identity, 55+18+5 accounting, qualified-node audit, confinement and ADR-0172 representation remain approved from iteration 1 |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews only the corrected verification contract in
`deliver/distill-13-05-native-support.md`. The runner and staging source
hashes are unchanged from iteration 1. No production source, test source,
runner source, staging source, body, assertion, marker, API, lane, group,
model, DES event, execution log or commit changed for this remediation.

Only this review artifact was appended. No normal package test or native
GuestKit replay has run yet; those remain the post-approval evidence required
from the original STEP 13-05 crafter.

## Iteration 2 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Corrected `deliver/distill-13-05-native-support.md` | `6516f0f53067cb02366c4292359fb3ac09c2351ae3018fa53e57f768a81db2de` |
| Iteration 1 review before this append | `df9a286fcc33848ff677397492c682a93c2dab418626a9b5d4a23cf34809d958` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `290d25e1734f890190f9be592f3c48cf88cf865adccd89501f2b6cd7b1d42d05` — unchanged |
| `xtask/src/guest_kit_prepare.rs` | `9ff73d8fb6c506995e7e4936b503ba9807ddd8fca428484d37988dc7545d8eb7` — unchanged |

All executable-source approvals from iteration 1 therefore remain in force:
the exact two control-plane plus three worker cells, their Contract Shape
declarations and unchanged bodies; the existing three disposable guests and
CID slots; separate 55, 18, 5 and 78 runtime gates; STEP 13-04 route cleanup
before STEP 13-05; route-localnet/GW-LOCAL/no-route/no-uplink read-back around
each leg-B body; `sysctl`-only tool expansion; unchanged module closure,
`/var/run` alias and confinement; and the exact ADR-0172 TCP-reset then
non-TCP fallback identity.

## F-01 remediation disposition

**RESOLVED.** The corrected handoff no longer substitutes compilation,
source inspection, focused tests or the two native groups for the roadmap's
ordinary package test gate.

The corrected contract now states explicitly that:

- an actual `cargo nextest run` over exactly `overdrive-control-plane`,
  `overdrive-dataplane` and `overdrive-netlink` is required in addition to
  native evidence (`distill-13-05-native-support.md:115-140`);
- compile run `64bdd16ffeb8608976471ce3` is compile-only preflight and cannot
  satisfy that gate (`:270-287`);
- retained run `fa224a04f75a914094235928` remains only causal
  classification evidence for the two typed cgroup-bootstrap ENOENT
  failures, and its dataplane omission prevents final-gate use (`:274-287,
  :331-339`);
- the exact list inventory and selector must be retained before the normal
  run, with an identity-level ledger for every excluded kernel-, cgroup-,
  namespace-, route-, nftables- or BPF-mutating family (`:142-166`);
- every exclusion requires an already-existing disposable Cloud Hypervisor
  route and exact expected native count; an unaccounted family blocks the
  run instead of becoming a physical-host execution or silent skip
  (`:142-166, :300-306`);
- the actual normal run executes as the unprivileged Metal login user, so a
  selector mistake cannot gain the root permission surface needed for the
  forbidden physical-host mutations (`:308-329`);
- the retained receipt must name all three packages and the list/selector
  identity, reconcile exact selected/passed/skipped/failed totals, report
  zero failures and exit zero, and fail closed for package omission,
  unaccounted exclusion or concealment through an infrastructure skip
  (`:331-339`); and
- only after the normal gate passes do the separate current native gates run:
  exactly seven `mtls-relay` bodies and exactly 55 baseline + 18 STEP 13-04 +
  5 STEP 13-05 = 78 `cgroup-default` bodies, each with zero failed/ignored
  and exit zero (`:341-358`).

This is the bounded remediation iteration 1 requested. It restores the
roadmap's actual three-package test surface while preserving the repository's
stronger physical-host safety boundary through explicit, independently
accounted disposable-guest routing.

## Normal/native boundary reconciliation

The handoff's initial ledger accounts for the known excluded package families:

| Normal-package family routed away from the physical namespace | Existing disposable route | Exact separately required bodies |
| --- | --- | ---: |
| Control-plane cgroup-qualified baseline | `cgroup-default` control-plane acceptance/integration guests | 3 + 45 |
| Control-plane STEP 13-04 native | `cgroup-default` control-plane integration guest | 6 |
| Control-plane STEP 13-05 startup refusals | `cgroup-default` control-plane integration guest | 2 |
| Dataplane `mtls_inbound_guest_relay` | `mtls-relay` guest | 7 |

The first three rows reconcile to the 56 control-plane bodies inside the
separate 78-body `cgroup-default` gate. The other 22 bodies in that gate are
worker-package bodies and therefore are not part of the normal three-package
selection. The dataplane relay stays a separate seven-body native gate.

The handoff does not pretend this initial table is a complete precomputed
selector. It requires the actual `nextest list` inventory to identify every
additional dataplane real-kernel family exposed by `integration-tests`. Each
such family must be absent from selection or gain an existing disposable
route and exact count before it can be excluded; otherwise the package gate
is blocked. That fail-closed materialization is the correct stage for exact
totals, because inventing totals before the actual list/selector exists would
be weaker evidence.

The planned `--no-sudo` placement is valid for the current Metal CLI: it is a
flag of `metal run`, before the trailing command. The actual `nextest run`
still names all three roadmap packages. Its selector changes placement of
independently accounted native bodies, not the package set or the obligation
to execute every ordinary userspace/Sim body.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Coverage / package traceability | PASS | The actual normal gate names all three exact roadmap packages; native exclusions retain identity-level routes and counts. |
| Observable evidence | PASS | Exact totals, reconciliation, zero failures and process exit zero are required; compile success cannot stand in for test execution. |
| Boundary honesty | PASS | Ordinary tests run unprivileged; every kernel-mutating exclusion must have an existing disposable-guest proof. |
| Fail-closed selection | PASS | Unknown physical-effect families, missing routes, package omission, unaccounted exclusions and infrastructure-skip concealment block completion. |
| Native separation | PASS | Seven relay bodies and 78 cgroup-default bodies remain independent gates and are not credited to the ordinary totals. |
| Scope | PASS | Text-only handoff correction; no scenario, body, assertion, API, lane, group, model, production or DES change. |
| F-01 | **RESOLVED** | The required actual normal package run is restored with retained identity/accounting evidence. |

The acceptance-review scores and mandate results from iteration 1 remain
unchanged. F-01 was the sole blocker and concerned the verification contract,
not an executable test-design defect. With the corrected contract, there are
no open DISTILL findings.

## Iteration 2 mechanical verification

| Check | Result |
| --- | --- |
| Corrected handoff identity | PASS — exact requested SHA-256 `6516f0f53067cb02366c4292359fb3ac09c2351ae3018fa53e57f768a81db2de`. |
| Runner source identity | PASS — unchanged from iteration 1. |
| Staging source identity | PASS — unchanged from iteration 1. |
| Actual normal package test required | PASS — explicit `cargo nextest run`, no compile/focused/native substitution. |
| Exact package set | PASS — control-plane, dataplane and netlink, matching roadmap STEP 13-05. |
| Exact list/selector artifacts | PASS — required before execution with retained identity/digest. |
| Kernel-mutating family ledger | PASS — every exclusion requires an existing disposable route and exact count. |
| Unaccounted-family handling | PASS — blocks before execution; no physical-host fallback or silent skip. |
| Physical-host permission boundary | PASS — normal inventory/run use `metal run --no-sudo`; native effects stay in disposable guests. |
| Ordinary total reconciliation | PASS — exact selected/passed/skipped/failed totals, zero failed and exit zero required. |
| Package omission / concealed skip | PASS — explicitly fail the gate. |
| Old 1,728/1,730 run | PASS — retained as causal classification only, never final evidence. |
| `mtls-relay` native gate | PASS in plan — separate exact 7, zero failed/ignored, exit zero. |
| `cgroup-default` native gate | PASS in plan — separate exact 55 + 18 + 5 = 78, zero failed/ignored, exit zero. |
| Broad/normal package execution | NOT RUN — correctly follows source-plan approval. |
| Native 7/78 execution | NOT RUN — correctly follows source-plan approval and normal-gate success. |
| `cargo fmt --check` | PASS — independently rerun in iteration 2. |
| `git diff --check` | PASS — independently rerun in iteration 2. |
| Source/body/API/lane/model/DES/commit change | NONE. |

## Iteration 2 verdict

**APPROVED.** F-01 is resolved. The corrected handoff restores an actual
normal `nextest` gate over the exact control-plane, dataplane and netlink
packages, requires retained list/selector identity and reconciled result
totals, and fails closed when a kernel-mutating family lacks an independently
accounted disposable-guest route. Compilation, focused tests, the old
dataplane-omitting run and the native subsets are explicitly barred from
substituting for that normal gate.

The seven-body `mtls-relay` and 78-body `cgroup-default` runs remain separate
mandatory native evidence after the normal gate, with exact zero-failure and
exit-zero requirements. The executable GuestKit source remains byte-identical
to iteration 1 and retains that iteration's approval of scenario identity,
fixture honesty, route ordering, accounting, tools, modules, confinement and
ADR-0172 rule shape.

The original STEP 13-05 crafter may now materialize the retained normal
inventory/selector/ledger, run the actual three-package gate, and only after
it passes run the separate seven-body and 78-body disposable-guest gates. Any
unaccounted physical-effect family or non-reconciling total reopens this
review instead of being silently excluded.

# APPROVED — ITERATION 2

---

# Review iteration 3 — complete dataplane native-family routing

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 3 |
| Review date | 2026-10-28 |
| Scope | Bounded routing of the 36 existing dataplane kernel/BPF/netns/XDP/kTLS bodies identified by the approved three-package inventory |
| Prior disposition | Iterations 1 and 2 remain approved: five STEP 13-05 cells, `cgroup-default` 55+18+5 accounting, qualified-node fixture, confinement, ADR-0172 identity, and the mandatory actual normal three-package gate |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews the source support added after the iteration-2 list
plan was materialized. The retained three-package inventory proved that the
dataplane integration binary contains 36 active physical-kernel bodies beyond
the seven existing inbound guest-relay bodies. The runner now executes those
36 bodies through the existing `mtls-relay` disposable guest before the
normal selector may exclude them.

Only this review artifact was appended. No production source, acceptance
body, assertion, pending marker, public API, CI lane, selectable runner group,
scenario, model, DES event, execution log or commit was changed by this
reviewer. No native 43-body/78-body replay or actual normal three-package test
gate has run yet.

## Iteration 3 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `87186b22760c8b8a0d9cf6301190b289ead259955cec87acf4a68f3de2d9bc26` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `ce01d938cb2450a03a2611267ea53fde14aeb9d79486bb7671904f8192423956` |
| `xtask/src/guest_kit_prepare.rs` | `19d6a300754a706e21f7d075c1b7343331e7163d8d9dc74d64a5f63a78cca541` |
| Approved three-package list receipt | `4bd78f192e3d017588616b041fbe25d678198adb407b02714f44876666d192b9` |
| Iteration 2 review before this append | `a85a14e20059b8fbd1762fa237101b05a0bef501590dc7beba3ba6ff683511f2` |

The production, selected STEP 13-05 test-body and ADR-0172 source inspected in
iteration 1 is unchanged by this support iteration. The existing
`cgroup-default` manifest remains exactly 55 baseline + 18 STEP 13-04 + 5
STEP 13-05 = 78 bodies across the same three guests and CID slots.

## Approved list inventory

The retained list came from durable Metal run
`19999a70f5afa15060a6d75c`, exit zero. Independent parsing of the JSON body
inside the retained receipt reproduces the handoff's package totals:

| Package | Listed identities |
| --- | ---: |
| `overdrive-control-plane` | 1,126 |
| `overdrive-dataplane` | 179 |
| `overdrive-netlink` | 50 |
| **Total** | **1,355** |

The dataplane integration binary contains 64 listed identities: 61 active and
three ignored future/negative-environment bodies. The 61 active identities
partition without overlap as:

| Partition | Bodies | Execution boundary |
| --- | ---: | --- |
| Existing `mtls_inbound_guest_relay` | 7 | Original first phase of existing `mtls-relay` guest |
| Ordinary/build/parser bodies | 18 | Actual normal three-package selector |
| Kernel/BPF/netns/XDP/kTLS bodies | 36 | New exact second phase of existing `mtls-relay` guest |
| **Active total** | **61** | — |

The three ignored dataplane integration bodies remain absent from all active
manifests. No pending/future test was activated to reach 36.

## Exact 36-body identity proof

Independent extraction of `DATAPLANE_NATIVE` gives 36 rows and 36 unique exact
names. Subtracting the seven relay identities and the 18 ordinary/build/parser
identities from the 61 active inventory gives another 36-name set. Sorted set
comparison reports no missing and no extra identity.

| Existing family | Exact active bodies |
| --- | ---: |
| `atomic_swap` | 7 |
| `deregister_retry_safety` | 1 |
| `local_backend_proto_connect` | 1 |
| `maglev_real` | 1 |
| `mtls_agent_handshake` | 1 |
| `mtls_guardrails` | 1 |
| `mtls_inbound_enforce` | 1 |
| `multi_listener_tcp_udp_e2e` | 4 |
| `redirect_neigh_attach` kernel body | 1 |
| `reverse_nat_e2e` | 5 |
| `reverse_nat_udp_e2e` | 3 |
| `sanity_mixed_batch` | 1 |
| `service_map_forward` | 1 |
| `service_map_vip_port` | 1 |
| `unconnected_udp_reply_hardening` kernel bodies | 2 |
| `unconnected_udp_roundtrip` | 4 |
| `veth_attach` kernel body | 1 |
| **Total** | **36** |

Every row is exact, active and expected to pass one body. The private manifest
adds no test body, assertion, ignore transition or scenario. Parser-only
siblings stay in the ordinary package run; the three ignored/future bodies
stay ignored and outside the 36-body credit.

## Existing group, guest and phase order

The selectable group registry remains the same eight values. `mtls-relay`
still uses the existing `overdrive-dataplane--integration` staged binary,
guest name and CID slot 13. No new lane, target, guest protocol or executor is
introduced.

Within that same disposable guest, the order is fail-closed:

1. boot with the original TLS module as the only loaded extra module;
2. execute the original `integration::mtls_inbound_guest_relay::` prefix and
   require exactly seven passes;
3. load the additional dependency-ordered module closure through the existing
   scripted guest peer;
4. mount the guest-private cgroup-v2 and bpffs filesystems, install the
   `/var/run -> /run` compatibility alias, extract the staged Python standard
   library and prove every external precondition;
5. execute the 36 identities one at a time through exact libtest selection;
6. require exactly 36 routed-native passes; and
7. require exactly 7 + 36 = 43 combined passes before powering off the guest.

The additional tools, object file, Python archive and unloaded module files
are present in the image before phase 1, but the baseline's kernel module
state remains the original TLS-only state. The new cgroup/bpffs/netns/BPF/nft
substrate is established only after all seven original bodies pass. Therefore
the new substrate cannot make the original relay phase green.

## Runtime skip honesty

Both the seven-body baseline invocation and each of the 36 exact invocations
use `exec_required`. It preserves the existing exit-zero and exact
`<N> passed; 0 failed` summary gates, then additionally rejects console output
containing `[skip]`, `skip:` or `(skip)`.

Source inspection of the routed families shows that every capability bailout
which returns from an otherwise successful libtest emits one of those exact
forms. This includes root/CAP_BPF/CAP_NET_ADMIN, veth/netns and below-floor
precondition bailouts. The selected mTLS bodies that cannot degrade safely
already fail loudly instead of returning. The scan therefore closes the
known “libtest ok after infrastructure skip” path without altering a body or
turning a missing capability into native evidence.

The scan is scoped to required native invocations. Existing non-required
runner groups retain the prior executor behavior, so a generic diagnostic in
an unrelated group cannot create a new failure. Within the selected 43
bodies, no normal successful diagnostic contains the three reserved skip
forms. Exit failure is evaluated first, so a genuine test failure remains the
reported cause rather than being relabelled as a skip.

## Tools and Python runtime

The additional source invokes the existing test-support tools only:
`bpftool`, `cat`, `id`, `ip`, `kill`, `ln`, `mkdir`, `mount`, `nc`, `nft`,
`python3`, `sh`, `ss`, `strace`, `sysctl`, `tar`, `tcpdump`, `true` and
`uname`. `tool()` stages each at `/usr/bin` with the existing ELF interpreter
and shared-library closure behavior. Read-only Metal audit
`58bdbec68c4ea7b3bc69fc54` found every tool and the matching
`/usr/lib/python3.14` standard library; it ran no test and performed no host
kernel mutation.

The selected UDP fixtures execute existing Python snippets importing
`socket`, `sys` and `time`. The runner queries the qualified host interpreter
through `sysconfig`, archives that exact standard-library directory, stages
the archive at `/opt/overdrive-testing/python-stdlib.tar`, and extracts it to
the same absolute location inside the disposable guest. This retains the
matching `lib-dynload` modules used by `socket`; it does not introduce a
replacement workload, change a test body or let the guest use a different
runtime library.

The standard `/var/run -> /run` alias is created before any netns fixture and
validated by canonical path and directory read-back. All paths and extracted
runtime files die with the disposable guest/work directory.

## BPF artifact identity

`cargo xtask guest-kit-prepare` remains the sole producer for this native
route. It runs `bpf-build` with `OVERDRIVE_BPF_NATIVE=1` before compiling and
staging the dataplane integration binary. The runner then reads the same
workspace-scoped contract path and stages it at the identical in-guest path:

```text
/home/ubuntu/overdrive/target/bpf/overdrive_bpf.o
```

Image construction refuses if that exact file is absent. The in-guest setup
also requires the exact path to be readable before any of the 36 invocations.
File-loading bodies therefore consume the artifact built immediately before
their staged libtest binary; `include_bytes!` bodies retain their separately
compiled embedded identity. With no competing Metal run and one preparation
step, the runner does not substitute a shared target-directory artifact from
another workspace.

This is test-fixture staging of an existing artifact contract. It adds no BPF
program, production hook, loader API or public path.

## Private filesystems and module closure

The routed-native setup mounts cgroup v2 and bpffs inside the disposable guest
and verifies both through `/proc/mounts`. It never mounts or attaches against
the physical Metal host. The guest peer loads the extra modules in dependency
order after the baseline phase:

1. `nfnetlink`, `nf_tables`, `veth`;
2. `nf_defrag_ipv4`, `nf_defrag_ipv6`;
3. `nf_conntrack`, then `nf_nat`;
4. IPv4/IPv6 reject and tproxy protocol helpers;
5. `nft_reject`, `nft_reject_ipv4`, `nft_tproxy`; and
6. `nft_chain_nat`, then `nft_masq`.

TLS was already loaded for the original relay phase. Read-only Metal audit
`afb0538bee7e6b0e28a2f6df` found every declared module file. The
`cgroup-default` nested-device and STEP 13-04 module closure is unchanged.

All module loads, BPF loads/attachments/pins, XDP attachments, nftables
objects, cgroups, routes, network namespaces, captures and kTLS sockets are
created by the existing guest peer or staged test binary after Cloud
Hypervisor boot. Guest destruction is the final fail-closed cleanup boundary.

## Confinement and physical-host boundary

The broader staged payload does not change the Cloud Hypervisor boundary.
The VMM retains uid/gid 4200, the actual KVM and vhost-vsock device-owner
supplementary groups, root-owned parent traversal through gid 4200 mode
`0710`, per-allocation ownership, Landlock grant, seccomp filter,
`no_new_privs`, device modes, runtime lifetime and RAII cleanup.

The physical Metal side only builds/reads the BPF object and staged libtest,
reads tool/runtime/module bytes, creates the guest image and launches the
confined VMM. It issues no physical-host module load, sysctl, nft operation,
BPF load/attach/pin, XDP attach, route/rule operation, cgroup mount or network
namespace mutation for this group.

## Complete normal/native overlap ledger

Iteration 2 left the dataplane real-kernel row deliberately pending the list
inventory. Iteration 3 closes that row exactly:

| Three-package identity excluded from normal selector | Disposable route | Exact separately required bodies |
| --- | --- | ---: |
| Control-plane cgroup-qualified baseline | Existing `cgroup-default` control-plane guests | 3 acceptance + 45 integration |
| Control-plane STEP 13-04 native | Existing `cgroup-default` control-plane integration guest | 6 |
| Control-plane STEP 13-05 startup refusals | Existing `cgroup-default` control-plane integration guest | 2 |
| Dataplane inbound guest relay | Existing `mtls-relay` guest, phase 1 | 7 |
| Dataplane kernel/BPF/netns/XDP/kTLS partition | Same existing `mtls-relay` guest, phase 2 | 36 |
| **Native-covered exclusions** | — | **99** |

The control-plane rows are 56 bodies inside the independent 78-body
`cgroup-default` gate; its other 22 bodies belong to the worker package and
are not part of the three-package normal selector. The dataplane rows are 43
of 61 active integration bodies; the other 18 stay in the normal run. The
netlink package has no native exclusion in this ledger.

Only passing native receipts authorize the selector to exclude those exact 99
identities. Any further exclusion needs separate accepted authority and
independent evidence. The selector must retain the approved list digest and
reconcile its resulting selected count before the actual normal run.

The actual normal gate remains mandatory and unchanged in substance: it runs
as the unprivileged Metal login user over exactly
`overdrive-control-plane`, `overdrive-dataplane` and `overdrive-netlink`,
includes every remaining ordinary userspace/Sim body, and records exact
selected/passed/skipped/failed totals with reconciliation, zero failures and
exit zero. Compile success, focused tests, the causal 1,728/1,730 run and
either native group remain explicitly invalid substitutes.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Exact inventory | PASS | Retained exit-zero list gives 1,126 control-plane, 179 dataplane and 50 netlink identities. |
| Dataplane partition | PASS | 61 active = 7 relay + 18 ordinary + 36 native; three ignored future bodies remain excluded from active credit. |
| Manifest identity | PASS | 36 exact unique runner names equal the native partition byte-for-byte; no missing or extra row. |
| Existing boundary reuse | PASS | Same `mtls-relay` group, guest, CID, staged binary and guest executor. |
| Baseline preservation | PASS | Original seven run first with TLS as the only loaded extra module and remain separately counted. |
| Routed-native accounting | PASS | Exact 36 and combined 43 gates are independent of the 78-body cgroup gate. |
| Runtime skip honesty | PASS | Every known capability bailout emits a reserved skip token rejected by `exec_required`. |
| Tool/runtime closure | PASS | Exact external tools and matching Python stdlib are staged; no workload body is recreated. |
| BPF identity | PASS | Mandatory preparation builds before libtest staging; exact workspace artifact path is staged/read back. |
| Module/filesystem closure | PASS | Private cgroup/bpffs/netns substrate and dependency-ordered modules exist only inside the guest. |
| Physical-host safety | PASS | Host performs staging/VMM launch only; all kernel effects occur after guest boot. |
| Normal package gate | PASS in plan | Exact three packages, overlap ledger, reconciled totals and no compile/subset substitution remain required. |
| Scope | PASS | Test-support routing only; no product API, lane, selectable group, scenario, model or mechanism. |

The 36 bodies are pre-existing integration scenarios, so the C1-C7 taxonomy,
Contract Shape declarations and scenario completeness are unchanged. This
iteration closes execution-boundary completeness: every active dataplane
kernel-mutating identity in the approved inventory now has an exact
disposable-guest route. Delta completeness remains 15/15 under the canonical
checklist's N/A rule.

## Iteration 3 mechanical verification

| Check | Result |
| --- | --- |
| Supplied handoff/runner/staging hashes | PASS — exact requested identities. |
| Retained list receipt hash | PASS — `4bd78f192e3d017588616b041fbe25d678198adb407b02714f44876666d192b9`. |
| Package inventory | PASS — 1,126 / 179 / 50, total 1,355. |
| Dataplane integration active count | PASS — 61. |
| Baseline relay partition | PASS — exact 7. |
| Ordinary/build/parser partition | PASS — exact 18. |
| Native partition | PASS — exact 36. |
| Runner manifest count / uniqueness | PASS — 36 / 36. |
| Inventory-to-runner set comparison | PASS — zero missing, zero extra. |
| Ignored/future bodies | PASS — three remain ignored and outside the active manifest. |
| `mtls-relay` phase order | PASS — seven baseline before module/substrate expansion, then 36 exact rows. |
| `mtls-relay` runtime totals | PASS in source — separately gated 7, 36 and 43. |
| Runtime infrastructure skip scan | PASS — required invocations reject all skip forms emitted by selected bailout paths. |
| Existing group / guest / CID / executor | PASS — unchanged `mtls-relay`, slot 13 and staged dataplane integration binary. |
| `cgroup-default` | PASS — unchanged exact 55 + 18 + 5 = 78 boundary. |
| Tools / Python standard library | PASS in source; read-only audit `58bdbec68c4ea7b3bc69fc54`. |
| BPF artifact | PASS — exact prepared workspace path, staged before boot and read back in guest. |
| Module files / dependency order | PASS in source; read-only audit `afb0538bee7e6b0e28a2f6df`. |
| cgroup-v2 / bpffs / netns alias | PASS — guest-private setup with fail-closed read-back. |
| UID/GID, device groups, Landlock, seccomp, `no_new_privs` | PASS — unchanged. |
| Physical-host kernel mutations | NONE. |
| Actual native 43-body replay | NOT RUN — required next after this source approval. |
| Actual native 78-body replay | NOT RUN — required next after this source approval. |
| Actual normal three-package gate | NOT RUN — required after passing native receipts. |
| Format / runner and xtask Clippy | PASS as reported by durable Metal run `6917b5cccd28320c5e7a0b0a`; `cargo fmt --check` independently rerun. |
| Tool audit | PASS as reported by read-only Metal run `58bdbec68c4ea7b3bc69fc54`. |
| Module-file audit | PASS as reported by read-only Metal run `afb0538bee7e6b0e28a2f6df`. |
| `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| New production/body/API/lane/group/scenario/model/DES/commit change | NONE. |

## Iteration 3 verdict

**APPROVED.** The approved inventory's 36 active dataplane physical-kernel
bodies are now routed exactly once through the existing `mtls-relay`
disposable guest. The original seven relay bodies remain first and separately
gated under their original TLS-only loaded-module state. The private runner
then establishes the exact guest-only module, cgroup-v2, bpffs, netns, BPF,
tool and Python substrate, rejects every known runtime capability bailout,
and gates the routed-native and combined totals at 36 and 43.

The runner adds no product API, lane, selectable group, scenario, model or new
test mechanism. It preserves the `cgroup-default` 78-body boundary and all
prior STEP 13-05 fixture, confinement and ADR-0172 approvals. The physical
Metal host remains a staging and confined-VMM launch substrate.

The normal three-package gate remains mandatory after both native gates pass.
Its selector may exclude only the exact 99 native-covered identities in the
retained overlap ledger, plus any independently authorized non-native
exclusion, and must reconcile exact totals with zero failures and exit zero.
No native or normal result is claimed by this source approval.

The original STEP 13-05 crafter may now run and retain the exact 43-body
`mtls-relay` and 78-body `cgroup-default` receipts, then materialize the
identity-ledger selector and execute the actual unprivileged normal gate over
control-plane, dataplane and netlink. Any runtime skip, identity mismatch,
unaccounted exclusion or non-reconciling total reopens this review.

# APPROVED — ITERATION 3

---

# Review iteration 4 — complete all-binary native routing and ordinary fixture corrections

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 4 |
| Review date | 2026-10-28 |
| Scope | Classification of the first actual unprivileged normal run, bounded correction of two ordinary fixtures, exact routing of 20 additional control-plane and three dataplane-library native identities, and the complete 122-identity normal/native overlap ledger |
| Prior disposition | Iterations 1-3 remain approved: five STEP 13-05 native cells, 78-body `cgroup-default`, 43-body retained `mtls-relay`, exact dataplane-integration partition, qualified-node fixture, confinement, ADR-0172 identity and mandatory actual normal three-package gate |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration follows the first actual unprivileged three-package run. That
run exposed native identities outside the integration-only inventory audit and
two ordinary fixtures that failed before their existing oracle. The updated
support routes only the proven native identities into the two existing
disposable-guest groups and keeps the two corrected fixtures in the normal
run.

Only this review artifact was appended. No production source, acceptance
assertion, pending marker, public product API, CI lane, selectable runner
group, scenario, model, DES event, execution log or commit was changed by the
reviewer. The native 46-body and 98-body gates and the clean 1,224-body normal
rerun have not executed yet.

## Iteration 4 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `f936489c4cb28c345355d111d36834f04f7960e53566826f19587e230a8c41ab` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `592a33d22b7bc8df0a3e6c07624c6489b6f2ac432188ab745413b85ce71df5ae` |
| `xtask/src/guest_kit_prepare.rs` | `086f1214d6244e801d492e2211a1c99cadf771265d94f2268ab502bebbf951e1` |
| `action_shim_restart_uses_spec_from_action.rs` | `b891e746a542d2382d9a4531254ea5ec20955cc8a551fe2d2ae43c1c3802f7e9` |
| `redb_view_store.rs` | `d98ea505039c84f7134ea1095e3eb151584cf6bb22630686f0212f92d8b1fe5a` |
| Approved three-package list receipt | `4bd78f192e3d017588616b041fbe25d678198adb407b02714f44876666d192b9` |
| First actual normal-run receipt | `d11a288b8e76bb9cecc68f58a0b9a638b04dbeef3c812440f1ab2405996a3b23` |
| Iteration 3 review before this append | `405af23b829f6aaa89941333125588203e5bd3b00376dfb01bada5a543bc4fa0` |

The selected STEP 13-05 bodies and concurrent production implementation are
unchanged by this iteration. The two ordinary fixture files and the two
DISTILL execution-support files are the only new review surfaces.

## First actual normal-run evidence

Durable Metal run `3fb5a2f1e2bc7a8f2fccd58e`, nextest
`d8df41da-9320-4b67-b7cf-354cb4eef2d3`, used seed `0x94db98ed` and the exact
three packages. Its retained raw output reports:

| Result | Count |
| --- | ---: |
| Selected/run | 1,247 |
| Passed | 1,240 |
| Failed | 7 |
| Skipped/excluded | 108 |
| Full listed identity total | 1,355 |

The accounting reconciles: `1,247 + 108 = 1,355`. The run is classification
evidence, not a passing final gate.

Independent parsing finds 18 distinct successful-libtest capability bailouts:
16 control-plane integration bodies printing uppercase `SKIP ...` and two
dataplane library bodies printing `[skip] ...`. The seven unique failures are:

- the action-shim restart-spec ordinary fixture;
- two root/kernel DNS composition bodies;
- the redb read-only ordinary fixture;
- two real workload-netns bodies; and
- one dataplane real-map library body.

The two ordinary fixture failures stay in the normal selector after bounded
fixture correction. The other five failures plus the 18 explicit bailouts are
23 genuine native identities. This is an evidence-derived partition, not a
blanket conversion of every first-run failure into a native exclusion.

## Ordinary fixture correction 1 — restart-spec action-shim scenario

The existing scenario's declared observable is the exact
`AllocationSpec` received by `Driver::start`: command, args, resources and
single start call. Its fixture explicitly uses no canonical workload address
and no guest transport. It does not claim workload-netns provisioning.

The prior generic `dispatch` composition supplied the real host network
provisioner for a VM-shaped driver and reached `NetnsDelFailed` before the
recording driver's oracle. The correction uses the already-existing
`dispatch_with_network_provisioner` entry point and supplies a private
`NoopNetworkProvisioner` only for that out-of-scope driven port.

This is an honest fixture boundary:

- the action still enters the production action-shim dispatcher;
- the real driver registry, observation store, lifecycle channel, allocator,
  broker and mTLS worker wiring remain;
- the `RestartAllocation` action and full VM `AllocationSpec` are unchanged;
- every existing captured-driver assertion is byte-for-byte unchanged;
- the private adapter implements only the existing provision/teardown port;
- no production default, fallback, test override field or public seam is
  added; and
- a defect in spec forwarding, driver selection or driver invocation still
  fails the scenario.

The no-op does not weaken a VM-netns contract because this scenario declares
no transport/netns input and makes no network outcome claim. Tests that own
real network provisioning remain in the exact native manifests. The adapter
cannot make those tests pass and is not used by production composition.

## Ordinary fixture correction 2 — redb read-only storage scenario

The original fixture materialized `memory.redb`, closed the store, then made
only the parent directory read-only. An unprivileged process could still open
the existing writable database file `O_RDWR`, so the probe correctly
succeeded and the fixture's read-only premise was false.

The correction now:

1. materializes and closes the store, releasing the writable redb handle;
2. resolves the existing canonical `memory.redb` path;
3. records the file's and directory's original permissions;
4. applies `0400` to the database file and `0500` to its parent;
5. reopens and exercises the existing probe/open refusal path;
6. accepts the pre-existing stronger open-side refusal or the existing
   `ProbeError::WriteFailed` probe result; and
7. restores both original permission objects before tempdir cleanup.

The test no longer relies on a writable file descriptor opened before chmod.
It preserves the raw file, canonical path, read permission, expected typed
probe failure, open-error alternative and cleanup semantics. It changes no
redb adapter, `ViewStore`, persisted shape or production permission handling.

The two corrected ordinary fixtures passed together as the unprivileged Metal
login identity in durable run `c72471e83359a3f4b7d38b01`: two selected, two
passed, zero failed. They therefore remain ordinary normal-gate identities and
are not hidden by native routing.

## All-binary root and capability audit

The inventory audit now covers every binary kind from the three packages, not
only `overdrive-dataplane::integration`:

- control-plane library, acceptance, integration, compile-pass/fail and
  registry binaries;
- dataplane library, integration, acceptance, property, schema and tool
  binaries; and
- the netlink library.

The runtime log's 18 explicit bailouts was reconciled with source searches for
root, `CAP_NET_ADMIN`, `CAP_BPF`, `CAP_SYS_ADMIN`, netns and BPF-map guards.
The complete additional partition is:

| Package binary | Runtime bailouts | Native failures | Exact native bodies |
| --- | ---: | ---: | ---: |
| Control-plane integration | 16 | 4 | 20 |
| Dataplane library | 2 | 1 | 3 |
| **Additional total** | **18** | **5** | **23** |

The remaining control-plane `alloc_netns_lifecycle` root checks guard optional
bonus observations only after the unprivileged in-memory slot oracle has
already run; they do not turn an unexecuted native contract into a pass.
Previously routed control-plane cgroup/bridge bodies and dataplane integration
bodies remain in the earlier exact manifests. Netlink has no active root/CAP
bailout. No other successful body in the first run concealed its primary
oracle behind a privilege return.

## Exact additional manifest identity

Independent extraction from the first-run receipt yields 20 expected
control-plane names and three expected dataplane-library names. Independent
extraction from `CONTROL_PLANE_NORMAL_NATIVE` and `DATAPLANE_LIB_NATIVE`
yields 20/20 and 3/3 unique names. Set comparison gives zero missing and zero
extra identities in both manifests.

The 20 control-plane identities are exactly:

| Family | Bodies | Native reason |
| --- | ---: | --- |
| `adopt_on_restart` | 2 | Real cgroup/netns/nft restart adoption and cleanup |
| `dns_responder_bind` | 7 | Real privileged `:53`, socket audit and composed-kernel probe |
| `serve_boot_provisions_veth` | 2 | Real veth/XDP attach and collision classification |
| `veth_provision_idempotent` | 7 | Real veth/netns/offload convergence and repair |
| `workload_netns_provision` | 2 | Real netns/veth/sysctl/resolver provisioning |
| **Total** | **20** | — |

The three dataplane library identities are the two existing real
`hash_of_maps` BPF-map tests and the existing real reverse-local-map deletion
test. No integration body is duplicated in the library phase.

The new arrays contain exact test identities, expected count one and exact
selection. They add no body, assertion, ignore transition or future test.

## Existing group and binary staging

The runner registry remains the same eight selectable groups. The additional
identities reuse only `mtls-relay` and `cgroup-default`.

The staging manifest adds one record for the existing
`overdrive-dataplane` library test target. `InGuestTarget::Lib` renders the
existing Cargo `--lib` selector and stages the reported existing libtest
binary beside the integration binary. This is an additional binary in the
existing GuestKit preparation route, not a new CI/selectable lane, test
executor or product surface. The existing scripted peer and exact libtest
protocol execute it at `/opt/overdrive-testing/dataplane-lib-tests`.

The current staging contract units pass 2/2 in durable Metal run
`3251ba18a4ece881cf4200bb`. Current runner/xtask formatting and Clippy pass in
durable run `dade15f1bf48082997381fec`.

## `mtls-relay` exact phase accounting

The same guest name, CID slot 13, integration binary, scripted peer and
selectable group remain. Its order and independent gates are:

1. original relay prefix under original TLS-only loaded-module state: exactly
   7;
2. previously approved integration-native exact identities: exactly 36;
3. retained subtotal: exactly 43;
4. existing dataplane library binary exact identities: exactly 3; and
5. complete total: exactly 46.

The three library identities execute only after the 43-body boundary. They use
the same already-established guest-private bpffs/cgroup/module/BPF substrate;
no second group, boot or executor is created. Every invocation uses the
required skip policy, exact exit-zero summary and one-pass expectation.

## `cgroup-default` exact phase accounting

The same three guest names, CID slots 14/15/6 and staged binaries remain. The
historical counters remain independent:

| Boundary | Bodies |
| --- | ---: |
| Baseline | 55 |
| STEP 13-04 | 18 |
| STEP 13-05 | 5 |
| **Retained subtotal** | **78** |
| Additional control-plane native | 20 |
| **Complete total** | **98** |

Within the control-plane integration guest, its retained 45 baseline + 6 STEP
13-04 + 2 STEP 13-05 bodies execute before the 20 new bodies. The acceptance
and worker guests retain their existing 3 and 22 bodies. Because these are
separate disposable guests, the aggregate 78 subtotal is verified from their
independent retained counters before the complete 98 check; no new
control-plane effect can leak into the later worker guest.

The control-plane integration image stages the additional tools/modules
needed by its new exact suffix, but each historical body remains ahead of that
suffix. No historical body, filter or expected count is rewritten. The new 20
use `exec_cgroup_required`, so uppercase `SKIP ` and the previously supported
lowercase skip forms make the group fail even when libtest reports one pass.

## Control-plane native substrate

The new control-plane suffix reuses the existing private cgroup-v2 and bpffs
mounts, `/var/run -> /run` alias, `ip`, `nft`, `uname`, staged BPF object and
veth/TLS modules. Source command inventory adds only the tools already invoked
by the selected bodies: `ethtool` and `sysctl`.

The control-plane integration guest reuses the worker guest's already-approved
dependency-ordered defragmentation/reject/tproxy module closure for composed
mTLS and restart nft paths. It adds no new module kind. Read-only Metal audit
`382b46beb8c157f6986e6ba0` found the two tools and ethtool's shared-library
closure. Existing module-file audit `afb0538bee7e6b0e28a2f6df` covers the
staged closure.

The DNS bodies are routed because their actual entry points bind/audit real
`:53` sockets or boot the production composition through a real kernel mTLS
probe; the two failures were real missing-privilege/substrate failures, not
ordinary logic regressions. Adopt-on-restart, veth/XDP, offload and workload-
netns families likewise preserve their real kernel entry points and authored
oracles. No no-op adapter is used in these native scenarios.

## BPF artifact, tools, modules and private filesystems

`cargo xtask guest-kit-prepare` still runs the native BPF build before staging
the control-plane, dataplane integration and dataplane library libtests. The
runner stages the exact workspace-scoped artifact at the identical path those
file-loading tests were compiled to open:

```text
/home/ubuntu/overdrive/target/bpf/overdrive_bpf.o
```

Missing source artifacts fail image construction; guest setup proves the path
readable. `include_bytes!` users retain their embedded compile-time object.
No new BPF program, production loader, hook or path is introduced.

The dataplane guest retains the iteration-3 exact tools, matching Python
stdlib, private cgroup-v2/bpffs mounts, netns alias and dependency-ordered
module closure. The dataplane library phase reuses that exact substrate. The
control-plane guest uses only its selected tools and the closure needed by its
real production-composition paths.

All module loads, BPF map/program operations, XDP attachments, nftables
objects, `:53` sockets, cgroups, routes, veths, network namespaces, captures
and kTLS sockets occur after Cloud Hypervisor boot through the existing guest
peer or staged libtest. The physical Metal host only prepares artifacts,
reads tool/runtime/module bytes, builds images and launches confined VMMs.

## Complete 122-identity overlap ledger

The prior native-covered ledger contained 99 three-package identities: 56
control-plane and 43 dataplane. Iteration 4 adds exactly 20 control-plane and
three dataplane-library identities with no overlap:

| Package | Prior native-covered | Additional | Complete native-covered |
| --- | ---: | ---: | ---: |
| Control-plane | 56 | 20 | 76 |
| Dataplane | 43 | 3 | 46 |
| Netlink | 0 | 0 | 0 |
| **Total** | **99** | **23** | **122** |

The 22 worker-only bodies in `cgroup-default` are correctly absent from this
three-package exclusion count. They remain separately required by the 98-body
native group but were never listed by the normal three-package inventory.

Only passing 46-body and 98-body receipts authorize the selector to exclude
the exact 122 identities. Every remaining ordinary userspace/Sim identity
must execute in the unprivileged normal run. The selector must preserve the
list digest, exact exclusion identities and selected total; any omission,
duplicate, unknown exclusion or still-selected primary runtime privilege
bailout fails the gate.

The expected clean accounting is mechanically derived from the unchanged
1,355 listed identities:

| Final normal result | Expected |
| --- | ---: |
| Native-covered exclusions | 122 |
| Existing ignored/other approved skips | 9 |
| Total skipped/excluded | 131 |
| Selected/run | 1,224 |
| Passed | 1,224 |
| Failed | 0 |
| Exit status | 0 |

`1,224 + 131 = 1,355`. The first-run transition also reconciles:
`1,247 - 23 = 1,224`, and `1,240 - 18` false-positive bailout passes + 2
corrected ordinary failures = 1,224 genuine passes.

Compilation, focused fixture tests, the old dataplane-omitting causal run, the
failed 1,247-body classification run, and either native group remain invalid
substitutes for this clean actual three-package gate.

## Confinement and physical-host boundary

The added staged binary, tools and modules do not change Cloud Hypervisor's
execution identity or grants. The VMM retains uid/gid 4200, actual KVM and
vhost-vsock owner supplementary groups, root-owned parent traversal through
gid 4200 mode `0710`, per-allocation ownership, Landlock, launch seccomp,
`no_new_privs`, device modes, lifetime and RAII cleanup.

No source-review action or runner extension loads a physical-host module,
mounts a physical-host cgroup/bpffs, binds physical-host `:53`, creates a host
netns/veth, attaches host BPF/XDP, changes a host sysctl, or writes host nft/
route/rule state. Every such effect remains inside a disposable guest and is
destroyed with it.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Classification evidence | PASS | Actual first normal run provides exact failures and 18 explicit capability bailouts; no hypothetical family was routed. |
| Fixture purpose | PASS | Restart fixture retains driver-spec oracle; redb fixture now establishes its declared read-only premise. |
| Production boundary | PASS | Both corrected fixtures still use real production entry points; only an out-of-scope existing driven port is test-doubled. |
| Assertion preservation | PASS | Driver-spec and redb typed-failure assertions remain; no body is weakened to pass. |
| All-binary guard audit | PASS | Control-plane, dataplane and netlink binary kinds reconciled against runtime output and source guards. |
| Exact native manifests | PASS | Additional 20 and 3 arrays exactly equal the evidence-derived identity sets, with no duplicate/missing/extra name. |
| Existing group reuse | PASS | Same `mtls-relay` and `cgroup-default` groups, guests, CIDs and scripted executor. |
| Runtime skip honesty | PASS | Required native invocations reject uppercase and lowercase capability bailout forms. |
| Native accounting | PASS | Independent 7/36/43/3/46 and 55/18/5/78/20/98 gates. |
| Ordinary normal coverage | PASS in plan | Exact 122-overlap ledger leaves all remaining identities selected; expected 1,224/1,224 clean rerun. |
| Product scope | PASS | No product API, CI/selectable lane, scenario, model, production hook or mechanism. |
| Physical-host safety | PASS | Kernel and privileged effects remain confined to disposable guests. |

The two fixture edits repair existing preconditions without adding acceptance
behavior. The 23 native identities are pre-existing scenarios. Contract Shape,
C1-C7 taxonomy and scenario completeness therefore remain unchanged. This
iteration closes all-binary execution-boundary completeness, so delta
completeness remains 15/15 under the canonical checklist's N/A rule.

## Iteration 4 mechanical verification

| Check | Result |
| --- | --- |
| Supplied five source hashes | PASS — exact requested runner, staging, two fixture and handoff identities. |
| First actual normal receipt hash | PASS — `d11a288b8e76bb9cecc68f58a0b9a638b04dbeef3c812440f1ab2405996a3b23`. |
| First actual normal accounting | PASS as classification — 1,247 run, 1,240 pass, 7 fail, 108 skip. |
| Runtime privilege bailouts | PASS — exact 18 unique diagnostics. |
| Native failures after ordinary-fixture split | PASS — exact 5. |
| Ordinary fixture defects | PASS — exact 2, retained in normal boundary. |
| Focused corrected fixtures | PASS as reported by Metal `c72471e83359a3f4b7d38b01`, 2/2 unprivileged. |
| Action-shim fixture | PASS — private existing-port adapter only; production dispatch/spec/driver oracle retained. |
| Redb fixture | PASS — closed prior handle, file+parent permissions, original-permission restoration, typed/open refusal retained. |
| Control-plane additional manifest | PASS — 20 rows / 20 unique / zero missing or extra against runtime-derived set. |
| Dataplane-library additional manifest | PASS — 3 rows / 3 unique / zero missing or extra against runtime-derived set. |
| Prior integration-native manifest | PASS — unchanged exact 36. |
| Complete overlap ledger | PASS — 76 control-plane + 46 dataplane = 122; no netlink exclusion. |
| Worker-only bodies | PASS — remain in native group accounting and outside the three-package overlap ledger. |
| `mtls-relay` source totals | PASS — exact 7 + 36 = 43, then 3, complete 46. |
| `cgroup-default` source totals | PASS — exact 55 + 18 + 5 = 78, then 20, complete 98. |
| Existing selectable groups | PASS — unchanged eight. |
| Existing guest names / CID slots | PASS — unchanged. |
| New CI/selectable lane or executor | NONE — one existing library target staged through existing GuestKit preparation. |
| Tool closure | PASS in source; control-plane additions limited to ethtool/sysctl, dataplane closure unchanged. |
| Module closure | PASS in source; dependency order and guest-only load retained. |
| BPF artifact | PASS — mandatory preparation, exact workspace path and guest read-back retained. |
| cgroup-v2 / bpffs / netns aliases | PASS — private guest setup. |
| Runtime skip rejection | PASS — uppercase `SKIP ` plus `[skip]`, `skip:` and `(skip)` covered for control-plane; dataplane forms retained. |
| Final expected normal arithmetic | PASS — 1,224 pass + 131 skip/excluded = 1,355; zero fail, exit zero required. |
| Runner/xtask format and Clippy | PASS as reported by durable Metal `dade15f1bf48082997381fec`; `cargo fmt --check` independently rerun. |
| GuestKit staging unit tests | PASS as reported by Metal `3251ba18a4ece881cf4200bb`, 2/2. |
| `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Actual native 46-body replay | NOT RUN — required next. |
| Actual native 98-body replay | NOT RUN — required next. |
| Clean actual normal 1,224-body gate | NOT RUN — required after both native receipts. |
| New production/API/scenario/model/DES/commit/mutation work | NONE. |

## Iteration 4 verdict

**APPROVED.** The first actual normal run has been classified without hiding
ordinary defects or counting privilege bailouts as passes. The two ordinary
fixtures now reach their existing production-path oracles: the action-shim
scenario injects a private no-op only at its explicitly out-of-scope existing
network provisioner port, and the redb scenario makes the already-materialized
database file plus its parent genuinely read-only after closing the writable
handle. Both remain in the normal selector and pass unprivileged.

The remaining 23 identities are proven native: exact 20 control-plane
integration and three dataplane library bodies. They are routed through the
existing groups with no new scenario, body, selectable lane, executor or
product API. The historical 78 and 43 boundaries remain separately counted;
the new suffixes extend them to 98 and 46 with runtime skip diagnostics
rejected.

The complete overlap ledger now contains exactly 122 three-package native
identities. Worker-only native bodies remain separately accounted and do not
inflate that ledger. All remaining normal identities must execute in the
clean unprivileged gate, whose expected evidence is exactly 1,224 passed,
zero failed, 131 skipped/excluded and exit zero. No native or clean normal
result is claimed by this source approval.

The original STEP 13-05 crafter may now run and retain the exact 46-body
`mtls-relay` and 98-body `cgroup-default` receipts, materialize the exact
122-identity selector, and rerun the actual control-plane/dataplane/netlink
normal gate. Any runtime bailout, identity mismatch, omitted ordinary case,
unaccounted exclusion or non-reconciling total reopens this review.

# APPROVED — ITERATION 4

---

# Review iteration 5 — `nft_nat` dependency closure

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 5 |
| Review date | 2026-10-28 |
| Scope | One proven GuestKit module dependency discovered by the first actual 46-body `mtls-relay` attempt |
| Trigger | Durable Metal `832e3a551f09ac4d4fd08724`, exit 101: first routed-native body reached its existing DNAT topology and failed because the pinned kernel's `nft_nat` expression module was absent |
| Prior disposition | Iterations 1-4 remain approved: exact manifests, two ordinary fixture corrections, 46/98 native accounting, complete 122-identity overlap ledger, normal-gate plan, confinement and all scenario oracles |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews only the added `net/netfilter/nft_nat` module in the
existing dataplane-native guest dependency closure. It is a reproduced
fixture dependency, not a production defect or a new scenario requirement.

Only this review artifact was appended. No production source, test body,
assertion, marker, staging manifest, API, lane, selectable group, scenario,
model, DES event, execution log or commit was changed by the reviewer. The
46-body/98-body native gates and the clean normal rerun have not been retried.

## Iteration 5 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `41c0941890a6f313318efd75d376ad5b399a857d88c12ff93663b018bab2a968` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `871873d92dd79750819b25235671c20a6e9a189f9c0166c2d2df66de19ef2cf5` |
| Failed native raw receipt | `2c00a7110c162a2efd37d0aabffcc2f148351058fe93935177744e7faeb0d49c` |
| Iteration 4 review before this append | `b8e2b260cef24f6f7665abcf0fe623231d6d17c36a99cfebc20d478f18fd1285` |

The runner's scenario manifests, binary staging, tools, Python runtime, BPF
artifact, guest names, CID slots, groups, phase order and count constants are
unchanged from iteration 4.

## Reproduced failure classification

The retained raw receipt shows that the original seven-body relay prefix had
already returned successfully: runner control flow reaches the first exact
routed-native invocation only after its required seven-pass gate. The first
native identity was
`integration::mtls_agent_handshake::agent_handshake_presents_held_svid_server_role`.
It failed before its authored handshake assertion while installing the
unchanged test topology:

```text
meta mark 2 ip daddr 127.0.0.2 tcp dport 18443 dnat to 10.66.0.2:18443
                                               ^^^^
Error: Could not process rule: No such file or directory
```

The exact source path is the existing
`tests/integration/helpers/mtls_netns_topology.rs` setup used by the selected
mTLS native families. The error occurs while nft resolves the `dnat`
expression. The production enforcement, handshake, kTLS and leg-S assertions
have not yet executed, so changing their logic or oracle would be unrelated to
the observed failure.

This is a concrete disposable-guest fixture failure through the real selected
entry point. It satisfies the repository's reproduce-before-fix rule without
inventing a hypothetical dependency.

## Pinned-kernel dependency proof

Read-only Metal run `9f6fc1d6358b26463f1062d0` examined the exact running
kernel `7.0.0-38-generic` and found:

- `CONFIG_NFT_NAT=m`;
- module file
  `/lib/modules/7.0.0-38-generic/kernel/net/netfilter/nft_nat.ko.zst`;
- module alias `nft-expr-nat`; and
- dependencies `nf_tables,nf_conntrack,nf_nat`.

Linux v7.0 source registers the `nat` expression in
`net/netfilter/nft_nat.c` through `MODULE_ALIAS_NFT_EXPR("nat")`. Its DNAT
path maps `NFT_NAT_DNAT` to destination manipulation. The observed `ENOENT` at
the `dnat` token is therefore the exact failure expected when this modular
expression implementation is absent from the minimal guest.

The already-approved module order had loaded `nf_tables`, `nf_conntrack` and
`nf_nat`. Adding `nft_nat` after those three satisfies every reported
dependency. It remains before `nft_chain_nat` and `nft_masq`, the later NAT
chain and masquerade consumers.

## Selected-expression scope audit

Read-only source audit `f188bef22d6c300cdecd73eb` checked the complete
selected dataplane-native source set. Its nft expressions are limited to:

- TPROXY in prerouting;
- DNAT in output; and
- masquerade in postrouting.

The prior closure already included `nft_tproxy`, `nft_chain_nat`, `nft_masq`,
`nf_nat` and their other dependencies. No selected body installs nft
`redirect`/`redir` or explicit SNAT. “Redirect” and “reverse NAT” in other
selected family names refer to BPF/XDP behavior rather than nft expressions.

Accordingly the remediation stages and loads only `nft_nat`. It does not bulk
copy an unselected module family and does not add `nft_redir`, protocol helpers
or another speculative netfilter module.

## Exact runner delta and order

The bounded source change is:

1. one private guest-path constant,
   `/lib/overdrive-testing/modules/nft_nat.ko`;
2. one `DATAPLANE_NAT_MODULES` source/path row for
   `net/netfilter/nft_nat`; and
3. one existing `module_specs` entry after the already-loaded `nf_nat` and
   before `nft_chain_nat`/`nft_masq`.

`host_module_file` reads or decompresses the pinned host module into the guest
image staging tree. The existing scripted guest peer performs the actual
module load after Cloud Hypervisor boot. No physical-host `modprobe`,
`finit_module` or module-state mutation is introduced.

The execution sequence remains:

1. original seven relay bodies with TLS as the only loaded extra module;
2. load the approved dataplane-native closure, now including `nft_nat`;
3. run the exact 36 integration-native bodies and require 43 retained total;
4. run the exact three library-native bodies and require 46 complete.

The module cannot make the original seven pass because it is loaded only
after their exact required gate returns. It does not alter any body, identity,
timeout, expected summary or runtime-skip policy.

## Preserved accounting and boundaries

| Boundary | Preserved result requirement |
| --- | --- |
| `mtls-relay` baseline | exactly 7 first |
| Dataplane integration-native | exactly 36 |
| Retained relay subtotal | exactly 43 |
| Dataplane library-native | exactly 3 |
| `mtls-relay` complete | exactly 46 |
| `cgroup-default` retained | exactly 55 + 18 + 5 = 78 |
| Control-plane native suffix | exactly 20 |
| `cgroup-default` complete | exactly 98 |
| Three-package native overlap ledger | exactly 122 identities |
| Clean normal gate | exactly 1,224 passed, zero failed, 131 skipped/excluded, exit zero |

The 36 integration names, three library names, 20 control-plane names and both
ordinary fixture corrections are unchanged. No new normal exclusion is
created. The failed `832e3a551f09ac4d4fd08724` run remains causal evidence
only and cannot satisfy the 46-body gate.

All existing runtime capability-bailout scans remain active. `nft_nat`
availability must allow the authored topology and oracle to run; it is not
accepted as a substitute pass or skip.

## Confinement and physical-host boundary

The additional module file follows the exact existing GuestKit path:
physical Metal reads/decompresses the pinned file for image construction, the
confined Cloud Hypervisor guest receives it, and the scripted peer loads it
inside that guest. The file and loaded module die with the disposable guest.

Cloud Hypervisor retains uid/gid 4200, actual KVM/vhost-vsock supplementary
groups, gid-4200 `0710` parent traversal, per-allocation ownership, Landlock,
seccomp, `no_new_privs`, device modes, lifetime and RAII cleanup. No host BPF,
nft, netns, cgroup, route, rule, sysctl, XDP or module operation is added.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Reproduction | PASS | Actual first native body failed at unchanged DNAT setup with exact ENOENT. |
| Causal dependency | PASS | Pinned kernel maps the `dnat` expression to modular `nft_nat` and reports exact dependencies. |
| Minimal closure | PASS | One module only; no redirect/SNAT or bulk module staging. |
| Dependency order | PASS | `nf_tables`/`nf_conntrack`/`nf_nat` precede `nft_nat`; chain/masquerade consumers follow. |
| Scenario honesty | PASS | No body/assertion/oracle change; the real topology must now reach its original handshake assertions. |
| Baseline preservation | PASS | Seven relay bodies still execute before the additional module load. |
| Native accounting | PASS | 7/36/43/3/46 and 55/18/5/78/20/98 unchanged. |
| Normal overlap | PASS | Exact 122 identities and 1,224-body normal requirement unchanged. |
| Product scope | PASS | Test-support module staging only; no production API/hook, lane, group, scenario or model. |
| Physical-host safety | PASS | Module is loaded only by the guest peer after disposable-guest boot. |

The dependency correction adds no acceptance behavior, input class, state,
mode, error or concurrency claim. Contract Shape, C1-C7 taxonomy and scenario
completeness remain unchanged. Delta completeness remains 15/15 under the
canonical checklist's N/A rule.

## Iteration 5 mechanical verification

| Check | Result |
| --- | --- |
| Supplied runner/handoff hashes | PASS — exact requested identities. |
| Failed native receipt hash | PASS — `2c00a7110c162a2efd37d0aabffcc2f148351058fe93935177744e7faeb0d49c`. |
| Failed native exit | PASS as classification — exit 101, first routed-native body, no clean-gate claim. |
| Original seven phase | PASS by runner reachability — first native invocation occurs only after exact seven-pass gate. |
| Observed failure | PASS — unchanged DNAT rule, `No such file or directory`, before authored oracle. |
| Pinned module file / config / alias | PASS as reported by read-only Metal `9f6fc1d6358b26463f1062d0`. |
| Exact dependencies | PASS — `nf_tables,nf_conntrack,nf_nat`, all loaded first. |
| Selected nft expression inventory | PASS as reported by read-only Metal `f188bef22d6c300cdecd73eb`. |
| Added module rows | PASS — exact one `nft_nat` constant/source/path/load entry. |
| Load order | PASS — after `nf_nat`, before `nft_chain_nat` and `nft_masq`. |
| Unrelated modules | PASS — no `nft_redir`, explicit-SNAT helper or bulk staging. |
| Test bodies/assertions/identities | PASS — unchanged. |
| Groups/guests/CIDs/executor | PASS — unchanged. |
| `mtls-relay` totals | PASS in source — exact 7 + 36 = 43 + 3 = 46. |
| `cgroup-default` totals | PASS in source — unchanged exact 78 + 20 = 98. |
| Runtime-skip rejection | PASS — unchanged. |
| Complete native overlap ledger | PASS — unchanged exact 122. |
| Runner formatting / Clippy | PASS as reported by durable Metal `05a6cdb0e1b89d16c2428894`; `cargo fmt --check` independently rerun. |
| Tracked support diff | PASS as reported by `d4ee7c2450fdd0ba8f366327`. |
| Handoff whitespace | PASS as reported by `71a4b87f84e91d83e65c31a4`. |
| `git diff --check` | PASS — independently rerun. |
| Native 46-body retry | NOT RUN — required next. |
| Native 98-body gate | NOT RUN — required next. |
| Clean normal three-package gate | NOT RUN — required after both native gates. |
| Production/staging/body/API/lane/group/scenario/model/DES/commit/mutation changes | NONE beyond the reviewed private runner dependency. |

## Iteration 5 verdict

**APPROVED.** The first actual native replay proved one missing disposable-
guest dependency at the exact DNAT expression. The pinned 7.0 kernel provides
that expression through modular `nft_nat`, whose complete dependencies were
already loaded. The runner now stages and loads only that module in the exact
dependency position. No production path, test body, assertion, scenario,
group, lane, count or physical-host state changes.

The original seven relay bodies remain first under their TLS-only loaded-
module state. The 36 integration-native and three library-native bodies remain
separately gated at 43 and 46; the 98-body cgroup gate and 122-identity normal
ledger remain unchanged. The failed receipt is retained only as causal
classification evidence.

The original STEP 13-05 crafter may now rerun the exact 46-body
`mtls-relay` gate, then the 98-body `cgroup-default` gate, and only after both
pass rerun the clean actual three-package normal gate. Any further dependency
failure must be reproduced and bounded independently; it must not broaden the
module closure or weaken an authored oracle without another review.

# APPROVED — ITERATION 5

---

# Review iteration 6 — fail-closed three-interface TX-offload fixture

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 6 |
| Review date | 2026-10-28 |
| Scope | Evidence-confirmed GuestKit `ethtool` dependency, fail-closed four-end TX-checksum-offload fixture precondition, diagnostic-only S-05-C evidence, and bounded exact replay |
| Trigger | Durable native attempt `fe0c4da54d92be8295bd350f`: S-05-C had all reverse-map keys but all three unchanged clients were quiet after the guest silently discarded four absent-`ethtool` commands |
| Prior disposition | Iterations 1-5 remain approved: exact 46/98 manifests and counts, `nft_nat`, two ordinary fixture corrections, complete 122-identity overlap ledger, normal-gate plan, confinement and scenario oracles |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews a reproduced test-fixture precondition. It does not
propose a production BPF change or reinterpret the S-05-C contract. The exact
existing S-05-C body, inputs, production dataplane path, reverse-map
assertions and three traffic outcomes remain the acceptance oracle.

Only this review artifact was appended. No production source, public API,
error variant, test identity, pending marker, scenario, model, CI/selectable
lane, selectable runner group, guest executor, DES event, execution log or
commit was changed by the reviewer. The complete 46-body, 98-body and clean
normal gates have not rerun.

## Iteration 6 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `d70ffae99efb522ca7514547efd354f53bdd4fe3b5671af494014ee113443e1a` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `fd8eae9b446b505aadfc4810069280df2e6a79925a50e3f30534549846a3c723` |
| `overdrive-testing/src/netns.rs` | `89c1afc9ebee95fb335e082f063fdf111d61a0e51df3edea3666dc0d4df93362` |
| `multi_listener_tcp_udp_e2e.rs` | `e7de24992a09067f6989510927c142482533ec8cf465dc89dc14b6667af47707` |
| Bounded harness `increment-a/src/main.rs` | `733bafc412e7b2322b08e45d191a8f1402743960d792e372c8b361bf81034616` |
| Failed second native receipt | `76cf5eb858ad7b60db932ba2169f8fb551d8777f8540b9e8ed166c0fb1f65743` |
| Bounded exact replay receipt | `99c07ea3b978d90025f0fc4fb379c6dc3b8ba8570c579a86aa2b4e2791545c82` |
| Iteration 5 review before this append | `1983659e35765e287c99442af605b83e4610a9db8ff7c1013d7f1e27327c5539` |

The runner's native identity arrays, count constants, binary staging,
module closure, guest names, CID slots, selectable groups and overlap ledger
are unchanged from iteration 5.

## Second native replay classification

After the approved `nft_nat` repair, durable Metal
`fe0c4da54d92be8295bd350f` reached the existing S-05-C body
`adding_listener_on_resubmit_converges_without_breaking_existing`. It exited
101 with all three original clients quiet:

- UDP/VIP:8082 returned no marker;
- UDP/VIP:8081 returned no marker; and
- TCP/VIP:8080 returned no marker.

The reverse-NAT keys already existed, so the failure was downstream of map
convergence. The first authored assertion reported UDP/8082 because it is the
first traffic assertion; the retained diagnostic showed the failure affected
the added listener and both preserved listeners.

`ThreeIfaceTopology::create` already documented a load-bearing prerequisite:
incremental XDP L4 checksum updates require a complete checksum at ingress,
so TX checksum offload must be disabled on all four veth ends. Its source
issued four `ethtool -K <iface> tx off` commands through the namespace helper
but discarded every `Result`. The dataplane GuestKit image did not stage
`ethtool`, making every command fail before it could change the offload state.

This is a reproduced fixture cause:

- the required tool was absent from the disposable image;
- every result that would have reported absence was explicitly discarded;
- Linux v7.0 veth exposes TX checksum offload by default;
- the BPF programs perform incremental checksum adjustment; and
- all three traffic paths were silent despite converged reverse-map keys.

No production BPF premise was falsified. The test never established the
precondition under which its authored traffic oracle is meaningful.

## Bounded fixture correction

The executable correction is limited to the existing test-support boundary:

1. add `ethtool` to the existing dataplane-native GuestKit tool payload;
2. keep the existing `NetNs::ethtool_tx_off` command and return type;
3. replace the four discarded calls in `ThreeIfaceTopology::create` with a
   private `ensure_tx_checksumming_off` helper;
4. propagate the existing `NetNsError` on command failure;
5. read `ethtool -k` back inside the same namespace;
6. require the aggregate `tx-checksumming: off` line for that exact interface;
   and
7. retain one namespace/interface/readback evidence line for each of the four
   ends.

The four required ends are:

- client namespace `client_veth`;
- LB namespace `lb_veth_a`;
- LB namespace `lb_veth_b`; and
- backend namespace `backend_veth`.

The helper is private. It adds no public method, port, parameter or error
variant. A successful `ethtool -K` with a non-off aggregate readback now fails
through the existing `NetNsError::IpCommand`; missing permission remains the
existing `CapNetAdminRequired` classification.

The correction applies the precondition declared by the fixture. It does not
change an XDP program, checksum algorithm, packet, frontend, backend, route,
neighbor, listener, map entry or expected traffic marker.

## S-05-C scenario preservation

The existing body still performs the same sequence:

1. construct the same three-namespace/four-veth topology;
2. start the same TCP/8080 and UDP/8081 listeners;
3. install the same production service-map and reverse-NAT paths;
4. resubmit by adding UDP/8082 through the same `update_service` call;
5. assert all three proto-distinguished reverse-NAT keys;
6. run the unchanged UDP/8082, UDP/8081 and TCP/8080 clients; and
7. require the same three VIP-sourced markers.

The reverse-key assertions and all three traffic assertions are unchanged.
The fixture still loads the real compiled `xdp_service_map` and
`xdp_reverse_nat` programs, plus the pre-existing peer-side `xdp_pass` stubs
required by veth XDP semantics. No replacement BPF object, fake program,
alternate socket path or stubbed traffic oracle is introduced.

## Diagnostic-only evidence

Before the unchanged assertions can unwind and destroy the namespaces,
S-05-C now prints read-only evidence for:

- exact namespace and interface identities;
- link details;
- IPv4 route tables;
- neighbor tables;
- backend TCP/UDP listeners;
- LB BPF attachments; and
- loaded BPF program identities/tags.

The diagnostic helpers call only `ip ... show`, `ss` and `bpftool ... show`.
They do not add routes/neighbors/listeners, mutate maps, attach programs,
change offload state or affect pass/fail. Spawn/read failures are printed as
diagnostics and do not replace the authored assertions. The existing traffic
oracles remain the only scenario verdict.

The additional elapsed diagnostic time does not replace a readiness
precondition: the body retains its existing backend startup delay, and the
receipt separately proves all three listeners were already bound before the
clients. The causal correction is the four fail-closed offload readbacks.

## Exact BPF artifact identity

The standard `guest-kit-prepare` route builds the existing workspace artifact
before staging the dataplane integration binary. Read-only Metal
`db410cb6ebd04a1ac7b54879` recorded the prepared object as:

| Property | Value |
| --- | --- |
| Path | `target/bpf/overdrive_bpf.o` |
| Size | 10,744 bytes |
| SHA-256 | `5aec54ee315fbf451b313b1e1d1faf0de9ff6fdb89b262357b36ae4f76d19e3a` |

The runner stages that exact workspace path at the identical guest path and
now prints its SHA-256 during the existing Python preflight. The bounded
receipt records the loaded production program names/tags:
`xdp_service_map`, `xdp_reverse_nat`, both `xdp_pass` peers and the existing
cgroup socket programs.

The exact prepared artifact and original test identity are therefore the
system under test. The bounded harness does not compile or substitute a probe
BPF program.

## Canonical bounded replay harness

The retained harness at
`spike-scratch/netns-density-295-dataplane-offload/increment-a` uses the
existing public GuestKit image/boot API, scripted peer, prepared dataplane
integration binary, module closure, tools, Python runtime and exact S-05-C
libtest identity. It adds no production hook, crate-local seam, selector,
selectable group, CI lane or executor.

Its only purpose is to run one exact existing body inside the same disposable
boundary while retaining causal evidence before another full 46-body attempt.
The harness requires exit zero, the exact one-pass/zero-fail summary and no
`[skip]` diagnostic. Its source and evidence are retained in the workspace,
not an unrecorded temporary probe.

## Bounded replay result

Durable Metal `6c6a56320ecafbfd509a3609` exited zero. Its canonical raw
receipt proves:

- all four topology ends read back `tx-checksumming: off`;
- the client and backend default routes were present;
- both LB connected routes were present;
- both permanent LB neighbors were present;
- backend UDP/8081, UDP/8082 and TCP/8080 listeners were bound;
- `xdp_service_map` and `xdp_reverse_nat` were attached to the two LB veths;
- `xdp_pass` was loaded on both peer ends;
- the unchanged UDP/8082 client received `ovd-multi-c-udp2`;
- the unchanged UDP/8081 client received `ovd-multi-c-udp1`;
- the unchanged TCP/8080 client received `ovd-multi-c-tcp`;
- libtest reported one passed, zero failed, zero ignored; and
- the harness reported `S05C_BOUNDED_REPLAY_PASS` and durable exit zero.

This proves the fixture precondition and the exact original S-05-C oracle in
the disposable guest. It does not claim the complete 46-body gate and does not
authorize a production BPF change.

## Runner and accounting preservation

The runner delta adds only `ethtool` to `dataplane_native_tools` and an
`ethtool --version` fail-closed preflight. BPF SHA output uses the already-
staged matching Python runtime. No native manifest or counter changes.

| Boundary | Preserved requirement |
| --- | ---: |
| Original relay baseline | 7 first, TLS-only loaded-module state |
| Dataplane integration-native | 36 |
| Retained `mtls-relay` subtotal | 43 |
| Dataplane library-native | 3 |
| Complete `mtls-relay` | 46 |
| Retained `cgroup-default` | 78 |
| Control-plane native suffix | 20 |
| Complete `cgroup-default` | 98 |
| Complete normal/native overlap ledger | 122 |

Runtime capability-skip rejection remains unchanged. The bounded one-body
pass cannot be credited as any portion of the complete 46-body gate.

## Confinement and physical-host boundary

`ethtool` is copied into the existing guest image with its ELF interpreter
and shared-library closure. It runs only through `ip netns exec` after Cloud
Hypervisor boot. All namespaces, veths, offload changes, routes, neighbors,
listeners, BPF attachments and maps exist only in that disposable guest.

Cloud Hypervisor retains uid/gid 4200, KVM/vhost-vsock supplementary groups,
gid-4200 `0710` parent traversal, per-allocation ownership, Landlock, seccomp,
`no_new_privs`, device modes, lifetime and RAII cleanup. No physical-host
ethtool, netns, veth, sysctl, BPF, XDP, nft, route, rule, cgroup or module
operation is introduced.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Reproduction | PASS | Actual native S-05-C reached all map setup but all three clients were quiet with four discarded absent-tool results. |
| Fixture contract | PASS | Existing source already declared TX offload disabled on all four ends as required by incremental checksum adjustment. |
| Minimal correction | PASS | Stage existing `ethtool`; replace discarded results with one private apply/read-back helper. |
| Typed failure | PASS | Existing `NetNsError` variants carry command, permission and readback failures; no new public error. |
| Scenario identity | PASS | Same S-05-C libtest body, service update, map assertions and three traffic oracles. |
| Production fidelity | PASS | Same prepared BPF object and production dataplane; no alternate/stub path blesses the case. |
| Diagnostic scope | PASS | Read-only output before unchanged assertions; no state or verdict mutation. |
| Bounded confirmation | PASS | Exact body passes with four off-readbacks, expected topology/BPF/listeners and all original markers. |
| Native accounting | PASS | 7/36/43/3/46 and 78/20/98 unchanged. |
| Normal overlap | PASS | Exact 122 identities and clean 1,224-body normal requirement unchanged. |
| Product scope | PASS | No production API, BPF remedy, scenario, model, lane, group or executor. |
| Physical-host safety | PASS | Tool and all effects remain inside disposable GuestKit guests. |

The fixture correction and diagnostics add no acceptance behavior, state,
input partition, mode, error taxonomy or concurrency claim. Contract Shape,
C1-C7 taxonomy and scenario completeness remain unchanged. Delta
completeness remains 15/15 under the canonical checklist's N/A rule.

## Iteration 6 mechanical verification

| Check | Result |
| --- | --- |
| Supplied runner/netns/scenario/handoff/spike hashes | PASS — exact requested identities. |
| Second failed native receipt hash | PASS — `76cf5eb858ad7b60db932ba2169f8fb551d8777f8540b9e8ed166c0fb1f65743`. |
| Failed stimulus | PASS — exact S-05-C, all reverse keys installed, all three clients quiet. |
| Missing payload cause | PASS — runner lacked `ethtool`; four prior calls discarded errors. |
| Runner tool delta | PASS — only existing `ethtool` plus version preflight; existing Python prints BPF SHA. |
| Private fail-closed helper | PASS — four apply/read-back calls, aggregate exact `off`, existing `NetNsError`. |
| Public API/error variants | PASS — none added or changed. |
| Existing sysctl/route/neighbour/XDP/listener setup | PASS — unchanged. |
| Scenario stimuli/assertions | PASS — reverse-key and UDP2/UDP1/TCP marker assertions unchanged. |
| Diagnostic commands | PASS — read-only links/routes/neighbors/listeners/BPF identity observations. |
| Prepared BPF identity | PASS — 10,744 bytes, SHA `5aec54ee315fbf451b313b1e1d1faf0de9ff6fdb89b262357b36ae4f76d19e3a`. |
| Bounded harness boundary | PASS — existing GuestKit API/peer/prepared binary/exact identity; no selector/group/lane/executor. |
| Bounded receipt hash | PASS — `99c07ea3b978d90025f0fc4fb379c6dc3b8ba8570c579a86aa2b4e2791545c82`. |
| Four TX-off readbacks | PASS — client, both LB ends and backend. |
| Routes / neighbors / listeners | PASS — expected route topology, permanent LB neighbors and 8080/8081/8082 listeners. |
| BPF attachment identity | PASS — service-map, reverse-NAT and both peer `xdp_pass` programs present. |
| Original traffic markers | PASS — UDP2, UDP1 and TCP markers returned. |
| Exact libtest result | PASS — 1 passed, 0 failed, 0 ignored, durable exit zero. |
| Source formatting / Clippy | PASS as reported by durable Metal `6c6a56320ecafbfd509a3609`; `cargo fmt --check` independently rerun. |
| `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Full native 46-body replay | NOT RUN — required next. |
| Full native 98-body replay | NOT RUN — required next. |
| Clean normal 1,224-body gate | NOT RUN — required after both native gates. |
| New production/API/scenario/model/lane/group/DES/commit/mutation work | NONE. |

## Iteration 6 verdict

**APPROVED.** The second native attempt exposed a false fixture precondition,
not a production BPF defect: the existing topology required four TX-offload
operations, the guest lacked `ethtool`, and the fixture discarded all four
errors. The correction stages only that existing tool and fail-closes each
apply/read-back through the existing error type.

The retained bounded replay runs the exact compiled S-05-C body over the exact
prepared BPF object and original production dataplane path. It proves four
offload readbacks, the intended routes/neighbors/listeners/program identities,
all three unchanged traffic markers, and one honest pass. The added diagnostics
are read-only and do not replace or relax an assertion.

All native manifests, group boundaries, counts, runtime-skip policy,
confinement and the 122-identity normal ledger remain unchanged. The bounded
pass is causal confirmation only and does not satisfy the complete native or
normal gates.

The original STEP 13-05 crafter may now rerun the full 46-body `mtls-relay`
gate, then the 98-body `cgroup-default` gate, and only after both pass execute
the clean actual three-package normal gate. Any further failure must be
classified through its real existing path without substituting a different
BPF object, weakening an oracle or expanding production scope.

# APPROVED — ITERATION 6

---

# Review iteration 7 — guest-local tcpdump account closure

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 7 |
| Review date | 2026-10-28 |
| Scope | Evidence-confirmed runtime identity dependency of the already-staged Ubuntu tcpdump binary, minimal guest-local account files, exact UDP-capture replay, and final transitive executable audit |
| Trigger | Third full native attempt `b823070869ab5ea5ec879e01`: exact reverse-NAT UDP capture case failed its unchanged readiness gate because tcpdump exited before emitting `listening on` |
| Prior disposition | Iterations 1-6 remain approved: exact manifests/counts, `nft_nat`, fail-closed TX-offload fixture, two ordinary corrections, 122-identity ledger, normal-gate plan, confinement and all packet oracles |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews one transitive runtime dependency of a staged test tool.
The selected capture body, tcpdump command line, readiness gate, packet
witnesses and VIP-source assertions are unchanged. The correction does not
grant tcpdump more privilege; it enables the package's compiled default
privilege drop inside the minimal disposable guest.

Only this review artifact was appended. No production source, test body,
assertion, marker, packet oracle, public API, lane, selectable group, guest
executor, scenario, model, DES event, execution log or commit was changed by
the reviewer. The complete 46-body/98-body native gates and clean normal gate
remain unpassed.

## Iteration 7 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `9a82741120f5b56bc7e5c3160fdeb2f0d61173949f4311e009ca81e02f728d98` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `e0f9d25843605ec71165c1068ccdd3eec70090260abb39c8c645e05a9078a5cb` |
| Canonical increment-B harness | `4a3b0d60e3c50c823bfc9a0e700510858c7436a6509c02b5d1ec201c4e6b13b0` |
| Third full native failure receipt | `00c8b001feee2e26b79e08f14b1eb3465df35a1d30885d85425464dd046df84a` |
| No-account causal receipt | `10cde4d5090066d37d99f011b9a7cf58d36d10c13b21e1ea5335660dd3f972b6` |
| With-account exact replay receipt | `9bb8dc6b24eba7f3d61c2315e810e4c3e1fe8765515e514c290bab390402aabf` |
| Iteration 6 review before this append | `06cbd2540729a424d6a50243753f46470e877d4511ee9b23ea4277be2ddfdaba` |

The 46/98 manifests, BPF artifact, module closure, tool list, Python runtime,
guest names/CIDs, group registry, runtime-skip policy and overlap ledger are
otherwise unchanged.

## Third full native attempt

After iteration 6's approved offload correction, durable Metal
`b823070869ab5ea5ec879e01` reached
`integration::reverse_nat_udp_e2e::every_udp_reply_independently_source_rewritten`.
All four veth ends read back `tx-checksumming: off`, but tcpdump entered and
immediately left promiscuous mode. The body failed at its existing load-bearing
capture readiness precondition:

```text
tcpdump did not report 'listening on' within 5s ... Disconnected
```

The case deliberately refuses to trust capture silence. It did not count an
uncaptured packet outcome as a pass, and no packet-source assertion was
weakened. The full run remains classification evidence only.

## Exact causal diagnostic

The retained increment-B diagnostic used the same GuestKit boundary and the
fixture's exact capture process shape:

```text
/usr/bin/ip netns exec ovd-capdiag /usr/bin/tcpdump \
  -U -n -i ovdcd-peer -w /tmp/ovd-capdiag.pcap udp
```

Durable Metal `9d95728658b949ca36438ff0` proved:

- `/etc/passwd` was absent;
- `/etc/group` was absent;
- the interface entered promiscuous mode;
- tcpdump exited 1 before the readiness window;
- stdout was empty; and
- stderr was exactly `tcpdump: Couldn't find user 'tcpdump'`.

The diagnostic then ran the unchanged exact reverse-NAT UDP body and reproduced
the disconnected readiness channel. This isolates a guest image identity
dependency before the packet oracle, not a routing, BPF or capture-filter
defect.

## Qualified package identity

Read-only Metal audit `fa7b7ae4ef9eab0be777ce21` inspected the exact staged
Ubuntu package and recorded:

- `/usr/bin/tcpdump` version 4.99.6;
- passwd identity
  `tcpdump:x:985:985:tcpdump:/nonexistent:/usr/sbin/nologin`; and
- group identity `tcpdump:x:985:`.

Ubuntu's package is compiled/configured to drop from root to the `tcpdump`
account after opening the capture interface and before reporting readiness.
The executable and its shared-library closure were already present. The
missing account lookup, not a missing binary or permission on the capture
interface, caused the exit.

## Minimal guest-local account files

The runner creates two private staging files in its existing `WorkDir` and
places them at `/etc/passwd` and `/etc/group` only in the `mtls-relay` guest:

```text
/etc/passwd:
root:x:0:0:root:/root:/bin/sh
tcpdump:x:985:985:tcpdump:/nonexistent:/usr/sbin/nologin

/etc/group:
root:x:0:
tcpdump:x:985:
```

The files are minimal and deterministic:

- no host account database is copied;
- no password or shadow credential is introduced;
- the tcpdump home is nonexistent;
- its shell is nologin;
- no supplementary group is granted;
- no device group is reused;
- no chroot or setuid helper is added; and
- tcpdump is not forced to remain root through `-Z root`.

The existing GuestFile installer places the files in the root-owned image
staging tree and copies their ordinary non-executable permissions. They exist
only for that disposable image/work-directory lifetime and are destroyed with
it.

Before any routed-native body, the existing setup now compares both files
byte-for-byte against those exact contents and requires:

```text
uid=985(tcpdump) gid=985(tcpdump) groups=985(tcpdump)
```

Any UID/GID drift, extra account/group entry, missing file or NSS resolution
failure blocks the guest before a native test can count.

## Security-boundary preservation

The test binary still launches tcpdump with its original command line. It
starts with the guest-root permission required to open AF_PACKET, then follows
the package-default drop to uid/gid 985. No capability, ambient set, setuid
bit, extra group or root-retention option is added.

The tcpdump account is an in-guest userspace identity. It does not alter the
Cloud Hypervisor process identity. The VMM remains uid/gid 4200 with only the
actual KVM/vhost-vsock device-owner supplementary groups, the same Landlock
grant, launch seccomp filter, `no_new_privs`, device modes, `0710` parent
traversal and RAII lifetime.

The physical Metal host's account database is unchanged. No host user/group,
capability, tcpdump process, interface mode or capture is created by the runner
extension.

## Exact bounded UDP-capture replay

The retained increment-B harness uses the existing GuestKit public API,
scripted peer, prepared dataplane integration binary, exact module/tool/BPF
payload and exact existing test identity. It adds no selector, selectable
group, CI lane, executor, production hook or substitute capture path.

Durable Metal `ed145fdfdd75dca1dbfe8c7b` exited zero. Before the body, its
direct diagnostic proved the same exact tcpdump argv remained live through
the readiness window and emitted:

```text
tcpdump: listening on ovdcd-peer, link-type EN10MB (Ethernet), snapshot length 262144 bytes
```

The unchanged exact body then proved:

- all four TX-offload readbacks remained `off`;
- the load-bearing capture emitted its own readiness banner;
- all three client queries returned their original markers;
- the capture observed three outbound query witnesses to UDP/5353;
- the capture observed three replies;
- every captured reply source was VIP `10.0.0.1`;
- libtest reported one passed, zero failed, zero ignored; and
- the harness reported `UDP_CAPTURE_BOUNDED_REPLAY_PASS` with durable exit
  zero.

The bounded run uses the original packet readiness and source oracles. It does
not treat process liveness alone as a packet pass and does not substitute a
synthetic pcap or altered capture filter.

## Full transitive dependency audit

After the tcpdump account finding, the author re-audited executable calls
through every selected 46-body and 98-body call graph, including nested
helpers, namespace commands and library binaries. The current closure contains
the already-approved executables, modules, Python stdlib, BPF object,
cgroup-v2/bpffs mounts, netns alias, `ethtool`, and the minimal tcpdump account.

No further selected executable has an omitted dynamic library, runtime account,
module, sysctl, filesystem or command dependency. This audit does not claim
the broad gates passed; it establishes that another omission was not silently
papered over by adding bulk tools/modules or weakening a precondition.

## Preserved runner phases and accounting

The account files are staged in the existing `mtls-relay` image. They do not
load a module or create runtime state used by the original relay bodies. The
seven original bodies still run first under the TLS-only loaded-module state;
the exact 36/43 and 3/46 phases follow unchanged.

| Boundary | Preserved requirement |
| --- | ---: |
| Original relay baseline | 7 first |
| Dataplane integration-native | 36 |
| Retained relay subtotal | 43 |
| Dataplane library-native | 3 |
| Complete `mtls-relay` | 46 |
| Retained `cgroup-default` | 78 |
| Control-plane native suffix | 20 |
| Complete `cgroup-default` | 98 |
| Complete normal/native ledger | 122 |

No test identity, expected summary, runtime-skip pattern or normal exclusion
changes. The bounded exact pass cannot be credited toward the complete 46-body
gate.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Reproduction | PASS | Exact tcpdump argv exits 1 with absent account files and exact missing-user stderr. |
| Package contract | PASS | Qualified tcpdump 4.99.6 expects tcpdump uid/gid 985 after opening capture. |
| Minimal closure | PASS | Two guest-local account files only; executable/library/module/sysctl lists unchanged. |
| Fail-closed identity | PASS | Exact passwd/group bytes and `id tcpdump` output required before native execution. |
| Privilege preservation | PASS | Package-default root-to-985 drop retained; no `-Z root`, capabilities, setuid or extra groups. |
| Capture honesty | PASS | Original readiness, query-witness, reply-count and VIP-source oracles unchanged. |
| Exact bounded confirmation | PASS | Same body/argv/filter returns one honest pass with all packet evidence. |
| Existing groups/accounting | PASS | 7/36/43/3/46, 78/20/98 and 122 ledger unchanged. |
| Product scope | PASS | No production API/hook, scenario, model, lane, group or executor. |
| Physical-host safety | PASS | Account and capture exist only in disposable guest; host account/device/network state unchanged. |

The account closure adds no acceptance behavior, state transition, input
partition, mode, typed error or concurrency claim. Contract Shape, C1-C7
taxonomy and scenario completeness remain unchanged. Delta completeness
remains 15/15 under the canonical checklist's N/A rule.

## Iteration 7 mechanical verification

| Check | Result |
| --- | --- |
| Supplied runner/handoff/harness hashes | PASS — exact requested identities. |
| Third full native failure receipt | PASS as classification — `00c8b001feee2e26b79e08f14b1eb3465df35a1d30885d85425464dd046df84a`. |
| No-account causal receipt | PASS — `10cde4d5090066d37d99f011b9a7cf58d36d10c13b21e1ea5335660dd3f972b6`. |
| No-account filesystem | PASS — `/etc/passwd` and `/etc/group` absent. |
| Exact causal argv | PASS — same `ip netns exec ... tcpdump -U -n -i ... -w ... udp`. |
| Exact causal outcome | PASS — exit 1, readiness channel closed, `Couldn't find user 'tcpdump'`. |
| Qualified host audit | PASS as reported by read-only Metal `fa7b7ae4ef9eab0be777ce21`. |
| Guest passwd/group contents | PASS — minimal exact root + tcpdump 985:985 entries. |
| Guest ownership/lifetime | PASS — root-owned image files within existing WorkDir/disposable guest. |
| Fail-closed identity preflight | PASS — exact bytes plus exact `id tcpdump` output. |
| Root-retention/security weakening | NONE — no `-Z root`, chroot, setuid, capabilities or new group. |
| With-account bounded receipt | PASS — `9bb8dc6b24eba7f3d61c2315e810e4c3e1fe8765515e514c290bab390402aabf`. |
| Capture readiness | PASS — tcpdump `listening on` banner retained. |
| Packet witness count | PASS — three queries and three replies. |
| Reply source | PASS — all three `10.0.0.1`. |
| Original client markers | PASS — all three returned. |
| Exact libtest result | PASS — 1 passed, 0 failed, 0 ignored, durable exit zero. |
| Indirect selected-call-graph audit | PASS in source — no remaining executable/runtime-account omission in 46+98. |
| Tool/module/sysctl lists | PASS — unchanged except prior approved ethtool/nft_nat; account files only. |
| VMM uid/gid/device groups/Landlock/seccomp/`no_new_privs` | PASS — unchanged. |
| Native/normal accounting | PASS in source — exact 46, 98 and 122 ledger unchanged. |
| Formatting / Clippy | PASS as reported by durable Metal `649a44f773b2e77b02004971`; `cargo fmt --check` independently rerun. |
| Final scoped source check | PASS as reported by `760e96fcb437a9473e078cbf`. |
| `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Complete 46-body native replay | NOT PASSED — third run remains classification evidence; clean retry required. |
| Complete 98-body native replay | NOT RUN — required after approval. |
| Clean normal 1,224-body gate | NOT RUN — required after both native gates. |
| New production/API/scenario/model/lane/group/DES/commit/mutation work | NONE. |

## Iteration 7 verdict

**APPROVED.** The third native attempt exposed the final proven transitive
fixture dependency: Ubuntu tcpdump was already staged and opened the capture
interface, then exited because the minimal guest had no tcpdump account for
its compiled default privilege drop. The correction supplies only exact
guest-local root/tcpdump passwd and group entries, verifies them byte-for-byte
and by numeric identity, and preserves the drop to uid/gid 985.

The retained exact replay uses the unchanged capture argv and packet oracle.
It proves readiness, three query witnesses, three replies, VIP source on every
reply and all original client markers. No root-retention flag, capability,
alternate capture, synthetic packet, body rewrite or assertion weakening is
involved.

All manifests, tools/modules/sysctls, group boundaries, runtime-skip policy,
confinement, 46/98 totals and 122-identity normal ledger remain unchanged. The
bounded pass is causal confirmation only, not a complete native result.

The original STEP 13-05 crafter may now rerun the full 46-body `mtls-relay`
gate, then the 98-body `cgroup-default` gate, and only after both pass execute
the clean actual three-package normal gate. Any further failure must be
classified from its real selected path without adding bulk identity data,
retaining root privilege or weakening the capture/packet oracle.

# APPROVED — ITERATION 7

---

# Review iteration 8 — control-plane guest resolver target and `setsid` closure

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 8 |
| Review date | 2026-10-28 |
| Scope | One reproduced control-plane GuestKit command-chain closure: qualified `setsid`, existing `/bin/sleep`, and an empty ordinary root `/etc/resolv.conf` bind target |
| Trigger | First actual 98-body attempt `3e3b3463786963e459aadc2a`: first 20-body suffix identity reached real netns provisioning but `ip netns exec` lacked the resolver bind target and `setsid`; runtime `SKIP` was correctly rejected |
| Prior disposition | Iterations 1-7 remain approved; the complete dataplane 46-body native gate has now independently passed and is retained without rerun |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews only the reproduced control-plane guest dependency.
It does not change the adoption scenario, production resolver content,
namespace ownership, slot allocator, orphan collection or cgroup behavior.

Only this review artifact was appended. No production source, test body,
assertion, marker, public API, lane, selectable group, guest executor, model,
scenario, DES event, execution log or commit was changed by the reviewer. The
complete 98-body gate and clean normal gate remain pending.

## Iteration 8 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `19bd791fb982a7ebf9148b297c2f00c85909e3f5cb93cdfe3ff12fdf58e79a05` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `c699f4b336d9c642fe2493a08007d9629d49d420dcdf2650ea7d5fc4897048b4` |
| Canonical increment-C harness | `7a811d5edc0209fe4e3a298e4c9787ffec0772241269254706430d185275b9ab` |
| Failed 98-body attempt receipt | `35f737fd9a985ebd8968ca0bc21f1533f421d52c9b5be809e329780543f7ef23` |
| Bounded exact adoption receipt | `202242ae3ddbb958cc9a612662adbe9b552c892c7850bea2df4c7ebd3a475d53` |
| Passing 46-body dataplane receipt | `abd817092d3554facc960601d8fcefb4a052bd49eaf03f1a77204108220bd494` |
| Iteration 7 review before this append | `9108d2b601ab7d69b68454049cc5b43970a80c58768654049ad026b57a0f1844` |

All native manifest identities, count constants, groups, CID slots, dataplane
payload, BPF identity, tcpdump account, module closure and 122-identity normal
ledger are unchanged.

## Actual 46-body dataplane gate

Durable Metal `8066a14e0a5afc0f0c4919fd` completed successfully before
this control-plane-only correction. Independent receipt parsing finds:

- one exact seven-pass baseline summary first;
- 36 exact one-pass dataplane-integration summaries;
- three exact one-pass dataplane-library summaries;
- zero failed summaries;
- zero ignored bodies;
- zero runtime capability-skip diagnostics; and
- durable exit zero.

The receipt also retains the dataplane preflight, BPF SHA and tcpdump numeric
identity. This is the required complete 7 + 36 = 43 + 3 = 46 gate, not a
bounded substitute. Iteration 8 changes only the conditional control-plane
guest payload, so the passing 46-body receipt remains current and must not be
rerun.

## First 98-body attempt classification

Durable Metal `3e3b3463786963e459aadc2a` exited 101. The runner reached
the first exact additional control-plane identity,
`serve_restart_readopts_surviving_slot_and_gcs_orphan_netns`, only after the
control-plane integration guest's retained baseline/STEP 13-04/STEP 13-05
subcounts returned successfully.

The exact subprocess chain in the existing body is:

```text
ip netns exec <namespace> setsid sleep 3600
```

The raw receipt records both missing guest prerequisites:

```text
Bind /etc/netns/ovd-ns-0030/resolv.conf -> /etc/resolv.conf failed: No such file or directory
exec of "setsid" failed: No such file or directory
```

The test then printed its existing `SKIP ... survivor PID ... did not land`
diagnostic and libtest returned one pass. The runner's required-suffix skip
policy rejected that result and failed the group. No survivor-adoption,
collision or orphan-GC assertion was credited.

This is an honest fixture failure before the acceptance oracle, not a
production adoption defect.

## Exact command-chain audit

Source inspection proves the chain has no hidden shell or alternate command:

- `ip` enters the already-provisioned namespace;
- iproute2 performs the standard `/etc/netns/<namespace>/resolv.conf` bind;
- `setsid` establishes the crash-survivor session;
- `sleep 3600` is the survivor process; and
- the existing guest already stages that executable at `/bin/sleep`.

Production `provision_workload_netns` already creates
`/etc/netns/<namespace>/resolv.conf` with the intended node-local responder.
The missing object is the ordinary root-namespace bind target
`/etc/resolv.conf`, not the production per-namespace content.

The follow-up transitive argv/script audit covered all selected 98 control-
plane/worker and 46 dataplane bodies, including nested helpers. It found no
other missing executable, shell chain, root target, module, account or runtime
file. `/bin/true` is not required; the retained `/bin/sleep` and existing
`/usr/bin/sh` uses remain exactly accounted.

## Minimal conditional guest closure

Only the control-plane integration guest with
`stage_normal_gate_native = true` receives the new support:

1. qualified host `setsid` staged at `/usr/bin/setsid` with its normal ELF
   interpreter and shared-library closure;
2. a generated zero-byte ordinary file staged at `/etc/resolv.conf`; and
3. a fail-closed setup that requires the target to be a regular file, not a
   symlink, exactly empty, and proves `/usr/bin/setsid /bin/sleep 0` exits
   zero.

No host resolver file is copied. No DNS server, nameserver line, search domain
or production resolver content is injected. The empty root file exists only
as the mount target over which `ip netns exec` places the already-authored
namespace-specific file. Inside the workload namespace, the production
`nameserver <responder>` content remains authoritative.

The generated file is private to the existing runner `WorkDir`, is copied
into the root-owned disposable image with ordinary non-executable permissions,
and dies with that guest. It is not added to other groups or the dataplane
guest.

## Scenario and oracle preservation

The bounded proof runs the unchanged exact adoption case. Its load-bearing
outcomes remain:

- the survivor PID actually resides in the survivor namespace;
- the survivor namespace and cgroup survive recovery;
- the lost in-memory slot is re-adopted for the survivor;
- saturation plus a fresh assignment cannot collide with that slot; and
- the orphan namespace is garbage-collected.

The production `adopt_on_restart_recovery`, real namespace/veth provisioner,
real cgroup scope and exact allocator behavior are unchanged. No no-op network
adapter, replacement process, fake namespace or relaxed skip path is used.

## Canonical bounded replay

The retained increment-C harness uses the existing GuestKit API, scripted
peer, prepared control-plane integration binary, exact module/tool closure,
same CID slot 15 and exact existing libtest identity. It adds no production
seam, selectable group, CI lane, executor or alternate scenario.

Durable Metal `73e45a4bc5b719df49b62f80` exited zero. Its receipt reports:

- resolver target preflight passed as empty ordinary non-symlink file;
- `setsid /bin/sleep 0` passed;
- the exact adoption body ran;
- libtest reported one passed, zero failed, zero ignored;
- no uppercase `SKIP ` diagnostic appeared; and
- `CONTROL_PLANE_NATIVE_BOUNDED_REPLAY_PASS` was retained.

This proves the exact affected entry point and original assertions. It does
not substitute for the complete 98-body gate.

## Runner phase and accounting preservation

The complete dataplane result is retained. The control-plane correction
changes neither manifest nor counter:

| Boundary | Status / requirement |
| --- | --- |
| `mtls-relay` 7 + 36 + 3 | **46 passed**, retained Metal `8066a14e0a5afc0f0c4919fd` |
| `cgroup-default` baseline | 55 |
| STEP 13-04 | 18 |
| STEP 13-05 | 5 |
| Retained cgroup subtotal | 78 |
| Additional control-plane native | 20 |
| Complete `cgroup-default` | 98 |
| Complete normal/native overlap ledger | 122 |

The empty resolver target and `setsid` are staged before the control-plane
guest runs, but they carry no active network, namespace or process state. The
existing retained control-plane bodies still run before the 20-body suffix,
and the acceptance/worker guests are unchanged. Required invocations still
reject uppercase/lowercase infrastructure skips.

## Confinement and physical-host boundary

`setsid`, `/bin/sleep` and the empty resolver target are guest image files.
The survivor process, mount namespace, resolver bind, netns, veths and cgroup
scope exist only after Cloud Hypervisor boot and die with the disposable
guest/RAII guards.

Cloud Hypervisor retains uid/gid 4200, KVM/vhost-vsock supplementary groups,
gid-4200 `0710` parent traversal, per-allocation ownership, Landlock, seccomp,
`no_new_privs`, device modes and lifetime. No physical-host resolver file,
process session, DNS configuration, namespace, cgroup, route, nft, BPF or
sysctl state is read into or mutated by this closure.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Reproduction | PASS | Exact adoption path reports missing root resolver target and missing `setsid`; required skip policy rejects false pass. |
| Command-chain precision | PASS | `ip netns exec -> setsid -> sleep`; no shell/true/alternate command. |
| Resolver ownership | PASS | Production still writes per-netns responder content; empty root file is mount target only. |
| Minimal closure | PASS | Conditional `setsid` plus one zero-byte regular file; existing `/bin/sleep` reused. |
| Fail-closed preflight | PASS | Regular, non-symlink, empty target and exact setsid/sleep execution required. |
| Scenario honesty | PASS | Same real adoption body and all survivor/slot/collision/orphan assertions. |
| Bounded proof | PASS | Exact body 1/1, no skip, exit zero in existing guest boundary. |
| Dataplane gate | PASS | Complete 46-body receipt independently retained; CP-only delta cannot affect it. |
| Accounting | PASS | 98-body plan and exact 122 overlap ledger unchanged. |
| Product scope | PASS | No production API/config/resolver behavior, scenario, model, lane, group or executor. |
| Physical-host safety | PASS | File/tool/process/netns/cgroup effects stay inside disposable guest. |

The closure adds no acceptance behavior, input class, state transition, mode,
typed error or concurrency claim. Contract Shape, C1-C7 taxonomy and scenario
completeness remain unchanged. Delta completeness remains 15/15 under the
canonical checklist's N/A rule.

## Iteration 8 mechanical verification

| Check | Result |
| --- | --- |
| Supplied runner/handoff/harness hashes | PASS — exact requested identities. |
| Failed 98-body receipt | PASS as classification — SHA `35f737fd9a985ebd8968ca0bc21f1533f421d52c9b5be809e329780543f7ef23`, exit 101. |
| Exact failing identity | PASS — first 20-body suffix adoption case. |
| Resolver failure | PASS — existing per-netns source present; root `/etc/resolv.conf` target absent. |
| Process failure | PASS — exact `setsid sleep 3600`; `setsid` absent, `/bin/sleep` already staged. |
| Runtime skip honesty | PASS — libtest ok with `SKIP` was rejected by runner. |
| Runner delta | PASS — conditional `setsid`/ELF closure plus generated zero-byte resolver target only. |
| Resolver target invariants | PASS — regular file, non-symlink, zero length; no host content copied. |
| Preflight | PASS — `/usr/bin/setsid /bin/sleep 0`. |
| Selected 98+46 transitive command audit | PASS in source — no further executable/script/runtime omission. |
| Bounded adoption receipt | PASS — SHA `202242ae3ddbb958cc9a612662adbe9b552c892c7850bea2df4c7ebd3a475d53`. |
| Original assertions | PASS — survivor PID/netns/cgroup, slot adoption, collision prevention and orphan GC retained. |
| Bounded libtest result | PASS — 1 passed, 0 failed, 0 ignored, no skip, durable exit zero. |
| Complete dataplane native receipt | PASS — SHA `abd817092d3554facc960601d8fcefb4a052bd49eaf03f1a77204108220bd494`; one seven-pass + 39 one-pass summaries, no skip, exit zero. |
| Do-not-rerun dataplane disposition | PASS — runner's dataplane path unchanged by CP-only correction. |
| `cgroup-default` totals | PASS in source — exact 78 + 20 = 98 unchanged. |
| Complete overlap ledger | PASS — exact 122 unchanged. |
| VMM/confinement/host effects | PASS — unchanged guest-only boundary. |
| Formatting / Clippy | PASS as reported by durable Metal `a454dea7957f8ffd935a8f51`; `cargo fmt --check` independently rerun. |
| Final source check | PASS as reported by `15445e512727e6fc7606e927`. |
| `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Complete 98-body native retry | NOT RUN — required next. |
| Clean normal 1,224-body gate | NOT RUN — required after the 98-body pass. |
| New production/API/scenario/model/lane/group/DES/commit/mutation work | NONE. |

## Iteration 8 verdict

**APPROVED.** The first 98-body attempt proved two missing minimal-rootfs
dependencies in the exact adoption process chain. Production already created
the namespace-specific resolver body and the guest already carried sleep; the
fixture lacked only the ordinary root bind target and `setsid` executable.

The correction stages qualified `setsid` and an empty regular
`/etc/resolv.conf` only for the affected control-plane integration guest, then
fails closed on exact shape and execution. It does not copy host DNS content,
invent a resolver, add a shell chain or weaken a skip/assertion.

The bounded exact replay reaches every original survivor, adoption,
collision and orphan-GC oracle with no skip. The complete 46-body dataplane
receipt already passes and remains current; it must not be rerun for this
control-plane-only delta. The 98-body native and clean normal gates remain
pending and are not claimed by this approval.

The original STEP 13-05 crafter may now rerun only the 98-body
`cgroup-default` gate. After it passes, the retained 46-body dataplane receipt
plus the new 98-body receipt authorize the exact 122-identity selector and the
clean actual three-package normal gate. Any further failure must be classified
without copying host configuration, adding speculative tools or weakening the
existing oracle.

# APPROVED — ITERATION 8

---

# Review iteration 9 — DNS fallback fixture and qualified gateway route

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 9 |
| Review date | 2026-10-28 |
| Scope | Actual DNS/53 ownership diagnosis, two positive fallback-fixture corrections, qualified gateway-local fixture route and cleanup, plus the still-active negative no-bindable-port control |
| Prior disposition | Iterations 1-8 remain approved; the complete 46-body dataplane gate remains passed and current |
| Verdict | **CHANGES_REQUIRED** |
| Open findings | 1 blocker |

The positive DNS fixture and the runner's qualified gateway precondition are
correctly bounded and pass their exact original outcomes. One existing
negative DNS fixture remains intrinsically unable to reach its assertion, and
the runner does not recognize its uppercase-colon skip diagnostic. That gap
would allow the pending 98-body gate to count a skipped negative scenario as a
pass.

Only this review artifact was appended. No production source, test source,
runner source, assertion, marker, API, lane, group, scenario, model, DES event,
execution log or commit was changed by the reviewer. No Metal work was
started by this review.

## Iteration 9 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `726b05485b07406f161358c5944453fdedd7ae3d4e614a33ae1178304f4fb5cc` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `ca5a0e708e25e3ddf4393da85c04515091080e9e750db6eb47f109b4315932e5` |
| `dns_responder_bind.rs` | `620cb26fe2f565a667fadfdfdad575e8f270789215e62946426a01c24a913741` |
| Failed 98-body DNS receipt | `7a9c0bd872352433677c82e5f83959bfe300ec21fa84d99060457f09d7460781` |
| DNS owner/sequence diagnostic | `d82c5a2ad66c928e9b8edc59ec3082073c18d9a975fe49f3db720a5e0394e209` |
| Bounded positive DNS receipt | `c9e09ba097cadab006ba7627a4869220d1022f11427be38c39a80d686ff58edc` |
| Iteration 8 review before this append | `cdb3d0262df212226a3a6465de46a8d5937da9ac6fcdba95132ff57605b84f3a` |

The 46-body dataplane receipt, 98-body manifests/counts, 122-identity overlap
ledger, module/tool/account closures and confinement remain otherwise
unchanged.

## Actual 98-body DNS failure

After iteration 8's approved control-plane closure, durable Metal
`3421ff034fb99c41ea416500` reached the first positive DNS fallback body
`empty_fallback_binds_zero_sockets_and_warns_it_is_deaf`. It exited 101 when
the production responder attempted its exact shared-gateway fallback:

```text
Bind { addr: 100.95.0.1:53,
       source: Os { code: 98, kind: AddrInUse, message: "Address already in use" } }
```

The failure occurred after the body successfully created its own wildcard
`0.0.0.0:53` stand-in. The production fallback path and typed error remained
unchanged. This was a fixture-input conflict, not a responder defect.

## Fresh and adoption-prefix diagnosis

Durable diagnostic `a1a5715691ae238e0a7032a7` compared the positive DNS
body in a fresh disposable guest and in another fresh guest after the exact two
preceding adoption cases. Both sequences proved before the DNS body:

- zero UDP/53 kernel rows;
- zero process owners of UDP/53;
- only loopback `127.0.0.1/8` in the root namespace;
- no local `100.95.0.1/32` route;
- both adoption cases passed with no `SKIP`; and
- no inherited DNS descriptor survived either adoption process.

The body itself then created the sole UDP/53 owner:

- same libtest PID and command;
- root network namespace `4026531833`;
- fd 9 with an exact socket inode;
- local address `0.0.0.0:53`;
- `SO_REUSEADDR=0`;
- `SO_REUSEPORT=0`; and
- `FD_CLOEXEC=1`.

The fresh and adoption-prefix runs failed identically. This rules out
production/adoption leakage, a prior process, inherited fd, sibling namespace
or test-order contamination. The wildcard holder was the body's own socket.

Linux's overlapping-bind rule rejects a specific bind when an overlapping
wildcard socket lacks reuse. The body's wildcard holder therefore blocked the
same gateway fallback it expected to succeed. The absent gateway-local route
was a second independent missing qualified-node precondition.

## Positive fallback fixture correction

The two positive fallback bodies now use a private holder bound to
`127.0.0.53:53`. That address represents the appliance's specific loopback
resolver rather than a wildcard listener.

This preserves the intended stimulus:

- the no-reuse specific holder makes the production wildcard
  `0.0.0.0:53` bind return `EADDRINUSE`;
- it does not occupy the distinct shared gateway `100.95.0.1:53`;
- the production code must therefore take its unchanged per-gateway fallback;
  and
- the original positive `probe()` success remains the verdict.

No socket reuse option, responder method, fallback selection, owner behavior,
typed error or public test surface changes. The private helper is used only by
the two positive fixtures.

Diagnostic output around the second positive retains UDP/53 kernel rows,
process/fd/inode ownership, local address, reuse flags, CLOEXEC, command,
netns, interface state and gateway route. It is read-only and does not replace
an assertion.

## Qualified gateway-local fixture route

The control-plane native suffix now receives the exact appliance precondition:

```text
local 100.95.0.1/32 dev lo proto 247
```

The runner:

1. proves the exact route is absent before setup;
2. adds only that local `/32`;
3. reads back the exact protocol/interface/scope identity;
4. proves `100.95.255.200` remains unroutable;
5. runs only the 20-body control-plane native suffix;
6. removes the exact proto-247 route afterward;
7. proves the route is absent and the non-gateway prefix remains unroutable;
   and
8. attempts the same cleanup after any suffix failure before returning the
   original test error.

It adds no interface address, prefix-wide route, main-table/default route,
network namespace, runtime writer or physical-host route. The retained
baseline/STEP 13-04/STEP 13-05 control-plane bodies execute before this setup.

This fixture cleanup does not weaken product GW-LOCAL. Product ownership says
the production boot-converged route has no runtime remover. The standalone DNS
fixtures do not own that boot lifecycle; their disposable-guest test harness
owns and removes its precondition after the suffix, and guest destruction is
the fail-closed complement.

## Bounded positive proof

Durable Metal `61d8273016ed5daa3b3cf1cb` ran two sequences under the
qualified route:

- the two positive fallback bodies in a fresh guest; and
- both adoption cases followed by the same two positive fallback bodies in a
  second fresh guest.

All six exact bodies passed, no `SKIP` appeared and the run exited zero. The
retained evidence shows:

- no UDP/53 owner before each positive body;
- only `127.0.0.53:53` after the private holder;
- distinct production-owned `100.95.0.1:53` after probe;
- the exact proto-247 gateway-local route;
- the holder's `SO_REUSEADDR=0`, `SO_REUSEPORT=0` and CLOEXEC state; and
- no adoption-prefix descriptor leakage.

The proof validates the positive fixture and qualified-node route. It does not
exercise the separate no-bindable-port negative body and cannot establish the
complete 98-body result.

## Preserved boundaries

The complete 46-body dataplane receipt remains passed and current. DNS and
gateway changes are confined to the control-plane test/runner path. Native
manifests and counts remain:

| Boundary | Status / requirement |
| --- | ---: |
| `mtls-relay` complete | **46 passed**, retained |
| `cgroup-default` retained subtotal | 78 |
| Control-plane native suffix | 20 |
| `cgroup-default` complete | 98 |
| Complete normal/native ledger | 122 |

Cloud Hypervisor uid/gid 4200, device-owner groups, Landlock, seccomp,
`no_new_privs`, parent traversal, device modes and guest-only kernel effects
remain unchanged. The route and sockets exist only in disposable guests.

## Blocking finding

### F-09-01 — The active no-bindable-port negative fixture still skips before its typed error assertion

- **Severity:** Blocker.
- **Kind:** `AT_GAP_IN_DELIVERY_SCOPE` — the typed `DnsResponderError::Bind`
  outcome and the existing private socket fixture are fully specified; no
  DESIGN choice is missing.
- **Claim:** `probe_refuses_when_no_bindable_port` remains an exact active row
  in `CONTROL_PLANE_NORMAL_NATIVE`, but its two stand-in binds cannot coexist.
  It returns through `SKIP:` before calling the production probe, and the
  required runner's skip detector does not recognize uppercase `SKIP:`.

#### Reachable source path

The body first binds a default no-reuse wildcard socket at `0.0.0.0:53`
(`dns_responder_bind.rs:418-423`). It then attempts to bind another default
no-reuse socket at `100.95.0.1:53` (`:433-438`). On the qualified runner route,
the gateway address is local, but the first wildcard socket overlaps it.

The retained native diagnostic already proves the relevant kernel fact on the
same kernel and standard-library socket path: a `UdpSocket::bind` wildcard
holder has `SO_REUSEADDR=0` and `SO_REUSEPORT=0`, and it caused the production
gateway bind to fail `EADDRINUSE`. The second fixture holder is subject to the
same overlap check. Therefore its `let Ok(...) else` branch is reachable and
the body prints:

```text
SKIP: could not bind stand-in gateway :53 at 100.95.0.1
```

It then returns before `dns_collision.probe()` and before the existing
`DnsResponderError::Bind` assertion. This is not a hypothetical schedule or a
production failure; it is the same reproduced wildcard-overlap mechanism that
iteration 9 corrected in the positive fixtures.

#### False-pass path in the runner

`exec_cgroup_with_skip_policy` rejects `[skip]`, uppercase `SKIP `, lowercase
`skip:` and `(skip)`. It does not reject uppercase `SKIP:`. The negative body
therefore reports `test result: ok. 1 passed; 0 failed` and its skip can advance
the 20-body/98-body counters.

The bounded six-pass receipt does not close this gap: it contains only the two
positive DNS bodies, twice, and the two adoption-prefix bodies. It does not
run `probe_refuses_when_no_bindable_port`.

#### Required bounded remediation

Keep the negative contract and assertions unchanged, but make its fixture
constructible:

1. use the existing private `bind_specific_resolver_stand_in()` to block the
   production wildcard path without occupying the gateway address;
2. bind the second holder at the exact gateway `100.95.0.1:53`;
3. call the unchanged production `probe()` and require the existing typed
   `DnsResponderError::Bind` outcome;
4. add uppercase `SKIP:` to the required runner's skip detector (or normalize
   all active DNS fixture diagnostics to a form it already rejects); and
5. retain one exact bounded native receipt for this negative body under the
   qualified gateway route, proving one pass, zero failed/ignored, no runtime
   skip and the typed assertion reached.

Do not change production responder binding/reuse behavior, add a public API,
weaken the typed error, remove the negative row, or credit the existing
positive-only receipt. The passed 46-body dataplane result remains current and
must not be rerun.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Positive fixture | PASS | Specific 127.0.0.53 holder blocks wildcard while leaving gateway fallback free. |
| Qualified route | PASS | Exact local gateway `/32`, no rest-prefix/default/interface expansion, fail-closed cleanup. |
| Leakage diagnosis | PASS | Fresh and adoption-prefix runs prove zero prior UDP/53 owners and CLOEXEC holder ownership. |
| Positive bounded proof | PASS | Both positive bodies pass fresh and after adoption prefix with distinct holder/production sockets. |
| Negative fixture | **FAIL** | Wildcard holder prevents the second gateway holder; typed error assertion is unreachable. |
| Runtime skip honesty | **FAIL** | Uppercase `SKIP:` is not matched by the required cgroup skip detector. |
| Dataplane gate | PASS | Complete retained 46-body receipt unaffected. |
| Accounting | BLOCKED | Pending 98-body result could falsely credit the skipped negative body. |
| Product scope | PASS | Required remediation is private test fixture/runner support only. |
| Physical-host safety | PASS | All route/socket effects remain disposable-guest local. |

## Iteration 9 mechanical verification

| Check | Result |
| --- | --- |
| Supplied runner/DNS/handoff hashes | PASS — exact requested identities. |
| Failed DNS receipt | PASS as classification — SHA `7a9c0bd872352433677c82e5f83959bfe300ec21fa84d99060457f09d7460781`. |
| Fresh/adoption-prefix diagnosis | PASS — SHA `d82c5a2ad66c928e9b8edc59ec3082073c18d9a975fe49f3db720a5e0394e209`. |
| Preexisting UDP/53 owners | PASS — zero in both diagnostic sequences. |
| Intrinsic wildcard conflict | PASS — body-owned wildcard holder, reuseaddr/reuseport zero, gateway fallback EADDRINUSE. |
| Positive private holder | PASS — exact 127.0.0.53:53; production gateway remains distinct. |
| Qualified route setup/readback | PASS — exact local 100.95.0.1/32 dev lo proto 247. |
| Rest-prefix isolation | PASS — 100.95.255.200 remains unroutable. |
| Route cleanup | PASS in source — success and failure paths attempt exact removal/readback. |
| Positive bounded receipt | PASS — SHA `c9e09ba097cadab006ba7627a4869220d1022f11427be38c39a80d686ff58edc`, six passed, no skip. |
| Negative fixture reachability | **FAIL — blocker F-09-01.** |
| Uppercase-colon skip detection | **FAIL — blocker F-09-01.** |
| Complete 46-body dataplane gate | PASS and retained; no rerun authorized. |
| Complete 98-body control-plane/worker gate | NOT RUN / BLOCKED pending F-09-01. |
| Clean normal 1,224-body gate | NOT RUN — follows honest 98-body pass. |
| Formatting / source checks | PASS as reported by `24abc76fe8bf38ea00ca1834`; `cargo fmt --check` independently rerun. |
| `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Production/API/lane/group/scenario/model/DES/commit/mutation changes by reviewer | NONE. |

## Iteration 9 verdict

**CHANGES_REQUIRED.** The two positive DNS fallback fixtures and the runner's
qualified gateway-local precondition are approved. Native evidence proves no
prior UDP/53 owner or adoption leak, exact private-holder ownership, distinct
production gateway socket, rest-prefix isolation and cleanup. The complete
46-body dataplane receipt remains passed and current.

The active no-bindable-port negative fixture still cannot construct both of
its stand-in sockets. It exits through uppercase `SKIP:` before its production
probe and typed `DnsResponderError::Bind` assertion, while the runner fails to
recognize that skip spelling. A 98-body receipt could therefore claim one pass
without executing this negative contract.

Correct only that private negative fixture and the skip detector, retain an
exact bounded no-skip typed-error proof, then return for iteration 10. Do not
rerun the passed dataplane gate. The full 98-body and clean normal gates remain
blocked until this finding is resolved.

# CHANGES_REQUIRED — ITERATION 9

---

# Review iteration 10 — no-bindable-port negative fixture and shared skip detector

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 10 |
| Review date | 2026-10-28 |
| Scope | Remediation of F-09-01: constructible no-bindable-port fixture, unchanged typed `Bind` assertion, and one line-oriented runtime-skip detector shared by both native executors |
| Prior disposition | Iteration 9's positive DNS fixtures and qualified gateway-local route remain approved; the complete 46-body dataplane receipt remains passed and current |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews only the last blocker from iteration 9. The negative
scenario's production call and typed error assertion are unchanged. The
fixture removes an impossible redundant holder, and the runner now rejects the
actual uppercase-colon skip form without treating arbitrary prose as a skip.

Only this review artifact was appended. No production source, public API,
error variant, body count, scenario, model, lane, selectable group, guest
executor, DES event, execution log or commit was changed by the reviewer. The
complete 98-body and clean normal gates remain pending.

## Iteration 10 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `98673721ee0ca713cc4f648e1168772091c16332c2ed121a4758fb39777936e2` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `8117e64399936d447274de499d5b98255e8b62f98007b347a3eed24611bd14d7` |
| `dns_responder_bind.rs` | `9dceda23a2bc4958520c373654ac13a4f9566355c88214baae9e4eeb1f443029` |
| Old negative skip-hole receipt | `f15a1b4460260a0cdaac04d75d0361804a57a0857d0e93e08b6757aaa9f54ab6` |
| Fixed negative typed-error receipt | `de3b767616128c2285e2906260e203a805f52c965c1cfe2476f8d6e229368cf5` |
| Iteration 9 review before this append | `8e1504e1fccf04c281b2fa8761e01d8d7cfdae34fc1bb576e65a654da567fae3` |

All positive DNS fixture source, gateway route setup/cleanup, native manifests,
counts, module/tool/runtime closure, confinement and 122-identity overlap
ledger are unchanged from iteration 9.

## Exact old skip-hole proof

Durable Metal `15c30cfa1bb75e34e45f9637` ran the old exact negative body
under the qualified gateway route. It retained:

```text
SKIP: could not bind stand-in gateway :53 at 100.95.0.1
test result: ok. 1 passed; 0 failed; 0 ignored
DNS53_NEGATIVE_OLD_SKIP_HOLE_CONFIRMED
```

The old body never reached the production `probe()` or its typed
`DnsResponderError::Bind` assertion. This is direct RED evidence for F-09-01,
not merely inference from the positive fixture.

## Negative fixture correction

The corrected body retains its original no-reuse wildcard holder at
`0.0.0.0:53`. On Linux, that one socket overlaps every IPv4 local address and
therefore blocks both:

- the production wildcard `0.0.0.0:53` attempt; and
- the production exact gateway `100.95.0.1:53` fallback.

The second exact stand-in holder was redundant and impossible to create while
the wildcard holder was live. The correction removes only that second holder
and its early-return branch. It does not change the wildcard holder, gateway,
responder, production method, expected error or assertion.

The body now necessarily calls the unchanged production `probe()`, requires
an error, prints its diagnostic representation and retains:

```rust
matches!(err, DnsResponderError::Bind { .. })
```

The observed cause remains exact gateway `100.95.0.1:53` `AddrInUse`, proving
the fallback was attempted and refused by the still-live wildcard holder.

No socket reuse option is added, no production binding rule changes, and the
two positive fixtures continue to use their distinct specific loopback
holder.

## Shared line-oriented runtime-skip detector

The runner now has one private `reports_runtime_skip` function used by both
the dataplane `exec_required` path and control-plane
`exec_cgroup_required` path. It recognizes exactly the documented diagnostic
prefixes:

```text
[skip]
SKIP 
SKIP:
skip:
(skip)
```

Matching is line-oriented. Each line is left-trimmed; for libtest's inline
shape `test <name> ... <diagnostic>`, only the text after the first ` ... ` is
classified. A token in a test name, assertion text, BPF JSON, diagnostic
payload or arbitrary prose does not fail a passing invocation unless it begins
the actual diagnostic field.

This closes the exact uppercase-colon hole while avoiding a broad substring
scan that could invalidate honest retained evidence.

## Classifier replay against retained evidence

An independent read-only replay of the new classifier semantics produces:

| Receipt | Result |
| --- | --- |
| Old negative skip-hole raw | **MATCH** — exact `SKIP:` diagnostic rejected |
| Fixed negative typed-error raw | CLEAR — no runtime skip |
| Retained complete 46-body dataplane raw | CLEAR — no newly uncovered infrastructure skip |

The retained dataplane receipt therefore remains an honest 46-body pass under
the stricter shared detector. No full dataplane rerun is required or
authorized by this control-plane fixture correction.

## Fixed exact native proof

Durable Metal `a9a3ea8eb54fc1ee4697ed0d` first confirmed that the new detector
rejects the retained old uppercase-colon raw output. It then prepared the
current GuestKit artifacts and ran only the corrected exact negative body
under the qualified gateway route.

The receipt records:

```text
[diag] DNS53 negative typed error:
Bind { addr: 100.95.0.1:53,
       source: ... AddrInUse ... }
test result: ok. 1 passed; 0 failed; 0 ignored
DNS53_NEGATIVE_TYPED_ASSERTION_PASS
```

No `SKIP` diagnostic appears. The real typed assertion was reached and passed;
the result is not a fixture return masquerading as a libtest pass.

## Preservation of positive and qualified-route approvals

Iteration 9's approved positive fixtures remain unchanged:

- specific `127.0.0.53:53` holder blocks the production wildcard path;
- exact production `100.95.0.1:53` fallback remains free;
- both positive bodies pass fresh and after the adoption prefix;
- no prior UDP/53 owner or inherited descriptor exists; and
- holder and production sockets remain distinct.

The runner still installs only `local 100.95.0.1/32 dev lo proto 247` around
the 20-body suffix, proves the rest of the guest prefix unroutable and removes
its exact fixture route on success or attempted failure cleanup. No product
GW-LOCAL lifecycle or host route changes.

## Accounting and confinement

No body identity, filter or expected count changes:

| Boundary | Status / requirement |
| --- | ---: |
| Complete `mtls-relay` | **46 passed**, retained |
| Retained `cgroup-default` subtotal | 78 |
| Control-plane native suffix | 20 |
| Complete `cgroup-default` | 98 |
| Complete normal/native overlap ledger | 122 |

Cloud Hypervisor uid/gid 4200, device-owner supplementary groups, Landlock,
seccomp, `no_new_privs`, parent traversal, device modes and guest-only kernel
effects remain unchanged. The negative fixture's sockets and gateway route
exist only inside the disposable guest.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| F-09-01 fixture | **RESOLVED** | One wildcard holder honestly blocks all production bind attempts; no impossible second holder. |
| Typed error | PASS | Unchanged production probe reaches exact gateway `AddrInUse` and existing `DnsResponderError::Bind` assertion. |
| Old false pass | PASS as RED evidence | Exact uppercase-colon skip and libtest one-pass summary retained. |
| Skip detector | PASS | Shared exact-prefix, line-oriented classification includes uppercase `SKIP:`. |
| False-positive control | PASS | Retained 46-body raw remains clear under new classifier. |
| Fixed bounded proof | PASS | Exact body reaches typed assertion, 1/1, no skip, exit zero. |
| Positive DNS fixtures | PASS | Iteration 9 approvals unchanged. |
| Qualified gateway fixture | PASS | Exact `/32`, rest-prefix isolation and cleanup unchanged. |
| Product scope | PASS | Private fixture/runner classification only; no product API/error/binding behavior. |
| Native accounting | PASS | 46/98/122 boundaries unchanged. |

The remediation repairs fixture reachability and evidence classification
without adding acceptance behavior, input partitions, state, mode, error
taxonomy or concurrency claims. Contract Shape, C1-C7 taxonomy and scenario
completeness remain unchanged. Delta completeness remains 15/15 under the
canonical checklist's N/A rule.

## Iteration 10 mechanical verification

| Check | Result |
| --- | --- |
| Supplied runner/DNS/handoff hashes | PASS — exact requested identities. |
| Old negative receipt | PASS — SHA `f15a1b4460260a0cdaac04d75d0361804a57a0857d0e93e08b6757aaa9f54ab6`. |
| Old fixture behavior | PASS as RED — uppercase `SKIP:`, 1 libtest pass, typed assertion not reached. |
| Fixture delta | PASS — impossible second gateway holder only; original wildcard and typed assertion retained. |
| Fixed negative receipt | PASS — SHA `de3b767616128c2285e2906260e203a805f52c965c1cfe2476f8d6e229368cf5`. |
| Fixed typed cause | PASS — exact gateway `AddrInUse`, `DnsResponderError::Bind`. |
| Fixed libtest result | PASS — 1 passed, 0 failed, 0 ignored, no skip, durable exit zero. |
| Shared detector prefixes | PASS — `[skip]`, `SKIP `, `SKIP:`, `skip:`, `(skip)`. |
| Line/libtest separator classification | PASS — diagnostic start only; no arbitrary substring matching. |
| Old raw classifier replay | PASS — rejected. |
| Fixed raw classifier replay | PASS — clear. |
| Retained 46-body classifier replay | PASS — clear; no dataplane rerun needed. |
| Positive DNS / route source | PASS — unchanged from iteration 9. |
| `cgroup-default` totals | PASS in source — exact 78 + 20 = 98. |
| Complete overlap ledger | PASS — exact 122 unchanged. |
| VMM/confinement/host effects | PASS — unchanged guest-only boundary. |
| Formatting / source check | PASS as reported by `f51a5570d495279be0e2d0d8`; `cargo fmt --check` independently rerun. |
| `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Complete 98-body native retry | NOT RUN — required next. |
| Clean normal 1,224-body gate | NOT RUN — required after honest 98-body pass. |
| Production/API/scenario/body-count/lane/group/model/executor/DES/commit/mutation changes by reviewer | NONE. |

## Iteration 10 verdict

**APPROVED.** F-09-01 is resolved. The negative no-bindable-port body now uses
its one wildcard holder as the actual all-address blocker, reaches the
production gateway fallback and passes the original typed `Bind` assertion.
The impossible second holder and its early return are gone.

The shared line-oriented detector rejects the exact uppercase-colon false pass
without matching unrelated text. Read-only replay confirms the retained
46-body dataplane receipt remains clean under the stricter semantics, so that
passed gate remains current and must not be rerun.

Positive DNS fixtures, gateway route isolation/cleanup, manifests, counts,
confinement and the 122-identity ledger remain unchanged. The bounded negative
pass is causal confirmation only; it does not satisfy the complete 98-body or
normal gates.

The original STEP 13-05 crafter may now rerun only the 98-body
`cgroup-default` gate. After it passes, the retained 46-body dataplane receipt
and new 98-body receipt authorize the exact selector and clean actual three-
package normal gate. Any runtime diagnostic matched by the shared detector or
any non-reconciling count reopens this review.

# APPROVED — ITERATION 10

---

# Review iteration 11 — exact gateway-route ownership precondition

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 11 |
| Review date | 2026-10-28 |
| Scope | Remediation of the complete-98 replay's contradiction between an already-converged GW-LOCAL route and the control-plane native suffix's former absence-only fixture precondition |
| Prior disposition | Iteration 10 remains approved; the retained complete 46-body dataplane receipt remains passed and current |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews the private GuestKit fixture's classification and
cleanup of the exact gateway-local route. It does not approve a production
route writer, a product ownership protocol, or a complete 98-body result. The
accepted GW-LOCAL behavior remains: boot keeps an identical route, replaces a
differing route, adds a missing route, and never removes the route.

Only this review artifact was appended. No production source, test body,
assertion, public API, typed error, scenario, model, lane, selectable group,
guest executor, staging declaration, DES event, execution log or commit was
changed by the reviewer. No Metal, native, broad, kernel-mutating or network-
mutating command was run during this source review.

## Iteration 11 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `256e500e96a0fe8bd141965abfa853b4e66aafa1bc2b468bdb98d0d09485082c` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `bab8ef9047c157a090bc1d337d74293bbf038052d7803df88fbd58c39567e75b` |
| Canonical bounded `increment-h/src/main.rs` | `4ea4157f51427a5ed28f173b7d138209726fc4e95396a92866a73f44491bbe95` |
| Causal complete-98 attempt raw | `aa273d913d510edce182f27e8d37c2b23bad7e1dab981dfc98d624fcdd2888a5` |
| Bounded dual-branch proof raw | `274e4541d3e9bab3a99106709995f6ff17f8d47237ab9c1a59538fca904f2033` |
| `xtask/src/guest_kit_prepare.rs` | `086f1214d6244e801d492e2211a1c99cadf771265d94f2268ab502bebbf951e1` — unchanged |
| `dns_responder_bind.rs` | `9dceda23a2bc4958520c373654ac13a4f9566355c88214baae9e4eeb1f443029` — unchanged from iteration 10 |
| `overdrive-netlink/src/client.rs` | `d2b204e796472f3a0c77ec631b95ee751cdaa9bea4c44b0d3ae9c97f17516f3e` |
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| `deliver/roadmap.json` | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| Iteration 10 review before this append | `ed3d32ddb46bfec6d614903ad7bfd68312f3a948ab3f44fc710fc2c847578021` |

## Causal replay evidence

Durable Metal `46a9510e9f4754ac06a947dc` attempted the complete 98-body
`cgroup-default` gate after iteration 10. Its retained and shared-startup
bodies passed through the qualified boot path, which correctly left the
GW-LOCAL route installed. Before the 20-body native suffix, the former fixture
then required this route to be absent:

```text
test -z "$(ip -4 route show table local exact 100.95.0.1/32)"
```

That precondition exited nonzero, so the runner stopped before selecting the
suffix. The receipt ended with run id `46a9510e9f4754ac06a947dc`, exit 101.
This proves a fixture contradiction at the real disposable-guest sequence. It
does not expose a production defect and cannot count as a complete-98 pass.

The accepted design and production source agree on the route's lifetime and
shape. STEP 13-05 requires a gateway `/32` local route that is kept when
identical, replaced when differing, added when missing, never removed, and has
no runtime writer. `Client::converge_ipv4_host_local_route` builds exactly:

- table `local`;
- destination `100.95.0.1/32` when called for `GUEST_NODE_GATEWAY`;
- type `Local` and scope `Host`;
- protocol 247;
- output interface `lo`; and
- no gateway attribute.

The fixture's old absence check and unconditional later removal therefore
could not represent a sequence in which an earlier production boot had
already converged the valid route.

## Source reconciliation — missing, borrowed and differing states

`CONTROL_PLANE_NORMAL_NATIVE_SETUP` remains private runner support and now
reads the exact local-table key before the 20-body suffix. Its three-way
classification is bounded and fail-closed:

| Observed state | Fixture action | Required evidence |
| --- | --- | --- |
| No route at `100.95.0.1/32` | Add `local 100.95.0.1/32 dev lo proto 247`; record `created` | Re-read exact qualified rendering |
| Exact qualified route already present | Make no route mutation; record `borrowed` | Re-read exact qualified rendering |
| Any other rendering at the key | Exit with the actual rendering | No replace, repair or normalization |

The only accepted renderings are the normalized `iproute2` forms
`local 100.95.0.1 dev lo proto 247 scope host` and the same form with an
explicit `table local` token. This pins destination, prefix length, table,
route type, loopback output, protocol, host scope and the absence of a gateway
attribute. It does not accept a prefix route, default route, other protocol,
other interface, other route type or broader address set.

Both successful branches also require lookup of `100.95.255.200` to fail.
Thus the setup continues to prove that no route covers the rest of the guest
prefix. It creates no interface address, namespace, uplink, main-table route
or default route.

The ownership marker contains only `created` or `borrowed`, under `/run` in
the disposable guest. Setup requires the path not to exist before writing it.
This is run-local fixture bookkeeping, not a product ownership or recovery
mechanism, and it adds no public or cross-crate surface.

## Branch-specific cleanup and failure behavior

Cleanup first requires both the marker and the same exact qualified route.
Only then does it branch:

- `created` deletes the validated local-table `/32` and proves the exact key
  absent;
- `borrowed` performs no delete and retains the exact route readback; and
- any missing/differing route or unknown marker fails closed.

The created delete is preceded by exact validation including protocol 247 and
the private `created` marker, so it cannot delete a differing route. The
borrowed branch cannot delete or replace an existing product-converged route.
Both branches recheck that `100.95.255.200` remains unroutable.

`run_cgroup_binary` executes this cleanup after the native suffix. A body
error still triggers the same branch-specific cleanup attempt before the
disposable guest is dropped, while preserving the authored body failure as
the returned error. A cleanup error after successful bodies fails the group.
The marker intentionally lasts only for the remaining disposable-guest
lifetime; there is no later selected family in that guest after this suffix.

This is the smallest fixture correction consistent with GW-LOCAL. It does not
teach the fixture to replace a differing route, conceal a prior body's wrong
state, or clean a route it merely borrowed.

## Bounded dual-branch proof

Canonical bounded GuestKit proof `678b2e2aba65af9927e27fe0` ran both ownership
branches in separate disposable Cloud Hypervisor guests and exited zero. The
retained raw output proves:

| Branch | Selected unchanged bodies | Route result |
| --- | ---: | --- |
| Fresh / `created` | Two positive DNS fallback bodies | Exact route installed; cleanup `gateway_after=''` |
| Preexisting / `borrowed` | Two adoption bodies plus the same two positive DNS fallback bodies | Exact route accepted without setup mutation and retained after cleanup |

All six invocations reported `1 passed; 0 failed; 0 ignored`. The shared
line-oriented runtime-skip classifier finds no `[skip]`, `SKIP `, `SKIP:`,
`skip:` or `(skip)` diagnostic. The proof records distinct private resolver
holder `127.0.0.53:53` and production gateway `100.95.0.1:53` sockets,
retains the non-gateway-prefix unroutability check, and ends with:

```text
DNS53_GATEWAY_OWNERSHIP_PASS
OVERDRIVE_METAL_RUN_COMPLETE run_id=678b2e2aba65af9927e27fe0 exit_code=0
```

The fresh branch proves fixture-created removal. The preexisting branch proves
the defect's actual required case: adoption/boot-visible valid GW-LOCAL state
is borrowed and survives the native suffix fixture cleanup.

This proof is causal confirmation of the private precondition and cleanup. It
does not replace the complete 98-body gate because it selects only six bodies
across two bounded diagnostic guests.

## Preservation and accounting

Iteration 11 changes no test stimulus, production entry point, assertion,
error oracle, scenario or expected count. Iterations 9 and 10 remain intact:

- the positive DNS fixtures retain the specific no-reuse
  `127.0.0.53:53` holder and the exact gateway fallback;
- the negative body retains one no-reuse wildcard holder and reaches the
  original typed `DnsResponderError::Bind` / `AddrInUse` assertion;
- the shared native skip detector retains its exact line-oriented prefixes;
- the adoption bodies retain their survivor PID/netns/cgroup, namespace,
  slot-adoption, collision and orphan-GC assertions; and
- the route support does not alter product GW-LOCAL convergence or lifetime.

The approved boundary and ledger are unchanged:

| Boundary | Status / requirement |
| --- | ---: |
| Complete `mtls-relay` native gate | **46 passed**, retained; no rerun |
| Retained `cgroup-default` subtotal | 78 |
| Control-plane native suffix | 20 |
| Complete `cgroup-default` gate | 98, still pending |
| Complete normal/native overlap ledger | 122 identities |

The same eight selectable groups, three `cgroup-default` guests, CIDs/slots,
executors, module/tool payloads, private cgroup2/bpffs mounts and prepared BPF
identity remain in place. Cloud Hypervisor uid/gid 4200, supplementary device
groups, 0710 traversal, device modes, Landlock, seccomp and `no_new_privs`
remain unchanged. All route effects in the supplied proof occur in disposable
guests; this review performed no host kernel or network mutation.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Causal failure | PASS as RED evidence | Actual qualified sequence reached a valid existing GW-LOCAL route and the old absence-only fixture rejected it. |
| Design reconciliation | PASS | Fixture accepts only the exact production route shape and preserves GW-LOCAL's never-remove lifetime for borrowed state. |
| Missing branch | PASS | Adds exact local `/32`, records `created`, re-reads and later removes only after exact validation. |
| Identical branch | PASS | Records `borrowed`, performs no setup mutation, never deletes, and requires the exact route through cleanup. |
| Differing branch | PASS | Fails with actual rendering; no replace or broad repair. |
| Prefix isolation | PASS | `100.95.255.200` remains unroutable in setup and cleanup; no prefix-wide workaround. |
| Body-failure cleanup | PASS | Branch-specific cleanup is attempted while the selected body's original failure remains authoritative. |
| Bounded proof | PASS | Both branches, six exact unchanged passes, zero ignores/skips, exit zero. |
| DNS/adoption oracles | PASS | Prior source and assertions unchanged; bounded output reaches them. |
| Public/design surface | PASS | Private fixture bookkeeping only; no API, error, scenario, model, group, lane or executor. |
| Native accounting | PASS | 46/98/122 boundaries unchanged and honestly distinguished. |

The correction changes only test-fixture HOW. It does not add an acceptance
input partition or observable product behavior. Contract Shape, C1-C7
taxonomy coverage and the canonical 15-item completeness checklist therefore
remain unchanged and complete under the checklist's N/A rule.

## Iteration 11 mechanical verification

| Check | Result |
| --- | --- |
| Supplied runner/handoff/harness hashes | PASS — exact requested identities. |
| Causal raw hash | PASS — `aa273d913d510edce182f27e8d37c2b23bad7e1dab981dfc98d624fcdd2888a5`. |
| Causal sequence | PASS as RED — retained/startup bodies passed; old absence-only suffix setup exited before selection. |
| Production route shape | PASS in source — local table, Local, Host, protocol 247, `/32`, `lo`, no gateway. |
| Production lifetime | PASS in design/source — boot converge; shutdown/stale sweep has no remove path. |
| Setup classification | PASS — missing/create, identical/borrow, differing/fail; exact readback. |
| Cleanup ownership | PASS — only validated created route removed; borrowed route preserved; unknown/mismatch fails. |
| Rest-prefix isolation | PASS — non-gateway `100.95.255.200` remains unroutable. |
| Bounded raw hash | PASS — `274e4541d3e9bab3a99106709995f6ff17f8d47237ab9c1a59538fca904f2033`. |
| Bounded body results | PASS — six exact one-pass summaries, zero failed/ignored/runtime-skipped. |
| Bounded route results | PASS — created removed; borrowed retained exactly; durable exit zero. |
| Positive/negative DNS and skip detector source | PASS — unchanged from approved iterations 9/10. |
| `cgroup-default` totals | PASS in source — exact 78 + 20 = 98. |
| Complete overlap ledger | PASS — exact 122 unchanged. |
| VMM/confinement/host effects | PASS — unchanged guest-only execution boundary. |
| `cargo fmt --check` | PASS — independently rerun source-only. |
| `git diff --check` | PASS — independently rerun source-only. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun source-only. |
| Complete 46-body dataplane gate | PASS — retained `8066a14e0a5afc0f0c4919fd`; no rerun. |
| Complete 98-body cgroup retry | NOT RUN — required next. |
| Clean normal 1,224-body gate | NOT RUN — required only after the complete 98-body pass. |
| Production/API/scenario/body-count/lane/group/model/executor/DES/commit/mutation changes by reviewer | NONE. |

## Iteration 11 verdict

**APPROVED.** The fixture now distinguishes a missing route it creates from an
identical GW-LOCAL route it borrows. It accepts only the exact qualified
`local 100.95.0.1/32 dev lo proto 247 scope host` state, fails closed on any
difference, preserves the borrowed route, removes only the validated
fixture-created route, and keeps the rest of the guest prefix unroutable.

The causal 98-body attempt proves why this correction is necessary, and the
dual-guest bounded receipt proves both ownership branches with the unchanged
DNS/adoption bodies and no runtime skip. No production route lifecycle, test
oracle or accepted interface changed.

The original STEP 13-05 crafter may now rerun only the complete 98-body
`cgroup-default` gate. The 46-body dataplane receipt remains current and must
not be repeated. A passing, zero-skip, exactly reconciled 98-body receipt is
still required before the clean actual three-package normal gate. Neither
pending gate is claimed by this approval.

# APPROVED — ITERATION 11

---

# Review iteration 12 — one-separator LEGB route-readback normalization

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 12 |
| Review date | 2026-10-28 |
| Scope | One private normalization line in the existing STEP 13-05 leg-B pre/post audit after actual `iproute2` output exposed a trailing-separator mismatch |
| Prior disposition | Iteration 11's created/borrowed control-plane gateway ownership remains approved; the complete 46-body dataplane receipt remains passed and current |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews only the leg-B fixture audit's parsing of the exact
gateway-local route rendering. The route setup, accepted route shape, every
other audit complement and the selected production body are unchanged.

Only this review artifact was appended. No production source, test body,
assertion, public API, typed error, scenario, model, lane, selectable group,
guest executor, tool payload, dependency, DES event, execution log or commit
was changed by the reviewer. No Metal, native, broad, kernel-mutating or
network-mutating command was run during this source review.

## Iteration 12 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `002da12f61a9fcb05e77388177e32652c2f1c8f4d61ef4d94b0881cc7610e2b3` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `849ac03ad0794a29ff03df9753d9d75a658b56973cd46488244e1de0bb067b78` |
| Canonical bounded `increment-i/src/main.rs` | `e46497ed6b6dae42a6120da525c480674910a539cef896cce5aededc443c683c` |
| Causal complete-98 attempt raw | `4daf2d06b59b4be50222c364d93e8f56f16fd575bd4107f1259b7b91f691af30` |
| Bounded LEGB-audit proof raw | `cb0c1b578b3d869d3965d474bbd8fc36070356808a8d7a5e7bbceee090dce63e` |
| Source-package receipt | `6328e77e9b3d0354f702630d527245b1b7ebd4de6493e377eadd6f0dfe8e4fb7` |
| Iteration 11 review before this append | `dea3eb2b6378fc840f4ab5bbdffb3f017bd2a63b938878685de14eeefc7dd37c` |

All production, DNS/adoption test, staging, module/tool, native-manifest,
confinement and overlap-ledger identities reviewed in iteration 11 remain
unchanged by this one-line runner correction.

## Causal complete-98 replay evidence

Durable Metal `4575d307849bb67617734275` attempted the complete 98-body
`cgroup-default` gate after iteration 11 approval. It passed the 20-body
control-plane native suffix and then the worker retained and STEP 13-04
bodies, including STEP 13-04's temporary-route cleanup. Before selecting the
first existing STEP 13-05 leg-B body, the unchanged audit read:

```text
STEP13-05 route_localnet readback: all=0 default=0 lo=0
STEP13-05 gateway local-route mismatch:
local 100.95.0.1 dev lo proto 247 scope host 
```

The route tokens are exactly the approved GW-LOCAL state. The qualified
`iproute2` build rendered one ASCII space after `host`; command substitution
removed the trailing newline but retained that separator. The old audit
compared the unnormalized string to the exact literal without the separator
and exited before the leg-B body. The durable run ended exit 101.

This is direct evidence of a private evidence-parser defect. It does not
contradict GW-LOCAL, LEGB-DIAL, the production route writer or the selected
body, and the failed receipt cannot count as a complete 98-body pass.

## Exact normalization and strictness

The only runner-source correction in this bounded remediation is immediately
after the existing exact route query:

```sh
gateway="$(/usr/bin/ip -4 route show table local exact 100.95.0.1/32)"
gateway="${gateway% }"
case "$gateway" in
  'local 100.95.0.1 dev lo proto 247 scope host'|\
  'local 100.95.0.1 dev lo table local proto 247 scope host') ;;
  *) exit 1 ;;
esac
```

`${gateway% }` removes at most one literal trailing ASCII space. With no
trailing space it leaves the value unchanged. With two trailing spaces it
removes one and leaves one, which the unchanged strict `case` rejects. It does
not trim leading space, tabs, interior separators, extra attributes or an
arbitrary suffix.

The same exact one-separator normalization was already reviewed in the
control-plane native setup and cleanup. Reusing it in the leg-B audit aligns
the two private readers without broadening the accepted route contract.

The unchanged strict literals still pin:

- table `local`, allowing only `iproute2`'s optional explicit `table local`
  token in the already-approved position;
- destination `100.95.0.1/32` from the exact query;
- route type `local`;
- output device `lo`;
- protocol 247;
- scope `host`; and
- no gateway or other route attribute.

A second trailing separator, different destination/prefix, main-table route,
different device/protocol/scope/type, gateway attribute or unknown token still
fails the audit. There is no prefix-wide workaround and no route repair in
this parser.

## Unchanged qualified-node complement

`STEP13_05_LEGB_AUDIT` still runs both before and after every existing leg-B
body and still requires all of the following:

| Boundary | Unchanged assertion |
| --- | --- |
| Local delivery | `route_localnet=0` for `all`, `default` and `lo` |
| Gateway route | Exact qualified GW-LOCAL `/32` only |
| Main table | Empty |
| Rest of guest prefix | `100.95.255.200` lookup fails |
| Root interfaces | Only `/sys/class/net/lo` |
| Transient policy rule | No `fwmark 0x1 lookup 100` outside the production body |
| Transient policy table | Table 100 empty outside the production body |

STEP 13-04's dummy interface/default-route setup and cleanup remain earlier
and separately bounded. The STEP 13-05 setup still pins and reads back the
three `route_localnet` values and installs only the gateway `/32`. No tool,
guest file, module, namespace, interface, default route, selector, group,
lane, executor or runtime dependency was added.

## Bounded unchanged-body proof

Canonical GuestKit proof `6eeb8cde64128fd1c0daa173` exited zero in a
disposable Cloud Hypervisor guest. Before and after the selected body it
captured both forms explicitly:

```text
gateway_raw=<local 100.95.0.1 dev lo proto 247 scope host >
gateway_normalized=<local 100.95.0.1 dev lo proto 247 scope host>
```

Both audits also recorded `route_localnet=0:0:0`, an empty main table, only
`lo`, the ordinary local/main/default policy-rule baseline, and an empty table
100. The non-gateway prefix lookup remained unroutable.

Between those audits the proof ran the existing exact body:

```text
integration::vsock_legb_dial::
  a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c
```

The original body reported one pass, zero failed and zero ignored. Its socket
readback retained the required production outcome: the client side was sourced
from `127.0.0.1`, carried fwmark 1, and reached the guest-prefix leg-C
destination. The receipt contains no recognized runtime-skip diagnostic and
ends:

```text
LEGB_AUDIT_BOUNDED_REPLAY_PASS
OVERDRIVE_METAL_RUN_COMPLETE run_id=6eeb8cde64128fd1c0daa173 exit_code=0
```

This proves the real one-space rendering is normalized, the full pre/post
complement remains effective, and the unchanged body reaches its real
production path and assertions. It selects one body, so it does not replace
the complete 98-body gate.

## Preservation and accounting

Iteration 12 changes no scenario, stimulus, production entry point, body,
assertion, expected count or runtime-skip rule. In particular:

- exact mark 1 and loopback source bind remain required together by the
  positive leg-B body;
- mark-only and bind-only negative controls remain unchanged;
- gateway exclusion, STEP 13-04 route cleanup, nft output/prerouting oracles,
  var/run alias and payload isolation remain unchanged;
- iteration 11's created/borrowed control-plane route bookkeeping and
  branch-specific cleanup remain unchanged; and
- no production API, route behavior, error representation, scenario, model,
  selectable group, lane or executor was added.

The approved boundary and ledger remain:

| Boundary | Status / requirement |
| --- | ---: |
| Complete `mtls-relay` native gate | **46 passed**, retained; no rerun |
| Retained `cgroup-default` subtotal | 78 |
| Control-plane native suffix | 20 |
| Complete `cgroup-default` gate | 98, still pending |
| Complete normal/native overlap ledger | 122 identities |

Cloud Hypervisor uid/gid 4200, supplementary device groups, 0710 traversal,
device modes, Landlock, seccomp, `no_new_privs`, private cgroup2/bpffs mounts
and guest-only kernel effects remain unchanged. The supplied proof's route,
policy and socket effects occur only in the disposable guest.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Causal failure | PASS as RED evidence | Actual complete-98 attempt reached a valid one-separator route rendering after prior groups passed, and the old parser rejected it before the first leg-B body. |
| Normalization bound | PASS | Removes zero or one trailing ASCII space only; a second separator remains and fails. |
| Route contract | PASS | Exact strict route literals and all route attributes remain unchanged. |
| Audit complement | PASS | `route_localnet`, main table, rest-prefix, interface, rule and table-100 checks remain before/after. |
| Existing body | PASS | Exact unchanged positive body reaches leg C with loopback source and mark 1. |
| Bounded evidence | PASS | Raw/normalized values captured twice; one exact pass, zero failures/ignores/skips; exit zero. |
| Product/design surface | PASS | Private audit parsing only; no production behavior, API, error, scenario, model, group, lane or executor. |
| Native accounting | PASS | 46/98/122 boundaries remain distinct and unchanged. |

The correction changes only fixture evidence parsing. It adds no acceptance
partition or observable product behavior. Contract Shape, C1-C7 taxonomy
coverage and the canonical 15-item completeness checklist remain unchanged
and complete under the checklist's N/A rule.

## Iteration 12 mechanical verification

| Check | Result |
| --- | --- |
| Supplied runner/handoff hashes | PASS — exact requested identities. |
| Causal raw hash | PASS — `4daf2d06b59b4be50222c364d93e8f56f16fd575bd4107f1259b7b91f691af30`. |
| Causal ordering | PASS as RED — control-plane 20 and worker retained/STEP 13-04 paths passed before the old audit rejected the valid route rendering. |
| Source correction | PASS — one `${gateway% }` normalization line before the unchanged strict case. |
| Two-separator strictness | PASS — direct shell semantics leave one separator and therefore cannot match the literals. |
| Route/audit source | PASS — every qualified-node token and complement remains unchanged. |
| Bounded raw hash | PASS — `cb0c1b578b3d869d3965d474bbd8fc36070356808a8d7a5e7bbceee090dce63e`. |
| Bounded body result | PASS — exact existing body 1 passed, 0 failed, 0 ignored; no runtime skip. |
| Bounded production oracle | PASS — client source `127.0.0.1`, fwmark 1, guest-prefix leg-C destination. |
| Source package | PASS — Metal `feec585094c5c96d66fec522`, current runner and bounded harness check/Clippy; receipt SHA `6328e77e9b3d0354f702630d527245b1b7ebd4de6493e377eadd6f0dfe8e4fb7`. |
| `cargo fmt --check` | PASS — independently rerun source-only. |
| `git diff --check` | PASS — independently rerun source-only. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun source-only. |
| Complete 46-body dataplane gate | PASS — retained `8066a14e0a5afc0f0c4919fd`; no rerun. |
| Complete 98-body cgroup retry | NOT RUN — required next. |
| Clean normal 1,224-body gate | NOT RUN — required only after the complete 98-body pass. |
| Production/API/scenario/body-count/lane/group/model/executor/DES/commit/mutation changes by reviewer | NONE. |

## Iteration 12 verdict

**APPROVED.** The leg-B audit now removes exactly the one trailing ASCII
separator emitted by the qualified `iproute2` build before applying the same
strict route-shape literals. A second separator or any route-token difference
still fails. Every route-localnet, route-table, prefix-isolation, interface,
fwmark and table-100 assertion remains unchanged before and after each body.

The causal receipt isolates the old parser failure after the earlier selected
families passed. The bounded receipt captures the raw and normalized route on
both sides of the unchanged positive leg-B body and retains the exact
loopback-source/mark-1 outcome with no skip.

The original STEP 13-05 crafter may now rerun only the complete 98-body
`cgroup-default` gate. The 46-body dataplane receipt remains current and must
not be repeated. A passing, zero-skip, exactly reconciled 98-body receipt is
still required before the clean actual three-package normal gate. Neither
pending gate is claimed by this approval.

# APPROVED — ITERATION 12

---

# Review iteration 13 — verified automatic dummy-device cleanup

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 13 |
| Review date | 2026-10-28 |
| Scope | Remove only the fresh scripted guest's source-proven automatic `dummy0` before any selected body in the two existing STEP 13-04 cgroup guests; preserve the strict STEP 13-05 only-loopback audit |
| Prior disposition | Iteration 12's one-separator route normalization remains approved; the complete 46-body dataplane receipt remains passed and current |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews fixture-owned guest boot state. The strict only-
loopback leg-B audit is unchanged and correctly detected the unexpected
device. The remediation proves the device's origin and inert state before
deleting it; it does not teach the audit to ignore an uplink or delete an
unknown interface.

Only this review artifact was appended. No production source, acceptance
body, assertion, public API, typed error, scenario, model, lane, selectable
group, guest executor, module/tool payload, DES event, execution log or commit
was changed by the reviewer. No Metal, native, broad, kernel-mutating or
network-mutating command was run during this source review.

## Iteration 13 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| Updated `deliver/distill-13-05-native-support.md` | `73a16446717b5043799542e3d984c77e848e62faa1b18d2fb0eacac5d2f6d397` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `4800d75722a1c5b78a69cf7db5a217710c1f4d32eecce194799114e237cd7598` |
| `overdrive-testing/bin/guest_peer.rs` | `ea3de0694b0164c7b8c84b3a08db2d6f275b72b252e29b10ef8eed31fb6cf8ea` — unchanged provenance source |
| `overdrive-testing/src/guest_kit.rs` | `9061cd378171fb864d4bc5fcfa51305604cf15dcd35af759f8305dc78f1dc54e` — unchanged staging source |
| Canonical bounded `increment-j/src/main.rs` | `30a9e3c1a9fd41b56bb8a85a32da480e97aa9f00caea73af5ace092a41bc84a1` |
| Causal complete-98 attempt raw | `8102440e235c80d6a48ea3045b767dde60287626b80597763815f4481cbdb287` |
| Bounded automatic-device proof raw | `8ebdb62d5b18396bda26dd67f66b5715b62c5ef9e3625009c8ba122dbd16da7c` |
| Source-package receipt | `594319921ab964e170384c145065b66edbdcb0ec9772a3ebc15ef3ae2d0d9c41` |
| Iteration 12 review before this append | `351a16e8d30729097d8b9457c305b443b0945926c0d8ac8ca087206ae0579b31` |

The staging source, prepared artifacts, native manifests, selected bodies,
counts, confinement and 122-identity overlap ledger are unchanged.

## Causal complete-98 replay evidence

Durable Metal `ff82cf33b0ddeb9b2b00fc50` attempted the complete 98-body
`cgroup-default` gate after iteration 12 approval. It passed the 20-body
control-plane native suffix and then the worker retained and STEP 13-04
bodies, including the exact named temporary-route cleanup. The first STEP
13-05 leg-B audit then retained the approved route-localnet and GW-LOCAL
readbacks but rejected the complete interface set:

```text
STEP13-05 unexpected node uplink/interface set:
/sys/class/net/dummy0 /sys/class/net/lo
```

The durable run exited 101 before selecting the leg-B body. This demonstrates
that the strict only-loopback complement works. The result is not evidence for
relaxing that complement and cannot count as a complete 98-body pass.

## Source-backed device provenance

The device predates every selected body and is created by unchanged GuestKit
boot machinery:

1. `GuestKit::image` stages the qualified stock `drivers/net/dummy` module at
   `DUMMY_MODULE_GUEST_PATH` in each scripted image.
2. Scripted-guest PID 1 mounts the minimal filesystems, brings loopback up and
   iterates its fixed optional-module list before opening the host control
   connection. When the staged dummy path exists, it calls the existing
   `load_module` helper.
3. That helper invokes `finit_module` with an empty parameter string. Linux
   7.0 `drivers/net/dummy.c` declares `static int numdummies = 1`, registers it
   as `module_param(numdummies, int, 0)`, and creates `dummy%d` once for each
   default instance. Permission zero intentionally exposes no runtime
   `/sys/module/dummy/parameters/numdummies` file.
4. The runner's first command loads its remaining explicit module closure and
   executes only `true`. The new cleanup is the next command, before device
   chmod, cgroup/bpffs setup, aliases, or any selected acceptance body.

The disposable guest is fresh, no test command has run, and no earlier actor
can have created or configured a link. The exact `dummy0` plus `lo` first-
observable inventory, the loaded module, the absent permission-zero parameter
file and the source default jointly establish fixture ownership. The bounded
receipt independently records this first-command state.

## Fail-closed ownership and state checks

`DUMMY_AUTO_DEVICE_CLEANUP` runs only in the existing control-plane and worker
`cgroup-default` guests whose `stage_step13_04_nft` support already stages the
dummy module. Before mutation it requires every item below:

| Proof | Required value |
| --- | --- |
| Module identity | `/sys/module/dummy` exists |
| Parameter visibility | `/sys/module/dummy/parameters/numdummies` absent, matching permission 0 |
| Complete interface inventory | Exactly `/sys/class/net/dummy0` and `/sys/class/net/lo` |
| Link kind | `ip -d -o link` reports dummy kind |
| Operational state | `down` |
| Master | Absent |
| IPv4 addresses | Empty |
| IPv6 addresses | Empty |
| IPv4 routes in all tables | Empty for `dummy0` |
| IPv6 routes in all tables | Empty for `dummy0` |

Only after every check succeeds does the fixture issue
`ip link del dev dummy0`. It then requires the complete interface set to be
exactly `lo` and verifies that the dummy module remains loaded.

An extra interface, different name or kind, configured state, master,
address, route, absent module or unexpected parameter file fails before the
delete. The command is not best-effort and has no `|| true`; deletion or
postcondition failure stops the group. This excludes blind cleanup of a
foreign or live interface.

The support uses only the already-staged `cat`, `ip` and shell. It adds no
module, executable, module parameter, public selector or production hook.

## STEP 13-04 lifecycle remains distinct

Removing the automatic device leaves the dummy module loaded. The existing
STEP 13-04 fixture later creates its explicitly named `ovd-gk-route` dummy,
assigns only `192.0.2.1/32`, installs its metric-42760 default route around the
two host-output bodies, and reads the expected destination/source path.

Its existing cleanup removes that exact named link and route and proves the
non-gateway target is unroutable before the remote/no-default and STEP 13-05
families. The new early cleanup neither deletes `ovd-gk-route` nor changes the
named fixture's setup/readback/cleanup scripts. Keeping the module loaded is
what permits this existing named lifecycle to continue.

The STEP 13-05 audit itself is untouched: it still demands only `lo` before
and after every leg-B body, alongside the exact route, route-localnet, empty
main/table-100 and no-fwmark-1 complement. This remediation removes the
proven boot artifact rather than weakening the accepted no-uplink oracle.

## Bounded provenance, lifecycle and unchanged-body proof

Canonical GuestKit proof `bec78c8640bd312348e34623` exited zero. Its first
guest observation, before any test body, recorded:

```text
module=loaded runtime_numdummies_file=absent
interfaces=/sys/class/net/dummy0 /sys/class/net/lo
```

Before deletion it captured the dummy-kind `ip -d` identity, `operstate=down`,
no master, and empty IPv4/IPv6 address and all-table route sets. After deletion
it recorded only `lo` with the module still loaded and emitted
`DUMMY_AUTO_DEVICE_PROVEN_AND_REMOVED`.

The same bounded guest then exercised the unchanged STEP 13-04 named sink:

- created `ovd-gk-route` through the still-loaded module;
- installed the exact address and metric-42760 default;
- read a route through that link with source `192.0.2.1`;
- removed the named route and link; and
- proved the target unroutable and the interface inventory back to only `lo`.

It next ran the complete STEP 13-05 audit before and after the unchanged exact
body
`integration::vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c`.
Both audits retained `route_localnet=0:0:0`, exact GW-LOCAL normalization,
empty main/table 100, no mark-1 policy rule, rest-prefix unroutability and only
loopback.

The original body reported one pass, zero failed and zero ignored. Its socket
readback retained a client source of `127.0.0.1`, fwmark 1 and the exact
guest-prefix leg-B destination. No recognized runtime-skip diagnostic appears,
and the receipt ends:

```text
DUMMY_AUTO_DEVICE_CLEANUP_LEGB_PASS
OVERDRIVE_METAL_RUN_COMPLETE run_id=bec78c8640bd312348e34623 exit_code=0
```

This proof covers device provenance, deletion safety, retained named-dummy
functionality and the unchanged strict leg-B path. It selects one leg-B body,
so it does not replace the complete 98-body gate.

## Preservation and accounting

Iteration 13 changes no scenario, production entry point, body, assertion,
expected count or runtime-skip rule. In particular:

- STEP 13-04's named dummy/default route and exact TCP-reset/UDP-EPERM/remote
  drop/leg-C oracles remain unchanged;
- STEP 13-05's only-loopback audit is not relaxed;
- exact mark 1 and loopback source bind remain required together by the
  positive body, with the mark-only and bind-only controls unchanged;
- iteration 12's one-separator strict route normalization and iteration 11's
  created/borrowed gateway bookkeeping remain unchanged; and
- no production API, route protocol, scenario, model, group, lane or executor
  is added.

The approved boundary and ledger remain:

| Boundary | Status / requirement |
| --- | ---: |
| Complete `mtls-relay` native gate | **46 passed**, retained; no rerun |
| Retained `cgroup-default` subtotal | 78 |
| Control-plane native suffix | 20 |
| Complete `cgroup-default` gate | 98, still pending |
| Complete normal/native overlap ledger | 122 identities |

Cloud Hypervisor uid/gid 4200, supplementary device groups, 0710 traversal,
device modes, Landlock, seccomp, `no_new_privs`, private cgroup2/bpffs mounts
and guest-only kernel effects remain unchanged. All supplied proof mutations
occur inside the disposable Cloud Hypervisor guest.

## Acceptance-review disposition

| Gate | Result | Assessment |
| --- | --- | --- |
| Causal failure | PASS as RED evidence | Actual complete-98 attempt passed prior families and named-route cleanup; strict leg-B audit exposed `dummy0` plus `lo`. |
| Origin proof | PASS | Fresh scripted PID 1 loads the staged module with empty parameters before host commands; pinned source default creates one `dummy%d`. |
| Ownership proof | PASS | First-observable complete inventory and inert state identify the sole automatic fixture device. |
| Fail-closed deletion | PASS | Every identity/state/address/route check precedes deletion; any mismatch stops before mutation. |
| Cleanup postcondition | PASS | Exactly `lo` remains and the dummy module remains loaded. |
| STEP 13-04 path | PASS | Exact named sink/address/default/readback/cleanup cycle succeeds after automatic-device deletion. |
| STEP 13-05 audit | PASS | Only-loopback and all route/policy complements remain unchanged before and after. |
| Existing body | PASS | Exact unchanged positive body reaches leg C with loopback source and mark 1. |
| Bounded evidence | PASS | Full provenance/lifecycle, one exact pass, zero failures/ignores/skips, exit zero. |
| Product/design surface | PASS | Private guest preparation only; no product API, behavior, scenario, model, group, lane or executor. |
| Native accounting | PASS | 46/98/122 boundaries remain distinct and unchanged. |

The remediation changes only disposable-fixture preparation. It adds no
acceptance partition or observable product behavior. Contract Shape, C1-C7
taxonomy coverage and the canonical 15-item completeness checklist remain
unchanged and complete under the checklist's N/A rule.

## Iteration 13 mechanical verification

| Check | Result |
| --- | --- |
| Supplied runner/handoff hashes | PASS — exact requested identities. |
| Causal raw hash | PASS — `8102440e235c80d6a48ea3045b767dde60287626b80597763815f4481cbdb287`. |
| Causal ordering | PASS as RED — control-plane 20, worker retained and STEP 13-04 named cleanup passed before the unchanged audit rejected `dummy0`. |
| Guest PID-1 source | PASS — staged optional dummy loaded before control connection using existing empty-parameter module helper. |
| Pinned dummy source | PASS — default one, permission-zero parameter and `dummy%d` creation loop match the observed device. |
| Cleanup placement | PASS — both existing `stage_step13_04_nft` guests, before every selected body. |
| Cleanup preconditions | PASS — exact inventory/kind/down/no-master/no-address/no-route checks precede mutation. |
| Cleanup postconditions | PASS — only `lo`; dummy module remains loaded. |
| No blind foreign cleanup | PASS — every differing or configured state exits before `ip link del`. |
| Selected-source dependency audit | PASS — no control-plane/worker test body refers to or requires boot-owned `dummy0`. |
| Bounded raw hash | PASS — `8ebdb62d5b18396bda26dd67f66b5715b62c5ef9e3625009c8ba122dbd16da7c`. |
| Bounded named route | PASS — exact existing STEP 13-04 create/readback/remove cycle retained. |
| Bounded body result | PASS — exact existing body 1 passed, 0 failed, 0 ignored; no runtime skip. |
| Bounded production oracle | PASS — client source `127.0.0.1`, fwmark 1, guest-prefix leg-C destination. |
| Source package | PASS — Metal `8a6a73dc0b9bdc2ff03eea1c`, runner and bounded harness check/Clippy; receipt SHA `594319921ab964e170384c145065b66edbdcb0ec9772a3ebc15ef3ae2d0d9c41`. |
| `cargo fmt --check` | PASS — independently rerun source-only. |
| `git diff --check` | PASS — independently rerun source-only. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun source-only. |
| Complete 46-body dataplane gate | PASS — retained `8066a14e0a5afc0f0c4919fd`; no rerun. |
| Complete 98-body cgroup retry | NOT RUN — required next. |
| Clean normal 1,224-body gate | NOT RUN — required only after the complete 98-body pass. |
| Production/API/scenario/body-count/lane/group/model/executor/DES/commit/mutation changes by reviewer | NONE. |

## Iteration 13 verdict

**APPROVED.** The remediation preserves the strict no-uplink audit and removes
only the automatic, inert `dummy0` whose ownership is established by the fresh
guest lifecycle, unchanged PID-1 loader, pinned module source, exact first-
observable inventory and complete link/address/route readback. Any different
or configured interface state fails before deletion.

The dummy module remains loaded, and bounded evidence proves the original STEP
13-04 named sink lifecycle plus the full pre/post STEP 13-05 audit around the
unchanged mark-1/loopback-source positive body. No product or acceptance
contract changed.

The original STEP 13-05 crafter may now rerun only the complete 98-body
`cgroup-default` gate. The 46-body dataplane receipt remains current and must
not be repeated. A passing, zero-skip, exactly reconciled 98-body receipt is
still required before the clean actual three-package normal gate. Neither
pending gate is claimed by this approval.

# APPROVED — ITERATION 13
