# Attempt c — 16,384 stock activated transport devices

**WORKS.** Native Linux 7.0.0-29-generic, stock CH v53.0 dependency pinned to
`9ed824d6d08df3e96f7d5f50795d9449ac99f431`. Zero KVM VMs or guest boots.

Stages 4 / 1,024 / 4,096 / 8,192 / 16,384 each retained independent activated
stock Vsock/Unix-muxer owners and passed a complete CID/epoch-tagged
bidirectional stock virtqueue/muxer exchange. Separate frozen kernel-object
audits reconciled every listener inode, nested/outer epoll owner, exit eventfd,
connection and device/context identity. A final full-pool recheck passed and was
not separately timed. The 16,384-stage 3.282924353s timer includes bidirectional
exchange, owner identity capture/write and process FD/task enumeration. Native
build duration is 28.476242s; retained launcher duration is 145.503525s.

At 16,384: PSS 1,226,449KiB, RSS 1,228,420KiB, 344,067 external FDs and 16,385
threads. Existing per-process FD hard limit was 524,288; only the capacity
process soft limit was raised to it. No global settings or host configuration
changed. Exact native dependency lock is retained as `Cargo.lock` and in the
build receipts.

Source/preparation failures from attempts a/b remain preserved. This corrected
attempt binds snapshot output and disables Python import-cache writes.

Canonical command:

```sh
python3 spike-scratch/netns-density-295-vsock-scale/launch.py increment-c
```

This existing attempt ID must not be rerun: source/evidence directories reject
reuse. A new experiment requires a new preserved increment.

Native runner / launcher exits: 0 / 0. Stock reset/shutdown restored 1 thread,
4 self-enumerated FDs and zero owned socket paths before process exit; the
administrative complement matched and host OOM count did not increase. Exact
PID/path/lease absence was independently attested after release. See the
[findings](../../../docs/feature/netns-density-295/spike/shared-memory-vsock-scale-findings.md)
and [evidence directory](evidence/). Large text receipts have a `.gz` suffix
with verified lossless decompression, recorded in the preservation manifest.
