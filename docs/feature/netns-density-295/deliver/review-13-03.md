# DELIVER review — STEP 13-03 guest VIP resolution and owner cgroup placement

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `13-03` |
| Review ID | `STEP13-03-DELIVER-POSTCOMMIT-01` |
| Review date | 2026-10-28 |
| Reviewer | Fresh isolated post-commit DELIVER implementation reviewer |
| Iteration | 1 |
| Implementation range | `cf7dbe50c53fa288c9f229442e65e28dfee2826f..8ff253c0f6b57e63c57fda6cabe17fd237f51326` |
| DES commit | `08e96a0b28dfd6b553ea155583d57f9be6705482` |
| Final verdict | **APPROVED** |
| Open findings | 0 |

This review applies the repository `AGENTS.md`, the applicable Rust, testing,
verification, BPF, spike, development, and workflow rules, and the installed
review dimensions. The repository's DELIVER rules require this native Markdown
artifact and take precedence over the installed review skill's legacy YAML
output and two-iteration cap.

Only this review artifact is written. No production source, test body, design,
roadmap, execution log, spike source, host state, or commit was changed. The
pre-existing modified `AGENTS.md` remains untouched.

## Authority and review boundary

The accepted contract is D24/D24a and DG-2 row 14 in
`docs/feature/netns-density-295/feature-delta.md`, ADR-0164, and roadmap
step 13-03. The relevant contract clauses are:

| Contract | Evidence |
|---|---|
| TCP `(VIP, listener port)` is classified like the service frontend; an unmatched address in the configured VIP range is `MeshUnreachable`; UDP uses the existing `connect4` rewrite | `feature-delta.md:6552-6567`; ADR-0164 Decision; `MtlsResolution` rustdoc at `crates/overdrive-core/src/traits/mtls_resolve.rs:56-85` |
| The owner is at `overdrive.slice/control-plane.slice` and refuses outside the `connect4` subtree | `feature-delta.md:5785-5789`, `:14747-14754`; `GuestNetworkError::OwnerCgroupPlacement` at `crates/overdrive-control-plane/src/guest_network.rs:369-373` |
| The owner placement and VIP flows are this step's D24/D24a criteria | `deliver/roadmap.json:1320-1349` |
| A-19/V-15 is a separate native premise: the `overdrive.slice` ancestor attachment must affect sockets in `overdrive.slice/control-plane.slice` | `feature-delta.md:915`, `:965`; roadmap implementation notes at `deliver/roadmap.json:1349` |

The accepted evidence increment is `docs/feature/netns-density-295/spike/v11-vip-v14-findings.md` and its retained source under
`spike-scratch/netns-density-295-v11-vip-v14/`. The relevant current hashes
are:

| Artifact | SHA-256 |
|---|---|
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| `deliver/roadmap.json` | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| `adr-0164-guest-vip-tcp-resolves-as-mesh-and-datagrams-use-the-connect4-rewrite.md` | `c5a697a29944e4c486928816d813f572132f9392ed8ee74d8b9ecc3417c9a1e7` |
| `spike/v11-vip-v14-findings.md` | `325a89bbe8b734639579b9814f442f54ed18cb03f7a158494ef7bd300fc2b894` |

## Commit and DES integrity

The implementation commit changes exactly three source files:

| File | Change |
|---|---:|
| `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs` | `+88 / -19` |
| `crates/overdrive-control-plane/src/guest_network/prefix_boot_acceptance.rs` | `-1` pending marker |
| `crates/overdrive-control-plane/src/guest_network/owner_acceptance/datagrams.rs` | `-1` pending marker |

`crates/overdrive-control-plane/src/lib.rs` is listed by the roadmap, but its
constructor wiring was already present at `lib.rs:3899-3909`; the commit does
not create a second composition path or alter that existing wiring. The
implementation commit preserves author `Marcus Schack Abildskov`, contains
exactly `Co-Authored-By: Codex <codex@openai.com>`, and has `Step-Id: 13-03`.
The DES-only commit has the same author, exact Codex trailer, and step trailer,
and changes only the execution log. The execution log records the required
ordered phases at `execution-log.json:1595-1613`: RED, GREEN, COMMIT, each
`EXECUTED` with `PASS`.

## Contract and implementation assessment

The implementation adds only private implementation state. `VipKey` is private
(`mtls_resolve_adapter.rs:258-274`), `BackendIndex::by_vip` is a private field
(`:328-331`), `apply_vip_row` is private (`:366-375`), and the range-aware
classifier is private (`:578-587`). The existing public constructor has the
exact approved shape at `mtls_resolve_adapter.rs:731-740`:

```text
ServiceBackendsResolve::new(store, frontend, vip_ranges)
```

The `MtlsResolve` port signature is unchanged. The existing `lib.rs` caller
passes the configured `config.vip_range.clone()` at `:3899-3909`, preserving the
single configured VIP-range input with no default or alternate constructor.

The four classification arms are implemented in the contract order:

1. Existing frontend lookup remains first and selects the first healthy backend
   by `Ord` (`mtls_resolve_adapter.rs:588-600`).
2. A known `(VIP, TCP port)` uses the same service selection, while an address
   in the configured `VipRange` with no matching service returns
   `MeshUnreachable` (`:602-616`).
3. The existing frontend-subnet fail-closed branch remains unchanged
   (`:618-624`).
4. The existing backend-address/non-mesh fall-through remains unchanged
   (`:626-629`).

Row updates and relists maintain the VIP projection in the same synchronous
index critical section as the backend projection (`:398-405`, `:857-866`). A
VIP with no healthy backend is also `MeshUnreachable` through
`first_healthy_backend_for`, so it cannot fall through to cleartext. The
production TCP caller maps that arm to the existing typed
`FlowRefusal::MeshUnreachable` at `guest_network/owner_runtime.rs:1055-1059`
and `:1117-1119`; resolver store errors remain the existing
`FlowRefusal::ResolveFailed` path. No new error variant or string-carried
semantic distinction was invented.

The datagram path is separate as required. `datagram_associate` checks the
destination, sets `owner_egress_mark` false for a VIP, calls the existing
`GuestFlowIo::connect_udp`, then checks the kernel peer before attaching
(`owner_runtime.rs:1193-1262`). The real host adapter calls the actual UDP
socket's `connect()` at `flow_io/host.rs:691-698`; the Sim adapter only models
the rewrite in memory at `flow_io/sim.rs:2450-2474`. The production cancellation
and cleanup order is bounded: cancellation before or during route/open/connect
returns through the existing outcome, aborts an opened peer, and attach refusal
closes or aborts both halves (`owner_runtime.rs:1214-1255`, `:1261-1278`).

Owner placement is checked before the forwarder probe. `HostGuestFlowIo` reads
`/proc/self/cgroup` into `GuestHostFacts` (`flow_io/host.rs:325-340`), and
`probe_guest_flow_startup` returns the existing typed
`OwnerCgroupPlacement { owner, attach_point }` before `probe_forwarder`
(`guest_network.rs:823-865`). The Sim acceptance test exercises the same
production owner path and checks both refusal and the valid child path at
`prefix_boot_acceptance.rs:439-468`.

## Test and TDD integrity

The diff removes exactly seven pre-authored pending markers and does not edit
test bodies:

- five D24a resolver markers at `mtls_resolve_adapter.rs:1127`, `:1154`,
  `:1174`, `:1202`, and `:1234`;
- the D24 placement marker at `prefix_boot_acceptance.rs:443`; and
- the D24 datagram marker at `owner_acceptance/datagrams.rs:164`.

The activated tests retain their existing `CONTRACT_SHAPE` declarations and
drive the production adapter/owner over the existing Sim observation and flow
ports. No assertion was weakened, no scenario was authored, no new public test
seam was added, and no production binary is spawned by these Rust tests. The
crafter handoff reports 29 targeted tests passing plus workspace check,
Clippy with `-D warnings`, and formatting on Metal. The repository currently
contains no `.context/netns-density-295/13-03/` receipt directory; the generic
DES phase entries above are the only persisted step-specific phase record in
this checkout. That missing durable receipt is secondary to the native gate
finding below.

The known quiescence seed `0x651125aa` remains the unchanged 13-01 baseline
limitation and is unrelated to this step; it was not used as evidence for this
review. No mutation testing was run, consistent with the repository rule that
mutation testing is a single final DELIVER-wave gate.

## Typed error and caller mapping gate

| Designed condition | Implemented representation | Caller/observable outcome |
|---|---|---|
| Known service VIP with a healthy backend | `MtlsResolution::Mesh(ResolvedBackend)` | `tcp_connect` registers the selected backend and connects to leg-F (`owner_runtime.rs:1061-1095`) |
| Known service VIP with no healthy backend | `MtlsResolution::MeshUnreachable` | `FlowRefusal::MeshUnreachable`, no socket connection (`:1117-1119`) |
| Unknown address in configured VIP ranges | `MtlsResolution::MeshUnreachable` | Same fail-closed refusal, never `NonMesh` (`mtls_resolve_adapter.rs:602-616`; `:1117-1119`) |
| Destination outside VIP range and existing mesh projections | Existing frontend-subnet/backend-address arms | Existing `Mesh`, `MeshUnreachable`, or `NonMesh` behavior (`:618-629`) |
| Owner outside attach subtree | Existing `GuestNetworkError::OwnerCgroupPlacement { owner, attach_point }` | Boot probe returns before forwarder/serving effects (`guest_network.rs:823-865`) |
| VIP datagram | No mTLS resolution; existing connected UDP `connect4` path | Unmarked peer socket, kernel-peer policy check, then attach (`owner_runtime.rs:1213-1262`) |

The accepted contract names the three `MtlsResolution` arms and the existing
owner-placement error; the implementation preserves those exact
representations. No separate `AmbiguousVip` public variant was invented.

## Findings

### `issue (blocking):` A-19/V-15 ancestor-cgroup native proof is absent

The step cannot be approved until a retained native Linux 7.0 receipt proves
the exact production premise that the `connect4` program attached at the
`overdrive.slice` ancestor affects sockets opened by the
`overdrive.slice/control-plane.slice` owner. This is a verification gate, not
a claim that the Rust implementation is defective.

The existing spike explicitly does not prove this premise. Its VIP verdict
says the mirror was applied to the owner's **own** host sockets
(`spike/v11-vip-v14-findings.md:338-359`), and its not-proven list explicitly
includes `connect4` inherited from an ancestor cgroup
(`:460-469`). The retained E-vip evidence proves TCP/UDP rewrite, guest-visible
VIP, `getpeername`/`recvfrom`, registered tuple from the kernel peer, and
300/300 stress on Linux `7.0.0-29-generic`; it does not place the owner in
`control-plane.slice` below an attachment at `overdrive.slice`.

The activated Sim cells do not close this gap. The placement cell changes only
the simulated `GuestHostFacts.owner_cgroup` and checks the typed boot refusal
(`prefix_boot_acceptance.rs:444-467`). The datagram cell installs an in-memory
`SimGuestFlowIo` rewrite (`owner_acceptance/datagrams.rs:165-194`; the Sim
rewrite is `flow_io/sim.rs:970-972` and `:2462-2474`). Neither exercises a
kernel cgroup attachment or a real `connect4` hook. A Sim boot check therefore
cannot establish ancestor inheritance, and the old own-cgroup spike cannot be
relabelled as V-15.

The production reachability path is concrete and bounded. `run_server` wires
the real resolver and configured VIP range (`lib.rs:3899-3909`); the owner
reads `/proc/self/cgroup` and checks placement before its forwarder probe
(`flow_io/host.rs:325-340`; `guest_network.rs:823-865`); a guest datagram
enters `datagram_associate` and calls the real `HostGuestFlowIo::connect_udp`
(`owner_runtime.rs:1193-1262`; `flow_io/host.rs:691-698`). Cancellation and
failure paths already abort/close the peer and close the guest half, so no
new lifecycle or retry mechanism is required to verify this premise. The
missing fact is solely the native observation of ancestor attachment effect.

The repository search found no retained `13-03` native receipt, no V-15/A-19
native runner output, and no source/raw capture that places the attach at
`overdrive.slice` while the owner runs in `control-plane.slice`. The crafter's
reported 29 targeted PASS results and workspace check/Clippy/fmt PASS do not
contain that observation. This is therefore an open, bounded evidence gap.

Required disposition: the original STEP 13-03 crafter/root must first provide
an existing receipt if one exists. If none exists, rerun only the already
authorized native V-15 proof through the canonical durable `cargo xtask metal
run --` / same-run collection path, retaining the standalone Aya Rust source,
raw output, kernel version, actual owner/ancestor cgroup paths, registered
`getpeername()` tuple, guest-visible TCP/UDP result, and cleanup evidence. The
probe must remain in `spike-scratch` or use the existing disposable
Cloud-Hypervisor runner; it must not add production APIs, tests, scenarios,
lanes, models, or mechanisms, and it must not mutate the physical host
cgroup/module/management-network state. A receipt proving the exact ancestor
effect closes this finding; a failed native result would invoke the native
falsification gate and return the affected design/roadmap premise to DESIGN.

**Iteration 1 disposition: OPEN.** This finding blocked approval and blocked
starting STEP 13-04. It did not authorize production changes or contract
weakening. Iteration 2 below records the bounded native remediation and closes
the finding.

## Review dimensions

| Dimension | Result | Evidence |
|---|---|---|
| Exact API shape | PASS | Exact three-argument `ServiceBackendsResolve::new`; unchanged `MtlsResolve` and existing error variants; new projection state is private. |
| Typed semantic errors | PASS | VIP unknown/unhealthy cases map to existing `MeshUnreachable`; caller maps it to existing `FlowRefusal::MeshUnreachable`; owner placement uses existing typed payload. |
| Classification behavior | PASS for source-local contract | VIP branch precedes frontend-subnet/backend fall-through; configured range is consulted; non-VIP behavior is preserved; UDP bypasses mTLS resolution. |
| Owner lifecycle and cleanup | PASS by source inspection | Real production path's cancellation, abort, close, attach, and guest-half cleanup are traced above. |
| Test integrity | PASS | Seven marker-only activations; no test body edits, assertion weakening, fixture theater, or new seam. |
| Native/Tier-3 evidence | **BLOCKED** | A-19/V-15 ancestor inheritance is explicitly unproven by the old spike and absent from retained 13-03 receipts. |
| Scope / implementation bias | PASS | No API, architecture, persistence, retry, model, lane, or unrelated hardening was added. |
| DES phase order | PASS | Persisted RED → GREEN → COMMIT entries all report `EXECUTED/PASS`. |

## Iteration history and verdict

### Iteration 1 — 2026-10-28

The fresh post-commit review covered the complete implementation diff, exact
D24/D24a and ADR-0164 contracts, typed error/caller projections, marker-only
test activation, DES phase history, existing VIP spike evidence, and the
production owner path. The implementation and source-local tests match the
approved API and fail-closed classification contract. The required native
A-19/V-15 ancestor-cgroup proof is missing and remains open.

**Verdict: NEEDS_REVISION.** Obtain or execute the bounded exact native receipt
and return this same step to the original crafter for remediation and this
reviewer for re-review. Do not begin STEP 13-04.

# NEEDS_REVISION

## Iteration 2 — native A-19/V-15 remediation review

### Remediation metadata

| Field | Value |
|---|---|
| Remediation commits | `a62d774fce546508d7a497556bf085de8ae831ef`, `84b7bc646d6480bb3a7e96024a8f38b6c91097bd` |
| Remediation scope | Standalone Aya Rust probe, disposable Cloud Hypervisor evidence, raw V-15 findings, source and artifact manifests |
| Production/API/test/DES changes | None |
| Durable Metal run | `OVERDRIVE_METAL_RUN_ID=8493f2bedb4b40c8c0618762`, terminal `exit_code=0`, `cause=command_returned` |
| Native findings artifact | `docs/feature/netns-density-295/spike/v15-ancestor-connect4-findings.md` |
| Canonical raw evidence | `spike-scratch/netns-density-295-ancestor-connect4/increment-f/evidence/` |
| Final verdict | **APPROVED** |

The remediation commits preserve author `Marcus Schack Abildskov`, contain
exactly `Co-Authored-By: Codex <codex@openai.com>`, and carry `Step-Id: 13-03`.
The reviewed range from the prior step commit to `84b7bc646` contains only the
new standalone spike and its findings/evidence; it does not alter production
source, executable tests, roadmap, DES log, APIs, lanes, or the accepted
mechanism.

### Native boundary and execution authenticity

The proof is a real disposable Cloud Hypervisor run, not a host-side cgroup
simulation and not a Sim boot check. The retained Metal run log records:

| Evidence | Receipt |
|---|---|
| Native host preflight | `run.log:1-7`: x86_64, `systemd-detect-virt=none`, usable `/dev/kvm`, AMD EPYC 8024P |
| Build | `run.log:8-27`: vendored Cloud Hypervisor, Aya eBPF, and vfwd builds all return 0; Cloud Hypervisor v53.0.0; `aya-ebpf` probe source is Rust |
| Disposable guest launch | `run.log:28-31`: Cloud Hypervisor launch with `--vsock cid=37136,backend=vhost-kernel`, guest kernel boot, separate guest serial readback |
| Guest litmus | `run.log:52-55`, `guest-commands.jsonl:1-2`: guest executes `/vfwd ancestor-vip --obj /vfwd-bpf.o`, returns `rc=0` and `ok=true` |
| Run completion | `run.log:67`: `OVERDRIVE_METAL_RUN_COMPLETE ... exit_code=0 cause=command_returned` |

The host preflight and guest kernel are distinguished explicitly. The Metal
host readback is `7.0.0-38-generic` (`run.log:3`); the disposable guest
readback is also `7.0.0-38-generic` (`run.log:31`, `guest-commands.jsonl:1`).
This is the pinned Linux 7.0 kernel line, while the older owner-only spike's
7.0.0-29 result remains historical evidence. The proof does not confuse
`systemd-detect-virt=none` on the host with the guest substrate: the Cloud
Hypervisor launch and guest serial output are retained separately.

The guest image is an actual CH boot boundary. `run.log:30` contains the
complete Cloud Hypervisor command, including the guest kernel, initramfs, CID,
and `backend=vhost-kernel`; `vm/serial.log` records guest boot and network
initialization; `vm/ch.log` records the Cloud Hypervisor virtio-vsock device.
The guest command itself creates and uses the cgroup hierarchy and the BPF
attachment, so the observed hook is in the guest kernel rather than the
physical Metal host.

### Exact A-19/V-15 premise

The standalone Aya source creates the required hierarchy and performs the
operations in this order. `vfwd/src/owner.rs:850-920`:

1. creates `/sys/fs/cgroup/overdrive.slice`, its
   `/control-plane.slice` child, and a sibling outside cgroup;
2. moves the litmus process into `control-plane.slice` and reads back
   `0::/overdrive.slice/control-plane.slice`;
3. loads the `vip_connect4` `cgroup_sock_addr(connect4)` program from the
   Aya ELF and attaches it to the ancestor `/sys/fs/cgroup/overdrive.slice`;
4. registers the TCP and UDP `(VIP, port, protocol) → backend` entries; and
5. creates the client sockets only after the child placement and ancestor
   attachment.

The source's actual VIP program is retained at
`bpf/src/main.rs:764-782`. Its complete file SHA-256 is identical to the
working V-11 VIP probe: `8fb5c3cba241811509bf5201641867fce043a65eb1d7a0d62e4c838feb88cf13`.
The program uses the same host-order `(VIP, port, IANA protocol)` key and
backend rewrite as the prior working `vip_connect4`; the new probe changes
the attachment location and cgroup placement, not the kernel mechanism.

The raw result at `increment-f/evidence/results.json:41-77` and its complete
litmus line at `guest-commands.jsonl:2` establish:

| Premise/control | Observed result |
|---|---|
| Ancestor attachment | `bpftool cgroup show` at `/sys/fs/cgroup/overdrive.slice` lists `cgroup_inet4_connect`, `multi`, `vip_connect4`; program id 8, verified instruction count 38 |
| Owner placement | `owner=/sys/fs/cgroup/overdrive.slice/control-plane.slice`; `owner_cgroup=0::/overdrive.slice/control-plane.slice` |
| TCP mapped VIP | `tcp_getpeername=127.0.0.1:37083`, equal to the TCP backend; TCP echo completed |
| UDP mapped VIP | `udp_getpeername=127.0.0.1:55521`, equal to the UDP backend; UDP echo completed |
| Non-VIP control | `direct_getpeername=127.0.0.1:37083`, equal to the direct backend |
| Unmapped VIP | `unmapped_vip_error=Network is unreachable (os error 101)` |
| Mapped VIP outside subtree | Process moved to `/sys/fs/cgroup/ancestor-connect4-outside-124`; `outside_mapped_vip_error=Network is unreachable (os error 101)` |
| Overall litmus | `controls.*` all true and `ok=true` |

The standalone command therefore proves the exact missing fact: a program
attached at the `overdrive.slice` ancestor rewrites TCP and UDP sockets opened
by a process in `control-plane.slice`, while an equivalent socket outside that
subtree does not receive the rewrite. It also proves direct non-VIP behavior
and unmapped-VIP refusal, without substituting a Sim model for the kernel
effect.

### Artifact identity and retained history

The source and output identity is independently retained in
`increment-f/evidence/source-manifest.json:1-29` and
`artifact-pins.json`. The source manifest pins the exact standalone source
hashes, vendored Cloud Hypervisor head
`f252bb20b0d141d59e4c15a9ac09c5c2dd7c350d`, the exact canonical command
(`cargo xtask metal run -- python3 .../increment-f/run.py`), and the
superproject head `08e96a0b28dfd6b553ea155583d57f9be6705482`.

The findings document records the prior increments honestly: increment-a's
initial vfwd compile failure, increments b-e's corrected fixture attempts,
and increment-f as the canonical successful capture. The earlier attempts are
retained in the committed `increment-{a,b,c,d,e}` source/evidence history;
they are not replaced with a narrated green result.

Hashes of the decisive retained artifacts are:

| Artifact | SHA-256 |
|---|---|
| `spike/v15-ancestor-connect4-findings.md` | `427d07b647d9701012fc3e75dbe48ef00ad80dcf5c6b20729ec94ba597695eb7` |
| `increment-f/evidence/results.json` | `d7cff306fa37550987d31a9717ae2300cfb0d8ee2e7c504ba69e95a23269a36b` |
| `increment-f/evidence/run.log` | `716fc79f270e48608126d0dbb5af517293af4f3df997ede2b941882923e5098f` |
| `increment-f/evidence/guest-commands.jsonl` | `0c55e205e4329293bc60f94bc43a7223d799028f327079ef33b220509548161a` |
| `increment-f/evidence/source-manifest.json` | `3edd24dab056a883e0b06b43e21d13a5eaa5955e7135885363624ac6b21ff5d4` |
| `increment-f/evidence/artifact-pins.json` | `278b0507e3c364ca0c6244320bbba238b9d9215f380807a06bebd830522bd855` |

### Cleanup and physical-host boundary

`results.json:5-24` reports `links_unchanged`, `routes_unchanged`,
`netns_unchanged`, `lo_tcx_unchanged`, `root_cgroup_bpf_unchanged`,
`modules_restored`, `modules_diff=[]`, `bpf_stats_enabled=["0","0"]`, and
`spike_cgroups_left=[]`. The retained `inventory-before.json` and
`inventory-after.json` are byte-identical (both SHA-256
`0005c2ecb55e8247df1d1f39e5973fa8a049f774cc9532dc3107540b3b06bf4b`).
`commands.jsonl` records the host baseline and post-run readback; after the
guest exits, `pgrep` reports no Cloud Hypervisor/vfwd process and the host
`bpftool cgroup show /sys/fs/cgroup` contains only the pre-existing
`sysctl_monitor` entry (`commands.jsonl` post-run records). The physical host
was used only for the canonical Metal runner and preflight; the probe's
cgroup, BPF attach, sockets, and VIP map are inside the disposable guest.

This satisfies the remediation constraints: no physical host cgroup or module
mutation, no management-network change, no production dependency on the
spike, and no new production/test API, scenario, lane, model, or mechanism.

### F1 disposition

**CLOSED.** The exact A-19/V-15 native premise is now proven through the
required disposable Cloud Hypervisor boundary on the Linux 7.0 kernel line.
The prior review's prohibition remains satisfied: the old owner-only spike is
not used as proof, and the Sim boot check is not used as proof. The new
receipt is a standalone native evidence increment with retained source/raw
output and cleanup readback.

No unchanged 29-test suite, full package union, formal 279-check run, mutation
run, or other broad rerun was necessary for this finding: the remediation
changes only evidence artifacts and independently closes the one native proof
gap. The accepted existing quiescence baseline limitation remains unchanged.

### Iteration 2 verdict — 2026-10-28

The original crafter's remediation closes the sole blocking finding from
iteration 1. The exact ancestor path, child owner path, attach type, actual
TCP/UDP backend tuples, non-VIP and refusal controls, CH guest boundary,
kernel readbacks, source/artifact pins, honest prior attempts, and physical
host cleanup are all retained and independently inspected.

**Verdict: APPROVED.** STEP 13-03 satisfies D24/D24a and the A-19/V-15 native
verification gate. The roadmap may advance to STEP 13-04. Any later change to
the approved production contract or mechanism would require its own design and
review; no such change is part of this approval.

# APPROVED
