# Wave decisions: node-local persistent volume spikes

Gate recommendation per probe. PROMOTE rests on measured behaviour, not on a candidate working once. Every PROMOTE below is conditional on the DESIGN decision it names. No user ruling is implied; these are inputs to the decision request.

| Probe | Recommendation | Rests on | Condition or next step |
|---|---|---|---|
| A1/A2 durability (write-back, any backend, `direct` on/off, SIGKILL) | **PROMOTE** write-back plus guest FLUSH as the durability contract | 11 block cells and 5 XFS-file cells, 0 violations; negative control 40/40 | Scope is block-layer ordering, not power loss; DISTILL needs a Tier-3 test for it, because a spike is evidence, not coverage |
| A3 fsync-error propagation | **PIVOT**: do not offer write-through on the stock fork; **F1 PROMOTE** (proven) | Stock: 20/20 swallowed on io_uring and AIO, sync wedges. Fixed binary: `errs_after_injection=20`, `acks_after_injection=0` on uring, AIO and sync, no wedge; explicit-flush and A1 wb/wt cells unchanged (0 violations), `run-030` | Write-through is offerable only with F1 in the fork. Proof is dm-`error` injection, not a real media error |
| B single writer (CH vs CH) | **PROMOTE** the OFD lock as arbiter | Second VMM refused on file and bdev, both granularities | Design the replacement start as "retry on `AlreadyLocked` within a bound" |
| B process exit as "volume free" | **DISCARD** | io_uring: process gone 28 ms, lock held 2.46 s | The lock, not the exit, is the signal |
| B alias nodes, non-CH openers | **PIVOT**: F2 is required for a block-device volume; **F2 PROMOTE** (proven) | Stock: second VMM booted through an alias node. Fixed binary: refused with `EBUSY` on `Open`; B1 refused, B2 replacements boot (4/4), `run-031` | Replacement retry must treat block-device `EBUSY` (not only `AlreadyLocked`) as "still held"; in the B2b window the replacement got `EBUSY` at about 100 ms and booted after the 2.4 s drain. All openers must be the fork |
| C1 `RLIMIT_FSIZE` | **PROMOTE** a derived limit at least the largest file volume, or choose a block device | SIGXFSZ (sync, AIO) / EIO (io_uring) below the limit; bdev unaffected | Derivation is a DESIGN decision |
| C2 memory cgroup | **PROMOTE** `direct=on`, or more headroom than the current 8 MiB reserve | Page cache charged; 2.1x to 5.7x slower buffered; no OOM seen | OOM boundary under heavy dirtying is unmapped |
| C3 confinement | **PROMOTE** | Second writable disk works under uid 4200, landlock, seccomp | The platform must chown the file or set the node mode before launch |
| D backing benchmark | Evidence for the user's choice: **block device** wins fsync-bound loads (1.2x to 2.5x); prealloc buys nothing | n = 10 per cell, two substrates | Nested virtualization and co-tenants; a bare-metal re-run could be done before approval if the user wants it |
| E1 ENOSPC | **PROMOTE** a decision between sparse (EIO mid-flight) and reserved (fails at creation) | Guest EIO at 234 MiB, VMM survives; `fallocate` ENOSPC up front | Space model is a DESIGN decision |
| E2 discard | **PROMOTE** `sparse=off` if a reservation must survive guest discard | Default `sparse=true` returned 16 MiB | Open-time `fallocate` cost not measured |
| E3 image-type sniff | **PIVOT**: F3 is required, or the volume can be bricked by its own guest; **F3 PROMOTE** (proven) | Stock: qcow2, VHDX and VHD markers fail the next boot. Fixed binary: second boot reaches the guest and reads its marker back, file and bdev; omitted `image_type` unchanged, `run-032` | Always pass `image_type=raw`; omitted, a fixed-VHD footer shrinks the disk by 512 bytes (stock and spike) |
| F serial | **PROMOTE** an explicit `serial=` | Default changes with inode and host; 31 bytes refused | Derivation (max 20 bytes) is a DESIGN decision |

**Contract-shaping items:** F1, F2 and F3 were each built into one fixed binary (`3c8b8eb6f`) and re-run against the stock controls (`run-030` to `run-032`); each closes its gap, so none is open. Remaining, non-contract-shaping: see findings section 7.
