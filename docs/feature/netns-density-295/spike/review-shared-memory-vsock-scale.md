# Review — stock virtio-vsock transport-device capacity

## Metadata and verdict

- Review ID: `spike_review_20261004_vsock_scale_16384`.
- Feature: `netns-density-295`.
- Reviewer: independent Codex reviewer, using the local `nw-researcher-reviewer` evidence/replicability dimensions within the explicitly bounded SPIKE review assignment.
- Iterations: 1 (initial review) and 2 (F1 documentary remediation review).
- Reviewed native attempt: `increment-c`; failed preparation attempts `increment-a` and `increment-b` remain evidence.
- Current review verdict: **APPROVED** after iteration 2 closed F1. Iteration 1 returned **CHANGES_REQUESTED**, solely for F1's documentary timing correction.
- Experimental result: **WORKS at the approved 16,384 stock transport-device boundary**. No capacity, owner-independence, stock-path, provenance, resource, or cleanup failure was found.

The user approved 16,384 activated stock Cloud Hypervisor devices and independent Unix muxers with synthetic peer drivers, and explicitly removed the new running-guest cohort. This review applies that boundary: zero KVM VMs and zero guest boots are intended. Combined guest-mTLS execution is explicitly excluded. Neither full microVM capacity nor production networking, policy, TLS, observation, lifecycle, or appliance-kernel correctness is an acceptance gate for this experiment. DESIGN/E18 remains pending.

## Finding and bounded remediation

### F1 — reported timing scopes differ from the executed timers

**Severity: medium; blocks approval of the findings document's measurement presentation.** This does not invalidate the transport-device capacity result. Remediation requires only correcting the findings prose/table against existing receipts; no source change, rerun, additional mechanism, or architectural choice is required.

The current [findings table](shared-memory-vsock-scale-findings.md#measured-stages-and-resource-budget) calls `full_pool_recheck_s` an all-device bidirectional recheck duration. In the executed [main.rs](../../../../spike-scratch/netns-density-295-vsock-scale/increment-c/src/main.rs), line 72 starts this timer before the exchange loop; line 73 then serializes and writes every owner's identity; line 74 enumerates process FDs/tasks before sampling the timer for the emitted record. Consequently the quoted **3.282924353s includes the exchange, identity capture/write, and process enumeration**. The retained final identity receipt decompresses to 7,342,077 bytes and 16,384 entries, so this instrumentation was actually executed. There is no separately captured pure traffic duration or separately timed final epoch-999999 recheck.

The [timing paragraph](shared-memory-vsock-scale-findings.md#predicted-versus-actual-native-output) also labels **28.570209s** as native dependency-build duration. [events.jsonl](../../../../spike-scratch/netns-density-295-vsock-scale/increment-c/evidence/events.jsonl) records `build-start.at_s = 0.09396740514785051` and `build-complete.at_s = 28.570209346711636`. Their difference is **28.476241941563785s**; 28.570209346711636 is the completion timestamp from runner start. Finally, the retained [native-launch.log](../../../../spike-scratch/netns-density-295-vsock-scale/increment-c/evidence/native-launch.log) ends with launcher `wall_s = 145.50352470899816`; the document's 145.503891s is not that retained sample. The launcher samples the clock again when printing to stdout, so two near-identical readings are possible, but the document should identify a retained sample or round appropriately.

Required disposition:

1. Label the stage timer as exchange **plus identity capture and process enumeration**, preserving its actual receipt value and distinguishing it from the untimed final complete recheck. It may be described as an upper bound on the sequential exchange's wall time, not a pure traffic benchmark.
2. State build duration as approximately **28.476242s**, or explicitly label **28.570209s** as the runner-relative build-completion timestamp.
3. Use the retained launcher sample (approximately **145.503525s**) or a suitably rounded **145.504s**; alternatively retain and cite the exact stdout receipt used for another sample.

Bounded reproduction against existing artifacts:

```sh
python3 - <<'PY'
import json
from pathlib import Path
base = Path('spike-scratch/netns-density-295-vsock-scale/increment-c/evidence')
events = [json.loads(line) for line in (base / 'events.jsonl').read_text().splitlines()]
at = {event['event']: event['at_s'] for event in events}
print(at['build-complete'] - at['build-start'])
print((base / 'native-launch.log').read_text().splitlines()[-1])
PY
```

Actual analysis output:

```text
28.476241941563785
{"launcher_rc": 0, "wall_s": 145.50352470899816}
```

**Iteration 1 remediation disposition: OPEN.** F1 was returned to the original crafter for the bounded documentary correction. **Iteration 2 disposition: CLOSED**, as verified below.

## Evidence and correctness verification

### Actual stock activation and independent owners

The standalone [Cargo.toml](../../../../spike-scratch/netns-density-295-vsock-scale/increment-c/Cargo.toml) has its own `[workspace]` and imports upstream CH, not Overdrive production crates. `Peer::new` at `main.rs:28` constructs a fresh `VsockUnixBackend`, exit eventfd, `Vsock`, memory mapping, interrupt callback, status object, and three queues for each distinct index/CID/path. It invokes stock `VirtioDevice::activate` at line 42. The retained `Peer` vector keeps all previous owners alive while the next population is added. There is no shared substitute muxer, reused stock owner, logical CID registry, or listener-only counting path.

I read upstream files **at the pinned git object**, using `git show 9ed824d6d08df3e96f7d5f50795d9449ac99f431:<path>`. The read-only local source repository's current checkout is a different revision; it must not be mistaken for the pinned sources. All seven git-object bytes/hashes match [upstream-source-manifest.json](../../../../spike-scratch/netns-density-295-vsock-scale/upstream-source-manifest.json) and the independently captured native checkout hashes. The revision carries tag `v53.0`.

At that exact revision:

- `vmm/src/device_manager.rs` uses the same `VsockUnixBackend::new` and `Vsock::new` constructors.
- `virtio-devices/src/vsock/unix/mod.rs` exports `VsockMuxer` as `VsockUnixBackend` and defines **1,023 connections per muxer**.
- `virtio-devices/src/vsock/unix/muxer.rs:370` constructs each independent listener, nested epoll, CID, connection/listener maps, and RX/kill queues.
- `virtio-devices/src/vsock/device.rs:461` moves the actual three queues and memory into a stock worker handler retaining that device's backend; `run:210` registers its nested muxer epoll and all three queue eventfds in the worker's outer epoll.
- Stock `process_rx:128`/`process_tx:175` consume real descriptor chains and call the stock backend. The harness creates descriptor chains and reads/writes the actual used/available rings, not a bypass API.

The 128KiB memory contexts are freshly allocated by `from_ranges` for each peer. Wrapper addresses alone would not prove independent mappings; here that source construction, separate queue state, actual per-context stock traffic, retained map receipts, and independent kernel-object ownership collectively establish the counted population.

### Every-device traffic and complete recheck

`connect:60` performs the real stock backend `CONNECT 5001` handshake and completes request/response packets through the actual RX/TX queues. `exchange:61` checks different CID/epoch-tagged bytes in both directions. RX assertions also check stock-produced host CID 2, that context's destination CID, destination port 5001, and expected packet operation.

At `main.rs:71–74`, creation finishes before the exchange loop runs over **every retained peer** at each stage, including 16,384. A stage success record can be emitted only after every fallible exchange and assertion succeeds. A failure propagates out of `main` or panics and cannot reach the successful cleanup record. After the final observer hold, line 76 exchanges another distinct epoch through the entire still-held pool, then performs cleanup. Its subsequent `cleaned` record, the capacity-process exit 0, and empty stderr establish completion. That final recheck is not separately timed.

The proof concerns light, sequential bidirectional traffic with one established connection per owner. It establishes operational owners at held population, without claiming simultaneous bulk transfer, saturation, fairness, latency, guest AF_VSOCK behavior, KVM/PCI composition, or production seccomp. The findings correctly expose those limitations.

### Independent native kernel audit and resource reconciliation

The separate Python runner's `audit:22` stops the owned process and verifies every thread is stopped before observing kernel state; `finally:60` resumes it. I independently parsed all five preserved owner-audit, identity, and resource receipts. For **every row at every stage**, I reproduced distinct CIDs, listener paths/inodes/FDs, nested epoll FDs, outer worker epoll FDs, exit eventfd identities, host connection socket identities, device addresses, and memory-wrapper addresses. Each nested epoll has exactly its listener plus connection registered; each corresponding outer epoll contains that nested epoll and six stock event registrations total. The raw receipts establish distinct outer epolls, beyond merely trusting the runner's aggregate label.

| Held stock devices | External process FDs | Process threads | PSS, KiB | RSS, KiB | Reproduced identity cardinalities |
|---:|---:|---:|---:|---:|---|
| 4 | 87 | 5 | 1,313 | 3,284 | All 4 |
| 1,024 | 21,507 | 1,025 | 79,969 | 81,940 | All 1,024 |
| 4,096 | 86,019 | 4,097 | 309,329 | 311,300 | All 4,096 |
| 8,192 | 172,035 | 8,193 | 615,121 | 617,092 | All 8,192 |
| 16,384 | 344,067 | 16,385 | 1,226,449 | 1,228,420 | All 16,384 |

All resource-summary fields reproduce from the actual `status`, `smaps_rollup`, mapping counts, and host MemAvailable receipts. The final CIDs are exactly **100000–116383**. The external FD formula **21 × count + 3** reconciles with stock ownership: two exit-eventfd handles, nine queue-eventfd handles, two kill-eventfd handles, three pause-eventfd handles (including the stock epoll helper's clone), two epolls, and three Unix socket FDs per peer, plus standard input/output/error. Internal enumeration adds its transient directory FD. Total native process threads are stock workers plus the main thread.

The loaded executable's independently read `/proc/814497/exe` hash matches the built binary hash at all five stages: `4d0b81949b62283fe31396ec0e83cf2aab10e452002122d41442fc4b3a19c11e`. The runner hashes that actual symlink target's bytes directly, avoiding a helper-process `/proc/self/exe` fingerprint. The final PSS/RSS, 61,118,072KiB MemAvailable, virtual size, PTE accounting, FD headroom, and scope distinction between process PSS and whole-host available memory are correctly qualified.

The NOFILE readbacks and `before_exec:70` confirm that only the capacity process's soft limit changes to its existing hard limit **524288**. No global-limit writer appears in the probe. Live receipts retain the session cgroup identity; later absent-session and surviving-ancestor observations are correctly labeled post-release. The retained separate hardware receipt supports the processor/core description; native scale receipts establish kernel, memory, zero swap, tasks, and cgroup. Linux **7.0.0-29-generic** is explicitly distinguished from appliance 6.18 proof.

### Provenance, failed attempts, isolation, and cleanup

- All source hashes in the three attempt manifests match their retained sources. Attempt a really stops at the missing snapshot `OUT` binding before building devices; attempt b really stops at canonical rsync exit 23. Their sources/failure receipts were not rewritten into the successful attempt.
- The retained native executed lock decompresses byte-for-byte to the checked-in-candidate `Cargo.lock`. Its CH git dependencies all resolve to the exact pinned revision. The build transcript identifies the pinned dependency build and standalone probe.
- Native post-release source hashes match the retained actual Rust, runner, snapshot, manifest, and executed launcher sources. The actual upstream checkout HEAD and tracked cleanliness agree with the claimed stock source; Cargo's `.cargo-ok` is the sole reported untracked upstream item.
- All **seven** preservation-manifest gzip round trips independently match original lengths, original SHA-256 values, and compressed SHA-256 values. The private exact archive's digest matches its retrieval receipt. I recomputed both administrative complements from that exact archive; both match the retained complement and each other. Shared plain-text address redaction does not indicate a configuration difference.
- The canonical launch log and native identity record agree on exclusive lease token `366ba1d33a9392b0b674f30f`. The previous exact import cache cleanup has its separate lease and receipt; no broader cleanup is implied.
- `Peer::drop:67` calls stock reset/shutdown. At the pin, `VirtioCommon::reset` drops `WorkerThreads`; its destructor signals, unparks, and joins the threads. The capacity process actually returns to **1 thread / 4 internally enumerated FDs / zero socket pathnames** before exit. Its runner cleanup receipt reports no forced residual socket removal and zero added host OOM kills.
- Post-release evidence independently records all seven exact task/lease PIDs absent, both owned socket directories absent, the exact prior cache absent, and canonical lease-owner metadata absent. Administrative links/addresses/routes/rules/nft/BPF/netns/module identities and sampled sysctls match. No equality of dynamic foreign counters, neighbors, timers, or processes is claimed.
- Eligible spike files are standalone sources, scripts, lock, Markdown, and native receipts, including lossless compressed text. `target/` build output and `out/` private archives are gitignored. No VM images are present in the candidate evidence boundary. Pre-existing dirty production/DES work is outside this review and was preserved.

## Documentary compatibility with #303 and PR #306

I verified both supplied metadata/body digests against [interaction-source-metadata.json](../../../../spike-scratch/netns-density-295-vsock-scale/interaction-source-metadata.json), including PR #306 head `e0c1b4190cbcfaf44c82d325f405a224f2cc8ffc`, and read the bounded bidirectional comparison in the prior [functional guest findings](shared-memory-vsock-findings.md#bidirectional-interaction-with-issue-303-and-pr-306).

The scale document correctly treats #303's host-held SVID key, host rustls/policy, guest TCP/kTLS socket hook, and vsock resolution/handshake channels as **proposal/source claims**, not accepted DESIGN or execution performed here. In both directions, the implications remain accurate: proposed guest TLS does not remove observed stock transport-device costs; this device-capacity result does not establish combined handshake, policy, resolution, encrypted bulk-flow, or contention budgets. Host control-relay survival and host bulk-transport-owner survival remain distinct. CID/request bytes are not authenticated allocation identity. No new topology, ownership, channel, persistence, or recovery mechanism is required by this review.

## Verification method and final disposition

This was a bounded source-and-receipt review: exact pinned upstream git-object reads, standalone source inspection, digest and lock comparison, complete parsing of all five owner populations, lossless receipt verification, resource arithmetic, exact-archive complement reconstruction, and documentary #303/#306 comparison. No full-population rerun, guest, module/mTLS experiment, production/DES/design edit, or mutation test was performed. No scale-local `local-verification*.json` exists among the supplied artifacts; the independent calculations above reproduce the necessary claims directly from native receipts instead.

The iteration 1 evidence met the approved capacity boundary. Its **CHANGES_REQUESTED** verdict was confined to F1's timing scopes and retained sample attribution. DESIGN/E18 remains pending; approval of this SPIKE cannot promote a replacement design or weaken any accepted invariant.

## Iteration 2 — F1 documentary remediation review

**Verdict: APPROVED. F1 is CLOSED.** The original crafter corrected the findings document, the scratch root and increment-c READMEs, and the derived measurement summary. This re-review was limited to those documentary corrections and their existing receipts.

| F1 requirement | Corrected presentation | Independent verification | Disposition |
|---|---|---|---|
| State the actual stage timer scope | The findings table and adjacent prose, both READMEs, and summary explicitly include exchange, owner identity capture/write, and FD/task enumeration. The final complete recheck is explicitly not separately timed. | All five summary timer values equal the original `harness.stdout` `full_pool_recheck_s` values exactly: 0.000959765, 0.244360008, 0.943922722, 1.704718321, and 3.282924353 seconds. All five final-recheck timing flags are false. | CLOSED |
| Distinguish build duration from completion timestamp | Findings and both READMEs state 28.476242s; findings explicitly identify 28.570209346711636 as the runner-relative completion timestamp. | Existing build-event subtraction reproduces 28.476241941563785s, rounding to 28.476242s. | CLOSED |
| Cite a retained launcher sample | Findings and both READMEs state 145.503525s; findings identify the retained launch log value. | The original log's final record is 145.50352470899816s, rounding to 145.503525s. | CLOSED |

I also revalidated all three executed-source manifests against the retained source bytes; all still match. The native sources, execution entry points, pinned stock dependency, and originally reviewed experiment remain unchanged. No additional execution or architecture work was required or performed.

There are no remaining blocking findings. **APPROVED applies to the corrected SPIKE evidence for 16,384 actual activated stock transport devices and independent stock Unix muxers with synthetic peers, zero KVM VMs and zero guest boots.** Combined guest-mTLS remains excluded. DESIGN/E18 remains pending.
