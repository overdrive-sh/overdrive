# Step 11-06 review remediation reproduction — Increment B

Increment B retains the exact probe sources used to reproduce and validate the
same-reviewer F1/F3 findings. Increment A remains untouched and preserves the
original failure/control captures and reviewed source. The standalone Rust
probe is not a test lane and is never run by CI.

## Scope and execution boundary

The host runner launches the production static `overdrive-init` as PID 1 in a
disposable Cloud Hypervisor guest using the existing GuestKit and production
`CloudHypervisorVmm` confinement. The host side only accepts the existing
ControlOpen/Paired session and echoes the canonical OVD1 datagram frames. All
module, route and BPF effects remain inside disposable guests; the leased Metal
machine is only the Cloud Hypervisor launch substrate. The guest kernel is Linux
`7.0.0-38-generic` on x86_64.

Increment B's `Cargo.toml`, `src/guest.rs`, and `src/host.rs` are byte-identical
to Increment A's retained current sources. Their SHA-256 values are recorded in
`SHA256SUMS`; the production and GuestKit support sources used by the captures
are snapshotted under `source-at-capture/`. The probe does not add a public API,
test seam, permanent test, CI lane, or production runtime behavior.

## Outcomes

| Run | Contract outcome | Captured result |
|---|---|---|
| Increment A F3 baseline, before BPF release-order correction | While 1,024 old slots are held, a new send may be refused with `EPERM`; an admitted replacement datagram must not be lost. | All 32 replacement sends were admitted and lost; 32 retries echoed; `HOST_REQUESTS=1056`, `RECYCLE_RESULT lost=32/32`, guest `EXIT 3`. See `evidence/reviewer-f3-pre-fix-admitted-loss.log`. The old binary/object hashes are in `evidence/pre-fix-artifact-identities-metal.log`. |
| Increment B default workload, after F1/F3 and DISTILL F2 support | The default workload has `loopback_module=false`, reaches READY with only the guest's DeviceOpen/Version forwarder checks, starts UDP capture with the staged diagnostics modules, and keeps resident slots unavailable until owner cleanup. | `READY`, `ALL_1024_SLOTS_ECHOED`, 32 immediate replacements refused with `EPERM`, 32 retries echoed, `RECYCLE_RESULT lost=0/32`, `HOST_REQUESTS=1056`, guest `EXIT 0`. See `evidence/reruns/` and `evidence/default-kit-f1-f3-post-fix.log`. |
| Increment B optional-loopback contrast, after F3 | Same accepted release outcome with the optional loopback module present. | `RECYCLE_RESULT lost=0/32`, `HOST_REQUESTS=1056`, guest `EXIT 0`. See `evidence/reviewer-f3-post-fix-loopback.log`. |

`evidence/harness-double-diag-load-eexist.log` is a deliberately retained
harness incompatibility, not a product failure: the updated test-only GuestKit
PID 1 now preloads `inet_diag`, `tcp_diag`, and `udp_diag`, while Increment A's
old `with-diag` flag asks the workload to load those same modules again and gets
`EEXIST`. The successful Increment B runs omit that flag and exercise the same
production workload with the modules already staged and loaded by the fixture.
The default-workload capture has no `with-loopback` flag.

## Reproduction

Prepare the current guest kit on Metal, then run from the repository root:

```sh
cargo xtask metal run -- cargo xtask guest-kit-prepare
bash spike-scratch/netns-density-295-review-11-06/increment-b/run.sh default-kit
bash spike-scratch/netns-density-295-review-11-06/increment-b/run.sh loopback-recycle
```

The runner records its full stdout/stderr and the process exit status in a new
timestamped directory under `evidence/reruns/`. `double-diag-preload` is
available only to reproduce the fixture's duplicate-load `EEXIST` condition;
its expected nonzero result must not be counted as a product outcome.
`evidence/commands-at-capture.txt` records the exact executed command lines,
and `evidence/retention-manifest.json` records file hashes and lengths.
The runner's guest binary is an ignored build output; the raw console and full
runner log are retained under `evidence/reruns/`. Exact remote output-path
ownership repairs and their matching local/remote hashes are recorded in
`evidence/ownership-repair.json`; canonical Metal sync passed afterward.

## Artifact identity

The current successful run used the exact host and guest kernel versions
recorded above; `evidence/post-fix-artifact-identities-metal.log` contains the
production init, scripted peer, Rust BPF object, C/CO-RE candidate object and
`ovd_fwd.ko` digests. The host toolchain receipt is
`evidence/metal-toolchain-versions.log`: Clang 21.1.8, bpftool 7.7.0, libbpf
1.7 (development headers 1.6.3), the pinned `nightly-2026-08-05` toolchain,
`bpf-linker` 0.12.0, and Cloud Hypervisor 53.0.0. GuestKit preparation and its
kernel/module target are recorded in `evidence/guest-kit-prepare-metal.log`.
