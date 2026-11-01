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
