# Independent review: shared-memory/vsock host kernel TCP/UDP probe

## Review metadata and verdict

- Date: 2026-10-04.
- Wave: NW-SPIKE PROBE, iteration 1.
- Reviewer: independent Codex reviewer, GPT 6.1 Sol, extra-high reasoning, as explicitly selected for this bounded non-DELIVER review.
- Reviewed report: [shm-vsock-kernel-proxy-findings.md](shm-vsock-kernel-proxy-findings.md).
- Reviewed sources and receipts: [scratch README](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/README.md), final metal increments i/k, their original native captures, retained executable archive, source manifests, kernel reference captures, launchers, and cleanup readback.
- Verdict: **APPROVED for the stated host kernel forwarding feasibility and 16,384 live UDP transport/adapter population evidence.** Blocking findings: **0**.

This approves a bounded SPIKE result, not a production implementation, a DESIGN promotion, or permission to resume DELIVER 08-04. Ordinary guest socket transparency, incoming host TCP connection initiation toward a guest listener, Cloud Hypervisor integration, guest kTLS, SSH, eBPF forwarding, and production retirement guarantees remain unvalidated. The roadmap remains pending and DELIVER 08-04 remains blocked.

## Governing contract and review method

The review applied `AGENTS.md`, `.claude/rules/spike.md`, and the bounded independent-review instructions from the installed software-crafter-reviewer role. Its review-dimensions, TDD-review-enforcement, and TDD-methodology skills were read; production TDD/DES gates were treated as inapplicable to this throwaway PROBE. The user authorized minimal loadable C kernel socket glue. The module is not C eBPF. The user's explicit prohibition on Lima overrides the repository SPIKE rule's default Lima execution instruction: final WORKS and population claims must use the qualified metal fixture.

The question is whether real stock vhost-vsock shared split virtqueues can carry application payloads to a host kernel TCP/UDP adapter without TAPs, a userspace application forwarding relay, or a Linux patch. Synthetic virtqueue peers and ordinary TCP/UDP source/sink actors are permitted endpoints. A cohort of 16,384 fully booted guest operating systems, a combined #303/#306 test, and additional SSH/BPF experiments are outside this question.

This review used artifact analysis only. It did not launch a VM, perform a new native experiment, load a module, edit production/design/roadmap/DES files, or create a commit. Pre-existing dirty work was preserved. Only this review artifact was written.

## Executed topology and provenance

The load-bearing population evidence is **metal increment i**, with a private QEMU/KVM host-role VM running stock Ubuntu **7.0.0-29-generic**. It is neither a Lima run nor an experiment executing in the physical host kernel. The original i capture records canonical lease acquisition, the x86_64/KVM preflight, matching installed kernel-header build, actual KVM detection during boot, Q35 hardware, four present CPUs plus 60 possible hotplug CPUs, and the private kernel's release/default limits. See [original i launch](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-i/evidence/native-launch.log), lines 10–19, 21–50, 100–193, 303–307, and 532–555.

The executed i runner's source manifest pins the QEMU invocation to `-accel kvm`, `-m 16384`, `-smp 4,maxcpus=64`, `-nodefaults`, the copied stock kernel, and a probe-only initramfs. It checks availability for 16GiB private RAM plus an 8GiB outer-host reserve; it does not change a host sysctl. The only population limit change is the inner harness's `RLIMIT_NOFILE=131072`. See [i runner](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-i/run.py), lines 20–60 and 100–121, and [i harness](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-i/src/main.rs), line 85. Stock private limits recorded in i are PID max 65,536 and threads max 115,140. Possible CPU topology is part of this measured configuration, not a claim about an arbitrary four-CPU host.

Metal k is a separate, small functional run with 1GiB private RAM, the same stock kernel, four online/64 possible CPUs, and recorded native QEMU command/result. Its stock threads max is 3,967, reflecting that smaller VM. It does not repeat or establish the population claim. See [k command](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-k/evidence/qemu-command.json), [k result](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-k/evidence/native-result.json), and [k serial](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-k/evidence/native-serial.log), lines 426–447.

The read-only Linux reference is revision `502d45774af09f1c681c754c4b7cdfb5d7f72fd9`, a **7.2 development tree**. I independently checked that all 15 captured files match both their declared SHA256 and the files at `/Users/marcus/git/linux`, and checked that tree's HEAD. That reference was not compiled or patched by the runner. The reference manifest and stock BPF capability readback retain an earlier historical 7.0.0-34 runtime label; they are not receipts for the final 7.0.0-29 metal run. Final runtime identity comes from i/k native receipts.

### Independent integrity checks

| Check performed during review | Result |
|---|---|
| Recompute all six i source hashes against its original prelaunch manifest | All match. |
| Recompute all six k source hashes against its original prelaunch manifest | All match. |
| Hash original ignored i out archive | `9ce703db16577ac0c35e28d31182c8f155d277f191bd5b6cc4c570b4e4e9546e`, matches disclosure. |
| Hash original ignored k native evidence archive | `19db25c62bc0b8daa3d775a20edea986557226bd959704ca59acb8df3ab92c58`, matches recorded retrieval. |
| Hash i archive kernel and Image | Both `b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682`; x86 bzImage header present. No transformation was applied on this x86 run. |
| Hash i archive module | `736801736b5ee5468e0f6c190474d84c56afb07cc047d536003122367be36df5`, matches private init's original printed hash. |
| Hash i archive Rust binary | `3e5c566b32a498c10c019632739733dbafda1d84a2936fe525cd4c4027682a7f`, matches private init's original printed hash. |
| Compare archive i module source and init | Module source equals pinned source; init equals explicitly extracted `init.executed.from-archive`. |
| Hash retained i stock vsock/vhost/virtio transport modules | All inspected payload hashes match the explicitly derived artifact manifest. |
| Compare all original i JSON lines with `events.from-original-launch.json` | All 24 objects match exactly: 23 harness events plus the original launcher result. |
| Compare k archive serial, executable pins, and QEMU command with saved native receipts | Equal. All five recorded build/control command exits are 0. |
| Parse i/k runner and supporting Python scripts without execution | Syntax valid. |
| Hash executed physical-host cleanup readback script | `50c09d4bb648100d2f9a73a1a076440ec908c92b18298fcb3698b932db76157b`, matches receipt. |
| Check build/archive ignore boundary | Original i/k archives are ignored under scratch `out/`. |

These are post-run verification results, not newly manufactured runtime attestations.

### Receipt-loss disposition

The original i launcher retrieved the ignored build/rootfs archive instead of the increment's diagnostic evidence directory. Subsequent k synchronization overwrote the remote i diagnostic JSON. I inspected [the executed i launcher](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-i/evidence/launch.executed.py), especially line 20, and the corrected current launcher, which now retrieves only evidence and preserves local launcher/source logs. The reports disclose the failure.

The complete original i stdout/native serial, original prelaunch source manifest, live population audit events, and exact executed kernel/module/binary/rootfs payload survive. The derived artifact manifest and extracted serial/events are explicitly labeled. The missing original i command-result JSON, QEMU-command/result JSON, admission-budget JSON, and outer-QEMU resource samples are not treated as original receipts. In particular, **no original i outer-QEMU CPU utilization measurement is available**.

The surviving original launcher exit is 0. With the pinned runner's return condition, this supports the inference that QEMU returned 0 and both completion/unload markers passed; it is not a surviving original QEMU-result JSON. Actual KVM boot, private memory/default limits, traffic, drain, and normal power-down are visible in the original serial. The essential feasibility/population result is independently supported without the missing CPU samples. A new full population run is not required to supply an optional CPU metric that the findings do not claim.

The k local launcher-log overwrite is also disclosed, in `launcher-result.recorded.json`. Its native serial, commands, pins, QEMU result/command, and resource samples remain intact in the original retrieved archive. The recorded launcher result is distinguished from the native receipts.

## Data-path and owner verification

The real forwarding path is synthetic virtio endpoint memory → stock vhost-vsock → kernel AF_VSOCK stream → scratch kernel module → ordinary kernel TCP/UDP socket → ordinary endpoint, with the reverse path through the same kernel adapter.

[The i Rust driver](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-i/src/main.rs), lines 12–26, opens a distinct `/dev/vhost-vsock`, allocates a 256KiB mapping, installs its own vhost memory table, configures two actual split queues and independent kick/call eventfds, sets the CID, and asserts successful `VHOST_VSOCK_SET_RUNNING`. Lines 34–45 post actual virtio-vsock headers/descriptors, await used indices, validate destination CID/port, consume the returned buffer, and send credit updates. There is no userspace AF_VSOCK socket or alternate plain ring that could bypass the stock backend. The distinct contexts are independently mapped synthetic guest buffers, not 16,384 booted guest address spaces.

[The kernel module](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-i/module/shmproxy.c), lines 138–144 and 122–131, creates a kernel AF_VSOCK listener at CID 2:29500 and accepts real streams. Lines 74–92 create a separate kernel TCP/UDP socket for each valid session and connect or bind it. Lines 39–53, 55–72, and 93–108 perform all cross-protocol payload receive/send work with actual `kernel_recvmsg`/`kernel_sendmsg`. The accepted vsock socket and ordinary socket belong to the module's session. No production crate is linked or edited by this standalone crate; its Cargo manifest has its own `[workspace]` and only libc/serde_json dependencies.

The Rust TCP echo actor reads/writes its own ordinary TCP endpoint. UDP echo actors read/write their own ordinary UDP endpoint. The synthetic peer produces and consumes its own endpoint frames. Those actors do not relay between an ordinary socket and vsock. Python only builds, launches and records control/evidence. QEMU supplies the isolated host-role kernel's hardware; the topology contains no QEMU userspace vsock application backend. Initial/final private link lists show only `lo`. The C code is a loaded GPL kernel module, not a BPF program; no Linux patch/fork, TAP, nftables change, or userspace cross-protocol payload relay is present.

The retained 7.2 stock source separately corroborates CID lookup/duplicate rejection, owner/access checks when starting vhost queues, and vhost's virtio transport callbacks (`drivers/vhost/vsock.c`, lines 74–104, 611–690, 817–852). Runtime success on stock 7.0.0-29 establishes the executed backend/socket API behavior; source reasoning on 7.2 is not substituted for that execution.

## Functional evidence and assertion honesty

| Behavior | Source and original evidence | Assessment |
|---|---|---|
| TCP 262,144 bytes each direction | i harness lines 62–71; original i launch lines 557–558 | One module-initiated ordinary TCP connection, with byte-exact payload comparison in both directions after independent wire/TCP read chunking. The extra 16 reverse bytes are the asserted `after-half-close` tail. |
| TCP half-close | Module lines 97, 110–115; harness line 70 | TCP write shutdown occurs while the separate reverse worker drains. The ordinary TCP endpoint emits its tail after observing EOF. |
| Backpressure | i harness lines 67–69; original event line 557 | No RX descriptors are posted for 300ms; `to_peer` stays 51,733 while `send_waits` rises 3→152. Delivery subsequently resumes and compares exactly. This is an observed bounded stall/resume, not a fairness or throughput benchmark. |
| UDP outbound 0/1/1,431/59,000 bytes | i harness lines 72–75; original i launch lines 559–562 | Exact payload, frame kind, source address/port and length checks. Zero bytes are a real send/receive datagram. |
| UDP inbound 0/17/59,000/1,500 bytes from two peers | i harness lines 76–78; original i launch lines 564–567 | Exact source port/payload checks and a reply observed from the actual bound kernel UDP server port. |
| Distinct UDP source/destination IP association | k harness lines 72–78; k serial lines 452–460 | 127.0.0.2 outbound endpoint, and inbound peers 127.0.0.2/127.0.0.3, are checked together with their ports and byte-exact replies. |
| Oversized whole-datagram rejection | Module lines 61–68; harness line 80; original i/k truncation events | `MSG_TRUNC` receives the real 61,000-byte length; the module consumes the whole datagram and emits kind 30 with empty payload at the 60,000-byte bound. No truncated prefix becomes a success frame. |
| Unsupported registration and peer loss | Module lines 77–79; i harness lines 81–83; original i launch lines 568–569 | Registration kind 99 is rejected; explicit virtio peer reset drains the inbound owner, and functional session counts reconcile at accepted=closed=4. This is not exhaustive malformed-frame or production failure-path coverage. |

UDP uses a bounded scratch 12-byte framing protocol over **AF_VSOCK STREAM**, not native vsock SOCK_DGRAM. Stream headers/payloads are reassembled before ordinary UDP send. Each kernel UDP receive produces one complete frame with actual source metadata, preserving zero-length datagrams. This test-local format is not a new production API or approved wire contract. Head-of-line blocking, kernel buffer copies, prototype worker cost, and loopback IPv4 endpoint scope are accurately qualified in the findings.

The TCP mode calls `kernel_connect` toward the ordinary TCP endpoint listener. Reverse payload on that established connection is tested; host-initiated TCP toward a guest-side listener/accept path is not. The module does not implement a TCP listener registration mode in this probe. Incoming UDP datagrams to a bound kernel adapter are separately tested and do not establish incoming TCP connection initiation.

The success flags are backed by executed source assertions and real native counters, rather than only narrative fields. The unsupported-registration event prints a counter rather than asserting it in isolation; the later population assertions require `closed=4` and exact live counts, and the original receipts independently show `rejected=1`. Backpressure likewise has recorded counter evidence plus byte-exact eventual delivery. Neither is represented here as an exhaustive acceptance suite.

## The 16,384 live-owner proof

The claim is **16,384 simultaneously started stock vhost devices and kernel UDP adapters**, not 16,384 simultaneous TCP flows or guest operating systems. In i harness lines 89–98:

1. Each new peer is constructed once with CID `300000+i`, a successfully started distinct vhost FD and a new mapped memory context.
2. Its setup handshake returns the actual kernel UDP local port. FD, mapping and port uniqueness are checked while every preceding peer remains held.
3. At every stage, every peer sends a CID-and-stage-tagged payload and checks the exact reverse payload, remote endpoint address/port, and virtio destination CID/port.
4. The kernel module's `accepted−closed` must equal the stage, with `closed=4` throughout the population. `/proc/net/udp` independently contains every returned local port and exactly stage+1 distinct bindings. The extra binding is the ordinary endpoint.
5. `/proc` enumeration independently counts exactly stage input workers and stage reverse workers. The harness's task/FD counts also expose the stock vhost worker population and open owner resources.

No logical registry count, shared CID alias, shared UDP binding, expired owner, or single vsock peer can satisfy that combination. The original i launch has the final audit at line 576:

```text
actual_started_vhost_devices=16384
distinct_open_vhost_fds=16384
independent_guest_memory_contexts=16384
actual_kernel_udp_adapter_sockets=16384
independently_observed_udp_bindings=16385
module_input_kernel_workers=16384 module_reverse_kernel_workers=16384
first_cid=300000 last_cid=316383
held_process_fd_count=81925 held_process_task_count=16386
accepted=16388 closed=4 to_net=1930693 to_peer=1930709
```

The final stage finished at 112.937676243s, below the module's 300s registration idle interval. The earliest owners were included in the final CID-tagged check and the final independent live-count reconciliation, so an expired early cohort is not counted as live. I independently recomputed the cumulative payload byte total from all seven stage sizes and CID-tagged string lengths: 1,930,693 including the functional prefix, exactly matching `to_net`; `to_peer` adds the 16-byte TCP tail.

The final `pool_traffic_check_s=28.565969839` spans the serial whole-pool traffic loop **and subsequent independent audits/resource reads**. It is not an isolated datagram throughput interval. Likewise the 112.937676243s stage timer spans population creation, all preceding checks and audits. These qualifications match the findings.

Population resources are significant: approximately three kernel workers per live adapter, 81,925 inner process FDs, 16,386 inner process tasks, plus 32,768 module session/reverse workers and one acceptor. Recorded private `MemAvailable=9199764 KiB`, `SUnreclaim=5585008 KiB`, `KernelStack=788736 KiB`, and harness `VmRSS=406356 KiB` have different scopes. RSS is not kernel memory or full population cost; outer-QEMU CPU utilization is unavailable. Earlier small-VM/FD/idle-time failures remain preserved historical attempts and do not establish a stock kernel capacity ceiling.

## Cleanup evidence and retained limitation

The module's ordinary sockets are shut down/released after stopping and joining the reverse worker; module unload joins the accepted-session workers and releases the acceptor/listener. See module lines 112–120 and 147–151. The harness retires each peer, stops its backend, unmaps memory and releases owned FDs, then joins the ordinary endpoint and checks all accepted sessions closed. See harness lines 57 and 99–103.

Original i evidence records:

```text
capacity_elapsed_s=380.28041197699997 released_vhost_devices=16384
process_fds=4 process_tasks=1 accepted=16388 closed=16388
SHMPROXY_HARNESS_EXIT=0
shmproxy: drained accepted=16388 closed=16388
SHMPROXY_MODULE_UNLOAD_EXIT=0
vhost_vsock 28672 1044 - Live
SHMPROXY_COMPLETE_EXIT=0
```

The retirement interval is independently calculated as **380.28041197699997−112.937676243 = 267.342735734s**. Original close-work warnings report that `virtio_transport_close_timeout` repeatedly hogged CPU for over 10ms, with the cumulative warning count reaching 67. The later dmesg repeats those messages; it is not a second independent set of warnings.

**The 1,044 value is the stock vhost-vsock module usage/reference counter in `/proc/modules`.** It is not an independently enumerated tally of 1,044 leaked socket objects. The surviving counter and close-work warnings disallow a claim of prompt zero-reference teardown in a surviving host kernel. They do not contradict successful custom-module drain/unload. Even small k still records stock usage 3 before VM shutdown, despite draining its four accepted sessions and unloading the custom module.

The private kernel ends through normal power-down. k's original native result explicitly records QEMU exit 0 and absence of its PID. i's original launcher exit, pinned good-result branch and serial normal power-down support the same result with the provenance qualification above. Final readback records no owned QEMU processes, no `shmproxy` in the physical host kernel, and no canonical lease-owner metadata. I verified the readback script's hash and its owned-process path filter. Experimental module load/unload is confined to the private kernel; the runner does not alter foreign physical-host modules, links or sysctls.

**Disposition:** accepted as an explicitly recorded SPIKE limitation. This review does not approve fast per-allocation retirement, complete live-kernel stock reference reclamation, production lifecycle correctness, or a new teardown architecture. No additional remediation mechanism is mandated by this bounded feasibility review.

## eBPF, kTLS, SSH and integration boundaries

The supplied 7.2 reference supports a qualified capability analysis:

- `vsock_bpf_update_proto` requires a transport with `read_skb`; stock vhost's transport supplies that callback (`net/vmw_vsock/vsock_bpf.c`, lines 150–170; `drivers/vhost/vsock.c`, line 499).
- Sockmap admission includes established vsock STREAM/SEQPACKET and hashed UDP sockets, subject to protocol suitability (`net/core/sock_map.c`, lines 536–556).
- SK_SKB map/hash redirects permit an eligible vsock egress target and explicitly reject a vsock ingress target (lines 647–663 and 1253–1269).
- SK_MSG map/hash redirects reject vsock targets; non-ingress targets must be TCP (lines 675–694 and 1281–1300).
- The same socket cannot be upgraded to active kTLS with an attached psock, and psock initialization rejects an inet ULP socket (`net/tls/tls_main.c`, lines 636–645; `net/core/skmsg.c`, lines 757–760). Separate host non-TLS sockets carrying opaque guest ciphertext are a different, unvalidated composition.

These are reference-version constraints and capabilities, not an executed BPF forwarding result. The probe builds/loads no BPF program. No categorical conclusion that eBPF cannot handle vsock or that vsock requires a userspace application relay follows. Socket creation, connect/accept, lifecycle, ordinary guest adaptation and cross-protocol datagram framing still require their own mechanism. The byte-stream probe also does not prove reachability of an unmodified guest sshd. The findings preserve these boundaries and do not use the prior CH Unix backend proof or historical Lima BPF configuration as a substitute.

## Contract Shape Compliance

No new executable repository tests, Rust properties, acceptance scenarios or production interfaces were authored by this SPIKE. The Rust entry point is a standalone native probe, with real kernel/socket assertions and retained receipts. Consequently per-test Contract Shape declarations, Outcome-Value Anchor, test budget, production composition-root acceptance gates, RED→GREEN→COMMIT events and mutation score are **not applicable** to this PROBE. No DES events were invented or used to replace independent evidence. This review does not relax those gates for a later DISTILL/DELIVER implementation.

## Findings and remediation dispositions

| ID | Assessment | Disposition |
|---|---|---|
| L1 | Slow retirement, close-work CPU warnings and nonzero stock module reference counter are real native limitations. | Already disclosed; retained as evidence. No prompt live-kernel retirement approval and no mandatory new architecture in this SPIKE. |
| L2 | Original i native diagnostic JSON and outer-QEMU samples were lost; k launcher log was overwritten during retrieval. | Already disclosed; original i serial/live audits and exact executable archive independently support the bounded claim. Derived/recorded artifacts remain explicitly labeled. No CPU claim or reconstruction masquerading as original attestation. |
| L3 | The kernel API/glue result does not establish guest ordinary sockets, CH integration, kTLS, SSH, or BPF forwarding. | Already disclosed; these remain unvalidated and are excluded from the verdict. No new test lane imposed. |
| L4 | Reference manifests contain historical runtime labels distinct from final metal kernel identity. | Read as historical/reference evidence; final identity is pinned by i/k original receipts. No inference from 7.2 source to untested 7.0 BPF behavior. |

There is no concrete reproduced contradiction within the reported host proxy/UDP population claim and no unresolved essential provenance gap. Hypothetical production cancellation, generalized recovery, malformed-input hardening and architecture changes were not promoted into findings.

## Iteration record and final disposition

Iteration 1 reviewed the executed sources, native receipts, exact retained artifacts, source/binary correlation, owner-count assertions, timer/resource scopes, teardown limitations and reference-only capability claims. All necessary mechanical checks completed through read-only local analysis. No remediation was required within the authorized scope.

**APPROVED.** The original qualified metal evidence supports host kernel TCP/UDP adaptation through real stock vhost-vsock shared virtqueues, without TAPs or a userspace application payload relay, and 16,384 simultaneously live UDP transport/adapters. Approval is limited to that recorded feasibility result. No DESIGN promotion, roadmap approval, production lifecycle guarantee, or DELIVER resumption is implied.
