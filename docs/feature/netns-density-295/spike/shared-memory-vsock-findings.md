# Shared-memory / virtio-vsock probe findings

## Verdict: WORKS — 64 real VMs at 96MiB and 0.125 host CPU quota; 16,384 NOT VALIDATED

Stock Cloud Hypervisor v53.0 carried real guest↔host services for **64 simultaneously retained Linux microVM identities**, each configured with **96MiB RAM** and an independently enforced **0.125 host logical-CPU quota**, with **zero new host TAPs or bridge ports**. Each guest has **one virtual CPU**; that integer guest topology is separate from its scheduled host CPU budget. The final full-pool HTTP recheck passed for all 64 distinct guest identities. The earlier **128-VM / 96MiB run without an explicit CPU quota also passed** and remains preserved as a separate result.

These are ordinary TCP applications using explicit guest loopback↔vsock wrappers. The probe does not validate transparent ordinary IP networking, the accepted Overdrive enforcement/observer/lifecycle contract, or a combined #303/#306 guest-kTLS implementation. The original **16,384 DESIGN/E18 contract remains pending**; the user approved lowering this experiment's resources and population, rather than substituting streams or host sockets for VMs.

## Revised resource/count decision

The user explicitly approved: **“lets lower both so it fits within the machine”**. This supersedes the immediate 16,384-count requirement for this resource-constrained experiment; it does **not** amend the source E18/DESIGN contract. The 256MiB Ubuntu fixture is historical evidence, not an architectural memory floor.

The same stock guest/kernel/service successfully ran at **128MiB** (attempt 07) and **96MiB** (attempt 08), each with the minimum **one vCPU**. The **64MiB / 80MiB** profiles (attempts 09/10) did not reach guest init/service readiness; the VMM a log reports `VcpuRun(... InternalError)` at 64MiB. Their setup-specific failure cause is not diagnosed as an OOM or a universal kernel minimum. **96MiB is the smallest service-capable profile actually demonstrated here**, with no new OS or kernel development.

The revised staged experiment is **128 actual VMM/device/backend identities**, booted eight at a time, with **96MiB and one vCPU per VM**. Admission requires current MemAvailable to cover all 12GiB of configured guest RAM, an additional **4GiB overhead allowance**, and **20GiB host reserve** (36GiB total). It also requires all 8GiB of worst-case image copies plus an 8GiB disk reserve. Headroom is rechecked before every batch, and the host `oom_kill` counter is read before/after. Only guest-ready, CID/executable-attested identities with real guest TCP-wrapper echo and real inbound guest HTTP count; a final full-pool HTTP recheck verifies reachability at the retained population cardinality. That without an explicit CPU quota run completed as attempt 12. The subsequent explicit CPU correction was validated as attempt 14 at 64 actual VM identities; no requested count is treated as an actual count.

## Final native fractional-CPU profile

Canonical final attempt: [increment 14](../../../../spike-scratch/netns-density-295-shared-memory-vsock/increment-14-quota0125-staged64-20261004T1032Z/README.md). **64 / 64** distinct VMM PIDs, loaded guest executables, local CID readbacks (**41000–41063**), guest TCP-wrapper echoes and inbound ordinary TCP HTTP responses passed. There were 256 transport negatives (three guest-side and one host-side per VM), and all 64 live HTTP services were rechecked while the complete population remained retained. This is a VM/device/backend count, not a stream count.

Every owned VMM entered its cgroup **before CH exec**. Each readback reports `cpu.max = 12500 100000`; all **576 VMM threads** (nine per VMM, including the device/service workers) were in their assigned cgroup. The owned ancestor had `cpu.max = 800000 100000` (eight logical-CPU time units across 64 children); root had no `cpu.max` limit and the actual scheduler affinity was `0–15`. The host has 16 logical CPUs. Quotas cap CPU time; they do not reserve physical cores. The shared host receiver is measured separately and is **outside** the per-VMM CPU quota.

The root already enabled the CPU controller. The probe enabled `+cpu` only on its new empty owned parent, and never changed a foreign/root controller or affinity. Cpuset was not enabled on the owned subtree: its absent `cpuset.cpus.effective` is recorded as inherited scheduler affinity, not a missing CPU-quota boundary. Attempt 13 preserved a probe readback error caused by incorrectly assuming that optional cpuset file existed; it started no VMM. Corrected attempt 14 proved the CPU controller and effective ancestor limits without enabling cpuset globally.

Quota enforcement was observed: **100–106 throttled periods per VMM**, totaling 6,607 sampled throttled periods and 570,427,914µs of summed per-VMM throttled time. Those sums are across 64 cgroups, not a single wall-clock pause. Sampled total VMM CPU usage was 88,129,270µs. **0.125 passed; no 0.25 fallback ran.**

| Actual live communicating VMs | Configured guest RAM | Total VMM RSS (KiB) | Total VMM PSS (KiB) | VMM FDs | VMM threads | Host MemAvailable (KiB) |
|---:|---:|---:|---:|---:|---:|---:|
| 16 | 1,536MiB | 1,662,784 | 1,591,616 | 1,776 | 144 | 61,772,800 |
| 32 | 3,072MiB | 3,325,568 | 3,178,496 | 3,552 | 288 | 60,190,192 |
| 64 | 6,144MiB | 6,651,160 | 6,352,280 | 7,104 | 576 | 56,967,448 |

At the final 64-VM stage the host receiver used **872KiB RSS / 864KiB PSS, 67 FDs and 65 threads**. It is a throwaway thread-per-listener receiver; these observations are not production proxy sizing. Admission required **6GiB configured guest RAM + 4GiB overhead allowance + 20GiB host reserve = 30GiB**, against 63,438,212KiB MemAvailable. The peak sample still had 56,967,448KiB available (54.328GiB), and the host `oom_kill` counter stayed **37→37**. No memory-overcommit capacity claim is made.

Monotonic timings: native run through cleanup **126.643111s**, canonical sync/preflight launcher **177.027014s**, full-pool HTTP recheck **0.293890s**. Spawn→all guest checks ready ranged **14.109–14.609s** (median 14.324s), including deliberate CID timeout checks. Initial HTTP exchanges ranged **2.440–88.212ms** (median 3.807ms). These are observed thin-service samples under a 100ms quota period, not an invented SLO or a cryptographic workload benchmark.

The separate without an explicit CPU quota **128-VM** epoch reached 128 unique live PIDs/CIDs, all initial traffic checks and all 128 final-cardinality HTTP rechecks. Its sampled VMM totals were **13,299,368KiB RSS / 12,696,872KiB PSS**, 14,208 FDs and 1,152 threads, with 50,681,776KiB MemAvailable; native cleanup completed in 111.039973s and OOM count remained 37→37. Those 128 results are not represented as fractional-CPU evidence.

## Question and boundary

Can existing stock CH virtio-vsock/shared-memory transport provide real host↔microVM services without a host TAP/bridge port per VM, with an explicit feasible mapping to Overdrive's TCP/service/DNS/isolation/capture-before-CPU requirements?

This is an isolated PROBE. It changes no production source, tests, design, roadmap, API or promotion decision. The canonical source/run index is [the spike README](../../../../spike-scratch/netns-density-295-shared-memory-vsock/README.md). The probe follows `.claude/rules/spike.md`, retaining all executed increments instead of generic `/tmp`/deletion defaults.

## What actually ran

The initial successful epochs (04, 05, 06) each booted two Linux guests from the existing native fixture. Epoch 06 adds independent loaded-executable, module, CID and resource attestation. Earlier unsuccessful attempts remain retained.

| Predicted | Actual epoch 06 observation |
|---|---|
| CH can boot virtio-vsock without `--net` | Both VMM command lines omit `--net`; each guest exposes only `lo`. The live host link inventory contains the same link identities as the before inventory. Neither VMM holds `/dev/net/tun` or `/dev/vhost-vsock`. |
| Separate VMM backends demultiplex probe VM identities | Guest ioctl reads CIDs **39501 / 39502**; the owned `va_5000` / `vb_5000` listeners produce their respective `host-echo-a` / `host-echo-b` replies. CID is not treated as a host-global authentication credential. |
| Real guest→host bytes, adapting ordinary TCP | A guest `TcpStream` connects to its local wrapper on `127.0.0.1:8081`; the wrapper opens real guest `AF_VSOCK` to host CID 2, port 5000. Both 26-byte requests return the correct 38-byte identity-specific echo. |
| Real host→guest bytes, adapting an ordinary HTTP app | Host `AF_UNIX CONNECT 5001` becomes a guest vsock stream; the guest wrapper opens ordinary TCP to its HTTP application on `127.0.0.1:8080`. Both **73-byte HTTP responses** contain the correct 15-byte a/b body. |
| Unregistered ports/CIDs have no successful service path | Missing host port 5009 resets both guest connects; host `CONNECT 5009` receives no `OK` in either guest. Unknown CID 49999 and each sibling CID fail the bounded two-second connect attempt. This is transport behavior, not an Overdrive policy-loss/adversarial security proof. |
| Receiver/capture can be armed before guest CPU runs | Both host receivers were ready at **0.975861s**, AF_PACKET capture ready at **1.026951s**, and VMM spawns occurred at **1.162351s / 1.272544s**. Readiness precedes any spawned VMM CPU opportunity. The capture cannot see virtio-vsock payloads. |

Exact output excerpts, retained in [epoch 06 evidence](../../../../spike-scratch/netns-density-295-shared-memory-vsock/increment-06-native-20261004T0954Z/evidence/events.jsonl):

```text
GUEST_LOCAL_CID role=a ioctl_rc=0 expected=39501 actual=39501 matched=true
GUEST_NET_INTERFACES ["lo"]
GUEST_TCP_WRAPPER_RESULT role=a passed=true elapsed_us=498 request="guest-to-host-a-cid-39501\n" reply="host-echo-a:guest-to-host-a-cid-39501\n"
GUEST_DENIAL role=a label=unregistered-port cid=2 port=5009 denied=true errno=Some(104)
GUEST_PROBE_READY role=a transport_result=true
```

The Rust host backend and Rust guest helper are the transport implementation. Python packages the exact helper, starts/stops exact children and drives the real backend handshake; it does not emulate virtio-vsock in a host Unix loopback fixture.

## Initial two-VM timings and resource observations

All durations use a monotonic clock. These are individual mechanism observations with no invented performance SLO or throughput claim. Guest-ready measurements include roughly four seconds of deliberately denied-CID deadlines.

| Epoch 06 operation | Actual |
|---|---:|
| Native release build | 0.699083s |
| Guest a / b spawn→all guest checks ready | 5.296011s / 5.287449s |
| Guest ordinary TCP wrapper echo a / b | 498µs / 537µs |
| Host→guest ordinary HTTP a / b | 3.495361ms / 2.379982ms |
| Native experiment through cleanup | 6.793470s |
| Canonical launcher including sync/preflight | 52.187543s |

| Live object, epoch 06 | Configured memory | VmRSS | PSS | FDs | Threads |
|---|---:|---:|---:|---:|---:|
| VMM a | 256MiB | 151,032KiB | 148,656KiB | 111 | 9 |
| VMM b | 256MiB | 151,040KiB | 148,664KiB | 111 | 9 |
| Shared Rust host receiver | — | 596KiB | 588KiB | 5 | 3 |
| Python runner | — | 31,144KiB | 25,074KiB | 8 | 1 |
| Guest Rust PID 1, each guest | included in guest memory | 604KiB | not sampled | 8 during `/proc/self/fd` enumeration | 4 |

Guest kernel `MemTotal` is 205,424KiB per configured 256MiB VM. Guest PID-1 memory is **inside** the VMM guest-memory mapping, so it must not be added again to host RSS/PSS. The shared receiver's low memory is not a per-VM memory measurement. Actual successful application exchanges were four across two guests; connection-map occupancy and queue depth were not instrumented or saturated.

## The 16,384 target: exact remaining gap

[Read-only hardware inventory](../../../../spike-scratch/netns-density-295-shared-memory-vsock/hardware-budget.json) and native meminfo record an AMD EPYC 8024P, **8 cores / 16 logical CPUs**, **65,401,772KiB RAM (62.372GiB)** and **zero swap**. The native runner and VMMs inherit NOFILE soft/hard **1,024 / 524,288**; each guest has **1,024 / 4,096**. Host `threads-max=470322` and `pid_max=4194304` are global settings; the root cgroup has no sampled `pids.max` file, and production confinement/cgroup admission was not exercised.

The revised 64-VM fractional-CPU and 128-VM without an explicit CPU quota results replace the earlier large-fixture arithmetic as the useful measured profile. Neither the initial 256MiB fixture nor the failed 64/80MiB attempts is treated as an architectural limit. **16,384 distinct VM/device/backend identities were not run**, and no different residency/paused/resume/shared-memory lifecycle is silently substituted. Counts, configured limits, physical RSS/PSS, host receiver overhead and application/crypto load remain separate quantities.

A stock CH vsock endpoint requires a real VMM-owned device/muxer/backend, not merely a pathname or a host listener. A 16,384 result must count **16,384 retained distinct VM/device/backend identities** in the agreed lifecycle state and show the required service reachability and resource/cleanup evidence. No such population was created. This is a hardware/fixture and unpinned lifecycle/testability gap for the scale question, rather than evidence against vsock's small transport mechanism.

The **1,023 tracked connection entries are per Unix-vsock muxer/VMM**, including initialization and retained states. They are not a host-wide VM or CID cap. At both the v53.0 source tag and the research source pin, the muxer RX queue holds 256 entries, the kill queue 128, and there are three 256-descriptor virtqueues. A 16,384-VMM topology would therefore have 16,384 separate connection budgets; multiplying their constants is not a supported aggregate connection promise. Native FD limits and buffering can bind first. Saturation, sustained load and 16,384 identities remain **NOT VALIDATED**. [Exact source/ABI audit](../../../../spike-scratch/netns-density-295-shared-memory-vsock/source-audit.md).

## Mapping to the accepted Overdrive requirements

This table identifies feasible integration directions and exact unproved boundaries. It approves no API or normative contract change.

| Required behavior / owner | Concrete transport mapping and remaining proof |
|---|---|
| Ordinary application TCP and original IP/port | The tested explicit loopback wrapper proves stream adaptation only. It changes the endpoint the ordinary application connects to; original VIP/IP/port is not transported automatically. Transparent ordinary guest sockets would need an explicit guest agent/OS socket backend or wrapper carrying destination metadata, with the exact wire/error/security contract pinned in DESIGN. No unmodified ordinary-network ABI transparency was proved. |
| Allocation identity / Service resolution | Trust the owned per-VMM channel/path and its registered allocation generation, rather than request bytes or CID alone; then resolve requested Service identity through the existing Service owner. The probe has two fixed listener roles and no Service registry/SVID integration. Host-mediated VM↔VM streams are feasible, but direct sibling-CID connections are not provided by the tested stock muxer. |
| Egress policy | A host proxy can mediate approved targets. A guest CID/port request is not authorization for arbitrary host TCP/Internet access. Destination validation, capability policy and failure behavior remain unproved. No virtio-net device existed in these guests, but that does not prove the production enforcement/loss contract. |
| DNS wire behavior / existing guest-DNS owner | Vsock provides streams, not automatic UDP DNS/IP gateway behavior. An explicit guest DNS adapter could carry real DNS queries to the existing host responder and return DNS wire answers; returned logical addresses must be usable by the chosen socket adaptation. No DNS, guest VIP routing, external ingress or generic UDP traffic ran in this probe. |
| TCX classifier and independent proof-mark guard | With no TAP, their ifindex/L2 hooks and `0x295a`/`0x295b` marks do not witness this stream. An exact approved replacement gate would have to preserve source identity, registration, independent loss protection and denied bypass outcomes. The existing TAP-specific contract is **not** silently satisfied by the absence of Ethernet frames. |
| TPROXY / leg-F original destination | `AF_UNIX` input has no intercepted `AF_INET` original destination. The trusted destination mapping and composition into the existing transparent-mTLS path need an explicit design and real production-owner proof; sending vsock bytes alone does not prove TPROXY equivalence. |
| Mutual TLS 1.3, kTLS TX/RX, splice / legs B–C | Those host TCP sockets could retain the existing TLS policy after stream adaptation. This experiment is plaintext within the guest/host channel and contains no SVID, TLS, kTLS or splice experiment. A plaintext local channel does not waive mandatory protected host transport. |
| Capture before CPU and loss/truncation-sensitive observer | Receiver and AF_PACKET readiness precede VMM spawn. AF_PACKET captures **no virtio-vsock channel bytes**; successful Rust reads are not a loss-accounted pre-enforcement observer. Exact pre-CPU observation, positive controls, loss accounting, tuple/channel correlation and zero-unmarked/zero-bypass outcomes remain unproved. The epoch-06 AF_PACKET process reported 26 captured / 40 received / 0 kernel drops; this is not a qualifying zero-leakage receipt. |
| Intercept-installed → activation → EXEC; READY zero-frame barrier | The guest init sends probe traffic after its listeners are ready; no Overdrive intercept-install, command-release or inactive attachment barrier ran. Admission/activation/quiescence must be explicitly mapped and verified through the real lifecycle owner. No weakening of the accepted zero/never/must clauses is implied. |
| Cleanup / restart isolation and singular owner | Exact scratch child/path cleanup is proved below. Production stale-CID/path/generation adoption, channel teardown, restart, loss audit and retry/error ownership were not tested. No new persistence or recovery subsystem is invented by this probe. |
| Appliance OS prerequisites | Stock guest vsock/virtio-vsock support must be built in or its matching upstream modules packaged/loaded before use. Epoch 06 loaded the three existing stock guest modules only. The pinned 6.18 appliance kernel was not exercised, and no host module was loaded/unloaded or kernel source patched/forked. |

These gaps follow the accepted [TCX/mTLS/observer contract](../../../product/architecture/adr-0115-tcx-endpoint-classification-feeds-transparent-mtls.md), [activation contract](../../../product/architecture/adr-0131-activate-allocation-tap-after-intercept-live.md) and [required intercept/DNS owners](../../../product/architecture/adr-0138-required-serve-boundary-intercept-and-guest-dns-ports.md). The mapping makes the candidate substantive while leaving those outcomes required.

## Bidirectional interaction with issue #303 and PR #306

**Combined native proof: NOT RUN — intentionally excluded by the user from this spike.** This probe loaded only stock virtio-vsock modules and transported plaintext toy echo/HTTP streams. It did not load the proposed socket hook, install guest kTLS, run host rustls/policy/SVID resolution, kill a handshake/data relay, or exercise a cryptographic/handshake workload.

Primary inputs are the current [issue #303 proposal](https://github.com/overdrive-sh/overdrive/issues/303) and [PR #306](https://github.com/overdrive-sh/overdrive/pull/306), head `e0c1b4190cbcfaf44c82d325f405a224f2cc8ffc`, read from the supplied current metadata/body receipts. The issue authors report stock Linux guest-module client/server/nonblocking proofs on **Firecracker**, record-granular host-driven vsock handshakes, separate-socket resolution, host-held SVID keys, guest session material and established-connection survival after the handshake relay dies. The PR is throwaway proof code, **not accepted production DESIGN**, and the Linux CH variant remains unrun in its body. Those are attributed source claims, not independently repeated results of this probe.

| Interaction | Source claim, observed boundary and bounded inference |
|---|---|
| Guest socket/kTLS hook versus this no-`--net` wrapper | #303 hooks the workload's own **TCP socket**, resolves/re-writes its destination and installs kTLS there. This probe gives the ordinary app a local TCP connection to a wrapper and forwards stream bytes over vsock; it provides no transparent peer IP/TCP route or hook/handshake installation proof. Carrying an encrypted TLS byte stream over a transport wrapper is conceivable, but exact socket/original-destination/peer and inbound-accept semantics would need separate design/proof. Our plaintext HTTP success establishes none of that. |
| Network transport remains required under #303 | Moving encryption to the guest does not itself remove its data transport or TAP/bridge-port demand. Routed TAP, multiple bridges and kernel OVS retain a virtio-net/IP path on which proposed guest kTLS could emit ciphertext. Conversely, vsock as **data** transport removes that host netdevice but requires explicit socket adaptation and host stream forwarding. The proposed handshake/control use of vsock and the proposed bulk-data use are different layers. |
| Host relay cost and survival | #303's reported survival is possible because established TCP data bypasses the killed handshake relay. The present stream-wrapper path still traverses the VMM Unix muxer and a live host data receiver/forwarder. Deleting a host TLS proxy does not delete this transport relay's FDs, buffering, CPU or lifecycle. If the same host agent owns bulk-data forwarding, losing it cannot be assumed to preserve established streams; a separately surviving data owner would be a **new explicit design choice**, not a mechanism authorized or tested here. No loss test or preservation claim is made. |
| Per-VMM channel identity and contention | Both proposals can use the same existing CH guest↔host capability. Resolution, record-granular handshakes and bulk streams would share the **per-VMM 1,023 tracked-entry budget**, virtqueues and muxer queues unless an approved design chooses different framing/port ownership. Separate sockets avoid the specific lock re-entry source #303 names; they do not create unlimited capacity or isolation from bulk-data congestion. Trusted allocation/generation→owned path/port mapping, admission, fairness, control liveness and teardown must be pinned. CID/request bytes alone are not authenticated allocation identity. No combined contention or handshake-admission experiment ran. |
| Encryption/capture locations and key custody | Proposed guest kTLS moves plaintext/encryption boundaries into the workload socket's guest kernel; an Ethernet datapath could then capture ciphertext on the TAP, while a vsock data channel still escapes AF_PACKET. Control vsock can carry session key/secret material under #303 even while the SVID private key remains host-held. Observer placement, custody/redaction, handshake-record boundaries and exact zero-leakage proof must therefore distinguish data ciphertext, guest plaintext and host-control secrets. This probe carried no keys and proves no #303 custody or TLS observer claim. |
| Reciprocal density/resource effects | If accepted and implemented, guest encryption could remove existing host per-flow TLS/splice-pump cost, but host-mediated vsock **data** can reintroduce an independent per-flow transport cost. Neither is measured by our short echo receiver. The **96MiB / 0.125 quota** profile excludes the hook module, guest kTLS AES/record work, proposed per-socket handshake kthreads/driver pool, host rustls/policy/resolve load and concurrency. Any later approved combined design would need its own resource measurements; this is outside the user-authorized probe, and neither #303 nor these results promises that budget for encrypted production services. |
| Acceptance and current contracts | #303 adoption would change encryption ownership and which TPROXY/host-proxy mechanisms remain required, while #295's current accepted TCX/guard/observer/activation outcomes remain normative today. PR proof and theoretical fit do not ratify that change, weaken a zero/never/must invariant or pin a public/error/wire contract. The interaction assessment supplies decisions/proof boundaries for a later approved DESIGN, and implements none of them. |

## Provenance, failed attempts and cleanup

Native host **and tested guest kernel**: `7.0.0-29-generic`, x86_64. CH reports v53.0. Rust: 1.95.0, commit `59807616e1fa2540724bfbac14d7976d7e4a3860`; target `x86_64-unknown-linux-musl`, optimized static executable. The source audit additionally checks source tag v53.0 (`9ed824d6…`) because research HEAD (`ae14eb5d…`) is newer; the executable fingerprint does not itself attest a reproducible upstream source build.

| Epoch-06 object | SHA-256 |
|---|---|
| Native parent `/proc/self/exe` (Python 3.14, read by that Python process) | `52e0a13e60a981d8c4b6478be2ba5176f69da07948a056bf49cf6f077e30cb41` |
| CH executable and each live VMM `/proc/PID/exe` | `448af3d4e59b22c2987f7df94c213ad40fb53a10d437e42b5ee6c4fce7c29ecc` |
| Selected guest kernel | `b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682` |
| Baseline 64MiB guest image | `4f7f97841e22384f1aebb040e199d78844ab418ad4859e6c262809d557898d85` |
| Rust source | `147293a09f3e427e3cb91b2c4c8e082b71bcec763cacdd340372e461a936768b` |
| Built helper **and loaded guest `/proc/self/exe`**, both guests | `78a0b5b32f27637c449f210b1c928c020c06ba0e8267842153d0a84d293327c4` |

Guest modules have independent hashes in both console logs. Their guest-only loads succeeded. PID 1 was the helper whose compiled source marker and executable hash match the retained source/build receipt. This avoids fingerprinting a child `sha256sum` executable as the parent.

Attempt 01 stopped before any VMM because canonical paths exceeded Linux's Unix-socket pathname length. Attempt 02 booted two real guests but the stripped fixture had no BusyBox binary; the helper failed before vsock use. Its exact children exited, but CH's two API `.lock` files survived a probe cleanup bug. Attempt 03 removed **only those witnessed owned lock files** and then stopped at a musl ioctl argument-type compile error. Attempt 04 corrected setup and passed; 05 repeated the same transport code with pinned libc/added SHA dependency after a local preparation script failed before writing the intended attestation changes; 06 applied and executed those attestation changes. None of the early failures is rewritten as a transport negative or hidden.

Epoch 06 used exclusive canonical lease token `b46672e94bde74ab6b50d834`. Its VMM PIDs 796496/796508, receiver PID 796488 and capture PID 796491 all exited; only exact owned socket/API/lock/image paths were removed. Before/after **administrative configuration complements matched** for links, addresses, routes, rules, nftables, BPF maps/links, namespaces, module identities and sampled sysctls. Raw snapshots retain volatile fields; comparison normalizes counters/timers. It does **not** assert equality of foreign dynamic neighbor contents, packet counters, memory/page-cache state or every foreign process.

[Final post-release native attestation](../../../../spike-scratch/netns-density-295-shared-memory-vsock/post-release-attestation-04.json) reports the canonical lease-owner file absent, **334 recorded probe PIDs absent**, and all **14 exact runtime/quota paths absent**, including both quota parents. Attempt 14 stopped all 64 VMMs, receiver/capture processes, removed the exact images/socket/API paths and all 64 owned CPU cgroups plus parent; its administrative complement matched. No probe systemd scope, netdevice, netns, route, nft/BPF rule or host module was created. The only host cgroups created were the explicitly owned CPU-quota fixture paths, all removed. No global cleanup, broad kill, foreign module unload, sysctl tuning or neighbor flush ran.

Large readbacks/text are losslessly compressed with verified byte round trips. Whole-host PCAPs include unrelated SSH target headers, so **exact originals remain in ignored canonical `out/` private archives/raw-captures**, while committed derivatives replace only the configured target IPv4 with `198.18.255.254` and recompute checksums. Those derivatives are explicitly not raw wire evidence and support no zero-leakage claim. Text receipts redact only the configured metal address. [Preservation manifest](../../../../spike-scratch/netns-density-295-shared-memory-vsock/evidence-preservation.json) records original/derivative/compressed hashes and transformations.

## Recommendation / gate

Keep shared-memory/virtio-vsock open as an **explicitly different guest/host transport architecture**: the no-TAP stream mechanism works at the measured 64-VM fractional-CPU profile and separate 128-VM without an explicit CPU quota profile. **Do not promote it or declare the 16,384 comparison complete.** Full population, guest adaptation, production policy/TLS/DNS composition, observation and lifecycle design/proof remain outstanding. The official Unikraft high-density precedent justifies investigation; its private fork/scaled-to-zero machinery and conflicting current TAP-pool documentation are not attestation of this stock CH configuration. No source-bound workaround, performance promise or contract weakening follows.

The initial one-hour probe was extended only for the user's successive explicit resource/count, fractional-CPU and #303/#306 assessment corrections. No additional speculative mechanism, OS/kernel development, production change or promotion followed.

The user explicitly excluded testing the combined guest-mTLS path here. The #303/#306 assessment is documentary only; NOT RUN is the intended scope boundary, not a pending gate for this probe.
