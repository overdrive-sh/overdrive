# Guest-networking website content review

## Scope and verdict

**APPROVED.** Independent review of content commit `075fc9adb`, from its parent through that commit. No blocking factual, editorial, navigation, discovery, or change-induced validation defect was found. This approval covers the website content only. It does not finalize the feature, resolve historical execution-log gaps, or authorize deployment or publication.

Reviewed on 2026-11-01. The review applies `AGENTS.md`, `.claude/rules/finalize.md`, `.claude/rules/domain-naming.md`, `website/CLAUDE.md`, and `.claude/skills/technical-writer/SKILL.md`. The user requested an engineering journey, so historical narrative is appropriate in the new blog. Product documentation must describe current behavior. The website's independent Bun/Next/Fumadocs checks apply; Rust and native-kernel execution are outside this review.

The complete eleven-file content scope is:

- `website/content/blog/networking-without-a-host-interface-per-vm.mdx`.
- `website/content/docs/concepts/guest-networking.mdx` and `concepts/meta.json`.
- The architecture, microVMs, transparent-mTLS, and dial-by-name concept pages.
- The comparisons page, deploy guide, CLI reference, and documentation introduction.

Only this review artifact was edited. The pre-existing dirty `AGENTS.md`, branch, authored content, source, tests, policy, dependencies, lockfile, and execution log were preserved. No commit, push, deployment, native test, VM, BPF, cgroup, nftables, Lima, or Metal operation was performed.

## Factual evidence

The feature delta, relevant accepted architecture decisions, final delivery-slice review iteration, native correctness review, canonical-suite review, editorial ledger, and selected production paths support the following content claims:

| Claim and content location | Evidence and review result |
|---|---|
| VM attachment uses shared node resources and guest-neutral vsock flow transport; process namespace/TPROXY capture remains separate. Blog opening and flow-boundary section; guest-networking introduction; architecture networking section. | Feature delta's current attachment and host-owner contracts, ADR-0207, and ADR-0098's distinct process cleanup responsibility support the distinction. The prose confines the absence of per-VM host namespace/veth/TAP/bridge to VM attachment and explicitly retains reusable bridge/TAP APIs. |
| Host sockets are separate from guest TCP packets. Blog flow-boundary section; guest-networking socket-boundary section; transparent-mTLS capture section. | ADR-0207's request/frame/end protocol and the host owner/forwarder contract support the exact boundary. No claim carries guest TCP behavior directly into host vsock transport. |
| Kernel forwarding preserves backpressure through peek, nonblocking send, and consumption of accepted bytes, with per-CPU work rather than one thread per flow. Blog backpressure section; guest-networking forwarder section. | ADR-0197 and `kmod/ovd_fwd/ovd_fwd.c:1393`, `:1540` support the mechanism. The historical sockmap failure is bounded to the tested kernel rather than generalized into a universal Linux limit or current performance result. |
| The forwarder refuses TCP ULP/kTLS sockets; host mTLS owns separate TLS endpoints, with temporary userspace relay for guest delivery. Blog backpressure section; transparent-mTLS record-processing section; architecture networking section. | ADR-0200, `kmod/ovd_fwd/ovd_fwd.c:646`, and `crates/overdrive-dataplane/src/mtls/inbound.rs:37` support the boundary. Process-path blocking splice is supported by `mtls/outbound.rs:5` and its production pump. The prose does not claim that every VM mesh byte avoids userspace copies or that process splice has no prelude handling. |
| Fast Open correlation uses a seventh, handshake-only C/CO-RE fentry program paired with Rust/Aya SockOps, both loaded through Aya 0.13.1. Blog handshake section; guest-networking capture section. | Feature delta's TCP correlation replacement contract, the native candidate spike, and `crates/overdrive-bpf/src/tcp_correlation_candidate.bpf.c:113` support the selected program and candidate-before-validation behavior. The blog correctly distinguishes the earlier handwritten dynptr relocation failure from the selected Aya-loaded program. |
| Temporary map staging is removed before program loading/attachment and READY; runtime objects remain process-owned. Same capture sections. | `crates/overdrive-init/src/transport/kernel.rs:429` creates the two loader objects, shares the maps through temporary pins, and removes staging before returning to program load/attachment. The prose does not describe permanent runtime staging pins or crash adoption. |
| Node and allocation flow bounds are admission limits, not measured node capacity; pending/half-closed flows retain places, and unread ends count during audits. Guest-networking capacity section; blog ownership section. | ADR-0187, ADR-0208, ADR-0209; `crates/overdrive-core/src/guest_transport/wire.rs:792`; host owner admission/dial paths; and the corrected shared audit support 65,536 node places, 4,096 ordinary allocation flows, separate probe allowance, and release timing. ADR-0199 and ADR-0202 distinguish context release from ordinary end-record delivery. |
| Queued-end accounting has exact seeded parent-fail/restored-pass evidence through the production owner and supervisor. Blog ownership section. | The native correctness review's approved remediation iteration retains identical seed/action trace and awaited cleanup, with false recovery against the parent and no recovery after the correction. This is accurately narrower than a general simulation proof. |
| Guest EXIT is sent and its beacon closed before poweroff; host report draining prevents successful VMM exit from substituting for workload success. Blog ownership section; microVM exit section. | `crates/overdrive-init/src/main.rs:187`, `:658`, and `crates/overdrive-worker/src/vm_driver.rs:2159` support the report, one-second linger, and host drain. The blog explicitly identifies the second as shutdown handling, not measured boot time. |
| Formal results and native results prove different things; disposable production CloudHypervisorVmm/GuestKit and nested fixtures provide the kernel boundary. Blog testing section. | Final delivery-slice approval and the native correctness review preserve that isolation and evidence distinction. The 279 outcomes are stated as 209 holds and 70 intentional violations against models, not universal running-system proofs. |
| Two final 64-VM correctness bodies establish addresses, TCP/UDP node-source behavior, refusal, fail-closed behavior, reclaim, and CID reuse, without benchmark credit. Blog testing section; guest-networking limits. | The approved native correctness review records the two exact bodies, final source/staged executable identity, clean results, and cleanup. The post does not combine them with the preceding canonical 119/46 suites into one current-source run. No cold-start, throughput, density, memory, or latency measurement is asserted. |
| Operator requirements and current limits remain explicit. Guest-networking prerequisites/limits; microVM artifact section; deploy guide; CLI startup reference. | ADR-0205/0206, current feature delta, VM-service acceptance, and delivery-slice approval support host-image module loading, matching guest kernel/module/init, kernel vhost-vsock, RESET-EXIT source integration, job/service specs, and reflink rootfs requirements. No fork release, tag, prebuilt download, invented TOML port key, or arbitrary-kernel support is advertised. |

The content preserves the material exclusions: VM-backend Service-VIP UDP, non-Linux guest adaptation, node-port TOML loading, CID generation replacement crash window, performance measurements, and hardware-offload feasibility. Guest DNS and remote UDP, including empty datagrams, remain distinct from the deferred Service-VIP path. The post and docs do not claim global finalization or production GA.

## Editorial assessment

The blog opens with its thesis: a VM needs network access without necessarily needing a host interface. Each later section explains a concrete engineering consequence: receiver backpressure, exact accepted-socket identity, ownership while completion is unread, honest workload exit, and disposable native evidence. The conclusion moves to the remaining measurement work. This satisfies the requested journey without inventing dates, quotations, performance wins, or internal approval machinery.

The concept page supplies current mechanisms, owners, prerequisites, capacity semantics, and limitations. The related docs distinguish process capture from VM attachment and host enforcement from guest adaptation. The new prose uses stable responsibility names rather than delivery coordinates. Historical explanation stays in the blog; product docs state present behavior. No unrelated site rewrite is required for this review.

## Validation and practical limits

The editorial ledger records frozen dependency installation, typecheck, lint, Next build, and OpenNext build passing with the existing lockfile. Those full builds were not redundantly rerun by this reviewer. The existing build artifacts were available and exercised directly.

Independent reviewer checks:

| Check | Result |
|---|---|
| Complete content diff and `git diff --check 075fc9adb^ 075fc9adb` | PASS; eleven-file content scope, no whitespace errors. |
| Existing one-index verifier against the retained prerendered artifact | PASS: 25 published pages, including five blogs, reachable from Markdown export, llms export, and search; one draft excluded. |
| New blog metadata and discovery | PASS: required title/description/date/author present; schema default includes the post; production `/blog` links it. |
| Concept navigation | PASS: `concepts/meta.json` includes `guest-networking`; introduction and related pages link the concept. |
| Local content links and heading anchors | PASS: 57 links checked across all ten changed/new MDX files; existing category routes excluded from file-path matching. |
| Production Next HTTP smoke on local port 8996 | PASS: blog listing, new blog HTML/Markdown, new concept HTML/Markdown, and `llms.txt` returned 200; both new URLs occur in the llms export. |
| Focused search discovery | PASS: `socket backpressure` reaches the new concept; `handshake candidate` reaches the new blog through `/api/search`. |
| Existing runtime blog script against retained current OpenNext output, local port 8994 | Reproduced the known ranking assertion failure after blog rendering, search/MCP join, Markdown identity, and draft-exclusion checks passed. |
| Identical runtime script against the prepared pristine baseline website export, local port 8995 | Reproduced the same assertion: `deterministic simulation` ranks workload-identity-lifecycle ahead of the expected deterministic-simulation-testing page. The baseline contains neither new networking page. |

Runtime reproduction logs are `/tmp/website-review-current-blog.log` and `/tmp/website-review-baseline-blog.log`. Both runs exited 1 at the same assertion and stopped their local workers. The production Next smoke server was also stopped. Runtime scripts used the task-local `bunx` shim already documented in the ledger, backed by installed Bun 1.4.4 canary; repository tooling was unchanged.

The author also records `test:mcp` and `test:mcp:analytics` stopping at that same shared ranking assertion. Their assertions and control flow were inspected; they were not rerun here. Analytics checks after the assertion remain unexecuted and are not claimed as passing. The baseline reproduction supports treating this failure as pre-existing, not a networking-content blocker. The successful index and targeted discovery checks show no additional observed indexing or discovery regression from this content change; they are not a universal ranking guarantee.

External issue URLs were checked as source references in repository evidence, not fetched over the network. This review does not re-execute native receipts or assert a new kernel/performance result.

## Findings and disposition

No material content finding requires remediation. The pre-existing search-ranking failure remains an accurately recorded validation limitation and does not require a search implementation, fixture, or test-expectation change within this task.

## Final verdict

**APPROVED — website guest-networking documentation and engineering journey.** The current content is supported by the reviewed implementation and bounded evidence, preserves operator limitations, and passes the targeted navigation, links, export, and discovery checks. Publication and the feature's global archive/integrity gate remain separate decisions.

## Review iteration 2 — motivating constraint and alternatives

### Scope and prior review omission

The user identified a material omission in the initially approved post: it did not explain the native bridge failure that motivated replacement or the successful Open vSwitch alternative. I missed that omission. My first review checked the implementation explanation and evidence qualifications but did not establish that the engineering journey explained the actual decision. Its broad narrative approval was insufficient. The earlier review remains above as history, and this iteration supplies the missing decision-completeness assessment.

This iteration reviews the revised blog and editorial ledger, the new decision-story requirement in `.claude/rules/finalize.md`, and the punctuation requirement in `.claude/skills/technical-writer/SKILL.md`. Product docs are unchanged and retain their prior bounded approval. Only this review artifact was edited by the reviewer. The user-owned `AGENTS.md`, writer's blog revision, and root-owned policy/skill changes were preserved.

Reviewed blog SHA-256: `ffb234eb13860c9b4b372892d40b786b52bf1ee50ad4d572dc0f019233c55d4f`.

### Independent decision and evidence assessment

The revised post was read in full against the feature delta opening, accepted ADR-0145, the native bridge falsification record and its approved documentary review, the multi-bridge findings, OVS findings and independent review, current-hardware vDPA/SR-IOV findings, and the two distinct vsock transport-probe records. The prior website approval was not treated as factual authority for these additions.

| Revised claim and location | Source evidence and assessment |
|---|---|
| Opening: the native attempt could not provision the 16,384-attachment measurement target and returned `EXFULL` while attaching a TAP to its bridge. | `native-e18-bridge-capacity-falsification.md`, completed native evidence, records the exact `TapAttachBridge` / `set-master` netlink `-54` refusal, failed measurement, successful cleanup, and initially empty bridge. The feature delta opening and ADR-0145 identify this as the motivating topology failure. The post correctly calls 16,384 a measurement target rather than an achieved VM population. |
| Opening: 1,024 bridge port-number slots with zero reserved means 1,023 usable ports on one bridge, while the native failure ordinal was not recorded. | The falsification record's kernel interpretation and its independent review distinguish upstream Linux 7.0 allocator facts, synchronized serial source-path inference, absent at-failure inventory, and unavailable benchmark executable attestation. The post retains the decisive source-versus-observation distinction. It does not claim a directly counted 1,024th failure or a universal Linux interface ceiling. |
| Opening: the older project-defined 4,096 `NetSlot` bound is a different constraint. | ADR-0145 context and the hardware feasibility findings explicitly separate the earlier allocation bound from the subsequent single-bridge port limit. The post does not merge them into one kernel capacity result. |
| Alternatives section: five leaves and one root retained 4,097 TAPs and queue descriptors, and two real guests among 1,024 TAPs passed the stated TCP/DNS journeys. | `spike/multi-bridge-findings.md`, verdict, fanout, and traffic sections establish those separate experiments. The post does not convert the 4,097 attachment objects into 4,097 booted guests or full production security composition. |
| Alternatives section: the untuned 1,100-TAP ARP burst first missed source 342 after 341 successes, with 341 neighbors on each of two leaves and the root. | The multi-bridge neighbor-pressure record contains those counts and increasing `table_fulls`. The post confines the result to that tested configuration, avoids a universal 341-VM limit, and names the remaining neighbor/security ownership work. |
| Alternatives section: stock kernel OVS held 16,384 TAP devices, queue descriptors, and NETDEV vports, with all-port ARP/ICMP and 8,192 pairs of byte-identical synthetic TCP frames. | `spike/ovs-kernel-findings.md` and `spike/review-ovs-kernel.md` approve exactly this attachment and frame-preservation boundary. Retained increment-d independent-audit events also show 16,384 distinct ifindices, owner FD identities, and kernel NETDEV vports, with OUTPUT-only flows and zero packet EXECUTE requests. The post correctly credits a successful OVS experiment and excludes booted VMs, workload TCP sessions, physical wire speed, and complete production identity/lifecycle proof. |
| Alternatives section: OVS remained unselected because it retained TAP-based per-endpoint host objects, not because its experiment failed. | ADR-0145's alternatives explicitly reject multi-bridge for keeping the per-VM netdevice cost and OVS by the TAP-based constraint. Its selected decision removes per-allocation host netdevices. This is a recorded selection criterion, not a rationale invented from the final implementation. |
| Alternatives section: 2.699 seconds of OVS datapath deletion and 99.548 seconds of TAP descriptor closure expose a measured cleanup cost. | The final OVS findings' resources/cleanup section records those two durations. The independent review accepts the bounded comparison and retained cleanup evidence. The post labels them observations from that run and expressly declines a delivered-vsock comparison or general teardown guarantee. It does not claim these timings were the sole historical selection criterion. |
| Hardware paragraph: the tested current NIC/firmware configuration supplied no usable hardware vDPA/SR-IOV endpoint. | `spike/findings-vdpa-sriov-metal.md` distinguishes available software support from absent usable hardware endpoints and records the read-only inventory boundary. The post preserves that machine/configuration scope, leaves feasibility deferred to the existing issue, and does not condemn either technology or propose hardware reconfiguration. |
| Vsock section: the stock CH Unix-muxer device result and separate kernel vhost adapter result each reached 16,384 at their own transport boundary. | `spike/shared-memory-vsock-scale-findings.md` counts activated stock device/muxer owners with tagged traffic and zero guest boots. `spike/shm-vsock-kernel-proxy-findings.md` separately counts kernel vhost transports/adapters with tagged datagrams, without a resident guest cohort or completed production lifecycle. The post keeps the two results separate and does not use the stock muxer result to prove the selected kernel backend. |
| Vsock/backpressure sections: removing per-VM host netdevices retains device/CID/socket/task capacity and adds guest adaptation, protocol, VMM integration, and kernel module obligations. The threaded proxy was left unselected for per-peer worker cost. | ADR-0145's decision, alternatives, and consequences support these tradeoffs. The kernel-proxy findings explicitly count per-peer module workers in addition to stock vhost tasks. ADR-0197 remains the accepted event-driven replacement for the falsified sockmap approach. The post does not claim zero resource growth or a measured density/startup benefit. |

The earlier reviewed implementation discoveries remain present: stalled-reader backpressure, exact Fast Open accepted-socket correlation, queued-end ownership, honest guest EXIT reporting, and disposable native testing. The delivered two-body 64-VM correctness evidence stays separate from the 16,384 attachment/transport experiments and from future performance measurements.

This supplies the consequential decision story required by the updated policy: observed constraint, alternatives that worked or remained limited, the actual no-per-VM-netdevice selection criterion, costs shifted to transport and flow ownership, discoveries that changed the mechanism, and bounded delivered evidence. It neither recounts every delivery task nor claims that every rejected alternative was nonfunctional.

### Policy, punctuation, and validation

The updated finalization policy is consistent with the requested bounded gate. It requires the feature blog to read its evidence and explain the concrete problem, serious alternatives, actual selection criteria, costs, and proof limits. Its instruction to select consequential experiments avoids a requirement to reproduce every historical task. It preserves independent review and the separation between content completion and publication. No policy change or expansion into a whole-site rewrite is required by this review.

The technical-writer punctuation requirement explicitly applies to authored prose, headings, summaries, descriptions, examples, and revisions, while preserving punctuation required by code, data, or exact quotations. Independent scans found zero semicolons in both the complete revised blog, including metadata and diagram, and the updated skill itself. The revised post complies without relying on an exception. Existing unrelated website prose and the immutable earlier review were not rewritten.

Independent checks in this iteration:

- `git diff --check`: PASS.
- Existing one-index verifier against the current retained build: PASS, with 25 published pages, five blogs, and one excluded draft.
- Retained production blog HTML contains the revised `EXFULL`, Open vSwitch, and scoped cleanup text. Concept navigation remains present.
- Full revised blog and skill punctuation scans: PASS, zero semicolons.
- The blog retains its concept link and established issue links, adding only the already documented hardware-feasibility issue. No navigation or route change was introduced.

The updated editorial ledger records passing typecheck, production Next build through `assert:one-index`, revised blog HTML/Markdown smoke, blog listing and `Open vSwitch` search discovery, links, punctuation, and domain-name checks. Those builds and HTTP checks were not redundantly rerun here. The retained HTML was inspected directly. The old native failed-attempt raw directory is not available at its linked local `target/` path in this checkout, so this review relies on its preserved factual record and independent documentary review rather than claiming a fresh raw-data attestation or a native rerun.

The pre-existing runtime search-ranking failure remains qualified exactly as in iteration 1. It was not rerun. Analytics assertions beyond that failure remain unexecuted and no full runtime-suite pass is claimed. No new test scenario, search fix, native run, Rust check, VM, Cloudflare operation, commit, push, deployment, or publication occurred.

### Finding and remediation disposition

| Finding | Prior state | Current disposition |
|---|---|---|
| Missing motivating bridge failure and consequential alternatives, especially successful OVS | Material omission in the originally approved engineering journey, identified by the user and missed by this reviewer | **CLOSED.** The revised opening and alternatives explain the proved constraint, distinguish source-derived limits from observed counts, credit successful alternatives, and ground the no-TAP selection in ADR-0145. |

No new blocking content or policy finding was identified. No production or architecture remediation is prescribed.

### Iteration 2 final verdict

**APPROVED — revised guest-networking engineering journey and unchanged product documentation.** The revision closes the material narrative omission with source-grounded decision criteria and honest experimental boundaries. This is the current content verdict. It does not erase the prior review miss, finalize the feature's global archive, or authorize publication.
