# Independent review: Aya Rust zero-length UDP over vhost-vsock

## Review metadata

- Feature: `netns-density-295`.
- Date: 2026-10-04.
- Reviewer: independent Codex reviewer.
- Iterations: 1 and 2.
- Scope: the standalone zero-length UDP SPIKE and its native receipts, especially executed increment f. This is a mechanism review, not production networking or DESIGN approval.
- Latest verdict: **APPROVED** at iteration 2. R1 is closed by the documentary correction recorded below. The source and original native receipts establish **WORKS at the stated functional boundary**.

Reviewed the findings, README, attempt index, executed Rust controller and Aya BPF source, runner and archived launcher, Cargo manifests and locks, original native serial and launch captures, executed source/artifact manifests, retained archive contents, resource observations, and cleanup readback. Loaded the local reviewer definition and its three review skills, repository `AGENTS.md`, and `.claude/rules/spike.md`. The user's explicit metal/private-VM instruction supersedes the spike rule's default Lima execution instruction.

## Functional conclusion

The measured mechanism carries ordinary UDP application payloads, including genuine zero-length messages, through two independently started stock vhost-vsock devices and their shared virtqueues. Aya SK_SKB prepends an eight-byte private frame, so the socket redirection leg carries a nonempty message. In the reverse direction, Aya TCX removes that framing at the packet boundary and reconstructs the UDP header. An ordinary UDP application receives an actual empty datagram.

The synthetic guest actor owns its logical endpoint messages and private codec. It is not an ordinary guest UDP stack. This qualification is essential and is present in the findings. The proof establishes the mechanism on the executed stock `7.0.0-29-generic` private host-role VM; it does not establish production guest transparency, UDP scale, throughput, zero-copy, or the full UDP maximum length.

## Findings and disposition

### R1 — Correct the claims about archived retrieval receipts

**Blocking artifact correction; no functional mechanism defect.**

[Findings](aya-zero-udp-findings.md) lines 164–170 say the c/e retrieval failure explicitly records that the remote directory was absent and that private raw archive hashes are in the launch captures. Those receipts are absent from the retained `increment-*/evidence/native-launch.log` files. A read-only search of the retained `.log` and `.json` artifacts found no `retrieved` or `private_raw_archive_sha256` receipt.

The actual [launcher](../../../../spike-scratch/netns-density-295-aya-zero-udp/launch.py) closes the launch capture after recording the launcher return code at line 22, then performs retrieval at lines 23–38. Its retrieval success/hash and failure messages go to console output outside that capture. The c/e captures end with their lease-acquisition failure and launcher return code; they contain no captured retrieval error. The launcher source explains the discrepancy directly.

**Required remediation:** revise those findings sentences to identify the evidence that is actually retained. Either name an existing retained console capture containing those receipts, or state that the launcher prints them outside the retained launch log and remove the unsupported c/e retrieval-error claim. Do not fabricate or append a historical launcher receipt. No rerun or source change is necessary.

The local ignored private archives a/b/d/f do exist. Independently reading the b/d/f original archive members established that their native serial captures are byte-for-byte identical to the retained serial captures. Thus R1 does not cast doubt on the native functional observations or require reopening the mechanism choice. The reviewer records the available archive hashes below to distinguish current readback from an original launcher receipt.

| Private original archive | SHA256 from this review's readback |
|---|---|
| a | `60359d92a61d1c83dc321eebe68661caa50bd9eab3d9869f5e1f571382556662` |
| b | `f5deb23122ddb405c6b6e491c85545e107d61c3130fbd03b3d264e1904998000` |
| d | `8f28bbc9de57584e734ebe75a87774a063c041f9d6bab3be6b07cbe24489da86` |
| f | `253237d0ae93623f77f46ad59fdbfbef7a4eafe3747e80ee04ee945f6b6dd002` |

Iteration 1 disposition: open, limited to documentary accuracy. No production, design, roadmap, DES, test, or probe-source remediation was requested. Iteration 2 closes this finding below.

### Coverage precision — mapping removal

The recorded negatives remove a guest-to-host **FLOWS tuple** and a host-to-guest **ROUTES cookie route**. They do not remove a target `SOCKETS` SockHash entry during traffic. Counter 11, the target-redirection failure counter, remains zero. The findings accurately name tuple and cookie-route removal; retain that wording. A generalized claim that native SockHash target-entry removal was exercised would exceed the evidence. This is a coverage limit, not a required additional experiment for the requested zero-length proof.

## Source and boundary verification

### Aya BPF owns the forwarding transform

[BPF source](../../../../spike-scratch/netns-density-295-aya-zero-udp/increment-f/bpf/src/main.rs) lines 14–28 perform the cookie lookup, private prefix insertion on app-to-guest messages, magic/declared-length validation on guest-to-app messages, skb linearization, and SockHash egress redirection. Encoding runs for every owned message, including zero. The native zero cases exercise the SK_SKB callback with an original length of zero; this is not inferred from a compile result.

At lines 31–55, TCX constrains the transform to the configured IPv4 addresses/server port and registered complete tuple. It leaves app-to-proxy UDP packets unchanged. For reverse traffic it saves the UDP header, shrinks eight bytes at the network-header boundary, restores the UDP header over the private prefix, updates IP/UDP lengths and IPv4 checksum, then reads 42 actual transformed bytes with `bpf_skb_load_bytes`. No retained packet pointer is reused across the mutating helper. The native verifier accepts both programs.

The IPv4 packet remains nonempty at this boundary: an empty application datagram still has its IP and UDP headers. This explains why the mechanism avoids the direct zero-length socket-send limitation. The retained 7.2 `filter.c` distinguishes the SK_SKB prefix-growth helper from the TC network-header helper; it is a reference, while the executed 7.0 native observations remain decisive.

### No userspace proxy payload relay

[Controller source](../../../../spike-scratch/netns-density-295-aya-zero-udp/increment-f/src/main.rs) creates/listens/accepts/connects proxy sockets and registers their FDs/cookies. Payload sends and receives occur on the ordinary application sockets or in the synthetic guest actor's own virtqueue memory. The controller never reads a mapped proxy socket and writes its payload to the other proxy socket. Its native `controller_payload_reads_writes: 0` field is a literal diagnostic, so the conclusion relies on inspecting the complete controller source, not on that field alone.

In particular, lines 387–388 independently exercise both directions: the forward guest message is decoded from a completed native RX message and compared with the application's bytes; the guest then sends its own encoded message through the TX queue, after which the ordinary UDP application receives it. A zero-sized expected vector cannot create a guest receipt: `message` first requires a received `OP_RW` frame ending in EOM, and `decode` requires a valid eight-byte prefix.

### Actual stock vhost device and memory context

`Peer::new` opens `/dev/vhost-vsock`, sets owner/features, registers its actual mmap as vhost memory, configures RX/TX descriptor/avail/used addresses and kick/call eventfds, sets the CID, and starts the device (controller lines 69–142). The guest actor submits descriptors, waits for kernel used-ring progression, and reads completed buffers (lines 171–230). This is not an AF_VSOCK loopback substitute.

The two original owner receipts at native serial lines 504 and 515 show different vhost FDs (19 and 27), memory bases (`0x759723963000` and `0x759723923000`), CIDs (295300 and 295301), and app addresses. The accepted AF_VSOCK listener uses CID 2, port 29500, and `SOCK_SEQPACKET`. Actual 44-byte virtio headers, descriptor heads, used heads and avail/used progression are retained. Read-only re-parsing checked header CIDs/ports/type/operation, `OP_RW` EOM, wire length, and the completed RX used length `44 + wire_len` for all 16 positive messages, including collision cases. Credit-update `OP=6` frames were excluded from message counts.

## Original native receipt reconciliation

Re-derived the observations directly from [native serial](../../../../spike-scratch/netns-density-295-aya-zero-udp/increment-f/evidence/native-serial.log), independently of `receipt-summary.derived.json` and the summary counters:

| Owner | Actual application tuple | Payload sequence in each direction | Real guest empty message receipts | Ordinary UDP empty receives |
|---|---|---|---:|---:|
| CID 295300 | `127.0.0.2:36271` ↔ `127.0.0.1:49468` | `0,0,1,0,1431,59000,0` | 4 | 4 |
| CID 295301 | `127.0.0.3:52321` ↔ `127.0.0.1:49468` | `0,0,1,0,1431,59000,0` | 4 | 4 |

The eight actual empty guest RX message events are at native serial lines 538, 588, 688, 838, 893, 943, 1043 and 1193. Each has `OP_RW=5`, EOM=1, wire length 8 and the received `ZUD1`/zero-length prefix. They correspond to actual used-ring completions and are separate from control packets.

The eight ordinary UDP empty receives are at lines 563, 613, 713, 863, 918, 968, 1068 and 1218. Each records `POLLIN`, return length zero and actual source `127.0.0.1:49468`. Controller line 388 always allocates a 65,536-byte receive buffer before `recv_from`, including the zero cases. Following 1-, 1431- and 59,000-byte receives compare the complete bytes with independently generated endpoint data, and each owner ends with no ghost readiness.

For all 14 sequence roundtrips, independently decoded the raw 42-byte packet witnesses: IPv4 version/IHL and UDP protocol, actual tuple, IPv4/UDP lengths, skb length and IPv4 one's-complement checksum. The empty reverse packets have IP length 28, UDP length 8 and skb length 42. Witness sequence numbers strictly increase, so the receipts are not stale expected metadata. The capture code reads actual bytes after mutation. Both large cases record one 59,008-byte EOM vhost message and one complete 59,000-byte UDP receive. The two marker-collision cases preserve the original eight-byte prefix-shaped application data using a distinct outer logical length of eight.

The probe deliberately uses IPv4 `SO_NO_CHECK`; the actual UDP checksum is zero. This proves the stated checksum configuration, not general checksum/offload behavior. `recv_from` exposes no `MSG_TRUNC` receipt; none is claimed.

## Negative observations

- Invalid magic and declared-length mismatch produce no app readiness and increment the SK_SKB invalid-frame counter to 1 and 2 (serial lines 1250 and 1273).
- Removed reverse tuple produces no app readiness and increments the TCX tuple-miss counter to 1 (line 1297).
- Removed forward cookie route produces no `OP_RW` guest message and increments the SK_SKB route-miss counter to 1; received credit controls are explicitly distinguished (line 1328).
- The separate foreign ordinary UDP empty datagram is received unchanged from `127.0.0.4:46428` on `127.0.0.1:34665` (line 1226). This is the exercised foreign case, not exhaustive foreign-flow coverage.

These outcomes are driven through the actual probe endpoint paths. No hypothetical production cancellation, security, lifecycle or architecture defect is promoted into a review finding.

## Native loads, preservation and cleanup

Both crates are standalone Cargo workspaces. The BPF crate is `no_std` Rust with Aya macros and `aya-ebpf=0.1.1`; the controller uses `aya=0.13.1`. Native `BPF_PROG_LOAD` receipts at serial lines 478 and 484 load SK_SKB and SCHED_CLS programs with GPL licenses. Lines 482 and 488 report native IDs 8/9, tags `36f905163fccb7de` / `a1d3be6181987314`, verified instruction counts 255/481, and JIT bytes 1397/1967. The actual TCX link creation is at line 489; effective egress query reports `[9]`.

Checked every recorded executed source hash for a/b/d/f against retained files (6/7/7/7 files), all six local source manifests, both executed f Cargo locks, and every entry in the final evidence manifest: **131 files, zero mismatches**. The original f archive serial is byte-identical to retained serial, SHA256 `43c837e1c3c90e6c682b49971113c8cd8792d756ac52dd556adac6b28a78d1d5`. The serial's actual `/probe` and `/program.o` SHA256 readbacks match the executed artifact pins. The recorded original build commands, source manifests and locked dependency copies establish the source/build/runtime chain; this review did not rebuild or rerun it.

Earlier attempts remain negative evidence: a has the controller compile failure; b records native small/zero success and the 59,000-byte timeout; c/e record lease-acquisition failures before source sync; d retains the native verifier rejection for a variable zero-sized witness read. The b helper's exact native errno is not recorded and the findings correctly avoid asserting it. The successful f source does not overwrite those attempts.

The runner loads the BPF and unchanged stock vhost modules only inside the private KVM VM. Its command uses `-accel kvm`, 1GiB memory, four boot CPUs and 64 possible CPUs; the native VM reports the stock kernel/BTF and only `lo`. No custom module, kernel patch, C eBPF, TAP or physical host BPF loading appears in this execution path.

Native cleanup occurs before VM shutdown: all four SockHash entries are removed and the map reads zero occupied keys; peers reset/stop and release mmap/eventfds/sockets; TCX detaches and queries no remaining egress program; SK_SKB detaches and map/program/listener ownership is dropped. Serial lines 1356/1358/1360 record zero map occupancy, an empty TCX query and FDs 4 → 4. Line 1363 reads `vhost_vsock` usage zero in the still-running private VM. Harness and terminal completion exits are zero. The QEMU result records exit zero and PID absence; post-release readback records no owned QEMU PID or canonical owner file. Independently compared the physical module/link inventories before, after and at post-release readback: they are equal.

These are FD, module-reference, attachment and process cleanup observations, not a kernel heap leak proof. Foreign physical host BPF objects were not enumerated. Resource derivation is explicitly labeled as boot/probe/shutdown: six samples, peak QEMU RSS 309,284KiB, observed CPU delta 5.62s across 5.003s. The 6.530s whole-VM elapsed result is not transport throughput. Build output, rootfs, images and original private archives are ignored; `git check-ignore` confirms that boundary.

## Contract Shape Compliance

This is a native SPIKE executable, with zero new Rust `#[test]` tests or live pure-function properties and no DELIVER step. DES phase order, unit-test budget, acceptance-test outcome-anchor declarations and mutation gates are therefore not applicable. No production contract or executable test assertion was weakened to make the spike pass. Its observations enter through real ordinary UDP application sockets and real vhost virtqueues; packet/map/descriptor readbacks support the native mechanism explanation.

The inspected source does not expose a new production API. Its internal private wire format is probe support. The findings explicitly leave ordinary guest transparency, Cloud Hypervisor integration, kTLS/mTLS, SSH, IPv6, concurrent/large-population UDP behavior, full payload maximum and offload variants unvalidated. In particular, the eight-byte framed intermediate UDP send can constrain the effective maximum before TCX shrinking; 59,000-byte success does not approve weakening an accepted 65,507-byte IPv4 UDP payload contract.

## Verification performed and final disposition

Review verification was read-only source inspection, original capture parsing, independent packet/checksum/message-count reconstruction, archive comparison, SHA256 verification, cleanup-inventory comparison and ignore-boundary checks. No new kernel execution, probe experiment, production edit, design edit, roadmap edit, DES event or commit was performed. Only this review artifact was written.

**Iteration 1: NEEDS_REVISION for R1 only.** The functional proof is established at its qualified native boundary. Correct the two archival evidence-location statements; preserve all sources and original captures. No expanded networking mechanism or additional test lane is required.

## Iteration 2 — documentary provenance correction

**Verdict: APPROVED. R1 is closed.**

Re-reviewed only the R1 correction in the findings, attempt index, derived artifact audit, new `archive-readback.derived.json`, and refreshed final evidence manifest. The findings now state that retrieval runs after `native-launch.log` closes and that its console success/hash/failure messages are absent from those retained logs. They no longer claim a historical c/e retrieval-error receipt. The c/e attempt entries correctly identify only the retained lease-acquisition failure and launcher return code.

The archive hashes and member comparisons are explicitly labeled current post-execution derived readbacks. This distinguishes them from historical launcher output and matches the actual launcher control flow reviewed in iteration 1. The audit likewise labels its retrieval chronology as probe-agent execution chronology rather than a captured retrieval receipt.

Independently recomputed the refreshed manifest: **132 files, zero mismatches**. Rechecked a/b/d/f executed-source hashes against the retained sources with no mismatch. Recomputed all four original archive hashes and compared original b/d/f native serial members with the retained native files: all are byte-identical and their serial SHA256 values match the new derived readback and iteration 1 observations. The a archive correctly reports no native serial member because compilation failed before native execution.

R1's required correction is complete. No native rerun, probe-source change, original capture/archive edit, production/API change, new architecture mechanism or additional networking requirement was needed. This review's only mutation is the review artifact.

**Final approval boundary:** the recorded Aya Rust private-prefix and TCX packet-boundary mechanism accomplishes zero-length UDP in both measured directions through real stock vhost-vsock shared virtqueues and produces genuine empty ordinary UDP datagrams. All iteration 1 qualifications remain: synthetic guest endpoint/private codec, measured IPv4 checksum configuration, two peers, sampled payload sizes, bounded negatives and observed cleanup. This approval does not promote the scratch protocol to production or resolve ordinary guest transparency, full UDP maximum, checksum/offload variants, Cloud Hypervisor integration, kTLS/mTLS, SSH, IPv6, UDP scale or performance.
