# STEP 11-06 review reproduction — Increment A

This retains the standalone Rust review probe and every captured failure/control
receipt from the first STEP 11-06 DELIVER review. The original committed product
under review was `2f92c925d374a21f41f74e34bd9f58767d53429e`; the final source
marker was `9d7463e8bf1c8bb89926dff8b9394cd68a9ad12f` after its DES/formal
follow-ons. The native guest kernel was Linux `7.0.0-38-generic`, x86_64.

The probe is an isolated Cargo workspace with path dependencies on the existing
production/test-support crates. It boots the production static `overdrive-init`
as PID 1 through GuestKit and the production `CloudHypervisorVmm` confinement.
The workload and independent host peer exercise UDP sockets and the existing
ControlOpen/Paired/OVD1 wire protocol. Every module/BPF/routing effect under
investigation remains in disposable CH guests. The physical Metal host is launch
substrate. There is no C/BPF probe source, production edit, new permanent test
lane, expectation runner or benchmark here.

Verdict for the original product: **DOESN'T-WORK** for the three already-approved
requirements recorded in [review-11-06.md](../../../docs/feature/netns-density-295/deliver/review-11-06.md):

| Case | Prediction required by the existing contract | Captured actual result |
|---|---|---|
| Default workload kit | Guest's DeviceOpen/Version setup reaches READY without a host-only loopback probe prerequisite. | Full probe fails at StreamRoundTrip before READY; host runner exits 4. |
| First UDP send, optional loopback present | Supported guest kit supplies the diagnostic capability needed to open a UDP association. | inet-diag replies ENOENT; first admitted datagram stalls, zero host flow requests, guest EXIT 2. |
| Full pool, immediate socket replacement, diagnostics available | A still-busy pool may refuse EPERM; an admitted datagram must not enter the previous slot association. | 32/32 admitted first replacement datagrams lost; 32/32 later retries echo, zero EPERM refusals, guest EXIT 3. |
| Same pool, workload waits after old close | Allowing the ordinary owner release to finish isolates release ordering. | Zero admitted losses, guest EXIT 0. |

Keep 11-06 at NEEDS_REVISION until its original crafter/DISTILL remediation and
the same reviewer's re-review complete. This probe does not prescribe a new
public probe API, slot-ownership mechanism or model. There is no promotion into
CI or a permanent test tier. **No mutation testing is authorized anywhere in this
workflow.**

## Exact reproduction commands

From the repository root, after the existing native guest-kit prerequisites have
been prepared, each command below runs through the canonical Metal lease and
retains a new timestamped stdout/stderr/exit-status capture. The script never
overwrites the historical receipts.

```sh
bash spike-scratch/netns-density-295-review-11-06/increment-a/run.sh default-kit
bash spike-scratch/netns-density-295-review-11-06/increment-a/run.sh udp-no-diag
bash spike-scratch/netns-density-295-review-11-06/increment-a/run.sh udp-recycle
bash spike-scratch/netns-density-295-review-11-06/increment-a/run.sh udp-recycle-wait
```

The script expands each into `cargo xtask metal run -- bash .../run.sh
--on-metal ...`. Inside the leased host it runs:

```sh
rustc --edition=2024 --target x86_64-unknown-linux-musl \
  spike-scratch/netns-density-295-review-11-06/increment-a/src/guest.rs \
  -o spike-scratch/netns-density-295-review-11-06/increment-a/target/review-guest
CARGO_TARGET_DIR=target cargo run \
  --manifest-path spike-scratch/netns-density-295-review-11-06/increment-a/Cargo.toml \
  --bin review-host -- \
  spike-scratch/netns-density-295-review-11-06/increment-a/target/review-guest \
  [case flags from the table below]
```

| Script case | Host runner flags |
|---|---|
| `default-kit` | none |
| `udp-no-diag` | `with-loopback` |
| `udp-recycle` | `with-loopback with-diag` |
| `udp-recycle-wait` | `with-loopback with-diag after-release-wait` |

`OVERDRIVE_REVIEW_EVIDENCE_DIR` selects only the console-capture directory. It
changes no kernel setup, wire message, workload or oracle. Build output stays
under ignored `target/` directories and is not retained as source/evidence.

The commands above reproduce the cases with the current product tree; the
historical outcomes are claims about the original reviewed product. No runtime
rerun was performed merely to relocate this material.

## Retention and capture history

All 13 original files were moved/preserved from the former ignored directory.
The ten raw logs are byte-identical. `evidence/retention-manifest.json` records
every original path, retained path, byte length and SHA-256; it verifies every
original byte survived. `evidence/commands-at-capture.txt` preserves the wrapper
invocations verbatim, including their original paths. Those historical path
strings are documentary and were not rewritten in the raw evidence.

The original final host source is preserved byte-for-byte at
`source-at-capture/host.rs` (SHA-256
`a51d6420dae6607219659e30ea17219383606e77971b9ceb5a6ea92eb3e71ebf`).
`src/host.rs` differs only in output-directory plumbing needed for this
relocation. The guest source and manifest are byte-identical to their originals:

| Original final source | SHA-256 |
|---|---|
| `src/guest.rs` | `dca5e8cbd868b803499c28f77b56dca79f2c8f135da1e221d9bbaefe62b91334` |
| `Cargo.toml` | `d714c35ef3e91a1971c41188bc0570ea0921048d2062f2bc4ee65ef818352fbf` |

Earlier harness source versions were not separately snapshotted when first run;
the raw capture chronology is preserved without inventing such snapshots:

| Raw capture | Meaning |
|---|---|
| `native-run.log` | Initial host-harness compile errors; no product conclusion. |
| `native-run-2.log` | Initial default-kit refusal before console diagnostics were added. |
| `native-default-kit.log` | Completed F1 refusal with full guest console. |
| `native-udp-loopback-prerequisite.log` | F1 prerequisite contrast reaches READY, then first UDP stalls. |
| `native-udp-diagnostic.log` | F2 module inventory and exact ENOENT inet-diag replies. |
| `native-udp-diag-loaded.log` | Diagnostic prerequisite contrast, followed by a host-harness nofile mistake; no release-order conclusion taken from this run. |
| `native-udp-recycle.log` | Initial full-pool admitted-loss reproduction and successful retries. |
| `native-udp-recycle-wait-control.log` | Same product with the workload wait: zero losses. |
| `native-udp-admitted-loss.log` | Final oracle permits EPERM and counts only admitted datagrams lost; 32/32. |
| `artifact-identity-cleanup.log` | Product/BPF digests and completed host-runner cleanup receipt. |

The final guest oracle accepts the designed EPERM refusal while a full pool is
still being released. It does not require immediate admission after application
close. One four-byte datagram is in flight per exchange; the 1,024 socket pool
and 32 release cycles are bounded correctness stimuli, not a performance receipt.
